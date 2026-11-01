//! Says in a native dialog why no window opened.
//!
//! eframe's glow renderer needs OpenGL 2 or later. A computer without its
//! graphics driver (the Microsoft Basic Display Adapter, many virtual
//! machines, some remote sessions, a Linux system without Mesa) offers less,
//! so `run_native` fails before the window draws anything. Started from a
//! desktop there is no console to see the error on, so the app would seem
//! to quit without a word. This shows the cause, the remedy and a link to
//! the page that explains it: a Win32 message box, an AppKit alert, or
//! zenity or kdialog on Linux (with neither, the error stays in the log).
//!
//! There is no second renderer to fall back to: MilkDrop draws in the window
//! with glow.

#[cfg(any(target_os = "linux", test))]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(windows)]
mod windows;

/// The Microsoft Store page of the OpenCL, OpenGL & Vulkan Compatibility
/// Pack, which gives Windows on ARM its OpenGL.
pub const COMPATIBILITY_PACK_PAGE: &str = "ms-windows-store://pdp/?ProductId=9NQPSL29BFFF";
/// The getting-started guide, which documents the OpenGL 2.0 requirement.
pub const GETTING_STARTED_PAGE: &str =
    "https://github.com/rwojsznis/fastsonic/blob/main/docs/_guide/getting-started.md";

/// Why no window could open, which decides the remedy.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cause {
    /// No graphics driver offers OpenGL 2: the remedy is the driver, or 3D
    /// acceleration for a virtual machine.
    NoGraphicsDriver,
    /// Windows on ARM, whatever the build's architecture (an x64 build runs
    /// there emulated). Snapdragon graphics drivers have no OpenGL of their
    /// own; Windows provides it through Microsoft's compatibility pack, a
    /// Store app that translates it to Direct3D 12.
    WindowsOnArm,
}

impl Cause {
    /// The cause on this computer.
    pub fn here() -> Self {
        Self::for_machine(windows_on_arm())
    }

    pub fn for_machine(windows_on_arm: bool) -> Self {
        if windows_on_arm {
            Self::WindowsOnArm
        } else {
            Self::NoGraphicsDriver
        }
    }
}

/// The dialog's words, and the page its first button opens.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Dialog {
    pub title: &'static str,
    /// Ends with the question the buttons answer: Windows' message box only
    /// has Yes and No, in the system's language.
    pub body: String,
    pub link: &'static str,
    /// The button that opens the link, where the platform names its buttons
    /// (macOS, and zenity on Linux).
    pub open_label: &'static str,
    pub close_label: &'static str,
}

/// The dialog for `cause`: Windows on ARM is pointed at Microsoft's
/// compatibility pack, anything else at the graphics driver and the guide.
pub fn dialog(cause: Cause) -> Dialog {
    let (body, link) = match cause {
        Cause::WindowsOnArm => (
            "On Windows on ARM, the OpenGL that Fastsonic draws with comes from \
             Microsoft's OpenCL, OpenGL & Vulkan Compatibility Pack. Open its \
             Microsoft Store page to install or repair it?"
                .to_owned(),
            COMPATIBILITY_PACK_PAGE,
        ),
        Cause::NoGraphicsDriver => (
            format!(
                "Fastsonic needs a graphics driver with OpenGL 2.0 or newer, and this PC \
                 or virtual machine does not provide one. Install the graphics driver, \
                 or turn on GPU acceleration for the virtual machine. Open \
                 {GETTING_STARTED_PAGE} to read more?"
            ),
            GETTING_STARTED_PAGE,
        ),
    };
    Dialog {
        title: "Fastsonic cannot open its window",
        body,
        link,
        open_label: "Open page",
        close_label: "Close",
    }
}

/// Whether `error` says the computer's graphics cannot draw the window:
/// glutin found no configuration or context, or the OpenGL version is below
/// 2. Window-system and app errors are not.
pub fn is_graphics_failure(error: &eframe::Error) -> bool {
    matches!(
        error,
        eframe::Error::Glutin(_) | eframe::Error::NoGlutinConfigs(..) | eframe::Error::OpenGL(_)
    )
}

/// Whether a failed `run_native` deserves the dialog. A window that started
/// (the app creator ran) had graphics that drew it, so a later failure is
/// not this one.
pub fn needs_explaining(error: &eframe::Error, window_started: bool) -> bool {
    !window_started && is_graphics_failure(error)
}

/// Shows the dialog for this computer and returns when it closes, opening
/// the page if the person asks for it.
pub fn explain() {
    let dialog = dialog(Cause::here());
    if show(&dialog)
        && let Err(error) = crate::opener::open(dialog.link)
    {
        log::warn!("cannot open {}: {error}", dialog.link);
    }
}

/// Shows the dialog and returns whether the person asked for the page.
fn show(dialog: &Dialog) -> bool {
    #[cfg(windows)]
    return windows::show(dialog);
    #[cfg(target_os = "macos")]
    return macos::show(dialog);
    #[cfg(target_os = "linux")]
    return linux::show(dialog);
    #[cfg(not(any(windows, target_os = "macos", target_os = "linux")))]
    {
        let _ = dialog;
        false
    }
}

/// Whether Windows runs on an ARM64 processor, asked at run time, so an x64
/// build running emulated there counts too.
fn windows_on_arm() -> bool {
    #[cfg(windows)]
    return windows::native_arm64();
    #[cfg(not(windows))]
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn windows_on_arm_points_at_the_store_pack() {
        assert_eq!(Cause::for_machine(true), Cause::WindowsOnArm);
        let arm = dialog(Cause::WindowsOnArm);
        assert_eq!(arm.title, "Fastsonic cannot open its window");
        assert!(
            arm.body
                .contains("OpenCL, OpenGL & Vulkan Compatibility Pack")
        );
        assert!(
            arm.body
                .ends_with("Open its Microsoft Store page to install or repair it?")
        );
        assert!(!arm.body.contains("graphics driver"));
        assert_eq!(arm.link, COMPATIBILITY_PACK_PAGE);
        assert!(arm.link.starts_with("ms-windows-store:"));
        assert_eq!((arm.open_label, arm.close_label), ("Open page", "Close"));
    }

    #[test]
    fn anything_else_points_at_the_driver_and_the_guide() {
        assert_eq!(Cause::for_machine(false), Cause::NoGraphicsDriver);
        let driver = dialog(Cause::NoGraphicsDriver);
        assert_eq!(driver.title, "Fastsonic cannot open its window");
        assert!(driver.body.contains("OpenGL 2.0"));
        assert!(driver.body.contains("virtual machine"));
        assert!(driver.body.contains(GETTING_STARTED_PAGE));
        assert!(driver.body.ends_with("to read more?"));
        assert_eq!(driver.link, GETTING_STARTED_PAGE);
        assert!(driver.link.contains("rwojsznis/fastsonic"));
        assert_eq!(
            (driver.open_label, driver.close_label),
            ("Open page", "Close")
        );
    }

    #[test]
    fn this_computer_is_not_windows_on_arm_unless_it_is_windows() {
        if !cfg!(windows) {
            assert_eq!(Cause::here(), Cause::NoGraphicsDriver);
        }
    }

    #[test]
    fn opengl_below_2_is_explained_unless_the_window_had_started() {
        let old_opengl = eframe::Error::OpenGL(eframe::egui_glow::PainterError::from(
            "egui_glow requires opengl 2.0+. ".to_owned(),
        ));
        assert!(is_graphics_failure(&old_opengl));
        assert!(needs_explaining(&old_opengl, false));
        assert!(!needs_explaining(&old_opengl, true));
    }

    #[test]
    fn app_errors_are_not_graphics_failures() {
        let app = eframe::Error::AppCreation("the library is locked".into());
        assert!(!is_graphics_failure(&app));
        assert!(!needs_explaining(&app, false));
    }
}
