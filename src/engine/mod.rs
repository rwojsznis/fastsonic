//! Local playback: the music server's stream, decoded in this process.
//!
//! ```text
//!   the server  --HTTP GET stream.view-->  source.rs  (cache.rs, on disk)
//!                                             |
//!                                        decode.rs  (symphonia + src/opus.rs)
//!                                             |
//!                                        src/resample.rs
//!                                             |
//!                                        chain.rs   (src/eq.rs)
//!                                             |
//!                                        chain.rs   --> the visualisers (src/vis.rs)
//!                                             |
//!                                        chain.rs   (ReplayGain, src/limiter.rs)
//!                                             |
//!                                        output.rs  (volume, rodio, the device)
//! ```
//!
//! [`Engine`] is the handle the rest of the app holds: commands in,
//! [`LocalState`] snapshots out. Everything else happens on one thread in
//! `worker`, which owns the decoder and the device.
//!
//! This is the engine that replaces librespot's, so it keeps librespot's
//! contract — see `migration/02-audio-engine.md`, and `state` for the two
//! Connect-only fields that did not come across. Where it is going next is
//! on the board in `migration/PROGRESS.md`.
//!
//! The bytes of a stream are kept on disk as they are read ([`mod@cache`]),
//! so a track played before opens with no request at all — which is three
//! round trips of silence saved every time an album is played twice.
//!
//! An album runs from one track into the next without a gap: the track
//! after this one is opened while this one is still playing, and its sound
//! goes into the same sink behind it. So for the last half second of every
//! track two tracks are open, and what is being *decoded* is not what is
//! being *heard* — [`LocalState`] and the queue always follow the second
//! of those. `worker` holds that line and `output` is what makes it
//! measurable.
//!
//! The queue is [`mod@queue`] — the rules in `docs/_reference/queue.md`, as
//! pure state. It comes back out through [`Engine::queue`] and
//! [`EngineEvent::Queue`], because with no Connect there is no web API to
//! ask what plays next.

pub mod cache;
mod chain;
mod decode;
mod output;
pub mod queue;
mod source;
pub mod state;
mod worker;

use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{Context, Result, anyhow};

use crate::api::subsonic::SubsonicClient;

pub use cache::{Cache, CacheStats};
pub use queue::{QueueRow, QueueSnapshot};
pub use state::{
    EngineEvent, Interrupted, LoadSpec, LocalState, LocalTrack, Notify, Playback, PlayerCommand,
    RepeatMode,
};

/// How long a stream may take to answer before playback gives up on it.
/// Only the wait for the first byte: a stream is read for as long as it
/// plays, so there is no deadline on the body.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Clone, Debug)]
pub struct EngineConfig {
    /// The output device from Settings; `None` follows the system default.
    pub audio_device: Option<String>,
    /// How much sound to ask the device to hold, in milliseconds.
    pub buffer_ms: u32,
    pub initial_volume: u16,
    /// Whether tracks play at the ReplayGain the server reports for them,
    /// so that a loud album and a quiet one sound alike.
    pub normalisation: bool,
    /// Where the visualisers read the sound from, shared with the windows
    /// that draw it.
    pub tap: Arc<crate::vis::AudioTap>,
    /// The equalizer's settings, shared with the window that sets them.
    pub eq: crate::eq::SharedEq,
    /// Where the bytes of a stream are kept, so that playing a track again
    /// costs the server nothing. `None` turns the cache off, which is a
    /// setting; it is shared rather than made here because the track after
    /// this one is opened on the runtime and reads through the same cache.
    pub cache: Option<Arc<Cache>>,
}

impl Default for EngineConfig {
    fn default() -> Self {
        Self {
            audio_device: None,
            buffer_ms: crate::sink::DEFAULT_BUFFER_MS,
            initial_volume: u16::MAX / 2,
            normalisation: false,
            tap: crate::vis::AudioTap::new(),
            eq: crate::eq::shared(),
            cache: None,
        }
    }
}

pub(crate) enum Message {
    Command(PlayerCommand),
    /// The server's answer about a song the queue only knew the id of —
    /// asked for on the runtime so that the audio thread never waits on a
    /// queue row. `None` means it would not answer.
    Described(String, Box<Option<crate::api::subsonic::Child>>),
    /// The track that plays next, opened before the join it is for (P3.4),
    /// which is what makes the join silent. `None` means it would not
    /// open; the join then opens it the slow way, where a failure can be
    /// reported to the interface.
    Prefetched(String, Box<Option<worker::Opened>>),
    Shutdown,
}

pub struct Engine {
    commands: Sender<Message>,
    /// Kept so a shutdown can wait for the device to be let go before
    /// another engine opens it.
    thread: Mutex<Option<std::thread::JoinHandle<()>>>,
    state: Arc<Mutex<LocalState>>,
    queue: Arc<Mutex<QueueSnapshot>>,
}

impl Engine {
    /// Starts the audio thread. It opens no device until something plays,
    /// so a machine with no sound card still runs.
    ///
    /// `runtime` is the handle the app's asynchronous work already runs on:
    /// the audio thread borrows it for the few metadata calls that decide
    /// what to play, and reads the stream itself with a blocking client.
    pub fn start(
        config: &EngineConfig,
        client: Arc<SubsonicClient>,
        runtime: tokio::runtime::Handle,
        notify: Notify,
    ) -> Result<Self> {
        let credentials = client.credentials();
        let state = Arc::new(Mutex::new(LocalState {
            volume: config.initial_volume,
            connected: credentials.is_some(),
            username: credentials
                .map(|credentials| credentials.username)
                .unwrap_or_default(),
            ..LocalState::default()
        }));
        let queue: Arc<Mutex<QueueSnapshot>> = Arc::default();
        let (commands, receiver) = std::sync::mpsc::channel();
        // The worker is built on its own thread rather than moved onto it:
        // an open device holds a CoreAudio listener that cannot be sent
        // between threads, which is the same reason librespot hands its
        // sink to the player thread as a builder.
        let config = config.clone();
        let worker_notify = Arc::clone(&notify);
        let worker_state = Arc::clone(&state);
        let worker_queue = Arc::clone(&queue);
        let replies = commands.clone();
        // The stream is read with a blocking client, and a blocking client
        // may not be *built* from inside an asynchronous context: reqwest
        // stands a runtime up to do it and dropping that runtime on a tokio
        // worker panics. `Engine::start` is called from the backend's
        // command loop, which is exactly such a context — so the client is
        // built on the audio thread, and its outcome comes back here.
        let (ready_tx, ready_rx) = std::sync::mpsc::channel();
        let thread = std::thread::Builder::new()
            .name("fastsonic-audio".into())
            .spawn(move || {
                let http = crate::blocking_http_client_builder()
                    .connect_timeout(CONNECT_TIMEOUT)
                    .build();
                let http = match http {
                    Ok(http) => {
                        let _ = ready_tx.send(None);
                        http
                    }
                    Err(error) => {
                        let _ = ready_tx.send(Some(error.to_string()));
                        return;
                    }
                };
                worker::Worker::new(
                    &config,
                    client,
                    runtime,
                    http,
                    receiver,
                    replies,
                    worker::Shared {
                        notify: worker_notify,
                        state: worker_state,
                        queue: worker_queue,
                    },
                )
                .run()
            })
            .context("unable to start the audio thread")?;
        // Over in under a millisecond, and worth waiting for: a caller that
        // was told the engine started must not then find it silently gone.
        match ready_rx.recv() {
            Ok(None) => {}
            Ok(Some(error)) => {
                let _ = thread.join();
                return Err(anyhow!("unable to build the audio stream client: {error}"));
            }
            Err(_) => {
                let _ = thread.join();
                return Err(anyhow!("the audio thread stopped before it started"));
            }
        }
        notify(EngineEvent::State(
            state.lock().unwrap_or_else(|p| p.into_inner()).clone(),
        ));
        Ok(Self {
            commands,
            thread: Mutex::new(Some(thread)),
            state,
            queue,
        })
    }

    pub fn command(&self, command: PlayerCommand) -> Result<()> {
        self.commands
            .send(Message::Command(command))
            .map_err(|_| anyhow!("the audio thread is not running"))
    }

    /// The last snapshot, for a caller that needs one without waiting for
    /// the next change.
    pub fn state(&self) -> LocalState {
        self.state.lock().unwrap_or_else(|p| p.into_inner()).clone()
    }

    /// What is playing next, for a caller that needs the queue without
    /// waiting for the next change.
    pub fn queue(&self) -> QueueSnapshot {
        self.queue.lock().unwrap_or_else(|p| p.into_inner()).clone()
    }

    /// Playback state to resume after replacing this engine, which is what
    /// changing the output device does.
    pub fn interrupted(&self) -> Option<Interrupted> {
        self.state
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .interrupted()
    }

    /// Stops playback and waits for the device to be let go.
    pub fn shutdown(&self) {
        let _ = self.commands.send(Message::Shutdown);
        let thread = self.thread.lock().unwrap_or_else(|p| p.into_inner()).take();
        if let Some(thread) = thread
            && thread.join().is_err()
        {
            log::error!("the audio thread ended badly");
        }
    }
}

impl Drop for Engine {
    fn drop(&mut self) {
        self.shutdown();
    }
}
