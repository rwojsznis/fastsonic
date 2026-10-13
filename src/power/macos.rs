//! macOS: Low Power Mode, and App Nap activities.
//!
//! - `NSProcessInfo.isLowPowerModeEnabled` is read when the app asks for
//!   the conditions rather than followed through its notification: one
//!   message send, and no observer object to keep.
//! - An activity from `beginActivityWithOptions:reason:` keeps playback out
//!   of App Nap.
//!
//! Sleep and wake from NSWorkspace are not followed yet.
//! <https://developer.apple.com/documentation/foundation/processinfo/islowpowermodeenabled>

use std::sync::Arc;

use objc2::rc::Retained;
use objc2::runtime::ProtocolObject;
use objc2::sel;
use objc2_foundation::{NSActivityOptions, NSObjectProtocol, NSProcessInfo, NSString};

use super::Shared;

/// Nothing to register: [`read_low_power`] reads what is needed.
pub(super) struct Reader;

impl Reader {
    pub(super) fn start(shared: Arc<Shared>) -> Result<Self, String> {
        read_low_power(&shared);
        Ok(Self)
    }
}

/// Reads Low Power Mode. It arrived in macOS 12, where asking an older
/// system would raise an exception, so the selector is checked first.
pub(super) fn read_low_power(shared: &Shared) {
    let info = NSProcessInfo::processInfo();
    if !info.respondsToSelector(sel!(isLowPowerModeEnabled)) {
        return;
    }
    let saver = info.isLowPowerModeEnabled();
    shared.update(|conditions| conditions.saver = saver);
}

/// A user-initiated activity that still allows idle system sleep, ended
/// when dropped.
pub(super) struct Activity(Retained<ProtocolObject<dyn NSObjectProtocol>>);

// SAFETY: the token is an opaque object that is only ever handed back to
// `endActivity:`, once, in `drop`. NSProcessInfo is thread-safe (objc2 marks
// it Send and Sync), and retaining and releasing an object is atomic, so
// the token may be ended and released on any thread.
unsafe impl Send for Activity {}
// SAFETY: as above; a shared reference gives no access to the token.
unsafe impl Sync for Activity {}

impl Activity {
    pub(super) fn begin(reason: &str) -> Self {
        Self(
            NSProcessInfo::processInfo().beginActivityWithOptions_reason(
                NSActivityOptions::UserInitiatedAllowingIdleSystemSleep,
                &NSString::from_str(reason),
            ),
        )
    }
}

impl Drop for Activity {
    fn drop(&mut self) {
        // SAFETY: the token came from `beginActivityWithOptions:reason:` and
        // is ended exactly once, here, before its reference is released.
        unsafe { NSProcessInfo::processInfo().endActivity(&self.0) };
    }
}
