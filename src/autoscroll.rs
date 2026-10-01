//! Middle-click scrolling on Windows, and on Linux when the listener turns
//! it on. macOS retains its existing input.
//!
//! Views report their scroll areas; the controller applies changes after drawing.

#[cfg_attr(any(windows, target_os = "linux"), path = "autoscroll/active.rs")]
#[cfg_attr(
    not(any(windows, target_os = "linux")),
    path = "autoscroll/inactive.rs"
)]
mod platform;

pub use platform::{Autoscroll, Outcome, lyrics, playlist, row, show};

/// Whether middle-click scrolling is on: always on Windows, on Linux only by
/// choice because a middle click there usually pastes, and never on macOS.
pub fn enabled(chosen_on_linux: bool) -> bool {
    enabled_for(cfg!(windows), cfg!(target_os = "linux"), chosen_on_linux)
}

const fn enabled_for(windows: bool, linux: bool, chosen_on_linux: bool) -> bool {
    windows || (linux && chosen_on_linux)
}

#[cfg(test)]
mod choice_tests {
    use super::enabled_for;

    #[test]
    fn windows_always_autoscrolls_linux_by_choice_and_macos_never() {
        assert!(enabled_for(true, false, false));
        assert!(enabled_for(true, false, true));
        assert!(!enabled_for(false, true, false));
        assert!(enabled_for(false, true, true));
        assert!(!enabled_for(false, false, true));
        assert!(!enabled_for(false, false, false));
    }
}
