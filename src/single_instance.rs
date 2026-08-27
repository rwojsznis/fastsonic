//! One running instance at a time.
//!
//! Two copies of Fastpotify fight over things a user notices: two Spotify
//! Connect devices with the same name, two MPRIS players for the media keys
//! to disagree about, two tray icons. So a second launch does not start a
//! second app; it asks the one already running to show itself and exits.
//!
//! Detection is a D-Bus well-known name, requested without queuing. The bus
//! grants it to exactly one process and releases it the moment that process
//! ends, crash included, so there is no stale lock file to clean up and no
//! race between two launches at once. Surfacing the running instance is the
//! MPRIS `Raise` method it already implements, which is also what a desktop's
//! own "jump to the running player" gesture calls.
//!
//! This uses zbus's blocking API deliberately. Both of zbus's executors are
//! compiled in here (the tray brings async-io, MPRIS brings tokio), so an
//! async connection awaited from an arbitrary runtime is not guaranteed to be
//! driven. The blocking API owns that problem, and a check that runs once
//! before the window exists has no reason to be asynchronous anyway.
//!
//! macOS and Windows have no session bus, so there the same two jobs are done
//! by a listening socket bound to loopback: binding is exclusive, so whoever
//! binds is the running instance, and a later launch connects to say "show
//! yourself" before exiting. It is bound to 127.0.0.1 so no firewall has an
//! opinion about it, it speaks only to itself, and the operating system
//! releases the port when the process ends.

/// The name held for the lifetime of the running instance.
#[cfg(target_os = "linux")]
const INSTANCE_NAME: &str = "rocks.fastpotify.Instance";

/// The MPRIS player to ask when another instance already holds the name.
#[cfg(target_os = "linux")]
const MPRIS_NAME: &str = "org.mpris.MediaPlayer2.fastpotify";

pub enum Outcome {
    /// This process is the only instance. Hold the guard until it exits.
    Only(Guard),
    /// Another instance is running and has been asked to show its window.
    Surfaced,
}

/// Holds whatever marks this process as the running instance. Dropping it
/// gives that up.
pub struct Guard {
    #[cfg(target_os = "linux")]
    _connection: Option<mpris_server::zbus::blocking::Connection>,
    /// Set by a later launch that wants this window brought forward. On Linux
    /// the same request arrives through MPRIS instead.
    show_requests: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

impl Guard {
    /// The flag a later launch sets to ask for the window. The interface
    /// polls it and clears it.
    pub fn show_requests(&self) -> std::sync::Arc<std::sync::atomic::AtomicBool> {
        std::sync::Arc::clone(&self.show_requests)
    }
}

/// Loopback port that marks a running instance on platforms without a bus.
/// Registered to nothing; chosen high and out of the ephemeral range.
#[cfg(not(target_os = "linux"))]
const INSTANCE_PORT: u16 = 47_113;

/// Sent by a later launch, and answered, so a foreign program that happens to
/// hold the port is never mistaken for Fastpotify.
#[cfg(not(target_os = "linux"))]
const SHOW_REQUEST: &[u8] = b"fastpotify:show\n";
#[cfg(not(target_os = "linux"))]
const SHOW_ACK: &[u8] = b"fastpotify:ok\n";

#[cfg(not(target_os = "linux"))]
pub fn acquire(waker: &crate::backend::Waker) -> Outcome {
    use std::io::{Read, Write};
    use std::net::{Ipv4Addr, TcpListener, TcpStream};
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::time::Duration;

    let listener = match TcpListener::bind((Ipv4Addr::LOCALHOST, INSTANCE_PORT)) {
        Ok(listener) => listener,
        Err(_) => {
            // Someone holds the port. Ask them to show themselves, and only
            // stand down if they answer as Fastpotify.
            let answered = TcpStream::connect((Ipv4Addr::LOCALHOST, INSTANCE_PORT))
                .and_then(|mut stream| {
                    stream.set_read_timeout(Some(Duration::from_secs(2)))?;
                    stream.write_all(SHOW_REQUEST)?;
                    let mut reply = [0u8; SHOW_ACK.len()];
                    stream.read_exact(&mut reply)?;
                    Ok(reply == SHOW_ACK)
                })
                .unwrap_or(false);
            if answered {
                return Outcome::Surfaced;
            }
            log::warn!("port {INSTANCE_PORT} is busy but not with Fastpotify; running unguarded");
            return Outcome::Only(Guard {
                show_requests: Default::default(),
            });
        }
    };

    let show_requests: Arc<AtomicBool> = Default::default();
    let flag = Arc::clone(&show_requests);
    let waker = waker.clone();
    let spawned = std::thread::Builder::new()
        .name("fastpotify-instance".to_owned())
        .spawn(move || {
            for stream in listener.incoming().flatten() {
                let mut stream = stream;
                let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
                let mut request = [0u8; SHOW_REQUEST.len()];
                if stream.read_exact(&mut request).is_ok() && request == SHOW_REQUEST {
                    let _ = stream.write_all(SHOW_ACK);
                    flag.store(true, Ordering::SeqCst);
                    waker.wake();
                }
            }
        });
    if let Err(error) = spawned {
        log::warn!("cannot listen for other launches: {error}");
    }
    Outcome::Only(Guard { show_requests })
}

#[cfg(target_os = "linux")]
pub fn acquire(_waker: &crate::backend::Waker) -> Outcome {
    use mpris_server::zbus::blocking::Connection;
    use mpris_server::zbus::fdo::{RequestNameFlags, RequestNameReply};

    let connection = match Connection::session() {
        Ok(connection) => connection,
        Err(error) => {
            // No session bus at all: nothing to coordinate through, so run.
            log::debug!("no session bus, running unguarded: {error}");
            return Outcome::Only(Guard {
                _connection: None,
                show_requests: Default::default(),
            });
        }
    };

    // Holding the name is how this process says it is the one running.
    // zbus reports a name another peer already owns as `NameTaken` rather
    // than as a reply, so that error is the ordinary second-launch path.
    match connection.request_name_with_flags(INSTANCE_NAME, RequestNameFlags::DoNotQueue.into()) {
        Ok(RequestNameReply::PrimaryOwner | RequestNameReply::AlreadyOwner) => {
            Outcome::Only(Guard {
                _connection: Some(connection),
                show_requests: Default::default(),
            })
        }
        Ok(_) | Err(mpris_server::zbus::Error::NameTaken) => {
            if !raise_running_instance(&connection) {
                log::warn!(
                    "Fastpotify is already running but did not answer; not starting a second copy"
                );
            }
            Outcome::Surfaced
        }
        Err(error) => {
            log::warn!("cannot check for a running instance, starting anyway: {error}");
            Outcome::Only(Guard {
                _connection: None,
                show_requests: Default::default(),
            })
        }
    }
}

/// Asks the running instance to show its window, retrying briefly because it
/// may still be registering MPRIS when this launch arrives.
#[cfg(target_os = "linux")]
fn raise_running_instance(connection: &mpris_server::zbus::blocking::Connection) -> bool {
    for attempt in 0..10 {
        if attempt > 0 {
            std::thread::sleep(std::time::Duration::from_millis(150));
        }
        let raised = connection.call_method(
            Some(MPRIS_NAME),
            "/org/mpris/MediaPlayer2",
            Some("org.mpris.MediaPlayer2"),
            "Raise",
            &(),
        );
        if raised.is_ok() {
            return true;
        }
    }
    false
}
