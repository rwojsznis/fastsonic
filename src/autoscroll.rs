//! Windows middle-click scrolling. Other platforms retain their existing input.
//!
//! Views report their scroll areas; the controller applies changes after drawing.

#[cfg_attr(windows, path = "autoscroll/windows.rs")]
#[cfg_attr(not(windows), path = "autoscroll/inactive.rs")]
mod platform;

pub use platform::{Autoscroll, Outcome, lyrics, playlist, row, show};
