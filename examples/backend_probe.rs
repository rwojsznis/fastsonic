//! P4.1: drive `src/backend.rs` the way `src/app.rs` does, and watch what
//! comes back.
//!
//! `examples/engine_probe.rs` proves the engine; this proves the *bridge*.
//! It signs in with a password the way the form does, then sends the
//! `ApiRequest`s the pages send and the `PlayerCommand`s the player bar
//! sends, and checks the `Event`s against what the interface would need
//! them to say. Everything here goes through the same channels the running
//! app uses — no client is built by hand, and nothing is called directly.
//!
//! ```sh
//! (cd migration/devserver && docker compose up -d)
//! cargo run --example backend_probe
//! ```
//!
//! It uses a scratch config directory, so running it neither reads nor
//! disturbs the credential of an app you are actually signed in to.
//! `FASTSONIC_TEST_SERVER`, `FASTSONIC_TEST_USER` and
//! `FASTSONIC_TEST_PASSWORD` point it at a server other than
//! `migration/devserver`.

use std::time::{Duration, Instant};

use fastsonic::backend::{
    AlbumShelf, ApiRequest, ApiResponse, AuthStatus, Backend, Command, Event, LocalPlayback,
    RecentsFor, SignInRequest, Waker,
};
use fastsonic::engine::{LoadSpec, LocalState, Playback, PlayerCommand, QueueSnapshot};
use fastsonic::paths::AppDirs;

/// How long any one answer is waited for. Everything here is one request to
/// a server on the same machine, or a command to a thread in this process.
const PATIENCE: Duration = Duration::from_secs(20);

fn main() -> anyhow::Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("warn")).init();

    let server = env("FASTSONIC_TEST_SERVER", "http://localhost:4533");
    let username = env("FASTSONIC_TEST_USER", "admin");
    let password = env("FASTSONIC_TEST_PASSWORD", "fastsonic");
    let query = env("FASTSONIC_TEST_QUERY", "signal");

    let scratch =
        std::env::temp_dir().join(format!("fastsonic-backend-probe-{}", std::process::id()));
    let dirs = AppDirs {
        config: scratch.join("config"),
        state: scratch.join("state"),
        cache: scratch.join("cache"),
    };
    dirs.ensure()?;
    println!("server {server}, scratch {}", scratch.display());

    let mut probe = Probe::new(Backend::spawn(
        dirs.clone(),
        fastsonic::engine::EngineConfig::default(),
        Waker::default(),
    ));

    println!("\n-- before signing in");
    let start = probe.next_auth()?;
    probe.check(
        "the app starts signed out",
        matches!(start, AuthStatus::SignedOut),
    );

    println!("\n-- sign in");
    probe.backend.send(Command::SignIn(Box::new(SignInRequest {
        server: server.clone(),
        username: username.clone(),
        password: Some(password.clone()),
    })));
    let connected = loop {
        match probe.next_auth()? {
            AuthStatus::Connecting => continue,
            other => break other,
        }
    };
    probe.check(
        "the server accepted the password",
        matches!(&connected, AuthStatus::Connected { .. }),
    );
    if let AuthStatus::Failed(message) = &connected {
        anyhow::bail!("sign-in failed: {message}");
    }
    probe.check(
        "the credential was written, and the password was not",
        credential_is_a_token(&dirs, &password),
    );
    let playback = loop {
        match probe.next_playback()? {
            LocalPlayback::Connecting => continue,
            other => break other,
        }
    };
    probe.check(
        "local playback came up with the sign-in",
        matches!(playback, LocalPlayback::Ready),
    );
    let me = probe.next_api(|response| match response {
        ApiResponse::Me(result) => Some(result),
        _ => None,
    })?;
    probe.check("the account answered", me.is_ok());

    println!("\n-- the pages ask, the way app.rs asks");
    probe.backend.api(ApiRequest::MyPlaylists { offset: 0 });
    let playlists = probe.next_api(|response| match response {
        ApiResponse::MyPlaylists { result, .. } => Some(result),
        _ => None,
    })?;
    // A library with no playlists in it is a library, not a failure — this
    // is the sidebar loading, and an empty sidebar is a legitimate answer.
    probe.check("the playlist sidebar loads", playlists.is_ok());

    probe.backend.api(ApiRequest::SavedTracks { offset: 0 });
    let saved = probe.next_api(|response| match response {
        ApiResponse::SavedTracks { result, .. } => Some(result),
        _ => None,
    })?;
    probe.check("starred songs load", saved.is_ok());

    probe.backend.api(ApiRequest::Search {
        query: query.clone(),
        serial: 1,
    });
    let found = probe.next_api(|response| match response {
        ApiResponse::Search { result, .. } => Some(result),
        _ => None,
    })??;
    // The album to play the rest of this with. Taken from the search rather
    // than from a shelf because a search is the one listing every library
    // answers; `FASTSONIC_TEST_QUERY` is how a library that does not hold
    // `migration/devserver`'s fixtures says what to look for.
    let album_id = found
        .albums
        .as_ref()
        .and_then(|page| page.items.first())
        .map(|album| album.id.clone())
        .or_else(|| {
            found
                .tracks
                .as_ref()?
                .items
                .iter()
                .find_map(|track| track.album.as_ref().map(|album| album.id.clone()))
        })
        .ok_or_else(|| {
            anyhow::anyhow!("nothing in this library matches {query:?}; set FASTSONIC_TEST_QUERY")
        })?;
    probe.check(
        "search answers with something to open",
        !found.is_empty() && !album_id.is_empty(),
    );

    probe.backend.api(ApiRequest::Album {
        id: album_id.clone(),
    });
    let album = probe.next_api(|response| match response {
        ApiResponse::Album { result, .. } => Some(result),
        _ => None,
    })??;
    probe.check(
        "an album arrives with its tracks, in one call",
        album
            .tracks
            .as_ref()
            .is_some_and(|page| !page.items.is_empty()),
    );
    probe.check(
        "artwork is carried as a request rather than a URL",
        album
            .images
            .iter()
            .any(|image| image.url.starts_with("sonic:art:")),
    );
    probe.check(
        "a song states whether it is starred, so no page has to ask",
        album
            .tracks
            .as_ref()
            .and_then(|page| page.items.first())
            .is_some_and(|track| track.starred.is_some()),
    );

    // Home's three album shelves. Recently added and the random one have
    // something in them on any real library; most played needs the server
    // to have counted a play, so only the answer is checked.
    for shelf in [AlbumShelf::Newest, AlbumShelf::Frequent, AlbumShelf::Random] {
        probe.backend.api(ApiRequest::AlbumShelf {
            shelf,
            generation: 1,
        });
        let albums = probe.next_api(|response| match response {
            ApiResponse::AlbumShelf { result, .. } => Some(result),
            _ => None,
        })?;
        probe.check(&format!("the {shelf:?} shelf answers"), albums.is_ok());
    }
    probe.backend.api(ApiRequest::AlbumShelf {
        shelf: AlbumShelf::Newest,
        generation: 1,
    });
    let newest = probe.next_api(|response| match response {
        ApiResponse::AlbumShelf { result, .. } => Some(result),
        _ => None,
    })??;
    probe.check("recently added has records on it", !newest.is_empty());

    // The native API's three. They are empty on a server that is not
    // Navidrome and on one nothing has been played on — which is not a
    // failure (D11), so this only checks that an answer arrives at all.
    probe.backend.api(ApiRequest::RecentlyPlayed {
        who: RecentsFor::Panel,
        generation: 1,
        before: None,
        limit: 10,
    });
    let recents = probe.next_api(|response| match response {
        ApiResponse::RecentlyPlayed { result, .. } => Some(result),
        _ => None,
    })?;
    probe.check("recently played answers", recents.is_ok());

    probe.backend.api(ApiRequest::TopTracks {
        offset: 0,
        full: false,
        generation: 1,
    });
    let top = probe.next_api(|response| match response {
        ApiResponse::TopTracks { result, .. } => Some(result),
        _ => None,
    })?;
    probe.check("top songs answer", top.is_ok());

    println!("\n-- play an album through the bridge");
    probe.backend.player(PlayerCommand::Load(LoadSpec {
        context_uri: Some(album.uri.clone()),
        play: true,
        ..LoadSpec::default()
    }));
    let playing = probe.until_local(|state| state.playback == Playback::Playing)?;
    probe.check("the player bar would show a track", playing.track.is_some());
    std::thread::sleep(Duration::from_millis(900));
    let moved = probe.until_local(|state| state.position_ms > 300).is_ok();
    probe.check("the position moves", moved);
    let queue = probe.last_queue.clone().unwrap_or_default();
    probe.check(
        "the queue panel was told what plays next",
        !queue.is_empty(),
    );
    probe.check(
        "the queue's rows carry a track each",
        queue.rows().all(|row| row.track.is_some()),
    );

    probe.backend.player(PlayerCommand::AddToQueue(
        queue
            .upcoming
            .last()
            .map(|row| row.uri.clone())
            .unwrap_or_default(),
    ));
    let queued = probe.until_queue(|queue| !queue.queued.is_empty())?;
    probe.check("Play next puts a row in its own section", {
        queued.queued.len() == 1
    });

    probe.backend.player(PlayerCommand::Toggle);
    let paused = probe
        .until_local(|state| state.playback == Playback::Paused)
        .is_ok();
    probe.check("pause reaches the engine", paused);

    println!("\n-- the queue's rules, through the channels app.rs uses (P4.3)");
    // Paused, deliberately: an album playing on underneath would consume
    // the very rows these rules are about, and the fixtures are seconds
    // long. Every rule below is about the queue rather than the sound.
    //
    // Rule 2: two asks are two rows, in the order they were asked for and
    // ahead of the album's own rows.
    let wanted: Vec<String> = queue
        .upcoming
        .iter()
        .rev()
        .take(2)
        .map(|row| row.uri.clone())
        .collect();
    let mut waiting = queued.queued.len();
    probe.forget_queues();
    for uri in &wanted {
        probe.backend.player(PlayerCommand::AddToQueue(uri.clone()));
        waiting += 1;
    }
    let queued = probe.until_queue(move |queue| queue.queued.len() == waiting)?;
    probe.check(
        "Play next keeps the order it was asked in, ahead of the album",
        queued
            .queued
            .iter()
            .skip(queued.queued.len() - wanted.len())
            .map(|row| &row.uri)
            .eq(wanted.iter()),
    );
    // Rule 1: the rows are the play order — yours, then the album's — and
    // that is what the row number a click carries counts along.
    let rows: Vec<String> = queued.rows().map(|row| row.uri.clone()).collect();
    probe.check(
        "the rows the panel draws are your songs and then the album's",
        rows.len() == queued.queued.len() + queued.upcoming.len(),
    );
    // Rule 5: one command skips down to a row; the rows above go with it
    // and the ones below stay.
    let target = rows[1].clone();
    let below = rows.len() - 2;
    probe.backend.player(PlayerCommand::PlayQueued(1));
    let skipped = probe
        .until_queue(move |queue| queue.current.as_ref().is_some_and(|row| row.uri == target))?;
    probe.check(
        "playing a row skips down to it and takes the rows above",
        skipped.rows().count() == below,
    );
    // Rule 6: a new album changes the rows underneath and keeps yours.
    let mine: Vec<String> = skipped.queued.iter().map(|row| row.uri.clone()).collect();
    probe.forget_queues();
    probe.backend.player(PlayerCommand::Load(LoadSpec {
        context_uri: Some(album.uri.clone()),
        play: false,
        ..LoadSpec::default()
    }));
    let reloaded = probe.until_queue(|queue| !queue.upcoming.is_empty())?;
    probe.check(
        "a new album keeps the songs you queued on top of it",
        reloaded
            .queued
            .iter()
            .map(|row| row.uri.clone())
            .collect::<Vec<_>>()
            == mine,
    );
    // Rule 7: Clear empties your section and leaves the album's alone.
    let album_rows = reloaded.upcoming.len();
    probe.forget_queues();
    probe.backend.player(PlayerCommand::ClearQueue);
    let cleared = probe.until_queue(|queue| queue.queued.is_empty())?;
    probe.check(
        "Clear takes your songs and leaves the album's",
        cleared.upcoming.len() == album_rows,
    );

    println!("\n-- rule 9: the queue survives the engine being replaced (P4.4)");
    // Changing the output device or the normalisation switch builds a new
    // engine, and the queue lives in the engine — so the old one would take
    // it away. The hardest case is the one set up here: a song you queued
    // playing *over* an album, which keeps its own place underneath it
    // (rule 3). Both have to arrive on the other side.
    let mine: Vec<String> = cleared
        .upcoming
        .iter()
        .rev()
        .take(2)
        .map(|row| row.uri.clone())
        .collect();
    probe.forget_queues();
    for uri in &mine {
        probe.backend.player(PlayerCommand::AddToQueue(uri.clone()));
    }
    let queued = probe.until_queue(|queue| queue.queued.len() == 2)?;
    probe.check(
        "two songs are queued to carry across",
        queued.queued.len() == 2,
    );
    probe.forget_queues();
    probe.backend.player(PlayerCommand::PlayQueued(0));
    let before = probe.until_queue(|queue| queue.queued.len() == 1)?;
    // Paused, so that an album playing on cannot consume the rows this is
    // about while the engine is being swapped.
    probe.backend.player(PlayerCommand::Toggle);
    let paused = probe.until_local(|state| state.playback == Playback::Paused)?;
    let current = before.current.as_ref().map(|row| row.uri.clone());
    let carried = current.clone();
    let queued_uris: Vec<String> = before.queued.iter().map(|row| row.uri.clone()).collect();
    let context_at = before.context_at.clone();
    let album_rows = before.upcoming.len();
    probe.check(
        "a queued song plays over an album that keeps its own row",
        current.is_some() && context_at.is_some() && context_at != current,
    );
    probe.forget_queues();
    probe.backend.send(Command::RestartEngine(
        fastsonic::engine::EngineConfig::default(),
    ));
    let after = probe
        .until_queue(move |queue| queue.current.as_ref().map(|row| &row.uri) == carried.as_ref())?;
    probe.check(
        "the song playing arrives in the new engine",
        after.current.as_ref().map(|row| row.uri.clone()) == current,
    );
    probe.check(
        "with the songs queued behind it, in order",
        after
            .queued
            .iter()
            .map(|row| row.uri.clone())
            .collect::<Vec<_>>()
            == queued_uris,
    );
    probe.check(
        "and the album still on the row it was on, with the rest of it below",
        after.context_at == context_at && after.upcoming.len() == album_rows,
    );
    let resumed =
        probe.until_local(|state| state.track.is_some() && state.playback != Playback::Loading)?;
    probe.check(
        "a paused player comes back paused, where it was",
        resumed.playback == Playback::Paused
            && resumed.position_ms.abs_diff(paused.position_ms) < 2_000,
    );

    println!("\n-- Liked Songs is a context of its own");
    // The starred list has no id to address, so it is `sonic:collection:songs`
    // and the engine expands it with one `getStarred2` (P4.2). Star a song
    // first, so the check does not depend on what the library already holds.
    let song = queue
        .current
        .as_ref()
        .map(|row| row.uri.clone())
        .unwrap_or_default();
    probe.backend.api(ApiRequest::SetSaved {
        uris: vec![song.clone()],
        saved: true,
    });
    let starred = probe.next_api(|response| match response {
        ApiResponse::SavedChanged { result, .. } => Some(result),
        _ => None,
    })?;
    probe.check("a heart reaches the server", starred.is_ok());
    probe.backend.player(PlayerCommand::Load(LoadSpec {
        context_uri: Some(fastsonic::api::subsonic::convert::COLLECTION_URI.to_string()),
        play: true,
        ..LoadSpec::default()
    }));
    let liked = probe
        .until_local(|state| state.playback == Playback::Playing && state.error.is_none())
        .is_ok();
    probe.check("the starred songs play as a context", liked);
    probe.backend.api(ApiRequest::SetSaved {
        uris: vec![song],
        saved: false,
    });
    let _ = probe.next_api(|response| match response {
        ApiResponse::SavedChanged { result, .. } => Some(result),
        _ => None,
    });
    probe.backend.player(PlayerCommand::Toggle);

    println!("\n-- start again against the same files");
    probe.backend.shutdown();
    let mut probe = Probe::new(Backend::spawn(
        dirs.clone(),
        fastsonic::engine::EngineConfig::default(),
        Waker::default(),
    ));
    let restored = loop {
        match probe.next_auth()? {
            AuthStatus::Connecting => continue,
            other => break other,
        }
    };
    probe.check(
        "the stored pair signs in again, with no password to type",
        matches!(&restored, AuthStatus::Connected { .. }),
    );
    if let AuthStatus::Failed(message) = &restored {
        println!("      ({message})");
    }

    println!("\n-- sign out");
    probe.backend.send(Command::SignOut);
    let signed_out = loop {
        match probe.next_auth()? {
            AuthStatus::Connecting => continue,
            other => break other,
        }
    };
    probe.check(
        "signing out ends the session",
        matches!(signed_out, AuthStatus::SignedOut),
    );
    // The engine's own events from the restore above are still queued, so
    // this reads past them rather than taking the first one it finds.
    let stopped = loop {
        if matches!(probe.next_playback()?, LocalPlayback::Unavailable) {
            break true;
        }
    };
    probe.check("signing out stops playback", stopped);
    probe.check(
        "signing out forgets the credential",
        !dirs.credentials_file().exists(),
    );

    probe.backend.shutdown();
    let _ = std::fs::remove_dir_all(&scratch);

    println!();
    if probe.failures.is_empty() {
        println!("all checks passed");
        return Ok(());
    }
    println!("{} check(s) failed:", probe.failures.len());
    for failure in &probe.failures {
        println!("  - {failure}");
    }
    std::process::exit(1);
}

/// The interface's half of the channel: it polls, so this polls.
struct Probe {
    backend: Backend,
    failures: Vec<String>,
    /// Events read while waiting for a different one. Nothing may be thrown
    /// away — a state snapshot arrives between every two answers.
    pending: Vec<Event>,
    last_queue: Option<QueueSnapshot>,
}

impl Probe {
    fn new(backend: Backend) -> Self {
        Self {
            backend,
            failures: Vec::new(),
            pending: Vec::new(),
            last_queue: None,
        }
    }

    fn check(&mut self, claim: &str, held: bool) {
        println!("    {} {claim}", if held { "ok" } else { "FAILED" });
        if !held {
            self.failures.push(claim.to_string());
        }
    }

    /// The next event matching `want`, keeping everything else for later.
    /// `Err(None)` throws one away, which is what an answer nobody is
    /// waiting for deserves.
    fn next<T>(
        &mut self,
        mut want: impl FnMut(Event) -> Result<T, Option<Event>>,
    ) -> anyhow::Result<T> {
        let until = Instant::now() + PATIENCE;
        let mut queue: Vec<Event> = std::mem::take(&mut self.pending);
        loop {
            while !queue.is_empty() {
                let event = queue.remove(0);
                if let Event::Queue(snapshot) = &event {
                    self.last_queue = Some((**snapshot).clone());
                }
                match want(event) {
                    Ok(found) => {
                        self.pending.extend(queue);
                        return Ok(found);
                    }
                    Err(Some(other)) => self.pending.push(other),
                    Err(None) => {}
                }
            }
            if Instant::now() > until {
                anyhow::bail!("nothing answered within {PATIENCE:?}");
            }
            queue = self.backend.poll();
            if queue.is_empty() {
                std::thread::sleep(Duration::from_millis(20));
            }
        }
    }

    fn next_auth(&mut self) -> anyhow::Result<AuthStatus> {
        self.next(|event| match event {
            Event::Auth(status) => Ok(status),
            other => Err(Some(other)),
        })
    }

    fn next_playback(&mut self) -> anyhow::Result<LocalPlayback> {
        self.next(|event| match event {
            Event::Playback(status) => Ok(status),
            other => Err(Some(other)),
        })
    }

    fn next_api<T>(&mut self, mut want: impl FnMut(ApiResponse) -> Option<T>) -> anyhow::Result<T> {
        self.next(move |event| match event {
            // A response of another kind is dropped rather than kept: no
            // check here waits for two answers at once, and keeping it
            // would let a stale one satisfy a later wait.
            Event::Api(response) => want(*response).ok_or(None),
            other => Err(Some(other)),
        })
    }

    fn until_local(
        &mut self,
        mut ready: impl FnMut(&LocalState) -> bool,
    ) -> anyhow::Result<LocalState> {
        self.next(move |event| match event {
            Event::Local(state) if ready(&state) => Ok(*state),
            other => Err(Some(other)),
        })
    }

    /// Forget the queue snapshots already in hand. A rule about what a
    /// command *changes* has to be read from a snapshot published after it
    /// was sent, and every snapshot from before is still in the buffer —
    /// including ones that would satisfy the question by accident.
    fn forget_queues(&mut self) {
        self.pending
            .retain(|event| !matches!(event, Event::Queue(_)));
        let keep: Vec<Event> = self
            .backend
            .poll()
            .into_iter()
            .filter(|event| !matches!(event, Event::Queue(_)))
            .collect();
        self.pending.extend(keep);
    }

    fn until_queue(
        &mut self,
        mut ready: impl FnMut(&QueueSnapshot) -> bool,
    ) -> anyhow::Result<QueueSnapshot> {
        self.next(move |event| match event {
            Event::Queue(queue) if ready(&queue) => Ok(*queue),
            other => Err(Some(other)),
        })
    }
}

/// What was written is the salted pair and not the password (D10). Read as
/// text rather than as a struct, because the point is what is *in the file*.
fn credential_is_a_token(dirs: &AppDirs, password: &str) -> bool {
    let Ok(text) = std::fs::read_to_string(dirs.credentials_file()) else {
        return false;
    };
    !text.contains(password) && text.contains("\"salt\"") && text.contains("\"token\"")
}

fn env(name: &str, fallback: &str) -> String {
    std::env::var(name).unwrap_or_else(|_| fallback.to_string())
}
