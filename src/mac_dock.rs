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

#[cfg(test)]
mod tests {
    use super::*;

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
