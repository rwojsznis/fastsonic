//! The dialog as an AppKit alert.

use objc2::MainThreadMarker;
use objc2_app_kit::{
    NSAlert, NSAlertFirstButtonReturn, NSAlertStyle, NSApplication, NSApplicationActivationOptions,
    NSRunningApplication,
};
use objc2_foundation::NSString;

use super::Dialog;

pub(super) fn show(dialog: &Dialog) -> bool {
    let Some(main_thread) = MainThreadMarker::new() else {
        log::warn!("the error dialog can only be shown from the main thread");
        return false;
    };
    // An app whose window never opened may not be active yet; the alert
    // would open behind the window that was in front.
    let _ = NSApplication::sharedApplication(main_thread);
    NSRunningApplication::currentApplication()
        .activateWithOptions(NSApplicationActivationOptions::ActivateAllWindows);
    let alert = NSAlert::new(main_thread);
    alert.setAlertStyle(NSAlertStyle::Critical);
    alert.setMessageText(&NSString::from_str(dialog.title));
    alert.setInformativeText(&NSString::from_str(&dialog.body));
    alert.addButtonWithTitle(&NSString::from_str(dialog.open_label));
    alert.addButtonWithTitle(&NSString::from_str(dialog.close_label));
    alert.runModal() == NSAlertFirstButtonReturn
}
