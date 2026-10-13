//! The Now Playing card under a MacBook's notch: what it shows, where it
//! goes, when it opens, and what a click on it asks for.
//!
//! Everything here is plain data and decisions, so it is tested on every
//! platform. The AppKit panel that draws it is `mac_notch.rs`; elsewhere
//! [`sync`] and [`shutdown`] do nothing.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crate::engine::RepeatMode;
use crate::model::Action;

/// What a click on the card asks the application to do.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NotchCommand {
    PlayPause,
    Next,
    Previous,
    ShowWindow,
    ToggleSaved(String),
    Seek(u32),
    ToggleShuffle,
    CycleRepeat,
}

impl NotchCommand {
    pub fn action(self) -> Action {
        match self {
            Self::PlayPause => Action::TogglePlay,
            Self::Next => Action::Next,
            Self::Previous => Action::Previous,
            Self::ShowWindow => Action::ShowWindow,
            Self::ToggleSaved(uri) => Action::ToggleSaved(uri),
            Self::Seek(position_ms) => Action::Seek(position_ms),
            Self::ToggleShuffle => Action::ToggleShuffle,
            Self::CycleRepeat => Action::CycleRepeat,
        }
    }
}

/// The way from the card's controls to the application, which reads it
/// every pass, with a window or from the tray without one.
#[derive(Clone)]
pub struct NotchLink {
    commands: Arc<Mutex<Vec<NotchCommand>>>,
    wake: Arc<dyn Fn() + Send + Sync>,
}

impl NotchLink {
    /// `wake` asks for the next pass, so a click is not left waiting for one.
    pub fn new(wake: impl Fn() + Send + Sync + 'static) -> Self {
        Self {
            commands: Arc::default(),
            wake: Arc::new(wake),
        }
    }

    pub fn push(&self, command: NotchCommand) {
        self.commands
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .push(command);
        (self.wake)();
    }

    pub fn drain(&self) -> Vec<NotchCommand> {
        std::mem::take(&mut *self.commands.lock().unwrap_or_else(|p| p.into_inner()))
    }
}

/// What the card shows, as the application sees it this pass.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct NotchTrack {
    pub uri: String,
    pub title: String,
    pub artist: String,
    pub playing: bool,
    pub duration_ms: u32,
    pub position_ms: u32,
    /// The cover, once the art cache has it on disk.
    pub art_path: Option<PathBuf>,
    pub saved: bool,
    pub shuffle: bool,
    pub repeat: RepeatMode,
    /// The colour of a control that is on: the heart, shuffle and repeat.
    pub accent: [u8; 3],
}

/// Another song, as opposed to the same one moving on.
pub fn is_another_track(shown: Option<&NotchTrack>, next: Option<&NotchTrack>) -> bool {
    match (shown, next) {
        (Some(shown), Some(next)) => {
            shown.uri != next.uri || shown.title != next.title || shown.artist != next.artist
        }
        (None, None) => false,
        _ => true,
    }
}

/// A song replacing another on an open card fades in; anything else, the
/// first song included, is simply shown.
pub fn crossfades(expanded: bool, shown: Option<&NotchTrack>, next: Option<&NotchTrack>) -> bool {
    expanded && shown.is_some() && next.is_some() && is_another_track(shown, next)
}

/// Which parts of the card a new pass changes, so the rest is left alone.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Changes {
    /// The clock moved by a whole second, the only step the card shows.
    pub time: bool,
    pub playing: bool,
    pub saved: bool,
    pub art: bool,
    pub accent: bool,
    pub shuffle: bool,
    pub repeat: bool,
}

pub fn changes(shown: &NotchTrack, next: &NotchTrack) -> Changes {
    Changes {
        time: shown.position_ms / 1000 != next.position_ms / 1000
            || shown.duration_ms / 1000 != next.duration_ms / 1000,
        playing: shown.playing != next.playing,
        saved: shown.saved != next.saved,
        art: shown.art_path != next.art_path,
        accent: shown.accent != next.accent,
        shuffle: shown.shuffle != next.shuffle,
        repeat: shown.repeat != next.repeat,
    }
}

/// Whether Fastsonic is what the person is looking at, which is when the
/// card stays out of the way.
///
/// `window_background` is the application's own view of its window: closed
/// to the tray, minimized, or not focused. AppKit adds whether the app is
/// active and whether any of its windows, the card's aside, is on screen.
pub fn in_foreground(app_active: bool, window_on_screen: bool, window_background: bool) -> bool {
    app_active && window_on_screen && !window_background
}

/// The card is on screen only when it is wanted, has something to show,
/// Fastsonic is in the background, and a display has a notch to hang it
/// from: a MacBook closed on an external monitor has none.
pub fn should_show(enabled: bool, foreground: bool, has_track: bool, has_notch: bool) -> bool {
    enabled && !foreground && has_track && has_notch
}

/// The pointer has to rest on the notch this long before the card opens,
/// so a pointer passing on its way to the menu bar does not open it.
pub const EXPAND_DWELL: Duration = Duration::from_millis(60);
/// The card stays this long after the pointer leaves, so leaving it by a
/// pixel on the way to a button does not close it.
pub const COLLAPSE_LINGER: Duration = Duration::from_millis(400);
pub const EXPAND_ANIMATION: Duration = Duration::from_millis(260);
pub const COLLAPSE_ANIMATION: Duration = Duration::from_millis(200);

/// A timer the hover is waiting on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Wait {
    Expand,
    Collapse,
    /// The card has faded out; the panel shrinks back to the notch.
    Shrink,
}

impl Wait {
    pub fn delay(self) -> Duration {
        match self {
            Self::Expand => EXPAND_DWELL,
            Self::Collapse => COLLAPSE_LINGER,
            Self::Shrink => COLLAPSE_ANIMATION,
        }
    }
}

/// What the panel has to do next.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    Nothing,
    /// Start a timer and come back with [`Hover::due`].
    Start(Wait),
    /// Grow the panel and fade the card in.
    Expand,
    /// Fade the card out, then come back for [`Wait::Shrink`].
    Collapse,
    /// Shrink the panel to the notch.
    Shrink,
}

/// Whether the card is open, and what it is waiting for.
///
/// A timer that fires for anything but the wait in progress is out of
/// date, from a hover since abandoned, and does nothing.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Hover {
    expanded: bool,
    waiting: Option<Wait>,
}

impl Hover {
    pub fn expanded(&self) -> bool {
        self.expanded
    }

    pub fn entered(&mut self, can_expand: bool) -> Step {
        if self.expanded {
            // Back before it closed: it stays open.
            self.waiting = None;
            return Step::Nothing;
        }
        if !can_expand {
            return Step::Nothing;
        }
        self.waiting = Some(Wait::Expand);
        Step::Start(Wait::Expand)
    }

    pub fn exited(&mut self) -> Step {
        if self.expanded {
            self.waiting = Some(Wait::Collapse);
            return Step::Start(Wait::Collapse);
        }
        if self.waiting == Some(Wait::Expand) {
            self.waiting = None;
        }
        Step::Nothing
    }

    pub fn due(&mut self, wait: Wait, can_expand: bool) -> Step {
        if self.waiting != Some(wait) {
            return Step::Nothing;
        }
        self.waiting = None;
        match wait {
            Wait::Expand if can_expand && !self.expanded => {
                self.expanded = true;
                Step::Expand
            }
            Wait::Collapse if self.expanded => {
                self.expanded = false;
                self.waiting = Some(Wait::Shrink);
                Step::Collapse
            }
            Wait::Shrink if !self.expanded => Step::Shrink,
            _ => Step::Nothing,
        }
    }

    /// The card went off screen: it comes back closed, waiting for nothing.
    pub fn reset(&mut self) {
        *self = Self::default();
    }
}

/// A rectangle in points. In screen coordinates the origin is the bottom
/// left, as AppKit has it; inside the card it is the top left, because the
/// card's views are flipped.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Frame {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

impl Frame {
    pub const fn new(x: f64, y: f64, w: f64, h: f64) -> Self {
        Self { x, y, w, h }
    }

    pub fn right(&self) -> f64 {
        self.x + self.w
    }

    pub fn bottom(&self) -> f64 {
        self.y + self.h
    }

    #[cfg(test)]
    fn overlaps(&self, other: &Frame) -> bool {
        self.x < other.right()
            && other.x < self.right()
            && self.y < other.bottom()
            && other.y < self.bottom()
    }

    fn differs(&self, other: &Frame) -> bool {
        (self.x - other.x).abs() > 0.5
            || (self.y - other.y).abs() > 0.5
            || (self.w - other.w).abs() > 0.5
            || (self.h - other.h).abs() > 0.5
    }
}

pub const CARD_WIDTH: f64 = 400.0;
pub const CARD_HEIGHT: f64 = 148.0;
pub const CARD_RADIUS: f64 = 18.0;
/// Between the bottom of the notch and the top of the card.
const CARD_GAP: f64 = 8.0;
/// The resting panel, which only catches the pointer at the notch.
const NOTCH_WIDTH: f64 = 200.0;

/// Where the panel sits on screen, resting and open, and where its card
/// sits inside it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Geometry {
    pub collapsed: Frame,
    pub expanded: Frame,
    /// The card inside the panel, before it slides down into view.
    pub card_collapsed: Frame,
    pub card_expanded: Frame,
}

impl Geometry {
    /// For a screen whose frame is given in AppKit's coordinates and whose
    /// safe area starts `top_inset` points down. `None` for a screen with no
    /// notch, or numbers no screen has.
    pub fn new(x: f64, y: f64, width: f64, height: f64, top_inset: f64) -> Option<Self> {
        let finite = [x, y, width, height, top_inset]
            .iter()
            .all(|value| value.is_finite());
        if !finite || top_inset <= 0.0 || width <= 0.0 || height <= 0.0 {
            return None;
        }
        let center = x + width / 2.0;
        let top = y + height;
        let notch_w = NOTCH_WIDTH.min(width);
        let card_w = CARD_WIDTH.min(width);
        let open_h = top_inset + CARD_GAP + CARD_HEIGHT;
        Some(Self {
            collapsed: Frame::new(center - notch_w / 2.0, top - top_inset, notch_w, top_inset),
            expanded: Frame::new(center - card_w / 2.0, top - open_h, card_w, open_h),
            card_collapsed: Frame::new(0.0, 6.0, card_w, CARD_HEIGHT),
            card_expanded: Frame::new(0.0, top_inset + CARD_GAP, card_w, CARD_HEIGHT),
        })
    }

    /// Whether moving to `other` moves anything, so a pass that finds the
    /// same screen leaves the panel alone.
    pub fn differs(&self, other: &Geometry) -> bool {
        self.collapsed.differs(&other.collapsed) || self.expanded.differs(&other.expanded)
    }
}

const BUTTON: f64 = 30.0;
const PLAY_BUTTON: f64 = 36.0;
const BUTTON_ROW: f64 = 97.0;
const BUTTON_HEIGHT: f64 = 36.0;
const EDGE: f64 = 16.0;

/// Where everything on the card goes, in the card's flipped coordinates.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CardLayout {
    pub art: Frame,
    pub title: Frame,
    pub artist: Frame,
    pub elapsed: Frame,
    pub duration: Frame,
    pub seek: SeekBar,
    pub like: Frame,
    pub shuffle: Frame,
    pub previous: Frame,
    /// The light disc behind Play and Pause, and the button on it.
    pub play: Frame,
    pub next: Frame,
    pub repeat: Frame,
}

impl CardLayout {
    pub fn new(card_w: f64) -> Self {
        let text_x = 76.0;
        let text_w = (card_w - text_x - EDGE).max(60.0);
        let like = Frame::new(EDGE, BUTTON_ROW, BUTTON, BUTTON_HEIGHT);
        // The transport sits in the middle, unless the card is so narrow
        // that it would run into the heart.
        let half_group = 108.0;
        let center = (card_w / 2.0).max(like.right() + 8.0 + half_group);
        let button = |x: f64| Frame::new(x, BUTTON_ROW, BUTTON, BUTTON_HEIGHT);
        Self {
            art: Frame::new(EDGE, 14.0, 50.0, 50.0),
            title: Frame::new(text_x, 16.0, text_w, 20.0),
            artist: Frame::new(text_x, 38.0, text_w, 18.0),
            elapsed: Frame::new(14.0, 74.0, 42.0, 16.0),
            duration: Frame::new((card_w - 58.0).max(120.0), 74.0, 44.0, 16.0),
            seek: SeekBar::new(card_w),
            like,
            shuffle: button(center - half_group),
            previous: button(center - 62.0),
            play: Frame::new(
                center - PLAY_BUTTON / 2.0,
                BUTTON_ROW,
                PLAY_BUTTON,
                PLAY_BUTTON,
            ),
            next: button(center + 32.0),
            repeat: button(center + 78.0),
        }
    }

    pub fn buttons(&self) -> [Frame; 6] {
        [
            self.like,
            self.shuffle,
            self.previous,
            self.play,
            self.next,
            self.repeat,
        ]
    }
}

/// The thin seek bar between the two clocks.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SeekBar {
    pub start: f64,
    pub end: f64,
    pub center_y: f64,
}

impl SeekBar {
    fn new(card_w: f64) -> Self {
        let start = 60.0;
        Self {
            start,
            end: (card_w - 58.0).max(start + 20.0),
            center_y: 82.0,
        }
    }

    pub fn width(&self) -> f64 {
        (self.end - self.start).max(1.0)
    }

    /// Whether a press at `x, y` lands on the bar, give or take a little
    /// either side, rather than on the card around it.
    pub fn hit(&self, x: f64, y: f64) -> bool {
        (self.center_y - 12.0..=self.center_y + 12.0).contains(&y)
            && (self.start - 4.0..=self.end + 4.0).contains(&x)
    }

    /// The place in the song at `x`, held to the bar's two ends.
    pub fn position_at(&self, x: f64, duration_ms: u32) -> u32 {
        let fraction = ((x - self.start) / self.width()).clamp(0.0, 1.0);
        (fraction * f64::from(duration_ms)) as u32
    }

    /// How far along the bar `position_ms` is, from 0 to 1.
    pub fn fraction(position_ms: u32, duration_ms: u32) -> f64 {
        if duration_ms == 0 {
            return 0.0;
        }
        (f64::from(position_ms) / f64::from(duration_ms)).clamp(0.0, 1.0)
    }
}

/// A title or artist on one line, and the app's name for a song without
/// a title.
pub fn one_line(text: &str, fallback: &str) -> String {
    let text = text.replace(['\r', '\n'], " ");
    if text.trim().is_empty() {
        fallback.to_string()
    } else {
        text
    }
}

/// Keeps the card in step with what is playing. `window_background` is
/// whether Fastsonic's window is closed to the tray, minimized or not
/// focused. Turning the setting off takes the card down.
#[cfg(target_os = "macos")]
pub use crate::mac_notch::sync;

/// Takes the card down, if it is up. Called whenever the window around the
/// application is replaced or goes, and before the process ends.
#[cfg(target_os = "macos")]
pub use crate::mac_notch::shutdown;

#[cfg(not(target_os = "macos"))]
pub fn sync(
    _link: &NotchLink,
    _enabled: bool,
    _window_background: bool,
    _track: Option<&NotchTrack>,
) {
}

#[cfg(not(target_os = "macos"))]
pub fn shutdown() {}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn song() -> NotchTrack {
        NotchTrack {
            uri: "sonic:track:a".into(),
            title: "Song A".into(),
            artist: "Artist A".into(),
            playing: true,
            duration_ms: 200_000,
            position_ms: 10_100,
            art_path: None,
            saved: false,
            shuffle: false,
            repeat: RepeatMode::Off,
            accent: [30, 215, 96],
        }
    }

    #[test]
    fn a_click_is_queued_in_order_and_asks_for_a_pass() {
        let woken = Arc::new(AtomicUsize::new(0));
        let counter = Arc::clone(&woken);
        let link = NotchLink::new(move || {
            counter.fetch_add(1, Ordering::SeqCst);
        });
        link.push(NotchCommand::PlayPause);
        link.push(NotchCommand::Seek(45_000));
        assert_eq!(woken.load(Ordering::SeqCst), 2);
        assert_eq!(
            link.drain(),
            vec![NotchCommand::PlayPause, NotchCommand::Seek(45_000)]
        );
        assert!(link.drain().is_empty());
    }

    #[test]
    fn every_command_is_the_action_the_player_bar_would_send() {
        assert!(matches!(
            NotchCommand::PlayPause.action(),
            Action::TogglePlay
        ));
        assert!(matches!(NotchCommand::Next.action(), Action::Next));
        assert!(matches!(NotchCommand::Previous.action(), Action::Previous));
        assert!(matches!(
            NotchCommand::ShowWindow.action(),
            Action::ShowWindow
        ));
        assert!(matches!(
            NotchCommand::ToggleSaved("sonic:track:a".into()).action(),
            Action::ToggleSaved(uri) if uri == "sonic:track:a"
        ));
        assert!(matches!(
            NotchCommand::Seek(12_345).action(),
            Action::Seek(12_345)
        ));
        assert!(matches!(
            NotchCommand::ToggleShuffle.action(),
            Action::ToggleShuffle
        ));
        assert!(matches!(
            NotchCommand::CycleRepeat.action(),
            Action::CycleRepeat
        ));
    }

    #[test]
    fn the_card_shows_only_in_the_background_with_a_song_and_a_notch() {
        assert!(should_show(true, false, true, true));
        assert!(!should_show(false, false, true, true), "turned off");
        assert!(!should_show(true, true, true, true), "Fastsonic in front");
        assert!(!should_show(true, false, false, true), "nothing to show");
        assert!(!should_show(true, false, true, false), "no notched display");
    }

    #[test]
    fn fastsonic_is_in_front_only_when_active_with_its_window_up_and_focused() {
        assert!(in_foreground(true, true, false));
        assert!(!in_foreground(false, true, false), "another app is active");
        assert!(!in_foreground(true, false, false), "closed to the tray");
        assert!(!in_foreground(true, true, true), "minimized or unfocused");
    }

    #[test]
    fn resting_on_the_notch_opens_the_card_after_the_dwell() {
        let mut hover = Hover::default();
        assert_eq!(hover.entered(true), Step::Start(Wait::Expand));
        assert_eq!(hover.due(Wait::Expand, true), Step::Expand);
        assert!(hover.expanded());
        assert_eq!(Wait::Expand.delay(), EXPAND_DWELL);
    }

    #[test]
    fn passing_over_the_notch_does_not_open_the_card() {
        let mut hover = Hover::default();
        hover.entered(true);
        assert_eq!(hover.exited(), Step::Nothing);
        assert_eq!(hover.due(Wait::Expand, true), Step::Nothing);
        assert!(!hover.expanded());
    }

    #[test]
    fn the_card_does_not_open_when_it_could_not_show() {
        let mut hover = Hover::default();
        assert_eq!(hover.entered(false), Step::Nothing);
        hover.entered(true);
        // Fastsonic came to the front during the dwell.
        assert_eq!(hover.due(Wait::Expand, false), Step::Nothing);
        assert!(!hover.expanded());
    }

    #[test]
    fn leaving_closes_the_card_after_a_linger_and_then_shrinks_the_panel() {
        let mut hover = Hover::default();
        hover.entered(true);
        hover.due(Wait::Expand, true);
        assert_eq!(hover.exited(), Step::Start(Wait::Collapse));
        assert_eq!(hover.due(Wait::Collapse, true), Step::Collapse);
        assert!(!hover.expanded());
        assert_eq!(hover.due(Wait::Shrink, true), Step::Shrink);
        assert_eq!(hover.due(Wait::Shrink, true), Step::Nothing, "only once");
    }

    #[test]
    fn coming_back_during_the_linger_keeps_the_card_open() {
        let mut hover = Hover::default();
        hover.entered(true);
        hover.due(Wait::Expand, true);
        hover.exited();
        assert_eq!(hover.entered(true), Step::Nothing);
        assert_eq!(hover.due(Wait::Collapse, true), Step::Nothing, "stale");
        assert!(hover.expanded());
    }

    #[test]
    fn coming_back_while_it_fades_out_opens_it_again_without_shrinking() {
        let mut hover = Hover::default();
        hover.entered(true);
        hover.due(Wait::Expand, true);
        hover.exited();
        hover.due(Wait::Collapse, true);
        assert_eq!(hover.entered(true), Step::Start(Wait::Expand));
        assert_eq!(hover.due(Wait::Shrink, true), Step::Nothing, "stale");
        assert_eq!(hover.due(Wait::Expand, true), Step::Expand);
    }

    #[test]
    fn a_reset_card_ignores_the_timers_it_left_behind() {
        let mut hover = Hover::default();
        hover.entered(true);
        hover.due(Wait::Expand, true);
        hover.exited();
        hover.reset();
        assert_eq!(hover, Hover::default());
        assert_eq!(hover.due(Wait::Collapse, true), Step::Nothing);
    }

    #[test]
    fn only_another_song_counts_as_a_new_track() {
        let a = song();
        let mut moved = a.clone();
        moved.position_ms = 90_000;
        moved.playing = false;
        moved.saved = true;
        moved.art_path = Some("/tmp/cover.jpg".into());
        assert!(!is_another_track(Some(&a), Some(&moved)));
        assert!(!crossfades(true, Some(&a), Some(&moved)));

        let mut b = a.clone();
        b.title = "Song B".into();
        assert!(is_another_track(Some(&a), Some(&b)));
        assert!(crossfades(true, Some(&a), Some(&b)));
        assert!(!crossfades(false, Some(&a), Some(&b)), "closed card");

        assert!(!is_another_track(None, None));
        assert!(is_another_track(None, Some(&a)));
        assert!(!crossfades(true, None, Some(&a)), "the first song");
        assert!(is_another_track(Some(&a), None));
        assert!(!crossfades(true, Some(&a), None));
    }

    #[test]
    fn the_clock_changes_only_by_whole_seconds() {
        let a = song();
        let mut b = a.clone();
        b.position_ms = 10_900;
        assert_eq!(changes(&a, &b), Changes::default());
        b.position_ms = 11_050;
        assert!(changes(&a, &b).time);
        let mut b = a.clone();
        b.duration_ms = 205_000;
        assert!(changes(&a, &b).time);
    }

    #[test]
    fn each_control_reports_its_own_change() {
        let a = song();
        let changed = |edit: fn(&mut NotchTrack)| {
            let mut b = a.clone();
            edit(&mut b);
            changes(&a, &b)
        };
        assert!(changed(|b| b.playing = false).playing);
        assert!(changed(|b| b.saved = true).saved);
        assert!(changed(|b| b.art_path = Some("/tmp/a.jpg".into())).art);
        assert!(changed(|b| b.accent = [1, 2, 3]).accent);
        assert!(changed(|b| b.shuffle = true).shuffle);
        assert!(changed(|b| b.repeat = RepeatMode::Track).repeat);
        let only_playing = changed(|b| b.playing = false);
        assert_eq!(
            only_playing,
            Changes {
                playing: true,
                ..Changes::default()
            }
        );
    }

    #[test]
    fn the_panel_hangs_from_the_notch_and_opens_below_it() {
        // A 14-inch MacBook Pro: 1512 x 982 points, a 32-point notch.
        let geometry = Geometry::new(0.0, 0.0, 1512.0, 982.0, 32.0).expect("a notch");
        assert_eq!(
            geometry.collapsed,
            Frame::new(756.0 - 100.0, 982.0 - 32.0, 200.0, 32.0)
        );
        let open_h = 32.0 + 8.0 + CARD_HEIGHT;
        assert_eq!(
            geometry.expanded,
            Frame::new(756.0 - 200.0, 982.0 - open_h, 400.0, open_h)
        );
        assert_eq!(geometry.card_expanded.y, 40.0);
        assert_eq!(geometry.card_expanded.bottom(), open_h);
        // A display to the right of the main one keeps its own origin.
        let second = Geometry::new(1512.0, 0.0, 1728.0, 1117.0, 34.0).unwrap();
        assert_eq!(second.collapsed.x + 100.0, 1512.0 + 864.0);
    }

    #[test]
    fn no_notch_or_nonsense_numbers_give_no_panel() {
        assert_eq!(Geometry::new(0.0, 0.0, 1920.0, 1080.0, 0.0), None);
        assert_eq!(Geometry::new(0.0, 0.0, 1920.0, 1080.0, -10.0), None);
        assert!(Geometry::new(0.0, 0.0, 1512.0, 982.0, f64::NAN).is_none());
        assert!(Geometry::new(0.0, 0.0, 1512.0, 982.0, f64::INFINITY).is_none());
        assert!(Geometry::new(f64::NAN, 0.0, 1512.0, 982.0, 32.0).is_none());
        assert!(Geometry::new(0.0, 0.0, -100.0, 982.0, 32.0).is_none());
        assert!(Geometry::new(0.0, 0.0, 1512.0, 0.0, 32.0).is_none());
    }

    #[test]
    fn a_narrow_screen_narrows_the_card_to_fit() {
        let narrow = Geometry::new(0.0, 0.0, 300.0, 500.0, 24.0).unwrap();
        assert_eq!(narrow.expanded.w, 300.0);
        assert_eq!(narrow.card_expanded.w, 300.0);
        assert_eq!(narrow.collapsed.w, 200.0);
    }

    #[test]
    fn the_same_screen_moves_nothing() {
        let a = Geometry::new(0.0, 0.0, 1512.0, 982.0, 32.0).unwrap();
        let b = Geometry::new(0.2, 0.0, 1512.0, 982.0, 32.0).unwrap();
        assert!(!a.differs(&b));
        let c = Geometry::new(0.0, 0.0, 1728.0, 1117.0, 32.0).unwrap();
        assert!(a.differs(&c));
    }

    #[test]
    fn the_card_layout_fits_and_no_two_buttons_touch() {
        for card_w in [CARD_WIDTH, 300.0, 200.0] {
            let layout = CardLayout::new(card_w);
            let buttons = layout.buttons();
            for (i, a) in buttons.iter().enumerate() {
                assert!(a.y >= 0.0 && a.bottom() <= CARD_HEIGHT, "{card_w}: {a:?}");
                for b in &buttons[i + 1..] {
                    assert!(!a.overlaps(b), "{card_w}: {a:?} and {b:?}");
                }
            }
            assert!(layout.title.x >= layout.art.right());
            assert!(layout.seek.start > layout.elapsed.x);
            assert!(layout.seek.end <= layout.duration.x);
        }
        let layout = CardLayout::new(CARD_WIDTH);
        assert!(layout.repeat.right() <= CARD_WIDTH - EDGE);
        let middle = layout.play.x + layout.play.w / 2.0;
        assert_eq!(middle, CARD_WIDTH / 2.0, "the transport is centred");
    }

    #[test]
    fn the_seek_bar_maps_a_press_to_a_place_in_the_song() {
        let bar = CardLayout::new(CARD_WIDTH).seek;
        assert_eq!(bar.position_at(bar.start, 200_000), 0);
        assert_eq!(bar.position_at(bar.end, 200_000), 200_000);
        let middle = bar.start + bar.width() / 2.0;
        assert_eq!(bar.position_at(middle, 200_000), 100_000);
        assert_eq!(bar.position_at(bar.start - 50.0, 200_000), 0, "held");
        assert_eq!(bar.position_at(bar.end + 50.0, 200_000), 200_000);
        assert_eq!(bar.position_at(middle, 0), 0, "no length yet");

        assert!(bar.hit(middle, bar.center_y));
        assert!(bar.hit(bar.start - 3.0, bar.center_y + 10.0));
        assert!(!bar.hit(middle, bar.center_y + 20.0), "the buttons below");
        assert!(!bar.hit(bar.start - 10.0, bar.center_y), "the clock");
    }

    #[test]
    fn the_bar_fills_by_how_far_the_song_is() {
        assert_eq!(SeekBar::fraction(50_000, 200_000), 0.25);
        assert_eq!(SeekBar::fraction(250_000, 200_000), 1.0);
        assert_eq!(SeekBar::fraction(5_000, 0), 0.0);
    }

    #[test]
    fn titles_are_one_line_and_never_empty() {
        assert_eq!(one_line("Part 1\nPart 2", "Fastsonic"), "Part 1 Part 2");
        assert_eq!(one_line("  ", "Fastsonic"), "Fastsonic");
        assert_eq!(one_line("", ""), "");
    }
}
