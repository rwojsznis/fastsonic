//! Where each control sits in the main window, in skin pixels.
//!
//! The positions are Winamp's own, via Webamp's `main-window.css`; every
//! classic skin paints its background to match them.

/// A rectangle in the window, in skin pixels.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Area {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

impl Area {
    pub const fn new(x: u32, y: u32, width: u32, height: u32) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }

    pub fn contains(&self, x: u32, y: u32) -> bool {
        x >= self.x && y >= self.y && x < self.x + self.width && y < self.y + self.height
    }
}

pub const WINDOW_WIDTH: u32 = 275;
pub const WINDOW_HEIGHT: u32 = 116;
/// The window's height in shade mode, when only the title bar shows.
pub const SHADE_HEIGHT: u32 = 14;

macro_rules! areas {
    ($($(#[$attr:meta])* $name:ident = ($x:expr, $y:expr, $w:expr, $h:expr);)*) => {
        $($(#[$attr])* pub const $name: Area = Area::new($x, $y, $w, $h);)*
        /// Every control, so the layout can be checked against the window.
        pub const ALL: &[(&str, Area)] = &[$((stringify!($name), $name),)*];
    };
}

areas! {
    TITLE_BAR = (0, 0, 275, 14);
    OPTIONS_BUTTON = (6, 3, 9, 9);
    MINIMIZE_BUTTON = (244, 3, 9, 9);
    SHADE_BUTTON = (254, 3, 9, 9);
    CLOSE_BUTTON = (264, 3, 9, 9);
    CLUTTER_BAR = (10, 22, 8, 43);
    CLUTTER_O = (10, 25, 8, 8);
    CLUTTER_A = (10, 33, 8, 7);
    CLUTTER_I = (10, 40, 8, 7);
    CLUTTER_D = (10, 47, 8, 8);
    CLUTTER_V = (10, 55, 8, 7);
    WORK_INDICATOR = (24, 28, 3, 9);
    STATUS = (26, 28, 9, 9);
    /// The minus sign when the skin has `nums_ex`: a full digit cell.
    MINUS_EX = (36, 26, 9, 13);
    /// The minus sign borrowed from the digit sheet: one row of pixels.
    MINUS = (38, 32, 5, 1);
    MINUTE_TENS = (48, 26, 9, 13);
    MINUTE_ONES = (60, 26, 9, 13);
    SECOND_TENS = (78, 26, 9, 13);
    SECOND_ONES = (90, 26, 9, 13);
    MARQUEE = (111, 27, 154, 6);
    KBPS = (111, 43, 15, 6);
    KHZ = (156, 43, 10, 6);
    VISUALIZER = (24, 43, 76, 16);
    MONO = (212, 41, 27, 12);
    STEREO = (239, 41, 29, 12);
    VOLUME = (107, 57, 68, 13);
    BALANCE = (177, 57, 38, 13);
    EQ_BUTTON = (219, 58, 23, 12);
    PLAYLIST_BUTTON = (242, 58, 23, 12);
    POSITION = (16, 72, 248, 10);
    PREVIOUS = (16, 88, 23, 18);
    PLAY = (39, 88, 23, 18);
    PAUSE = (62, 88, 23, 18);
    STOP = (85, 88, 23, 18);
    NEXT = (108, 88, 22, 18);
    EJECT = (136, 89, 22, 16);
    SHUFFLE = (164, 89, 47, 15);
    REPEAT = (210, 89, 28, 15);
    ABOUT = (253, 91, 13, 15);
}

/// The digit cells of the time display, left to right.
pub const TIME_DIGITS: [Area; 4] = [MINUTE_TENS, MINUTE_ONES, SECOND_TENS, SECOND_ONES];

/// How far a slider's thumb can travel: the track's width less the thumb's.
pub const VOLUME_TRAVEL: u32 = VOLUME.width - 14;
pub const BALANCE_TRAVEL: u32 = BALANCE.width - 14;
pub const POSITION_TRAVEL: u32 = POSITION.width - 29;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_control_fits_inside_the_window() {
        for (name, area) in ALL {
            assert!(
                area.x + area.width <= WINDOW_WIDTH && area.y + area.height <= WINDOW_HEIGHT,
                "{name} leaves the window"
            );
        }
    }

    #[test]
    fn the_transport_row_is_contiguous() {
        assert_eq!(PREVIOUS.x + PREVIOUS.width, PLAY.x);
        assert_eq!(PLAY.x + PLAY.width, PAUSE.x);
        assert_eq!(PAUSE.x + PAUSE.width, STOP.x);
        assert_eq!(STOP.x + STOP.width, NEXT.x);
    }

    #[test]
    fn hit_testing_uses_half_open_edges() {
        assert!(PLAY.contains(39, 88));
        assert!(PLAY.contains(61, 105));
        assert!(!PLAY.contains(62, 88));
        assert!(!PLAY.contains(39, 106));
    }
}
