//! Bridge between the UI thread and asynchronous work.
//!
//! egui runs on the main thread and must never block. A dedicated tokio
//! runtime hosts the music server's client, the sign-in, and artwork
//! fetches; the audio engine runs on a thread of its own beside them. The
//! three talk to the interface through channels, and every event wakes it
//! with `request_repaint`, so the app stays event-driven and idle when
//! nothing is happening.
//!
//! What changed when the server did: there is no browser sign-in, no second
//! application identity to route between, and no remote device to poll.
//! Signing in is a form, the credential is one salted-token pair (D10), and
//! playback state comes from [`crate::engine`] rather than from a request —
//! so the requests that were about Spotify Connect answer nothing at all.
//! `migration/01-api-mapping.md` is the table this file implements.

use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use tokio::sync::mpsc;

use crate::api::NetActivity;
use crate::api::models::*;
use crate::api::subsonic::{
    ApiError, Credentials, NativeClient, NativeError, NativeSession, Report, Scrobbler,
    SubsonicClient, convert,
};
use crate::engine::{Engine, EngineConfig, EngineEvent, LocalState, PlayerCommand, QueueSnapshot};
use crate::images::{ArtLoader, accent_color};
use crate::paths::AppDirs;

pub type ApiResult<T> = Result<T, ApiError>;

/// How many playlist entries a page of the interface holds. The server
/// returns every entry of a playlist in one response, so this is where the
/// list is cut rather than what is asked for.
pub const PLAYLIST_PAGE_SIZE: u32 = 50;

/// How many songs and artists the personalisation shelves ask the native
/// API for.
const PERSONAL_PAGE: u32 = 50;

/// How many records one of Home's album shelves holds.
const HOME_SHELF: u32 = 20;

/// How many of each kind a search asks for. Per type, not in total: the
/// server pages artists, albums and songs separately.
const SEARCH_LIMIT: u32 = 50;

#[derive(Clone, Debug, PartialEq)]
pub enum AuthStatus {
    Starting,
    SignedOut,
    Connecting,
    Connected { username: String },
    Failed(String),
}

/// One of Home's album shelves. All three are `getAlbumList2` with a
/// different `type`, and none of them needs the native API — which is what
/// keeps Home from being empty when the session behind the personalisation
/// shelves has lapsed (D13).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AlbumShelf {
    /// Recently added to the library.
    Newest,
    /// Most played, by the server's own counts.
    Frequent,
    /// A different handful each time Home is loaded.
    Random,
}

/// Which of the two readers of the recently-played endpoint an answer
/// belongs to: the shelf on Home, or the Recents tab in the queue panel.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RecentsFor {
    Home,
    Panel,
}

/// What the sign-in form collected.
///
/// `password` is `None` when the stored credential is being tried again —
/// after an unreachable server at startup, say. The password itself never
/// leaves this struct: it is exchanged once and the salted pair is what is
/// kept (D10).
#[derive(Clone)]
pub struct SignInRequest {
    pub server: String,
    pub username: String,
    pub password: Option<String>,
}

impl std::fmt::Debug for SignInRequest {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("SignInRequest")
            .field("server", &self.server)
            .field("username", &self.username)
            .field("password", &self.password.as_ref().map(|_| "<redacted>"))
            .finish()
    }
}

#[derive(Clone, Debug)]
pub enum ApiRequest {
    Me,
    RecentlyPlayed {
        /// Request owner. Home and Recents use separate generation counters,
        /// so generation alone cannot route the response.
        who: RecentsFor,
        generation: u64,
        /// The offset to read from, as a number in a string: the native API
        /// pages by offset where Spotify paged by timestamp, and the
        /// interface's cursor plumbing is unchanged.
        before: Option<String>,
        limit: u32,
    },
    TopTracks {
        offset: u32,
        full: bool,
        generation: u64,
    },
    TopArtists {
        generation: u64,
    },
    AlbumShelf {
        shelf: AlbumShelf,
        generation: u64,
    },
    MyPlaylists {
        offset: u32,
    },
    Playlist {
        id: String,
        generation: u64,
    },
    PlaylistItems {
        id: String,
        offset: u32,
        generation: u64,
    },
    /// A slice of a playlist read only for who added its songs; the rows
    /// on screen stay untouched.
    PlaylistSample {
        id: String,
        offset: u32,
        generation: u64,
    },
    CreatePlaylist {
        name: String,
        public: bool,
        description: String,
    },
    UpdatePlaylist {
        id: String,
        name: Option<String>,
        description: Option<String>,
        public: Option<bool>,
    },
    AddToPlaylist {
        playlist_id: String,
        playlist_name: String,
        uris: Vec<String>,
    },
    RemoveFromPlaylist {
        playlist_id: String,
        uris: Vec<String>,
        snapshot_id: Option<String>,
    },
    ReorderPlaylist {
        playlist_id: String,
        range_start: u32,
        insert_before: u32,
        snapshot_id: Option<String>,
    },
    DeletePlaylist {
        id: String,
    },
    SavedTracks {
        offset: u32,
    },
    SavedAlbums {
        offset: u32,
    },
    FollowedArtists {
        after: Option<String>,
    },
    SavedShows {
        offset: u32,
    },
    SavedEpisodes {
        offset: u32,
    },
    SetSaved {
        uris: Vec<String>,
        saved: bool,
    },
    Search {
        query: String,
        serial: u64,
    },
    Artist {
        id: String,
    },
    ArtistTopTracks {
        id: String,
    },
    ArtistAlbums {
        id: String,
        groups: String,
        offset: u32,
    },
    RelatedArtists {
        id: String,
    },
    Album {
        id: String,
    },
    AlbumTracks {
        id: String,
        offset: u32,
    },
    Show {
        id: String,
    },
    ShowEpisodes {
        id: String,
        offset: u32,
    },
    Track {
        id: String,
    },
}

impl ApiRequest {
    fn background(&self) -> bool {
        matches!(
            self,
            Self::RecentlyPlayed { .. }
                | Self::TopTracks { .. }
                | Self::TopArtists { .. }
                | Self::AlbumShelf { .. }
                | Self::MyPlaylists { .. }
                | Self::PlaylistSample { .. }
        )
    }
}

#[derive(Debug)]
pub enum ApiResponse {
    Me(ApiResult<User>),
    RecentlyPlayed {
        who: RecentsFor,
        generation: u64,
        limit: u32,
        result: ApiResult<CursorPage<PlayHistory>>,
    },
    TopTracks {
        offset: u32,
        full: bool,
        generation: u64,
        result: ApiResult<Page<Track>>,
    },
    TopArtists {
        generation: u64,
        result: ApiResult<Vec<Artist>>,
    },
    AlbumShelf {
        shelf: AlbumShelf,
        generation: u64,
        result: ApiResult<Vec<Album>>,
    },
    MyPlaylists {
        offset: u32,
        result: ApiResult<Page<Playlist>>,
    },
    Playlist {
        id: String,
        generation: u64,
        result: ApiResult<Playlist>,
    },
    PlaylistItems {
        id: String,
        offset: u32,
        generation: u64,
        result: ApiResult<Page<PlaylistItem>>,
    },
    PlaylistSample {
        id: String,
        generation: u64,
        result: ApiResult<Page<PlaylistItem>>,
    },
    PlaylistCreated(ApiResult<Playlist>),
    PlaylistUpdated {
        id: String,
        result: ApiResult<()>,
    },
    PlaylistItemsChanged {
        id: String,
        message: String,
        result: ApiResult<Option<String>>,
    },
    PlaylistDeleted {
        id: String,
        result: ApiResult<()>,
    },
    SavedTracks {
        offset: u32,
        result: ApiResult<Page<SavedTrack>>,
    },
    SavedAlbums {
        offset: u32,
        result: ApiResult<Page<SavedAlbum>>,
    },
    FollowedArtists {
        after: Option<String>,
        result: ApiResult<CursorPage<Artist>>,
    },
    SavedShows {
        offset: u32,
        result: ApiResult<Page<SavedShow>>,
    },
    SavedEpisodes {
        offset: u32,
        result: ApiResult<Page<SavedEpisode>>,
    },
    SavedChanged {
        uris: Vec<String>,
        saved: bool,
        result: ApiResult<()>,
    },
    Search {
        query: String,
        serial: u64,
        result: ApiResult<SearchResults>,
    },
    Artist {
        id: String,
        result: ApiResult<Artist>,
    },
    ArtistTopTracks {
        id: String,
        result: ApiResult<Vec<Track>>,
    },
    ArtistAlbums {
        id: String,
        groups: String,
        offset: u32,
        result: ApiResult<Page<Album>>,
    },
    RelatedArtists {
        id: String,
        result: ApiResult<Vec<Artist>>,
    },
    Album {
        id: String,
        result: ApiResult<Album>,
    },
    AlbumTracks {
        id: String,
        offset: u32,
        result: ApiResult<Page<Track>>,
    },
    Show {
        id: String,
        result: ApiResult<Show>,
    },
    ShowEpisodes {
        id: String,
        offset: u32,
        result: ApiResult<Page<Episode>>,
    },
    Track {
        id: String,
        result: ApiResult<Track>,
    },
}

pub enum Command {
    /// Sign in to the music server with what the form collected.
    SignIn(Box<SignInRequest>),
    CancelSignIn,
    SignOut,
    /// Reload the engine config (audio settings changed).
    RestartEngine(EngineConfig),
    Player(PlayerCommand),
    Api(ApiRequest),
    Accent {
        url: String,
    },
    Shutdown,
    /// Internal: the sign-in finished, one way or the other.
    SignedIn(Box<SignInOutcome>),
    /// Internal: what the scrobbler decided to tell the server, computed on
    /// the audio thread and sent because the calls belong on the runtime.
    Scrobble(Vec<Report>),
    /// Ask GitHub whether a newer release exists.
    CheckForUpdates,
    /// The words of a track, from the server and then from LRCLIB.
    Lyrics(Box<LyricsRequest>),
    /// Read a playlist's cached items from disk.
    LoadPlaylistCache {
        id: String,
    },
    /// Remember a fully loaded playlist on disk under its snapshot.
    StorePlaylistCache {
        id: String,
        snapshot: String,
        items: Vec<PlaylistItem>,
    },
}

/// What a sign-in attempt produced, on its way back to the command loop.
pub struct SignInOutcome {
    pub credentials: Credentials,
    /// Absent on a server that is not Navidrome, or one that refused the
    /// native sign-in. The personalisation shelves are empty then, which is
    /// D11's normal case rather than an error.
    pub session: Option<NativeSession>,
    pub user: User,
    pub error: Option<String>,
}

pub struct LyricsRequest {
    /// The track the answer is for, so a stale one is ignored.
    pub uri: String,
    pub query: crate::lyrics::Query,
}

pub enum Event {
    Auth(AuthStatus),
    Playback(LocalPlayback),
    Local(Box<LocalState>),
    /// What plays next, as the engine holds it. With no Connect there is no
    /// web API to ask, so this is the only source (P3.3).
    Queue(Box<QueueSnapshot>),
    Api(Box<ApiResponse>),
    Accent {
        url: String,
        color: [u8; 3],
    },
    Error(String),
    /// A newer release than this build exists.
    UpdateAvailable {
        version: String,
        url: String,
    },
    /// Track lyrics, or `None` when unavailable.
    Lyrics {
        uri: String,
        result: Result<Option<crate::lyrics::Lyrics>, String>,
    },
    /// A playlist's items as last cached, with the snapshot they belong to.
    PlaylistCache {
        account_id: String,
        id: String,
        snapshot: String,
        items: Vec<PlaylistItem>,
    },
    /// Who the app is signed in as, or was last signed in as. The form
    /// shows the first two, so a failed start comes back filled in rather
    /// than empty; `account` is which directory on disk this session's
    /// cached playlists belong to.
    KnownServer {
        server: String,
        username: String,
        account: String,
    },
}

/// The state of playback on this computer.
///
/// It follows the credential now: an engine that has a server to stream from
/// is ready, and one that does not is unavailable. There is nothing to
/// authorize separately and nothing to connect to.
#[derive(Clone, Debug, PartialEq)]
pub enum LocalPlayback {
    Unavailable,
    Connecting,
    Ready,
    Failed(String),
}

/// Wakes whichever window currently exists, if any.
///
/// Background services (the runtime, MPRIS, the tray) outlive individual
/// windows: the window is destroyed when it closes to the tray and created
/// again on demand. They therefore hold this handle instead of an
/// `egui::Context`.
#[derive(Clone, Default)]
pub struct Waker(Arc<std::sync::Mutex<Option<egui::Context>>>);

impl Waker {
    pub fn attach(&self, ctx: &egui::Context) {
        *self.0.lock().unwrap_or_else(|p| p.into_inner()) = Some(ctx.clone());
    }

    pub fn detach(&self) {
        *self.0.lock().unwrap_or_else(|p| p.into_inner()) = None;
    }

    pub fn wake(&self) {
        if let Some(ctx) = self.0.lock().unwrap_or_else(|p| p.into_inner()).as_ref() {
            ctx.request_repaint();
        }
    }
}

/// The interface's handle to the runtime.
pub struct Backend {
    commands: mpsc::UnboundedSender<Command>,
    events: std::sync::mpsc::Receiver<Event>,
    art: ArtLoader,
    activity: Arc<NetActivity>,
    thread: Option<std::thread::JoinHandle<()>>,
    offline: bool,
    /// What the interface asked the engine to do, kept for the queue tests
    /// in `src/app.rs`: the queue is the engine's since P3.3, so what the
    /// app owes those rules is the right command — and in a test there is
    /// no engine on the other end to observe it arriving.
    #[cfg(test)]
    asked: Mutex<Vec<PlayerCommand>>,
}

impl Backend {
    pub fn spawn(dirs: AppDirs, engine_config: EngineConfig, waker: Waker) -> Self {
        let (command_tx, command_rx) = mpsc::unbounded_channel();
        let (event_tx, event_rx) = std::sync::mpsc::channel();
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .thread_name("fastsonic-runtime")
            .enable_all()
            .build()
            .expect("unable to start the async runtime");
        let http = crate::http_client_builder()
            .user_agent(concat!("fastsonic/", env!("CARGO_PKG_VERSION")))
            .timeout(Duration::from_secs(30))
            .build()
            .expect("unable to build the HTTP client");
        let art = ArtLoader::new(http.clone(), runtime.handle().clone(), dirs.art_cache_dir());
        let activity = Arc::new(NetActivity::default());

        let worker_activity = Arc::clone(&activity);
        let worker_art = art.clone();
        let worker_commands = command_tx.clone();
        let thread = std::thread::Builder::new()
            .name("fastsonic-backend".to_string())
            .spawn(move || {
                runtime.block_on(async move {
                    let mut worker = Worker::new(
                        dirs,
                        engine_config,
                        http,
                        worker_art,
                        worker_activity,
                        event_tx,
                        worker_commands,
                        waker,
                    );
                    worker.run(command_rx).await;
                });
                // Give the audio thread a moment to release the device.
                runtime.shutdown_timeout(Duration::from_secs(2));
            })
            .expect("unable to start the backend thread");

        Self {
            commands: command_tx,
            events: event_rx,
            art,
            activity,
            thread: Some(thread),
            offline: false,
            #[cfg(test)]
            asked: Mutex::new(Vec::new()),
        }
    }

    /// Live network activity, for the interface's busy indicator.
    pub fn activity(&self) -> &NetActivity {
        &self.activity
    }

    /// Stops server-bound commands from leaving the process; artwork and
    /// shutdown still work. Used by the demo mode and by headless tests.
    #[cfg_attr(not(any(test, feature = "demo")), allow(dead_code))]
    pub fn set_offline(&mut self, offline: bool) {
        self.offline = offline;
    }

    pub fn send(&self, command: Command) {
        if self.offline && !matches!(command, Command::Accent { .. } | Command::Shutdown) {
            return;
        }
        let _ = self.commands.send(command);
    }

    pub fn api(&self, request: ApiRequest) {
        self.send(Command::Api(request));
    }

    pub fn player(&self, command: PlayerCommand) {
        #[cfg(test)]
        self.asked
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .push(command.clone());
        self.send(Command::Player(command));
    }

    /// Every player command sent since the last call, oldest first.
    #[cfg(test)]
    pub fn asked(&self) -> Vec<PlayerCommand> {
        std::mem::take(
            &mut *self
                .asked
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner()),
        )
    }

    pub fn poll(&self) -> Vec<Event> {
        self.events.try_iter().collect()
    }

    pub fn art(&self) -> &ArtLoader {
        &self.art
    }

    pub fn shutdown(&mut self) {
        self.send(Command::Shutdown);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

struct Worker {
    dirs: AppDirs,
    engine_config: EngineConfig,
    http: reqwest::Client,
    client: Arc<SubsonicClient>,
    native: Arc<NativeClient>,
    background_api: Arc<tokio::sync::Semaphore>,
    art: ArtLoader,
    events: std::sync::mpsc::Sender<Event>,
    commands: mpsc::UnboundedSender<Command>,
    waker: Waker,
    engine: Option<Arc<Engine>>,
    /// Shared with the audio thread, which is where a state change is first
    /// seen. It only decides; the calls it asks for are made here.
    scrobbler: Arc<Mutex<Scrobbler>>,
    signing_in: bool,
    account: Option<String>,
}

impl Worker {
    #[allow(clippy::too_many_arguments)]
    fn new(
        dirs: AppDirs,
        engine_config: EngineConfig,
        http: reqwest::Client,
        art: ArtLoader,
        activity: Arc<NetActivity>,
        events: std::sync::mpsc::Sender<Event>,
        commands: mpsc::UnboundedSender<Command>,
        waker: Waker,
    ) -> Self {
        Self {
            dirs,
            engine_config,
            client: Arc::new(SubsonicClient::new(
                http.clone(),
                Arc::clone(&activity),
                SEARCH_LIMIT,
            )),
            native: Arc::new(NativeClient::new(http.clone(), activity)),
            background_api: Arc::new(tokio::sync::Semaphore::new(4)),
            http,
            art,
            events,
            commands,
            waker,
            engine: None,
            scrobbler: Arc::new(Mutex::new(Scrobbler::new())),
            signing_in: false,
            account: None,
        }
    }

    fn emit(&self, event: Event) {
        let _ = self.events.send(event);
        self.waker.wake();
    }

    async fn run(&mut self, mut commands: mpsc::UnboundedReceiver<Command>) {
        self.restore_session();
        while let Some(command) = commands.recv().await {
            match command {
                Command::Shutdown => break,
                Command::SignIn(request) => self.sign_in(*request),
                Command::CancelSignIn => self.signing_in = false,
                Command::SignOut => self.sign_out(),
                Command::RestartEngine(config) => {
                    self.engine_config = config;
                    self.restart_engine();
                }
                Command::Player(command) => match &self.engine {
                    Some(engine) => {
                        if let Err(error) = engine.command(command) {
                            self.emit(Event::Error(format!("Playback error: {error}")));
                        }
                    }
                    None => self.emit(Event::Error(
                        "Sign in to your music server before playing anything".into(),
                    )),
                },
                Command::Api(request) => self.dispatch(request),
                Command::Accent { url } => self.accent(url),
                Command::SignedIn(outcome) => self.on_signed_in(*outcome),
                Command::Scrobble(reports) => self.scrobble(reports),
                Command::CheckForUpdates => self.check_for_updates(),
                Command::Lyrics(request) => self.fetch_lyrics(*request),
                Command::LoadPlaylistCache { id } => self.load_playlist_cache(id),
                Command::StorePlaylistCache {
                    id,
                    snapshot,
                    items,
                } => self.store_playlist_cache(id, snapshot, items),
            }
        }
        if let Some(engine) = self.engine.take() {
            engine.shutdown();
        }
    }

    // ---- sign-in ----------------------------------------------------------

    /// Comes back to the server the app was last signed in to. The stored
    /// pair authenticates indefinitely, so this is a `ping` rather than a
    /// sign-in — but the native session may have run out while the app was
    /// closed, and then the personalisation shelves are empty until the
    /// password is typed again (D13).
    fn restore_session(&mut self) {
        let Some(stored) = StoredSession::load(&self.dirs.credentials_file()) else {
            self.emit(Event::Auth(AuthStatus::SignedOut));
            return;
        };
        self.account = Some(account_key(&stored.credentials));
        self.emit(Event::KnownServer {
            server: stored.credentials.server.clone(),
            username: stored.credentials.username.clone(),
            account: account_key(&stored.credentials),
        });
        self.emit(Event::Auth(AuthStatus::Connecting));
        self.adopt(&stored);
        let client = Arc::clone(&self.client);
        let commands = self.commands.clone();
        let events = self.events.clone();
        let waker = self.waker.clone();
        self.signing_in = true;
        tokio::spawn(async move {
            match client.me().await {
                Ok(user) => {
                    let _ = commands.send(Command::SignedIn(Box::new(SignInOutcome {
                        credentials: client.credentials().unwrap_or_default(),
                        session: None,
                        user,
                        error: None,
                    })));
                }
                Err(error) => {
                    let _ = events.send(Event::Auth(AuthStatus::Failed(error.to_string())));
                    waker.wake();
                    let _ = commands.send(Command::CancelSignIn);
                }
            }
        });
    }

    /// Points everything that needs a credential at the same one.
    fn adopt(&self, stored: &StoredSession) {
        self.client
            .set_credentials(Some(stored.credentials.clone()));
        self.art.set_credentials(Some(stored.credentials.clone()));
        self.native.set_session(stored.session.clone());
    }

    /// Exchanges the password for the pair that is kept, or tries the stored
    /// pair again when there is no password to exchange.
    ///
    /// A server that has no `/auth/login` is not a failure: Gonic and the
    /// other Subsonic servers do not have Navidrome's native API, and the
    /// salted pair can be derived here instead. What is lost is the
    /// personalisation the native API answers (D7, D11).
    fn sign_in(&mut self, request: SignInRequest) {
        if self.signing_in {
            return;
        }
        if request.server.trim().is_empty() || request.username.trim().is_empty() {
            self.emit(Event::Auth(AuthStatus::Failed(
                "A server address and a username are needed".into(),
            )));
            return;
        }
        self.signing_in = true;
        self.emit(Event::Auth(AuthStatus::Connecting));
        let stored = (request.password.is_none())
            .then(|| StoredSession::load(&self.dirs.credentials_file()))
            .flatten();
        let http = self.http.clone();
        let activity = self.client.activity();
        let client = Arc::new(SubsonicClient::new(
            http.clone(),
            activity,
            PLAYLIST_PAGE_SIZE,
        ));
        let native = Arc::clone(&self.native);
        let commands = self.commands.clone();
        let events = self.events.clone();
        let waker = self.waker.clone();
        tokio::spawn(async move {
            let (credentials, session, mut error) = match &request.password {
                Some(password) => match native
                    .sign_in(&request.server, &request.username, password)
                    .await
                {
                    Ok(signed) => (signed.credentials, Some(signed.session), None),
                    // Not Navidrome, so there is no native API — but the
                    // Subsonic half of the app works on any server that
                    // speaks the protocol, so carry on with a pair derived
                    // here and say what will be missing.
                    Err(NativeError::NotNavidrome) => (
                        Credentials::from_password(&request.server, &request.username, password),
                        None,
                        Some(
                            "This server has no listening history to read, so Home's \
                             personal shelves stay empty."
                                .to_string(),
                        ),
                    ),
                    Err(error) => {
                        let _ = events.send(Event::Auth(AuthStatus::Failed(error.to_string())));
                        waker.wake();
                        let _ = commands.send(Command::CancelSignIn);
                        return;
                    }
                },
                None => match stored {
                    Some(stored) => (stored.credentials, stored.session, None),
                    None => {
                        let _ = events.send(Event::Auth(AuthStatus::Failed(
                            "Enter the password for this server".into(),
                        )));
                        waker.wake();
                        let _ = commands.send(Command::CancelSignIn);
                        return;
                    }
                },
            };
            client.set_credentials(Some(credentials.clone()));
            match client.me().await {
                Ok(user) => {
                    let _ = commands.send(Command::SignedIn(Box::new(SignInOutcome {
                        credentials,
                        session,
                        user,
                        error: error.take(),
                    })));
                }
                Err(failure) => {
                    let _ = events.send(Event::Auth(AuthStatus::Failed(failure.to_string())));
                    waker.wake();
                    let _ = commands.send(Command::CancelSignIn);
                }
            }
        });
    }

    fn on_signed_in(&mut self, outcome: SignInOutcome) {
        // A sign-in that was cancelled while it was in flight must not
        // arrive anyway: `Command::CancelSignIn` is what clears this.
        if !self.signing_in {
            return;
        }
        self.signing_in = false;
        let stored = StoredSession {
            credentials: outcome.credentials,
            session: outcome.session,
        };
        if let Err(error) = stored.save(&self.dirs.credentials_file()) {
            log::warn!("unable to remember the sign-in: {error}");
        }
        self.adopt(&stored);
        self.account = Some(account_key(&stored.credentials));
        let username = outcome
            .user
            .display_name
            .clone()
            .filter(|name| !name.is_empty())
            .unwrap_or_else(|| stored.credentials.username.clone());
        self.emit(Event::KnownServer {
            server: stored.credentials.server.clone(),
            username: stored.credentials.username.clone(),
            account: account_key(&stored.credentials),
        });
        self.emit(Event::Auth(AuthStatus::Connected { username }));
        self.emit(Event::Api(Box::new(ApiResponse::Me(Ok(outcome.user)))));
        if let Some(message) = outcome.error {
            self.emit(Event::Error(message));
        }
        self.start_engine();
    }

    fn sign_out(&mut self) {
        if let Some(engine) = self.engine.take() {
            engine.shutdown();
        }
        self.signing_in = false;
        self.account = None;
        self.client.set_credentials(None);
        self.native.set_session(None);
        self.art.set_credentials(None);
        StoredSession::remove(&self.dirs.credentials_file());
        self.emit(Event::Playback(LocalPlayback::Unavailable));
        self.emit(Event::Auth(AuthStatus::SignedOut));
    }

    // ---- the audio engine -------------------------------------------------

    /// The engine's side of the channel. It runs on the audio thread, so
    /// nothing here may block: the scrobbler decides in memory and the calls
    /// it asks for go back to the runtime as a command.
    fn engine_notify(&self) -> crate::engine::Notify {
        let events = self.events.clone();
        let commands = self.commands.clone();
        let waker = self.waker.clone();
        let scrobbler = Arc::clone(&self.scrobbler);
        Arc::new(move |event| match event {
            EngineEvent::State(state) => {
                let reports = observe(&scrobbler, &state);
                if !reports.is_empty() {
                    let _ = commands.send(Command::Scrobble(reports));
                }
                let _ = events.send(Event::Local(Box::new(state)));
                waker.wake();
            }
            EngineEvent::Queue(queue) => {
                let _ = events.send(Event::Queue(Box::new(queue)));
                waker.wake();
            }
        })
    }

    /// Brings the engine up against the credential the client is holding.
    fn start_engine(&mut self) {
        if self.engine.is_some() {
            return;
        }
        if self.client.credentials().is_none() {
            self.emit(Event::Playback(LocalPlayback::Unavailable));
            return;
        }
        self.emit(Event::Playback(LocalPlayback::Connecting));
        match Engine::start(
            &self.engine_config,
            Arc::clone(&self.client),
            tokio::runtime::Handle::current(),
            self.engine_notify(),
        ) {
            Ok(engine) => {
                self.engine = Some(Arc::new(engine));
                self.emit(Event::Playback(LocalPlayback::Ready));
            }
            Err(error) => {
                log::error!("unable to start local playback: {error:#}");
                self.emit(Event::Playback(LocalPlayback::Failed(format!("{error:#}"))));
            }
        }
    }

    /// Replaces the engine after an audio setting changed. Whatever was
    /// playing is picked up again at the same spot on the new one, and the
    /// queue comes across with it.
    ///
    /// The queue lives in the engine (P3.3), so the old one would take it
    /// away: rule 9 of `docs/_reference/queue.md` is what says it may not.
    /// `carry_over` writes it down as a load — the album by name, the songs
    /// queued by hand, and the place the album keeps under them.
    fn restart_engine(&mut self) {
        let before = self.engine.as_ref().map(|engine| engine.state());
        let carried = self
            .engine
            .as_ref()
            .zip(before.as_ref())
            .and_then(|(engine, state)| crate::engine::state::carry_over(state, &engine.queue()));
        if let Some(engine) = self.engine.take() {
            engine.shutdown();
        }
        self.start_engine();
        let (Some(engine), Some(spec)) = (self.engine.as_ref(), carried) else {
            return;
        };
        if let Err(error) = engine.command(PlayerCommand::Load(spec)) {
            log::warn!("unable to pick playback up again: {error}");
            return;
        }
        // Shuffle travels with the load; repeat has nothing to do with what
        // plays now, so it follows behind it.
        if let Some(repeat) = before
            .map(|state| state.repeat)
            .filter(|repeat| *repeat != crate::engine::RepeatMode::Off)
            && let Err(error) = engine.command(PlayerCommand::Repeat(repeat))
        {
            log::warn!("unable to set repeat again: {error}");
        }
    }

    /// Sends what the scrobbler asked for. Nothing here may interrupt
    /// playback, so a failure is a log line and not an error on screen.
    fn scrobble(&self, reports: Vec<Report>) {
        let client = Arc::clone(&self.client);
        tokio::spawn(async move {
            for report in reports {
                let outcome = match &report {
                    Report::NowPlaying { id } => client.scrobble(id, None, false).await,
                    Report::Played { id, started_at_ms } => {
                        client.scrobble(id, Some(*started_at_ms), true).await
                    }
                };
                if let Err(error) = outcome {
                    log::debug!("the server did not take a play report: {error}");
                }
            }
        });
    }

    // ---- odds and ends ----------------------------------------------------

    fn check_for_updates(&self) {
        let http = self.http.clone();
        let events = self.events.clone();
        let waker = self.waker.clone();
        tokio::spawn(async move {
            match crate::updates::newer_release(&http).await {
                Ok(Some(release)) => {
                    let _ = events.send(Event::UpdateAvailable {
                        version: release.version,
                        url: release.url,
                    });
                    waker.wake();
                }
                Ok(None) => log::debug!("this is the newest release"),
                Err(error) => log::debug!("could not check for a newer release: {error:#}"),
            }
        });
    }

    /// The server's own words first — Navidrome reads the `.lrc` sidecar and
    /// answers with per-line timings — then LRCLIB, which is unchanged.
    fn fetch_lyrics(&self, request: LyricsRequest) {
        let http = self.http.clone();
        let events = self.events.clone();
        let waker = self.waker.clone();
        let cache_dir = self.dirs.lyrics_cache_dir();
        let client = Arc::clone(&self.client);
        tokio::spawn(async move {
            let result = match server_lyrics(&client, &request.uri, &cache_dir).await {
                Some(found) => Ok(Some(found)),
                None => crate::lyrics::fetch(&http, &cache_dir, &request.query)
                    .await
                    .map_err(|error| format!("{error:#}")),
            };
            let _ = events.send(Event::Lyrics {
                uri: request.uri,
                result,
            });
            waker.wake();
        });
    }

    /// Loads cached playlist items. The UI compares the cached snapshot with
    /// the live playlist before using them.
    fn load_playlist_cache(&self, id: String) {
        let Some(account) = self.account.clone() else {
            return;
        };
        let events = self.events.clone();
        let waker = self.waker.clone();
        let path = self
            .dirs
            .account_playlist_cache_dir(&account)
            .join(format!("{id}.json"));
        tokio::spawn(async move {
            let Ok(text) = tokio::fs::read_to_string(&path).await else {
                return;
            };
            let Ok(cached) = serde_json::from_str::<CachedPlaylist>(&text) else {
                return;
            };
            let _ = events.send(Event::PlaylistCache {
                account_id: account,
                id,
                snapshot: cached.snapshot,
                items: cached.items,
            });
            waker.wake();
        });
    }

    fn store_playlist_cache(&self, id: String, snapshot: String, items: Vec<PlaylistItem>) {
        let Some(account) = self.account.clone() else {
            return;
        };
        let path = self
            .dirs
            .account_playlist_cache_dir(&account)
            .join(format!("{id}.json"));
        tokio::spawn(async move {
            if let Some(parent) = path.parent() {
                let _ = tokio::fs::create_dir_all(parent).await;
            }
            if let Ok(text) = serde_json::to_string(&CachedPlaylist { snapshot, items }) {
                let temporary = path.with_extension("json.tmp");
                if tokio::fs::write(&temporary, text).await.is_ok() {
                    let _ = tokio::fs::rename(temporary, path).await;
                }
            }
        });
    }

    // ---- api ----------------------------------------------------------------

    fn dispatch(&self, request: ApiRequest) {
        let client = Arc::clone(&self.client);
        let native = Arc::clone(&self.native);
        let background_api = Arc::clone(&self.background_api);
        let background = request.background();
        let events = self.events.clone();
        let waker = self.waker.clone();
        tokio::spawn(async move {
            let _background_permit = if background {
                background_api.acquire_owned().await.ok()
            } else {
                None
            };
            let Some(response) = handle(&client, &native, request).await else {
                return;
            };
            if let ApiResponse::Me(Err(error)) = &response
                && error.is_auth()
            {
                let _ = events.send(Event::Auth(AuthStatus::Failed(
                    "The server no longer accepts this sign-in. Sign in again.".into(),
                )));
            }
            let _ = events.send(Event::Api(Box::new(response)));
            waker.wake();
        });
    }

    fn accent(&self, url: String) {
        let art = self.art.clone();
        let events = self.events.clone();
        let waker = self.waker.clone();
        tokio::spawn(async move {
            if let Ok(bytes) = art.fetch(&url).await {
                let color = tokio::task::spawn_blocking(move || accent_color(&bytes))
                    .await
                    .ok()
                    .flatten();
                if let Some(color) = color {
                    let _ = events.send(Event::Accent { url, color });
                    waker.wake();
                }
            }
        });
    }
}

/// Runs the scrobbler over one state snapshot. Separate from the closure so
/// that the rule — and the fact that a track with no id reports nothing —
/// can be tested without an engine.
fn observe(scrobbler: &Mutex<Scrobbler>, state: &LocalState) -> Vec<Report> {
    let song = state
        .track
        .as_ref()
        .and_then(|track| convert::id_of(&track.uri, convert::Kind::Track));
    let position = Duration::from_millis(u64::from(state.position_now()));
    let duration = state.track.as_ref().map_or(Duration::ZERO, |track| {
        Duration::from_millis(u64::from(track.duration_ms))
    });
    let playing = state.playback == crate::engine::Playback::Playing;
    let now_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64;
    scrobbler
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .observe(song, position, duration, playing, now_ms)
}

/// One request, one answer — or none at all.
///
/// `None` is what the cut requests get: podcasts and
/// recommendations have no call to make and no honest answer to give, so
/// they leave the interface holding what it already had rather than being
/// told an empty list is the truth. `migration/03-removals.md` is where
/// their askers go in Phase 5.
async fn handle(
    client: &SubsonicClient,
    native: &NativeClient,
    request: ApiRequest,
) -> Option<ApiResponse> {
    let response = match request {
        ApiRequest::Me => ApiResponse::Me(client.me().await),

        ApiRequest::RecentlyPlayed {
            who,
            generation,
            before,
            limit,
        } => {
            let offset = before
                .as_deref()
                .and_then(|at| at.parse().ok())
                .unwrap_or(0);
            let result = native
                .recently_played(offset, limit)
                .await
                .map(|tracks| history_page(tracks, offset, limit))
                .or_else(empty_cursor_page);
            ApiResponse::RecentlyPlayed {
                who,
                generation,
                limit,
                result,
            }
        }
        ApiRequest::TopTracks {
            offset,
            full,
            generation,
        } => {
            let limit = if full { PERSONAL_PAGE } else { 20 };
            let result = native
                .top_tracks(offset, limit)
                .await
                .map(|tracks| convert::page(tracks, offset, limit))
                .or_else(|error| empty_page(error).map(|()| Page::default()));
            ApiResponse::TopTracks {
                offset,
                full,
                generation,
                result,
            }
        }
        ApiRequest::TopArtists { generation } => ApiResponse::TopArtists {
            generation,
            result: native
                .top_artists(0, 20)
                .await
                .or_else(|error| empty_page(error).map(|()| Vec::new())),
        },

        ApiRequest::AlbumShelf { shelf, generation } => ApiResponse::AlbumShelf {
            result: match shelf {
                AlbumShelf::Newest => client.newest_albums(HOME_SHELF).await,
                AlbumShelf::Frequent => client.frequent_albums(HOME_SHELF).await,
                AlbumShelf::Random => client.random_albums(HOME_SHELF).await,
            },
            shelf,
            generation,
        },
        ApiRequest::MyPlaylists { offset } => ApiResponse::MyPlaylists {
            offset,
            result: client.my_playlists(offset, PLAYLIST_PAGE_SIZE).await,
        },
        ApiRequest::Playlist { id, generation } => ApiResponse::Playlist {
            result: client.playlist(&id).await,
            id,
            generation,
        },
        ApiRequest::PlaylistItems {
            id,
            offset,
            generation,
        } => ApiResponse::PlaylistItems {
            result: client.playlist_items(&id, offset, PLAYLIST_PAGE_SIZE).await,
            id,
            offset,
            generation,
        },
        ApiRequest::PlaylistSample {
            id,
            offset,
            generation,
        } => ApiResponse::PlaylistSample {
            result: client.playlist_items(&id, offset, PLAYLIST_PAGE_SIZE).await,
            id,
            generation,
        },
        ApiRequest::CreatePlaylist {
            name,
            public,
            description,
        } => {
            ApiResponse::PlaylistCreated(client.create_playlist(&name, public, &description).await)
        }
        ApiRequest::DeletePlaylist { id } => ApiResponse::PlaylistDeleted {
            result: client.delete_playlist(&id).await,
            id,
        },
        ApiRequest::UpdatePlaylist {
            id,
            name,
            description,
            public,
        } => ApiResponse::PlaylistUpdated {
            result: client
                .update_playlist(&id, name.as_deref(), description.as_deref(), public)
                .await,
            id,
        },
        ApiRequest::AddToPlaylist {
            playlist_id,
            playlist_name,
            uris,
        } => ApiResponse::PlaylistItemsChanged {
            result: client
                .add_to_playlist(&playlist_id, &uris)
                .await
                .map(|()| None),
            id: playlist_id,
            message: format!("Added to {playlist_name}"),
        },
        ApiRequest::RemoveFromPlaylist {
            playlist_id, uris, ..
        } => ApiResponse::PlaylistItemsChanged {
            result: client
                .remove_from_playlist(&playlist_id, &uris)
                .await
                .map(|playlist| playlist.snapshot_id),
            id: playlist_id,
            message: "Removed from playlist".to_string(),
        },
        ApiRequest::ReorderPlaylist {
            playlist_id,
            range_start,
            insert_before,
            ..
        } => ApiResponse::PlaylistItemsChanged {
            result: client
                .reorder_playlist(&playlist_id, range_start, insert_before)
                .await
                .map(|playlist| playlist.snapshot_id),
            id: playlist_id,
            message: String::new(),
        },

        ApiRequest::SavedTracks { offset } => ApiResponse::SavedTracks {
            offset,
            result: client
                .saved_tracks(offset, PLAYLIST_PAGE_SIZE)
                .await
                .map(|page| {
                    page.map(|track| SavedTrack {
                        added_at: None,
                        track,
                    })
                }),
        },
        ApiRequest::SavedAlbums { offset } => ApiResponse::SavedAlbums {
            offset,
            result: client
                .saved_albums(offset, PLAYLIST_PAGE_SIZE)
                .await
                .map(|page| {
                    page.map(|album| SavedAlbum {
                        added_at: None,
                        album,
                    })
                }),
        },
        ApiRequest::FollowedArtists { after } => {
            let offset = after.as_deref().and_then(|at| at.parse().ok()).unwrap_or(0);
            ApiResponse::FollowedArtists {
                result: client
                    .saved_artists(offset, PLAYLIST_PAGE_SIZE)
                    .await
                    .map(|page| CursorPage {
                        cursors: page.next_offset().map(|next| Cursors {
                            after: Some(next.to_string()),
                            before: None,
                        }),
                        next: page.next_offset().map(|next| next.to_string()),
                        total: Some(page.total),
                        items: page.items,
                    }),
                after,
            }
        }
        ApiRequest::SetSaved { uris, saved } => ApiResponse::SavedChanged {
            result: client.set_saved(&uris, saved).await,
            uris,
            saved,
        },

        ApiRequest::Search { query, serial } => ApiResponse::Search {
            result: client.search(&query, 0).await,
            query,
            serial,
        },
        ApiRequest::Artist { id } => ApiResponse::Artist {
            result: client.artist(&id).await,
            id,
        },
        ApiRequest::ArtistTopTracks { id } => ApiResponse::ArtistTopTracks {
            result: client.artist_top_tracks(&id, 10).await,
            id,
        },
        ApiRequest::ArtistAlbums { id, groups, offset } => ApiResponse::ArtistAlbums {
            result: client.artist_albums(&id, offset, PLAYLIST_PAGE_SIZE).await,
            id,
            groups,
            offset,
        },
        ApiRequest::RelatedArtists { id } => ApiResponse::RelatedArtists {
            result: client.related_artists(&id, 20).await,
            id,
        },
        ApiRequest::Album { id } => ApiResponse::Album {
            result: client.album(&id).await,
            id,
        },
        ApiRequest::AlbumTracks { id, offset } => ApiResponse::AlbumTracks {
            result: client.album_tracks(&id, offset, PLAYLIST_PAGE_SIZE).await,
            id,
            offset,
        },
        ApiRequest::Track { id } => ApiResponse::Track {
            result: client.track(&id).await,
            id,
        },

        // Cut, and answered by saying nothing.
        ApiRequest::SavedShows { .. }
        | ApiRequest::SavedEpisodes { .. }
        | ApiRequest::Show { .. }
        | ApiRequest::ShowEpisodes { .. } => return None,
    };
    Some(response)
}

/// The Recents shelf, in the shape the interface reads. There are no
/// timestamps on the native API's answer beyond the order it came in, so
/// `played_at` is empty and the order is the fact.
fn history_page(tracks: Vec<Track>, offset: u32, limit: u32) -> CursorPage<PlayHistory> {
    let more = tracks.len() as u32 == limit;
    let next = more.then(|| (offset + limit).to_string());
    CursorPage {
        items: tracks
            .into_iter()
            .map(|track| PlayHistory {
                track,
                played_at: None,
                context: None,
            })
            .collect(),
        total: None,
        next: next.clone(),
        cursors: next.map(|before| Cursors {
            after: None,
            before: Some(before),
        }),
    }
}

/// A native-API section that this server cannot answer is empty, not
/// broken (D11, D13). Anything else is a real failure and is reported.
fn empty_page<T: Default>(error: NativeError) -> Result<T, ApiError> {
    if error.is_unavailable() {
        return Ok(T::default());
    }
    Err(ApiError::Server {
        code: 0,
        message: error.to_string(),
    })
}

fn empty_cursor_page<T>(error: NativeError) -> Result<CursorPage<T>, ApiError> {
    if error.is_unavailable() {
        return Ok(CursorPage {
            items: Vec::new(),
            total: None,
            next: None,
            cursors: None,
        });
    }
    Err(ApiError::Server {
        code: 0,
        message: error.to_string(),
    })
}

/// The server's own words, cached like LRCLIB's, "none" included; `None`
/// falls back to LRCLIB.
async fn server_lyrics(
    client: &SubsonicClient,
    uri: &str,
    cache_dir: &std::path::Path,
) -> Option<crate::lyrics::Lyrics> {
    let id = convert::id_of(uri, convert::Kind::Track)?;
    let path = cache_dir.join(format!("server-{id}.json"));
    if let Some(cached) = crate::lyrics::cached(&path) {
        return cached;
    }
    match client.lyrics(id).await {
        Ok(list) => {
            let found = crate::lyrics::from_subsonic(&list);
            crate::lyrics::store(&path, &found);
            found
        }
        Err(error) => {
            log::debug!("the server has no words for this song: {error}");
            None
        }
    }
}

/// Which account's playlist cache to read: one server, one user, one
/// directory. The credential is not in it — the salt would change the
/// directory on every re-derivation, and it has no business on that path.
fn account_key(credentials: &Credentials) -> String {
    let host = credentials
        .server
        .rsplit('/')
        .next()
        .unwrap_or(&credentials.server);
    let key = format!("{}@{host}", credentials.username);
    key.chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | '.' | '@') {
                character
            } else {
                '-'
            }
        })
        .collect()
}

/// What is kept between runs so that starting the app is not signing in.
#[derive(Debug, Default, serde::Deserialize, serde::Serialize)]
struct StoredSession {
    credentials: Credentials,
    /// Navidrome's own session, which expires where the pair above does not
    /// (D13). Kept because a JWT with hours left on it saves the
    /// personalisation shelves from being empty for no reason.
    #[serde(default)]
    session: Option<NativeSession>,
}

impl StoredSession {
    fn load(path: &std::path::Path) -> Option<Self> {
        let text = std::fs::read_to_string(path).ok()?;
        let stored: Self = serde_json::from_str(&text).ok()?;
        (!stored.credentials.is_empty()).then_some(stored)
    }

    /// Written whole and moved into place, so an interrupted write cannot
    /// leave half a credential behind.
    fn save(&self, path: &std::path::Path) -> std::io::Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let text = serde_json::to_string(self)?;
        let temporary = path.with_extension("json.tmp");
        std::fs::write(&temporary, text)?;
        std::fs::rename(temporary, path)
    }

    fn remove(path: &std::path::Path) {
        let _ = std::fs::remove_file(path);
    }
}

/// A playlist's items on disk, valid for exactly one snapshot.
#[derive(serde::Serialize, serde::Deserialize)]
struct CachedPlaylist {
    snapshot: String,
    items: Vec<PlaylistItem>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::{LocalTrack, Playback};

    fn playing(uri: &str, position_ms: u32) -> LocalState {
        LocalState {
            playback: Playback::Playing,
            track: Some(LocalTrack {
                uri: uri.to_string(),
                duration_ms: 240_000,
                ..LocalTrack::default()
            }),
            position_ms,
            ..LocalState::default()
        }
    }

    /// A song announces itself as soon as it starts, so other clients see it
    /// in `getNowPlaying` under this app's name.
    #[test]
    fn a_started_song_is_announced() {
        let scrobbler = Mutex::new(Scrobbler::new());
        let reports = observe(&scrobbler, &playing("sonic:track:7", 0));
        assert_eq!(
            reports,
            vec![Report::NowPlaying {
                id: "7".to_string()
            }]
        );
    }

    /// Nothing is reported for a state with no song in it, and the next song
    /// that does arrive is announced rather than being taken for the same
    /// one continuing.
    #[test]
    fn a_stopped_player_reports_nothing() {
        let scrobbler = Mutex::new(Scrobbler::new());
        assert!(observe(&scrobbler, &LocalState::default()).is_empty());
        assert!(!observe(&scrobbler, &playing("sonic:track:7", 0)).is_empty());
    }

    /// A URI the server did not give us has no song id in it, so there is
    /// nothing to report and nothing that could be reported wrongly.
    #[test]
    fn an_unrecognised_uri_reports_nothing() {
        let scrobbler = Mutex::new(Scrobbler::new());
        assert!(observe(&scrobbler, &playing("something:else:7", 0)).is_empty());
    }

    /// The playlist cache is per account and per server, and the credential
    /// is not part of the path.
    #[test]
    fn the_account_key_names_the_user_and_the_server() {
        let credentials = Credentials::from_password("http://music.example:4533", "ada", "secret");
        let key = account_key(&credentials);
        assert_eq!(key, "ada@music.example-4533");
        assert!(!key.contains(&credentials.token));
    }

    /// A full page offers the next one; a short page is the end of the list.
    #[test]
    fn recents_page_forward_until_they_run_out() {
        let full = history_page(vec![Track::default(); 4], 8, 4);
        assert_eq!(full.next.as_deref(), Some("12"));
        assert_eq!(
            full.cursors.and_then(|cursors| cursors.before).as_deref(),
            Some("12")
        );
        let short = history_page(vec![Track::default(); 2], 8, 4);
        assert!(short.next.is_none());
    }

    /// A server with no native API leaves the section empty rather than
    /// putting an error on a page that is otherwise fine (D11).
    #[test]
    fn a_server_without_the_native_api_answers_empty() {
        let empty: Vec<Track> = empty_page(NativeError::NotNavidrome).unwrap();
        assert!(empty.is_empty());
        assert!(empty_cursor_page::<PlayHistory>(NativeError::NoSession).is_ok());
    }
}
