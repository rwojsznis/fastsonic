//! macOS: Low Power Mode.
//!
//! `NSProcessInfo.isLowPowerModeEnabled` is read when the app asks for the
//! conditions rather than followed through its notification: one message
//! send, and no observer object to keep. Sleep and wake from NSWorkspace
//! are not followed yet.
//! <https://developer.apple.com/documentation/foundation/processinfo/islowpowermodeenabled>

use std::sync::Arc;

use objc2::sel;
use objc2_foundation::{NSObjectProtocol, NSProcessInfo};

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
