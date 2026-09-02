//! The application: state, event handling, and the actions views ask for.

use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};

use egui::Color32;

use crate::api::PlayRequest;
use crate::api::models::{
    Album, ArtistRef, Image, PlayableItem, Queue, Starred, Track, User, pick_image,
};
use crate::api::subsonic::convert::{self, COLLECTION_URI, Kind};
use crate::backend::{
    ApiRequest, ApiResponse, AuthStatus, Backend, Command, Event, LocalPlayback, LyricsRequest,
    PLAYLIST_PAGE_SIZE, RecentsFor, Waker,
};
use crate::engine::{EngineConfig, LoadSpec, LocalState, Playback, PlayerCommand, RepeatMode};
use crate::media::{MediaCommand, MediaState, MediaTrack};
use crate::media_controls::MediaService;
use crate::model::QueueTab;
use crate::model::*;
use crate::paths::AppDirs;
use crate::settings::{SessionState, Settings, ThemeChoice};
use crate::single_instance::ControlCommand;
use crate::theme::{self, Palette};
use crate::tray::{TrayCommand, TrayService};
use crate::util;

const SEARCH_DEBOUNCE: Duration = Duration::from_millis(280);
/// How far into a song Previous restarts it rather than stepping back,
/// matching what librespot does during playback.
const RESTART_BEFORE_PREVIOUS: u32 = 3_000;

const TOAST_LIFETIME: Duration = Duration::from_millis(3200);
const OPTIMISTIC_HOLD: Duration = Duration::from_millis(2500);

/// How long a newly started context remains visible while the backend catches up.
/// During local takeover, the backend may briefly alternate between old and new
/// context state.
const ASSUMED_CONTEXT_HOLD: Duration = Duration::from_secs(8);
/// How long the interface trusts its own play/pause over a polled state that
/// has not caught up yet. A backend can take a moment to report a command it
/// has already carried out.
const PLAYBACK_HOLD: Duration = Duration::from_secs(6);
/// Delay before checking playback again after a command.
/// Duplicate Play next events within this window count as one click, which
/// is the second half of rule 2 in `docs/_reference/queue.md`: two asks are
/// two rows, but one double-click is one ask.
const QUEUE_ADD_DEBOUNCE: Duration = Duration::from_millis(1500);

/// A context shown as playing before the backend confirms it.
struct AssumedContext {
    uri: String,
    /// Shuffle state included in the play request, if any.
    shuffle: Option<bool>,
    at: Instant,
}

/// The playing item as the interface sees it, whichever device plays it.
#[derive(Clone, Debug, PartialEq)]
pub struct NowPlaying {
    pub uri: String,
    pub id: Option<String>,
    pub title: String,
    pub artists: Vec<ArtistRef>,
    pub subtitle: String,
    pub album_name: String,
    pub album_id: Option<String>,
    pub art_url: Option<String>,
    pub art_small: Option<String>,
    pub duration_ms: u32,
    pub position_ms: u32,
    pub playing: bool,
    pub loading: bool,
    pub shuffle: bool,
    pub repeat: RepeatMode,
    pub volume_percent: u8,
    pub can_control: bool,
    /// The remembered song from the last session, shown paused before a
    /// first press. Nothing is playing yet.
    pub resuming: bool,
}

/// How the application is being started.
#[derive(Clone, Copy, Debug)]
pub struct AppOptions {
    /// Register the MPRIS media-control service (Linux).
    pub media_controls: bool,
    /// Register the system-tray item (Linux).
    pub tray: bool,
}

impl Default for AppOptions {
    fn default() -> Self {
        Self {
            media_controls: true,
            tray: true,
        }
    }
}

/// Listening time for the current track.
///
/// `listened` stores completed intervals. `playing_since` starts the current
/// interval and is `None` while paused.
struct Listening {
    uri: String,
    listened: std::time::Duration,
    playing_since: Option<Instant>,
    recorded: bool,
}

pub struct App {
    pub dirs: AppDirs,
    pub settings: Settings,
    settings_dirty: bool,
    last_settings_save: Instant,
    pub backend: Backend,
    media_controls: Option<MediaService>,
    tray: Option<TrayService>,
    pub window_hidden: bool,
    /// The window should close but the process should stay in the tray.
    pub hide_intent: bool,
    /// The outer loop should recreate the hidden window.
    pub wants_show: bool,
    /// The window should close and reopen at once as the other kind: the
    /// big window or the Winamp mini player.
    pub switch_intent: bool,
    /// Commands from control clients (a second `fastsonic <verb>` launch,
    /// a Raycast script), on the platforms where they do not arrive through
    /// MPRIS. Drained every frame.
    control_commands: Option<std::sync::Arc<std::sync::Mutex<Vec<ControlCommand>>>>,
    /// Now-playing snapshot for the control channel.
    control_now_playing: Option<std::sync::Arc<std::sync::Mutex<String>>>,
    /// Sample data is loaded; server requests are disabled.
    pub offline: bool,
    pub palette: Palette,
    applied_dark: Option<bool>,

    pub auth: AuthStatus,
    pub user: Option<User>,
    /// Local playback is authorized and the engine is connected.
    pub local_ready: bool,
    pub local_playback: LocalPlayback,
    pub local: LocalState,
    /// The restorable session (sorts, recents, resume point) changed and
    /// should be written shortly, not only at exit.
    pub session_dirty: bool,
    last_session_save: Instant,
    /// The saved zoom has been applied to the context once.
    zoom_applied: bool,
    /// The queue as the engine last published it, in the vocabulary the
    /// panel draws: what is playing, then every row below it in play order
    /// (rule 1 of `docs/_reference/queue.md`). It is never fetched and
    /// never fails — the queue is the engine's since P3.3, and the only
    /// other writer is the remembered one restored at startup.
    pub queue: Queue,
    /// How many leading rows of `queue` are songs queued by hand — the
    /// "Playing next" section. The engine says where the line falls, so
    /// two copies of one song land on the right sides of it.
    queued_len: usize,
    /// Whether the queue on show is the engine's rather than the one
    /// remembered from the last session. Only the engine's can be acted
    /// on: Clear asks the engine, and it has never heard of the other.
    queue_is_live: bool,
    /// Names for rows the engine has not described yet, from the Play next
    /// that queued them. A row is an id until the server answers about it,
    /// and a nameless row is not a queue panel.
    queue_names: HashMap<String, String>,
    /// What the window's title bar says, as last set.
    window_title: String,

    pub library: Library,
    pub home: HomeData,
    /// Local play history. See [`crate::history`].
    pub plays: crate::history::History,
    /// Current track timing used to decide when a play counts.
    listening: Option<Listening>,
    pub recents: crate::model::CursorList<crate::api::models::PlayHistory>,
    /// The Recent tab's rows: what was played here and what
    /// The server's recent plays and local history, as one list. Rebuilt when
    /// either side changes rather than every frame.
    pub recents_view: Vec<crate::api::models::PlayHistory>,
    pub recents_generation: u64,
    pub queue_tab: QueueTab,
    pub search: SearchState,
    pub playlist_pages: HashMap<String, PlaylistPage>,
    load_generation: u64,
    pub album_pages: HashMap<String, AlbumPage>,
    pub artist_pages: HashMap<String, ArtistPage>,
    pub track_cache: HashMap<String, Track>,
    track_requests: HashSet<String>,

    pub history: Vec<Page>,
    pub history_index: usize,

    pub saved: HashMap<String, bool>,
    saved_pending: HashSet<String>,
    pub accents: HashMap<String, Color32>,
    accent_pending: HashSet<String>,

    pub dialog: Option<Dialog>,
    pub show_queue_panel: bool,
    pub show_lyrics_panel: bool,
    /// The track the lyrics below are for.
    pub lyrics_uri: Option<String>,
    /// `Loaded(None)` when no lyrics are available.
    pub lyrics: Loadable<Option<crate::lyrics::Lyrics>>,
    /// Whether the panel follows the current line. Manual scrolling disables
    /// it until Follow is used or the track changes.
    pub lyrics_following: bool,
    /// The line the panel last positioned itself for (`Some(None)` before
    /// the first line), so it moves once per change; `None` until it has
    /// positioned itself at all for this track.
    pub lyrics_line_shown: Option<Option<usize>>,
    pub toasts: Vec<Toast>,
    pub actions: Vec<Action>,
    volume_before_mute: Option<u8>,
    /// Context and track URIs whose pending play buttons show a spinner.
    pending_play_keys: Vec<String>,
    pending_play_at: Option<Instant>,
    /// A play request made while the local engine was still connecting; it
    /// starts the moment the engine reports ready.
    queued_play: Option<PlayRequest>,
    /// Last list sent to local playback. Used for autoplay because librespot
    /// cannot continue a list without a context URI.
    local_list: Option<Vec<String>>,
    /// When to take a confirming look at remote playback after a command.
    pub seek_preview: Option<f32>,
    pub volume_preview: Option<f32>,
    /// Window geometry to restore on next attach, from the session file.
    session_window_size: Option<[f32; 2]>,
    session_window_pos: Option<[f32; 2]>,
    /// Last observed window geometry, updated each frame for saving.
    last_window_size: Option<[f32; 2]>,
    /// Where the open dialog drew itself, so its own layout can be held
    /// to the window it has to fit inside.
    pub dialog_rect: Option<egui::Rect>,
    last_window_pos: Option<[f32; 2]>,
    /// Where the MilkDrop window last was, as it reported, for restoring it.
    pub milkdrop_pos: Option<[f32; 2]>,
    /// The MilkDrop child process; `None` until it is first opened. Its
    /// `Drop` stops the child when the app does.
    #[cfg(feature = "milkdrop")]
    milkdrop_host: Option<crate::milkdrop::host::Host>,
    last_eviction: Instant,
    pub sign_in_url: Option<String>,
    /// The sign-in form: the server's address, the account on it, and the
    /// password on its way to being exchanged for the pair that is stored.
    /// The first two come back from the backend so a failed start returns to
    /// a filled-in form; the third is cleared the moment it is sent and is
    /// never written anywhere.
    pub server: String,
    pub server_user: String,
    pub server_password: String,
    /// Which account's files on disk belong to this session — the playlist
    /// cache's directory, and the check that a cached answer is this
    /// account's.
    pub account_id: Option<String>,
    /// The bytes of streams kept on disk, shared with the engine and held
    /// here so that replacing the engine does not rescan them (P3.6).
    pub audio_cache: Option<std::sync::Arc<crate::engine::Cache>>,
    /// A local volume set here that the engine has not echoed back yet. It
    /// reports `VolumeChanged` asynchronously while position snapshots land
    /// every second, so a snapshot must not undo the change on its way past.
    pending_local_volume: Option<(u16, Instant)>,
    optimistic_playing: Option<(bool, Instant)>,
    /// Track shown immediately after a play request, until the engine reports.
    intent_track: Option<(String, Instant)>,
    /// Requested shuffle mode, applied to every context until changed.
    shuffle_wanted: bool,
    /// Last local shuffle change, used to ignore its echo from the engine.
    shuffle_set_at: Option<Instant>,
    /// Context shown immediately after play, until the engine confirms it.
    assumed_context: Option<AssumedContext>,
    last_now_playing_uri: Option<String>,
    pub playlist_busy: bool,
    pub quit_requested: bool,
    /// The axis a scroll gesture settled on, and when it last moved.
    scroll_lock: Option<(ScrollAxis, Instant)>,
    /// Whether the current scroll gesture comes from a trackpad.
    scroll_from_trackpad: bool,
    /// Recent scroll positions, to read the gesture's speed when it ends.
    scroll_history: egui::util::History<egui::Vec2>,
    /// Where the gesture has scrolled to so far, for the history.
    scroll_accum: egui::Vec2,
    /// The speed still carrying the page after the fingers lifted.
    glide: Option<egui::Vec2>,
    /// Time of the last scroll event, used to detect the end of a gesture.
    scroll_last_event: Option<Instant>,
    /// How each table is sorted, per page, for as long as the app runs.
    /// The rows picked out in a track table, and the page they belong to.
    /// One table at a time: picking rows on another page replaces it.
    /// The page it belongs to, what that page's list looked like when the
    /// rows were picked, and the rows.
    pub selection: Option<(Page, String, RowSelection)>,
    pub table_sorts: HashMap<Page, TableSort>,
    /// User ids resolved to display names; `None` while unknown, so an id
    /// is asked about only once per run.
    pub user_names: HashMap<String, Option<String>>,
    pub user_names_revision: u64,
    /// Context URIs most recently played, newest first: the sidebar's
    /// order. Kept with the session, so it survives a restart.
    pub recent_contexts: Vec<String>,
    /// What was playing when the app last closed, to resume from cold.
    pub resume_context: Option<String>,
    pub resume_track: Option<String>,
    pub resume_position_ms: u32,
    /// The context row the remembered track was playing over, when it was
    /// a song queued by hand (rule 3). `None` means the remembered track
    /// *is* the album's row, which is the ordinary case.
    pub resume_context_track: Option<String>,
    /// Manually queued songs restored with the remembered track. Rule 9:
    /// they are handed back to the engine with the play that resumes it.
    pub resume_queue: Vec<String>,
    /// What the engine says it is playing from, and how far the album has
    /// got: the truth an assumed context is checked against, and what the
    /// session file remembers so a restart can put the album back.
    local_context: Option<String>,
    local_context_at: Option<String>,
    /// The songs queued by hand, oldest first, as the engine last reported
    /// them. This is what the session file keeps; the panel's own split
    /// comes from `queued_len`.
    pub manual_queue: Vec<String>,
    /// Play next clicks in the last moment, to tell a double-click from two
    /// asks (rule 2). The engine has already been told about all of them.
    queue_add_clicks: Vec<(String, Instant)>,
    /// A newer release than this build, once GitHub has said so.
    pub update: Option<crate::updates::Release>,
    last_update_check: Option<Instant>,
    /// Winamp window state and active skin.
    pub winamp: crate::winamp::WinampState,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ScrollAxis {
    Horizontal,
    Vertical,
}

/// A trackpad gesture that pauses this long has ended; the next movement
/// picks its axis afresh.
const SCROLL_GESTURE_GAP: Duration = Duration::from_millis(150);

/// How far short Linux trackpad deltas land of what other players scroll.
const TRACKPAD_SCALE: f32 = 1.8;

/// The glide's exponential decay time, in seconds; the speed below which a
/// lift starts no glide; and the speed at which a glide stops, points per
/// second.
/// How many plays the Home shelf asks for: it shows sixteen cards.
const HOME_RECENTS: u32 = 50;
/// How many plays the Recents tab asks for at a time. The server's
/// endpoint limit is fifty. A shorter page marks the end.
const RECENTS_PAGE: u32 = 50;

const GLIDE_DECAY: f32 = 0.35;
const GLIDE_START: f32 = 120.0;
const GLIDE_STOP: f32 = 40.0;

impl App {
    pub fn new(waker: &Waker, dirs: AppDirs, settings: Settings, options: AppOptions) -> Self {
        let plays = crate::history::History::load(&dirs.history_file());
        let tap = crate::vis::AudioTap::new();
        let eq = crate::eq::shared();
        if let Ok(mut shared) = eq.lock() {
            *shared = eq_settings(&settings);
        }
        let audio_cache = audio_cache(&dirs, &settings);
        let engine_config = engine_config(
            &settings,
            std::sync::Arc::clone(&tap),
            std::sync::Arc::clone(&eq),
            audio_cache.clone(),
        );
        let backend = Backend::spawn(dirs.clone(), engine_config, waker.clone());
        let session = SessionState::load(&dirs.session_file());
        let wake = waker.clone();
        let media_controls = options
            .media_controls
            .then(|| MediaService::spawn(move || wake.wake()));
        #[cfg(target_os = "macos")]
        let media_controls = {
            let mut media_controls = media_controls;
            if let (Some(controls), Some(track)) =
                (&mut media_controls, session.last_track.as_deref())
            {
                controls.claim_resume(track, session.last_position_ms);
            }
            media_controls
        };
        let wake = waker.clone();
        let tray = options
            .tray
            .then(|| TrayService::spawn(move || wake.wake()))
            .flatten();

        let first_page = session
            .last_page
            .as_deref()
            .and_then(Page::decode)
            .filter(|page| !matches!(page, Page::Settings | Page::Queue))
            .unwrap_or(Page::Home);

        let mut app = Self {
            dirs,
            settings,
            settings_dirty: false,
            last_settings_save: Instant::now(),
            backend,
            media_controls,
            tray,
            window_hidden: false,
            hide_intent: false,
            wants_show: false,
            switch_intent: false,
            control_commands: None,
            control_now_playing: None,
            offline: false,
            palette: Palette::dark(),
            applied_dark: None,
            auth: AuthStatus::Starting,
            user: None,
            local_ready: false,
            local_playback: LocalPlayback::Unavailable,
            local: LocalState::default(),
            session_dirty: false,
            last_session_save: Instant::now(),
            zoom_applied: false,
            // Filled in below, from what the last session left (rule 9).
            queue: Queue::default(),
            queued_len: 0,
            queue_is_live: false,
            queue_names: HashMap::new(),
            window_title: String::new(),
            library: Library::default(),
            home: HomeData::default(),
            plays,
            listening: None,
            recents: crate::model::CursorList::default(),
            recents_view: Vec::new(),
            recents_generation: 0,
            queue_tab: session
                .queue_tab
                .as_deref()
                .and_then(QueueTab::decode)
                .unwrap_or_default(),
            search: SearchState::default(),
            playlist_pages: HashMap::new(),
            load_generation: 0,
            album_pages: HashMap::new(),
            artist_pages: HashMap::new(),
            track_cache: HashMap::new(),
            track_requests: HashSet::new(),
            history: vec![first_page],
            history_index: 0,
            saved: HashMap::new(),
            saved_pending: HashSet::new(),
            accents: HashMap::new(),
            accent_pending: HashSet::new(),
            dialog: None,
            show_queue_panel: session.queue_open.unwrap_or(false),
            show_lyrics_panel: false,
            lyrics_uri: None,
            lyrics: Loadable::NotLoaded,
            lyrics_following: true,
            lyrics_line_shown: None,
            toasts: Vec::new(),
            actions: Vec::new(),
            volume_before_mute: None,
            pending_play_keys: Vec::new(),
            pending_play_at: None,
            queued_play: None,
            local_list: None,
            seek_preview: None,
            volume_preview: None,
            session_window_size: session.window_size,
            session_window_pos: session.window_pos,
            last_window_size: None,
            dialog_rect: None,
            last_window_pos: None,
            milkdrop_pos: session.milkdrop_pos,
            #[cfg(feature = "milkdrop")]
            milkdrop_host: None,
            last_eviction: Instant::now(),
            sign_in_url: None,
            server: String::new(),
            server_user: String::new(),
            server_password: String::new(),
            account_id: None,
            audio_cache,
            pending_local_volume: None,
            optimistic_playing: None,
            intent_track: None,
            shuffle_wanted: session.shuffle_on,
            shuffle_set_at: None,
            assumed_context: None,
            last_now_playing_uri: None,
            playlist_busy: false,
            quit_requested: false,
            scroll_lock: None,
            scroll_from_trackpad: false,
            scroll_history: egui::util::History::new(2..16, 0.1),
            scroll_accum: egui::Vec2::ZERO,
            glide: None,
            scroll_last_event: None,
            selection: None,
            table_sorts: session
                .sorts
                .iter()
                .filter_map(|(page, sort)| Some((Page::decode(page)?, *sort)))
                .collect(),
            user_names: HashMap::new(),
            user_names_revision: 0,
            recent_contexts: session.recent_contexts.clone(),
            resume_context: session.last_context.clone(),
            resume_track: session.last_track.clone(),
            resume_position_ms: session.last_position_ms,
            resume_context_track: session.last_context_track.clone(),
            resume_queue: session.last_added_queue.clone(),
            local_context: None,
            local_context_at: None,
            manual_queue: Vec::new(),
            queue_add_clicks: Vec::new(),
            update: None,
            last_update_check: None,
            winamp: crate::winamp::WinampState::new(session.winamp_pos, tap, eq),
        };
        app.local.volume = app.settings.volume;
        // The queue as it was at close, shown until something plays; then
        // the engine's is the queue (rule 9).
        if session.last_track.is_some() {
            app.set_remembered_queue(session.last_queue_rows, session.last_added_queue.len());
        }
        // What was played here is on disk and needs nothing from the
        // network, so the tab has rows before the server has answered.
        app.rebuild_recents();
        app
    }

    /// Watches the queue control clients fill and keeps the now-playing
    /// snapshot they read fresh.
    pub fn set_remote_control(&mut self, guard: &crate::single_instance::Guard) {
        self.control_commands = Some(guard.commands());
        self.control_now_playing = Some(guard.now_playing_slot());
    }

    /// Per-window setup: fonts, icons, loaders, theme. Called every time a
    /// window is (re)created around this long-lived application state.
    pub fn attach(&mut self, ctx: &egui::Context) {
        theme::install(ctx);
        ctx.add_bytes_loader(std::sync::Arc::new(self.backend.art().clone()));
        ctx.set_theme(match self.settings.theme {
            ThemeChoice::Dark => egui::ThemePreference::Dark,
            ThemeChoice::Light => egui::ThemePreference::Light,
            ThemeChoice::System => egui::ThemePreference::System,
        });
        self.applied_dark = None;
        self.winamp.forget_textures();
        self.window_hidden = false;
        self.hide_intent = false;
        self.wants_show = false;
        self.switch_intent = false;
        if let Some(tray) = &mut self.tray {
            tray.attach();
        }
        if self.settings.winamp_window {
            // The mini player sizes itself; the big window's geometry
            // waits here for its return.
            return;
        }
        if let Some(size) = self.session_window_size.take() {
            // Clamp to a sane range so a stale session never creates an
            // unusable window; the OS will further clamp to the monitor.
            if (400.0..=3000.0).contains(&size[0]) && (300.0..=2000.0).contains(&size[1]) {
                ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(egui::vec2(
                    size[0], size[1],
                )));
            }
        }
        if let Some(pos) = self.session_window_pos.take() {
            // On Wayland this is a no-op. Validate against a large virtual
            // desktop so a window that was on a now-disconnected monitor
            // doesn't open off-screen.
            if (-1000.0..=5000.0).contains(&pos[0]) && (-1000.0..=5000.0).contains(&pos[1]) {
                ctx.send_viewport_cmd(egui::ViewportCommand::OuterPosition(egui::pos2(
                    pos[0], pos[1],
                )));
            }
        }
        // egui's consensus wheel speed is 40 points per line, about a third
        // of what every other player scrolls per notch; trackpads report
        // pixels and are unaffected (#32).
        ctx.options_mut(|options| options.input_options.line_scroll_speed = 120.0);
    }

    /// The window is gone but the process stays: audio, the tray, and the
    /// media controls keep running until Show or Quit.
    pub fn window_gone(&mut self) {
        // The Winamp window went with it; it comes back where it was.
        self.winamp.remember_position();
        self.winamp.forget_textures();
        self.window_hidden = true;
        self.hide_intent = false;
        self.wants_show = false;
        if let Some(tray) = &mut self.tray {
            tray.hidden();
        }
    }

    /// Whether closing the window keeps the app in the tray rather than
    /// quitting.
    pub fn hides_to_tray(&self) -> bool {
        self.tray.is_some() && self.settings.keep_playing_in_background
    }

    // ---- derived state -----------------------------------------------------

    pub fn page(&self) -> &Page {
        &self.history[self.history_index]
    }

    pub fn is_connected(&self) -> bool {
        matches!(self.auth, AuthStatus::Connected { .. })
    }

    pub fn user_id(&self) -> Option<&str> {
        self.user.as_ref().map(|user| user.id.as_str())
    }

    pub fn is_saved(&self, uri: &str) -> Option<bool> {
        self.saved.get(uri).copied()
    }

    /// Context shown as playing, including pending local play requests.
    pub fn playing_context_uri(&self) -> Option<String> {
        let live = self.local_context.clone();
        if let Some(assumed) = &self.assumed_context {
            let held = assumed.at.elapsed() < ASSUMED_CONTEXT_HOLD;
            // A filtered or sorted context plays as plain tracks and will not
            // report a context URI. Keep the assumed URI unless contradicted.
            let contradicted = live.as_deref().is_some_and(|uri| uri != assumed.uri);
            if held || (!contradicted && self.believed_playing()) {
                return Some(assumed.uri.clone());
            }
        }
        live
    }

    /// The row the album is on under `playing`, when there is one to
    /// remember: a song queued by hand plays over the album and leaves it
    /// where it was (rule 3), and rule 9's restore needs the place as well
    /// as the song.
    ///
    /// `None` unless the engine's queue and the player agree on what is
    /// playing — a snapshot published for the track before this one would
    /// otherwise remember the album a row behind — and `None` when the
    /// album's row is the song playing, which is the ordinary case and
    /// needs nothing remembered.
    fn context_row_under(&self, playing: &str) -> Option<String> {
        let current = self
            .queue
            .currently_playing
            .as_ref()
            .map(|item| item.uri().to_string())?;
        if current != playing {
            return None;
        }
        self.local_context_at.clone().filter(|at| at != playing)
    }

    /// Track shown as current, including a recent unconfirmed play request.
    pub fn current_track_uri(&self) -> Option<String> {
        if let Some((uri, at)) = &self.intent_track
            && at.elapsed() < PLAYBACK_HOLD
        {
            return Some(uri.clone());
        }
        self.now_playing().map(|now| now.uri)
    }

    /// Playback state shown by the UI, including a recent local request.
    pub fn believed_playing(&self) -> bool {
        if let Some((playing, at)) = self.optimistic_playing
            && at.elapsed() < PLAYBACK_HOLD
        {
            return playing;
        }
        self.now_playing().is_some_and(|now| now.playing)
    }

    pub fn playing_context_shuffle(&self) -> bool {
        self.shuffle_wanted
    }

    /// Current item for menus, using cached track details when available.
    pub fn now_playing_item(&self) -> Option<PlayableItem> {
        let now = self.now_playing()?;
        if let Some(track) = now.id.as_deref().and_then(|id| self.track_cache.get(id)) {
            return Some(PlayableItem::Track(track.clone()));
        }
        Some(PlayableItem::Track(Track {
            id: now.id.clone(),
            uri: now.uri.clone(),
            name: now.title.clone(),
            artists: now.artists.clone(),
            duration_ms: now.duration_ms,
            ..Track::default()
        }))
    }

    pub fn now_playing(&self) -> Option<NowPlaying> {
        self.now_playing_live().or_else(|| self.resume_preview())
    }

    /// What a device is actually playing, here or elsewhere.
    fn now_playing_live(&self) -> Option<NowPlaying> {
        if self.local.is_active() {
            let track = self.local.track.as_ref()?;
            let cached = track
                .uri
                .rsplit(':')
                .next()
                .and_then(|id| self.track_cache.get(id));
            let artists = cached
                .map(|cached| cached.artists.clone())
                .unwrap_or_else(|| {
                    track
                        .artists
                        .iter()
                        .map(|name| ArtistRef {
                            id: None,
                            name: name.clone(),
                            uri: None,
                        })
                        .collect()
                });
            let playing = match self.optimistic_playing {
                Some((playing, at)) if at.elapsed() < PLAYBACK_HOLD => playing,
                _ => self.local.playback == Playback::Playing,
            };
            return Some(NowPlaying {
                uri: track.uri.clone(),
                id: util::uri_id(&track.uri).map(str::to_string),
                title: track.title.clone(),
                subtitle: track.artist_names(),
                artists,
                album_name: track.album.clone(),
                album_id: cached
                    .and_then(|cached| cached.album.as_ref())
                    .map(|album| album.id.clone()),
                art_url: track.art_url.clone(),
                art_small: track
                    .art_small_url
                    .clone()
                    .or_else(|| track.art_url.clone()),
                duration_ms: track.duration_ms,
                position_ms: self.local.position_now(),
                playing,
                loading: self.local.playback == Playback::Loading,
                shuffle: self.shuffle_wanted,
                repeat: self.local.repeat,
                volume_percent: volume_to_percent(self.local.volume),
                can_control: true,
                resuming: false,
            });
        }
        None
    }

    /// Last session's track, shown paused when no device is playing.
    fn resume_preview(&self) -> Option<NowPlaying> {
        let uri = self.resume_track.as_deref()?;
        let track = self.track_cache.get(util::uri_id(uri)?)?;
        Some(NowPlaying {
            uri: uri.to_string(),
            id: track.id.clone(),
            title: track.name.clone(),
            subtitle: track.artist_names(),
            artists: track.artists.clone(),
            album_name: track
                .album
                .as_ref()
                .map(|album| album.name.clone())
                .unwrap_or_default(),
            album_id: track.album.as_ref().map(|album| album.id.clone()),
            art_url: track.image(640).map(str::to_string),
            art_small: track.image(64).map(str::to_string),
            duration_ms: track.duration_ms,
            position_ms: self.resume_position_ms.min(track.duration_ms),
            playing: false,
            loading: false,
            shuffle: self.shuffle_wanted,
            repeat: RepeatMode::Off,
            volume_percent: volume_to_percent(self.local.volume),
            can_control: true,
            resuming: true,
        })
    }

    /// The play request for `key` (a context or track URI) is still waiting
    /// for the backend to react.
    pub fn play_pending(&self, key: &str) -> bool {
        self.pending_fresh() && self.pending_play_keys.iter().any(|k| k == key)
    }

    pub fn set_user_name(&mut self, id: String, name: Option<String>) {
        if self.user_names.get(&id) != Some(&name) {
            self.user_names.insert(id, name);
            self.user_names_revision = self.user_names_revision.wrapping_add(1);
        }
    }

    pub fn any_play_pending(&self) -> bool {
        self.pending_fresh() && !self.pending_play_keys.is_empty()
    }

    fn pending_fresh(&self) -> bool {
        // A request queued behind a connecting engine stays pending for as
        // long as the engine may take; an ordinary request times out fast.
        self.queued_play.is_some()
            || self
                .pending_play_at
                .is_some_and(|at| at.elapsed() < Duration::from_secs(8))
    }

    fn set_play_pending(&mut self, keys: Vec<String>) {
        self.pending_play_keys = keys;
        self.pending_play_at = Some(Instant::now());
    }

    fn clear_play_pending(&mut self) {
        self.pending_play_keys.clear();
        self.pending_play_at = None;
    }

    /// The colour to tint the interface with, from the playing art.
    pub fn now_playing_tint(&self) -> Option<Color32> {
        if !self.settings.accent_from_art {
            return None;
        }
        let now = self.now_playing()?;
        let url = now.art_small.or(now.art_url)?;
        self.accents.get(&url).copied()
    }

    pub fn tint_for(&mut self, url: Option<&str>) -> Option<Color32> {
        let url = url?;
        if let Some(color) = self.accents.get(url) {
            return Some(*color);
        }
        if self.accent_pending.insert(url.to_string()) {
            self.backend.send(Command::Accent {
                url: url.to_string(),
            });
        }
        None
    }

    // ---- frame ---------------------------------------------------------------

    fn handle_events(&mut self) {
        for event in self.backend.poll() {
            if self.offline {
                continue;
            }
            match event {
                Event::Auth(status) => self.handle_auth(status),
                Event::Playback(status) => self.handle_playback(status),
                Event::Local(state) => self.handle_local(*state),
                Event::Api(response) => self.handle_api(*response),
                Event::Accent { url, color } => {
                    self.accent_pending.remove(&url);
                    let tint = self.palette.tint_from_art(color);
                    self.accents.insert(url, tint);
                }
                Event::Error(message) => self.toast_error(message),
                Event::Lyrics { uri, result } => {
                    if self.lyrics_uri.as_deref() == Some(uri.as_str()) {
                        self.lyrics = match result {
                            Ok(found) => Loadable::Loaded(found),
                            Err(error) => Loadable::Failed(error),
                        };
                    }
                }
                Event::PlaylistCache {
                    account_id,
                    id,
                    snapshot,
                    items,
                } => {
                    if self.account_id.as_deref() != Some(account_id.as_str()) {
                        continue;
                    }
                    if let Some(page) = self.playlist_pages.get_mut(&id) {
                        page.pending_cache = Some((snapshot, items));
                    }
                    self.try_adopt_playlist_cache(&id);
                }
                Event::KnownServer {
                    server,
                    username,
                    account,
                } => {
                    self.server = server;
                    self.server_user = username;
                    self.account_id = Some(account);
                }
                Event::Queue(queue) => self.handle_queue(*queue),
                Event::UpdateAvailable { version, url } => {
                    let notice = crate::updates::Release { version, url };
                    if self.update.as_ref() != Some(&notice) {
                        self.toast(format!("Fastsonic {} is available", notice.version));
                    }
                    self.update = Some(notice);
                }
            }
        }
    }

    fn handle_auth(&mut self, status: AuthStatus) {
        match &status {
            AuthStatus::Connected { .. } => {
                self.sign_in_url = None;
                self.reset_data();
                self.load_playlists();
                self.ensure_loaded(self.page().clone());
            }
            AuthStatus::SignedOut => {
                self.sign_in_url = None;
                self.user = None;
                self.local = LocalState::default();
                self.local_ready = false;
                self.local_playback = LocalPlayback::Unavailable;
                self.reset_data();
            }
            AuthStatus::Failed(message) => {
                self.sign_in_url = None;
                self.toast_error(message.clone());
            }
            _ => {}
        }
        self.auth = status;
    }

    fn handle_playback(&mut self, status: LocalPlayback) {
        match &status {
            LocalPlayback::Ready => {
                self.local_ready = true;
                if let Some(request) = self.queued_play.take() {
                    self.play_request(request, false);
                }
            }
            LocalPlayback::Unavailable => {
                self.local_ready = false;
            }
            LocalPlayback::Failed(message) => {
                self.local_ready = false;
                if self.queued_play.take().is_some() {
                    self.clear_play_pending();
                }
                self.toast_error(format!("Local playback: {message}"));
            }
            LocalPlayback::Connecting => {}
        }
        self.local_playback = status;
    }

    fn reset_data(&mut self) {
        self.library = Library::default();
        self.home = HomeData::default();
        self.playlist_pages.clear();
        self.album_pages.clear();
        self.artist_pages.clear();
        self.saved.clear();
        self.saved_pending.clear();
        self.queue = Queue::default();
        self.queued_len = 0;
        self.queue_is_live = false;
        self.queue_names.clear();
        self.manual_queue.clear();
        self.local_context = None;
        self.local_context_at = None;
        self.search.results = Loadable::NotLoaded;
        self.search.committed.clear();
    }

    /// The engine's queue, in the vocabulary the queue panel reads.
    ///
    /// With no Connect there is nothing to reconcile against and nothing to
    /// poll: what the engine says is the queue *is* the queue. It answers a
    /// command on the audio thread before any request goes out, and the
    /// answer wakes the frame that draws it — which is rule 8, and why
    /// nothing here is optimistic.
    ///
    /// Demo mode publishes one of these itself, having no engine to run.
    pub(crate) fn handle_queue(&mut self, snapshot: crate::engine::QueueSnapshot) {
        self.queue = Queue {
            currently_playing: snapshot.current.as_ref().map(|row| self.queue_row(row)),
            queue: snapshot.rows().map(|row| self.queue_row(row)).collect(),
        };
        self.queued_len = snapshot.queued.len();
        self.queue_is_live = true;
        // What the engine is playing from, and where the album has got to
        // underneath a queued song. Nothing else knows either: the app can
        // assume a context, but only the engine holds one (rule 9).
        self.local_context = snapshot.context_uri.clone();
        self.local_context_at = snapshot.context_at.clone();
        // The queue carries the starred flag with each song, so the hearts
        // in the panel need no request of their own.
        let mut flags = starred_flags(&self.queue.queue);
        flags.extend(starred_flags(self.queue.currently_playing.iter()));
        self.note_saved(flags);
        let manual: Vec<String> = snapshot.queued.iter().map(|row| row.uri.clone()).collect();
        if manual != self.manual_queue {
            self.manual_queue = manual;
            self.session_dirty = true;
        }
        // A name is only owed while the row it belongs to has none — or
        // while the ask that carried it is new enough that this snapshot
        // may have been published before the engine had heard it.
        if !self.queue_names.is_empty() {
            let mut keep: std::collections::HashSet<String> = snapshot
                .queued
                .iter()
                .chain(snapshot.upcoming.iter())
                .filter(|row| row.track.is_none())
                .map(|row| row.uri.clone())
                .collect();
            keep.extend(
                self.queue_add_clicks
                    .iter()
                    .filter(|(_, at)| at.elapsed() < QUEUE_ADD_DEBOUNCE)
                    .map(|(uri, _)| uri.clone()),
            );
            self.queue_names.retain(|uri, _| keep.contains(uri));
        }
    }

    /// One queue row as a track. A row the engine has not been able to
    /// describe yet keeps the name the click that queued it knew, or the
    /// one a page already loaded knows, so that Play next shows a song
    /// rather than a blank line for the moment before the server answers.
    fn queue_row(&self, row: &crate::engine::QueueRow) -> PlayableItem {
        if row.track.is_some() {
            return queue_row_item(row);
        }
        if let Some(track) = util::uri_id(&row.uri).and_then(|id| self.track_cache.get(id)) {
            return PlayableItem::Track(track.clone());
        }
        PlayableItem::Track(Track {
            id: convert::id_of(&row.uri, Kind::Track).map(str::to_string),
            uri: row.uri.clone(),
            name: self.queue_names.get(&row.uri).cloned().unwrap_or_default(),
            ..Track::default()
        })
    }

    /// Signs in with what the form holds. The password goes to the backend
    /// once and is dropped here the moment it does; an empty one means "try
    /// the credential already stored", which is how a server that was
    /// unreachable at startup is retried.
    fn sign_in(&mut self) {
        let password = std::mem::take(&mut self.server_password);
        self.backend
            .send(Command::SignIn(Box::new(crate::backend::SignInRequest {
                server: self.server.trim().to_string(),
                username: self.server_user.trim().to_string(),
                password: (!password.is_empty()).then_some(password),
            })));
    }

    /// What the engine says it is doing. Demo mode sends one of these too,
    /// for the same reason it publishes a queue.
    pub(crate) fn handle_local(&mut self, state: LocalState) {
        let track_changed = state.track != self.local.track;
        let reconnected = state.connected && !self.local.connected;
        if state.shuffle != self.local.shuffle
            && self
                .shuffle_set_at
                .is_none_or(|at| at.elapsed() > Duration::from_secs(5))
        {
            // Accept shuffle changes made by another client.
            self.shuffle_wanted = state.shuffle;
        }
        if state.playback != self.local.playback {
            self.optimistic_playing = None;
            if matches!(state.playback, Playback::Playing | Playback::Loading) {
                self.clear_play_pending();
            }
        }
        if state.track != self.local.track {
            self.clear_play_pending();
            if let (Some(track), Some((intent, _))) = (&state.track, &self.intent_track)
                && track.uri == *intent
            {
                // The engine confirmed the requested track.
                self.intent_track = None;
            }
        }
        let held_volume = self.held_local_volume(state.volume);
        if held_volume.is_none() && state.volume != self.settings.volume {
            self.settings.volume = state.volume;
            self.settings_dirty = true;
        }
        if state.seek_sequence != self.local.seek_sequence
            && let Some(controls) = &self.media_controls
        {
            controls.seeked(state.position_ms);
        }
        if let Some(error) = &state.error
            && self.local.error.as_deref() != Some(error.as_str())
        {
            self.toast_error(error.clone());
        }
        self.local = state;
        if let Some(volume) = held_volume {
            self.local.volume = volume;
        }
        if track_changed {
            // The song carries its own starred flag, so the heart in the
            // player bar is answered by the song that just started.
            if let Some(track) = &self.local.track
                && let Some(flag) = track.starred
            {
                let uri = track.uri.clone();
                self.note_saved(vec![(uri, flag)]);
            }
            self.on_now_playing_changed();
        }
        if reconnected {
            if let Some(request) = self.queued_play.take() {
                self.play_request(request, false);
            }
            // Retry names requested before the session connected.
            let unresolved: Vec<String> = self
                .user_names
                .iter()
                .filter(|(_, name)| name.is_none())
                .map(|(id, _)| id.clone())
                .collect();
            for id in &unresolved {
                self.user_names.remove(id);
            }
            self.request_user_names(unresolved);
        }
    }

    /// Loads details for the remembered track before playback starts.
    fn request_resume_track(&mut self) {
        if self.now_playing_live().is_some() {
            return;
        }
        let Some(uri) = self.resume_track.clone() else {
            return;
        };
        // Only a song has a song endpoint; anything else the preview skips.
        let Some(id) = convert::id_of(&uri, Kind::Track).map(str::to_string) else {
            return;
        };
        if self.track_cache.contains_key(&id) || !self.track_requests.insert(id.clone()) {
            return;
        }
        self.backend.api(ApiRequest::Track { id });
    }

    /// Whether the player bar shows the remembered track without live playback.
    fn resume_only(&self) -> bool {
        self.resume_track.is_some() && self.now_playing_live().is_none()
    }

    /// Loads the remembered context so Previous and Next work before playback.
    fn ensure_resume_context_loaded(&mut self) {
        if !self.resume_only() {
            return;
        }
        let Some(context) = self.resume_context.clone() else {
            return;
        };
        if self.context_track_uris(&context).is_some() {
            return;
        }
        if let Some(page) = Page::decode(&Self::context_page(&context)) {
            self.ensure_loaded(page);
        }
    }

    /// Encodes a context as a value accepted by `Page::decode`.
    fn context_page(context_uri: &str) -> String {
        if context_uri.ends_with(":collection") {
            return "liked".to_owned();
        }
        match (util::uri_kind(context_uri), util::uri_id(context_uri)) {
            (Some(kind), Some(id)) => format!("{kind}:{id}"),
            _ => String::new(),
        }
    }

    /// Moves the paused remembered track within its context without playing.
    /// Returns `false` when the context is unavailable.
    fn step_resume(&mut self, forward: bool) -> bool {
        let Some(context) = self.resume_context.clone() else {
            return false;
        };
        let Some(uris) = self.context_track_uris(&context) else {
            return false;
        };
        let current = self.resume_track.clone().unwrap_or_default();
        let next = if self.shuffle_wanted && forward {
            // Preserve shuffle behavior before playback resumes.
            let choices: Vec<&String> = uris.iter().filter(|uri| **uri != current).collect();
            if choices.is_empty() {
                return false;
            }
            choices[rand::random_range(0..choices.len())].clone()
        } else {
            let Some(index) = uris.iter().position(|uri| *uri == current) else {
                return false;
            };
            let last = uris.len() - 1;
            let target = match (forward, index) {
                (true, i) if i == last => 0,
                (true, i) => i + 1,
                (false, 0) => last,
                (false, i) => i - 1,
            };
            uris[target].clone()
        };
        self.cache_track_from_context(&context, &next);
        self.resume_track = Some(next);
        // The song stepped to is the context's own row, so the album is on
        // it: whatever queued song was remembered over the top of it has
        // been stepped past (rule 3).
        self.resume_context_track = None;
        self.resume_position_ms = 0;
        self.session_dirty = true;
        true
    }

    /// Caches track details from a loaded context for immediate display.
    fn cache_track_from_context(&mut self, context_uri: &str, uri: &str) {
        let Some(id) = util::uri_id(uri) else {
            return;
        };
        if self.track_cache.contains_key(id) {
            return;
        }
        let found = if let Some(pid) = convert::id_of(context_uri, Kind::Playlist) {
            self.playlist_pages.get(pid).and_then(|page| {
                page.items
                    .items
                    .iter()
                    .find_map(|item| match item.playable() {
                        Some(PlayableItem::Track(track)) if track.uri == uri => Some(track.clone()),
                        _ => None,
                    })
            })
        } else if let Some(aid) = convert::id_of(context_uri, Kind::Album) {
            self.album_pages.get(aid).and_then(|page| {
                page.tracks
                    .items
                    .iter()
                    .find(|track| track.uri == uri)
                    .cloned()
            })
        } else if context_uri == COLLECTION_URI {
            self.library
                .liked
                .items
                .iter()
                .find(|item| item.track.uri == uri)
                .map(|item| item.track.clone())
        } else {
            None
        };
        if let Some(track) = found {
            self.track_cache.insert(id.to_owned(), track);
        }
    }

    fn on_now_playing_changed(&mut self) {
        let Some(now) = self.now_playing() else {
            return;
        };
        // Do not reset the saved position for a resume preview.
        if now.resuming {
            return;
        }
        if self.last_now_playing_uri.as_deref() == Some(now.uri.as_str()) {
            return;
        }
        // Rule 9: the songs owed to the queue go out with the play that
        // resumes the remembered track (`resume_last`). Anything else
        // playing first means the session was not resumed, and they are
        // owed to nobody — the engine's queue is the queue from here.
        if !self.resume_queue.is_empty() {
            self.resume_queue.clear();
            self.session_dirty = true;
        }
        self.last_now_playing_uri = Some(now.uri.clone());
        self.resume_context = self.playing_context_uri();
        self.resume_track = Some(now.uri.clone());
        self.resume_context_track = self.context_row_under(&now.uri);
        self.resume_position_ms = 0;
        if let Some(id) = &now.id
            && !self.track_cache.contains_key(id)
            && self.track_requests.insert(id.clone())
        {
            self.backend.api(ApiRequest::Track { id: id.clone() });
        }
        if let Some(url) = now.art_small.or(now.art_url) {
            self.tint_for(Some(&url));
        }
        if self.show_lyrics_panel {
            self.request_lyrics();
        }
    }

    /// Asks for the playing track's lyrics unless they are here or on the way.
    pub fn request_lyrics(&mut self) {
        let Some(now) = self.now_playing() else {
            return;
        };
        if self.lyrics_uri.as_deref() == Some(now.uri.as_str())
            && !matches!(self.lyrics, Loadable::NotLoaded | Loadable::Failed(_))
        {
            return;
        }
        self.lyrics_uri = Some(now.uri.clone());
        self.lyrics_following = true;
        self.lyrics_line_shown = None;
        if self.offline {
            self.lyrics = Loadable::Loaded(None);
            return;
        }
        self.lyrics = Loadable::Loading;
        self.backend.send(Command::Lyrics(Box::new(LyricsRequest {
            uri: now.uri,
            query: crate::lyrics::Query {
                artist: now
                    .artists
                    .first()
                    .map(|artist| artist.name.clone())
                    .unwrap_or_default(),
                title: now.title,
                album: now.album_name,
                duration_ms: now.duration_ms,
            },
        })));
    }

    fn tick(&mut self, ctx: &egui::Context) {
        let now = Instant::now();
        if !self.zoom_applied {
            self.zoom_applied = true;
            let zoom = self.settings.zoom.clamp(0.5, 2.5);
            if (zoom - 1.0).abs() > 0.001 {
                ctx.set_zoom_factor(zoom);
            }
        } else {
            let zoom = ctx.zoom_factor();
            if (zoom - self.settings.zoom).abs() > 0.001 {
                // Ctrl+plus/minus zoomed; remembered for the next start.
                self.settings.zoom = zoom;
                self.settings_dirty = true;
            }
        }
        self.toasts
            .retain(|toast| toast.created.elapsed() < TOAST_LIFETIME);

        if self.settings.check_for_updates
            && !self.offline
            && self
                .last_update_check
                .is_none_or(|at| at.elapsed() >= crate::updates::CHECK_INTERVAL)
        {
            self.last_update_check = Some(now);
            self.backend.send(Command::CheckForUpdates);
        }

        if self.is_connected() && !self.offline {
            self.request_resume_track();
            self.ensure_resume_context_loaded();
        }

        if let Some(typed) = self.search.typed_at {
            if typed.elapsed() >= SEARCH_DEBOUNCE {
                self.search.typed_at = None;
                let query = self.search.query.trim().to_string();
                self.run_search(query);
            } else {
                ctx.request_repaint_after(SEARCH_DEBOUNCE - typed.elapsed());
            }
        }

        if self.last_eviction.elapsed() > Duration::from_secs(20) {
            self.last_eviction = now;
            self.backend.art().evict(ctx);
        }
        self.sync_skin(ctx);
        if self.settings_dirty && self.last_settings_save.elapsed() > Duration::from_secs(2) {
            self.save_settings();
        }
        if self.session_dirty && self.last_session_save.elapsed() > Duration::from_secs(2) {
            self.save_session();
        }
    }

    /// Loads and applies the skin selected in settings.
    /// On failure, restores the active skin setting to avoid repeated retries.
    fn sync_skin(&mut self, ctx: &egui::Context) {
        if self.settings.winamp_window
            && !self.winamp.is_loading()
            && self.winamp.worn != self.settings.skin
        {
            match self.settings.skin.clone() {
                None => self.winamp.wear(None, crate::skin::Skin::builtin()),
                Some(name) => self.winamp.load(name, &self.dirs.skins_dir(), ctx),
            }
        }
        if let Some(loaded) = self.winamp.poll() {
            self.skin_loaded(loaded);
        }
        let fetched = self.winamp.presets.poll();
        if let Some(fetched) = fetched {
            match fetched {
                Ok(count) => {
                    self.toast(format!("Added {count} MilkDrop presets"));
                    // Restart the child so it loads the new preset list.
                    #[cfg(feature = "milkdrop")]
                    if let Some(host) = self.milkdrop_host.as_mut()
                        && host.is_running()
                    {
                        host.close();
                    }
                }
                Err(error) => self.toast_error(format!("Couldn't fetch presets: {error}")),
            }
        }
    }

    /// Syncs MilkDrop settings and receives window state and commands.
    #[cfg(feature = "milkdrop")]
    fn sync_milkdrop(&mut self, ctx: &egui::Context) {
        let presets = self.dirs.milkdrop_dir();
        let open = self.settings.milkdrop_open;
        let size = self.settings.milkdrop_size;
        let pos = self.milkdrop_pos;
        let fullscreen = self.settings.milkdrop_fullscreen;
        let fps = self.settings.milkdrop_fps;
        let seconds = self.settings.milkdrop_seconds;
        let scale = self.settings.milkdrop_scale.max(1);
        // Track metadata shown when the song changes.
        let song = self.now_playing().filter(|now| !now.resuming).map(|now| {
            // Title, artist, and album.
            vec![
                now.title.clone(),
                now.subtitle.clone(),
                now.album_name.clone(),
            ]
        });
        if self.milkdrop_host.is_none() {
            let tap = std::sync::Arc::clone(&self.winamp.tap);
            self.milkdrop_host = Some(crate::milkdrop::host::Host::new(tap));
        }
        let poll = {
            let host = self.milkdrop_host.as_mut().expect("the host was just made");
            if open {
                if !host.is_running() {
                    host.open(&presets, size, pos, fullscreen, fps, seconds, scale);
                }
                host.update(fps, seconds, scale);
                host.song(song);
            } else if host.is_running() {
                host.close();
            }
            host.poll()
        };
        if poll.closed {
            self.settings.milkdrop_open = false;
            self.mark_settings_dirty();
        }
        if let Some(size) = poll.size
            && self.settings.milkdrop_size != size
        {
            self.settings.milkdrop_size = size;
            self.mark_settings_dirty();
        }
        if let Some(pos) = poll.pos {
            self.milkdrop_pos = Some(pos);
        }
        for command in poll.commands {
            self.milkdrop_command(&command);
        }
        if let Some(hz) = poll.screen_hz {
            self.learn_screen_hz(hz);
        }
        // Poll the child while the main window is otherwise idle.
        if self.settings.milkdrop_open {
            ctx.request_repaint_after(std::time::Duration::from_millis(300));
        }
    }

    /// Records the MilkDrop screen refresh rate and uses it as the initial FPS.
    /// Later screen changes do not override a configured FPS.
    #[cfg(feature = "milkdrop")]
    fn learn_screen_hz(&mut self, hz: u32) {
        if hz == 0 || self.settings.milkdrop_screen_hz == hz {
            return;
        }
        let first = self.settings.milkdrop_screen_hz == 0
            && self.settings.milkdrop_fps == crate::milkdrop::DEFAULT_FPS;
        self.settings.milkdrop_screen_hz = hz;
        if first {
            self.settings.milkdrop_fps = hz;
        }
        self.mark_settings_dirty();
    }

    /// Applies playback commands received from the MilkDrop window.
    #[cfg(feature = "milkdrop")]
    fn milkdrop_command(&mut self, command: &str) {
        match command {
            "previous" => self.actions.push(Action::Previous),
            "next" => self.actions.push(Action::Next),
            "play-pause" => self.actions.push(Action::TogglePlay),
            "mute" => self.actions.push(Action::ToggleMute),
            "shuffle" => self.actions.push(Action::ToggleShuffle),
            "volume-up" => self.actions.push(Action::VolumeBy(5)),
            "volume-down" => self.actions.push(Action::VolumeBy(-5)),
            _ => {}
        }
    }

    /// Creates a config folder if needed and opens it in the file manager.
    fn open_folder(&mut self, folder: std::path::PathBuf) {
        let opened = std::fs::create_dir_all(&folder).and_then(|()| crate::opener::open(&folder));
        if let Err(error) = opened {
            self.toast_error(format!("Couldn't open {}: {error}", folder.display()));
        }
    }

    /// Applies a loaded skin. Installed files become the selected skin.
    fn skin_loaded(&mut self, loaded: crate::winamp::Loaded) {
        match loaded.result {
            Ok(skin) => {
                self.winamp
                    .wear(Some(loaded.name.clone()), std::sync::Arc::new(skin));
                if loaded.installed {
                    self.toast(format!("Added {} skin", crate::winamp::label(&loaded.name)));
                    self.winamp.list_choices(&self.dirs.skins_dir());
                    self.settings.skin = Some(loaded.name);
                    self.settings_dirty = true;
                }
            }
            Err(error) => {
                self.toast_error(format!("{}: {error}", crate::winamp::label(&loaded.name)));
                if !loaded.installed {
                    self.settings.skin = self.winamp.worn.clone();
                    self.settings_dirty = true;
                }
            }
        }
    }

    /// Sends equalizer settings to the player and marks them for saving.
    fn push_eq(&mut self) {
        if let Ok(mut shared) = self.winamp.eq.lock() {
            *shared = eq_settings(&self.settings);
        }
        self.settings_dirty = true;
    }

    /// Note that a setting changed, so the file is saved shortly.
    pub fn mark_settings_dirty(&mut self) {
        self.settings_dirty = true;
    }

    /// Note that the restorable session changed, so it is saved shortly.
    pub fn note_session_change(&mut self) {
        self.session_dirty = true;
    }

    fn save_settings(&mut self) {
        self.settings_dirty = false;
        self.last_settings_save = Instant::now();
        if self.offline {
            // Demo data must never overwrite the person's real preferences.
            return;
        }
        self.settings.save(&self.dirs.settings_file());
    }

    fn apply_theme(&mut self, ctx: &egui::Context) {
        let dark = ctx.theme() == egui::Theme::Dark;
        if self.applied_dark != Some(dark) {
            self.palette = if dark {
                Palette::dark()
            } else {
                Palette::light()
            };
            theme::apply(ctx, &self.palette);
            self.applied_dark = Some(dark);
            self.accents.clear();
            self.accent_pending.clear();
        }
    }

    fn handle_tray(&mut self) {
        let Some(commands) = self.tray.as_ref().map(TrayService::drain_commands) else {
            return;
        };
        for command in commands {
            match command {
                TrayCommand::Show => self.actions.push(Action::ShowWindow),
                TrayCommand::ShowHide => self.actions.push(if self.window_hidden {
                    Action::ShowWindow
                } else {
                    Action::HideWindow
                }),
                TrayCommand::PlayPause => self.actions.push(Action::TogglePlay),
                TrayCommand::Next => self.actions.push(Action::Next),
                TrayCommand::Previous => self.actions.push(Action::Previous),
                TrayCommand::Quit => self.actions.push(Action::Quit),
            }
        }
    }

    fn handle_control_commands(&mut self) {
        let Some(queue) = &self.control_commands else {
            return;
        };
        let commands: Vec<ControlCommand> =
            std::mem::take(&mut *queue.lock().unwrap_or_else(|p| p.into_inner()));
        for command in commands {
            let playing = self.now_playing().is_some_and(|now| now.playing);
            let action = match command {
                ControlCommand::Show => Some(Action::ShowWindow),
                ControlCommand::PlayPause => Some(Action::TogglePlay),
                ControlCommand::Play => (!playing).then_some(Action::TogglePlay),
                ControlCommand::Pause => playing.then_some(Action::TogglePlay),
                ControlCommand::Next => Some(Action::Next),
                ControlCommand::Previous => Some(Action::Previous),
                ControlCommand::SeekBy(offset) => Some(Action::SeekBy(offset)),
                ControlCommand::VolumeBy(delta) => Some(Action::VolumeBy(delta)),
                ControlCommand::SetVolume(volume) => Some(Action::SetVolume(volume.min(100))),
                ControlCommand::ToggleMute => Some(Action::ToggleMute),
                ControlCommand::ToggleShuffle => Some(Action::ToggleShuffle),
                ControlCommand::CycleRepeat => Some(Action::CycleRepeat),
                ControlCommand::SetShuffle(shuffle) => Some(Action::SetShuffle(shuffle)),
                ControlCommand::SetRepeat(mode) => Some(Action::SetRepeat(mode)),
                ControlCommand::SeekTo(position) => Some(Action::Seek(position)),
                // Nothing playing is nothing to save, so the verb is a
                // no-op rather than an error the client has to handle.
                ControlCommand::ToggleSaved => {
                    self.now_playing().map(|now| Action::ToggleSaved(now.uri))
                }
                ControlCommand::PlayUri(uri) => Some(Action::PlayContext {
                    uri,
                    offset_uri: None,
                    offset_index: None,
                }),
            };
            if let Some(action) = action {
                self.actions.push(action);
            }
        }
    }

    fn handle_media_commands(&mut self) {
        let Some(commands) = self
            .media_controls
            .as_ref()
            .map(MediaService::drain_commands)
        else {
            return;
        };
        for command in commands {
            let playing = self.now_playing().is_some_and(|now| now.playing);
            let action = match command {
                MediaCommand::Play => (!playing).then_some(Action::TogglePlay),
                MediaCommand::Pause | MediaCommand::Stop => playing.then_some(Action::TogglePlay),
                MediaCommand::PlayPause => Some(Action::TogglePlay),
                MediaCommand::Next => Some(Action::Next),
                MediaCommand::Previous => Some(Action::Previous),
                MediaCommand::SeekBy(offset) => Some(Action::SeekBy(offset)),
                MediaCommand::SetPosition {
                    track_uri,
                    position_ms,
                } => self
                    .now_playing()
                    .filter(|now| now.uri == track_uri)
                    .map(|_| Action::Seek(position_ms)),
                MediaCommand::SetVolume(volume) => Some(Action::SetVolume(
                    (volume.clamp(0.0, 1.0) * 100.0).round() as u8,
                )),
                MediaCommand::SetShuffle(shuffle) => Some(Action::SetShuffle(shuffle)),
                MediaCommand::SetRepeat(mode) => Some(Action::SetRepeat(mode)),
                MediaCommand::OpenUri(uri) => Some(Action::PlayContext {
                    uri,
                    offset_uri: None,
                    offset_index: None,
                }),
                MediaCommand::Raise => Some(Action::ShowWindow),
                MediaCommand::Quit => Some(Action::Quit),
            };
            if let Some(action) = action {
                self.actions.push(action);
            }
        }
    }

    fn sync_media_controls(&mut self) {
        let state = match self.now_playing() {
            Some(now) => MediaState {
                playback: if now.playing {
                    Playback::Playing
                } else if now.loading {
                    Playback::Loading
                } else {
                    Playback::Paused
                },
                track: Some(MediaTrack {
                    uri: now.uri.clone(),
                    title: now.title.clone(),
                    artists: now
                        .artists
                        .iter()
                        .map(|artist| artist.name.clone())
                        .collect(),
                    album: now.album_name.clone(),
                    art_url: now.art_url.clone(),
                    duration_ms: now.duration_ms,
                }),
                position_ms: now.position_ms,
                volume: f64::from(now.volume_percent) / 100.0,
                shuffle: now.shuffle,
                repeat: now.repeat,
                can_control: now.can_control,
            },
            None => MediaState::default(),
        };
        if let Some(controls) = &mut self.media_controls {
            controls.update(state);
        }
        let playing = self.now_playing().is_some_and(|now| now.playing);
        if let Some(tray) = &mut self.tray {
            tray.set_playing(playing);
        }
        if let Some(slot) = &self.control_now_playing {
            let snapshot = self.control_snapshot();
            *slot.lock().unwrap_or_else(|p| p.into_inner()) = snapshot;
        }
    }

    /// One line for the control channel's `nowplaying` verb: tab-separated
    /// `state, title, artists, album, position_ms, duration_ms, volume,
    /// shuffle, repeat, art_url, saved, device`, or
    /// [`crate::single_instance::NOTHING_PLAYING`].
    ///
    /// The last three are what a Stream Deck key needs and a media key does
    /// not: something to draw, whether the heart is filled, and where the
    /// sound is coming out. They are appended rather than woven in, so a
    /// script written against the older nine fields still reads correctly.
    fn control_snapshot(&self) -> String {
        let Some(now) = self.now_playing() else {
            return crate::single_instance::NOTHING_PLAYING.to_owned();
        };
        let state = if now.playing { "playing" } else { "paused" };
        // Not every track has been looked up yet; say so rather than
        // claiming an unsaved track the client would draw as a hollow heart
        // and then watch fill in a moment later.
        let saved = match self.is_saved(&now.uri) {
            Some(true) => "yes",
            Some(false) => "no",
            None => "unknown",
        };
        let device = "this computer";
        // Tabs separate the fields, so a tab inside one would shift the rest.
        // This runs every frame, and titles almost never contain one, so the
        // usual answer borrows rather than allocating a copy per field.
        fn clean(text: &str) -> std::borrow::Cow<'_, str> {
            match text.contains('\t') {
                true => std::borrow::Cow::Owned(text.replace('\t', " ")),
                false => std::borrow::Cow::Borrowed(text),
            }
        }
        format!(
            "{state}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{saved}\t{}",
            clean(&now.title),
            clean(&now.subtitle),
            clean(&now.album_name),
            now.position_ms,
            now.duration_ms,
            now.volume_percent,
            if now.shuffle { "on" } else { "off" },
            now.repeat.api_name(),
            clean(now.art_url.as_deref().unwrap_or_default()),
            clean(device),
        )
    }

    // ---- loading ---------------------------------------------------------------

    fn load_playlists(&mut self) {
        if self.library.playlists.is_loading() {
            return;
        }
        self.library.playlists = Loadable::Loading;
        self.library.playlists_next = None;
        self.backend.api(ApiRequest::MyPlaylists { offset: 0 });
    }

    pub fn ensure_loaded(&mut self, page: Page) {
        if !self.is_connected() {
            return;
        }
        match page {
            Page::Home => self.load_home(false),
            Page::TopSongs => self.load_top_songs(false),
            Page::Search => {}
            Page::LikedSongs => {
                if !self.library.liked.loaded_once {
                    self.load_more(Page::LikedSongs);
                }
            }
            Page::Albums => {
                if !self.library.albums.loaded_once {
                    self.load_more(Page::Albums);
                }
            }
            Page::Artists => {
                if !self.library.artists.loaded_once {
                    self.load_more(Page::Artists);
                }
            }
            Page::Playlist(id) => {
                let needs_generation = self
                    .playlist_pages
                    .get(&id)
                    .is_none_or(|page| page.generation == 0);
                if needs_generation {
                    self.load_generation += 1;
                    self.playlist_pages
                        .entry(id.clone())
                        .or_default()
                        .generation = self.load_generation;
                }
                let page = self.playlist_pages.entry(id.clone()).or_default();
                let generation = page.generation;
                if page.playlist.needs_load() {
                    page.playlist = Loadable::Loading;
                    self.backend.api(ApiRequest::Playlist {
                        id: id.clone(),
                        generation,
                    });
                }
                if !page.items.loaded_once && page.items.can_load_more() {
                    page.items.loading = true;
                    self.backend.api(ApiRequest::PlaylistItems {
                        id: id.clone(),
                        offset: 0,
                        generation,
                    });
                    // The disk may hold the whole list already; it is
                    // adopted only if the backend snapshot still matches.
                    self.backend
                        .send(Command::LoadPlaylistCache { id: id.clone() });
                }
            }
            Page::Album(id) => {
                let page = self.album_pages.entry(id.clone()).or_default();
                if page.album.needs_load() {
                    page.album = Loadable::Loading;
                    self.backend.api(ApiRequest::Album { id: id.clone() });
                }
            }
            Page::Artist(id) => {
                let page = self.artist_pages.entry(id.clone()).or_default();
                if page.artist.needs_load() {
                    page.artist = Loadable::Loading;
                    self.backend.api(ApiRequest::Artist { id: id.clone() });
                }
                let filter = page.filter;
                self.load_artist_albums(&id, filter);
                if page_related_needs_load(&self.artist_pages, &id) {
                    if let Some(page) = self.artist_pages.get_mut(&id) {
                        page.related = Loadable::Loading;
                    }
                    self.backend
                        .api(ApiRequest::RelatedArtists { id: id.clone() });
                }
            }
            // The queue page has nothing to load: the engine's last word
            // on the queue is already here.
            Page::Queue | Page::Settings => {}
        }
    }

    fn load_artist_albums(&mut self, id: &str, filter: DiscographyFilter) {
        let Some(page) = self.artist_pages.get_mut(id) else {
            return;
        };
        let list = page.albums.entry(filter.groups().to_string()).or_default();
        if !list.loaded_once && list.can_load_more() {
            list.loading = true;
            self.backend.api(ApiRequest::ArtistAlbums {
                id: id.to_string(),
                groups: filter.groups().to_string(),
                offset: 0,
            });
        }
    }

    fn load_home(&mut self, force: bool) {
        if self.home.requested
            && !force
            && self
                .home
                .loaded_at
                .is_some_and(|at| at.elapsed() < Duration::from_secs(600))
        {
            return;
        }
        self.home.requested = true;
        self.home.loaded_at = Some(Instant::now());
        self.home.generation += 1;
        let generation = self.home.generation;
        if self.home.recently_played.get().is_none() {
            self.home.recently_played = Loadable::Loading;
        }
        if self.home.top_artists.get().is_none() {
            self.home.top_artists = Loadable::Loading;
        }
        if self.home.top_tracks.get().is_none() {
            self.home.top_tracks = Loadable::Loading;
        }
        self.backend.api(ApiRequest::RecentlyPlayed {
            who: RecentsFor::Home,
            generation,
            before: None,
            limit: HOME_RECENTS,
        });
        self.backend.api(ApiRequest::TopArtists { generation });
        self.backend.api(ApiRequest::TopTracks {
            offset: 0,
            full: false,
            generation,
        });
        for shelf in ALBUM_SHELVES {
            if self.home.shelf_mut(shelf).get().is_none() {
                *self.home.shelf_mut(shelf) = Loadable::Loading;
            }
            self.backend
                .api(ApiRequest::AlbumShelf { shelf, generation });
        }
    }

    fn load_top_songs(&mut self, force: bool) {
        if self.home.top_songs_loading || (!force && self.home.top_songs_complete) {
            return;
        }
        self.home.top_songs = Loadable::Loading;
        self.home.top_songs_loading = true;
        self.home.top_songs_complete = false;
        self.home.top_songs_generation += 1;
        self.backend.api(ApiRequest::TopTracks {
            offset: 0,
            full: true,
            generation: self.home.top_songs_generation,
        });
    }

    pub fn load_recents(&mut self, force: bool) {
        if self.recents.loading {
            return;
        }
        if self.recents.complete && !force {
            return;
        }
        if force {
            self.recents.reset();
        }
        self.recents.loading = true;
        self.recents.error = None;
        self.recents_generation = self.recents_generation.wrapping_add(1);
        let generation = self.recents_generation;
        // This endpoint paginates backwards, so its continuation cursor is
        // named `before`.
        let before = self.recents.after.clone();
        self.backend.api(ApiRequest::RecentlyPlayed {
            who: RecentsFor::Panel,
            generation,
            before,
            limit: RECENTS_PAGE,
        });
    }

    pub fn load_more_recents(&mut self) {
        if !self.recents.can_load_more() {
            return;
        }
        self.load_recents(false);
    }

    pub fn reload_recents(&mut self) {
        self.load_recents(true);
    }

    pub fn load_more(&mut self, page: Page) {
        match page {
            Page::LikedSongs => {
                let list = &mut self.library.liked;
                if let Some(offset) = list.next_offset.filter(|_| list.can_load_more()) {
                    list.loading = true;
                    self.backend.api(ApiRequest::SavedTracks { offset });
                }
            }
            Page::Albums => {
                let list = &mut self.library.albums;
                if let Some(offset) = list.next_offset.filter(|_| list.can_load_more()) {
                    list.loading = true;
                    self.backend.api(ApiRequest::SavedAlbums { offset });
                }
            }
            Page::Artists => {
                let list = &mut self.library.artists;
                if list.can_load_more() {
                    list.loading = true;
                    self.backend.api(ApiRequest::FollowedArtists {
                        after: list.after.clone(),
                    });
                }
            }
            Page::Playlist(id) => {
                if let Some(page) = self.playlist_pages.get_mut(&id) {
                    let list = &mut page.items;
                    if let Some(offset) = list.next_offset.filter(|_| list.can_load_more()) {
                        list.loading = true;
                        self.backend.api(ApiRequest::PlaylistItems {
                            id,
                            offset,
                            generation: page.generation,
                        });
                    }
                }
            }
            Page::Album(id) => {
                if let Some(page) = self.album_pages.get_mut(&id) {
                    let list = &mut page.tracks;
                    if let Some(offset) = list.next_offset.filter(|_| list.can_load_more()) {
                        list.loading = true;
                        self.backend.api(ApiRequest::AlbumTracks { id, offset });
                    }
                }
            }
            Page::Home => {
                if let Some(offset) = self.library.playlists_next.take() {
                    self.backend.api(ApiRequest::MyPlaylists { offset });
                }
            }
            _ => {}
        }
    }

    fn reload(&mut self, page: Page) {
        match &page {
            Page::Home => self.load_home(true),
            Page::TopSongs => self.load_top_songs(true),
            Page::LikedSongs => self.library.liked.reset(),
            Page::Albums => self.library.albums.reset(),
            Page::Artists => self.library.artists.reset(),
            Page::Playlist(id) => {
                if let Some(playlist) = self.playlist_pages.get_mut(id) {
                    self.load_generation += 1;
                    playlist.generation = self.load_generation;
                    playlist.items.loading = true;
                    playlist.cache_complete = false;
                    playlist.pending_cache = None;
                    self.backend.api(ApiRequest::Playlist {
                        id: id.clone(),
                        generation: playlist.generation,
                    });
                    self.backend.api(ApiRequest::PlaylistItems {
                        id: id.clone(),
                        offset: 0,
                        generation: playlist.generation,
                    });
                    return;
                }
            }
            Page::Album(id) => {
                self.album_pages.remove(id);
            }
            Page::Artist(id) => {
                self.artist_pages.remove(id);
            }
            // The queue page has nothing to throw away and reload: the
            // engine is the only thing that knows what the queue is.
            _ => {}
        }
        self.ensure_loaded(page);
    }

    /// Rule 5: a chosen row of Next up plays at once, and the rows above
    /// it go with it, as if Next had been pressed down to it. The rows
    /// below stay and the context carries on underneath them.
    ///
    /// One command does it. The old remote path needed a skip per row;
    /// the engine counts the rows the way the panel draws them, so the
    /// index is all it needs — and it publishes the new queue before it
    /// opens the track, which is where the panel's immediacy comes from.
    fn play_queue_item(&mut self, index: usize, uri: String) {
        if self.resume_only() {
            // Nothing is playing and the rows on show are the remembered
            // ones, which the engine has never heard of: play them as a
            // plain list.
            let uris: Vec<String> = self
                .queue
                .queue
                .iter()
                .map(|item| item.uri().to_string())
                .collect();
            if uris.is_empty() {
                return;
            }
            let (uris, index) = cap_uris(uris, index as u32);
            self.play_request(PlayRequest::tracks(uris).starting_at_index(index), false);
            return;
        }
        // The click names a song; if the rows shifted under the pointer,
        // the song wins over the row number.
        let row = match self.queue.queue.get(index) {
            Some(item) if item.uri() == uri => Some(index),
            _ => self.queue.queue.iter().position(|item| item.uri() == uri),
        };
        let Some(row) = row else {
            return;
        };
        self.intent_track = Some((uri.clone(), Instant::now()));
        self.set_play_pending(vec![uri]);
        self.optimistic_playing = Some((true, Instant::now()));
        self.backend.player(PlayerCommand::PlayQueued(row));
    }

    /// How many leading rows of Next up are songs queued by hand, so the
    /// view can give them their own section. The engine draws the line;
    /// before anything has played it is where the remembered queue left
    /// it (rule 9).
    pub fn queued_rows_len(&self) -> usize {
        self.queued_len.min(self.queue.queue.len())
    }

    /// The rows a closed session left, shown before anything plays, of
    /// which the first `queued` were queued by hand (rule 9). They are not
    /// live: no engine is holding them, so there is nothing to ask to
    /// clear. `App::new` restores them from the session file; demo mode,
    /// which has no session file, fabricates the same thing.
    pub(crate) fn set_remembered_queue(&mut self, rows: Vec<PlayableItem>, queued: usize) {
        self.queued_len = queued.min(rows.len());
        self.queue = Queue {
            currently_playing: None,
            queue: rows,
        };
        self.queue_is_live = false;
    }

    /// Whether there is a "Playing next" section to empty. Rule 7: the
    /// trash button shows while there is something of yours to remove —
    /// which needs no playback, only a queue the engine is holding. The
    /// remembered queue from the last session is not one the engine has
    /// heard of, so there is nothing there to ask it to clear.
    pub fn can_clear_queue(&self) -> bool {
        self.queue_is_live && self.queued_rows_len() > 0
    }

    /// Rule 7: Clear empties the songs queued by hand and leaves the
    /// context's own rows alone. The engine does the emptying; the panel
    /// redraws from the queue it publishes for it.
    fn clear_queue(&mut self) {
        self.queue_add_clicks.clear();
        self.queue_names.clear();
        self.backend.player(PlayerCommand::ClearQueue);
        self.toast("Queue cleared");
    }

    /// Current and upcoming track URIs, deduplicated in playback order.
    pub fn queue_playlist_uris(&self) -> Vec<String> {
        let mut seen = std::collections::HashSet::new();
        let mut uris = Vec::new();
        let rows = self
            .now_playing()
            .map(|now| now.uri)
            .into_iter()
            .chain(self.queue.queue.iter().map(|item| item.uri().to_string()));
        for uri in rows {
            if seen.insert(uri.clone()) {
                uris.push(uri);
            }
        }
        uris
    }

    /// Name for a playlist created from the queue.
    pub fn queue_playlist_name(&self) -> String {
        let today = jiff::Zoned::now().strftime("%Y-%m-%d").to_string();
        format!("Queue {today}")
    }

    /// Saves the queue as a new playlist.
    fn save_queue_as_playlist(&mut self) {
        let uris = self.queue_playlist_uris();
        if uris.is_empty() {
            return;
        }
        let name = self.queue_playlist_name();
        self.actions.push(Action::CreatePlaylist {
            name,
            public: false,
            add_uris: uris,
        });
    }

    fn run_search(&mut self, query: String) {
        if query.is_empty() {
            self.search.results = Loadable::NotLoaded;
            self.search.committed.clear();
            return;
        }
        if query == self.search.committed && !self.search.results.needs_load() {
            return;
        }
        self.search.serial += 1;
        self.search.committed = query.clone();
        if self.search.results.get().is_none() {
            self.search.results = Loadable::Loading;
        }
        self.backend.api(ApiRequest::Search {
            query,
            serial: self.search.serial,
        });
    }

    /// Resolves user IDs that do not have a cached display name.
    pub fn request_user_names(&mut self, ids: Vec<String>) {
        // A Subsonic playlist carries its owner's name rather than an id to
        // resolve, so there is nothing to ask anybody. The map is kept
        // because the interface reads through it.
        for id in ids {
            self.user_names.entry(id.clone()).or_insert(Some(id));
        }
    }

    /// Remembers the starred flags an answer stated.
    ///
    /// This is what a `Contains` request used to ask for. The server puts
    /// `starred` on every song, album and artist it sends, so a page that
    /// has its rows already knows which hearts are filled and asks for
    /// nothing (`01-api-mapping.md`, P4.2).
    ///
    /// A heart the user just clicked is still in flight, and an answer that
    /// left the server before the click must not undo it — the
    /// optimistic-UI rule in `AGENTS.md` — so a URI with a change pending
    /// keeps what is on screen.
    fn note_saved(&mut self, flags: Vec<(String, bool)>) {
        for (uri, flag) in flags {
            if self.saved_pending.contains(&uri) {
                continue;
            }
            self.saved.insert(uri, flag);
        }
    }

    // ---- api responses -------------------------------------------------------

    fn handle_api(&mut self, response: ApiResponse) {
        match response {
            ApiResponse::Me(result) => match result {
                Ok(user) => {
                    self.user = Some(user);
                    let page = self.page().clone();
                    self.ensure_loaded(page);
                    if let Some(_now) = self.now_playing() {}
                }
                Err(error) => {
                    if error.is_auth() {
                        self.auth = AuthStatus::Failed(
                            "The server no longer accepts this sign-in. Sign in again.".into(),
                        );
                    } else {
                        self.toast_error(format!("Couldn't load your profile: {error}"));
                    }
                }
            },
            ApiResponse::RecentlyPlayed {
                who,
                generation,
                limit,
                result,
            } => match who {
                RecentsFor::Home => {
                    if generation != self.home.generation {
                        return;
                    }
                    if let Ok(page) = &result {
                        self.note_recent_contexts(&page.items);
                    }
                    let items = result
                        .as_ref()
                        .map(|page| page.items.clone())
                        .map_err(|error| error.to_string());
                    self.home.recently_played.refresh(items);
                }
                RecentsFor::Panel => {
                    if generation != self.recents_generation {
                        return;
                    }
                    self.recents.loading = false;
                    self.recents.loaded_once = true;
                    match result {
                        Ok(page) => self.absorb_recents(page, limit),
                        Err(error) => self.recents.error = Some(error.to_string()),
                    }
                }
            },
            ApiResponse::TopTracks {
                offset,
                full,
                generation,
                result,
            } => {
                if full {
                    if generation != self.home.top_songs_generation {
                        return;
                    }
                    match result {
                        Ok(page) => {
                            let received = page.items.len() as u32;
                            let tracks = page.items;
                            let flags = starred_flags(&tracks);
                            self.note_saved(flags);
                            if offset == 0 {
                                self.home.top_songs = Loadable::Loaded(tracks);
                            } else if let Some(current) = self.home.top_songs.get_mut() {
                                current.extend(tracks);
                            }
                            if page.next.is_some() && received > 0 && offset + received < 100 {
                                self.backend.api(ApiRequest::TopTracks {
                                    offset: offset + received,
                                    full: true,
                                    generation,
                                });
                            } else {
                                self.home.top_songs_loading = false;
                                self.home.top_songs_complete = true;
                            }
                        }
                        Err(error) => {
                            self.home.top_songs.refresh(Err::<Vec<Track>, _>(error));
                            self.home.top_songs_loading = false;
                        }
                    }
                } else if generation == self.home.generation
                    && let Ok(page) = result
                {
                    let tracks = page.items;
                    let flags = starred_flags(&tracks);
                    self.note_saved(flags);
                    self.home.top_tracks = Loadable::Loaded(tracks);
                } else if generation == self.home.generation
                    && offset == 0
                    && let Err(error) = result
                    && self.home.top_tracks.get().is_none()
                {
                    self.home.top_tracks = Loadable::Failed(error.to_string());
                }
            }
            ApiResponse::TopArtists { generation, result } => {
                if generation != self.home.generation {
                    return;
                }
                self.home.top_artists.refresh(result);
            }
            ApiResponse::AlbumShelf {
                shelf,
                generation,
                result,
            } => {
                if generation != self.home.generation {
                    return;
                }
                if let Ok(albums) = &result {
                    let flags = starred_flags(albums);
                    self.note_saved(flags);
                }
                self.home.shelf_mut(shelf).refresh(result);
            }
            ApiResponse::MyPlaylists { offset, result } => match result {
                Ok(page) => {
                    let next_offset = page.next_offset();
                    match &mut self.library.playlists {
                        Loadable::Loaded(existing) if offset > 0 => existing.extend(page.items),
                        slot => *slot = Loadable::Loaded(page.items),
                    }
                    self.library.playlists_next = next_offset;
                    if next_offset.is_some() {
                        self.load_more(Page::Home);
                    }
                    if let Some(playlists) = self.library.playlists.get() {
                        for playlist in playlists {
                            self.saved.insert(playlist.uri.clone(), true);
                        }
                    }
                }
                Err(error) => {
                    if offset == 0 {
                        self.library.playlists = Loadable::Failed(error.to_string());
                    } else {
                        self.toast_error(format!("Couldn't load more playlists: {error}"));
                    }
                }
            },
            ApiResponse::Playlist {
                id,
                generation,
                result,
            } => {
                if self
                    .playlist_pages
                    .get(&id)
                    .is_none_or(|page| page.generation != generation)
                {
                    return;
                }
                if let Ok(playlist) = &result
                    && let Some(image) = pick_image(&playlist.images, 300)
                {
                    self.tint_for(Some(image));
                }
                if let Some(page) = self.playlist_pages.get_mut(&id) {
                    page.playlist.refresh(result);
                }
                self.try_adopt_playlist_cache(&id);
            }
            ApiResponse::PlaylistItems {
                id,
                offset,
                generation,
                result,
            } => {
                if self
                    .playlist_pages
                    .get(&id)
                    .is_none_or(|page| page.generation != generation)
                {
                    return;
                }
                let mut flags = Vec::new();
                let mut adders: Vec<String> = Vec::new();
                if let Some(page) = self.playlist_pages.get_mut(&id) {
                    match result {
                        Ok(_) if page.cache_complete => {
                            // A page in flight from before the cache
                            // adopted; the list is already whole.
                        }
                        Ok(items) => {
                            flags = starred_flags(&items.items);
                            adders = items
                                .items
                                .iter()
                                .filter_map(|item| item.added_by.as_ref()?.id.clone())
                                .filter(|id| !id.is_empty())
                                .collect();
                            page.contributors.extend(adders.iter().cloned());
                            page.items.absorb(offset, items);
                            // The rows load from the top, and songs a friend
                            // added often sit at the end; look there once.
                            if !page.tail_checked {
                                page.tail_checked = true;
                                let loaded = page.items.items.len() as u32;
                                if let Some(total) =
                                    page.items.total.filter(|total| *total > loaded)
                                {
                                    self.backend.api(ApiRequest::PlaylistSample {
                                        id: id.clone(),
                                        offset: total.saturating_sub(PLAYLIST_PAGE_SIZE),
                                        generation,
                                    });
                                }
                            }
                        }
                        Err(error) => page.items.fail(friendly_page_error(&error)),
                    }
                }
                self.note_saved(flags);
                self.request_user_names(adders);
                // The whole list is here; remember it under its snapshot.
                if let Some(page) = self.playlist_pages.get(&id)
                    && page.items.is_complete()
                    && !page.cache_complete
                    && let Some(snapshot) = page
                        .playlist
                        .get()
                        .and_then(|playlist| playlist.snapshot_id.clone())
                {
                    self.backend.send(Command::StorePlaylistCache {
                        id: id.clone(),
                        snapshot,
                        items: page.items.items.clone(),
                    });
                }
                // A sorted table means the whole list, not the loaded part.
                if self.table_sorts.contains_key(&Page::Playlist(id.clone())) {
                    self.load_more(Page::Playlist(id));
                }
            }
            ApiResponse::PlaylistSample {
                id,
                generation,
                result,
            } => {
                if self
                    .playlist_pages
                    .get(&id)
                    .is_none_or(|page| page.generation != generation)
                {
                    return;
                }
                let mut adders: Vec<String> = Vec::new();
                if let Ok(items) = result
                    && let Some(page) = self.playlist_pages.get_mut(&id)
                {
                    adders = items
                        .items
                        .iter()
                        .filter_map(|item| item.added_by.as_ref()?.id.clone())
                        .filter(|id| !id.is_empty())
                        .collect();
                    page.contributors.extend(adders.iter().cloned());
                }
                self.request_user_names(adders);
            }
            ApiResponse::PlaylistCreated(result) => {
                self.playlist_busy = false;
                match result {
                    Ok(playlist) => {
                        self.toast(format!("Created {}", playlist.name));
                        if let Some(playlists) = self.library.playlists.get_mut() {
                            playlists.insert(0, playlist.clone());
                        }
                        self.saved.insert(playlist.uri.clone(), true);
                        if let Some(Dialog::CreatePlaylist { add_uris, .. }) = self.dialog.take()
                            && !add_uris.is_empty()
                        {
                            self.backend.api(ApiRequest::AddToPlaylist {
                                playlist_id: playlist.id.clone(),
                                playlist_name: playlist.name.clone(),
                                uris: add_uris,
                            });
                        }
                        self.open(Page::Playlist(playlist.id));
                    }
                    Err(error) => {
                        self.toast_error(format!("Couldn't create the playlist: {error}"))
                    }
                }
            }
            ApiResponse::PlaylistUpdated { id, result } => {
                self.playlist_busy = false;
                match result {
                    Ok(()) => {
                        self.toast("Playlist updated");
                        self.playlist_pages.remove(&id);
                        self.load_playlists();
                        if matches!(self.page(), Page::Playlist(current) if *current == id) {
                            self.ensure_loaded(Page::Playlist(id));
                        }
                    }
                    Err(error) => {
                        self.toast_error(format!("Couldn't update the playlist: {error}"))
                    }
                }
            }
            ApiResponse::PlaylistItemsChanged {
                id,
                message,
                result,
            } => {
                self.playlist_busy = false;
                match result {
                    Ok(snapshot) => {
                        if !message.is_empty() {
                            self.toast(message);
                        }
                        if let Some(page) = self.playlist_pages.get_mut(&id) {
                            if let Some(playlist) = page.playlist.get_mut()
                                && snapshot.is_some()
                            {
                                playlist.snapshot_id = snapshot;
                            }
                            page.items.reset();
                            page.contributors.clear();
                            page.tail_checked = false;
                            page.cache_complete = false;
                            page.pending_cache = None;
                        }
                        if matches!(self.page(), Page::Playlist(current) if *current == id) {
                            self.ensure_loaded(Page::Playlist(id.clone()));
                        }
                        if let Some(playlists) = self.library.playlists.get_mut() {
                            for playlist in playlists.iter_mut().filter(|p| p.id == id) {
                                playlist.snapshot_id = None;
                            }
                        }
                        self.load_playlists();
                    }
                    Err(error) => {
                        self.toast_error(format!("Playlist change failed: {error}"));
                        if let Some(page) = self.playlist_pages.get_mut(&id) {
                            page.items.reset();
                            page.contributors.clear();
                            page.tail_checked = false;
                            page.cache_complete = false;
                            page.pending_cache = None;
                        }
                        self.ensure_loaded(Page::Playlist(id));
                    }
                }
            }
            ApiResponse::PlaylistDeleted { id, result } => match result {
                Ok(()) => {
                    self.saved.remove(&convert::uri(Kind::Playlist, &id));
                    self.toast("Playlist deleted");
                    self.load_playlists();
                    if matches!(self.page(), Page::Playlist(current) if *current == id) {
                        self.open(Page::Home);
                    }
                }
                Err(error) => {
                    self.toast_error(format!("Couldn't delete the playlist: {error}"));
                    self.load_playlists();
                }
            },
            ApiResponse::SavedTracks { offset, result } => {
                match result {
                    Ok(page) => {
                        for item in &page.items {
                            self.saved.insert(item.track.uri.clone(), true);
                        }
                        self.library.liked.absorb(offset, page);
                    }
                    Err(error) => self.library.liked.fail(error.to_string()),
                }
                // A sorted table means the whole list, not the loaded part.
                if self.table_sorts.contains_key(&Page::LikedSongs) {
                    self.load_more(Page::LikedSongs);
                }
            }
            ApiResponse::SavedAlbums { offset, result } => match result {
                Ok(page) => {
                    for item in &page.items {
                        self.saved.insert(item.album.uri.clone(), true);
                    }
                    self.library.albums.absorb(offset, page);
                }
                Err(error) => self.library.albums.fail(error.to_string()),
            },
            ApiResponse::FollowedArtists { after, result } => {
                let list = &mut self.library.artists;
                list.loading = false;
                list.loaded_once = true;
                match result {
                    Ok(page) => {
                        if after.is_none() {
                            list.items.clear();
                        }
                        let received = page.items.len();
                        for artist in &page.items {
                            self.saved.insert(artist.uri.clone(), true);
                        }
                        list.items.extend(page.items);
                        let next = page.cursors.and_then(|cursors| cursors.after);
                        list.complete = next.is_none() || received == 0;
                        list.after = next;
                        list.error = None;
                    }
                    Err(error) => list.error = Some(error.to_string()),
                }
            }
            ApiResponse::SavedChanged {
                uris,
                saved,
                result,
            } => {
                for uri in &uris {
                    self.saved_pending.remove(uri);
                }
                match result {
                    Ok(()) => {
                        for uri in &uris {
                            self.saved.insert(uri.clone(), saved);
                            match util::uri_kind(uri) {
                                Some("track") => {
                                    if self.library.liked.loaded_once {
                                        if saved {
                                            let total = self
                                                .library
                                                .liked
                                                .total
                                                .map(|total| total.saturating_add(1));
                                            self.library.liked.reset();
                                            self.library.liked.total = total;
                                            if matches!(self.page(), Page::LikedSongs) {
                                                self.load_more(Page::LikedSongs);
                                            }
                                        } else {
                                            self.library
                                                .liked
                                                .items
                                                .retain(|item| item.track.uri != *uri);
                                            if let Some(total) = self.library.liked.total.as_mut() {
                                                *total = total.saturating_sub(1);
                                            }
                                        }
                                    }
                                }
                                Some("album") => self.library.albums.reset(),
                                Some("artist") => self.library.artists.reset(),
                                _ => {}
                            }
                        }
                        let message =
                            match (uris.first().and_then(|uri| util::uri_kind(uri)), saved) {
                                (Some("track"), true) => "Added to Liked Songs",
                                (Some("track"), false) => "Removed from Liked Songs",
                                (Some("artist"), true) => "Artist added to Your Library",
                                (Some("artist"), false) => "Artist removed from Your Library",
                                (_, true) => "Saved to Your Library",
                                (_, false) => "Removed from Your Library",
                            };
                        self.toast(message);
                    }
                    Err(error) => {
                        for uri in &uris {
                            self.saved.insert(uri.clone(), !saved);
                        }
                        self.toast_error(format!("Couldn't update your library: {error}"));
                    }
                }
            }
            ApiResponse::Search {
                query,
                serial,
                result,
            } => {
                if serial != self.search.serial || query != self.search.committed {
                    return;
                }
                if let Ok(results) = &result {
                    let mut flags =
                        starred_flags(results.tracks.iter().flat_map(|page| page.items.iter()));
                    flags.extend(starred_flags(
                        results.albums.iter().flat_map(|page| page.items.iter()),
                    ));
                    flags.extend(starred_flags(
                        results.artists.iter().flat_map(|page| page.items.iter()),
                    ));
                    self.note_saved(flags);
                    self.settings.remember_search(&query);
                    self.settings_dirty = true;
                }
                self.search.results.refresh(result);
            }
            ApiResponse::Artist { id, result } => {
                if let Ok(artist) = &result {
                    if let Some(image) = pick_image(&artist.images, 300) {
                        self.tint_for(Some(image));
                    }
                    if let Some(page) = self.artist_pages.get_mut(&id)
                        && page.top_tracks.needs_load()
                    {
                        page.top_tracks = Loadable::Loading;
                        self.backend
                            .api(ApiRequest::ArtistTopTracks { id: id.clone() });
                    }
                }
                if let Some(page) = self.artist_pages.get_mut(&id) {
                    page.artist = Loadable::from_result(result);
                }
            }
            ApiResponse::ArtistTopTracks { id, result } => {
                if let Ok(tracks) = &result {
                    let flags = starred_flags(tracks);
                    self.note_saved(flags);
                }
                if let Some(page) = self.artist_pages.get_mut(&id) {
                    page.top_tracks = Loadable::from_result(result);
                }
            }
            ApiResponse::ArtistAlbums {
                id,
                groups,
                offset,
                result,
            } => {
                if let Some(page) = self.artist_pages.get_mut(&id) {
                    let list = page.albums.entry(groups).or_default();
                    match result {
                        Ok(albums) => list.absorb(offset, albums),
                        Err(error) => list.fail(error.to_string()),
                    }
                }
            }
            ApiResponse::RelatedArtists { id, result } => {
                if let Some(page) = self.artist_pages.get_mut(&id) {
                    page.related = Loadable::from_result(result);
                }
            }
            ApiResponse::Album { id, result } => {
                let mut flags = Vec::new();
                if let Ok(album) = &result
                    && let Some(image) = pick_image(&album.images, 300)
                {
                    self.tint_for(Some(image));
                }
                if let Some(page) = self.album_pages.get_mut(&id) {
                    match result {
                        Ok(mut album) => {
                            if let Some(tracks) = album.tracks.take() {
                                flags = starred_flags(&tracks.items);
                                page.tracks.absorb(0, tracks);
                            }
                            flags.extend(starred_flags(std::slice::from_ref(&album)));
                            page.album = Loadable::Loaded(album);
                            if !page.tracks.loaded_once {
                                page.tracks.loading = true;
                                self.backend.api(ApiRequest::AlbumTracks { id, offset: 0 });
                            }
                        }
                        Err(error) => page.album = Loadable::Failed(error.to_string()),
                    }
                }
                self.note_saved(flags);
            }
            ApiResponse::AlbumTracks { id, offset, result } => {
                let mut flags = Vec::new();
                if let Some(page) = self.album_pages.get_mut(&id) {
                    match result {
                        Ok(tracks) => {
                            flags = starred_flags(&tracks.items);
                            page.tracks.absorb(offset, tracks);
                        }
                        Err(error) => page.tracks.fail(error.to_string()),
                    }
                }
                self.note_saved(flags);
                // A sorted table means the whole list, not the loaded part.
                if self.table_sorts.contains_key(&Page::Album(id.clone())) {
                    self.load_more(Page::Album(id));
                }
            }
            ApiResponse::Track { id, result } => {
                self.track_requests.remove(&id);
                if let Ok(track) = result {
                    let flags = starred_flags(std::slice::from_ref(&track));
                    self.note_saved(flags);
                    self.track_cache.insert(id, track);
                }
            }
        }
    }

    // ---- navigation ------------------------------------------------------------

    pub fn open(&mut self, page: Page) {
        if *self.page() == page {
            self.ensure_loaded(page);
            return;
        }
        self.history.truncate(self.history_index + 1);
        self.history.push(page.clone());
        if self.history.len() > 60 {
            self.history.remove(0);
        }
        self.history_index = self.history.len() - 1;
        self.ensure_loaded(page);
    }

    pub fn can_go_back(&self) -> bool {
        self.history_index > 0
    }

    pub fn can_go_forward(&self) -> bool {
        self.history_index + 1 < self.history.len()
    }

    // ---- playback --------------------------------------------------------------

    /// Remembers `uri` as the most recently played context, for the
    /// sidebar's order.
    fn note_recent_context(&mut self, uri: &str) {
        self.session_dirty = true;
        if !uri.contains(":playlist:") && !uri.contains(":album:") && !uri.contains(":collection") {
            return;
        }
        self.recent_contexts.retain(|held| held != uri);
        self.recent_contexts.insert(0, uri.to_string());
        self.recent_contexts.truncate(60);
    }

    /// Notes every context in a page of play history, oldest first, so
    /// the newest ends up at the front of the sidebar's order.
    fn note_recent_contexts(&mut self, history: &[crate::api::models::PlayHistory]) {
        let contexts: Vec<String> = history
            .iter()
            .rev()
            .filter_map(|play| play.context.as_ref().map(|context| context.uri.clone()))
            .collect();
        for context in contexts {
            self.note_recent_context(&context);
        }
    }

    /// Merges local and server history for the Recent tab.
    pub(crate) fn rebuild_recents(&mut self) {
        self.recents_view = crate::history::merged(self.plays.plays(), &self.recents.items);
    }

    /// Adds a page of play history to the Recents tab.
    ///
    /// Repeated plays remain separate. Only identical track-and-time entries
    /// are deduplicated across page boundaries.
    ///
    /// The server paginates backwards; a short page ends the list.
    fn absorb_recents(
        &mut self,
        page: crate::api::models::CursorPage<crate::api::models::PlayHistory>,
        limit: u32,
    ) {
        self.note_recent_contexts(&page.items);
        self.recents.error = None;
        let short_page = (page.items.len() as u32) < limit;
        let cursor = page.cursors.as_ref().and_then(|c| c.before.clone());
        let mut seen: std::collections::HashSet<(String, Option<String>)> = self
            .recents
            .items
            .iter()
            .map(|play| (play.track.uri.clone(), play.played_at.clone()))
            .collect();
        let fresh: Vec<crate::api::models::PlayHistory> = page
            .items
            .into_iter()
            .filter(|play| seen.insert((play.track.uri.clone(), play.played_at.clone())))
            .collect();
        let flags = starred_flags(&fresh);
        self.recents.items.extend(fresh);
        match cursor {
            Some(cursor) if !short_page => self.recents.after = Some(cursor),
            _ => {
                self.recents.complete = true;
                self.recents.after = None;
            }
        }
        self.note_saved(flags);
        self.rebuild_recents();
    }

    /// Loaded track URIs for a context, in display order.
    fn context_track_uris(&self, context_uri: &str) -> Option<Vec<String>> {
        let uris: Vec<String> = if let Some(id) = convert::id_of(context_uri, Kind::Playlist) {
            self.playlist_pages
                .get(id)?
                .items
                .items
                .iter()
                .filter_map(|item| item.playable())
                .map(|item| item.uri().to_string())
                .collect()
        } else if let Some(id) = convert::id_of(context_uri, Kind::Album) {
            self.album_pages
                .get(id)?
                .tracks
                .items
                .iter()
                .map(|track| track.uri.clone())
                .collect()
        } else if context_uri == COLLECTION_URI {
            self.library
                .liked
                .items
                .iter()
                .map(|item| item.track.uri.clone())
                .collect()
        } else {
            return None;
        };
        (!uris.is_empty()).then_some(uris)
    }

    fn random_track_in(&self, context_uri: &str) -> Option<String> {
        let uris = self.context_track_uris(context_uri)?;
        Some(uris[rand::random_range(0..uris.len())].clone())
    }

    /// Where a shuffled play of `context_uri` begins, as an offset by
    /// track URI: a random one of the rows the app holds. When it holds none,
    /// the engine receives no offset and chooses from the context itself.
    fn shuffle_start(&self, context_uri: &str) -> (Option<String>, Option<u32>) {
        if let Some(uri) = self.random_track_in(context_uri) {
            return (Some(uri), None);
        }
        (None, None)
    }

    /// With `shuffle_first`, shuffle is turned on before playback starts,
    /// in one ordered exchange: two independent requests race, and shuffle
    /// sometimes lost.
    fn play_request(&mut self, request: PlayRequest, shuffle_first: bool) {
        // Shuffle applies across contexts until disabled. A selected row still
        // starts first; otherwise choose a random starting track.
        let mut request = request;
        if shuffle_first {
            self.shuffle_wanted = true;
            self.shuffle_set_at = Some(Instant::now());
        }
        let shuffle = shuffle_first || self.shuffle_wanted;
        if shuffle
            && request.offset_uri.is_none()
            && request.offset_position.is_none()
            && request.uris.is_empty()
            && let Some(context) = request.context_uri.clone()
        {
            (request.offset_uri, request.offset_position) = self.shuffle_start(&context);
        }
        let mut keys: Vec<String> = Vec::new();
        if let Some(context) = &request.context_uri {
            keys.push(context.clone());
        }
        if let Some(offset) = &request.offset_uri {
            keys.push(offset.clone());
        }
        match request.offset_position {
            // The play starts at a chosen row; only that row is starting.
            Some(position) => {
                if let Some(uri) = request.uris.get(position as usize) {
                    keys.push(uri.clone());
                }
            }
            // No chosen row: the list starts at its first song.
            None if request.offset_uri.is_none() => {
                if let Some(first) = request.uris.first() {
                    keys.push(first.clone());
                }
            }
            None => {}
        }
        // A resumed session starts at the album's row (`offset_uri`) with a
        // queued song playing over it, and the song is what the player bar
        // owes the moment Play is pressed.
        if let Some(current) = &request.restore_current {
            keys.insert(0, current.clone());
        }
        self.intent_track = keys
            .iter()
            .find(|key| key.contains(":track:"))
            .cloned()
            .map(|uri| (uri, Instant::now()));
        self.set_play_pending(keys);
        if let Some(context) = request.context_uri.clone() {
            self.note_recent_context(&context);
            // Show the context as playing before the engine confirms it.
            self.assumed_context = Some(AssumedContext {
                uri: context,
                shuffle: shuffle.then_some(true),
                at: Instant::now(),
            });
        }
        if !self.local.connected {
            self.queued_play = Some(request);
            return;
        }
        self.queued_play = None;
        let load = local_load(&request, shuffle);
        self.local_list = load.context_uri.is_none().then(|| load.uris.clone());
        let shuffle_after = shuffle && load.shuffle.is_none() && !load.uris.is_empty();
        self.backend.player(PlayerCommand::Load(load));
        if shuffle_after {
            self.backend.player(PlayerCommand::Shuffle(true));
        }
        self.optimistic_playing = Some((true, Instant::now()));
    }

    /// Adopt a playlist's disk cache once both it and the live playlist
    /// are here and the backend snapshot still matches; a stale cache is
    /// discarded, never shown.
    fn try_adopt_playlist_cache(&mut self, id: &str) {
        let mut flags = Vec::new();
        let mut adders: Vec<String> = Vec::new();
        if let Some(page) = self.playlist_pages.get_mut(id) {
            let Some(snapshot_now) = page
                .playlist
                .get()
                .and_then(|playlist| playlist.snapshot_id.clone())
            else {
                return;
            };
            match &page.pending_cache {
                Some((held, _)) if *held == snapshot_now => {}
                Some(_) => {
                    // The playlist changed since; the cache is history.
                    page.pending_cache = None;
                    return;
                }
                None => return,
            }
            if page.items.is_complete() || page.cache_complete {
                page.pending_cache = None;
                return;
            }
            let Some((_, items)) = page.pending_cache.take() else {
                return;
            };
            flags = starred_flags(&items);
            adders = items
                .iter()
                .filter_map(|item| item.added_by.as_ref()?.id.clone())
                .filter(|id| !id.is_empty())
                .collect();
            page.contributors.extend(adders.iter().cloned());
            page.items.set_cached(items);
            page.cache_complete = true;
        }
        self.note_saved(flags);
        self.request_user_names(adders);
    }

    /// Play what was playing when the app last closed, with the queue that
    /// was underneath it. `false` when nothing is known to resume.
    ///
    /// Rule 9: the songs queued by hand travel with this play rather than
    /// being added once it has started, so the engine's first answer is the
    /// whole queue and the panel never draws a resumed session short.
    fn resume_last(&mut self) -> bool {
        let Some(track) = self.resume_track.clone() else {
            return false;
        };
        let queued = std::mem::take(&mut self.resume_queue);
        self.session_dirty = true;
        let mut request = match self.resume_context.clone() {
            Some(context) => {
                // The album goes back to the row it was on, which is not
                // the remembered track when a queued song was playing over
                // the top of it.
                let at = self
                    .resume_context_track
                    .clone()
                    .filter(|at| *at != track)
                    .unwrap_or_else(|| track.clone());
                let mut request = PlayRequest::context(context).starting_at_uri(at.clone());
                if at != track {
                    request.restore_current = Some(track);
                }
                request
            }
            None => PlayRequest::tracks(vec![track]),
        };
        request.restore_queued = queued;
        request.position_ms = self.resume_position_ms;
        self.play_request(request, false);
        true
    }

    fn toggle_play(&mut self) {
        let playing = self.now_playing().map(|now| now.playing);
        if self.local.is_active() {
            self.backend.player(PlayerCommand::Toggle);
        } else {
            if !self.resume_last() {
                self.toast("Pick something to play");
            }
            return;
        }
        if let Some(playing) = playing {
            self.optimistic_playing = Some((!playing, Instant::now()));
        }
    }

    fn seek(&mut self, position_ms: u32) {
        // Dragging the bar under the remembered song moves the point a press
        // of play will resume from; there is no stream to seek yet.
        if self.now_playing_live().is_none() && self.resume_track.is_some() {
            self.resume_position_ms = position_ms;
            self.session_dirty = true;
            return;
        }
        self.backend.player(PlayerCommand::Seek(position_ms));
    }

    /// The volume this side set that the engine has yet to confirm, if the
    /// hold is still good. Clears itself once the engine agrees or it expires.
    fn held_local_volume(&mut self, reported: u16) -> Option<u16> {
        match self.pending_local_volume {
            Some((volume, at)) if volume != reported && at.elapsed() < OPTIMISTIC_HOLD => {
                Some(volume)
            }
            _ => {
                self.pending_local_volume = None;
                None
            }
        }
    }

    /// `settle` is false while the slider is still moving: the level is heard
    /// at once, and the engine is told where it ended up on release.
    fn set_volume(&mut self, percent: u8, settle: bool) {
        let percent = percent.min(100);
        let volume = percent_to_volume(percent);
        self.local.volume = volume;
        self.pending_local_volume = Some((volume, Instant::now()));
        if self.settings.volume != volume {
            self.settings.volume = volume;
            self.settings_dirty = true;
        }
        self.backend.player(if settle {
            PlayerCommand::Volume(volume)
        } else {
            PlayerCommand::VolumePreview(volume)
        });
    }

    fn set_shuffle(&mut self, shuffle: bool) {
        self.shuffle_wanted = shuffle;
        self.shuffle_set_at = Some(Instant::now());
        self.session_dirty = true;
        if let Some(assumed) = &mut self.assumed_context {
            assumed.shuffle = Some(shuffle);
        }
        self.local.shuffle = shuffle;
        self.backend.player(PlayerCommand::Shuffle(shuffle));
    }

    fn set_repeat(&mut self, mode: RepeatMode) {
        self.local.repeat = mode;
        self.backend.player(PlayerCommand::Repeat(mode));
    }

    /// Adds a row to Next up immediately, before the context's upcoming rows.
    fn add_to_queue(&mut self, uri: String, label: String) {
        self.queue_one(uri, label, true);
    }

    /// Rule 2: one song, queued after the songs queued before it and
    /// ahead of the album's own rows. The engine puts it there and says so;
    /// all this owes is the ask, the name to draw until the server
    /// describes the row, and one toast.
    ///
    /// `announce` is false when a batch should produce one toast.
    fn queue_one(&mut self, uri: String, label: String, announce: bool) {
        if convert::id_of(&uri, Kind::Track).is_none() {
            // Nothing else can be queued: an album or a playlist is played
            // rather than queued.
            return;
        }
        // One double-click is one wish; two separate asks are two rows.
        self.queue_add_clicks
            .retain(|(_, at)| at.elapsed() < QUEUE_ADD_DEBOUNCE);
        if self
            .queue_add_clicks
            .iter()
            .any(|(clicked, _)| *clicked == uri)
        {
            return;
        }
        self.queue_add_clicks.push((uri.clone(), Instant::now()));
        if !label.is_empty() {
            self.queue_names.insert(uri.clone(), label.clone());
        }
        if announce {
            self.toast(format!("{label} will play next"));
        }
        self.backend.player(PlayerCommand::AddToQueue(uri));
    }

    fn set_saved(&mut self, uri: String, saved: bool) {
        self.saved.insert(uri.clone(), saved);
        // Held until the server answers, so that a page loading underneath
        // cannot draw the flag the click just changed (see `note_saved`).
        self.saved_pending.insert(uri.clone());
        self.backend.api(ApiRequest::SetSaved {
            uris: vec![uri],
            saved,
        });
    }

    // ---- actions -----------------------------------------------------------------

    fn apply_actions(&mut self, ctx: &egui::Context) {
        let mut actions = std::mem::take(&mut self.actions);
        while !actions.is_empty() {
            for action in actions.drain(..) {
                self.apply(action, ctx);
            }
            actions = std::mem::take(&mut self.actions);
        }
    }

    fn apply(&mut self, action: Action, ctx: &egui::Context) {
        match action {
            Action::Open(page) => self.open(page),
            Action::OpenUri(uri) => {
                if let Some(page) = Page::from_uri(&uri) {
                    self.open(page);
                }
            }
            Action::Back => {
                if self.can_go_back() {
                    self.history_index -= 1;
                    let page = self.page().clone();
                    self.ensure_loaded(page);
                }
            }
            Action::Forward => {
                if self.can_go_forward() {
                    self.history_index += 1;
                    let page = self.page().clone();
                    self.ensure_loaded(page);
                }
            }
            Action::PlayContext {
                uri,
                offset_uri,
                offset_index,
            } => {
                let mut request = PlayRequest::context(uri);
                request.offset_uri = offset_uri;
                request.offset_position = offset_index;
                self.play_request(request, false);
            }
            Action::PlayUris { uris, index } => {
                if uris.is_empty() {
                    return;
                }
                let (uris, index) = cap_uris(uris, index);
                let request = PlayRequest::tracks(uris).starting_at_index(index);
                self.play_request(request, false);
            }
            Action::PlayFromRow {
                context,
                uri,
                index,
            } => match context {
                RowContext::Context {
                    uri: context_uri, ..
                } => {
                    let request = PlayRequest::context(context_uri).starting_at_uri(uri);
                    self.play_request(request, false);
                }
                RowContext::Uris(uris) => {
                    let (uris, index) = cap_uris(uris, index);
                    let request = PlayRequest::tracks(uris).starting_at_index(index);
                    self.play_request(request, false);
                }
                RowContext::Queue => self.play_queue_item(index as usize, uri),
                RowContext::View { uris, context_uri } => {
                    let (uris, index) = cap_uris(uris, index);
                    let request = PlayRequest::tracks(uris).starting_at_index(index);
                    self.play_request(request, false);
                    self.note_recent_context(&context_uri);
                    self.assumed_context = Some(AssumedContext {
                        uri: context_uri,
                        shuffle: None,
                        at: Instant::now(),
                    });
                }
            },
            Action::ShufflePlay(uri) => {
                // The random starting song is picked in `play_request`,
                // which every shuffled play goes through.
                self.play_request(PlayRequest::context(uri), true);
            }
            Action::TogglePlay => self.toggle_play(),
            Action::Next if self.resume_only() => {
                self.step_resume(true);
            }
            // Rule 4: the top row is gone the moment Next is pressed. The
            // engine takes it off the queue and publishes before it opens
            // the track, so nothing here has to guess at it.
            Action::Next => self.backend.player(PlayerCommand::Next),
            // Previous restarts after three seconds and otherwise steps back,
            // matching librespot.
            Action::Previous if self.resume_only() => {
                if self.resume_position_ms > RESTART_BEFORE_PREVIOUS {
                    self.resume_position_ms = 0;
                    self.session_dirty = true;
                } else {
                    self.step_resume(false);
                }
            }
            Action::Previous => self.backend.player(PlayerCommand::Previous),
            Action::Seek(position_ms) => self.seek(position_ms),
            Action::SeekBy(offset) => {
                if let Some(now) = self.now_playing() {
                    let target = (i64::from(now.position_ms) + offset)
                        .clamp(0, i64::from(now.duration_ms))
                        as u32;
                    self.seek(target);
                }
            }
            Action::SetVolume(percent) => {
                self.volume_before_mute = None;
                self.set_volume(percent, true);
            }
            Action::PreviewVolume(percent) => self.set_volume(percent, false),
            Action::VolumeBy(delta) => {
                if let Some(now) = self.now_playing() {
                    let next =
                        (i16::from(now.volume_percent) + i16::from(delta)).clamp(0, 100) as u8;
                    self.volume_before_mute = None;
                    self.set_volume(next, true);
                } else if self.is_connected() {
                    let current = volume_to_percent(self.local.volume);
                    let next = (i16::from(current) + i16::from(delta)).clamp(0, 100) as u8;
                    self.set_volume(next, true);
                }
            }
            Action::ToggleMute => {
                let current = self
                    .now_playing()
                    .map(|now| now.volume_percent)
                    .unwrap_or_else(|| volume_to_percent(self.local.volume));
                if current == 0 {
                    let restore = self.volume_before_mute.take().unwrap_or(50).max(5);
                    self.set_volume(restore, true);
                } else {
                    self.volume_before_mute = Some(current);
                    self.set_volume(0, true);
                }
            }
            Action::ToggleShuffle => {
                let shuffle = self.now_playing().is_some_and(|now| now.shuffle);
                self.set_shuffle(!shuffle);
            }
            Action::SetShuffle(shuffle) => self.set_shuffle(shuffle),
            Action::CycleRepeat => {
                let mode = self.now_playing().map(|now| now.repeat).unwrap_or_default();
                self.set_repeat(mode.next());
            }
            Action::SetRepeat(mode) => self.set_repeat(mode),
            Action::AddToQueue { uri, label } => self.add_to_queue(uri, label),
            Action::QueueMany { songs } => {
                let count = songs.len();
                for (uri, label) in songs {
                    self.queue_one(uri, label, false);
                }
                self.toast(match count {
                    1 => "1 song will play next".to_string(),
                    count => format!("{count} songs will play next"),
                });
            }
            Action::SetSavedMany { uris, saved } => {
                for uri in &uris {
                    self.saved.insert(uri.clone(), saved);
                    self.saved_pending.insert(uri.clone());
                }
                if !uris.is_empty() {
                    self.backend.api(ApiRequest::SetSaved { uris, saved });
                }
            }
            Action::ToggleSaved(uri) => {
                let saved = self.saved.get(&uri).copied().unwrap_or(false);
                self.set_saved(uri, !saved);
            }
            Action::AddToPlaylist {
                playlist_id,
                playlist_name,
                uris,
            } => {
                self.playlist_busy = true;
                self.backend.api(ApiRequest::AddToPlaylist {
                    playlist_id,
                    playlist_name,
                    uris,
                });
            }
            Action::RemoveFromPlaylist { playlist_id, uris } => {
                let snapshot_id = self
                    .playlist_pages
                    .get(&playlist_id)
                    .and_then(|page| page.playlist.get())
                    .and_then(|playlist| playlist.snapshot_id.clone());
                if let Some(page) = self.playlist_pages.get_mut(&playlist_id) {
                    page.items.retain(|item| {
                        item.playable()
                            .is_none_or(|playable| !uris.iter().any(|uri| uri == playable.uri()))
                    });
                }
                self.playlist_busy = true;
                self.backend.api(ApiRequest::RemoveFromPlaylist {
                    playlist_id,
                    uris,
                    snapshot_id,
                });
            }
            Action::MoveInPlaylist {
                playlist_id,
                from,
                to,
            } => {
                let snapshot_id = self
                    .playlist_pages
                    .get(&playlist_id)
                    .and_then(|page| page.playlist.get())
                    .and_then(|playlist| playlist.snapshot_id.clone());
                if let Some(page) = self.playlist_pages.get_mut(&playlist_id) {
                    page.items.reorder(from as usize, to as usize);
                }
                self.playlist_busy = true;
                self.backend.api(ApiRequest::ReorderPlaylist {
                    playlist_id,
                    range_start: from,
                    insert_before: to,
                    snapshot_id,
                });
            }
            Action::ShowDialog(dialog) => self.dialog = Some(dialog),
            Action::CloseDialog => self.dialog = None,
            Action::CreatePlaylist {
                name,
                public,
                add_uris,
            } => {
                self.playlist_busy = true;
                self.dialog = Some(Dialog::CreatePlaylist {
                    name: name.clone(),
                    public,
                    add_uris,
                });
                self.backend.api(ApiRequest::CreatePlaylist {
                    name,
                    public,
                    description: String::new(),
                });
            }
            Action::UpdatePlaylist {
                id,
                name,
                description,
                public,
            } => {
                self.dialog = None;
                self.playlist_busy = true;
                self.backend.api(ApiRequest::UpdatePlaylist {
                    id,
                    name: Some(name),
                    description: Some(description),
                    public: Some(public),
                });
            }
            Action::DeletePlaylist(id) => {
                self.dialog = None;
                self.saved.remove(&convert::uri(Kind::Playlist, &id));
                if let Some(playlists) = self.library.playlists.get_mut() {
                    playlists.retain(|playlist| playlist.id != id);
                }
                self.backend.api(ApiRequest::DeletePlaylist { id });
            }
            Action::ClearQueue => self.clear_queue(),
            Action::SaveQueueAsPlaylist => self.save_queue_as_playlist(),
            Action::CopyLink(uri) => {
                ctx.copy_text(uri);
                self.toast("Link copied");
            }
            Action::Search(query) => {
                self.search.query = query.clone();
                self.search.typed_at = None;
                self.open(Page::Search);
                self.run_search(query.trim().to_string());
            }
            Action::SetSearchFilter(filter) => self.search.filter = filter,
            Action::FocusSearch => {
                self.search.focus_requested = true;
                if !matches!(self.page(), Page::Search) {
                    self.open(Page::Search);
                }
            }
            Action::LoadMore(page) => self.load_more(page),
            Action::LoadMoreRecents => self.load_more_recents(),
            Action::ReloadRecents => self.reload_recents(),
            Action::SetQueueTab(tab) => {
                self.queue_tab = tab;
                self.session_dirty = true;
                if tab == QueueTab::Recents
                    && self.recents.items.is_empty()
                    && !self.recents.loading
                {
                    self.load_recents(false);
                }
            }
            Action::LoadMoreArtistAlbums(id) => {
                let Some(page) = self.artist_pages.get_mut(&id) else {
                    return;
                };
                let groups = page.filter.groups().to_string();
                let list = page.albums.entry(groups.clone()).or_default();
                if let Some(offset) = list.next_offset.filter(|_| list.can_load_more()) {
                    list.loading = true;
                    self.backend
                        .api(ApiRequest::ArtistAlbums { id, groups, offset });
                }
            }
            Action::SetDiscographyFilter { artist_id, filter } => {
                if let Some(page) = self.artist_pages.get_mut(&artist_id) {
                    page.filter = filter;
                }
                self.load_artist_albums(&artist_id, filter);
            }
            Action::ToggleShowAllTop(id) => {
                if let Some(page) = self.artist_pages.get_mut(&id) {
                    page.show_all_top = !page.show_all_top;
                }
            }
            Action::Reload(page) => self.reload(page),
            Action::SignIn => self.sign_in(),
            Action::CancelSignIn => {
                self.backend.send(Command::CancelSignIn);
                self.auth = AuthStatus::SignedOut;
            }
            Action::SignOut => {
                self.backend.send(Command::SignOut);
                self.history = vec![Page::Home];
                self.history_index = 0;
            }
            Action::ToggleSidebar => {
                self.settings.sidebar_visible = !self.settings.sidebar_visible;
                self.settings_dirty = true;
            }
            Action::ToggleQueuePanel => {
                self.show_queue_panel = !self.show_queue_panel;
                if self.show_queue_panel {
                    self.show_lyrics_panel = false;
                }
            }
            Action::ToggleLyricsPanel => {
                self.show_lyrics_panel = !self.show_lyrics_panel;
                if self.show_lyrics_panel {
                    self.show_queue_panel = false;
                    self.lyrics_following = true;
                    self.request_lyrics();
                }
            }
            Action::SettingsChanged => {
                self.settings_dirty = true;
                ctx.set_theme(match self.settings.theme {
                    ThemeChoice::Dark => egui::ThemePreference::Dark,
                    ThemeChoice::Light => egui::ThemePreference::Light,
                    ThemeChoice::System => egui::ThemePreference::System,
                });
            }
            Action::RestartEngine => {
                self.save_settings();
                let config = engine_config(
                    &self.settings,
                    std::sync::Arc::clone(&self.winamp.tap),
                    std::sync::Arc::clone(&self.winamp.eq),
                    self.audio_cache.clone(),
                );
                self.backend.send(Command::RestartEngine(config));
                // The queue lives in the engine and this replaces it, but
                // the new engine is handed the old one's queue before it is
                // told anything else (rule 9, `Worker::restart_engine`), so
                // the rows on show stay true and a click on one still
                // counts the rows the engine has.
                if self.local_ready {
                    self.toast("Restarting local playback");
                }
            }
            Action::ShowWindow => {
                if self.window_hidden {
                    // No window exists; the outer loop creates one.
                    self.wants_show = true;
                } else {
                    ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
                }
            }
            Action::HideWindow => {
                if self.tray.is_some() {
                    self.hide_intent = true;
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                }
            }
            Action::OpenUrl(url) => {
                // Open links off the UI thread with the platform-specific
                // launcher used elsewhere in the app.
                std::thread::spawn(move || {
                    if let Err(error) = crate::opener::open(&url) {
                        log::warn!("unable to open {url}: {error}");
                    }
                });
            }
            Action::ClearPlayHistory => {
                self.plays.clear();
                self.save_plays();
                self.rebuild_recents();
                self.toast("Play history cleared".to_string());
            }
            Action::ClearArtCache => match self.backend.art().clear_disk_cache() {
                Ok(bytes) => {
                    ctx.forget_all_images();
                    self.toast(format!(
                        "Cleared {:.1} MB of artwork",
                        bytes as f64 / 1_048_576.0
                    ));
                }
                Err(error) => self.toast_error(format!("Couldn't clear artwork: {error}")),
            },
            Action::ClearAudioCache => match &self.audio_cache {
                Some(cache) => {
                    let bytes = cache.clear();
                    self.toast(format!(
                        "Cleared {:.1} MB of audio",
                        bytes as f64 / 1_048_576.0
                    ));
                }
                // The switch above it is off, so there is nothing on disk
                // this app put there.
                None => self.toast("The audio cache is switched off"),
            },
            Action::ToggleWinampWindow => {
                // One window at a time: this one closes and the loop in
                // `main` opens the other kind where each was last.
                if self.settings.winamp_window {
                    self.winamp.remember_position();
                } else {
                    self.session_window_size = self.last_window_size.or(self.session_window_size);
                    self.session_window_pos = self.last_window_pos.or(self.session_window_pos);
                }
                self.settings.winamp_window = !self.settings.winamp_window;
                self.settings_dirty = true;
                self.switch_intent = true;
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
            Action::SetSkin(name) => {
                self.settings.skin = name;
                self.settings_dirty = true;
            }
            Action::InstallSkin(path) => {
                self.winamp.install(path, &self.dirs.skins_dir(), ctx);
            }
            Action::SetSkinScale(scale) => {
                self.settings.skin_scale = Some(scale);
                self.settings_dirty = true;
            }
            Action::ToggleWinampOnTop => {
                self.settings.winamp_on_top = !self.settings.winamp_on_top;
                self.settings_dirty = true;
            }
            Action::ToggleWinampPlaylist => {
                self.settings.playlist_open = !self.settings.playlist_open;
                self.settings_dirty = true;
            }
            Action::SetPlaylistHeight(height) => {
                self.settings.playlist_height = height.clamp(
                    crate::skin::layout::PLAYLIST_MIN_HEIGHT,
                    crate::skin::layout::PLAYLIST_MAX_HEIGHT,
                );
                self.settings_dirty = true;
            }
            Action::ToggleWinampEq => {
                self.settings.eq_open = !self.settings.eq_open;
                self.settings_dirty = true;
            }
            Action::ToggleEq => {
                self.settings.eq_on = !self.settings.eq_on;
                self.push_eq();
            }
            Action::SetEqBand(band, gain_db) => {
                if let Some(slot) = self.settings.eq_bands_db.get_mut(band) {
                    *slot = gain_db.clamp(-crate::eq::RANGE_DB, crate::eq::RANGE_DB);
                    self.push_eq();
                }
            }
            Action::SetEqPreamp(gain_db) => {
                self.settings.eq_preamp_db =
                    gain_db.clamp(-crate::eq::RANGE_DB, crate::eq::RANGE_DB);
                self.push_eq();
            }
            Action::ApplyEqPreset(index) => {
                if let Some(preset) = crate::eq::PRESETS.get(index) {
                    self.settings.eq_bands_db = preset.bands_db;
                    self.settings.eq_on = true;
                    self.push_eq();
                }
            }
            Action::SetBalance(balance) => {
                self.settings.balance = balance.clamp(-1.0, 1.0);
                self.push_eq();
            }
            Action::ToggleMono => {
                self.settings.mono = !self.settings.mono;
                self.push_eq();
            }
            Action::ToggleWinampShade => {
                self.settings.winamp_shaded = !self.settings.winamp_shaded;
                self.settings_dirty = true;
            }
            Action::ToggleWinampPlaylistShade => {
                self.settings.playlist_shaded = !self.settings.playlist_shaded;
                self.settings_dirty = true;
            }
            Action::ToggleWinampEqShade => {
                self.settings.eq_shaded = !self.settings.eq_shaded;
                self.settings_dirty = true;
            }
            // The same request the window's own close button makes, so the
            // close-to-tray setting decides what follows.
            Action::CloseWindow => ctx.send_viewport_cmd(egui::ViewportCommand::Close),
            Action::CycleVisualiser => {
                self.settings.vis = self.settings.vis.next();
                self.settings_dirty = true;
                self.winamp.analyser.reset();
            }
            Action::SetVisualiser(mode) => {
                if self.settings.vis != mode {
                    self.settings.vis = mode;
                    self.settings_dirty = true;
                    self.winamp.analyser.reset();
                }
            }
            Action::OpenSkinsFolder => self.open_folder(self.dirs.skins_dir()),
            Action::ToggleWinampMilkdrop => {
                self.settings.milkdrop_open = !self.settings.milkdrop_open;
                self.settings_dirty = true;
                #[cfg(feature = "milkdrop")]
                if self.settings.milkdrop_open {
                    // A first open has nothing to draw but the idle preset,
                    // which hardly answers the music; fetch the packs in the
                    // background and the window fills up on its own.
                    let folder = self.dirs.milkdrop_dir();
                    self.winamp.presets.refresh(&folder);
                    if self.winamp.presets.count() == 0
                        && self.winamp.presets.downloading().is_none()
                    {
                        self.winamp.presets.download_missing(folder, ctx.clone());
                        self.toast("Downloading MilkDrop preset packs");
                    }
                }
            }
            Action::SetMilkdropSeconds(seconds) => {
                self.settings.milkdrop_seconds = seconds.clamp(1, 3600);
                self.settings_dirty = true;
            }
            Action::SetMilkdropFps(fps) => {
                self.settings.milkdrop_fps = if fps == 0 {
                    0
                } else {
                    fps.clamp(
                        *crate::milkdrop::FPS_RANGE.start(),
                        *crate::milkdrop::FPS_RANGE.end(),
                    )
                };
                self.settings_dirty = true;
            }
            Action::SetMilkdropScale(scale) => {
                self.settings.milkdrop_scale = scale.clamp(1, 4);
                self.settings_dirty = true;
            }
            Action::OpenMilkdropFolder => self.open_folder(self.dirs.milkdrop_dir()),
            Action::DownloadMilkdropPack(index) => {
                if let Some(pack) = crate::milkdrop::PACKS.get(index) {
                    self.winamp
                        .presets
                        .download(pack, self.dirs.milkdrop_dir(), ctx.clone());
                    self.toast(format!("Downloading {} presets", pack.name));
                }
            }
            Action::Quit => {
                self.quit_requested = true;
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
        }
    }

    pub fn toast(&mut self, message: impl Into<String>) {
        self.toasts.push(Toast {
            message: message.into(),
            kind: ToastKind::Info,
            created: Instant::now(),
        });
        self.toasts.truncate(4);
    }

    pub fn toast_error(&mut self, message: impl Into<String>) {
        let message = message.into();
        log::warn!("{message}");
        self.toasts.push(Toast {
            message,
            kind: ToastKind::Error,
            created: Instant::now(),
        });
    }

    /// Selected row indices for `page`.
    pub fn picked_rows(&self, page: &Page) -> Option<&std::collections::BTreeSet<usize>> {
        self.selection
            .as_ref()
            .filter(|(owner, _, _)| owner == page)
            .map(|(_, _, selection)| &selection.rows)
            .filter(|rows| !rows.is_empty())
    }

    /// Clears selection when the page's rows or order change.
    pub fn keep_picked_rows_for(&mut self, page: &Page, view: &str) {
        let stale = self
            .selection
            .as_ref()
            .is_some_and(|(owner, seen, _)| owner == page && seen != view);
        if stale {
            self.selection = None;
        }
    }

    /// Applies a single, toggle, or range row selection.
    ///
    /// `len` bounds ranges if rows changed after the anchor was set.
    pub fn pick_row(&mut self, page: &Page, view: &str, row: usize, pick: RowPick, len: usize) {
        let mut selection = match self.selection.take() {
            Some((owner, seen, selection)) if owner == *page && seen == view => selection,
            _ => RowSelection::default(),
        };
        match pick {
            RowPick::Only => {
                // Clicking the sole selected row clears the selection.
                let only_this = selection.rows.len() == 1 && selection.rows.contains(&row);
                selection.rows.clear();
                if only_this {
                    selection.anchor = None;
                } else {
                    selection.rows.insert(row);
                    selection.anchor = Some(row);
                }
            }
            RowPick::Toggle => {
                if !selection.rows.remove(&row) {
                    selection.rows.insert(row);
                }
                selection.anchor = Some(row);
            }
            RowPick::Range => {
                // Without an anchor, shift-click selects only this row.
                let anchor = selection.anchor.unwrap_or(row);
                let (from, to) = if anchor <= row {
                    (anchor, row)
                } else {
                    (row, anchor)
                };
                selection.rows.clear();
                selection.rows.extend((from..=to).filter(|row| *row < len));
                selection.anchor = Some(anchor);
            }
        }
        if selection.rows.is_empty() {
            self.selection = None;
        } else {
            self.selection = Some((page.clone(), view.to_string(), selection));
        }
    }

    /// Clears the current row selection.
    pub fn clear_picked_rows(&mut self) {
        self.selection = None;
    }

    pub fn editable_playlists(&self) -> Vec<(String, String)> {
        let Some(user_id) = self.user_id() else {
            return Vec::new();
        };
        self.library
            .playlists
            .get()
            .map(|playlists| {
                playlists
                    .iter()
                    .filter(|playlist| playlist.owned_by(user_id) || playlist.collaborative)
                    .map(|playlist| (playlist.id.clone(), playlist.name.clone()))
                    .collect()
            })
            .unwrap_or_default()
    }
}

impl App {
    /// Runs background work with or without a main window.
    pub fn background_frame(&mut self, ctx: &egui::Context) {
        self.handle_control_commands();
        self.handle_events();
        self.handle_media_commands();
        self.handle_tray();
        self.tick(ctx);
        self.note_listening();
        // MilkDrop runs in a child process and can outlive the main window.
        // Poll it before applying actions because its keys produce actions.
        #[cfg(feature = "milkdrop")]
        self.sync_milkdrop(ctx);
        self.apply_actions(ctx);
        self.sync_media_controls();
        self.sync_window_title(ctx);
    }

    /// Records a track after enough active listening time.
    ///
    /// Paused time and seeking do not count. Each track is recorded once.
    fn note_listening(&mut self) {
        let Some(now) = self.now_playing() else {
            self.listening = None;
            return;
        };
        // A resume preview is not a new play.
        if now.resuming {
            self.listening = None;
            return;
        }
        let listening = match &mut self.listening {
            Some(held) if held.uri == now.uri => held,
            _ => {
                self.listening = Some(Listening {
                    uri: now.uri.clone(),
                    listened: std::time::Duration::ZERO,
                    playing_since: now.playing.then(Instant::now),
                    recorded: false,
                });
                return;
            }
        };
        match (now.playing, listening.playing_since) {
            // Add the completed interval when playback pauses.
            (false, Some(since)) => {
                listening.listened += since.elapsed();
                listening.playing_since = None;
            }
            (true, None) => listening.playing_since = Some(Instant::now()),
            _ => {}
        }
        if listening.recorded {
            return;
        }
        let listened = listening.listened
            + listening
                .playing_since
                .map(|since| since.elapsed())
                .unwrap_or_default();
        if listened < crate::history::counts_after(now.duration_ms) {
            return;
        }
        listening.recorded = true;
        self.plays
            .record(crate::history::played_track(&now), jiff::Timestamp::now());
        self.save_plays();
        self.rebuild_recents();
    }

    /// Writes the play history, unless this is demo mode: sample data must
    /// never land in the real file, the same rule `save_settings` follows.
    fn save_plays(&mut self) {
        if self.offline {
            return;
        }
        self.plays.save(&self.dirs.history_file());
    }

    /// Keeps the current track in the window and taskbar title (#94).
    fn sync_window_title(&mut self, ctx: &egui::Context) {
        let title = match self.now_playing().filter(|now| now.playing) {
            Some(now) if now.subtitle.is_empty() => format!("{} - Fastsonic", now.title),
            Some(now) => format!("{} - {}", now.subtitle, now.title),
            None => "Fastsonic".to_string(),
        };
        if title != self.window_title {
            ctx.send_viewport_cmd(egui::ViewportCommand::Title(title.clone()));
            self.window_title = title;
        }
    }

    pub fn frame_ui(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        let ctx = &ctx;
        self.apply_theme(ctx);
        self.lock_scroll_axis(ctx);
        // Switch to the main window when sign-in is required.
        let needs_sign_in = !(self.is_connected() && self.user.is_some())
            && !matches!(self.auth, AuthStatus::Connecting | AuthStatus::Starting)
            && !(self.is_connected() && self.user.is_none());
        if self.settings.winamp_window && needs_sign_in && !self.switch_intent {
            self.actions.push(Action::ToggleWinampWindow);
        }
        if self.settings.winamp_window {
            crate::ui::winamp::show(self, ui);
        } else {
            crate::ui::show(self, ui);
        }
        self.apply_actions(ctx);
        self.sync_media_controls();

        if !self.settings.winamp_window {
            if let Some(rect) = ctx.input(|input| input.viewport().inner_rect) {
                self.last_window_size = Some([rect.width(), rect.height()]);
            }
            if let Some(rect) = ctx.input(|input| input.viewport().outer_rect) {
                self.last_window_pos = Some([rect.min.x, rect.min.y]);
            }
        }

        let playing = self.now_playing().is_some_and(|now| now.playing);
        if playing {
            ctx.request_repaint_after(Duration::from_millis(250));
        }
        if !self.toasts.is_empty() {
            ctx.request_repaint_after(Duration::from_millis(120));
        }
        if self.any_play_pending() {
            ctx.request_repaint_after(Duration::from_millis(120));
        }
        if ctx.input(|input| input.viewport().close_requested())
            && !self.quit_requested
            && !self.switch_intent
            && self.hides_to_tray()
        {
            // Close the window and keep the process running in the tray.
            self.hide_intent = true;
        }
    }

    /// Locks each scroll gesture to one axis.
    ///
    /// Trackpads report small cross-axis deltas. Choose from the first movement
    /// and hold that axis until the gesture ends.
    fn lock_scroll_axis(&mut self, ctx: &egui::Context) {
        let (raw, from_trackpad, ended) = ctx.input(|input| {
            let mut sum = egui::Vec2::ZERO;
            let mut pointish = false;
            let mut ended = false;
            for event in &input.events {
                if let egui::Event::MouseWheel {
                    unit, delta, phase, ..
                } = event
                {
                    sum += *delta;
                    pointish |= *unit == egui::MouseWheelUnit::Point;
                    ended |= matches!(phase, egui::TouchPhase::End | egui::TouchPhase::Cancel);
                }
            }
            (sum, pointish, ended)
        });
        let now = Instant::now();
        if raw != egui::Vec2::ZERO {
            self.scroll_from_trackpad = from_trackpad;
        }
        // Linux touchpad point deltas need scaling. Wheel deltas are already
        // scaled, and macOS point deltas need no adjustment.
        let trackpad_here = cfg!(target_os = "linux") && self.scroll_from_trackpad;
        if trackpad_here {
            ctx.input_mut(|input| input.smooth_scroll_delta *= TRACKPAD_SCALE);
        }
        // Add decaying momentum to Linux touchpad scrolling. Track the final
        // 100 ms of movement to estimate release velocity.
        if trackpad_here && raw != egui::Vec2::ZERO {
            self.glide = None;
            self.scroll_accum += raw * TRACKPAD_SCALE;
            self.scroll_history
                .add(ctx.input(|input| input.time), self.scroll_accum);
            self.scroll_last_event = Some(now);
            // Wayland announces the lift; where nothing does, the quiet-gap
            // check below needs a frame to run in.
            ctx.request_repaint_after(Duration::from_millis(60));
        } else if raw != egui::Vec2::ZERO || ctx.input(|input| input.pointer.any_down()) {
            // Wheel input or a press stops touchpad momentum.
            self.glide = None;
            self.scroll_history.clear();
            self.scroll_last_event = None;
        }
        let quiet = self
            .scroll_last_event
            .is_some_and(|at| now.duration_since(at).as_secs_f32() > 0.15);
        if ended || quiet {
            let mut velocity = self.scroll_history.velocity().unwrap_or(egui::Vec2::ZERO);
            if let Some((axis, _)) = self.scroll_lock {
                match axis {
                    ScrollAxis::Horizontal => velocity.y = 0.0,
                    ScrollAxis::Vertical => velocity.x = 0.0,
                }
            }
            self.glide = (velocity.length() > GLIDE_START).then_some(velocity);
            self.scroll_history.clear();
            self.scroll_accum = egui::Vec2::ZERO;
            self.scroll_last_event = None;
        }
        if let Some(velocity) = self.glide {
            if raw == egui::Vec2::ZERO {
                let dt = ctx.input(|input| input.stable_dt).clamp(0.001, 0.05);
                ctx.input_mut(|input| input.smooth_scroll_delta += velocity * dt);
                let slower = velocity * (-dt / GLIDE_DECAY).exp();
                self.glide = (slower.length() > GLIDE_STOP).then_some(slower);
            }
            ctx.request_repaint();
        }
        let held = self
            .scroll_lock
            .filter(|(_, at)| now.duration_since(*at) < SCROLL_GESTURE_GAP)
            .map(|(axis, _)| axis);
        let moved = raw != egui::Vec2::ZERO;
        let axis = match held {
            Some(axis) => axis,
            None if moved && raw.x.abs() > raw.y.abs() * 1.2 => ScrollAxis::Horizontal,
            None if moved => ScrollAxis::Vertical,
            None => {
                self.scroll_lock = None;
                return;
            }
        };
        if moved {
            self.scroll_lock = Some((axis, now));
        }
        ctx.input_mut(|input| match axis {
            ScrollAxis::Horizontal => input.smooth_scroll_delta.y = 0.0,
            ScrollAxis::Vertical => input.smooth_scroll_delta.x = 0.0,
        });
    }

    /// Persist state when a window closes (to the tray or for good).
    pub fn save_state(&mut self) {
        self.save_settings();
        self.save_session();
    }

    /// Write the restorable session: page, recents, resume point, sorts.
    fn save_session(&mut self) {
        self.session_dirty = false;
        self.last_session_save = Instant::now();
        // A resume preview is the remembered session drawn, not a playing
        // one: reading it back would overwrite what it was drawn from and
        // lose the album the track came out of.
        if let Some(now) = self.now_playing().filter(|now| !now.resuming) {
            self.resume_context = self.playing_context_uri();
            self.resume_track = Some(now.uri.clone());
            self.resume_context_track = self.context_row_under(&now.uri);
            self.resume_position_ms = now.position_ms;
        }
        if !self.offline {
            SessionState {
                last_page: Some(self.page().encode()),
                recent_contexts: self.recent_contexts.clone(),
                last_context: self.resume_context.clone(),
                last_track: self.resume_track.clone(),
                last_context_track: self.resume_context_track.clone(),
                last_position_ms: self.resume_position_ms,
                last_added_queue: if self.resume_queue.is_empty() {
                    self.manual_queue.clone()
                } else {
                    // Never resumed this session; the owed queue carries over.
                    self.resume_queue.clone()
                },
                last_queue_rows: self.queue.queue.iter().take(30).cloned().collect(),
                shuffle_on: self.shuffle_wanted,
                sorts: self
                    .table_sorts
                    .iter()
                    .map(|(page, sort)| (page.encode(), *sort))
                    .collect(),
                window_size: self.last_window_size.or(self.session_window_size),
                window_pos: self.last_window_pos.or(self.session_window_pos),
                queue_open: Some(self.show_queue_panel),
                queue_tab: Some(self.queue_tab.encode().to_string()),
                winamp_pos: self.winamp.last_pos.or(self.winamp.restore_pos),
                milkdrop_pos: self.milkdrop_pos,
            }
            .save(&self.dirs.session_file());
        }
    }

    /// Final teardown at real quit.
    pub fn shutdown(&mut self) {
        self.save_state();
        self.backend.shutdown();
    }
}

/// The engine as the settings describe it.
///
/// The audio cache is passed in rather than opened here: it is shared with
/// the prefetch that opens the next track on the runtime, and it must
/// survive the engine being replaced — rebuilding it would throw the index
/// away and rescan the disk (P3.6).
pub fn engine_config(
    settings: &Settings,
    tap: std::sync::Arc<crate::vis::AudioTap>,
    eq: crate::eq::SharedEq,
    cache: Option<std::sync::Arc<crate::engine::Cache>>,
) -> EngineConfig {
    EngineConfig {
        tap,
        eq,
        normalisation: settings.normalisation,
        buffer_ms: settings.audio_buffer_ms,
        audio_device: settings
            .audio_device
            .clone()
            .filter(|device| !device.trim().is_empty()),
        initial_volume: settings.volume,
        cache,
    }
}

/// The audio cache the settings ask for, or `None` when it is switched off.
pub fn audio_cache(
    dirs: &AppDirs,
    settings: &Settings,
) -> Option<std::sync::Arc<crate::engine::Cache>> {
    if !settings.audio_cache {
        return None;
    }
    let limit = settings.audio_cache_mb.max(64) * 1024 * 1024;
    match crate::engine::Cache::open(dirs.audio_cache_dir(), limit) {
        Ok(cache) => Some(cache),
        Err(error) => {
            log::warn!("the audio cache is unavailable: {error:#}");
            None
        }
    }
}

/// The equalizer as the settings describe it.
pub fn eq_settings(settings: &Settings) -> crate::eq::EqSettings {
    crate::eq::EqSettings {
        on: settings.eq_on,
        preamp_db: settings.eq_preamp_db,
        bands_db: settings.eq_bands_db,
        balance: settings.balance,
        mono: settings.mono,
    }
    .clamped()
}

pub fn volume_to_percent(volume: u16) -> u8 {
    ((u32::from(volume) * 100 + u32::from(u16::MAX) / 2) / u32::from(u16::MAX)) as u8
}

pub fn percent_to_volume(percent: u8) -> u16 {
    ((u32::from(percent.min(100)) * u32::from(u16::MAX)) / 100) as u16
}

fn page_related_needs_load(pages: &HashMap<String, ArtistPage>, id: &str) -> bool {
    pages.get(id).is_some_and(|page| page.related.needs_load())
}

fn friendly_page_error(error: &crate::api::ApiError) -> String {
    if error.is_not_found() {
        return "The server no longer has this.".to_string();
    }
    error.to_string()
}

/// One queue row as the panel reads it. A row the server has not described
/// yet carries its URI and nothing else — the engine fills it in when the
/// answer arrives and pushes the queue again (P3.3).
fn queue_row_item(row: &crate::engine::QueueRow) -> PlayableItem {
    let Some(track) = &row.track else {
        return PlayableItem::Track(Track {
            uri: row.uri.clone(),
            ..Track::default()
        });
    };
    PlayableItem::Track(Track {
        id: crate::api::subsonic::convert::id_of(
            &row.uri,
            crate::api::subsonic::convert::Kind::Track,
        )
        .map(str::to_string),
        name: track.title.clone(),
        uri: row.uri.clone(),
        duration_ms: track.duration_ms,
        starred: track.starred,
        artists: track
            .artists
            .iter()
            .map(|name| ArtistRef {
                name: name.clone(),
                ..ArtistRef::default()
            })
            .collect(),
        album: Some(Album {
            name: track.album.clone(),
            images: track
                .art_url
                .iter()
                .chain(track.art_small_url.iter())
                .map(|url| Image {
                    url: url.clone(),
                    ..Image::default()
                })
                .collect(),
            ..Album::default()
        }),
        ..Track::default()
    })
}

/// What the engine is told to play. A single song goes as a context of
/// its own rather than a list of one: the engine resolves a track URI as a
/// context, and a context with a URI is what librespot's autoplay carries
/// on from when it ends, the way one song from a search does here.
/// The starred flags an answer stated, ready for [`App::note_saved`].
///
/// Objects that did not carry the flag are skipped rather than counted as
/// unstarred: an album shell inside a song, or a list from an endpoint that
/// omits it, knows nothing about hearts.
fn starred_flags<'a, T: Starred + 'a>(
    items: impl IntoIterator<Item = &'a T>,
) -> Vec<(String, bool)> {
    items
        .into_iter()
        .filter_map(|item| {
            let (uri, flag) = item.starred_flag()?;
            (!uri.is_empty()).then(|| (uri.to_string(), flag))
        })
        .collect()
}

fn local_load(request: &PlayRequest, shuffle: bool) -> LoadSpec {
    let single_song = request.context_uri.is_none()
        && request.uris.len() == 1
        && request.uris[0].contains(":track:");
    if single_song {
        return LoadSpec {
            context_uri: Some(request.uris[0].clone()),
            position_ms: request.position_ms,
            play: true,
            queued: request.restore_queued.clone(),
            ..LoadSpec::default()
        };
    }
    // A plain track list must not load shuffled when a row was chosen:
    // librespot shuffles the list first and then cannot find the chosen
    // row in it, falls back to nowhere, and replays what was on. The list
    // loads straight and shuffle is switched on right after the load,
    // The chosen song plays first, then the rest shuffle.
    let chosen = request.offset_uri.is_some() || request.offset_position.is_some();
    let list = request.context_uri.is_none();
    LoadSpec {
        context_uri: request.context_uri.clone(),
        uris: request.uris.clone(),
        offset_uri: request.offset_uri.clone(),
        offset_index: request.offset_position,
        position_ms: request.position_ms,
        play: true,
        shuffle: (shuffle && !(list && chosen)).then_some(true),
        // Rule 9: a resumed session hands the engine its queue with the
        // play, so the panel is whole in the first snapshot it publishes.
        queued: request.restore_queued.clone(),
        current: request.restore_current.clone(),
    }
}

/// Caps large track lists at 500 items starting from the selected row.
fn cap_uris(uris: Vec<String>, index: u32) -> (Vec<String>, u32) {
    const MAX: usize = 500;
    if uris.len() <= MAX {
        return (uris, index);
    }
    let start = (index as usize).min(uris.len() - 1);
    let end = (start + MAX).min(uris.len());
    (uris[start..end].to_vec(), 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn volume_conversions_round_trip() {
        assert_eq!(volume_to_percent(u16::MAX), 100);
        assert_eq!(volume_to_percent(0), 0);
        assert_eq!(volume_to_percent(percent_to_volume(70)), 70);
        assert_eq!(percent_to_volume(200), u16::MAX);
    }

    /// The song the last session ended on is shown, paused, at the position
    /// it stopped at, so a cold start does not look like an empty player.
    #[test]
    fn the_remembered_song_is_shown_paused_at_its_position() {
        use crate::api::models::{Album, ArtistRef, Track};
        let mut app = headless_app();
        app.resume_track = Some("sonic:track:abc".into());
        app.resume_position_ms = 19_566;
        assert!(
            app.now_playing().is_none(),
            "nothing to show until the song's details arrive"
        );
        app.track_cache.insert(
            "abc".into(),
            Track {
                id: Some("abc".into()),
                uri: "sonic:track:abc".into(),
                name: "Karma Police".into(),
                artists: vec![ArtistRef {
                    id: None,
                    name: "Radiohead".into(),
                    uri: None,
                }],
                album: Some(Album {
                    id: "ok".into(),
                    name: "OK Computer".into(),
                    ..Default::default()
                }),
                duration_ms: 264_000,
                ..Default::default()
            },
        );
        let now = app.now_playing().expect("the remembered song is shown");
        assert!(now.resuming);
        assert!(!now.playing, "it is shown paused, not played");
        assert_eq!(now.title, "Karma Police");
        assert_eq!(now.subtitle, "Radiohead");
        assert_eq!(now.album_name, "OK Computer");
        assert_eq!(now.duration_ms, 264_000);
        assert_eq!(
            now.position_ms, 19_566,
            "the bar sits where the listener left it, not at zero"
        );
    }

    /// Drawing the remembered song must not be mistaken for a song that
    /// started: that would rewind the very position it exists to show.
    #[test]
    fn showing_the_remembered_song_keeps_its_position() {
        use crate::api::models::Track;
        let mut app = headless_app();
        app.resume_track = Some("sonic:track:abc".into());
        app.resume_position_ms = 19_566;
        app.track_cache.insert(
            "abc".into(),
            Track {
                id: Some("abc".into()),
                uri: "sonic:track:abc".into(),
                duration_ms: 264_000,
                ..Default::default()
            },
        );
        app.on_now_playing_changed();
        assert_eq!(app.resume_position_ms, 19_566);
        app.save_session();
        assert_eq!(
            app.resume_position_ms, 19_566,
            "closing again must not lose the position"
        );
    }

    /// Dragging the bar before pressing play moves where play will land.
    #[test]
    fn seeking_the_remembered_song_moves_the_resume_point() {
        let mut app = headless_app();
        app.resume_track = Some("sonic:track:abc".into());
        app.resume_position_ms = 19_566;
        app.seek(90_000);
        assert_eq!(app.resume_position_ms, 90_000);
    }

    /// Play resumes the remembered track at its saved position.
    #[test]
    fn pressing_play_on_a_cold_start_does_not_restart_the_song() {
        let mut app = headless_app();
        app.resume_context = Some("sonic:playlist:pl1".into());
        app.resume_track = Some("sonic:track:abc".into());
        app.resume_position_ms = 19_566;
        app.toggle_play();
        let request = app
            .queued_play
            .as_ref()
            .expect("the resumed play is held for the engine");
        assert_eq!(
            request.context_uri.as_deref(),
            Some("sonic:playlist:pl1"),
            "it resumes inside the playlist it was left in"
        );
        assert_eq!(request.offset_uri.as_deref(), Some("sonic:track:abc"));
        assert_eq!(
            request.position_ms, 19_566,
            "the song resumes where it stopped, not at zero"
        );
    }

    /// A media key can arrive before startup has reported that the saved
    /// local player is connecting. Its intent must survive that race.
    #[test]
    fn pressing_play_while_startup_connects_is_held_for_the_player() {
        let mut app = headless_app();
        app.local_ready = false;
        app.local_playback = LocalPlayback::Unavailable;
        app.auth = AuthStatus::Starting;
        app.settings.playback_authorized = true;
        app.resume_track = Some("sonic:track:abc".into());

        app.toggle_play();

        assert!(app.queued_play.is_some());
    }

    /// Previous and Next move a restored track without starting playback.
    #[test]
    fn the_transport_works_on_a_restored_song_without_playing_it() {
        use crate::api::models::{PlayableItem, PlaylistItem, Track};
        use crate::model::PagedList;
        let row = |uri: &str| PlaylistItem {
            item: Some(PlayableItem::Track(Track {
                id: Some(uri.rsplit(':').next().unwrap().into()),
                uri: uri.into(),
                name: uri.into(),
                ..Default::default()
            })),
            ..Default::default()
        };
        let ctx = egui::Context::default();
        let mut app = headless_app();
        app.playlist_pages.insert(
            "pl1".into(),
            PlaylistPage {
                items: PagedList {
                    items: vec![
                        row("sonic:track:one"),
                        row("sonic:track:two"),
                        row("sonic:track:three"),
                    ],
                    ..Default::default()
                },
                ..Default::default()
            },
        );
        app.resume_context = Some("sonic:playlist:pl1".into());
        app.resume_track = Some("sonic:track:two".into());
        app.resume_position_ms = 19_566;
        assert!(app.resume_only(), "loaded and current, but not playing");

        // Next steps to the following song, at its start, still not playing.
        app.apply(Action::Next, &ctx);
        assert_eq!(app.resume_track.as_deref(), Some("sonic:track:three"));
        assert_eq!(app.resume_position_ms, 0);
        assert!(
            app.queued_play.is_none() && app.local_list.is_none(),
            "skipping must not start the restored song"
        );
        // Use loaded row details for immediate display.
        let now = app.now_playing().expect("the new song is shown");
        assert!(now.resuming && !now.playing);
        assert_eq!(now.uri, "sonic:track:three");

        // Previous steps back from the start of a song.
        app.apply(Action::Previous, &ctx);
        assert_eq!(app.resume_track.as_deref(), Some("sonic:track:two"));

        // Past the threshold, Previous restarts instead, as it does while
        // playing.
        app.resume_position_ms = 19_566;
        app.apply(Action::Previous, &ctx);
        assert_eq!(app.resume_track.as_deref(), Some("sonic:track:two"));
        assert_eq!(app.resume_position_ms, 0, "it restarts the song");

        // The ends of the list wrap rather than dead-ending.
        app.apply(Action::Previous, &ctx);
        assert_eq!(app.resume_track.as_deref(), Some("sonic:track:one"));
        app.apply(Action::Previous, &ctx);
        assert_eq!(app.resume_track.as_deref(), Some("sonic:track:three"));

        // Play starts the selected track in its playlist.
        app.apply(Action::TogglePlay, &ctx);
        let request = app.queued_play.as_ref().expect("play starts it");
        assert_eq!(
            request.context_uri.as_deref(),
            Some("sonic:playlist:pl1"),
            "the playlist it was left in is kept"
        );
        assert_eq!(request.offset_uri.as_deref(), Some("sonic:track:three"));
    }

    /// A restored session keeps shuffle enabled when skipping.
    #[test]
    fn skipping_a_restored_song_keeps_shuffle_on() {
        use crate::api::models::{PlayableItem, PlaylistItem, Track};
        use crate::model::PagedList;
        let row = |uri: &str| PlaylistItem {
            item: Some(PlayableItem::Track(Track {
                uri: uri.into(),
                ..Default::default()
            })),
            ..Default::default()
        };
        let ctx = egui::Context::default();
        let mut app = headless_app();
        app.playlist_pages.insert(
            "pl1".into(),
            PlaylistPage {
                items: PagedList {
                    items: vec![row("sonic:track:one"), row("sonic:track:two")],
                    ..Default::default()
                },
                ..Default::default()
            },
        );
        app.resume_context = Some("sonic:playlist:pl1".into());
        app.resume_track = Some("sonic:track:one".into());
        app.shuffle_wanted = true;
        app.apply(Action::Next, &ctx);
        assert!(app.shuffle_wanted, "shuffle survives the skip");
        assert_eq!(
            app.resume_track.as_deref(),
            Some("sonic:track:two"),
            "a shuffled skip still lands on another song in the context"
        );
    }

    /// Rule 9: the songs queued when the app closed come back with the
    /// play that resumes the remembered song — one load, carrying the
    /// queue, in the order it was saved in.
    #[test]
    fn the_saved_queue_is_handed_back_with_its_remembered_song() {
        let mut app = headless_app();
        app.local.connected = true;
        app.resume_context = Some("sonic:album:x".into());
        app.resume_track = Some("sonic:track:abc".into());
        app.resume_queue = vec!["sonic:track:q1".into(), "sonic:track:q2".into()];
        app.backend.asked();
        assert!(app.resume_last(), "there is a session to resume");
        let asked = app.backend.asked();
        let [PlayerCommand::Load(load)] = asked.as_slice() else {
            panic!("one load resumes the session, not a load and a queue: {asked:?}");
        };
        assert_eq!(load.context_uri.as_deref(), Some("sonic:album:x"));
        assert_eq!(load.offset_uri.as_deref(), Some("sonic:track:abc"));
        assert_eq!(
            load.queued,
            vec!["sonic:track:q1", "sonic:track:q2"],
            "the queue comes back in the order it was saved in"
        );
        assert_eq!(load.current, None, "the remembered song is the album's row");
        assert_eq!(load.position_ms, 0);
        assert!(app.resume_queue.is_empty(), "and is owed only once");
    }

    /// Rule 9, and rule 3 underneath it: a session closed while a queued
    /// song played comes back with the song playing *and* the album where
    /// it was, rather than with the album moved to the queued song.
    #[test]
    fn a_queued_song_resumes_over_the_album_it_interrupted() {
        let mut app = headless_app();
        app.local.connected = true;
        app.resume_context = Some("sonic:album:x".into());
        app.resume_track = Some("sonic:track:q1".into());
        app.resume_context_track = Some("sonic:track:two".into());
        app.resume_queue = vec!["sonic:track:q2".into()];
        app.resume_position_ms = 42_000;
        app.backend.asked();
        assert!(app.resume_last());
        let asked = app.backend.asked();
        let [PlayerCommand::Load(load)] = asked.as_slice() else {
            panic!("one load resumes the session: {asked:?}");
        };
        assert_eq!(
            load.offset_uri.as_deref(),
            Some("sonic:track:two"),
            "the album goes back to the row it was on"
        );
        assert_eq!(
            load.current.as_deref(),
            Some("sonic:track:q1"),
            "and the queued song plays over the top of it"
        );
        assert_eq!(load.queued, vec!["sonic:track:q2"]);
        assert_eq!(load.position_ms, 42_000);
        assert_eq!(
            app.current_track_uri().as_deref(),
            Some("sonic:track:q1"),
            "the player bar owes the song, not the album row under it"
        );
    }

    /// Rule 9: the owed songs belong to the remembered track alone.
    /// Playing something else first lets them go rather than queueing them
    /// behind it.
    #[test]
    fn playing_something_else_first_lets_the_saved_queue_go() {
        let mut app = headless_app();
        app.resume_track = Some("sonic:track:abc".into());
        app.resume_queue = vec!["sonic:track:q1".into()];
        app.local.track = Some(crate::engine::LocalTrack {
            uri: "sonic:track:other".into(),
            ..Default::default()
        });
        app.local.playback = Playback::Playing;
        app.backend.asked();
        app.on_now_playing_changed();
        assert!(
            app.backend.asked().is_empty(),
            "a fresh start lets the saved queue go rather than queueing it \
             behind something else"
        );
        assert!(app.resume_queue.is_empty());
        assert!(app.session_dirty);
    }

    /// A selected list row loads without shuffle, then enables shuffle.
    #[test]
    fn a_chosen_row_in_a_list_never_loads_shuffled() {
        let request = PlayRequest::tracks(vec!["sonic:track:a".into(), "sonic:track:b".into()])
            .starting_at_index(1);
        let load = local_load(&request, true);
        assert_eq!(load.shuffle, None);
        assert_eq!(load.offset_index, Some(1));
        // Without a chosen row the list may shuffle from the start.
        let request = PlayRequest::tracks(vec!["sonic:track:a".into(), "sonic:track:b".into()]);
        assert_eq!(local_load(&request, true).shuffle, Some(true));
        // A context play keeps its shuffled load; the offset was already
        // picked to match.
        let request = PlayRequest::context("sonic:playlist:x").starting_at_uri("sonic:track:a");
        assert_eq!(local_load(&request, true).shuffle, Some(true));
    }

    // ---- the queue ------------------------------------------------------
    //
    // `docs/_reference/queue.md` is the contract, and since P3.3 the queue
    // itself is `src/engine/queue.rs` — rules 1 to 7 are the engine's, with
    // a test each beside the state machine that holds them up. What is
    // tested here is the other half: that the interface asks the engine for
    // the right thing, and draws what the engine answers rather than a
    // story of its own. Nothing about the queue is optimistic any more,
    // which is what rule 8 costs when the queue is a channel away instead
    // of a network away, and what retires rule 10.

    fn queued_song(uri: &str) -> crate::api::models::PlayableItem {
        crate::api::models::PlayableItem::Track(crate::api::models::Track {
            uri: uri.into(),
            ..Default::default()
        })
    }

    fn loaded_queue(current: &str, next: &[&str]) -> Queue {
        Queue {
            currently_playing: Some(queued_song(current)),
            queue: next.iter().map(|uri| queued_song(uri)).collect(),
        }
    }

    /// One row as the engine publishes it: described, the way a row of an
    /// album or a playlist is born.
    fn queue_row(uri: &str) -> crate::engine::QueueRow {
        crate::engine::QueueRow {
            uri: uri.into(),
            track: Some(crate::engine::LocalTrack {
                uri: uri.into(),
                title: uri.rsplit(':').next().unwrap_or_default().to_string(),
                ..Default::default()
            }),
        }
    }

    /// What the engine publishes: the song playing, the songs queued by
    /// hand, and what the album has left.
    fn snapshot(
        current: Option<&str>,
        queued: &[&str],
        upcoming: &[&str],
    ) -> crate::engine::QueueSnapshot {
        crate::engine::QueueSnapshot {
            current: current.map(queue_row),
            queued: queued.iter().copied().map(queue_row).collect(),
            upcoming: upcoming.iter().copied().map(queue_row).collect(),
            context_uri: Some("sonic:album:x".into()),
            context_at: current.map(str::to_string),
        }
    }

    fn queue_uris(app: &App) -> (Option<String>, Vec<String>) {
        (
            app.queue
                .currently_playing
                .as_ref()
                .map(|item| item.uri().to_string()),
            app.queue
                .queue
                .iter()
                .map(|item| item.uri().to_string())
                .collect(),
        )
    }

    /// Rules 1 and 3: the panel draws the play order — the songs you queued
    /// and then the album's — and the song playing is in neither list.
    #[test]
    fn the_engines_queue_is_the_one_the_panel_draws() {
        let mut app = headless_app();
        app.handle_queue(snapshot(
            Some("sonic:track:a"),
            &["sonic:track:q1", "sonic:track:q2"],
            &["sonic:track:ctx1", "sonic:track:ctx2"],
        ));
        let (current, next) = queue_uris(&app);
        assert_eq!(current.as_deref(), Some("sonic:track:a"));
        assert_eq!(
            next,
            vec![
                "sonic:track:q1",
                "sonic:track:q2",
                "sonic:track:ctx1",
                "sonic:track:ctx2",
            ],
            "your songs first, then the album's, which is the play order"
        );
        assert!(
            !next.contains(&"sonic:track:a".to_string()),
            "the song playing is never also a row that plays next"
        );
        assert_eq!(
            app.queued_rows_len(),
            2,
            "the line between the two sections is where the engine put it"
        );
        assert_eq!(
            app.manual_queue,
            vec!["sonic:track:q1", "sonic:track:q2"],
            "what the session file keeps is what you queued, nothing else"
        );
    }

    /// The section split is the engine's answer and not a guess made by
    /// matching songs: the same song can be in both halves.
    #[test]
    fn the_queued_section_ends_where_the_engine_says_it_does() {
        let mut app = headless_app();
        app.handle_queue(snapshot(
            Some("sonic:track:a"),
            &["sonic:track:b", "sonic:track:c"],
            &["sonic:track:ctx1", "sonic:track:c"],
        ));
        assert_eq!(
            app.queued_rows_len(),
            2,
            "the album's own copy of c is not one of yours"
        );
        app.handle_queue(snapshot(
            Some("sonic:track:a"),
            &[],
            &["sonic:track:ctx1", "sonic:track:c"],
        ));
        assert_eq!(app.queued_rows_len(), 0);
    }

    /// Rule 2: Play next asks the engine to queue the song, and asks it
    /// once per click. The engine decides where the row goes.
    #[test]
    fn play_next_asks_the_engine_to_queue_the_song() {
        let ctx = egui::Context::default();
        let mut app = headless_app();
        app.backend.asked();
        app.apply(
            Action::AddToQueue {
                uri: "sonic:track:b".into(),
                label: "Bell".into(),
            },
            &ctx,
        );
        assert_eq!(
            app.backend.asked(),
            vec![PlayerCommand::AddToQueue("sonic:track:b".into())],
            "one click, one ask, and no request to anybody else"
        );
    }

    /// Rule 2, the other half: a double-click counts once, two separate
    /// asks are two rows.
    #[test]
    fn a_double_click_is_one_row_and_two_asks_are_two() {
        let ctx = egui::Context::default();
        let mut app = headless_app();
        let add = Action::AddToQueue {
            uri: "sonic:track:b".into(),
            label: "Bell".into(),
        };
        app.backend.asked();
        app.apply(add.clone(), &ctx);
        app.apply(add.clone(), &ctx);
        assert_eq!(
            app.backend.asked(),
            vec![PlayerCommand::AddToQueue("sonic:track:b".into())],
            "the second half of a double-click is not a second wish"
        );
        // A later ask for the same song.
        for (_, at) in &mut app.queue_add_clicks {
            *at = Instant::now() - QUEUE_ADD_DEBOUNCE;
        }
        app.apply(add, &ctx);
        assert_eq!(
            app.backend.asked(),
            vec![PlayerCommand::AddToQueue("sonic:track:b".into())],
            "two asks are two rows"
        );
    }

    /// A row the server has not described yet still reads as the song that
    /// was queued: the name comes from the click that queued it.
    #[test]
    fn a_queued_row_shows_its_name_before_the_server_describes_it() {
        let ctx = egui::Context::default();
        let mut app = headless_app();
        app.apply(
            Action::AddToQueue {
                uri: "sonic:track:b".into(),
                label: "Bell".into(),
            },
            &ctx,
        );
        // The engine has the id and nothing else yet.
        app.handle_queue(crate::engine::QueueSnapshot {
            current: Some(queue_row("sonic:track:a")),
            queued: vec![crate::engine::QueueRow {
                uri: "sonic:track:b".into(),
                track: None,
            }],
            upcoming: Vec::new(),
            context_uri: None,
            context_at: None,
        });
        assert_eq!(
            app.queue.queue.first().map(|item| item.name()),
            Some("Bell"),
            "a nameless row is not a queue panel"
        );
        // Once it is described, the server's answer is what shows, and the
        // name the click carried is let go — a moment later, so that a
        // snapshot from before the ask cannot drop it early.
        app.handle_queue(snapshot(Some("sonic:track:a"), &["sonic:track:b"], &[]));
        assert_eq!(app.queue.queue.first().map(|item| item.name()), Some("b"));
        for (_, at) in &mut app.queue_add_clicks {
            *at = Instant::now() - QUEUE_ADD_DEBOUNCE;
        }
        app.handle_queue(snapshot(Some("sonic:track:a"), &["sonic:track:b"], &[]));
        assert!(app.queue_names.is_empty());
    }

    /// Rule 4: Next asks the engine, which takes the top row off before it
    /// opens anything — so the interface has nothing to fake.
    #[test]
    fn next_asks_the_engine_and_leaves_the_queue_to_it() {
        let ctx = egui::Context::default();
        let mut app = headless_app();
        app.local.track = Some(crate::engine::LocalTrack {
            uri: "sonic:track:a".into(),
            ..Default::default()
        });
        app.local.playback = Playback::Playing;
        app.handle_queue(snapshot(
            Some("sonic:track:a"),
            &[],
            &["sonic:track:b", "sonic:track:c"],
        ));
        app.backend.asked();
        app.apply(Action::Next, &ctx);
        assert_eq!(app.backend.asked(), vec![PlayerCommand::Next]);
        // The engine's answer, which the next frame draws.
        app.handle_queue(snapshot(Some("sonic:track:b"), &[], &["sonic:track:c"]));
        let (current, next) = queue_uris(&app);
        assert_eq!(current.as_deref(), Some("sonic:track:b"));
        assert_eq!(next, vec!["sonic:track:c"]);
    }

    /// Rule 5: playing a row skips down to it, in one command. The rows
    /// above it go with it; the ones below stay.
    #[test]
    fn a_chosen_queue_row_asks_to_skip_down_to_it() {
        let ctx = egui::Context::default();
        let mut app = headless_app();
        app.local.track = Some(crate::engine::LocalTrack {
            uri: "sonic:track:a".into(),
            ..Default::default()
        });
        app.local.playback = Playback::Playing;
        app.handle_queue(snapshot(
            Some("sonic:track:a"),
            &["sonic:track:b", "sonic:track:c"],
            &["sonic:track:d"],
        ));
        app.backend.asked();
        app.apply(
            Action::PlayFromRow {
                context: RowContext::Queue,
                uri: "sonic:track:c".into(),
                index: 1,
            },
            &ctx,
        );
        assert_eq!(
            app.backend.asked(),
            vec![PlayerCommand::PlayQueued(1)],
            "one command, counting the rows as the panel draws them"
        );
        assert!(
            app.play_pending("sonic:track:c"),
            "the chosen row is marked as playing while the engine opens it"
        );
        // What the engine makes of it.
        app.handle_queue(snapshot(Some("sonic:track:c"), &[], &["sonic:track:d"]));
        let (current, next) = queue_uris(&app);
        assert_eq!(current.as_deref(), Some("sonic:track:c"));
        assert_eq!(next, vec!["sonic:track:d"], "the rows below stay");
    }

    /// The click names a song: when the rows have shifted under the
    /// pointer, the song wins over the row number.
    #[test]
    fn a_clicked_queue_row_is_found_by_its_song_when_rows_shifted() {
        let ctx = egui::Context::default();
        let mut app = headless_app();
        app.local.track = Some(crate::engine::LocalTrack {
            uri: "sonic:track:a".into(),
            ..Default::default()
        });
        app.local.playback = Playback::Playing;
        app.handle_queue(snapshot(
            Some("sonic:track:a"),
            &[],
            &["sonic:track:b", "sonic:track:c"],
        ));
        app.backend.asked();
        app.apply(
            Action::PlayFromRow {
                context: RowContext::Queue,
                uri: "sonic:track:c".into(),
                index: 0,
            },
            &ctx,
        );
        assert_eq!(app.backend.asked(), vec![PlayerCommand::PlayQueued(1)]);
    }

    /// A row that has already played by the time the click lands asks for
    /// nothing: the queue on show is about to be replaced anyway.
    #[test]
    fn a_click_on_a_row_that_has_gone_asks_for_nothing() {
        let ctx = egui::Context::default();
        let mut app = headless_app();
        app.local.track = Some(crate::engine::LocalTrack {
            uri: "sonic:track:a".into(),
            ..Default::default()
        });
        app.local.playback = Playback::Playing;
        app.handle_queue(snapshot(Some("sonic:track:a"), &[], &["sonic:track:b"]));
        app.backend.asked();
        app.apply(
            Action::PlayFromRow {
                context: RowContext::Queue,
                uri: "sonic:track:gone".into(),
                index: 3,
            },
            &ctx,
        );
        assert!(app.backend.asked().is_empty());
    }

    /// Rule 6: a new album changes the rows underneath and leaves the songs
    /// you queued on top of them.
    #[test]
    fn starting_a_new_album_keeps_the_songs_you_queued() {
        let mut app = headless_app();
        app.handle_queue(snapshot(
            Some("sonic:track:a"),
            &["sonic:track:q1"],
            &["sonic:track:ctx1"],
        ));
        app.handle_queue(snapshot(
            Some("sonic:track:new1"),
            &["sonic:track:q1"],
            &["sonic:track:new2"],
        ));
        let (_, next) = queue_uris(&app);
        assert_eq!(next, vec!["sonic:track:q1", "sonic:track:new2"]);
        assert_eq!(app.queued_rows_len(), 1, "your song still plays first");
    }

    /// Rule 7: Clear empties your section and asks for nothing else. The
    /// button only shows while there is a section to empty.
    #[test]
    fn clear_asks_for_your_own_rows_and_only_shows_with_some() {
        let ctx = egui::Context::default();
        let mut app = headless_app();
        app.local.track = Some(crate::engine::LocalTrack {
            uri: "sonic:track:a".into(),
            ..Default::default()
        });
        app.local.playback = Playback::Playing;
        app.handle_queue(snapshot(Some("sonic:track:a"), &[], &["sonic:track:ctx1"]));
        assert!(
            !app.can_clear_queue(),
            "with nothing of yours queued there is nothing to clear"
        );
        assert!(
            !headless_app().can_clear_queue(),
            "and the remembered queue is not one the engine could clear"
        );
        app.handle_queue(snapshot(
            Some("sonic:track:a"),
            &["sonic:track:b", "sonic:track:c"],
            &["sonic:track:c", "sonic:track:ctx1"],
        ));
        assert!(app.can_clear_queue());
        app.backend.asked();
        app.apply(Action::ClearQueue, &ctx);
        assert_eq!(app.backend.asked(), vec![PlayerCommand::ClearQueue]);
        // The engine's answer keeps the album's own copy of c.
        app.handle_queue(snapshot(
            Some("sonic:track:a"),
            &[],
            &["sonic:track:c", "sonic:track:ctx1"],
        ));
        let (_, next) = queue_uris(&app);
        assert_eq!(next, vec!["sonic:track:c", "sonic:track:ctx1"]);
        assert!(app.manual_queue.is_empty());
    }

    /// Rule 8 with no remote service in it, and rule 10 retired: the engine's word
    /// is the queue, even where it differs from what was just asked for.
    /// There is no second copy to keep, and nothing stale to ignore.
    #[test]
    fn the_engines_word_is_the_queue_even_where_it_differs_from_the_ask() {
        let ctx = egui::Context::default();
        let mut app = headless_app();
        app.local.track = Some(crate::engine::LocalTrack {
            uri: "sonic:track:a".into(),
            ..Default::default()
        });
        app.local.playback = Playback::Playing;
        app.handle_queue(snapshot(
            Some("sonic:track:a"),
            &["sonic:track:b"],
            &["sonic:track:ctx1"],
        ));
        app.apply(Action::ClearQueue, &ctx);
        let (_, next) = queue_uris(&app);
        assert_eq!(
            next,
            vec!["sonic:track:b", "sonic:track:ctx1"],
            "the panel waits for the engine rather than guessing at it"
        );
        app.apply(Action::Next, &ctx);
        let (current, _) = queue_uris(&app);
        assert_eq!(
            current.as_deref(),
            Some("sonic:track:a"),
            "and it does not move the playing row on its own either"
        );
    }

    fn picked(app: &App, page: &Page) -> Vec<usize> {
        app.picked_rows(page)
            .map(|rows| rows.iter().copied().collect())
            .unwrap_or_default()
    }

    /// A plain click selects one row; clicking it again clears selection.
    #[test]
    fn a_plain_click_picks_one_row_and_a_second_lets_it_go() {
        let mut app = test_app("pick-one");
        let page = Page::LikedSongs;
        app.pick_row(&page, "v", 3, RowPick::Only, 10);
        assert_eq!(picked(&app, &page), vec![3]);
        app.pick_row(&page, "v", 5, RowPick::Only, 10);
        assert_eq!(picked(&app, &page), vec![5], "the first one is dropped");
        app.pick_row(&page, "v", 5, RowPick::Only, 10);
        assert!(picked(&app, &page).is_empty(), "clicking it again lets go");
    }

    /// Ctrl-click toggles one row.
    #[test]
    fn ctrl_click_adds_and_removes_one_row() {
        let mut app = test_app("pick-toggle");
        let page = Page::LikedSongs;
        app.pick_row(&page, "v", 1, RowPick::Only, 10);
        app.pick_row(&page, "v", 4, RowPick::Toggle, 10);
        app.pick_row(&page, "v", 7, RowPick::Toggle, 10);
        assert_eq!(picked(&app, &page), vec![1, 4, 7]);
        app.pick_row(&page, "v", 4, RowPick::Toggle, 10);
        assert_eq!(
            picked(&app, &page),
            vec![1, 7],
            "the same click takes it out"
        );
    }

    /// Shift-click selects from the anchor without exceeding the list.
    #[test]
    fn shift_click_takes_the_run_back_to_the_anchor() {
        let mut app = test_app("pick-range");
        let page = Page::LikedSongs;
        app.pick_row(&page, "v", 2, RowPick::Only, 10);
        app.pick_row(&page, "v", 5, RowPick::Range, 10);
        assert_eq!(picked(&app, &page), vec![2, 3, 4, 5]);
        // Back the other way, from the same anchor.
        app.pick_row(&page, "v", 0, RowPick::Range, 10);
        assert_eq!(picked(&app, &page), vec![0, 1, 2]);
        // A list that has since shrunk cannot be reached past its end.
        app.pick_row(&page, "v", 9, RowPick::Range, 4);
        assert_eq!(picked(&app, &page), vec![2, 3]);
    }

    /// Shift-click without an anchor selects one row.
    #[test]
    fn shift_click_with_no_anchor_picks_one_row() {
        let mut app = test_app("pick-no-anchor");
        let page = Page::LikedSongs;
        app.pick_row(&page, "v", 6, RowPick::Range, 10);
        assert_eq!(picked(&app, &page), vec![6]);
    }

    /// Selection clears when sorting, filtering, or paging changes the rows.
    #[test]
    fn the_rows_let_go_when_the_list_moves_underneath() {
        let mut app = test_app("pick-stale");
        let page = Page::LikedSongs;
        app.pick_row(&page, "by-name|", 3, RowPick::Only, 10);
        app.keep_picked_rows_for(&page, "by-name|");
        assert_eq!(picked(&app, &page), vec![3], "the same list keeps them");
        app.keep_picked_rows_for(&page, "by-date|");
        assert!(picked(&app, &page).is_empty(), "a re-sort lets them go");
    }

    /// Selecting rows in another table replaces the current selection.
    #[test]
    fn picking_rows_on_another_page_replaces_the_first() {
        let mut app = test_app("pick-other-page");
        let liked = Page::LikedSongs;
        let album = Page::Album("a".to_string());
        app.pick_row(&liked, "v", 1, RowPick::Only, 10);
        app.pick_row(&album, "v", 2, RowPick::Only, 10);
        assert_eq!(picked(&app, &album), vec![2]);
        assert!(picked(&app, &liked).is_empty());
    }

    fn test_app(name: &str) -> App {
        let root =
            std::env::temp_dir().join(format!("fastsonic-{name}-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        App::new(
            &Waker::default(),
            AppDirs {
                config: root.join("config"),
                state: root.join("state"),
                cache: root.join("cache"),
            },
            Settings::default(),
            AppOptions {
                media_controls: false,
                tray: false,
            },
        )
    }

    fn play(uri: &str, at: &str) -> crate::api::models::PlayHistory {
        crate::api::models::PlayHistory {
            track: crate::api::models::Track {
                uri: uri.to_string(),
                ..Default::default()
            },
            played_at: Some(at.to_string()),
            context: None,
        }
    }

    fn history(
        items: Vec<crate::api::models::PlayHistory>,
        before: Option<&str>,
    ) -> crate::api::models::CursorPage<crate::api::models::PlayHistory> {
        crate::api::models::CursorPage {
            items,
            cursors: Some(crate::api::models::Cursors {
                before: before.map(str::to_string),
                after: None,
            }),
            ..Default::default()
        }
    }

    /// Rule: the Recents tab is a history, so the same song played twice
    /// is two rows. Collapsing repeats would lose what the list is for.
    #[test]
    fn recents_keep_a_song_played_twice() {
        let mut app = test_app("recents-repeat");
        let page = history(
            vec![
                play("sonic:track:a", "2026-09-01T10:00:00Z"),
                play("sonic:track:a", "2026-09-01T09:00:00Z"),
                play("sonic:track:b", "2026-09-01T08:00:00Z"),
            ],
            Some("cursor-1"),
        );
        app.absorb_recents(page, 3);
        assert_eq!(app.recents.items.len(), 3, "both plays of a are kept");
    }

    /// Duplicate play records across page boundaries are removed.
    #[test]
    fn recents_drop_a_play_that_arrives_twice() {
        let mut app = test_app("recents-dedup");
        app.absorb_recents(
            history(
                vec![
                    play("sonic:track:a", "2026-09-01T10:00:00Z"),
                    play("sonic:track:b", "2026-09-01T09:00:00Z"),
                ],
                Some("cursor-1"),
            ),
            2,
        );
        app.absorb_recents(
            history(
                vec![
                    play("sonic:track:b", "2026-09-01T09:00:00Z"),
                    play("sonic:track:c", "2026-09-01T08:00:00Z"),
                ],
                Some("cursor-2"),
            ),
            2,
        );
        let uris: Vec<&str> = app
            .recents
            .items
            .iter()
            .map(|play| play.track.uri.as_str())
            .collect();
        assert_eq!(
            uris,
            vec!["sonic:track:a", "sonic:track:b", "sonic:track:c"]
        );
    }

    /// A short page ends history pagination, even with a cursor.
    #[test]
    fn a_short_page_ends_the_recents_list() {
        let mut app = test_app("recents-short");
        app.absorb_recents(
            history(
                vec![play("sonic:track:a", "2026-09-01T10:00:00Z")],
                Some("more"),
            ),
            50,
        );
        assert!(
            app.recents.complete,
            "a page of one against fifty is the end"
        );
        assert!(
            app.recents.after.is_none(),
            "and there is nothing to ask for"
        );
    }

    /// A full page with a cursor allows another history request.
    #[test]
    fn a_full_page_leaves_the_recents_list_open() {
        let mut app = test_app("recents-full");
        app.absorb_recents(
            history(
                vec![
                    play("sonic:track:a", "2026-09-01T10:00:00Z"),
                    play("sonic:track:b", "2026-09-01T09:00:00Z"),
                ],
                Some("cursor-1"),
            ),
            2,
        );
        assert!(!app.recents.complete);
        assert_eq!(app.recents.after.as_deref(), Some("cursor-1"));
    }

    /// Closing and reopening restores queue rows and their manual split.
    #[test]
    fn the_queue_comes_back_after_a_restart() {
        let root = std::env::temp_dir().join(format!(
            "fastsonic-queue-restart-test-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        let dirs = AppDirs {
            config: root.join("config"),
            state: root.join("state"),
            cache: root.join("cache"),
        };
        let options = AppOptions {
            media_controls: false,
            tray: false,
        };
        let mut app = App::new(
            &Waker::default(),
            dirs.clone(),
            Settings::default(),
            options,
        );
        app.local_ready = true;
        app.resume_track = Some("sonic:track:a".into());
        app.manual_queue = vec!["sonic:track:b".into()];
        app.queue = loaded_queue("sonic:track:a", &["sonic:track:b", "sonic:track:ctx1"]);
        app.save_session();

        let options = AppOptions {
            media_controls: false,
            tray: false,
        };
        let app = App::new(&Waker::default(), dirs, Settings::default(), options);
        let (_, next) = queue_uris(&app);
        assert_eq!(
            next,
            vec!["sonic:track:b", "sonic:track:ctx1"],
            "the queue is shown as it was left"
        );
        assert_eq!(
            app.queued_rows_len(),
            1,
            "the remembered hand-queued song keeps its own section"
        );
    }

    /// Rule 9, end to end: what the engine published is what the session
    /// file keeps, and what the session file keeps is what the resume asks
    /// the next engine for — the song, the position, the songs queued by
    /// hand, and the row the album was on underneath them.
    #[test]
    fn a_closed_session_comes_back_as_the_engine_left_it() {
        let root = std::env::temp_dir().join(format!(
            "fastsonic-session-restore-test-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        let dirs = AppDirs {
            config: root.join("config"),
            state: root.join("state"),
            cache: root.join("cache"),
        };
        let options = AppOptions {
            media_controls: false,
            tray: false,
        };
        let mut app = App::new(
            &Waker::default(),
            dirs.clone(),
            Settings::default(),
            options,
        );
        app.local_ready = true;
        // A queued song playing over an album that has got as far as b.
        app.handle_queue(crate::engine::QueueSnapshot {
            current: Some(queue_row("sonic:track:q1")),
            queued: vec![queue_row("sonic:track:q2")],
            upcoming: vec![queue_row("sonic:track:c")],
            context_uri: Some("sonic:album:x".into()),
            context_at: Some("sonic:track:b".into()),
        });
        app.local.track = Some(crate::engine::LocalTrack {
            uri: "sonic:track:q1".into(),
            duration_ms: 300_000,
            ..Default::default()
        });
        app.local.playback = Playback::Playing;
        app.local.position_ms = 42_000;
        assert_eq!(
            app.playing_context_uri().as_deref(),
            Some("sonic:album:x"),
            "the album playing is the one the engine says it is playing"
        );
        app.save_session();

        let options = AppOptions {
            media_controls: false,
            tray: false,
        };
        let mut app = App::new(&Waker::default(), dirs, Settings::default(), options);
        app.local_ready = true;
        app.local.connected = true;
        assert_eq!(app.resume_track.as_deref(), Some("sonic:track:q1"));
        assert_eq!(app.resume_context.as_deref(), Some("sonic:album:x"));
        assert_eq!(app.resume_context_track.as_deref(), Some("sonic:track:b"));
        assert_eq!(app.resume_queue, vec!["sonic:track:q2"]);
        assert_eq!(app.resume_position_ms, 42_000);
        let (current, next) = queue_uris(&app);
        assert_eq!(current, None, "nothing is playing until play is pressed");
        assert_eq!(
            next,
            vec!["sonic:track:q2", "sonic:track:c"],
            "the panel shows the queue it was closed with"
        );
        app.backend.asked();
        assert!(app.resume_last());
        let asked = app.backend.asked();
        let [PlayerCommand::Load(load)] = asked.as_slice() else {
            panic!("one load resumes the session: {asked:?}");
        };
        assert_eq!(load.context_uri.as_deref(), Some("sonic:album:x"));
        assert_eq!(load.offset_uri.as_deref(), Some("sonic:track:b"));
        assert_eq!(load.current.as_deref(), Some("sonic:track:q1"));
        assert_eq!(load.queued, vec!["sonic:track:q2"]);
        assert_eq!(load.position_ms, 42_000);
    }

    /// Rule 9 across an engine replacement: changing an audio setting
    /// hands the new engine the old one's queue, so the panel keeps
    /// drawing it rather than emptying and waiting to be told again.
    #[test]
    fn changing_an_audio_setting_keeps_the_queue_on_show() {
        let ctx = egui::Context::default();
        let mut app = headless_app();
        app.handle_queue(snapshot(
            Some("sonic:track:a"),
            &["sonic:track:q1"],
            &["sonic:track:b"],
        ));
        app.apply(Action::RestartEngine, &ctx);
        let (current, next) = queue_uris(&app);
        assert_eq!(current.as_deref(), Some("sonic:track:a"));
        assert_eq!(
            next,
            vec!["sonic:track:q1", "sonic:track:b"],
            "the queue survives the swap, because the engine carries it"
        );
        assert_eq!(app.queued_rows_len(), 1);
        assert!(
            app.can_clear_queue(),
            "and it is still a queue the engine can be asked to change"
        );
    }

    /// The album's row is only remembered when the engine's queue and the
    /// player agree on what is playing: a snapshot published for the track
    /// before this one would otherwise remember the album a row behind.
    #[test]
    fn a_stale_snapshot_does_not_move_the_remembered_album_row() {
        let mut app = headless_app();
        app.handle_queue(crate::engine::QueueSnapshot {
            current: Some(queue_row("sonic:track:a")),
            queued: Vec::new(),
            upcoming: vec![queue_row("sonic:track:b")],
            context_uri: Some("sonic:album:x".into()),
            context_at: Some("sonic:track:a".into()),
        });
        // The player has moved on; the queue has not been published yet.
        assert_eq!(app.context_row_under("sonic:track:b"), None);
        // Caught up, and the album's row is the song playing: nothing to
        // remember beyond the song itself.
        app.handle_queue(crate::engine::QueueSnapshot {
            current: Some(queue_row("sonic:track:b")),
            queued: Vec::new(),
            upcoming: Vec::new(),
            context_uri: Some("sonic:album:x".into()),
            context_at: Some("sonic:track:b".into()),
        });
        assert_eq!(app.context_row_under("sonic:track:b"), None);
        // A queued song playing over the album: both are remembered.
        app.handle_queue(crate::engine::QueueSnapshot {
            current: Some(queue_row("sonic:track:q1")),
            queued: Vec::new(),
            upcoming: Vec::new(),
            context_uri: Some("sonic:album:x".into()),
            context_at: Some("sonic:track:b".into()),
        });
        assert_eq!(
            app.context_row_under("sonic:track:q1").as_deref(),
            Some("sonic:track:b")
        );
    }

    /// The remembered song shown on a cold start is the session drawn, not
    /// a session played: saving again while it sits there must not read it
    /// back and lose the album it came out of.
    #[test]
    fn drawing_the_remembered_song_does_not_forget_its_album() {
        use crate::api::models::Track;
        let mut app = headless_app();
        app.resume_track = Some("sonic:track:abc".into());
        app.resume_context = Some("sonic:album:x".into());
        app.resume_context_track = Some("sonic:track:abc".into());
        app.track_cache.insert(
            "abc".into(),
            Track {
                id: Some("abc".into()),
                uri: "sonic:track:abc".into(),
                duration_ms: 264_000,
                ..Default::default()
            },
        );
        assert!(
            app.now_playing().is_some_and(|now| now.resuming),
            "the player bar draws the remembered song"
        );
        app.save_session();
        assert_eq!(
            app.resume_context.as_deref(),
            Some("sonic:album:x"),
            "the album survives a save while the preview is on show"
        );
        assert_eq!(app.resume_context_track.as_deref(), Some("sonic:track:abc"));
    }

    /// Saving the queue writes the playing song and every row after it,
    /// each song once, in playing order.
    #[test]
    fn saving_the_queue_writes_each_song_once_in_order() {
        let mut app = headless_app();
        app.local.track = Some(crate::engine::LocalTrack {
            uri: "sonic:track:a".into(),
            ..Default::default()
        });
        app.local.playback = Playback::Playing;
        app.queue = loaded_queue(
            "sonic:track:a",
            &[
                "sonic:track:b",
                "sonic:track:a",
                "sonic:track:b",
                "sonic:track:c",
            ],
        );
        assert_eq!(
            app.queue_playlist_uris(),
            vec!["sonic:track:a", "sonic:track:b", "sonic:track:c"],
            "the playing song leads and a repeat wrap adds nothing"
        );
    }

    /// A queue saved as a playlist is named after the day.
    #[test]
    fn a_saved_queue_is_named_after_the_day() {
        let app = headless_app();
        assert!(app.queue_playlist_name().starts_with("Queue "));
    }

    /// MilkDrop playback keys produce the same actions as the main window.
    #[cfg(feature = "milkdrop")]
    #[test]
    fn the_milkdrop_window_drives_playback() {
        let mut app = headless_app();
        app.local.track = Some(crate::engine::LocalTrack {
            uri: "sonic:track:a".into(),
            ..Default::default()
        });
        app.local.playback = Playback::Playing;

        for command in [
            "play-pause",
            "next",
            "previous",
            "mute",
            "shuffle",
            "volume-up",
            "volume-down",
        ] {
            app.actions.clear();
            app.milkdrop_command(command);
            assert_eq!(
                app.actions.len(),
                1,
                "{command} asks the player for one thing"
            );
        }

        app.actions.clear();
        app.milkdrop_command("next");
        assert!(matches!(app.actions.first(), Some(Action::Next)));
        app.actions.clear();
        app.milkdrop_command("volume-down");
        assert!(matches!(app.actions.first(), Some(Action::VolumeBy(-5))));

        // Ignore unknown commands.
        app.actions.clear();
        app.milkdrop_command("teleport");
        assert!(app.actions.is_empty());
    }

    /// The first reported screen rate sets the default FPS, but later reports
    /// do not override a configured value.
    #[cfg(feature = "milkdrop")]
    #[test]
    fn the_frame_rate_matches_the_screen_the_first_time_it_is_known() {
        let mut app = headless_app();
        assert_eq!(app.settings.milkdrop_screen_hz, 0, "no screen has spoken");
        assert_eq!(app.settings.milkdrop_fps, crate::milkdrop::DEFAULT_FPS);

        app.learn_screen_hz(144);
        assert_eq!(app.settings.milkdrop_screen_hz, 144);
        assert_eq!(app.settings.milkdrop_fps, 144, "smooth without being asked");

        // Keep the configured FPS when the screen changes.
        app.settings.milkdrop_fps = 30;
        app.learn_screen_hz(60);
        assert_eq!(
            app.settings.milkdrop_screen_hz, 60,
            "the new screen is noted"
        );
        assert_eq!(app.settings.milkdrop_fps, 30, "their number stands");
    }

    /// A fresh app with state directories of its own.
    ///
    /// One directory per app, not one per test run: an app writes its
    /// session when it is asked to save, and a shared directory would hand
    /// that session to whichever test built the next one — which is a
    /// remembered track, a queue and a resume point appearing in a test
    /// that never set them.
    fn headless_app() -> App {
        static NEXT: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
        let count = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let root =
            std::env::temp_dir().join(format!("fastsonic-headless-{}-{count}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let dirs = AppDirs {
            config: root.join("config"),
            state: root.join("state"),
            cache: root.join("cache"),
        };
        let mut app = App::new(
            &Waker::default(),
            dirs,
            Settings::default(),
            AppOptions {
                media_controls: false,
                tray: false,
            },
        );
        app.local_ready = true;
        app
    }

    /// Shuffle picks a random loaded track or Web API offset. Local librespot
    /// playback chooses its own starting track.
    #[test]
    fn a_shuffled_play_does_not_start_at_track_one() {
        use crate::api::models::{
            Album, PlayableItem, Playlist, PlaylistItem, SavedAlbum, Track, TrackCount,
        };
        let track = |uri: &str| {
            Some(PlayableItem::Track(Track {
                uri: uri.into(),
                ..Default::default()
            }))
        };
        let mut app = headless_app();
        app.library.playlists = Loadable::Loaded(vec![
            Playlist {
                uri: "sonic:playlist:open".into(),
                tracks: Some(TrackCount { total: 3 }),
                ..Default::default()
            },
            Playlist {
                uri: "sonic:playlist:unopened".into(),
                tracks: Some(TrackCount { total: 57 }),
                ..Default::default()
            },
        ]);
        app.library.albums.items = vec![SavedAlbum {
            album: Album {
                uri: "sonic:album:saved".into(),
                total_tracks: Some(12),
                ..Default::default()
            },
            ..Default::default()
        }];
        app.library.liked.total = Some(9);
        app.playlist_pages.insert(
            "open".into(),
            PlaylistPage {
                items: PagedList {
                    items: vec![
                        PlaylistItem {
                            item: track("sonic:track:one"),
                            ..Default::default()
                        },
                        PlaylistItem {
                            item: track("sonic:track:two"),
                            ..Default::default()
                        },
                    ],
                    ..Default::default()
                },
                ..Default::default()
            },
        );
        // The playlist on screen: the start is one of its own rows.
        let (uri, position) = app.shuffle_start("sonic:playlist:open");
        assert!(
            matches!(uri.as_deref(), Some("sonic:track:one" | "sonic:track:two")),
            "the start comes from the rows, got {uri:?}"
        );
        assert_eq!(position, None);

        // Nothing saved, nothing loaded: no offset to give.
        assert_eq!(app.shuffle_start("sonic:playlist:unknown"), (None, None));

        // The local engine owns shuffling when no rows are loaded.
        assert_eq!(
            app.shuffle_start("sonic:playlist:unopened"),
            (None, None),
            "the engine is left to draw its own"
        );
    }

    /// One song plays as a context of its own; a list stays a list.
    #[test]
    fn one_song_is_loaded_as_a_context() {
        let one = local_load(&PlayRequest::tracks(vec!["sonic:track:a".into()]), false);
        assert_eq!(one.context_uri.as_deref(), Some("sonic:track:a"));
        assert!(one.uris.is_empty());
        let two = local_load(
            &PlayRequest::tracks(vec!["sonic:track:a".into(), "sonic:track:b".into()])
                .starting_at_index(1),
            true,
        );
        assert_eq!(two.context_uri, None);
        assert_eq!(two.uris.len(), 2);
        assert_eq!(two.offset_index, Some(1));
        // A chosen row keeps the list load straight; shuffle follows as a
        // command (see a_chosen_row_in_a_list_never_loads_shuffled).
        assert_eq!(two.shuffle, None);
    }

    #[test]
    fn copy_link_copies_the_server_independent_uri() {
        let mut app = headless_app();
        let ctx = egui::Context::default();
        ctx.begin_pass(Default::default());
        app.apply(Action::CopyLink("sonic:track:track-1".into()), &ctx);
        let mut output = ctx.end_pass();

        assert!(output.platform_output.commands.iter().any(|command| {
            matches!(command, egui::OutputCommand::CopyText(text) if text == "sonic:track:track-1")
        }));
        output.textures_delta.clear();
    }

    fn snapshot_at(percent: u8) -> LocalState {
        LocalState {
            volume: percent_to_volume(percent),
            ..LocalState::default()
        }
    }

    #[test]
    fn a_volume_set_here_is_saved_immediately() {
        let mut app = headless_app();
        app.set_volume(80, true);
        assert_eq!(volume_to_percent(app.settings.volume), 80);
        assert!(app.settings_dirty);
    }

    #[test]
    fn a_stale_engine_snapshot_does_not_pull_the_volume_back() {
        let mut app = headless_app();
        app.set_volume(80, true);

        // The engine reports `VolumeChanged` asynchronously, so its next
        // snapshot still carries the volume from before the change.
        app.handle_local(snapshot_at(20));
        assert_eq!(volume_to_percent(app.local.volume), 80);
        assert_eq!(volume_to_percent(app.settings.volume), 80);

        // Once it has caught up, its snapshots are trusted again.
        app.handle_local(snapshot_at(80));
        assert_eq!(volume_to_percent(app.local.volume), 80);
    }

    #[test]
    fn a_volume_changed_outside_the_app_is_adopted() {
        let mut app = headless_app();
        app.handle_local(snapshot_at(35));
        assert_eq!(volume_to_percent(app.local.volume), 35);
        assert_eq!(volume_to_percent(app.settings.volume), 35);
    }

    /// What a Raycast script sends becomes the same action a menu pick or a
    /// media key would produce.
    #[test]
    fn a_control_command_becomes_the_action_it_names() {
        // #given
        let mut app = headless_app();
        let queue: std::sync::Arc<std::sync::Mutex<Vec<ControlCommand>>> = Default::default();
        app.control_commands = Some(std::sync::Arc::clone(&queue));

        // #when
        queue.lock().expect("the queue").extend([
            ControlCommand::Next,
            ControlCommand::Previous,
            ControlCommand::SeekBy(-15_000),
            ControlCommand::VolumeBy(10),
            ControlCommand::SetVolume(240),
            ControlCommand::ToggleShuffle,
            ControlCommand::Show,
        ]);
        app.handle_control_commands();

        // #then
        assert!(
            matches!(
                app.actions.as_slice(),
                [
                    Action::Next,
                    Action::Previous,
                    Action::SeekBy(-15_000),
                    Action::VolumeBy(10),
                    // A percentage above the scale is clamped, not wrapped.
                    Action::SetVolume(100),
                    Action::ToggleShuffle,
                    Action::ShowWindow,
                ]
            ),
            "{:?}",
            app.actions
        );
        assert!(queue.lock().expect("the queue").is_empty());
    }

    /// Control clients can set state, seek, and play a URI.
    #[test]
    fn a_key_can_ask_for_a_state_rather_than_a_toggle() {
        // #given
        let mut app = headless_app();
        let queue: std::sync::Arc<std::sync::Mutex<Vec<ControlCommand>>> = Default::default();
        app.control_commands = Some(std::sync::Arc::clone(&queue));

        // #when
        queue.lock().expect("the queue").extend([
            ControlCommand::SetShuffle(true),
            ControlCommand::SetRepeat(RepeatMode::Track),
            ControlCommand::SeekTo(90_000),
            ControlCommand::PlayUri("sonic:playlist:pl1".to_owned()),
            // Nothing is playing in a headless app, so there is no track to
            // save and this one falls away rather than erroring.
            ControlCommand::ToggleSaved,
        ]);
        app.handle_control_commands();

        // #then
        assert!(
            matches!(
                app.actions.as_slice(),
                [
                    Action::SetShuffle(true),
                    Action::SetRepeat(RepeatMode::Track),
                    Action::Seek(90_000),
                    Action::PlayContext {
                        offset_uri: None,
                        offset_index: None,
                        ..
                    },
                ]
            ),
            "{:?}",
            app.actions
        );
        assert!(queue.lock().expect("the queue").is_empty());
    }

    /// New snapshot fields are appended so older clients keep working.
    #[test]
    fn the_snapshot_appends_what_a_key_needs_without_moving_what_was_there() {
        // #given
        let mut app = headless_app();
        app.handle_local(LocalState {
            playback: Playback::Playing,
            track: Some(crate::engine::LocalTrack {
                uri: "sonic:track:t1".to_owned(),
                title: "Go".to_owned(),
                artists: vec!["The Band".to_owned()],
                album: "First".to_owned(),
                art_url: Some("https://i.scdn.co/image/abc".to_owned()),
                duration_ms: 200_000,
                ..Default::default()
            }),
            position_ms: 20_000,
            volume: percent_to_volume(35),
            shuffle: true,
            repeat: RepeatMode::Track,
            ..LocalState::default()
        });

        // #when
        let snapshot = app.control_snapshot();
        let fields: Vec<&str> = snapshot.split('\t').collect();

        // #then
        assert_eq!(
            fields,
            [
                // The nine a media key or a Raycast script already read.
                "playing",
                "Go",
                "The Band",
                "First",
                "20000",
                "200000",
                "35",
                "on",
                "track",
                // The three a Stream Deck key needs, appended.
                "https://i.scdn.co/image/abc",
                // Saved state is unknown before sign-in.
                "unknown",
                "this computer",
            ]
        );
    }

    /// `play` and `pause` say what state to end in, so the one that would
    /// undo the current state does nothing.
    #[test]
    fn play_and_pause_do_not_toggle_the_wrong_way() {
        let mut app = headless_app();
        let queue: std::sync::Arc<std::sync::Mutex<Vec<ControlCommand>>> = Default::default();
        app.control_commands = Some(std::sync::Arc::clone(&queue));

        // Nothing is playing in a headless app, so `pause` has nothing to do
        // and `play` asks for the toggle.
        queue
            .lock()
            .expect("the queue")
            .extend([ControlCommand::Pause, ControlCommand::Play]);
        app.handle_control_commands();

        assert!(
            matches!(app.actions.as_slice(), [Action::TogglePlay]),
            "{:?}",
            app.actions
        );
    }

    /// A skin with a bitmap in it, for pretending one was read.
    fn some_skin(name: &str) -> crate::skin::Skin {
        let image = image::RgbImage::from_pixel(275, 116, image::Rgb([9, 9, 9]));
        let mut png = std::io::Cursor::new(Vec::new());
        image.write_to(&mut png, image::ImageFormat::Png).unwrap();
        let archive = crate::skin::zip::write(&[("main.bmp", png.get_ref(), false)]);
        crate::skin::Skin::from_archive(name, &archive).unwrap()
    }

    #[test]
    fn a_skin_read_late_does_not_override_a_newer_choice() {
        let mut app = headless_app();
        app.settings.winamp_window = true;
        app.settings.skin = Some("B.wsz".into());
        app.skin_loaded(crate::winamp::Loaded {
            name: "A.wsz".into(),
            result: Ok(some_skin("A")),
            installed: false,
        });
        assert_eq!(app.winamp.worn.as_deref(), Some("A.wsz"));
        assert_eq!(app.settings.skin.as_deref(), Some("B.wsz"));
    }

    #[test]
    fn a_dropped_skin_becomes_the_choice_and_a_failed_one_is_forgotten() {
        let mut app = headless_app();
        app.settings.winamp_window = true;
        app.skin_loaded(crate::winamp::Loaded {
            name: "Dropped.wsz".into(),
            result: Ok(some_skin("Dropped")),
            installed: true,
        });
        assert_eq!(app.settings.skin.as_deref(), Some("Dropped.wsz"));
        assert_eq!(app.winamp.worn.as_deref(), Some("Dropped.wsz"));
        assert!(
            app.toasts
                .iter()
                .any(|toast| toast.message == "Added Dropped skin")
        );

        app.settings.skin = Some("Gone.wsz".into());
        app.skin_loaded(crate::winamp::Loaded {
            name: "Gone.wsz".into(),
            result: Err(crate::skin::SkinError::Empty),
            installed: false,
        });
        assert_eq!(app.settings.skin.as_deref(), Some("Dropped.wsz"));
        assert_eq!(app.winamp.worn.as_deref(), Some("Dropped.wsz"));
        assert!(
            app.toasts
                .iter()
                .any(|toast| toast.message.starts_with("Gone: "))
        );
    }
}
