//! The tray item on Windows and macOS: the same menu as on Linux, so closing
//! the window leaves the music playing on every desktop.
//!
//! Windows runs the item on its own thread with a message loop, like the
//! Linux one. macOS allows status items on the main thread only, and only
//! while its event loop runs, so there the item is created with the first
//! window and, while no window exists, the headless loop in `main` pumps
//! the application's events itself. Its Dock icon remains available and a
//! Dock activation recreates the window.

use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender};
use std::time::Duration;

use tray_icon::menu::{Menu, MenuEvent, MenuId, MenuItem, PredefinedMenuItem};
use tray_icon::{Icon, TrayIcon, TrayIconBuilder};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TrayCommand {
    Show,
    ShowHide,
    PlayPause,
    Next,
    Previous,
    Quit,
}

type Wake = Arc<dyn Fn() + Send + Sync>;

const SHOW: &str = "show";
const PLAY_PAUSE: &str = "play-pause";
const NEXT: &str = "next";
const PREVIOUS: &str = "previous";
const QUIT: &str = "quit";

fn command_for(id: &MenuId) -> Option<TrayCommand> {
    match id.0.as_str() {
        SHOW => Some(TrayCommand::ShowHide),
        PLAY_PAUSE => Some(TrayCommand::PlayPause),
        NEXT => Some(TrayCommand::Next),
        PREVIOUS => Some(TrayCommand::Previous),
        QUIT => Some(TrayCommand::Quit),
        _ => None,
    }
}

fn play_pause_label(playing: bool) -> &'static str {
    if playing { "Pause" } else { "Play" }
}

/// What a primary click on the icon asks for, or `None` where it opens the
/// menu. A macOS menu-bar item opens its menu on either click, as the
/// others in the menu bar do. Each release of a Windows double-click
/// arrives separately, possibly on opposite sides of window creation, so
/// both must ask to show the window rather than toggle it.
fn left_click_command(on_macos: bool) -> Option<TrayCommand> {
    (!on_macos).then_some(TrayCommand::Show)
}

fn left_click(sender: &Sender<TrayCommand>, wake: &Wake, on_macos: bool) {
    if let Some(command) = left_click_command(on_macos)
        && sender.send(command).is_ok()
    {
        wake();
    }
}

/// The item, and the menu entry whose label follows playback.
struct Item {
    icon: TrayIcon,
    play_pause: MenuItem,
}

/// The tray item's size in pixels, which the system scales.
const ICON_SIZE: u32 = 32;

/// Builds the item on the current thread and routes its events to `sender`.
fn build(sender: Sender<TrayCommand>, wake: Wake) -> Result<Item, Box<dyn std::error::Error>> {
    let size = ICON_SIZE;
    #[cfg(not(target_os = "macos"))]
    let icon = Icon::from_rgba(crate::util::app_icon_rgba(size as usize), size, size)?;
    // macOS draws template images itself, black or white to match the menu
    // bar, so the item looks native in either theme.
    #[cfg(target_os = "macos")]
    let icon = Icon::from_rgba(crate::util::tray_template_rgba(size as usize), size, size)?;
    let menu = Menu::new();
    let play_pause = MenuItem::with_id(PLAY_PAUSE, play_pause_label(false), true, None);
    menu.append_items(&[
        &MenuItem::with_id(SHOW, "Show or hide Fastsonic", true, None),
        &PredefinedMenuItem::separator(),
        &play_pause,
        &MenuItem::with_id(NEXT, "Next", true, None),
        &MenuItem::with_id(PREVIOUS, "Previous", true, None),
        &PredefinedMenuItem::separator(),
        &MenuItem::with_id(QUIT, "Quit", true, None),
    ])?;
    let builder = TrayIconBuilder::new()
        .with_icon(icon)
        .with_tooltip("Fastsonic")
        .with_menu(Box::new(menu));
    // A Windows click raises the window and the menu waits for a right
    // click (#310); a macOS menu-bar item opens its menu on either click.
    let builder =
        builder.with_menu_on_left_click(left_click_command(cfg!(target_os = "macos")).is_none());
    #[cfg(target_os = "macos")]
    let builder = builder.with_icon_as_template(true);
    let icon = builder.build()?;

    let menu_sender = sender.clone();
    let menu_wake = Arc::clone(&wake);
    MenuEvent::set_event_handler(Some(move |event: MenuEvent| {
        if let Some(command) = command_for(&event.id)
            && menu_sender.send(command).is_ok()
        {
            menu_wake();
        }
    }));
    tray_icon::TrayIconEvent::set_event_handler(Some(move |event: tray_icon::TrayIconEvent| {
        if let tray_icon::TrayIconEvent::Click {
            button: tray_icon::MouseButton::Left,
            button_state: tray_icon::MouseButtonState::Up,
            ..
        } = event
        {
            left_click(&sender, &wake, cfg!(target_os = "macos"));
        }
    }));

    Ok(Item { icon, play_pause })
}

#[cfg(windows)]
mod host {
    use super::*;
    use windows_sys::Win32::System::Threading::GetCurrentThreadId;
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        DispatchMessageW, GetMessageW, MSG, PostThreadMessageW, TranslateMessage, WM_APP,
    };

    /// A change to the item, carried to its thread.
    pub enum Change {
        Playing(bool),
        Colour(crate::settings::TrayIcon),
    }

    /// Runs the item on its own thread. Answers with the thread's id once
    /// the item exists, or with why it could not be made.
    pub fn start(
        sender: Sender<TrayCommand>,
        wake: Wake,
        changes: Receiver<Change>,
    ) -> Result<u32, String> {
        let (ready_tx, ready_rx) = std::sync::mpsc::channel();
        let spawned = std::thread::Builder::new()
            .name("fastsonic-tray".to_owned())
            .spawn(move || {
                let item = match build(sender, wake) {
                    Ok(item) => item,
                    Err(error) => {
                        let _ = ready_tx.send(Err(error.to_string()));
                        return;
                    }
                };
                let _ = ready_tx.send(Ok(unsafe { GetCurrentThreadId() }));
                let mut message: MSG = unsafe { std::mem::zeroed() };
                while unsafe { GetMessageW(&mut message, std::ptr::null_mut(), 0, 0) } > 0 {
                    if message.message == WM_APP {
                        while let Ok(change) = changes.try_recv() {
                            apply(&item, change);
                        }
                        continue;
                    }
                    unsafe {
                        TranslateMessage(&message);
                        DispatchMessageW(&message);
                    }
                }
            });
        if let Err(error) = spawned {
            return Err(error.to_string());
        }
        ready_rx
            .recv_timeout(Duration::from_secs(5))
            .map_err(|_| "the tray thread did not answer".to_string())?
    }

    fn apply(item: &Item, change: Change) {
        match change {
            Change::Playing(playing) => item.play_pause.set_text(play_pause_label(playing)),
            Change::Colour(colour) => {
                let size = ICON_SIZE;
                let pixels = crate::util::tray_icon_rgba(colour, size as usize);
                let changed = Icon::from_rgba(pixels, size, size)
                    .map_err(|error| error.to_string())
                    .and_then(|icon| {
                        item.icon
                            .set_icon(Some(icon))
                            .map_err(|error| error.to_string())
                    });
                if let Err(error) = changed {
                    log::warn!("the tray icon could not be changed: {error}");
                }
            }
        }
    }

    /// Wakes the thread's message loop to read what was sent to it.
    pub fn poke(thread_id: u32) {
        unsafe {
            PostThreadMessageW(thread_id, WM_APP, 0, 0);
        }
    }
}

#[cfg(windows)]
pub struct TrayService {
    commands: Receiver<TrayCommand>,
    playing: bool,
    colour: crate::settings::TrayIcon,
    changes: Sender<host::Change>,
    thread_id: u32,
}

#[cfg(windows)]
impl TrayService {
    /// Registers the tray item. `None` when it cannot be made.
    pub fn spawn(wake: impl Fn() + Send + Sync + 'static) -> Option<Self> {
        let (sender, commands) = std::sync::mpsc::channel();
        let (changes, changes_rx) = std::sync::mpsc::channel();
        match host::start(sender, Arc::new(wake), changes_rx) {
            Ok(thread_id) => Some(Self {
                commands,
                playing: false,
                colour: crate::settings::TrayIcon::Colour,
                changes,
                thread_id,
            }),
            Err(error) => {
                log::info!("no system tray available: {error}");
                None
            }
        }
    }

    pub fn drain_commands(&self) -> Vec<TrayCommand> {
        self.commands.try_iter().collect()
    }

    /// Keeps the menu's Play/Pause label matching reality.
    pub fn set_playing(&mut self, playing: bool) {
        if self.playing != playing {
            self.playing = playing;
            self.change(host::Change::Playing(playing));
        }
    }

    /// Shows the icon in colour, or white or black to match the taskbar,
    /// in place: the item and its menu stay.
    pub fn set_icon_colour(&mut self, colour: crate::settings::TrayIcon) {
        if self.colour != colour {
            self.colour = colour;
            self.change(host::Change::Colour(colour));
        }
    }

    fn change(&self, change: host::Change) {
        if self.changes.send(change).is_ok() {
            host::poke(self.thread_id);
        }
    }

    /// Nothing to do: the item lives on its own thread from the start.
    pub fn attach(&mut self) {}

    /// Nothing to do either; see `attach`.
    pub fn hidden(&mut self) {}
}

/// Waits while the app lives in the tray without a window, for up to
/// `duration` or until something wakes the app, answering the main
/// thread's window messages meanwhile.
///
/// The item runs on its own thread, but eframe keeps winit's event loop
/// between windows, and winit's event-target window, visible on purpose,
/// lives on this thread for the whole run. Windows calls a thread that
/// leaves a window's messages for five seconds hung (`IsHungAppWindow`,
/// "Not responding" in Task Manager); winit buffers what reaches that
/// window until the next window's loop runs.
#[cfg(windows)]
pub fn idle(duration: Duration, waker: &crate::backend::Waker) {
    messages::idle(duration, waker);
}

/// Ends [`idle`]'s wait for the thread's next message, from any thread.
#[cfg(windows)]
pub fn interrupt_idle() {
    messages::interrupt();
}

#[cfg(windows)]
mod messages {
    use std::sync::OnceLock;
    use std::time::{Duration, Instant};

    use windows_sys::Win32::Foundation::HANDLE;
    use windows_sys::Win32::System::Threading::{CreateEventW, SetEvent};
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        DispatchMessageW, MSG, MWMO_INPUTAVAILABLE, MsgWaitForMultipleObjectsEx, PM_REMOVE,
        PeekMessageW, PostQuitMessage, QS_ALLINPUT, TranslateMessage, WM_QUIT,
    };

    /// The auto-reset event a wake sets, so the wait for messages also ends
    /// on a wake. Created once and kept for the life of the process.
    struct Event(HANDLE);

    // SAFETY: an event handle may be waited on and set from any thread.
    unsafe impl Send for Event {}
    // SAFETY: as above; the handle is never closed.
    unsafe impl Sync for Event {}

    static WAKE: OnceLock<Option<Event>> = OnceLock::new();

    fn wake_event() -> Option<&'static Event> {
        WAKE.get_or_init(|| {
            // SAFETY: no security attributes and no name; an auto-reset event
            // that starts unset.
            let handle = unsafe { CreateEventW(std::ptr::null(), 0, 0, std::ptr::null()) };
            (!handle.is_null()).then_some(Event(handle))
        })
        .as_ref()
    }

    pub(super) fn interrupt() {
        if let Some(Some(event)) = WAKE.get() {
            // SAFETY: the handle is a live event, never closed.
            unsafe { SetEvent(event.0) };
        }
    }

    pub(super) fn idle(duration: Duration, waker: &crate::backend::Waker) {
        // Made before the first look at the waker, so a wake after that look
        // sets it and ends the wait below.
        let event = wake_event();
        let started = Instant::now();
        loop {
            if waker.take() {
                return;
            }
            let left = duration.saturating_sub(started.elapsed());
            if left.is_zero() {
                return;
            }
            // Rounded up, so a sub-millisecond rest waits rather than spins.
            let millis = u32::try_from(left.as_micros().div_ceil(1000)).unwrap_or(u32::MAX - 1);
            let handles = event.map(|event| event.0);
            // SAFETY: `handles` holds at most one live event handle and
            // outlives the call, which returns on a message, the event or the
            // timeout.
            unsafe {
                MsgWaitForMultipleObjectsEx(
                    u32::from(handles.is_some()),
                    handles
                        .as_ref()
                        .map_or(std::ptr::null(), std::ptr::from_ref),
                    millis,
                    QS_ALLINPUT,
                    MWMO_INPUTAVAILABLE,
                );
            }
            if !dispatch() {
                return;
            }
        }
    }

    /// Answers every message waiting for this thread. `false` when one was
    /// a quit, which is put back for the next window's loop.
    fn dispatch() -> bool {
        // SAFETY: an all-zero MSG is a valid value for PeekMessageW to fill.
        let mut message: MSG = unsafe { std::mem::zeroed() };
        // SAFETY: `message` outlives each call; a null window takes every
        // message for this thread.
        while unsafe { PeekMessageW(&mut message, std::ptr::null_mut(), 0, 0, PM_REMOVE) } != 0 {
            if message.message == WM_QUIT {
                // SAFETY: posts WM_QUIT to this thread with the same code.
                unsafe { PostQuitMessage(message.wParam as i32) };
                return false;
            }
            // SAFETY: `message` came from PeekMessageW just now.
            unsafe {
                TranslateMessage(&message);
                DispatchMessageW(&message);
            }
        }
        true
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use windows_sys::Win32::System::Threading::GetCurrentThreadId;
        use windows_sys::Win32::UI::WindowsAndMessaging::{
            PM_NOREMOVE, PostThreadMessageW, WM_APP,
        };

        fn peek(kind: u32, remove: bool) -> Option<MSG> {
            // SAFETY: an all-zero MSG is a valid value for PeekMessageW to fill.
            let mut message: MSG = unsafe { std::mem::zeroed() };
            let flags = if remove { PM_REMOVE } else { PM_NOREMOVE };
            // SAFETY: `message` outlives the call, which looks at `kind` only.
            let found =
                unsafe { PeekMessageW(&mut message, std::ptr::null_mut(), kind, kind, flags) };
            (found != 0).then_some(message)
        }

        /// Idling answers the thread's messages, so Windows never calls an app
        /// in the tray hung, leaves a quit for the next window's loop, and
        /// ends at once on a wake. One test, since the wake event is shared.
        #[test]
        fn idling_answers_the_threads_messages_keeps_a_quit_and_ends_on_a_wake() {
            let waker = crate::backend::Waker::default();
            // Gives the thread its message queue.
            assert!(peek(WM_APP + 2, false).is_none());
            // SAFETY: posts a message with no data to this thread.
            assert_ne!(
                unsafe { PostThreadMessageW(GetCurrentThreadId(), WM_APP + 2, 0, 0) },
                0
            );
            idle(Duration::from_millis(50), &waker);
            assert!(peek(WM_APP + 2, false).is_none(), "answered");

            // SAFETY: posts WM_QUIT to this thread.
            unsafe { PostQuitMessage(3) };
            idle(Duration::from_millis(50), &waker);
            assert_eq!(peek(WM_QUIT, true).expect("the quit is kept").wParam, 3);

            let other = waker.clone();
            let ringer = std::thread::spawn(move || {
                std::thread::sleep(Duration::from_millis(50));
                other.wake();
            });
            let started = Instant::now();
            idle(Duration::from_secs(10), &waker);
            assert!(started.elapsed() < Duration::from_secs(5), "woken");
            ringer.join().expect("the ringer finished");
        }
    }
}

#[cfg(target_os = "macos")]
mod host {
    use std::cell::RefCell;
    use std::ffi::CString;

    use std::sync::atomic::{AtomicPtr, Ordering};

    use objc2::encode::Encoding;
    use objc2::rc::Retained;
    use objc2::runtime::{AnyClass, AnyObject, Bool, MethodImplementation, Sel};
    use objc2::{Encode, MainThreadMarker, msg_send, sel};
    use objc2_app_kit::{NSApplication, NSEvent, NSEventMask, NSEventModifierFlags, NSEventType};
    use objc2_foundation::{NSDate, NSDefaultRunLoopMode};

    use super::*;

    thread_local! {
        /// The status item, which only the main thread may touch.
        pub static ITEM: RefCell<Option<Item>> = const { RefCell::new(None) };
        /// The channel a Dock click asks through, and the wake that makes
        /// somebody read it.
        pub(super) static REOPEN: RefCell<Option<(Sender<TrayCommand>, Wake)>> = const { RefCell::new(None) };
    }

    /// A Dock click, asking for the app back.
    ///
    /// `has_visible_windows` is not the question it sounds like: a window
    /// sitting in the Dock still counts as visible, which is exactly the
    /// case that needs help, so the flag is not consulted. Asking for a
    /// window that is already up costs a focus and nothing else.
    pub(super) fn request_reopen(_has_visible_windows: bool) -> Bool {
        REOPEN.with(|slot| {
            if let Some((sender, wake)) = slot.borrow().as_ref() {
                let _ = sender.send(TrayCommand::Show);
                // Ask for a frame as well as leaving a message. A minimized
                // window is drawn none at all, and every repaint this app
                // schedules is armed by the frame before it, so the only
                // reader of that message is a loop that has already stopped.
                wake();
            }
        });
        Bool::YES
    }

    extern "C-unwind" fn application_should_handle_reopen(
        _delegate: *mut AnyObject,
        _selector: Sel,
        _application: *mut NSApplication,
        has_visible_windows: Bool,
    ) -> Bool {
        request_reopen(has_visible_windows.as_bool())
    }

    fn install_reopen_handler(app: &NSApplication) {
        let Some(delegate) = app.delegate() else {
            log::warn!("the macOS application delegate is unavailable");
            return;
        };
        let delegate: &AnyObject = AsRef::<AnyObject>::as_ref(&*delegate);
        let class = delegate.class();
        let selector = sel!(applicationShouldHandleReopen:hasVisibleWindows:);
        if class.responds_to(selector) {
            return;
        }
        let implementation: extern "C-unwind" fn(
            *mut AnyObject,
            Sel,
            *mut NSApplication,
            Bool,
        ) -> Bool = application_should_handle_reopen;
        let types = CString::new(format!("{}@:@{}", Bool::ENCODING, Bool::ENCODING))
            .expect("valid Objective-C type encoding");
        let installed = unsafe {
            objc2::ffi::class_addMethod(
                class as *const AnyClass as *mut AnyClass,
                selector,
                implementation.__imp(),
                types.as_ptr(),
            )
        };
        if !installed.as_bool() {
            log::warn!("the macOS Dock reopen handler could not be installed");
        }
    }

    /// Creates the item, once, on the main thread.
    pub fn create(sender: Sender<TrayCommand>, wake: Wake, playing: bool) {
        let Some(mtm) = MainThreadMarker::new() else {
            log::warn!("the status item can only be made on the main thread");
            return;
        };
        REOPEN.with(|slot| *slot.borrow_mut() = Some((sender.clone(), Arc::clone(&wake))));
        install_reopen_handler(&NSApplication::sharedApplication(mtm));
        match build(sender, wake) {
            Ok(item) => {
                item.play_pause.set_text(play_pause_label(playing));
                ITEM.with(|slot| *slot.borrow_mut() = Some(item));
            }
            Err(error) => log::info!("no status item: {error}"),
        }
    }

    pub fn exists() -> bool {
        ITEM.with(|slot| slot.borrow().is_some())
    }

    /// Makes the status item again, with its icon and menu. winit closes
    /// every window the application has when a window's event loop ends,
    /// the status item's own among them, which takes the icon out of the
    /// menu bar for good; a new status item has a window of its own.
    pub fn renew() {
        ITEM.with(|slot| {
            if let Some(item) = slot.borrow().as_ref() {
                let renewed = item
                    .icon
                    .set_visible(false)
                    .and_then(|()| item.icon.set_visible(true));
                if let Err(error) = renewed {
                    log::warn!("the status item could not be made again: {error}");
                }
            }
        });
    }

    pub fn set_playing(playing: bool) {
        ITEM.with(|slot| {
            if let Some(item) = slot.borrow().as_ref() {
                item.play_pause.set_text(play_pause_label(playing));
            }
        });
    }

    pub fn activate() {
        let Some(mtm) = MainThreadMarker::new() else {
            return;
        };
        let app = NSApplication::sharedApplication(mtm);
        #[allow(deprecated)]
        app.activateIgnoringOtherApps(true);
    }

    /// The application while the tray loop waits on its events, for a wake
    /// from another thread to post an event to; null otherwise. It is the
    /// process's one NSApplication, which lives as long as the process.
    static IDLE_APP: AtomicPtr<AnyObject> = AtomicPtr::new(std::ptr::null_mut());

    /// The subtype of the application-defined event a wake posts, so the
    /// pump knows its own and sends it nowhere.
    const WAKE_SUBTYPE: i16 = 0x4653;

    /// `NSPoint`, for the one message that takes one; objc2-foundation
    /// has it only behind a feature this crate does not otherwise need.
    #[repr(C)]
    struct Point {
        x: f64,
        y: f64,
    }

    // SAFETY: the layout and encoding of CGPoint, two doubles.
    unsafe impl Encode for Point {
        const ENCODING: Encoding = Encoding::Struct("CGPoint", &[f64::ENCODING, f64::ENCODING]);
    }

    /// Runs the application's event loop for up to `duration`, so the
    /// status item and the media controls keep answering without a window,
    /// and returns as soon as something wakes the app.
    pub fn pump(duration: Duration, waker: &crate::backend::Waker) {
        let Some(mtm) = MainThreadMarker::new() else {
            waker.wait(duration);
            return;
        };
        let app = NSApplication::sharedApplication(mtm);
        let deadline = NSDate::dateWithTimeIntervalSinceNow(duration.as_secs_f64());
        // Safety: reading an extern static AppKit defines and never changes.
        let mode = unsafe { NSDefaultRunLoopMode };
        // Published before the first look, so a wake that comes after the
        // look finds somewhere to post its event.
        IDLE_APP.store(Retained::as_ptr(&app).cast_mut().cast(), Ordering::SeqCst);
        while !waker.take() {
            let event = app.nextEventMatchingMask_untilDate_inMode_dequeue(
                NSEventMask::Any,
                Some(&deadline),
                mode,
                true,
            );
            match event {
                Some(event) if is_wake(&event) => {}
                Some(event) => app.sendEvent(&event),
                None => break,
            }
        }
        IDLE_APP.store(std::ptr::null_mut(), Ordering::SeqCst);
    }

    fn is_wake(event: &NSEvent) -> bool {
        // Asking another type of event for its subtype raises.
        event.r#type() == NSEventType::ApplicationDefined && event.subtype().0 == WAKE_SUBTYPE
    }

    /// Ends the wait for the next event in [`pump`], from any thread, by
    /// posting an application-defined event, which AppKit allows from any
    /// thread. Nothing is posted while no pump is waiting.
    pub fn interrupt() {
        let app = IDLE_APP.load(Ordering::SeqCst);
        if app.is_null() {
            return;
        }
        let point = Point { x: 0.0, y: 0.0 };
        let none: *const AnyObject = std::ptr::null();
        // SAFETY: the arguments match
        // otherEventWithType:location:modifierFlags:timestamp:windowNumber:
        // context:subtype:data1:data2:, whose context must be nil, and the
        // application is the process's own, which is never released.
        unsafe {
            let event: Option<Retained<NSEvent>> = msg_send![
                AnyClass::get(c"NSEvent").expect("AppKit is loaded"),
                otherEventWithType: NSEventType::ApplicationDefined,
                location: point,
                modifierFlags: NSEventModifierFlags(0),
                timestamp: 0.0f64,
                windowNumber: 0isize,
                context: none,
                subtype: WAKE_SUBTYPE,
                data1: 0isize,
                data2: 0isize
            ];
            if let Some(event) = event {
                let _: () = msg_send![app, postEvent: &*event, atStart: false];
            }
        }
    }
}

#[cfg(target_os = "macos")]
pub struct TrayService {
    commands: Receiver<TrayCommand>,
    playing: bool,
    /// What the item needs, until the first window lets it be made.
    pending: Option<(Sender<TrayCommand>, Wake)>,
    /// Whether a window's event loop has ended since the item was made.
    life: ItemLife,
}

/// Whether the macOS status item outlived a window's event loop, which
/// closes the item's window with its own (see `host::renew`).
#[cfg(any(target_os = "macos", test))]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct ItemLife {
    stale: bool,
}

#[cfg(any(target_os = "macos", test))]
impl ItemLife {
    /// A window opens around the item, and its loop will end before the
    /// next window or the tray: whether the item must be made again first,
    /// because the last window's loop ended without the tray in between.
    fn window(&mut self) -> bool {
        std::mem::replace(&mut self.stale, true)
    }

    /// A window's loop ended and the app went to the tray: whether the
    /// item must be made again.
    fn tray(&mut self) -> bool {
        std::mem::replace(&mut self.stale, false)
    }
}

#[cfg(target_os = "macos")]
impl TrayService {
    /// Prepares the item. It is made with the first window, when AppKit's
    /// event loop is running, which it must be.
    pub fn spawn(wake: impl Fn() + Send + Sync + 'static) -> Option<Self> {
        let (sender, commands) = std::sync::mpsc::channel();
        Some(Self {
            commands,
            playing: false,
            pending: Some((sender, Arc::new(wake))),
            life: ItemLife::default(),
        })
    }

    pub fn drain_commands(&self) -> Vec<TrayCommand> {
        self.commands.try_iter().collect()
    }

    /// Keeps the menu's Play/Pause label matching reality.
    pub fn set_playing(&mut self, playing: bool) {
        if self.playing != playing {
            self.playing = playing;
            host::set_playing(playing);
        }
    }

    /// The menu bar already draws the template icon black or white to match
    /// itself, so the choice would only add a way to make the icon vanish
    /// on one of the two bars.
    pub fn set_icon_colour(&mut self, _colour: crate::settings::TrayIcon) {}

    /// A window exists: make the item if this is the first one and bring the
    /// application forward.
    pub fn attach(&mut self) {
        if let Some((sender, wake)) = self.pending.take() {
            host::create(sender, wake, self.playing);
        }
        if self.life.window() {
            host::renew();
        }
        if host::exists() {
            host::activate();
        }
    }

    /// No window: the status item and Dock icon both remain available, the
    /// item made again after the window's loop closed its window.
    pub fn hidden(&mut self) {
        if self.life.tray() {
            host::renew();
        }
    }
}

/// Waits while the app lives in the tray without a window, keeping AppKit
/// served meanwhile, for up to `duration` or until something wakes the app.
#[cfg(target_os = "macos")]
pub fn idle(duration: Duration, waker: &crate::backend::Waker) {
    host::pump(duration, waker);
}

/// Ends [`idle`]'s wait for AppKit's next event, from any thread.
#[cfg(target_os = "macos")]
pub fn interrupt_idle() {
    host::interrupt();
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    use super::*;

    /// A Dock click asks for the window whatever AppKit says about visible
    /// ones, and asks for a frame as well as leaving a message.
    ///
    /// Both halves are load-bearing. A window in the Dock is reported as
    /// visible, so a handler that trusted that flag did nothing at all for
    /// the one case that needed it; and a minimized window is drawn no
    /// frames, so the message alone would wait for a reader that has
    /// stopped running.
    #[test]
    fn a_dock_click_asks_for_the_window_and_for_a_frame() {
        let (sender, commands) = std::sync::mpsc::channel();
        let woken = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let counter = Arc::clone(&woken);
        let wake: Wake = Arc::new(move || {
            counter.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        });
        host::REOPEN.with(|slot| *slot.borrow_mut() = Some((sender, wake)));

        // A minimized window is the reported-visible case, and the one the
        // bug report is about.
        assert!(host::request_reopen(true).as_bool());
        assert_eq!(
            commands.try_recv(),
            Ok(TrayCommand::Show),
            "a window in the Dock reports as visible, and was left there"
        );
        assert_eq!(
            woken.load(std::sync::atomic::Ordering::SeqCst),
            1,
            "the message was left but nobody was asked to read it"
        );

        // No window at all: the tray case, which already worked.
        assert!(host::request_reopen(false).as_bool());
        assert_eq!(commands.try_recv(), Ok(TrayCommand::Show));
        assert_eq!(woken.load(std::sync::atomic::Ordering::SeqCst), 2);
        host::REOPEN.with(|slot| *slot.borrow_mut() = None);
    }
}

#[cfg(test)]
mod click_tests {
    use super::*;

    /// winit closes the status item's window whenever a window's loop
    /// ends, so the item is made again after each one: on the way to the
    /// tray, or when the next window opens straight after (a switch to or
    /// from the mini player), but not twice for the same loop.
    #[test]
    fn the_status_item_is_made_again_once_after_each_window() {
        let mut life = ItemLife::default();
        assert!(!life.window(), "the first window makes the item itself");
        assert!(life.tray(), "closed to the tray");
        assert!(!life.window(), "shown from the tray, already made again");
        assert!(life.window(), "switched to the mini player");
        assert!(life.tray(), "then closed to the tray");
        assert!(!life.tray());
    }

    /// Both releases of a Windows double-click ask to show the window, so
    /// the second cannot hide it again; a macOS click opens the menu and
    /// asks for nothing else, so a pick in it does not also toggle the
    /// window.
    #[test]
    fn windows_clicks_show_the_window_and_macos_clicks_open_the_menu() {
        let (sender, commands) = std::sync::mpsc::channel();
        let count = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let counter = Arc::clone(&count);
        let wake: Wake = Arc::new(move || {
            counter.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        });
        left_click(&sender, &wake, false);
        left_click(&sender, &wake, false);
        assert_eq!(
            commands.try_iter().collect::<Vec<_>>(),
            vec![TrayCommand::Show, TrayCommand::Show]
        );
        assert_eq!(count.load(std::sync::atomic::Ordering::SeqCst), 2);
        left_click(&sender, &wake, true);
        assert_eq!(
            commands.try_recv(),
            Err(std::sync::mpsc::TryRecvError::Empty)
        );
        assert_eq!(count.load(std::sync::atomic::Ordering::SeqCst), 2);
        assert_eq!(left_click_command(true), None, "the menu opens instead");
    }
}
