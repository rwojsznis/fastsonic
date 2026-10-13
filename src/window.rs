//! Checks saved window positions before restoring them.
//!
//! eframe already places the window on-screen. The session can still refer
//! to a monitor that was unplugged, so check before moving the window there.

pub const ON_TOP_UNAVAILABLE: &str =
    "On Wayland, use your desktop's Keep Above shortcut or window rule.";

/// Says why no window opened on Windows on ARM and offers the fix.
///
/// Snapdragon graphics drivers have no OpenGL of their own. Windows provides
/// it on ARM through Microsoft's compatibility pack, a Store app that
/// translates it to Direct3D 12, so a missing or broken pack leaves eframe
/// without a context and the app would otherwise close without a word.
#[cfg(all(windows, target_arch = "aarch64"))]
pub fn report_missing_opengl() {
    use windows_sys::Win32::UI::WindowsAndMessaging::{IDYES, MB_ICONERROR, MB_YESNO, MessageBoxW};
    /// The Store page of the OpenCL, OpenGL & Vulkan Compatibility Pack.
    const STORE_PAGE: &str = "ms-windows-store://pdp/?ProductId=9NQPSL29BFFF";

    let wide = |text: &str| text.encode_utf16().chain([0]).collect::<Vec<u16>>();
    let title = wide("Fastsonic cannot open its window");
    let text = wide(
        "On Windows on ARM, the OpenGL that Fastsonic draws with comes from \
         Microsoft's OpenCL, OpenGL & Vulkan Compatibility Pack. Open its \
         Microsoft Store page to install or repair it?",
    );
    // SAFETY: both strings are NUL-terminated UTF-16 that outlive the call,
    // and the box has no owner window.
    let answer = unsafe {
        MessageBoxW(
            std::ptr::null_mut(),
            text.as_ptr(),
            title.as_ptr(),
            MB_YESNO | MB_ICONERROR,
        )
    };
    if answer == IDYES
        && let Err(error) = open::that(STORE_PAGE)
    {
        log::warn!("cannot open the Microsoft Store: {error}");
    }
}

/// The active backend matters: a Wayland session can also host X11 windows.
/// winit's Wayland backend cannot change a window's stacking level.
pub fn supports_window_level(display: raw_window_handle::RawDisplayHandle) -> bool {
    !matches!(display, raw_window_handle::RawDisplayHandle::Wayland(_))
}

/// Whether the mini player can keep its window without a taskbar entry.
/// Windows and X11 can; Wayland has no protocol for it, and macOS has the
/// Dock instead of a taskbar.
pub fn supports_hiding_from_taskbar(display: raw_window_handle::RawDisplayHandle) -> bool {
    use raw_window_handle::RawDisplayHandle;
    matches!(
        display,
        RawDisplayHandle::Windows(_) | RawDisplayHandle::Xlib(_) | RawDisplayHandle::Xcb(_)
    )
}

/// Keeps an X11 window out of the taskbar with `_NET_WM_STATE_SKIP_TASKBAR`.
///
/// winit can only do this on Windows, so this asks the window manager
/// directly. A window that is not mapped yet carries the state in its
/// property, which the window manager reads when it maps the window; a
/// mapped one asks the window manager, as EWMH requires. Any other handle is
/// left alone.
#[cfg(target_os = "linux")]
pub fn skip_x11_taskbar(window: raw_window_handle::RawWindowHandle) {
    use raw_window_handle::RawWindowHandle;
    let window = match window {
        RawWindowHandle::Xlib(handle) => match u32::try_from(handle.window) {
            Ok(window) => window,
            Err(_) => return,
        },
        RawWindowHandle::Xcb(handle) => handle.window.get(),
        _ => return,
    };
    if let Err(error) = x11::skip_taskbar(window) {
        log::warn!("could not hide the mini player from the taskbar: {error}");
    }
}

#[cfg(target_os = "linux")]
mod x11 {
    use x11rb::connection::Connection;
    use x11rb::protocol::xproto::{
        AtomEnum, ClientMessageEvent, ConnectionExt as _, EventMask, MapState, PropMode,
    };
    use x11rb::wrapper::ConnectionExt as _;

    /// `_NET_WM_STATE_ADD` in a `_NET_WM_STATE` client message.
    const ADD: u32 = 1;
    /// The request comes from an ordinary application.
    const FROM_APPLICATION: u32 = 1;

    pub(super) fn skip_taskbar(window: u32) -> Result<(), Box<dyn std::error::Error>> {
        let (connection, screen) = x11rb::connect(None)?;
        let root = connection.setup().roots[screen].root;
        let state = connection
            .intern_atom(false, b"_NET_WM_STATE")?
            .reply()?
            .atom;
        let skip = connection
            .intern_atom(false, b"_NET_WM_STATE_SKIP_TASKBAR")?
            .reply()?
            .atom;
        let mapped =
            connection.get_window_attributes(window)?.reply()?.map_state != MapState::UNMAPPED;
        if mapped {
            let event = ClientMessageEvent::new(32, window, state, message_data(skip));
            connection.send_event(
                false,
                root,
                EventMask::SUBSTRUCTURE_REDIRECT | EventMask::SUBSTRUCTURE_NOTIFY,
                event,
            )?;
        } else {
            connection.change_property32(
                PropMode::APPEND,
                window,
                state,
                AtomEnum::ATOM,
                &[skip],
            )?;
        }
        connection.flush()?;
        Ok(())
    }

    /// The `_NET_WM_STATE` request that adds one state.
    pub(super) fn message_data(atom: u32) -> [u32; 5] {
        [ADD, atom, 0, FROM_APPLICATION, 0]
    }
}

/// Whether the window already covers the screen, maximized or full screen.
///
/// eframe restores that state as it creates the window, and sizing or moving
/// the window afterwards restores it down again.
pub fn fills_the_screen(viewport: &egui::ViewportInfo) -> bool {
    viewport.maximized.unwrap_or(false) || viewport.fullscreen.unwrap_or(false)
}

/// Checks a position in egui points against the fixed coordinate limits.
#[cfg(not(any(windows, target_os = "macos")))]
pub fn can_restore(pos: [f32; 2], _pixels_per_point: f32) -> bool {
    // Wayland ignores window moves; keep the old limits on other platforms.
    within_fixed_limits(pos)
}

#[cfg(not(windows))]
fn within_fixed_limits(pos: [f32; 2]) -> bool {
    (-1000.0..=5000.0).contains(&pos[0]) && (-1000.0..=5000.0).contains(&pos[1])
}

/// Checks that the saved position leaves the title bar on a connected
/// display, outside the menu bar and the Dock.
///
/// AppKit creates and moves a borderless window wherever it is told, even
/// onto a display that has since been unplugged.
#[cfg(target_os = "macos")]
pub fn can_restore(pos: [f32; 2], _pixels_per_point: f32) -> bool {
    // winit places macOS windows in points, which saved positions already are.
    let Some(anchor) = titlebar_anchor(pos, 1.0) else {
        return false;
    };
    // AppKit answers only on the main thread, where windows are restored.
    // Without it, or without a display to ask about, keep the old limits.
    match visible_screens() {
        Some(screens) => screens.iter().any(|screen| screen.contains(anchor)),
        None => within_fixed_limits(pos),
    }
}

/// The area each connected screen leaves for windows (without the menu bar
/// and the Dock), main screen first, in points measured down from the top of
/// the main screen as winit measures windows. `None` off the main thread,
/// where AppKit cannot be asked, and without any screen.
#[cfg(target_os = "macos")]
fn visible_screens() -> Option<Vec<egui::Rect>> {
    use objc2::MainThreadMarker;
    use objc2_app_kit::NSScreen;

    let screens = NSScreen::screens(MainThreadMarker::new()?);
    let primary = screens.iter().next()?.frame();
    let primary_top = primary.origin.y + primary.size.height;
    Some(
        screens
            .iter()
            .map(|screen| {
                let area = screen.visibleFrame();
                visible_area(
                    primary_top,
                    [
                        area.origin.x,
                        area.origin.y,
                        area.size.width,
                        area.size.height,
                    ],
                )
            })
            .collect(),
    )
}

/// Where to move a window that no connected monitor shows, in physical
/// virtual-desktop pixels, or `None` to leave it where it is.
///
/// Any overlap with any monitor counts as visible, so a window on a
/// secondary display, left of or above the primary one included, is never
/// moved. A lost window goes to the middle of the first monitor listed (the
/// primary one), pinned to its top-left corner when it is larger. Without
/// monitors nothing moves.
#[cfg(any(not(target_os = "macos"), test))]
fn recovered_position(window: egui::Rect, monitors: &[egui::Rect]) -> Option<egui::Pos2> {
    let target = *monitors.first()?;
    if monitors
        .iter()
        .any(|monitor| window.intersect(*monitor).area() > 0.0)
    {
        return None;
    }
    Some(centered(window, target))
}

/// Where to move a window whose title bar is on none of `screens`, in the
/// same points, or `None` to leave it. It goes to the middle of the first
/// screen (the main one).
#[cfg(any(target_os = "macos", test))]
fn recovered_titlebar_position(window: egui::Rect, screens: &[egui::Rect]) -> Option<egui::Pos2> {
    let target = *screens.first()?;
    let on_screen = titlebar_anchor([window.min.x, window.min.y], 1.0)
        .is_some_and(|anchor| screens.iter().any(|screen| screen.contains(anchor)));
    (!on_screen).then(|| centered(window, target))
}

/// Where `window` sits in the middle of `target`, pinned to its top-left
/// corner when it is larger.
fn centered(window: egui::Rect, target: egui::Rect) -> egui::Pos2 {
    let slack = (target.size() - window.size()).max(egui::Vec2::ZERO);
    (target.min + slack / 2.0).round()
}

/// Moves a new window onto the primary monitor when it opened on none of
/// the connected ones, as a position saved on a display unplugged since, or
/// one that comes up late after a restart, can leave it. Call it on the
/// window's first frames. Does nothing on Wayland, which does not reveal
/// window positions, for a minimized window, and without a native window.
#[cfg(not(target_os = "macos"))]
pub fn recover_offscreen(ctx: &egui::Context, frame: &eframe::Frame) {
    let Some(window) = frame.winit_window() else {
        return;
    };
    // A minimized window on Windows reports a parking position far off
    // screen; restoring it brings back its real one.
    if window.is_minimized() == Some(true) {
        return;
    }
    let Ok(position) = window.outer_position() else {
        return;
    };
    let size = window.outer_size();
    let rect = |x: i32, y: i32, width: u32, height: u32| {
        egui::Rect::from_min_size(
            egui::pos2(x as f32, y as f32),
            egui::vec2(width as f32, height as f32),
        )
    };
    // The primary monitor comes first, as the place a lost window goes; a
    // platform without one falls back to the first connected monitor.
    let monitors: Vec<_> = window
        .primary_monitor()
        .into_iter()
        .chain(window.available_monitors())
        .map(|monitor| {
            let (position, size) = (monitor.position(), monitor.size());
            rect(position.x, position.y, size.width, size.height)
        })
        .collect();
    let Some(target) = recovered_position(
        rect(position.x, position.y, size.width, size.height),
        &monitors,
    ) else {
        return;
    };
    log::warn!("the window opened on no connected monitor; moving it to the primary one");
    ctx.send_viewport_cmd(move_to_physical(target, ctx.pixels_per_point()));
}

/// macOS: the window's title bar must be on a screen's visible area, clear
/// of the menu bar and the Dock, for the window to be dragged. winit gives
/// the frame in physical pixels at the window's own scale, so dividing by it
/// gives AppKit's points, the unit the screens come in. A window just
/// created is not minimized, and asking AppKit whether it is can deadlock
/// (emilk/egui#3494). A full-screen window covers the menu bar and is left
/// alone.
#[cfg(target_os = "macos")]
pub fn recover_offscreen(ctx: &egui::Context, frame: &eframe::Frame) {
    let Some(window) = frame.winit_window() else {
        return;
    };
    if window.fullscreen().is_some() {
        return;
    }
    let Some(screens) = visible_screens() else {
        return;
    };
    let Ok(position) = window.outer_position() else {
        return;
    };
    let scale = window.scale_factor();
    let position = position.to_logical::<f32>(scale);
    let size = window.outer_size().to_logical::<f32>(scale);
    let window = egui::Rect::from_min_size(
        egui::pos2(position.x, position.y),
        egui::vec2(size.width, size.height),
    );
    let Some(target) = recovered_titlebar_position(window, &screens) else {
        return;
    };
    log::warn!("the window's title bar opened on no screen; moving it to the main one");
    ctx.send_viewport_cmd(move_to_physical(
        target * scale as f32,
        ctx.pixels_per_point(),
    ));
}

/// The command that moves the window to `target` physical pixels. egui
/// multiplies the position by the window's pixels per point, so dividing by
/// the same factor asks for exactly these pixels.
fn move_to_physical(target: egui::Pos2, pixels_per_point: f32) -> egui::ViewportCommand {
    egui::ViewportCommand::OuterPosition(target / pixels_per_point)
}

/// Turns an AppKit screen rectangle `[x, y, width, height]` into window
/// coordinates. AppKit measures up from the bottom of the primary display,
/// and winit down from its top.
#[cfg(any(target_os = "macos", test))]
fn visible_area(primary_top: f64, [x, y, width, height]: [f64; 4]) -> egui::Rect {
    egui::Rect::from_min_size(
        egui::pos2(x as f32, (primary_top - y - height) as f32),
        egui::vec2(width as f32, height as f32),
    )
}

#[cfg(any(windows, target_os = "macos", test))]
fn titlebar_anchor(pos: [f32; 2], pixels_per_point: f32) -> Option<egui::Pos2> {
    // A maximized window can start at (-8, -8), so check a point inside
    // its title bar. Convert from egui points to pixels for Win32.
    let anchor = (egui::pos2(pos[0], pos[1]) + egui::vec2(32.0, 16.0)) * pixels_per_point;
    (pixels_per_point.is_finite() && pixels_per_point > 0.0 && anchor.is_finite()).then_some(anchor)
}

/// Checks that the saved position leaves the title bar in a monitor's work area.
#[cfg(windows)]
pub fn can_restore(pos: [f32; 2], pixels_per_point: f32) -> bool {
    use windows_sys::Win32::Foundation::POINT;
    use windows_sys::Win32::Graphics::Gdi::{
        GetMonitorInfoW, MONITOR_DEFAULTTONULL, MONITORINFO, MonitorFromPoint,
    };

    let Some(anchor) = titlebar_anchor(pos, pixels_per_point) else {
        return false;
    };
    let point = POINT {
        x: anchor.x as i32,
        y: anchor.y as i32,
    };
    let mut info = MONITORINFO {
        cbSize: std::mem::size_of::<MONITORINFO>() as u32,
        ..Default::default()
    };
    unsafe {
        // Asking for the nearest monitor would also accept off-screen positions.
        let monitor = MonitorFromPoint(point, MONITOR_DEFAULTTONULL);
        if monitor.is_null() || GetMonitorInfoW(monitor, &mut info) == 0 {
            return false;
        }
    }
    // Use the work area so the title bar cannot end up behind the taskbar.
    let area = info.rcWork;
    egui::Rect::from_min_max(
        egui::pos2(area.left as f32, area.top as f32),
        egui::pos2(area.right as f32, area.bottom as f32),
    )
    .contains(anchor)
}

#[cfg(any(target_os = "macos", test))]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum MacosDoubleClickAction {
    Ignore,
    Minimize,
    Zoom,
}

/// What Desktop & Dock's "Double-click a window's title bar to" asks for.
/// When that choice was never stored, the older "Double-click a window's
/// title bar to minimize" switch decides between Minimize and Zoom.
#[cfg(any(target_os = "macos", test))]
fn macos_double_click_action(
    preference: Option<&str>,
    legacy_minimize: bool,
) -> MacosDoubleClickAction {
    match preference {
        Some("Minimize") => MacosDoubleClickAction::Minimize,
        Some("None") => MacosDoubleClickAction::Ignore,
        // AppKit already handles Fill as part of the native window drag.
        Some("Maximize" | "Fill") => MacosDoubleClickAction::Ignore,
        Some("Zoom") => MacosDoubleClickAction::Zoom,
        None if legacy_minimize => MacosDoubleClickAction::Minimize,
        None => MacosDoubleClickAction::Zoom,
        Some(_) => MacosDoubleClickAction::Ignore,
    }
}

/// Handles a macOS title-bar double-click, or leaves a first click to drag.
#[cfg(target_os = "macos")]
pub fn macos_titlebar_should_drag() -> bool {
    use objc2::{MainThreadMarker, sel};
    use objc2_app_kit::{NSApplication, NSEventType};
    use objc2_foundation::{NSObjectNSDelayedPerforming, NSUserDefaults, ns_string};

    let Some(mtm) = MainThreadMarker::new() else {
        return true;
    };
    let app = NSApplication::sharedApplication(mtm);
    let Some(event) = app.currentEvent() else {
        return true;
    };
    if event.r#type() != NSEventType::LeftMouseDown || event.clickCount() != 2 {
        return true;
    }

    let defaults = NSUserDefaults::standardUserDefaults();
    let preference = defaults
        .stringForKey(ns_string!("AppleActionOnDoubleClick"))
        .map(|action| action.to_string());
    let legacy_minimize = defaults.boolForKey(ns_string!("AppleMiniaturizeOnDoubleClick"));
    let action = macos_double_click_action(preference.as_deref(), legacy_minimize);
    if let Some(window) = event.window(mtm) {
        // Let egui finish this frame before AppKit starts resizing the window.
        // SAFETY: Both NSWindow selectors take one optional sender argument.
        unsafe {
            match action {
                MacosDoubleClickAction::Ignore => {}
                MacosDoubleClickAction::Minimize => window.performSelector_withObject_afterDelay(
                    sel!(performMiniaturize:),
                    None,
                    0.0,
                ),
                MacosDoubleClickAction::Zoom => {
                    window.performSelector_withObject_afterDelay(sel!(performZoom:), None, 0.0)
                }
            }
        }
    }
    false
}

#[cfg(not(target_os = "macos"))]
pub fn macos_titlebar_should_drag() -> bool {
    true
}

/// Minimize the active window without leaving egui's macOS viewport flag stale
/// when the user later restores it from the Dock.
pub fn minimize_window(ctx: &egui::Context) {
    #[cfg(target_os = "macos")]
    {
        use objc2::{MainThreadMarker, sel};
        use objc2_app_kit::NSApplication;
        use objc2_foundation::NSObjectNSDelayedPerforming;

        let Some(mtm) = MainThreadMarker::new() else {
            return;
        };
        let app = NSApplication::sharedApplication(mtm);
        let window = app.currentEvent().and_then(|event| event.window(mtm));
        if let Some(window) = window.or_else(|| app.keyWindow()) {
            // Let egui finish this frame before AppKit minimizes the window.
            // SAFETY: NSWindow's selector takes one optional sender argument.
            unsafe {
                window.performSelector_withObject_afterDelay(sel!(miniaturize:), None, 0.0);
            }
        }
    }
    #[cfg(not(target_os = "macos"))]
    ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(true));
    #[cfg(target_os = "macos")]
    let _ = ctx;
}

/// Brings the open window forward.
///
/// A minimized window has to be restored before it can be focused: Focus
/// alone leaves it in the Dock, and the platform draws no frames while it
/// is down there, so nothing arrives to ask a second time. Wayland drops a
/// window's own request for focus, but niri focuses a window when asked
/// through its command line tool, switching to its workspace; that is asked
/// first, off the UI thread, and the window asks for itself only when niri
/// cannot.
pub fn raise_window(ctx: &egui::Context) {
    #[cfg(target_os = "linux")]
    if under_niri(std::env::var_os("NIRI_SOCKET")) {
        let window = ctx.clone();
        let spawned = std::thread::Builder::new()
            .name("fastsonic-raise".to_string())
            .spawn(move || {
                if !niri::focus(std::process::id()) {
                    restore_and_focus(&window);
                }
            });
        if spawned.is_err() {
            restore_and_focus(ctx);
        }
        return;
    }
    restore_and_focus(ctx);
}

fn restore_and_focus(ctx: &egui::Context) {
    ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
    ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
}

/// Whether `NIRI_SOCKET` names niri's socket. An empty value counts as
/// unset.
#[cfg(any(target_os = "linux", test))]
fn under_niri(socket: Option<std::ffi::OsString>) -> bool {
    socket.is_some_and(|socket| !socket.is_empty())
}

#[cfg(target_os = "linux")]
mod niri {
    use std::process::{Command, Stdio};

    /// Asks niri to focus the window of process `pid`: `niri msg --json
    /// windows` finds it, `niri msg action focus-window --id` focuses it.
    /// `false` when `niri` is missing (a Flatpak sandbox) or fails.
    pub(super) fn focus(pid: u32) -> bool {
        let Ok(windows) = Command::new("niri")
            .args(["msg", "--json", "windows"])
            .stdin(Stdio::null())
            .stderr(Stdio::null())
            .output()
        else {
            return false;
        };
        let Some(id) = windows
            .status
            .success()
            .then(|| super::niri_window_id(&windows.stdout, pid))
            .flatten()
        else {
            return false;
        };
        Command::new("niri")
            .args(["msg", "action", "focus-window", "--id", &id.to_string()])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|status| status.success())
    }
}

/// The id of the window owned by process `pid` in `niri msg --json windows`.
#[cfg(any(target_os = "linux", test))]
fn niri_window_id(json: &[u8], pid: u32) -> Option<u64> {
    let windows: serde_json::Value = serde_json::from_slice(json).ok()?;
    windows
        .as_array()?
        .iter()
        .find(|window| window["pid"].as_u64() == Some(u64::from(pid)))?["id"]
        .as_u64()
}

/// Whether windows wait for the display before swapping buffers. Without it
/// every input event draws a frame at once: moving the pointer over the
/// window cost about 1.6 cores on Windows, against 0.35 with it (upstream
/// 80c4fe6), and AppKit's resize and zoom animations need swaps paced with
/// the display. eframe paints nothing for a minimized or occluded window, so
/// a hidden window never waits on macOS, Windows or X11.
///
/// Wayland is the exception: winit 0.30 reports neither state there, and a
/// compositor sends no frame callbacks to a window it is not showing, so a
/// vsync wait would block the whole app. A Wayland session is the one winit
/// itself chooses, from `WAYLAND_DISPLAY` or `WAYLAND_SOCKET`.
pub fn vsync() -> bool {
    static VSYNC: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *VSYNC.get_or_init(|| {
        let wayland = ["WAYLAND_DISPLAY", "WAYLAND_SOCKET"]
            .iter()
            .any(|name| std::env::var_os(name).is_some_and(|value| !value.is_empty()));
        let vsync = vsync_for(cfg!(all(unix, not(target_os = "macos"))), wayland);
        log::info!(
            "vsync {}",
            if vsync {
                "on"
            } else {
                "off: a hidden Wayland window would block on it"
            }
        );
        vsync
    })
}

const fn vsync_for(free_unix: bool, wayland: bool) -> bool {
    !(free_unix && wayland)
}

/// How long to ask egui to wait for the next frame of an animation that
/// moves once per `frame`. egui takes one predicted frame off every delayed
/// repaint, expecting vsync to supply it; without vsync that leaves nothing
/// and spins a core, so ask for two frames and wait one.
pub fn animation_repaint_delay(frame: std::time::Duration) -> std::time::Duration {
    repaint_delay_for(vsync(), frame)
}

const fn repaint_delay_for(vsync: bool, frame: std::time::Duration) -> std::time::Duration {
    if vsync {
        frame
    } else {
        frame.saturating_mul(2)
    }
}

/// The main window's smallest size with no side panel open. The page and
/// its top bar fit beside the narrowest sidebar from here.
pub const MAIN_MIN_SIZE: [f32; 2] = [760.0, 520.0];

static FIXED_SIZE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Whether the main window keeps one size it was given, as demo shots ask,
/// so the interface must not raise its minimum.
pub fn fixed_size() -> bool {
    FIXED_SIZE.load(std::sync::atomic::Ordering::Relaxed)
}

/// Pins the main window to the size it is created with.
pub fn set_fixed_size(on: bool) {
    FIXED_SIZE.store(on, std::sync::atomic::Ordering::Relaxed);
}

/// The range a saved zoom is held to, as the Settings control offers it.
const ZOOM_RANGE: std::ops::RangeInclusive<f32> = 0.5..=2.5;

/// Gives this window the interface zoom kept in the settings, and returns
/// the zoom to save once the listener changes it (Ctrl and plus or minus).
///
/// Call it on every pass of the main window. A new window has a new egui
/// context: the first call on it applies `saved`, even over a zoom egui's
/// restored memory carried, so the settings are the one place the zoom is
/// kept. Every call returns `Some(zoom)` while the window's zoom differs
/// from `saved`, including a `saved` out of range, applied as the nearest
/// usable zoom. The mini player has a scale of its own and does not call it.
pub fn keep_zoom(ctx: &egui::Context, saved: f32) -> Option<f32> {
    let applied = egui::Id::new("fastsonic-zoom-applied");
    let zoom = if ctx.data(|data| data.get_temp::<bool>(applied).unwrap_or(false)) {
        ctx.zoom_factor()
    } else {
        ctx.data_mut(|data| data.insert_temp(applied, true));
        // egui takes a new zoom at the start of the next pass.
        let zoom = if saved.is_finite() {
            saved.clamp(*ZOOM_RANGE.start(), *ZOOM_RANGE.end())
        } else {
            1.0
        };
        ctx.set_zoom_factor(zoom);
        zoom
    };
    (saved.is_nan() || (zoom - saved).abs() > 0.001).then_some(zoom)
}

/// A button of the custom title bar.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WindowButton {
    Minimize,
    Maximize,
    Close,
}

/// The custom title bar's buttons on Linux, at each end, in the order the
/// desktop lays them out.
#[cfg(any(target_os = "linux", test))]
#[derive(Debug)]
pub struct LinuxWindowControls {
    pub left: Vec<WindowButton>,
    pub right: Vec<WindowButton>,
}

/// What the desktop says about the custom title bar: where its buttons go
/// and the desktop icon theme's pictures for them, as minimize, maximize,
/// close and restore. A missing picture falls back to the app's own icon.
#[cfg(target_os = "linux")]
pub struct LinuxDesktop {
    pub controls: LinuxWindowControls,
    pub icons: [Option<Vec<u8>>; 4],
}

#[cfg(target_os = "linux")]
static LINUX_DESKTOP: std::sync::OnceLock<LinuxDesktop> = std::sync::OnceLock::new();

/// Starts reading the desktop's title bar preferences off the UI thread, so
/// they are in by the first frame that draws the custom title bar. Nothing
/// reads them unless that title bar is chosen.
#[cfg(target_os = "linux")]
fn read_linux_desktop_soon() {
    if LINUX_DESKTOP.get().is_some() {
        return;
    }
    let spawned = std::thread::Builder::new()
        .name("fastsonic-window-controls".to_string())
        .spawn(|| {
            linux_desktop();
        });
    if let Err(error) = spawned {
        log::warn!("unable to read the desktop's window buttons: {error}");
    }
}

/// The desktop's title bar preferences. A frame that asks before the
/// startup read has finished waits for it, at most about a second.
#[cfg(target_os = "linux")]
pub fn linux_desktop() -> &'static LinuxDesktop {
    LINUX_DESKTOP.get_or_init(read_linux_desktop)
}

/// GNOME's button layout and icon theme come through the desktop portal,
/// which answers inside a Flatpak too, where running `gsettings` cannot see
/// the host's settings. Other desktops draw Minimize, Maximize and Close on
/// the right.
#[cfg(target_os = "linux")]
fn read_linux_desktop() -> LinuxDesktop {
    let [layout, icon_theme] = crate::appearance::read_strings(
        [
            ("org.gnome.desktop.wm.preferences", "button-layout"),
            ("org.gnome.desktop.interface", "icon-theme"),
        ],
        std::time::Duration::from_secs(1),
    );
    let controls = if is_gnome() {
        layout
            .as_deref()
            .and_then(parse_linux_controls)
            .filter(|controls| !controls.left.is_empty() || !controls.right.is_empty())
            .unwrap_or_else(|| parse_linux_controls(":close").unwrap())
    } else {
        parse_linux_controls(":minimize,maximize,close").unwrap()
    };
    let icon_theme = icon_theme
        .map(|theme| theme.trim().trim_matches('\'').to_owned())
        .filter(|theme| !theme.is_empty() && !theme.contains('/') && theme != "." && theme != "..")
        .unwrap_or_else(|| "Adwaita".into());
    LinuxDesktop {
        controls,
        icons: linux_window_icons(&icon_theme),
    }
}

#[cfg(target_os = "linux")]
fn is_gnome() -> bool {
    std::env::var("XDG_CURRENT_DESKTOP")
        .unwrap_or_default()
        .split(':')
        .any(|desktop| desktop.eq_ignore_ascii_case("gnome"))
}

/// The icon theme's symbolic window icons, recoloured white so egui can
/// tint them: SVG loaders do not apply GTK's symbolic palette.
#[cfg(target_os = "linux")]
fn linux_window_icons(theme: &str) -> [Option<Vec<u8>>; 4] {
    let mut roots = Vec::new();
    if let Some(dirs) = directories::BaseDirs::new() {
        roots.push(dirs.data_dir().join("icons"));
        roots.push(dirs.home_dir().join(".icons"));
    }
    roots.extend(
        std::env::split_paths(
            &std::env::var_os("XDG_DATA_DIRS")
                .unwrap_or_else(|| "/usr/local/share:/usr/share".into()),
        )
        .map(|root| root.join("icons")),
    );
    [
        "window-minimize-symbolic",
        "window-maximize-symbolic",
        "window-close-symbolic",
        "window-restore-symbolic",
    ]
    .map(|name| {
        for theme in [theme, "Adwaita"] {
            for root in &roots {
                for folder in [
                    "symbolic/ui",
                    "symbolic/actions",
                    "scalable/actions",
                    "16x16/actions",
                ] {
                    if let Ok(svg) = std::fs::read_to_string(
                        root.join(theme).join(folder).join(format!("{name}.svg")),
                    ) && svg.contains("</svg>")
                    {
                        let white = concat!(
                            "<style>path,rect,circle,ellipse,polygon,polyline",
                            " { fill: white !important; }</style></svg>",
                        );
                        return Some(svg.replace("</svg>", white).into_bytes());
                    }
                }
            }
        }
        None
    })
}

/// Reads a GNOME `button-layout`: the buttons before the colon go on the
/// left, those after it on the right, each at most once. The portal gives
/// it plain; `gsettings` quoted it.
#[cfg(any(target_os = "linux", test))]
fn parse_linux_controls(layout: &str) -> Option<LinuxWindowControls> {
    use WindowButton::*;
    let layout = layout.trim().trim_matches('\'');
    let (left, right) = layout.split_once(':')?;
    let mut seen = Vec::new();
    let mut buttons = |side: &str| {
        side.split(',')
            .filter_map(|name| match name.trim() {
                "minimize" => Some(Minimize),
                "maximize" => Some(Maximize),
                "close" => Some(Close),
                _ => None,
            })
            .filter(|button| {
                if seen.contains(button) {
                    false
                } else {
                    seen.push(*button);
                    true
                }
            })
            .collect()
    };
    Some(LinuxWindowControls {
        left: buttons(left),
        right: buttons(right),
    })
}

static CUSTOM_TITLEBAR: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Whether the main window draws its own title bar and window buttons
/// instead of the platform's frame. Windows and Linux offer the choice, and
/// it is off unless the listener turns it on in Settings.
pub fn custom_titlebar() -> bool {
    custom_titlebar_for(
        cfg!(any(windows, target_os = "linux")),
        CUSTOM_TITLEBAR.load(std::sync::atomic::Ordering::Relaxed),
    )
}

/// Sets the title bar choice the next main window is created with.
pub fn set_custom_titlebar(on: bool) {
    CUSTOM_TITLEBAR.store(on, std::sync::atomic::Ordering::Relaxed);
    #[cfg(target_os = "linux")]
    if on {
        read_linux_desktop_soon();
    }
}

const fn custom_titlebar_for(supported: bool, chosen: bool) -> bool {
    supported && chosen
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_windows_and_x11_can_hide_the_mini_player_from_the_taskbar() {
        use raw_window_handle::{
            AppKitDisplayHandle, RawDisplayHandle, WaylandDisplayHandle, WindowsDisplayHandle,
            XcbDisplayHandle, XlibDisplayHandle,
        };
        let wayland = WaylandDisplayHandle::new(std::ptr::NonNull::dangling());
        assert!(!supports_hiding_from_taskbar(RawDisplayHandle::Wayland(
            wayland
        )));
        assert!(!supports_hiding_from_taskbar(RawDisplayHandle::AppKit(
            AppKitDisplayHandle::new()
        )));
        assert!(supports_hiding_from_taskbar(RawDisplayHandle::Windows(
            WindowsDisplayHandle::new()
        )));
        assert!(supports_hiding_from_taskbar(RawDisplayHandle::Xlib(
            XlibDisplayHandle::new(None, 0)
        )));
        assert!(supports_hiding_from_taskbar(RawDisplayHandle::Xcb(
            XcbDisplayHandle::new(None, 0)
        )));
    }

    /// EWMH's layout: add, the state, no second state, from an application.
    #[cfg(target_os = "linux")]
    #[test]
    fn the_skip_taskbar_request_adds_one_state_as_an_application() {
        assert_eq!(x11::message_data(42), [1, 42, 0, 1, 0]);
    }

    /// Needs an X server (`DISPLAY`), so it is not part of the ordinary
    /// suite. The window is never mapped, so nothing appears on screen.
    #[cfg(target_os = "linux")]
    #[test]
    #[ignore = "needs an X server"]
    fn an_unmapped_x11_window_keeps_its_states_and_skips_the_taskbar() {
        use x11rb::connection::Connection;
        use x11rb::protocol::xproto::{AtomEnum, ConnectionExt as _, CreateWindowAux, WindowClass};
        use x11rb::wrapper::ConnectionExt as _;

        let (connection, screen) = x11rb::connect(None).expect("an X server");
        let root = connection.setup().roots[screen].root;
        let window = connection.generate_id().unwrap();
        connection
            .create_window(
                0,
                window,
                root,
                0,
                0,
                1,
                1,
                0,
                WindowClass::INPUT_OUTPUT,
                0,
                &CreateWindowAux::new(),
            )
            .unwrap();
        let atom = |name: &[u8]| {
            connection
                .intern_atom(false, name)
                .unwrap()
                .reply()
                .unwrap()
                .atom
        };
        let state = atom(b"_NET_WM_STATE");
        let above = atom(b"_NET_WM_STATE_ABOVE");
        let skip = atom(b"_NET_WM_STATE_SKIP_TASKBAR");
        connection
            .change_property32(
                x11rb::protocol::xproto::PropMode::REPLACE,
                window,
                state,
                AtomEnum::ATOM,
                &[above],
            )
            .unwrap();
        connection.flush().unwrap();

        x11::skip_taskbar(window).unwrap();

        let states: Vec<u32> = connection
            .get_property(false, window, state, AtomEnum::ATOM, 0, 16)
            .unwrap()
            .reply()
            .unwrap()
            .value32()
            .unwrap()
            .collect();
        connection.destroy_window(window).unwrap();
        connection.flush().unwrap();
        assert_eq!(states, vec![above, skip]);
    }

    #[test]
    fn only_the_wayland_backend_lacks_window_level_control() {
        use raw_window_handle::{
            RawDisplayHandle, WaylandDisplayHandle, XcbDisplayHandle, XlibDisplayHandle,
        };
        let wayland = WaylandDisplayHandle::new(std::ptr::NonNull::dangling());
        assert!(!supports_window_level(RawDisplayHandle::Wayland(wayland)));
        assert!(supports_window_level(RawDisplayHandle::Xlib(
            XlibDisplayHandle::new(None, 0)
        )));
        assert!(supports_window_level(RawDisplayHandle::Xcb(
            XcbDisplayHandle::new(None, 0)
        )));
    }

    #[test]
    fn only_a_wayland_session_goes_without_vsync() {
        assert!(vsync_for(false, false), "macOS and Windows");
        assert!(vsync_for(false, true), "a stray WAYLAND_DISPLAY off Linux");
        assert!(vsync_for(true, false), "X11");
        assert!(!vsync_for(true, true), "Wayland");
    }

    #[test]
    fn an_animation_waits_one_frame_with_or_without_vsync() {
        let frame = std::time::Duration::from_micros(16_667);
        assert_eq!(repaint_delay_for(true, frame), frame);
        assert_eq!(repaint_delay_for(false, frame), frame * 2);
    }

    #[test]
    fn macos_titlebar_preferences_map_to_native_actions() {
        for (preference, legacy, action) in [
            (Some("Minimize"), false, MacosDoubleClickAction::Minimize),
            (Some("None"), false, MacosDoubleClickAction::Ignore),
            (Some("Maximize"), false, MacosDoubleClickAction::Ignore),
            (Some("Fill"), false, MacosDoubleClickAction::Ignore),
            (Some("Zoom"), false, MacosDoubleClickAction::Zoom),
            (Some("Zoom"), true, MacosDoubleClickAction::Zoom),
            (None, false, MacosDoubleClickAction::Zoom),
            (None, true, MacosDoubleClickAction::Minimize),
            (Some("FutureAction"), false, MacosDoubleClickAction::Ignore),
        ] {
            assert_eq!(
                macos_double_click_action(preference, legacy),
                action,
                "{preference:?}, legacy minimize {legacy}"
            );
        }
    }

    fn monitor(x: f32, y: f32, width: f32, height: f32) -> egui::Rect {
        egui::Rect::from_min_size(egui::pos2(x, y), egui::vec2(width, height))
    }

    #[test]
    fn a_window_above_every_monitor_moves_to_the_middle_of_the_first() {
        let window = monitor(100.0, -500.0, 400.0, 300.0);
        assert_eq!(
            recovered_position(window, &[monitor(0.0, 0.0, 1920.0, 1080.0)]),
            Some(egui::pos2(760.0, 390.0))
        );
    }

    #[test]
    fn a_window_partly_on_a_monitor_stays() {
        let window = monitor(-100.0, 100.0, 400.0, 300.0);
        assert_eq!(
            recovered_position(window, &[monitor(0.0, 0.0, 1920.0, 1080.0)]),
            None
        );
    }

    #[test]
    fn a_window_on_a_monitor_left_of_or_above_the_primary_stays() {
        let monitors = [
            monitor(0.0, 0.0, 1920.0, 1080.0),
            monitor(-1920.0, 0.0, 1920.0, 1080.0),
            monitor(0.0, -1440.0, 2560.0, 1440.0),
        ];
        let left = monitor(-1600.0, 100.0, 800.0, 600.0);
        let above = monitor(200.0, -1300.0, 800.0, 600.0);
        assert_eq!(recovered_position(left, &monitors), None);
        assert_eq!(recovered_position(above, &monitors), None);
    }

    /// A window saved on a display to the right, gone after a restart; the
    /// primary display here is not at the origin.
    #[test]
    fn a_window_left_where_a_disconnected_monitor_was_moves_to_the_primary() {
        let window = monitor(3891.0, -358.0, 1180.0, 780.0);
        let monitors = [
            monitor(1920.0, 0.0, 1920.0, 1080.0),
            monitor(0.0, 0.0, 1920.0, 1080.0),
        ];
        assert_eq!(
            recovered_position(window, &monitors),
            Some(egui::pos2(2290.0, 150.0))
        );
    }

    #[test]
    fn a_window_larger_than_the_monitor_is_pinned_to_its_corner() {
        let window = monitor(5000.0, 5000.0, 2000.0, 1200.0);
        assert_eq!(
            recovered_position(window, &[monitor(-1280.0, 0.0, 1280.0, 1024.0)]),
            Some(egui::pos2(-1280.0, 0.0))
        );
        assert_eq!(recovered_position(window, &[]), None, "no monitors");
    }

    #[test]
    fn the_move_asks_for_physical_pixels_at_any_scale() {
        assert_eq!(
            move_to_physical(egui::pos2(2290.0, 150.0), 1.25),
            egui::ViewportCommand::OuterPosition(egui::pos2(1832.0, 120.0))
        );
    }

    /// A Mac window on an unplugged display comes back to the main screen,
    /// and so does one whose title bar is under the menu bar.
    #[test]
    fn a_title_bar_on_no_screen_moves_to_the_main_one() {
        let screens = [
            monitor(0.0, 25.0, 1440.0, 875.0),
            monitor(1440.0, 0.0, 1920.0, 1080.0),
        ];
        let on_second = monitor(2000.0, 100.0, 400.0, 300.0);
        assert_eq!(recovered_titlebar_position(on_second, &screens), None);

        let unplugged = monitor(-2000.0, 100.0, 400.0, 300.0);
        assert_eq!(
            recovered_titlebar_position(unplugged, &screens),
            Some(egui::pos2(520.0, 313.0))
        );

        let under_menu_bar = monitor(100.0, 0.0, 400.0, 300.0);
        assert!(recovered_titlebar_position(under_menu_bar, &screens).is_some());
    }

    #[test]
    fn niri_window_id_finds_the_window_of_this_process() {
        let json = br#"[{"id":7,"pid":100},{"id":226,"pid":16363},{"id":9,"pid":null}]"#;
        assert_eq!(niri_window_id(json, 16363), Some(226));
        assert_eq!(niri_window_id(json, 1), None);
        assert_eq!(niri_window_id(b"not json", 1), None);
        assert_eq!(niri_window_id(br#"{"Err":"no"}"#, 100), None);
        let full = br#"[{"id":12,"title":"Fastsonic","app_id":"fastsonic","pid":4242,
            "workspace_id":3,"is_focused":false,"is_floating":false,"is_urgent":false}]"#;
        assert_eq!(niri_window_id(full, 4242), Some(12));
    }

    #[test]
    fn niri_is_found_by_its_socket() {
        assert!(under_niri(Some("/run/user/1000/niri.sock".into())));
        assert!(!under_niri(Some("".into())));
        assert!(!under_niri(None));
    }

    /// One pass of a window that keeps the saved zoom, with what it returned.
    fn zoom_pass(ctx: &egui::Context, saved: f32) -> Option<f32> {
        ctx.begin_pass(egui::RawInput::default());
        let kept = keep_zoom(ctx, saved);
        ctx.end_pass().textures_delta.clear();
        kept
    }

    #[test]
    fn each_new_window_takes_the_saved_zoom() {
        for _ in 0..2 {
            let ctx = egui::Context::default();
            assert_eq!(zoom_pass(&ctx, 1.5), None);
            assert_eq!(zoom_pass(&ctx, 1.5), None, "nothing to save");
            assert_eq!(ctx.zoom_factor(), 1.5);
        }
    }

    /// egui's saved memory carries a zoom of its own; the settings win.
    #[test]
    fn a_zoom_restored_with_eguis_memory_gives_way_to_the_saved_one() {
        for (remembered, saved) in [(1.5, 1.0), (1.0, 1.25)] {
            let ctx = egui::Context::default();
            ctx.set_zoom_factor(remembered);
            assert_eq!(zoom_pass(&ctx, saved), None, "{remembered} over {saved}");
            assert_eq!(zoom_pass(&ctx, saved), None, "{remembered} over {saved}");
            assert_eq!(ctx.zoom_factor(), saved);
        }
    }

    #[test]
    fn a_zoom_the_listener_picks_is_returned_until_it_is_saved() {
        let ctx = egui::Context::default();
        zoom_pass(&ctx, 1.25);
        ctx.set_zoom_factor(1.5);
        assert_eq!(zoom_pass(&ctx, 1.25), Some(1.5));
        assert_eq!(zoom_pass(&ctx, 1.5), None);
        assert_eq!(ctx.zoom_factor(), 1.5, "applied once, never forced back");
    }

    #[test]
    fn an_unusable_saved_zoom_is_held_to_a_usable_one() {
        for (saved, applied) in [
            (f32::NAN, 1.0),
            (f32::INFINITY, 1.0),
            (0.0, 0.5),
            (40.0, 2.5),
        ] {
            let ctx = egui::Context::default();
            assert_eq!(zoom_pass(&ctx, saved), Some(applied), "{saved}");
            assert_eq!(zoom_pass(&ctx, applied), None);
            assert_eq!(ctx.zoom_factor(), applied, "{saved}");
        }
    }

    #[test]
    fn supported_platforms_draw_the_custom_title_bar_only_when_chosen() {
        assert!(custom_titlebar_for(true, true));
        assert!(!custom_titlebar_for(true, false));
        assert!(!custom_titlebar_for(false, true));
        assert!(!custom_titlebar_for(false, false));
    }

    /// The portal gives GNOME's button layout as a plain string, where
    /// gsettings quoted it; both read the same, and a side may be empty.
    #[test]
    fn gnome_button_layouts_read_with_or_without_quotes() {
        use WindowButton::*;
        for layout in ["appmenu:minimize,close", "'appmenu:minimize,close'\n"] {
            let controls = parse_linux_controls(layout).unwrap();
            assert!(controls.left.is_empty());
            assert_eq!(controls.right, [Minimize, Close]);
        }
        let controls = parse_linux_controls("close,maximize,close:").unwrap();
        assert_eq!(controls.left, [Close, Maximize], "each button once");
        assert!(controls.right.is_empty());
        let controls = parse_linux_controls(":minimize,maximize,close").unwrap();
        assert_eq!(controls.right, [Minimize, Maximize, Close]);
        assert!(parse_linux_controls("close").is_none(), "no side marker");
    }

    fn reachable(pos: [f32; 2], scale: f32, area: [f32; 4]) -> bool {
        let rect =
            egui::Rect::from_min_max(egui::pos2(area[0], area[1]), egui::pos2(area[2], area[3]));
        titlebar_anchor(pos, scale).is_some_and(|anchor| rect.contains(anchor))
    }

    #[test]
    fn disconnected_monitor_position_is_not_restored() {
        assert!(!reachable([1912.0, -8.0], 1.0, [0.0, 0.0, 1920.0, 1032.0]));
        assert!(reachable(
            [1912.0, -8.0],
            1.0,
            [1920.0, 0.0, 3840.0, 1080.0]
        ));
    }

    #[test]
    fn visible_positions_include_maximized_and_negative_coordinates() {
        assert!(reachable([-8.0, -8.0], 1.0, [0.0, 0.0, 1920.0, 1032.0]));
        assert!(reachable(
            [-1920.0, 100.0],
            1.0,
            [-1920.0, 0.0, 0.0, 1080.0]
        ));
    }

    #[test]
    fn logical_coordinates_are_scaled_to_monitor_pixels() {
        assert!(reachable([900.0, 100.0], 2.0, [0.0, 0.0, 1920.0, 1080.0]));
        assert!(!reachable([1000.0, 100.0], 2.0, [0.0, 0.0, 1920.0, 1080.0]));
    }

    /// A 1512×982 built-in display with a 33-point menu bar, and a 1920×1080
    /// display to its right, bottom-aligned, so its top sits 98 points higher.
    fn on_mac_screens(pos: [f32; 2]) -> bool {
        let primary_top = 982.0;
        [[0.0, 0.0, 1512.0, 949.0], [1512.0, 0.0, 1920.0, 1080.0]]
            .into_iter()
            .any(|area| {
                titlebar_anchor(pos, 1.0)
                    .is_some_and(|anchor| visible_area(primary_top, area).contains(anchor))
            })
    }

    #[test]
    fn mac_positions_on_an_unplugged_display_are_not_restored() {
        assert!(on_mac_screens([2000.0, 100.0]));
        // Below the right display, right of both, and left of both.
        assert!(!on_mac_screens([2000.0, 1000.0]));
        assert!(!on_mac_screens([3500.0, 100.0]));
        assert!(!on_mac_screens([-300.0, 100.0]));
    }

    #[test]
    fn mac_screens_are_flipped_against_the_primary_display() {
        // The right display reaches 98 points above the primary's top edge.
        assert!(on_mac_screens([1600.0, -80.0]));
        assert!(!on_mac_screens([100.0, -80.0]));
        // The menu bar is not part of the visible area.
        assert!(!on_mac_screens([100.0, 0.0]));
        assert!(on_mac_screens([100.0, 40.0]));
    }

    #[test]
    fn only_a_maximized_or_full_screen_window_fills_the_screen() {
        let state = |maximized, fullscreen| {
            fills_the_screen(&egui::ViewportInfo {
                maximized,
                fullscreen,
                ..Default::default()
            })
        };
        assert!(state(Some(true), Some(false)));
        assert!(state(Some(false), Some(true)));
        assert!(!state(Some(false), Some(false)));
        // A backend that does not report the state leaves the window ordinary.
        assert!(!state(None, None));
    }

    #[test]
    fn invalid_coordinates_and_scale_are_rejected() {
        assert!(titlebar_anchor([f32::NAN, 0.0], 1.0).is_none());
        assert!(titlebar_anchor([0.0, f32::INFINITY], 1.0).is_none());
        assert!(titlebar_anchor([0.0, 0.0], 0.0).is_none());
    }
}
