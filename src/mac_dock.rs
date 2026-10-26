//! The macOS Dock icon, apart from its menu (`mac_menu`).
//!
//! AppKit is asked only once winit's event loop exists: winit owns the
//! application object and refuses to start if anything made it first.

use objc2::MainThreadMarker;
use objc2_app_kit::NSApplication;

/// The packaged app's artwork, the one its bundle's icon is made from.
const ICON: &[u8] = include_bytes!("../packaging/macos/icon-1024.png");

/// Gives a run outside an app bundle (`cargo run`) the packaged app's Dock
/// icon instead of the generic executable's. A bundle's own icon is left
/// alone. Once per process, from the first window.
pub fn use_packaged_icon() {
    static DONE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
    if DONE.swap(true, std::sync::atomic::Ordering::SeqCst) {
        return;
    }
    use objc2::AnyThread;
    use objc2_app_kit::NSImage;
    use objc2_foundation::{NSBundle, NSData};

    let Some(mtm) = MainThreadMarker::new() else {
        return;
    };
    let bundle = NSBundle::mainBundle().bundlePath().to_string();
    if is_app_bundle(std::path::Path::new(&bundle)) {
        return;
    }
    let data = NSData::with_bytes(ICON);
    let Some(image) = NSImage::initWithData(NSImage::alloc(), &data) else {
        log::warn!("the Dock icon's artwork could not be read");
        return;
    };
    // SAFETY: on the main thread, as the marker proves; the application
    // keeps the image.
    unsafe { NSApplication::sharedApplication(mtm).setApplicationIconImage(Some(&image)) };
}

fn is_app_bundle(path: &std::path::Path) -> bool {
    path.extension().is_some_and(|extension| extension == "app")
}

/// Whether the Dock shows Fastsonic. Settings can take it out, but only
/// while the menu-bar item exists, which is then the way back to a closed
/// window. There is no setting to hide that item, so in practice only a
/// capture run, which has no item, keeps the icon anyway.
pub fn shows_in_dock(setting: bool, menu_bar_item: bool) -> bool {
    setting || !menu_bar_item
}

/// Shows or hides the Dock icon while the app runs, switching AppKit's
/// activation policy: `Accessory` is a menu-bar app, out of the Dock and
/// Cmd-Tab and without a menu bar of its own while its window keeps the
/// focus and its key equivalents; `Regular` is an ordinary app. An app
/// that was active is activated again after the switch, since leaving the
/// Dock deactivates it and sends its window behind the others, and
/// returning leaves the menu bar with the app before. Returns whether the
/// policy changed.
pub fn set_dock_icon_visible(visible: bool) -> bool {
    use objc2_app_kit::NSApplicationActivationPolicy as Native;

    let Some(mtm) = MainThreadMarker::new() else {
        return false;
    };
    let app = NSApplication::sharedApplication(mtm);
    let current = match app.activationPolicy() {
        Native::Regular => Policy::Regular,
        Native::Accessory => Policy::Accessory,
        _ => Policy::Other,
    };
    let Some(switch) = dock_switch(visible, current, app.isActive()) else {
        return false;
    };
    let policy = match switch.policy {
        Policy::Accessory => Native::Accessory,
        Policy::Regular | Policy::Other => Native::Regular,
    };
    if !app.setActivationPolicy(policy) {
        log::warn!(
            "AppKit refused to {} the Dock icon",
            if visible { "show" } else { "hide" }
        );
        return false;
    }
    if switch.reactivate {
        #[allow(deprecated)]
        app.activateIgnoringOtherApps(true);
    }
    true
}

/// AppKit's activation policies, as far as the Dock icon is concerned.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Policy {
    Regular,
    Accessory,
    /// `Prohibited`, which no window of this app ever runs under.
    Other,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Switch {
    policy: Policy,
    /// Activate the app again afterwards, because it was active.
    reactivate: bool,
}

/// The switch from `current` to the Dock icon `visible`, or `None` when the
/// policy already matches.
fn dock_switch(visible: bool, current: Policy, active: bool) -> Option<Switch> {
    let policy = if visible {
        Policy::Regular
    } else {
        Policy::Accessory
    };
    (current != policy).then_some(Switch {
        policy,
        reactivate: active,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Settings takes Fastsonic out of the Dock only while its menu-bar
    /// item is there to bring a closed window back.
    #[test]
    fn the_dock_icon_goes_only_while_the_menu_bar_item_stays() {
        assert!(shows_in_dock(true, true));
        assert!(!shows_in_dock(false, true));
        assert!(shows_in_dock(false, false), "no way back without it");
        assert!(shows_in_dock(true, false));
    }

    #[test]
    fn hiding_the_dock_icon_makes_a_menu_bar_app_and_keeps_it_in_front() {
        assert_eq!(
            dock_switch(false, Policy::Regular, true),
            Some(Switch {
                policy: Policy::Accessory,
                reactivate: true
            })
        );
        assert_eq!(
            dock_switch(true, Policy::Accessory, true),
            Some(Switch {
                policy: Policy::Regular,
                reactivate: true
            })
        );
        assert_eq!(
            dock_switch(true, Policy::Other, false),
            Some(Switch {
                policy: Policy::Regular,
                reactivate: false
            })
        );
    }

    #[test]
    fn a_matching_policy_is_left_alone() {
        assert_eq!(dock_switch(true, Policy::Regular, true), None);
        assert_eq!(dock_switch(false, Policy::Accessory, false), None);
    }

    #[test]
    fn only_an_app_bundle_brings_its_own_icon() {
        use std::path::Path;
        assert!(is_app_bundle(Path::new("/Applications/Fastsonic.app")));
        assert!(!is_app_bundle(Path::new("/code/fastsonic/target/release")));
        assert!(!is_app_bundle(Path::new("/code/app")));
    }

    #[test]
    fn the_artwork_is_a_png() {
        assert!(ICON.starts_with(b"\x89PNG\r\n\x1a\n"));
    }
}
