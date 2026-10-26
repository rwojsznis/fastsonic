//! User preferences, stored as one readable JSON file.

use std::path::Path;

use serde::{Deserialize, Serialize};

/// Liked Songs' key among the Library's pins and in its custom order: the
/// starred songs' context URI, which names the same list on every server.
pub const LIKED_SONGS_KEY: &str = crate::api::subsonic::convert::COLLECTION_URI;

/// A section of the sidebar's Library, each with its own sort.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LibraryShelf {
    #[default]
    Playlists,
    Albums,
    Artists,
}

/// How a Library section orders what it lists.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LibrarySort {
    /// The order the server lists them in.
    Library,
    RecentlyPlayed,
    Name,
    /// Playlists by owner, albums by artist.
    Creator,
    /// When an album was starred, which the server records.
    RecentlyAdded,
    /// The playlist arrangement saved by dragging rows.
    Local,
}

impl LibrarySort {
    pub fn supports(self, shelf: LibraryShelf) -> bool {
        match self {
            Self::Library | Self::RecentlyPlayed | Self::Name => true,
            Self::Creator => shelf != LibraryShelf::Artists,
            Self::RecentlyAdded => shelf == LibraryShelf::Albums,
            Self::Local => shelf == LibraryShelf::Playlists,
        }
    }
}

/// Reads `library_sort` one entry at a time, so a section or order this
/// version does not know is dropped instead of making the whole file
/// unreadable.
fn known_library_sorts<'de, D>(
    deserializer: D,
) -> Result<std::collections::BTreeMap<LibraryShelf, LibrarySort>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let saved = std::collections::BTreeMap::<String, serde_json::Value>::deserialize(deserializer)?;
    Ok(saved
        .into_iter()
        .filter_map(|(shelf, sort)| {
            Some((
                serde_json::from_value(serde_json::Value::String(shelf)).ok()?,
                serde_json::from_value(sort).ok()?,
            ))
        })
        .collect())
}

/// How the tray icon is drawn on Linux and Windows: in colour, or white or
/// black to match the panel. The macOS menu bar always gets the template
/// image, which it draws black or white to match itself.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TrayIcon {
    #[default]
    Colour,
    White,
    Black,
}

/// Reads `tray_icon`, taking a name this version does not know as the
/// colour icon instead of making the whole file unreadable.
fn known_tray_icon<'de, D>(deserializer: D) -> Result<TrayIcon, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let saved = serde_json::Value::deserialize(deserializer)?;
    Ok(serde_json::from_value(saved).unwrap_or_default())
}

/// Whether a Home shelf is drawn. Hidden shelves still refresh normally.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct HomeShelfSettings {
    pub visible: bool,
}

impl Default for HomeShelfSettings {
    fn default() -> Self {
        Self { visible: true }
    }
}

/// Which of Home's shelves are drawn, set by hand in `settings.json`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct HomeSettings {
    pub recently_added: HomeShelfSettings,
    pub recently_played: HomeShelfSettings,
    pub top_songs: HomeShelfSettings,
    pub most_played: HomeShelfSettings,
    pub top_artists: HomeShelfSettings,
    pub random: HomeShelfSettings,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ThemeChoice {
    Dark,
    Light,
    #[default]
    System,
}

/// Mini-player visualizer mode.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum VisMode {
    #[default]
    Bars,
    Scope,
    Off,
}

impl VisMode {
    /// Next mode in the display's click cycle.
    pub fn next(self) -> Self {
        match self {
            Self::Bars => Self::Scope,
            Self::Scope => Self::Off,
            Self::Off => Self::Bars,
        }
    }
}

impl ThemeChoice {
    /// In the order the Theme picker lists them, before any palette files.
    pub const ALL: [ThemeChoice; 3] = [Self::System, Self::Light, Self::Dark];

    pub fn label(self) -> &'static str {
        match self {
            Self::Dark => "Dark",
            Self::Light => "Light",
            Self::System => "Follow system",
        }
    }
}

/// What moves behind the main player bar's controls.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PlayerBarVis {
    #[default]
    Off,
    Spectrum,
    Waveform,
}

impl PlayerBarVis {
    pub fn next(self) -> Self {
        match self {
            Self::Off => Self::Spectrum,
            Self::Spectrum => Self::Waveform,
            Self::Waveform => Self::Off,
        }
    }
}

/// An equalizer preset saved by name: the preamp and the ten bands, so a
/// headphone correction curve keeps the headroom it needs.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct UserEqPreset {
    pub name: String,
    #[serde(default)]
    pub preamp_db: f32,
    pub bands_db: [f32; 10],
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub normalisation: bool,
    /// Legacy backend setting retained so existing settings files stay readable.
    pub audio_backend: Option<String>,
    pub audio_device: Option<String>,
    /// Windows output buffer in milliseconds. Smaller values may click under
    /// load; larger values delay playback controls.
    /// See [`crate::sink::DEFAULT_BUFFER_MS`].
    #[serde(default = "default_buffer_ms")]
    pub audio_buffer_ms: u32,
    pub audio_cache: bool,
    pub audio_cache_mb: u64,
    pub theme: ThemeChoice,
    pub home: HomeSettings,
    /// The palette file chosen from the themes folder, by filename. While
    /// set, it is shown instead of `theme`.
    pub custom_theme: Option<String>,
    /// That palette as last read, so the app starts in its colours and keeps
    /// them if the file goes missing or stops parsing.
    #[serde(
        default,
        deserialize_with = "crate::theme::custom::read_cached_theme",
        skip_serializing_if = "Option::is_none"
    )]
    pub custom_theme_cache: Option<crate::theme::custom::CustomTheme>,
    /// The palette files Follow system shows in light and in dark mode,
    /// each with its colours as last read. An empty side is Fastsonic's own
    /// Light or Dark, and the field is written only once a side is chosen.
    #[serde(
        default,
        deserialize_with = "crate::theme::custom::read_theme_pair",
        skip_serializing_if = "crate::theme::custom::ThemePair::is_default"
    )]
    pub theme_pair: crate::theme::custom::ThemePair,
    /// Tint the interface with the colour of the playing album's art.
    pub accent_from_art: bool,
    pub player_bar_vis: PlayerBarVis,
    /// A click on the player bar's empty space moves to the next
    /// visualizer mode.
    pub player_bar_vis_click: bool,
    /// How many frames a second the player bar's visualizer and the mini
    /// player's analyser draw; 0 is uncapped. MilkDrop has its own.
    pub visualizer_fps: u32,
    /// Last local volume, 0..=65535.
    pub volume: u16,
    /// Whether the library sidebar is visible.
    pub sidebar_visible: bool,
    /// The playing album's art docked large at the sidebar's bottom.
    pub art_expanded: bool,
    /// Use compact single-line rows without cover art in the sidebar.
    pub sidebar_compact: bool,
    /// Show the Library as responsive cover cards instead of rows.
    pub sidebar_grid: bool,
    pub sidebar_width: f32,
    pub lyrics_width: f32,
    pub queue_width: f32,
    /// Use compact single-line rows without cover art in track lists.
    pub tracklist_compact: bool,
    /// Linux: middle-click a list to autoscroll it. Off by default, because
    /// Linux desktops usually paste the primary selection on middle click.
    /// Windows always autoscrolls and macOS never does.
    pub middle_click_autoscroll: bool,
    pub search_history: Vec<String>,
    pub show_shortcut_hints: bool,
    /// Local playback has been authorized at least once on this machine, so
    /// the app can resume it silently instead of prompting.
    pub playback_authorized: bool,
    /// Closing the window hides to the tray and keeps the music playing.
    pub keep_playing_in_background: bool,
    /// Linux and Windows: the tray icon in colour, white or black.
    #[serde(deserialize_with = "known_tray_icon")]
    pub tray_icon: TrayIcon,
    /// macOS: hovering over a MacBook's notch while Fastsonic is in the
    /// background shows what is playing, with the player bar's controls.
    pub mac_notch_widget: bool,
    /// macOS: keep Fastsonic in the Dock and Cmd-Tab. Off makes it a
    /// menu-bar app, whose menu-bar icon is the way back.
    pub mac_show_in_dock: bool,
    /// Ask GitHub once a day whether a newer release exists.
    pub check_for_updates: bool,
    /// Context URIs pinned to the top of the sidebar, Liked Songs' among
    /// them, in pin order.
    pub pinned_contexts: Vec<String>,
    /// Liked Songs sits in the pin block. Older settings keep it first
    /// until it is moved or unpinned.
    pub liked_songs_pinned: bool,
    /// The sidebar's own playlist order, set by dragging rows. Kept while
    /// another sort is selected; empty means no saved local arrangement.
    pub sidebar_order: Vec<String>,
    /// The order chosen for each Library section. A missing section keeps
    /// its previous behaviour; choosing another order never deletes the
    /// local arrangement.
    #[serde(deserialize_with = "known_library_sorts")]
    pub library_sort: std::collections::BTreeMap<LibraryShelf, LibrarySort>,
    /// The Playlists section shows only the signed-in account's own
    /// playlists, beside Liked Songs.
    pub library_by_you: bool,
    /// Interface zoom, egui's zoom factor; Ctrl+plus/minus changes it.
    pub zoom: f32,
    /// Windows and Linux: draw Fastsonic's own title bar and window buttons
    /// instead of the desktop's frame.
    pub custom_titlebar: bool,
    /// The Winamp window is open.
    pub winamp_window: bool,
    /// Windows and X11: keep a taskbar button while the Winamp window is visible.
    pub winamp_show_taskbar: bool,
    /// Skin file or folder name. `None` selects the built-in skin.
    pub skin: Option<String>,
    /// Choose another installed skin whenever the mini player opens.
    pub random_skin: bool,
    /// Screen pixels per skin pixel; `None` picks double size for the
    /// display.
    pub skin_scale: Option<u8>,
    /// The Winamp window stays above other windows.
    pub winamp_on_top: bool,
    /// The mini player's visualiser: bars, scope, or off.
    pub vis: VisMode,
    /// The playlist window is open under the mini player.
    pub playlist_open: bool,
    /// How tall the playlist window is, in skin pixels.
    pub playlist_height: u32,
    /// The equalizer window is open under the mini player.
    pub eq_open: bool,
    /// The equalizer shapes local playback.
    pub eq_on: bool,
    /// The preamp, in decibels, never above zero.
    pub eq_preamp_db: f32,
    /// The ten bands, in decibels, 60 Hz to 16 kHz.
    pub eq_bands_db: [f32; 10],
    /// Equalizer presets saved here, listed after the built-in ones.
    pub eq_user_presets: Vec<UserEqPreset>,
    /// The balance, -1 all left to 1 all right.
    pub balance: f32,
    /// Play both channels the same.
    pub mono: bool,
    /// The playlist window is rolled up to its title bar.
    pub playlist_shaded: bool,
    /// The equalizer window is rolled up to its title bar.
    pub eq_shaded: bool,
    /// The main window is rolled up to its title bar.
    pub winamp_shaded: bool,
    /// The MilkDrop window is open (its own window, not part of the skin).
    pub milkdrop_open: bool,
    /// How long each preset plays before the next, in seconds.
    pub milkdrop_seconds: u32,
    /// How many frames a second the MilkDrop window draws; 0 is uncapped.
    pub milkdrop_fps: u32,
    /// Last reported MilkDrop screen refresh rate. The first value sets the
    /// default frame rate; this field is not directly configurable.
    pub milkdrop_screen_hz: u32,
    /// The picture's inner resolution: 1 full, 2 half, 4 quarter.
    pub milkdrop_scale: u32,
    /// The MilkDrop window fills the screen.
    pub milkdrop_fullscreen: bool,
    /// The MilkDrop window's size in logical points, when not full-screen.
    pub milkdrop_size: [f32; 2],
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            normalisation: false,
            audio_backend: None,
            audio_device: None,
            audio_buffer_ms: default_buffer_ms(),
            audio_cache: true,
            audio_cache_mb: 1024,
            theme: ThemeChoice::System,
            home: HomeSettings::default(),
            custom_theme: None,
            custom_theme_cache: None,
            theme_pair: Default::default(),
            accent_from_art: true,
            player_bar_vis: PlayerBarVis::Off,
            player_bar_vis_click: true,
            visualizer_fps: crate::milkdrop::DEFAULT_FPS,
            volume: (u16::MAX as u32 * 70 / 100) as u16,
            sidebar_visible: true,
            art_expanded: false,
            sidebar_compact: false,
            sidebar_grid: false,
            sidebar_width: 250.0,
            lyrics_width: 360.0,
            queue_width: 360.0,
            tracklist_compact: false,
            middle_click_autoscroll: false,
            search_history: Vec::new(),
            show_shortcut_hints: true,
            playback_authorized: false,
            keep_playing_in_background: true,
            tray_icon: TrayIcon::Colour,
            mac_notch_widget: false,
            mac_show_in_dock: true,
            check_for_updates: true,
            pinned_contexts: Vec::new(),
            liked_songs_pinned: true,
            sidebar_order: Vec::new(),
            library_sort: std::collections::BTreeMap::new(),
            library_by_you: false,
            zoom: 1.0,
            custom_titlebar: false,
            winamp_window: false,
            winamp_show_taskbar: true,
            skin: None,
            random_skin: false,
            skin_scale: None,
            winamp_on_top: false,
            vis: VisMode::default(),
            playlist_open: false,
            playlist_height: 174,
            eq_open: false,
            eq_on: false,
            eq_preamp_db: 0.0,
            eq_bands_db: [0.0; 10],
            eq_user_presets: Vec::new(),
            balance: 0.0,
            mono: false,
            playlist_shaded: false,
            eq_shaded: false,
            winamp_shaded: false,
            milkdrop_open: false,
            milkdrop_seconds: crate::milkdrop::DEFAULT_SECONDS,
            milkdrop_fps: crate::milkdrop::DEFAULT_FPS,
            milkdrop_screen_hz: 0,
            milkdrop_scale: 1,
            milkdrop_fullscreen: false,
            milkdrop_size: crate::milkdrop::DEFAULT_SIZE,
        }
    }
}

fn default_buffer_ms() -> u32 {
    crate::sink::DEFAULT_BUFFER_MS
}

impl Settings {
    /// The Library's pins in order, Liked Songs' key included while it is
    /// pinned: where it was put, or first when it was never moved.
    pub fn library_pins(&self) -> Vec<String> {
        let mut pins = self.pinned_contexts.clone();
        if !self.liked_songs_pinned {
            pins.retain(|key| key != LIKED_SONGS_KEY);
        } else if !pins.iter().any(|key| key == LIKED_SONGS_KEY) {
            pins.insert(0, LIKED_SONGS_KEY.into());
        }
        pins
    }

    pub fn load(path: &Path) -> Self {
        match std::fs::read_to_string(path) {
            Ok(text) => {
                // A file edited in Notepad or Windows PowerShell can start with
                // a byte order mark, which JSON does not allow.
                let text = text.strip_prefix('\u{feff}').unwrap_or(&text);
                let mut settings: Self = serde_json::from_str(text).unwrap_or_else(|error| {
                    log::warn!("settings at {} are unreadable: {error}", path.display());
                    Self::default()
                });
                settings.forget_unusable_screen_rate();
                settings
            }
            Err(_) => Self::default(),
        }
    }

    /// Forgets the frame rate an earlier version set from Windows's 1 Hz
    /// "hardware default", taken then as MilkDrop's first screen rate, and
    /// that screen rate with it, so MilkDrop's next real report is learned
    /// as the first. A frame rate chosen since, or learned from a real
    /// screen before, stays, and so does the 1 Hz beside it, which no dial
    /// shows and the next real report replaces.
    pub(crate) fn forget_unusable_screen_rate(&mut self) {
        let hz = self.milkdrop_screen_hz;
        if hz == 0 || crate::milkdrop::is_screen_rate(hz) || self.milkdrop_fps != hz {
            return;
        }
        self.milkdrop_fps = crate::milkdrop::DEFAULT_FPS;
        self.milkdrop_screen_hz = 0;
    }

    /// The settings as their file holds them.
    pub fn encode(&self) -> Option<String> {
        serde_json::to_string_pretty(self)
            .map_err(|error| log::warn!("unable to encode settings: {error}"))
            .ok()
    }

    /// Writes the settings and waits for the disk. The app saves through
    /// its [`crate::file_writer::FileWriter`] instead.
    pub fn save(&self, path: &Path) {
        let Some(text) = self.encode() else { return };
        if let Err(error) = crate::util::write_file(path, text) {
            log::warn!("unable to save settings to {}: {error}", path.display());
        }
    }

    pub fn platform_backend(&self) -> Option<String> {
        self.audio_backend.clone().or_else(|| {
            if cfg!(target_os = "linux") {
                Some("pulseaudio".to_string())
            } else {
                None
            }
        })
    }

    /// The chosen palette file's colours as last read, when one is chosen.
    pub fn cached_palette(&self) -> Option<crate::theme::Palette> {
        let chosen = self.custom_theme.as_deref()?;
        self.custom_theme_cache
            .as_ref()
            .filter(|theme| theme.filename == chosen)
            .map(|theme| theme.palette)
    }

    /// The palette to show while the system is `dark` or light: the chosen
    /// palette file, else Follow system's choice for that appearance.
    /// `None` leaves Fastsonic's own Light or Dark.
    pub fn palette_for(&self, dark: bool) -> Option<crate::theme::Palette> {
        if self.custom_theme.is_some() {
            return self.cached_palette();
        }
        (self.theme == ThemeChoice::System)
            .then(|| self.theme_pair.palette(dark))
            .flatten()
    }

    pub fn remember_search(&mut self, query: &str) {
        let query = query.trim();
        if query.is_empty() {
            return;
        }
        self.search_history.retain(|entry| entry != query);
        self.search_history.insert(0, query.to_string());
        self.search_history.truncate(12);
    }
}

#[cfg(test)]
mod tests {
    use super::Settings;

    #[test]
    fn partial_home_preferences_keep_defaults_and_survive_a_settings_round_trip() {
        for (home, random, top_artists) in [
            ("{}", true, true),
            (r#"{"random":{"visible":false}}"#, false, true),
            (r#"{"top_artists":{"visible":false}}"#, true, false),
            (
                r#"{"random":{"visible":false},"top_artists":{"visible":false}}"#,
                false,
                false,
            ),
        ] {
            let settings: Settings =
                serde_json::from_str(&format!(r#"{{"volume":12345,"home":{home}}}"#)).unwrap();
            assert_eq!(settings.home.random.visible, random);
            assert_eq!(settings.home.top_artists.visible, top_artists);
            assert!(settings.home.recently_added.visible);
            assert_eq!(settings.volume, 12345);
            let restored: Settings =
                serde_json::from_str(&serde_json::to_string(&settings).unwrap()).unwrap();
            assert_eq!(restored, settings);
        }
        let old: Settings = serde_json::from_str(r#"{"volume":12345}"#).unwrap();
        assert_eq!(old.home, super::HomeSettings::default());
        assert!(old.home.most_played.visible);
        assert_eq!(old.volume, 12345);
    }

    #[test]
    fn the_notch_card_is_off_in_older_files_and_stays_on_once_chosen() {
        let older: Settings = serde_json::from_str("{}").unwrap();
        assert!(!older.mac_notch_widget);
        let chosen: Settings = serde_json::from_str(r#"{"mac_notch_widget":true}"#).unwrap();
        assert!(chosen.mac_notch_widget);
        let restored: Settings =
            serde_json::from_str(&serde_json::to_string(&chosen).unwrap()).unwrap();
        assert!(restored.mac_notch_widget);
    }

    /// Older files keep the colour icon, and the choice round-trips as a
    /// lowercase word; a word this version does not know keeps it too.
    #[test]
    fn the_tray_icon_stays_in_colour_unless_chosen_and_round_trips() {
        use super::TrayIcon;
        let settings: Settings = serde_json::from_str("{}").unwrap();
        assert_eq!(settings.tray_icon, TrayIcon::Colour);
        for (choice, word) in [
            (TrayIcon::Colour, "colour"),
            (TrayIcon::White, "white"),
            (TrayIcon::Black, "black"),
        ] {
            let json = serde_json::to_value(Settings {
                tray_icon: choice,
                ..Settings::default()
            })
            .unwrap();
            assert_eq!(json["tray_icon"], word);
            let restored: Settings = serde_json::from_value(json).unwrap();
            assert_eq!(restored.tray_icon, choice);
        }
        let newer: Settings =
            serde_json::from_str(r#"{"tray_icon":"rainbow","volume":7}"#).unwrap();
        assert_eq!(newer.tray_icon, TrayIcon::Colour);
        assert_eq!(newer.volume, 7, "the rest of the file still reads");
    }

    /// Older files keep the Dock icon; hiding it survives a save.
    #[test]
    fn the_dock_icon_stays_unless_hidden_and_round_trips() {
        let empty: Settings = serde_json::from_str("{}").unwrap();
        assert!(empty.mac_show_in_dock);
        let hidden: Settings = serde_json::from_str(r#"{"mac_show_in_dock": false}"#).unwrap();
        assert!(!hidden.mac_show_in_dock);
        let encoded = serde_json::to_string(&hidden).unwrap();
        let decoded: Settings = serde_json::from_str(&encoded).unwrap();
        assert!(!decoded.mac_show_in_dock);
    }

    #[test]
    fn player_bar_visualizer_defaults_off_and_cycles_through_modes() {
        use super::PlayerBarVis;
        let settings: Settings = serde_json::from_str("{}").unwrap();
        assert_eq!(settings.player_bar_vis, PlayerBarVis::Off);
        assert!(
            settings.player_bar_vis_click,
            "an older file keeps clicks on"
        );
        let disabled = Settings {
            player_bar_vis_click: false,
            ..Settings::default()
        };
        let json = serde_json::to_string(&disabled).unwrap();
        let restored: Settings = serde_json::from_str(&json).unwrap();
        assert!(!restored.player_bar_vis_click);
        let spectrum: Settings = serde_json::from_str(r#"{"player_bar_vis":"spectrum"}"#).unwrap();
        assert_eq!(spectrum.player_bar_vis, PlayerBarVis::Spectrum);
        assert_eq!(PlayerBarVis::Off.next(), PlayerBarVis::Spectrum);
        assert_eq!(PlayerBarVis::Spectrum.next(), PlayerBarVis::Waveform);
        assert_eq!(PlayerBarVis::Waveform.next(), PlayerBarVis::Off);
    }

    /// The main window's visualizers draw 60 frames a second until the user
    /// picks another rate, and the rate they pick, uncapped included, is
    /// the rate they get back, without touching MilkDrop's.
    #[test]
    fn the_visualizer_frame_rate_defaults_to_60_and_round_trips() {
        let settings: Settings = serde_json::from_str("{}").unwrap();
        assert_eq!(settings.visualizer_fps, 60);
        assert_eq!(Settings::default().visualizer_fps, 60);
        for rate in [30, 144, 0] {
            let chosen = Settings {
                visualizer_fps: rate,
                ..Settings::default()
            };
            let json = serde_json::to_string(&chosen).unwrap();
            let restored: Settings = serde_json::from_str(&json).unwrap();
            assert_eq!(restored.visualizer_fps, rate);
            assert_eq!(restored.milkdrop_fps, Settings::default().milkdrop_fps);
        }
        let milkdrop_only: Settings =
            serde_json::from_value(serde_json::json!({"milkdrop_fps": 30})).unwrap();
        assert_eq!(
            milkdrop_only.visualizer_fps, 60,
            "MilkDrop's rate is its own"
        );
    }

    /// Earlier versions kept Windows's 1 Hz "hardware default" as MilkDrop's
    /// screen rate and, the first time, as its frame rate. Loading forgets
    /// both, and leaves a frame rate chosen since, and a real rate, alone.
    #[test]
    fn a_saved_hardware_default_screen_rate_is_forgotten() {
        let dir =
            std::env::temp_dir().join(format!("fastsonic-screen-default-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("settings.json");
        let load = |text: &str| {
            std::fs::write(&path, text).unwrap();
            let settings = Settings::load(&path);
            (settings.milkdrop_screen_hz, settings.milkdrop_fps)
        };
        let learned = load(r#"{"milkdrop_screen_hz":1,"milkdrop_fps":1}"#);
        let chosen = load(r#"{"milkdrop_screen_hz":1,"milkdrop_fps":30}"#);
        let sixty = load(r#"{"milkdrop_screen_hz":1,"milkdrop_fps":60}"#);
        let real = load(r#"{"milkdrop_screen_hz":144,"milkdrop_fps":144}"#);
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(learned, (0, crate::milkdrop::DEFAULT_FPS));
        assert_eq!(chosen, (1, 30), "their rate stands");
        assert_eq!(
            sixty,
            (1, 60),
            "a chosen 60 is not taken for the default, which a real report would replace"
        );
        assert_eq!(real, (144, 144), "a real rate stays");
    }

    #[test]
    fn older_settings_preserve_the_selected_skin_without_random_mode() {
        let settings: Settings = serde_json::from_str(r#"{"skin":"A.wsz"}"#).unwrap();
        assert!(!settings.random_skin);
        assert_eq!(settings.skin.as_deref(), Some("A.wsz"));
    }

    /// Notepad and Windows PowerShell can save UTF-8 with a byte order mark.
    /// A file edited that way is still read rather than dropped as
    /// unreadable and replaced with the defaults.
    #[test]
    fn a_settings_file_saved_with_a_byte_order_mark_keeps_its_preferences() {
        let dir = std::env::temp_dir().join(format!("fastsonic-bom-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("settings.json");
        std::fs::write(
            &path,
            "\u{feff}{\"volume\":12345,\"sidebar_visible\":false}",
        )
        .unwrap();
        let settings = Settings::load(&path);
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(settings.volume, 12345);
        assert!(!settings.sidebar_visible);
    }

    /// A new profile follows the desktop; a theme chosen before is kept,
    /// since every saved settings file names one.
    #[test]
    fn new_profiles_follow_the_system_and_saved_choices_are_preserved() {
        use super::ThemeChoice;
        assert_eq!(Settings::default().theme, ThemeChoice::System);
        assert_eq!(ThemeChoice::ALL[0], ThemeChoice::System);
        for (saved, choice) in [
            ("dark", ThemeChoice::Dark),
            ("light", ThemeChoice::Light),
            ("system", ThemeChoice::System),
        ] {
            let settings: Settings =
                serde_json::from_str(&format!(r#"{{"theme":"{saved}"}}"#)).unwrap();
            assert_eq!(settings.theme, choice);
        }
        let saved = serde_json::to_string(&Settings {
            theme: ThemeChoice::Dark,
            ..Settings::default()
        })
        .unwrap();
        assert!(saved.contains(r#""theme":"dark""#), "{saved}");
    }

    #[test]
    fn the_built_in_themes_read_follow_system_light_dark() {
        use super::ThemeChoice;
        assert_eq!(
            ThemeChoice::ALL,
            [ThemeChoice::System, ThemeChoice::Light, ThemeChoice::Dark]
        );
    }

    /// A file from before palette files chooses none and caches none, and
    /// writes neither back until one is chosen.
    #[test]
    fn older_settings_choose_no_palette_file() {
        let settings: Settings = serde_json::from_str(r#"{"theme":"light"}"#).unwrap();
        assert_eq!(settings.custom_theme, None);
        assert_eq!(settings.custom_theme_cache, None);
        assert_eq!(settings.cached_palette(), None);
        let saved = serde_json::to_string(&settings).unwrap();
        assert!(!saved.contains("custom_theme_cache"), "{saved}");
    }

    #[test]
    fn a_cached_palette_round_trips_and_a_damaged_one_keeps_the_rest() {
        use crate::theme::{Palette, custom::CustomTheme};
        let mut palette = Palette::light();
        palette.shadow = egui::Color32::from_rgba_unmultiplied(37, 128, 249, 117);
        let settings = Settings {
            custom_theme: Some("gruvbox.json".into()),
            custom_theme_cache: Some(CustomTheme {
                filename: "gruvbox.json".into(),
                palette,
            }),
            ..Settings::default()
        };
        let encoded = serde_json::to_string(&settings).unwrap();
        let restored: Settings = serde_json::from_str(&encoded).unwrap();
        assert_eq!(restored, settings);
        assert_eq!(restored.cached_palette(), Some(palette));

        // A cache saved before palettes had a tint is still the palette.
        let mut older = serde_json::to_value(&settings).unwrap();
        let cached = older["custom_theme_cache"]["palette"]
            .as_object_mut()
            .unwrap();
        assert!(cached.remove("tint").is_some());
        let restored: Settings = serde_json::from_value(older).unwrap();
        assert_eq!(restored.cached_palette(), Some(palette));

        let mut damaged = serde_json::to_value(&settings).unwrap();
        damaged["custom_theme_cache"] = serde_json::json!({"palette": "broken"});
        damaged["audio_cache_mb"] = 777.into();
        let recovered: Settings = serde_json::from_value(damaged).unwrap();
        assert_eq!(recovered.custom_theme.as_deref(), Some("gruvbox.json"));
        assert_eq!(recovered.audio_cache_mb, 777);
        assert_eq!(recovered.custom_theme_cache, None);
        assert_eq!(recovered.cached_palette(), None);
    }

    /// A cache left from another file, say after `custom_theme` was edited
    /// by hand, is not that file's colours.
    #[test]
    fn a_cache_counts_only_for_the_file_it_was_read_from() {
        use crate::theme::{Palette, custom::CustomTheme};
        let settings = Settings {
            custom_theme: Some("other.json".into()),
            custom_theme_cache: Some(CustomTheme {
                filename: "gruvbox.json".into(),
                palette: Palette::light(),
            }),
            ..Settings::default()
        };
        assert_eq!(settings.cached_palette(), None);
        let unchosen = Settings {
            custom_theme: None,
            ..settings
        };
        assert_eq!(unchosen.cached_palette(), None);
    }

    /// Settings from before the light and dark mode themes keep their look,
    /// and write no pair until a side is chosen.
    #[test]
    fn follow_system_pairs_load_old_settings_unchanged_and_round_trip() {
        use super::ThemeChoice;
        use crate::theme::{Palette, custom::CustomTheme};
        let mut nord = Palette::dark();
        nord.accent = egui::Color32::from_rgb(0x88, 0xc0, 0xd0);
        let cache = serde_json::to_value(CustomTheme {
            filename: "Nord.json".into(),
            palette: nord,
        })
        .unwrap();
        // One palette file, with Follow system underneath: Nord in light
        // and dark mode alike.
        let single: Settings = serde_json::from_value(serde_json::json!({
            "theme": "system", "custom_theme": "Nord.json", "custom_theme_cache": cache,
        }))
        .unwrap();
        assert!(single.theme_pair.is_default());
        assert_eq!(single.palette_for(false), Some(nord));
        assert_eq!(single.palette_for(true), Some(nord));
        // Plain Follow system: Fastsonic's own Light and Dark.
        let follow: Settings = serde_json::from_str(r#"{"theme":"system"}"#).unwrap();
        assert_eq!(follow.palette_for(false), None);
        assert_eq!(follow.palette_for(true), None);
        let encoded = serde_json::to_value(&follow).unwrap();
        assert!(encoded.get("theme_pair").is_none(), "no pair, no field");

        let mut latte = Palette::light();
        latte.window = egui::Color32::from_rgb(0xef, 0xf1, 0xf5);
        let mut paired = follow.clone();
        assert!(paired.theme_pair.set(
            false,
            Some(&CustomTheme {
                filename: "Catppuccin Latte.json".into(),
                palette: latte,
            })
        ));
        assert_eq!(paired.palette_for(false), Some(latte));
        assert_eq!(paired.palette_for(true), None, "dark stays Fastsonic's");
        let encoded = serde_json::to_string(&paired).unwrap();
        let decoded: Settings = serde_json::from_str(&encoded).unwrap();
        assert_eq!(decoded, paired);

        // A pair waits behind a fixed choice and comes back with Follow
        // system.
        let mut fixed = paired.clone();
        fixed.theme = ThemeChoice::Dark;
        assert_eq!(fixed.palette_for(false), None);
        fixed.theme = ThemeChoice::System;
        assert_eq!(fixed.palette_for(false), Some(latte));

        // A damaged pair, whole or in part, keeps the other settings.
        let mut damaged = serde_json::to_value(&paired).unwrap();
        damaged["theme_pair"]["light"]["palette"] = "broken".into();
        damaged["audio_cache_mb"] = 777.into();
        let recovered: Settings = serde_json::from_value(damaged.clone()).unwrap();
        assert_eq!(
            recovered.theme_pair.filename(false),
            Some("Catppuccin Latte.json")
        );
        assert_eq!(recovered.palette_for(false), None);
        assert_eq!(recovered.audio_cache_mb, 777);
        damaged["theme_pair"] = "broken".into();
        let recovered: Settings = serde_json::from_value(damaged).unwrap();
        assert!(recovered.theme_pair.is_default());
        assert_eq!(recovered.audio_cache_mb, 777);
    }

    #[test]
    fn older_settings_keep_the_sidebar_visible() {
        let settings: Settings = serde_json::from_str("{}").unwrap();
        assert!(settings.sidebar_visible);
    }

    #[test]
    fn retired_personal_app_setting_is_ignored() {
        let settings: Settings =
            serde_json::from_str(r#"{"web_client_id":"retired-client"}"#).unwrap();
        assert_eq!(settings, Settings::default());
    }

    #[test]
    fn retired_playback_settings_are_ignored_and_not_written_again() {
        let settings: Settings =
            serde_json::from_str(r#"{"bitrate":96,"autoplay":false,"gapless":false}"#).unwrap();
        assert_eq!(settings, Settings::default());

        let json = serde_json::to_string(&settings).unwrap();
        assert!(!json.contains("bitrate"));
        assert!(!json.contains("autoplay"));
        assert!(!json.contains("gapless"));
    }

    #[test]
    fn older_settings_keep_the_winamp_window_closed_and_the_built_in_skin() {
        let settings: Settings = serde_json::from_str(r#"{"zoom": 1.2}"#).unwrap();
        assert!(!settings.winamp_window);
        assert!(settings.winamp_show_taskbar);
        assert_eq!(settings.skin, None);
        assert_eq!(settings.skin_scale, None);
        assert!(!settings.winamp_on_top);
        assert_eq!(settings.vis, super::VisMode::Bars);
        assert!(!settings.playlist_open);
        assert_eq!(settings.playlist_height, 174);
        assert!(!settings.eq_on);
        assert_eq!(settings.eq_bands_db, [0.0; 10]);
        assert_eq!(settings.balance, 0.0);
        assert!(!settings.mono);
        assert!(!settings.playlist_shaded);
        assert!(!settings.eq_shaded);
        assert!(!settings.winamp_shaded);
    }

    #[test]
    fn the_visualiser_cycles_bars_scope_off() {
        use super::VisMode;
        assert_eq!(VisMode::Bars.next(), VisMode::Scope);
        assert_eq!(VisMode::Scope.next(), VisMode::Off);
        assert_eq!(VisMode::Off.next(), VisMode::Bars);
        let settings: Settings = serde_json::from_str(r#"{"vis": "scope"}"#).unwrap();
        assert_eq!(settings.vis, VisMode::Scope);
    }

    #[test]
    fn a_chosen_skin_round_trips() {
        let settings = Settings {
            winamp_window: true,
            skin: Some("Zaxon.wsz".into()),
            skin_scale: Some(3),
            ..Settings::default()
        };
        let json = serde_json::to_string(&settings).unwrap();
        let restored: Settings = serde_json::from_str(&json).unwrap();
        assert_eq!(restored, settings);
    }

    #[test]
    fn hidden_sidebar_round_trips() {
        let settings = Settings {
            sidebar_visible: false,
            ..Settings::default()
        };
        let json = serde_json::to_string(&settings).unwrap();
        let restored: Settings = serde_json::from_str(&json).unwrap();
        assert!(!restored.sidebar_visible);
    }

    #[test]
    fn older_settings_default_to_standard_sidebar() {
        let settings: Settings = serde_json::from_str("{}").unwrap();
        assert!(!settings.sidebar_compact);
    }

    #[test]
    fn compact_sidebar_round_trips() {
        let settings = Settings {
            sidebar_compact: true,
            ..Settings::default()
        };
        let json = serde_json::to_string(&settings).unwrap();
        let restored: Settings = serde_json::from_str(&json).unwrap();
        assert!(restored.sidebar_compact);
    }

    #[test]
    fn older_library_settings_keep_liked_songs_ahead_of_existing_pins() {
        let settings: Settings = serde_json::from_str(
            r#"{"pinned_contexts":["sonic:playlist:one"],"sidebar_order":["sonic:playlist:two"]}"#,
        )
        .unwrap();
        assert!(settings.liked_songs_pinned);
        assert_eq!(
            settings.library_pins(),
            [super::LIKED_SONGS_KEY, "sonic:playlist:one"]
        );
        assert_eq!(settings.sidebar_order, ["sonic:playlist:two"]);

        let moved = Settings {
            pinned_contexts: vec!["sonic:playlist:one".into(), super::LIKED_SONGS_KEY.into()],
            ..Settings::default()
        };
        let restored: Settings =
            serde_json::from_str(&serde_json::to_string(&moved).unwrap()).unwrap();
        assert_eq!(
            restored.library_pins(),
            ["sonic:playlist:one", super::LIKED_SONGS_KEY]
        );
        let unpinned = Settings {
            liked_songs_pinned: false,
            pinned_contexts: vec![super::LIKED_SONGS_KEY.into()],
            ..Settings::default()
        };
        assert!(unpinned.library_pins().is_empty());
    }

    #[test]
    fn library_sorts_round_trip_and_older_settings_keep_the_previous_order() {
        use super::{LibraryShelf, LibrarySort};
        let older: Settings =
            serde_json::from_str(r#"{"sidebar_order":["sonic:playlist:a"]}"#).unwrap();
        assert!(older.library_sort.is_empty());
        assert_eq!(older.sidebar_order, ["sonic:playlist:a"]);

        let mut settings = Settings::default();
        settings
            .library_sort
            .insert(LibraryShelf::Playlists, LibrarySort::Name);
        settings
            .library_sort
            .insert(LibraryShelf::Albums, LibrarySort::RecentlyAdded);
        let json = serde_json::to_string(&settings).unwrap();
        assert!(
            json.contains(r#""library_sort":{"playlists":"name","albums":"recently_added"}"#),
            "{json}"
        );
        let restored: Settings = serde_json::from_str(&json).unwrap();
        assert_eq!(restored, settings);
    }

    /// A section or order from another version is dropped on its own; the
    /// rest of the file, and the other sections' orders, are kept.
    #[test]
    fn an_unknown_library_sort_does_not_reset_the_settings() {
        use super::{LibraryShelf, LibrarySort};
        let settings: Settings = serde_json::from_str(
            r#"{"volume":123,"library_sort":{"podcasts":"name","playlists":"spotify","artists":"name"}}"#,
        )
        .unwrap();
        assert_eq!(settings.volume, 123);
        assert_eq!(
            settings.library_sort.into_iter().collect::<Vec<_>>(),
            [(LibraryShelf::Artists, LibrarySort::Name)]
        );
    }

    #[test]
    fn library_by_you_round_trips_and_older_settings_show_every_playlist() {
        let older: Settings = serde_json::from_str("{}").unwrap();
        assert!(!older.library_by_you);
        let settings = Settings {
            library_by_you: true,
            ..Settings::default()
        };
        let json = serde_json::to_string(&settings).unwrap();
        assert!(json.contains(r#""library_by_you":true"#), "{json}");
        let restored: Settings = serde_json::from_str(&json).unwrap();
        assert!(restored.library_by_you);
    }

    #[test]
    fn library_grid_round_trips_and_older_settings_keep_the_list() {
        let old: Settings = serde_json::from_str(r#"{"sidebar_compact":true}"#).unwrap();
        assert!(old.sidebar_compact);
        assert!(!old.sidebar_grid);

        let settings = Settings {
            sidebar_grid: true,
            ..Settings::default()
        };
        let json = serde_json::to_string(&settings).unwrap();
        let restored: Settings = serde_json::from_str(&json).unwrap();
        assert!(restored.sidebar_grid);
    }

    #[test]
    fn older_settings_default_to_standard_tracklist() {
        let settings: Settings = serde_json::from_str("{}").unwrap();
        assert!(!settings.tracklist_compact);
    }

    #[test]
    fn compact_tracklist_round_trips() {
        let settings = Settings {
            tracklist_compact: true,
            ..Settings::default()
        };
        let json = serde_json::to_string(&settings).unwrap();
        let restored: Settings = serde_json::from_str(&json).unwrap();
        assert!(restored.tracklist_compact);
    }

    #[test]
    fn older_settings_leave_middle_click_autoscroll_off() {
        let settings: Settings = serde_json::from_str(r#"{"tracklist_compact":true}"#).unwrap();
        assert!(settings.tracklist_compact);
        assert!(!settings.middle_click_autoscroll);
    }

    #[test]
    fn middle_click_autoscroll_round_trips() {
        let settings = Settings {
            middle_click_autoscroll: true,
            ..Settings::default()
        };
        let json = serde_json::to_string(&settings).unwrap();
        let restored: Settings = serde_json::from_str(&json).unwrap();
        assert!(restored.middle_click_autoscroll);
    }

    /// Saved equalizer presets read back as written; files from before them
    /// have none, and a preset without a preamp keeps it at zero.
    #[test]
    fn saved_equalizer_presets_round_trip_and_default_to_none() {
        let settings: Settings = serde_json::from_str(r#"{"zoom": 1.2}"#).unwrap();
        assert!(settings.eq_user_presets.is_empty());
        let settings: Settings = serde_json::from_str(
            r#"{"eq_user_presets":[{"name":"AirPods","preamp_db":-3.0,"bands_db":[1.2,0,-1.9,-2,0,1.6,1.6,2.6,0,-4.4]},{"name":"Old","bands_db":[0,0,0,0,0,0,0,0,0,1]}]}"#,
        )
        .unwrap();
        assert_eq!(settings.eq_user_presets[0].preamp_db, -3.0);
        assert_eq!(settings.eq_user_presets[1].preamp_db, 0.0);
        let again: Settings =
            serde_json::from_str(&serde_json::to_string(&settings).unwrap()).unwrap();
        assert_eq!(again.eq_user_presets, settings.eq_user_presets);
    }
}

/// Restorable UI session: what was open when the app last closed.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct SessionState {
    pub last_page: Option<String>,
    /// Context URIs most recently played, newest first.
    pub recent_contexts: Vec<String>,
    /// What was playing when the app closed, to resume from a cold start.
    pub last_context: Option<String>,
    pub last_track: Option<String>,
    /// The context row `last_track` was playing over, when it was a song
    /// queued by hand: the place the album keeps under it (rule 3), so a
    /// resumed session puts the album back where it was rather than at the
    /// queued song's place in it.
    pub last_context_track: Option<String>,
    pub last_position_ms: u32,
    /// Manually queued songs to restore with the remembered track.
    ///
    /// Context rows are excluded to prevent duplicates. This replaced the old
    /// `last_queue` field, so sessions using that field restore no added rows.
    pub last_added_queue: Vec<String>,
    /// Queue rows displayed on the next start. Playback restores manual rows
    /// from `last_added_queue`; it does not enqueue this list.
    pub last_queue_rows: Vec<crate::api::models::PlayableItem>,
    /// Shuffle mode saved across contexts and restarts.
    pub shuffle_on: bool,
    /// Repeat mode ("off", "context" or "track") saved across restarts.
    /// Sessions saved before it existed read as off.
    pub repeat: String,
    /// Each table's chosen sort, by encoded page, restored at start.
    pub sorts: Vec<(String, crate::model::TableSort)>,
    /// Last window inner size, to restore on next launch.
    pub window_size: Option<[f32; 2]>,
    /// Last window outer position, to restore on next launch.
    pub window_pos: Option<[f32; 2]>,
    /// Whether the queue panel was open.
    pub queue_open: Option<bool>,
    /// Which tab the queue panel showed: `queue` or `recents`.
    pub queue_tab: Option<String>,
    /// Last outer position of the Winamp window.
    pub winamp_pos: Option<[f32; 2]>,
    /// Last outer position of the MilkDrop window.
    pub milkdrop_pos: Option<[f32; 2]>,
    /// The window mode fullscreen lyrics left, when the app closed while
    /// showing them. eframe restores the window full screen, so the next
    /// start returns it to this mode instead.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lyrics_fullscreen_from: Option<WindowMode>,
}

/// Whether a window was full screen, and whether it was maximized.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct WindowMode {
    pub fullscreen: bool,
    pub maximized: bool,
}

impl SessionState {
    pub fn load(path: &Path) -> Self {
        std::fs::read_to_string(path)
            .ok()
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or_default()
    }

    /// The session as its file holds it.
    pub fn encode(&self) -> Option<String> {
        serde_json::to_string(self)
            .map_err(|error| log::warn!("unable to encode session: {error}"))
            .ok()
    }

    /// Writes the session and waits for the disk. The app saves through
    /// its [`crate::file_writer::FileWriter`] instead.
    pub fn save(&self, path: &Path) {
        let Some(text) = self.encode() else { return };
        if let Err(error) = crate::util::write_file(path, text) {
            log::warn!("unable to save session to {}: {error}", path.display());
        }
    }
}

#[cfg(test)]
mod session_tests {
    use super::SessionState;

    #[test]
    fn a_new_session_atomically_replaces_the_previous_one() {
        let root = std::env::temp_dir().join(format!(
            "fastsonic-session-test-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let path = root.join("session.json");
        let state = |page: &str| SessionState {
            last_page: Some(page.into()),
            ..SessionState::default()
        };

        state("home").save(&path);
        state("liked").save(&path);

        assert_eq!(
            SessionState::load(&path).last_page.as_deref(),
            Some("liked")
        );
        assert!(crate::util::temporary_files(&root).is_empty());
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn a_session_saved_before_repeat_reads_as_off() {
        let state: SessionState =
            serde_json::from_str(r#"{"last_page":"home","shuffle_on":true}"#).unwrap();
        assert_eq!(
            crate::engine::RepeatMode::from_api(&state.repeat),
            crate::engine::RepeatMode::Off
        );
    }

    #[test]
    fn the_repeat_mode_round_trips() {
        let state = SessionState {
            repeat: crate::engine::RepeatMode::Track.api_name().into(),
            ..SessionState::default()
        };
        let json = serde_json::to_string(&state).unwrap();
        let read: SessionState = serde_json::from_str(&json).unwrap();
        assert_eq!(
            crate::engine::RepeatMode::from_api(&read.repeat),
            crate::engine::RepeatMode::Track
        );
    }
}
