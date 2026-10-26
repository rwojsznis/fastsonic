//! Linux: the power-saver profile, sleep and wake from logind, and keeping
//! the computer awake.
//!
//! Each runs on a thread of its own over D-Bus, as the colour scheme does
//! (`appearance.rs`), so a slow or absent bus never holds up the app.
//!
//! - The power profile comes from the desktop portal's PowerProfileMonitor,
//!   which works inside Flatpak, or else from power-profiles-daemon.
//! - logind announces sleep with `PrepareForSleep`. A "delay" inhibitor lock
//!   gives the app a moment to save first; inside Flatpak it needs
//!   `--system-talk-name=org.freedesktop.login1`, and without it the app
//!   still hears about the wake.
//! - Keeping the computer awake asks the portal's Inhibit for Suspend,
//!   which GNOME and KDE honour for their automatic suspend and which works
//!   inside Flatpak, or else takes an "idle" block lock from logind.
//! - The engine's thread asks for nice -5, where the system allows it.
//!
//! <https://flatpak.github.io/xdg-desktop-portal/docs/doc-org.freedesktop.portal.PowerProfileMonitor.html>
//! <https://systemd.io/INHIBITOR_LOCKS/>

use std::sync::Arc;

use zbus::blocking::{Connection, Proxy};

use super::map::saves;
use super::{Event, SUSPEND_ACK, Shared};

/// The threads run for the life of the process; nothing to undo.
pub(super) struct Reader;

impl Reader {
    pub(super) fn start(shared: Arc<Shared>) -> Result<Self, String> {
        let saver = Arc::clone(&shared);
        spawn("fastsonic-power-profile", move || {
            if let Err(error) = watch_saver(&saver) {
                log::debug!("the power profile is unavailable: {error}");
            }
        })?;
        spawn("fastsonic-sleep", move || {
            if let Err(error) = watch_sleep(&shared) {
                log::debug!("sleep and wake are unavailable: {error}");
            }
        })?;
        Ok(Self)
    }
}

fn spawn(name: &str, run: impl FnOnce() + Send + 'static) -> Result<(), String> {
    std::thread::Builder::new()
        .name(name.to_string())
        .spawn(run)
        .map(|_| ())
        .map_err(|error| error.to_string())
}

fn watch_saver(shared: &Shared) -> zbus::Result<()> {
    match watch_portal_saver(shared) {
        Ok(()) => Ok(()),
        Err(error) => {
            log::debug!("no power profile portal ({error}); asking power-profiles-daemon");
            watch_daemon_profile(shared)
        }
    }
}

/// The portal's `power-saver-enabled` property, and its changes.
fn watch_portal_saver(shared: &Shared) -> zbus::Result<()> {
    let connection = Connection::session()?;
    let proxy = Proxy::new(
        &connection,
        "org.freedesktop.portal.Desktop",
        "/org/freedesktop/portal/desktop",
        "org.freedesktop.portal.PowerProfileMonitor",
    )?;
    let changes = proxy.receive_property_changed::<bool>("power-saver-enabled");
    let current: bool = proxy.get_property("power-saver-enabled")?;
    shared.update(|conditions| conditions.saver = current);
    for change in changes {
        if let Ok(on) = change.get() {
            shared.update(|conditions| conditions.saver = on);
        }
    }
    Ok(())
}

/// power-profiles-daemon's `ActiveProfile`, under its current name or the
/// one it had before 0.20.
fn watch_daemon_profile(shared: &Shared) -> zbus::Result<()> {
    let connection = Connection::system()?;
    let proxy = Proxy::new(
        &connection,
        "org.freedesktop.UPower.PowerProfiles",
        "/org/freedesktop/UPower/PowerProfiles",
        "org.freedesktop.UPower.PowerProfiles",
    )
    .and_then(|proxy| proxy.get_property::<String>("ActiveProfile").map(|_| proxy))
    .or_else(|_| {
        Proxy::new(
            &connection,
            "net.hadess.PowerProfiles",
            "/net/hadess/PowerProfiles",
            "net.hadess.PowerProfiles",
        )
    })?;
    let changes = proxy.receive_property_changed::<String>("ActiveProfile");
    let current: String = proxy.get_property("ActiveProfile")?;
    shared.update(|conditions| conditions.saver = saves(&current));
    for change in changes {
        if let Ok(profile) = change.get() {
            shared.update(|conditions| conditions.saver = saves(&profile));
        }
    }
    Ok(())
}

/// logind's `PrepareForSleep`, holding a delay lock while awake.
fn watch_sleep(shared: &Shared) -> zbus::Result<()> {
    let connection = Connection::system()?;
    let manager = Proxy::new(
        &connection,
        "org.freedesktop.login1",
        "/org/freedesktop/login1",
        "org.freedesktop.login1.Manager",
    )?;
    let signals = manager.receive_signal("PrepareForSleep")?;
    let mut lock = inhibit(&manager);
    for message in signals {
        let Ok(sleeping) = message.body().deserialize::<bool>() else {
            continue;
        };
        if sleeping {
            shared.push_and_wait(Event::Suspending, SUSPEND_ACK);
            // Closing the lock lets the system sleep.
            lock.take();
        } else {
            shared.push(Event::Resumed);
            lock = inhibit(&manager);
        }
    }
    Ok(())
}

/// The portal's Inhibit flag for suspending the session.
/// <https://flatpak.github.io/xdg-desktop-portal/docs/doc-org.freedesktop.portal.Inhibit.html>
const PORTAL_SUSPEND: u32 = 4;

/// Keeps the computer from sleeping on its own until dropped: a thread asks
/// the desktop portal, or else logind, and holds what it got until the
/// sender is dropped.
pub(super) struct KeepAwake {
    _release: std::sync::mpsc::Sender<()>,
}

impl KeepAwake {
    pub(super) fn begin(reason: &str) -> Self {
        let (release, released) = std::sync::mpsc::channel::<()>();
        let reason = reason.to_owned();
        if let Err(error) = spawn("fastsonic-keep-awake", move || {
            hold_awake(&reason, &released);
        }) {
            log::debug!("cannot keep the computer awake: {error}");
        }
        Self { _release: release }
    }
}

/// Inhibits idle sleep, waits for the release, then lets it go.
fn hold_awake(reason: &str, released: &std::sync::mpsc::Receiver<()>) {
    match portal_inhibit(reason) {
        Ok((connection, request)) => {
            // Blocks until the `KeepAwake` is dropped.
            let _ = released.recv();
            let closed = Proxy::new(
                &connection,
                "org.freedesktop.portal.Desktop",
                request.as_ref(),
                "org.freedesktop.portal.Request",
            )
            .and_then(|proxy| proxy.call::<_, _, ()>("Close", &()));
            if let Err(error) = closed {
                // Closing the connection, as this thread now does, ends it too.
                log::debug!("the portal's inhibit did not close: {error}");
            }
        }
        Err(error) => {
            log::debug!("no inhibit portal ({error}); asking logind");
            match logind_inhibit(reason) {
                Ok(lock) => {
                    let _ = released.recv();
                    drop(lock);
                }
                Err(error) => log::debug!("cannot keep the computer awake: {error}"),
            }
        }
    }
}

/// The portal's Inhibit with the Suspend flag. The inhibition lasts until
/// its request is closed or the connection ends.
fn portal_inhibit(reason: &str) -> zbus::Result<(Connection, zbus::zvariant::OwnedObjectPath)> {
    let connection = Connection::session()?;
    let proxy = Proxy::new(
        &connection,
        "org.freedesktop.portal.Desktop",
        "/org/freedesktop/portal/desktop",
        "org.freedesktop.portal.Inhibit",
    )?;
    let mut options = std::collections::HashMap::new();
    options.insert("reason", zbus::zvariant::Value::from(reason));
    let request: zbus::zvariant::OwnedObjectPath =
        proxy.call("Inhibit", &("", PORTAL_SUSPEND, options))?;
    Ok((connection, request))
}

/// An "idle" block lock from logind, held as long as its descriptor is
/// open. It is listed in `systemd-inhibit --list` under the app's name.
fn logind_inhibit(reason: &str) -> zbus::Result<zbus::zvariant::OwnedFd> {
    let connection = Connection::system()?;
    let manager = Proxy::new(
        &connection,
        "org.freedesktop.login1",
        "/org/freedesktop/login1",
        "org.freedesktop.login1.Manager",
    )?;
    manager.call("Inhibit", &("idle", "Fastsonic", reason, "block"))
}

/// Ahead of ordinary threads, well short of what a real-time audio thread
/// gets.
const FEEDER_NICE: libc::c_int = -5;

/// Raises the calling thread to [`FEEDER_NICE`]. Without the right to (no
/// `RLIMIT_NICE` allowance, as usual for a desktop user) it is refused.
pub(super) fn raise_feeder() -> std::io::Result<()> {
    // SAFETY: `gettid` takes nothing; with a thread id, PRIO_PROCESS
    // changes only that thread on Linux.
    let result = unsafe {
        libc::setpriority(
            libc::PRIO_PROCESS,
            libc::gettid() as libc::id_t,
            FEEDER_NICE,
        )
    };
    if result == 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error())
    }
}

/// Takes a "delay" lock on sleep, or gets on without one. It is listed in
/// `systemd-inhibit --list` under the app's name.
fn inhibit(manager: &Proxy<'_>) -> Option<zbus::zvariant::OwnedFd> {
    manager
        .call(
            "Inhibit",
            &(
                "sleep",
                "Fastsonic",
                "Saves the session before sleeping",
                "delay",
            ),
        )
        .inspect_err(|error| log::debug!("no delay before sleep: {error}"))
        .ok()
}
