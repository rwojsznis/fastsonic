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
    use objc2::MainThreadMarker;
    use objc2_app_kit::NSScreen;

    // winit places macOS windows in points, which saved positions already are.
    let Some(anchor) = titlebar_anchor(pos, 1.0) else {
        return false;
    };
    // AppKit answers only on the main thread, where windows are restored.
    // Without it, or without a display to ask about, keep the old limits.
    let Some(mtm) = MainThreadMarker::new() else {
        return within_fixed_limits(pos);
    };
    let screens = NSScreen::screens(mtm);
    let Some(primary) = screens.iter().next() else {
        return within_fixed_limits(pos);
    };
    let primary = primary.frame();
    let primary_top = primary.origin.y + primary.size.height;
    screens.iter().any(|screen| {
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
        .contains(anchor)
    })
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
