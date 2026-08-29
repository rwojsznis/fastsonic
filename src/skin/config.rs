//! The two text files in a skin: `pledit.txt` and `viscolor.txt`.
//!
//! Both are forgiving formats in practice. Colours arrive with or without a
//! `#`, sometimes with too many digits; the visualiser's list comes with
//! trailing commas, tabs, and comments after each line. Anything that cannot
//! be read keeps its default, as Winamp did.

pub type Rgb = [u8; 3];

/// Colours and font for the playlist editor.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlaylistStyle {
    pub normal: Rgb,
    pub current: Rgb,
    pub normal_background: Rgb,
    pub selected_background: Rgb,
    pub font: String,
}

impl Default for PlaylistStyle {
    fn default() -> Self {
        Self {
            normal: [0x00, 0xff, 0x00],
            current: [0xff, 0xff, 0xff],
            normal_background: [0x00, 0x00, 0x00],
            selected_background: [0x00, 0x00, 0xff],
            font: "Arial".to_string(),
        }
    }
}

impl PlaylistStyle {
    /// Reads `pledit.txt`: an INI file with a `[Text]` section.
    pub fn parse(text: &str) -> Self {
        let mut style = Self::default();
        let mut in_text = false;
        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with(';') {
                continue;
            }
            if let Some(section) = line.strip_prefix('[') {
                in_text = section
                    .strip_suffix(']')
                    .unwrap_or(section)
                    .trim()
                    .eq_ignore_ascii_case("text");
                continue;
            }
            if !in_text {
                continue;
            }
            let Some((key, value)) = line.split_once('=') else {
                continue;
            };
            let value = value.trim();
            let key = key.trim().to_ascii_lowercase();
            match key.as_str() {
                "normal" => style.normal = hex(value).unwrap_or(style.normal),
                "current" => style.current = hex(value).unwrap_or(style.current),
                "normalbg" => {
                    style.normal_background = hex(value).unwrap_or(style.normal_background)
                }
                "selectedbg" => {
                    style.selected_background = hex(value).unwrap_or(style.selected_background)
                }
                "font" if !value.is_empty() => style.font = value.to_string(),
                _ => {}
            }
        }
        style
    }
}

/// A `#RRGGBB` colour. Winamp accepted the hash missing and extra digits
/// after the six, so both are taken here.
pub fn hex(value: &str) -> Option<Rgb> {
    let digits = value.trim().trim_start_matches('#');
    if digits.len() < 6 || !digits.is_char_boundary(6) {
        return None;
    }
    let channel = |at: usize| u8::from_str_radix(&digits[at..at + 2], 16).ok();
    Some([channel(0)?, channel(2)?, channel(4)?])
}

/// The visualiser's palette: background, dot grid, 16 spectrum bands from
/// the top down, then five colours for the oscilloscope and one for the
/// analyser's peaks.
pub type VisColors = [Rgb; 24];

/// A palette that suits nothing in particular, for skins without one.
pub const DEFAULT_VIS_COLORS: VisColors = [
    [0, 0, 0],
    [24, 33, 41],
    [239, 49, 16],
    [206, 41, 16],
    [214, 90, 0],
    [214, 102, 0],
    [214, 115, 0],
    [198, 123, 8],
    [222, 165, 24],
    [214, 181, 33],
    [189, 222, 41],
    [148, 222, 33],
    [41, 206, 16],
    [50, 190, 16],
    [57, 181, 16],
    [49, 156, 8],
    [41, 148, 0],
    [24, 132, 8],
    [255, 255, 255],
    [214, 214, 222],
    [181, 189, 189],
    [160, 170, 175],
    [148, 156, 165],
    [150, 150, 150],
];

/// Reads `viscolor.txt`: one `r,g,b` per line, in order. Missing lines keep
/// the default colour at that position.
pub fn parse_vis_colors(text: &str) -> VisColors {
    let mut colors = DEFAULT_VIS_COLORS;
    let parsed = text.lines().filter_map(rgb_line).take(colors.len());
    for (slot, color) in colors.iter_mut().zip(parsed) {
        *slot = color;
    }
    colors
}

/// The leading `r,g,b` of a line, ignoring whatever follows it.
fn rgb_line(line: &str) -> Option<Rgb> {
    let mut channels = line
        .split(',')
        .map(str::trim)
        .map(|field| {
            field
                .split(|c: char| !c.is_ascii_digit())
                .next()
                .unwrap_or("")
        })
        .map(|digits| digits.parse::<u32>().ok().map(|value| value.min(255) as u8));
    Some([channels.next()??, channels.next()??, channels.next()??])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_typical_pledit_file_is_read() {
        let style = PlaylistStyle::parse(
            "[Text]\r\nNormal=#A0B0C0\r\nCurrent=#FFFFFF\r\nNormalBG=#101010\r\nSelectedBG=#0000FF\r\nFont=Tahoma\r\n",
        );
        assert_eq!(style.normal, [0xa0, 0xb0, 0xc0]);
        assert_eq!(style.normal_background, [0x10, 0x10, 0x10]);
        assert_eq!(style.font, "Tahoma");
    }

    #[test]
    fn comments_missing_hashes_and_long_values_are_tolerated() {
        let style = PlaylistStyle::parse(
            "; Do not change anything!\n[text]\nnormal=00ff00\ncurrent = #FFFFFF00\nnormalbg=#zz0000\n",
        );
        assert_eq!(style.normal, [0x00, 0xff, 0x00]);
        assert_eq!(style.current, [0xff, 0xff, 0xff]);
        assert_eq!(
            style.normal_background,
            PlaylistStyle::default().normal_background
        );
    }

    #[test]
    fn keys_outside_the_text_section_are_ignored() {
        let style = PlaylistStyle::parse("[Other]\nNormal=#123456\n");
        assert_eq!(style, PlaylistStyle::default());
    }

    #[test]
    fn vis_colours_survive_comments_tabs_and_trailing_commas() {
        let colors = parse_vis_colors(
            "38,15,58         // 0\n0,0,0,\n255,255,255\t// 2 = peak\n  7 , 8 , 9 ,// spaced\n",
        );
        assert_eq!(colors[0], [38, 15, 58]);
        assert_eq!(colors[1], [0, 0, 0]);
        assert_eq!(colors[2], [255, 255, 255]);
        assert_eq!(colors[3], [7, 8, 9]);
        assert_eq!(colors[4], DEFAULT_VIS_COLORS[4]);
    }

    #[test]
    fn lines_without_three_numbers_are_skipped_not_misread() {
        let colors = parse_vis_colors("; palette\n\n300,1,2\n");
        assert_eq!(colors[0], [255, 1, 2]);
        assert_eq!(colors[1], DEFAULT_VIS_COLORS[1]);
    }
}
