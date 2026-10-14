//! The device's power and session state, from the operating system, and
//! what the app may spend because of it.
//!
//! A thin reader per platform writes what the system reports into shared
//! [`Conditions`] and queues [`Event`]s, then wakes the app. The app reads
//! both in its logic pass, which runs with or without a window, and turns
//! the conditions into a [`Budget`] for its drawing and polling.
//!
//! What each platform reports:
//!
//! - Linux: the power-saver profile (the desktop portal, or
//!   power-profiles-daemon), and sleep and wake from logind.
//! - Windows: Energy Saver or battery saver, the power source, the console
//!   display, sleep and wake, the network's reach and cost, the session lock
//!   and the end of the session.
//! - macOS: Low Power Mode, and whether the internet can be reached.
//!
//! What a platform does not report keeps the default, which is the
//! unconstrained case, so nothing is ever held back because a platform
//! says nothing.

use std::sync::{Arc, Condvar, Mutex, PoisonError};
#[cfg(any(target_os = "windows", target_os = "linux", test))]
use std::time::Duration;

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
mod map;
#[cfg(target_os = "windows")]
mod windows;

#[cfg(target_os = "linux")]
use self::linux::Reader;
#[cfg(target_os = "macos")]
use self::macos::Reader;
#[cfg(target_os = "windows")]
use self::windows::Reader;

/// Joins the Multimedia Class Scheduler as an "Audio" task, once per thread.
/// The thread that feeds the audio device calls it, so playback keeps its
/// priority while the rest of the process runs under EcoQoS.
#[cfg(target_os = "windows")]
pub use self::windows::join_audio_task;

/// What the system says about power, the display, the session and the
/// network. A platform that cannot tell leaves the default, which is the
/// unconstrained case.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Conditions {
    /// The user asked the system to save energy: Energy Saver or battery
    /// saver on Windows, Low Power Mode on macOS, the power-saver profile on
    /// Linux.
    pub saver: bool,
    /// Running on battery (Windows).
    pub on_battery: bool,
    /// Every display is off (Windows).
    pub display_off: bool,
    /// The session is locked, or switched away from or disconnected
    /// (Windows).
    pub locked: bool,
    /// The system reports a way to the internet (Windows and macOS).
    ///
    /// Windows's connectivity hint can say there is no internet behind a
    /// proxy or a captive network that the app reaches anyway, so it is a
    /// reason to wait, never to give up, and the app follows it on macOS
    /// only.
    pub online: bool,
    /// The connection is metered or charged by use, roaming, or over its
    /// data limit (Windows).
    pub metered: bool,
}

impl Default for Conditions {
    fn default() -> Self {
        Self {
            saver: false,
            on_battery: false,
            display_off: false,
            locked: false,
            online: true,
            metered: false,
        }
    }
}

/// Something the system is about to do, or has just done.
#[derive(Debug)]
pub enum Event {
    /// The system is about to sleep. Save, then [`Ack::done`].
    Suspending(Ack),
    /// The system woke up. Reported on Linux and Windows.
    Resumed,
    /// The user is signing out or shutting down. Save, then [`Ack::done`].
    /// Reported on Windows.
    EndingSession(Ack),
}

/// Lets a reader hold the system back, for a bounded time, until the app
/// has handled an event the system will not wait long for.
#[derive(Clone, Default)]
pub struct Ack(Arc<(Mutex<bool>, Condvar)>);

/// Events are logged; the lock and the condition variable inside say
/// nothing worth reading there.
impl std::fmt::Debug for Ack {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Ack")
    }
}

impl Ack {
    /// The app has done what the event asked.
    pub fn done(&self) {
        let (done, ring) = &*self.0;
        *done.lock().unwrap_or_else(PoisonError::into_inner) = true;
        ring.notify_all();
    }

    /// Whether the app has answered, for tests elsewhere.
    #[cfg(test)]
    pub fn answered(&self) -> bool {
        self.wait(Duration::ZERO)
    }

    /// Waits until [`done`](Self::done) or `timeout`, whichever is first.
    /// Returns whether the app answered in time.
    #[cfg(any(target_os = "windows", target_os = "linux", test))]
    fn wait(&self, timeout: Duration) -> bool {
        let (done, ring) = &*self.0;
        let done = done.lock().unwrap_or_else(PoisonError::into_inner);
        let (done, _) = ring
            .wait_timeout_while(done, timeout, |done| !*done)
            .unwrap_or_else(PoisonError::into_inner);
        *done
    }
}

/// How much the app may spend on drawing and polling.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Budget {
    /// As designed.
    #[default]
    Normal,
    /// The user asked to save energy, or the window is not in focus:
    /// animations run at half rate.
    Saver,
    /// Nobody can see the window (none shown, the display off or the session
    /// locked): nothing is drawn.
    Background,
}

/// The budget for `conditions`, given whether a window is shown and focused.
pub fn budget(conditions: Conditions, window_shown: bool, focused: bool) -> Budget {
    if !window_shown || conditions.display_off || conditions.locked {
        Budget::Background
    } else if conditions.saver || !focused {
        Budget::Saver
    } else {
        Budget::Normal
    }
}

/// MilkDrop's frame rate within `conditions`, from the user's `fps` (0 is
/// unlimited): at most 30 while saving energy, and one a second while
/// nobody can see any window (the display off or the session locked).
/// MilkDrop has a window of its own, so the main window's state does not
/// count; its own window being covered or minimised is MilkDrop's to notice.
pub fn milkdrop_fps(conditions: Conditions, fps: u32) -> u32 {
    if conditions.display_off || conditions.locked {
        1
    } else if conditions.saver && (fps == 0 || fps > 30) {
        30
    } else {
        fps
    }
}

/// The frame rate of the main window's visualizers (the player bar's and
/// the mini player's) within `budget`, from the user's `fps` (0 is uncapped,
/// the display's own rate): as chosen in focus, half of it out of focus or
/// while saving energy, and at most 30 while saving energy. `None` while
/// nobody can see the window: no frames. Uncapped halves to 30, the half
/// rate these visualizers always had.
pub fn visualiser_fps(budget: Budget, conditions: Conditions, fps: u32) -> Option<u32> {
    match budget {
        Budget::Background => None,
        Budget::Normal => Some(fps),
        Budget::Saver => {
            let half = if fps == 0 { 30 } else { (fps / 2).max(1) };
            Some(if conditions.saver { half.min(30) } else { half })
        }
    }
}

/// How long a reader waits for the app to save before the system sleeps.
/// Windows gives a suspend callback about two seconds.
#[cfg(any(target_os = "windows", target_os = "linux"))]
const SUSPEND_ACK: Duration = Duration::from_millis(1500);
/// How long a reader holds back the end of the session for the app to save.
/// Windows waits about five seconds before it lists the app as blocking.
#[cfg(target_os = "windows")]
const END_SESSION_ACK: Duration = Duration::from_secs(3);

/// What the readers write and the app reads.
struct Shared {
    conditions: Mutex<Conditions>,
    events: Mutex<Vec<Event>>,
    wake: Box<dyn Fn() + Send + Sync>,
}

impl Shared {
    fn new(wake: impl Fn() + Send + Sync + 'static) -> Self {
        Self {
            conditions: Mutex::new(Conditions::default()),
            events: Mutex::new(Vec::new()),
            wake: Box::new(wake),
        }
    }

    /// Applies `change` and wakes the app if it changed anything.
    fn update(&self, change: impl FnOnce(&mut Conditions)) {
        let mut conditions = self
            .conditions
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let before = *conditions;
        change(&mut conditions);
        let after = *conditions;
        drop(conditions);
        if after != before {
            log::info!("power and session: {after:?}");
            (self.wake)();
        }
    }

    /// Queues `event` and wakes the app.
    fn push(&self, event: Event) {
        log::info!("power and session: {event:?}");
        self.events
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(event);
        (self.wake)();
    }

    /// Queues the event `make` builds and waits up to `timeout` for the app
    /// to answer it.
    #[cfg(any(target_os = "windows", target_os = "linux", test))]
    fn push_and_wait(&self, make: impl FnOnce(Ack) -> Event, timeout: Duration) {
        let ack = Ack::default();
        self.push(make(ack.clone()));
        if !ack.wait(timeout) {
            log::warn!("power and session: the app did not answer within {timeout:?}");
        }
    }
}

/// Watches the system's power and session state for the app.
///
/// Readers run on threads of their own (Linux, the Windows session window)
/// or on the system's own threads (Windows callbacks, macOS's main dispatch
/// queue), so a slow or absent service never holds up the app.
pub struct Power {
    shared: Arc<Shared>,
    #[cfg(any(target_os = "windows", target_os = "linux", target_os = "macos"))]
    _reader: Option<Reader>,
    #[cfg(target_os = "windows")]
    eco_qos: windows::EcoQos,
}

impl Power {
    /// Starts the platform's readers. `wake` runs on a reader's thread
    /// whenever the conditions change or an event is queued; it asks for
    /// the app's next logic pass and returns at once.
    pub fn spawn(wake: impl Fn() + Send + Sync + 'static) -> Self {
        let shared = Arc::new(Shared::new(wake));
        Self {
            #[cfg(any(target_os = "windows", target_os = "linux", target_os = "macos"))]
            _reader: Reader::start(Arc::clone(&shared))
                .inspect_err(|error| log::warn!("power and session state unavailable: {error}"))
                .ok(),
            #[cfg(target_os = "windows")]
            eco_qos: windows::EcoQos::default(),
            shared,
        }
    }

    /// Conditions that change only through
    /// [`set_conditions`](Self::set_conditions), with no reader: for tests
    /// and demo runs, which must not see the device's own state (Low Power
    /// Mode on a Mac).
    pub fn fixed(conditions: Conditions) -> Self {
        let shared = Arc::new(Shared::new(|| {}));
        *shared
            .conditions
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = conditions;
        Self {
            shared,
            #[cfg(any(target_os = "windows", target_os = "linux", target_os = "macos"))]
            _reader: None,
            #[cfg(target_os = "windows")]
            eco_qos: windows::EcoQos::default(),
        }
    }

    /// The latest conditions.
    ///
    /// On macOS, Low Power Mode is read here, one message to NSProcessInfo,
    /// rather than followed through a notification, so a change shows up on
    /// the next logic pass.
    pub fn conditions(&self) -> Conditions {
        #[cfg(target_os = "macos")]
        if self._reader.is_some() {
            macos::read_low_power(&self.shared);
        }
        *self
            .shared
            .conditions
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }

    /// The events since the last call, oldest first. Called in every logic
    /// pass, window or not: a reader holds the system back until the app
    /// answers an [`Ack`], or for a short while at most.
    pub fn events(&self) -> Vec<Event> {
        std::mem::take(
            &mut *self
                .shared
                .events
                .lock()
                .unwrap_or_else(PoisonError::into_inner),
        )
    }

    /// Tells the system whether the app runs where nobody can see it, so
    /// the work it still does can run on efficient cores (EcoQoS on
    /// Windows). Only a watching `Power` touches the process.
    pub fn set_background(&mut self, background: bool) {
        #[cfg(target_os = "windows")]
        if self._reader.is_some() {
            self.eco_qos.set(background);
        }
        #[cfg(not(target_os = "windows"))]
        let _ = background;
    }

    /// Replaces the conditions and wakes the app if they changed, as a
    /// reader would: for tests and demo runs on a [`fixed`](Self::fixed)
    /// `Power`.
    pub fn set_conditions(&self, conditions: Conditions) {
        self.shared.update(|current| *current = conditions);
    }

    /// Queues `event` as a reader would, for tests. Nothing waits for its
    /// [`Ack`].
    pub fn send(&self, event: Event) {
        self.shared.push(event);
    }
}

/// Keeps the app's work out of App Nap on macOS while it is held, and ends
/// when dropped.
///
/// macOS slows down an app whose windows are covered, hidden or on another
/// space while another app is in front: its timers fire late and its
/// threads get less time. Playback holds one while it plays or loads, and
/// drops it on pause, stop or an output error, so a paused app naps again.
/// It asks for a user-initiated activity that still lets the system sleep
/// when idle and the display turn off: it does not keep the Mac awake.
/// Linux and Windows have no App Nap, and there it does nothing.
/// <https://developer.apple.com/documentation/foundation/processinfo/activityoptions/userinitiatedallowingidlesystemsleep>
#[must_use = "the activity ends when it is dropped"]
pub struct Activity {
    #[cfg(target_os = "macos")]
    _token: macos::Activity,
}

impl Activity {
    /// Begins an activity. `reason` is what the system shows for it, in
    /// `pmset -g assertions` say.
    pub fn begin(reason: &str) -> Self {
        #[cfg(target_os = "macos")]
        {
            Self {
                _token: macos::Activity::begin(reason),
            }
        }
        #[cfg(not(target_os = "macos"))]
        {
            let _ = reason;
            Self {}
        }
    }
}

impl std::fmt::Debug for Activity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Activity")
    }
}

/// Whether local playback is doing work the user is waiting for, which
/// macOS must not slow down with App Nap while another app is in front:
/// playing or loading on an engine with a server. Pause, stop, an output
/// error and signing out all let the app nap again.
pub fn playback_needs_activity(local: &crate::engine::LocalState) -> bool {
    use crate::engine::Playback;
    local.connected
        && local.error.is_none()
        && matches!(local.playback, Playback::Playing | Playback::Loading)
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicU32, Ordering};

    use super::*;

    /// A platform that cannot tell holds nothing back.
    #[test]
    fn the_default_is_unconstrained() {
        let conditions = Conditions::default();
        assert!(conditions.online);
        assert!(!conditions.metered);
        assert!(!conditions.saver);
        assert!(!conditions.on_battery);
        assert!(!conditions.display_off);
        assert!(!conditions.locked);
    }

    #[test]
    fn nothing_is_drawn_when_nobody_can_see_it() {
        let normal = Conditions::default();
        assert_eq!(budget(normal, true, true), Budget::Normal);
        assert_eq!(budget(normal, false, true), Budget::Background, "no window");
        for conditions in [
            Conditions {
                display_off: true,
                ..normal
            },
            Conditions {
                locked: true,
                ..normal
            },
        ] {
            assert_eq!(budget(conditions, true, true), Budget::Background);
        }
    }

    #[test]
    fn saving_energy_or_losing_focus_halves_the_animations() {
        let normal = Conditions::default();
        let saving = Conditions {
            saver: true,
            ..normal
        };
        assert_eq!(budget(saving, true, true), Budget::Saver);
        assert_eq!(budget(normal, true, false), Budget::Saver);
        assert_eq!(
            budget(
                Conditions {
                    on_battery: true,
                    ..normal
                },
                true,
                true
            ),
            Budget::Normal,
            "battery alone is not a request to save"
        );
    }

    #[test]
    fn milkdrop_slows_down_with_the_system_not_the_main_window() {
        let normal = Conditions::default();
        assert_eq!(milkdrop_fps(normal, 60), 60);
        assert_eq!(milkdrop_fps(normal, 0), 0, "unlimited stays unlimited");
        let saving = Conditions {
            saver: true,
            ..normal
        };
        assert_eq!(milkdrop_fps(saving, 60), 30);
        assert_eq!(milkdrop_fps(saving, 0), 30);
        assert_eq!(milkdrop_fps(saving, 24), 24);
        for dark in [
            Conditions {
                display_off: true,
                ..normal
            },
            Conditions {
                locked: true,
                ..saving
            },
        ] {
            assert_eq!(milkdrop_fps(dark, 60), 1);
        }
    }

    /// The player bar and mini player visualizers keep the rate chosen in
    /// focus, halve it out of focus, draw at most 30 saving energy, and
    /// none while nobody can see them.
    #[test]
    fn the_visualizers_halve_their_rate_out_of_focus_and_stop_unseen() {
        let normal = Conditions::default();
        let saving = Conditions {
            saver: true,
            ..normal
        };
        assert_eq!(visualiser_fps(Budget::Normal, normal, 60), Some(60));
        assert_eq!(visualiser_fps(Budget::Normal, normal, 0), Some(0));
        assert_eq!(
            visualiser_fps(Budget::Saver, normal, 144),
            Some(72),
            "out of focus"
        );
        assert_eq!(visualiser_fps(Budget::Saver, normal, 0), Some(30));
        assert_eq!(
            visualiser_fps(Budget::Saver, saving, 144),
            Some(30),
            "saving energy"
        );
        assert_eq!(visualiser_fps(Budget::Saver, saving, 30), Some(15));
        assert_eq!(visualiser_fps(Budget::Background, normal, 60), None);
        assert_eq!(visualiser_fps(Budget::Background, normal, 0), None);
    }

    /// Demo runs and tests get a fixed `Power`, and the device's own state
    /// (Low Power Mode on a Mac) must not leak into what they see.
    #[test]
    fn a_fixed_monitor_keeps_its_conditions() {
        let given = Conditions {
            online: false,
            ..Conditions::default()
        };
        let mut power = Power::fixed(given);
        assert_eq!(power.conditions(), given);
        power.set_background(true);
        assert_eq!(power.conditions(), given);
        assert!(power.events().is_empty());
    }

    #[test]
    fn a_change_wakes_the_app_once_and_a_repeat_does_not() {
        let woken = Arc::new(AtomicU32::new(0));
        let count = Arc::clone(&woken);
        let shared = Shared::new(move || {
            count.fetch_add(1, Ordering::Relaxed);
        });
        shared.update(|conditions| conditions.saver = true);
        shared.update(|conditions| conditions.saver = true);
        shared.update(|conditions| conditions.online = false);
        assert_eq!(woken.load(Ordering::Relaxed), 2);
    }

    #[test]
    fn tests_can_set_the_conditions_and_send_events() {
        let power = Power::fixed(Conditions::default());
        let metered = Conditions {
            metered: true,
            ..Conditions::default()
        };
        power.set_conditions(metered);
        assert_eq!(power.conditions(), metered);

        let ack = Ack::default();
        power.send(Event::Resumed);
        power.send(Event::Suspending(ack.clone()));
        let events = power.events();
        assert!(matches!(
            events.as_slice(),
            [Event::Resumed, Event::Suspending(_)]
        ));
        assert!(power.events().is_empty(), "each event is taken once");
        if let Some(Event::Suspending(answer)) = events.into_iter().nth(1) {
            answer.done();
        }
        assert!(
            ack.wait(Duration::ZERO),
            "the app's answer reaches the sender"
        );
    }

    #[test]
    fn a_reader_waits_for_the_answer_but_not_forever() {
        let power = Power::fixed(Conditions::default());
        let shared = Arc::clone(&power.shared);
        let answered = std::thread::spawn(move || {
            let started = std::time::Instant::now();
            shared.push_and_wait(Event::Suspending, Duration::from_secs(30));
            started.elapsed()
        });
        let ack = loop {
            if let Some(Event::Suspending(ack)) = power.events().pop() {
                break ack;
            }
            std::thread::sleep(Duration::from_millis(1));
        };
        ack.done();
        assert!(answered.join().unwrap() < Duration::from_secs(10));

        let started = std::time::Instant::now();
        power
            .shared
            .push_and_wait(Event::EndingSession, Duration::from_millis(30));
        assert!(started.elapsed() >= Duration::from_millis(25), "gave up");
    }

    #[test]
    fn events_read_plainly_in_the_log() {
        let event = Event::Suspending(Ack::default());
        assert_eq!(format!("{event:?}"), "Suspending(Ack)");
        assert_eq!(format!("{:?}", Activity::begin("test")), "Activity");
    }

    /// Beginning and ending activities, one inside another, as playback
    /// starts and stops. On macOS this asks NSProcessInfo; elsewhere it does
    /// nothing.
    #[test]
    fn activities_begin_and_end() {
        let outer = Activity::begin("Playing audio");
        let inner = Activity::begin("Loading a track");
        drop(inner);
        drop(outer);
    }

    /// The app holds both in state that may move between threads.
    #[test]
    fn power_and_activities_can_move_between_threads() {
        fn send_and_sync<T: Send + Sync>() {}
        send_and_sync::<Power>();
        send_and_sync::<Activity>();
        send_and_sync::<Ack>();
    }

    /// Playback holds off App Nap while it plays or loads, and lets go on
    /// pause, stop, an output error or signing out.
    #[test]
    fn only_playback_at_work_needs_an_activity() {
        use crate::engine::{LocalState, Playback};
        let mut local = LocalState {
            connected: true,
            playback: Playback::Playing,
            ..LocalState::default()
        };
        assert!(playback_needs_activity(&local));
        local.playback = Playback::Loading;
        assert!(playback_needs_activity(&local));
        for playback in [Playback::Paused, Playback::Stopped] {
            local.playback = playback;
            assert!(!playback_needs_activity(&local));
        }
        local.playback = Playback::Playing;
        local.error = Some("output unavailable".into());
        assert!(!playback_needs_activity(&local));
        local.error = None;
        local.connected = false;
        assert!(!playback_needs_activity(&local));
        assert!(!playback_needs_activity(&LocalState::default()));
    }
}
