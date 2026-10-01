//! Text drawn the way the desktop draws its own.
//!
//! What the desktop asks for (hinting strength, antialiasing, sub-pixel
//! positions) becomes a small [`TextRendering`], with how glyph coverage
//! turns into ink, and is applied to every font and both themes.
//!
//! Where the settings come from:
//!
//! - Linux: the desktop portal (`org.gnome.desktop.interface`
//!   `font-hinting` and `font-antialiasing`), then fontconfig (`fc-match`),
//!   then [`TextRendering::default`].
//! - macOS: no hinting with sub-pixel positions, as CoreText renders.
//! - Windows: slight hinting with sub-pixel positions, the nearest match to
//!   DirectWrite's natural rendering.
//!
//! egui renders grayscale coverage only, so a sub-pixel (`rgba`) setting
//! counts as plain antialiasing.
//!
//! Adapted from fastframe-text (MIT), by upstream's author.

use std::sync::OnceLock;

use egui::Visuals;
use egui::epaint::FontColorTransferFunction;
use egui::epaint::text::{FontDefinitions, FontTweak, HintingTarget, SmoothHinting};

/// How strongly glyph outlines are fitted to the pixel grid: fontconfig's
/// `hintstyle` and GNOME's `font-hinting`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Hinting {
    /// The designer's outlines, as macOS draws them.
    None,
    /// Stems fitted vertically only, keeping the font's advances: most
    /// Linux desktops' default.
    #[default]
    Slight,
    /// Fitted both ways, letting horizontal metrics snap. Same as `Full`
    /// in egui.
    Medium,
    Full,
}

/// How the rasterizer's glyph coverage becomes text alpha, which decides
/// how heavy text looks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Coverage {
    /// As is in both themes, as cairo and FreeType draw text on Linux;
    /// egui's dark default (2c - c²) draws light text on dark heavier.
    Linear,
    /// egui's choice per theme, closer to CoreText and DirectWrite.
    ThemeDefault,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TextRendering {
    pub hinting: Hinting,
    /// `false` selects the strongest hinting target, meant for 1-bit text;
    /// egui itself always antialiases.
    pub antialias: bool,
    /// Whether glyphs may start at fractional pixel positions.
    pub subpixel_positioning: bool,
    /// Set by the platform, not read from the desktop.
    pub coverage: Coverage,
}

impl Default for TextRendering {
    /// Slight hinting, antialiased, sub-pixel positions and linear
    /// coverage: what current Linux desktops render.
    fn default() -> Self {
        Self {
            hinting: Hinting::Slight,
            antialias: true,
            subpixel_positioning: true,
            coverage: Coverage::Linear,
        }
    }
}

impl TextRendering {
    /// The platform's own rendering, for when the desktop says nothing.
    pub fn platform_default() -> Self {
        if cfg!(target_os = "macos") {
            Self {
                hinting: Hinting::None,
                coverage: Coverage::ThemeDefault,
                ..Self::default()
            }
        } else if cfg!(windows) {
            Self {
                coverage: Coverage::ThemeDefault,
                ..Self::default()
            }
        } else {
            Self::default()
        }
    }

    /// Slight hinting relies on egui's default target, which keeps linear
    /// metrics; Inter has no TrueType instructions, so it renders as light
    /// hinting would.
    pub fn hinting_target(&self) -> HintingTarget {
        if !self.antialias {
            return HintingTarget::Mono;
        }
        match self.hinting {
            Hinting::None | Hinting::Slight => HintingTarget::Smooth(SmoothHinting::default()),
            Hinting::Medium | Hinting::Full => HintingTarget::Smooth(SmoothHinting {
                light: false,
                symmetric_rendering: true,
                preserve_linear_metrics: false,
            }),
        }
    }

    pub fn color_transfer_function(&self, dark_mode: bool) -> FontColorTransferFunction {
        match self.coverage {
            Coverage::Linear => FontColorTransferFunction::Off,
            Coverage::ThemeDefault if dark_mode => FontColorTransferFunction::DARK_MODE_DEFAULT,
            Coverage::ThemeDefault => FontColorTransferFunction::LIGHT_MODE_DEFAULT,
        }
    }

    /// Sets one font's hinting, leaving its scale, offsets and variation
    /// coordinates alone.
    pub fn tweak(&self, tweak: &mut FontTweak) {
        tweak.hinting = Some(self.hinting != Hinting::None);
        tweak.hinting_target = self.hinting_target();
        tweak.subpixel_binning = Some(self.subpixel_positioning);
    }

    /// Tweaks every font in `fonts`, before `set_fonts`.
    pub fn apply_to(&self, fonts: &mut FontDefinitions) {
        for data in fonts.font_data.values_mut() {
            self.tweak(&mut std::sync::Arc::make_mut(data).tweak);
        }
    }

    /// A theme's text options. Replacing the visuals resets them, so this
    /// follows every palette change.
    pub fn apply_to_visuals(&self, visuals: &mut Visuals) {
        let options = &mut visuals.text_options;
        options.font_hinting = self.hinting != Hinting::None;
        options.subpixel_binning = self.subpixel_positioning;
        options.color_transfer_function = self.color_transfer_function(visuals.dark_mode);
    }
}

/// The desktop's rendering, read once per process. On Linux the first call
/// asks the portal, then fontconfig, and can block for about a second when
/// the portal does not answer; it runs as the first window opens, outside
/// any runtime.
pub fn current() -> TextRendering {
    static CURRENT: OnceLock<TextRendering> = OnceLock::new();
    *CURRENT.get_or_init(detect)
}

fn detect() -> TextRendering {
    #[cfg(target_os = "linux")]
    {
        let rendering = portal::read()
            .or_else(|| fontconfig::read("sans-serif"))
            .unwrap_or_else(TextRendering::platform_default);
        log::info!("text rendering: {rendering:?}");
        rendering
    }
    #[cfg(not(target_os = "linux"))]
    {
        TextRendering::platform_default()
    }
}

/// The portal's font keys. Only `read` talks to the bus.
#[cfg(any(target_os = "linux", test))]
mod portal {
    use super::{Hinting, TextRendering};

    #[cfg(target_os = "linux")]
    const NAMESPACE: &str = "org.gnome.desktop.interface";

    pub(super) fn parse_hinting(value: &str) -> Option<Hinting> {
        match value.trim() {
            "none" => Some(Hinting::None),
            "slight" => Some(Hinting::Slight),
            "medium" => Some(Hinting::Medium),
            "full" => Some(Hinting::Full),
            _ => None,
        }
    }

    pub(super) fn parse_antialiasing(value: &str) -> Option<bool> {
        match value.trim() {
            "none" => Some(false),
            "grayscale" | "rgba" => Some(true),
            _ => None,
        }
    }

    /// `None` when neither key holds a known value, so fontconfig gets its
    /// turn; a missing key keeps its default.
    pub(super) fn from_settings(
        hinting: Option<&str>,
        antialiasing: Option<&str>,
    ) -> Option<TextRendering> {
        let hinting = hinting.and_then(parse_hinting);
        let antialias = antialiasing.and_then(parse_antialiasing);
        if hinting.is_none() && antialias.is_none() {
            return None;
        }
        let default = TextRendering::default();
        Some(TextRendering {
            hinting: hinting.unwrap_or(default.hinting),
            antialias: antialias.unwrap_or(default.antialias),
            ..default
        })
    }

    #[cfg(target_os = "linux")]
    pub(super) fn read() -> Option<TextRendering> {
        use std::time::Duration;
        use zbus::zvariant::OwnedValue;

        let connection = zbus::blocking::connection::Builder::session()
            .ok()?
            .method_timeout(Duration::from_secs(1))
            .build()
            .ok()?;
        let proxy = zbus::blocking::Proxy::new(
            &connection,
            "org.freedesktop.portal.Desktop",
            "/org/freedesktop/portal/desktop",
            "org.freedesktop.portal.Settings",
        )
        .ok()?;
        // `ReadOne` arrived in version 2 of the interface; `Read` wraps the
        // value in a second variant.
        let read = |key: &str| -> Option<String> {
            let value: OwnedValue = proxy
                .call("ReadOne", &(NAMESPACE, key))
                .or_else(|_| proxy.call("Read", &(NAMESPACE, key)))
                .ok()?;
            string(&value)
        };
        from_settings(
            read("font-hinting").as_deref(),
            read("font-antialiasing").as_deref(),
        )
    }

    #[cfg(target_os = "linux")]
    pub(super) fn string(value: &zbus::zvariant::Value<'_>) -> Option<String> {
        use zbus::zvariant::Value;
        match value {
            Value::Str(text) => Some(text.as_str().to_owned()),
            Value::Value(inner) => string(inner),
            _ => None,
        }
    }
}

/// Fontconfig's rendering for a family, through `fc-match`, which avoids a
/// C library at build time.
#[cfg(any(target_os = "linux", test))]
mod fontconfig {
    use super::{Hinting, TextRendering};

    #[cfg(target_os = "linux")]
    const FORMAT: &str = "%{hintstyle}|%{hinting}|%{antialias}";

    pub(super) fn parse_hintstyle(value: &str) -> Option<Hinting> {
        match value.trim() {
            "0" | "hintnone" => Some(Hinting::None),
            "1" | "hintslight" => Some(Hinting::Slight),
            "2" | "hintmedium" => Some(Hinting::Medium),
            "3" | "hintfull" => Some(Hinting::Full),
            _ => None,
        }
    }

    pub(super) fn parse_bool(value: &str) -> Option<bool> {
        match value.trim().to_ascii_lowercase().as_str() {
            "true" | "1" | "yes" => Some(true),
            "false" | "0" | "no" => Some(false),
            _ => None,
        }
    }

    /// `None` when no field holds a known value. Hinting off wins over any
    /// style.
    pub(super) fn parse_fc_match(output: &str) -> Option<TextRendering> {
        let line = output.lines().next()?;
        let mut fields = line.split('|');
        let hintstyle = fields.next().and_then(parse_hintstyle);
        let hinting = fields.next().and_then(parse_bool);
        let antialias = fields.next().and_then(parse_bool);
        if hintstyle.is_none() && hinting.is_none() && antialias.is_none() {
            return None;
        }
        let default = TextRendering::default();
        Some(TextRendering {
            hinting: match hinting {
                Some(false) => Hinting::None,
                _ => hintstyle.unwrap_or(default.hinting),
            },
            antialias: antialias.unwrap_or(default.antialias),
            ..default
        })
    }

    #[cfg(target_os = "linux")]
    pub(super) fn read(family: &str) -> Option<TextRendering> {
        let output = std::process::Command::new("fc-match")
            .arg("-f")
            .arg(FORMAT)
            .arg(family)
            .stdin(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .output()
            .ok()?;
        if !output.status.success() {
            return None;
        }
        parse_fc_match(&String::from_utf8_lossy(&output.stdout))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const EGUI_DEFAULT: HintingTarget = HintingTarget::Smooth(SmoothHinting {
        light: false,
        symmetric_rendering: true,
        preserve_linear_metrics: true,
    });

    const GRID_FIT: HintingTarget = HintingTarget::Smooth(SmoothHinting {
        light: false,
        symmetric_rendering: true,
        preserve_linear_metrics: false,
    });

    #[test]
    fn slight_hinting_is_eguis_own_default() {
        assert_eq!(HintingTarget::default(), EGUI_DEFAULT);
        assert_eq!(TextRendering::default().hinting_target(), EGUI_DEFAULT);
    }

    #[test]
    fn each_setting_maps_to_its_hinting_target() {
        let target = |hinting, antialias| {
            TextRendering {
                hinting,
                antialias,
                ..TextRendering::default()
            }
            .hinting_target()
        };
        assert_eq!(target(Hinting::None, true), EGUI_DEFAULT);
        assert_eq!(target(Hinting::Medium, true), GRID_FIT);
        assert_eq!(target(Hinting::Full, true), GRID_FIT);
        assert_eq!(target(Hinting::Slight, false), HintingTarget::Mono);
    }

    #[test]
    fn each_platform_starts_from_its_own_rendering() {
        let platform = TextRendering::platform_default();
        if cfg!(target_os = "macos") {
            assert_eq!(platform.hinting, Hinting::None);
            assert_eq!(platform.coverage, Coverage::ThemeDefault);
        } else if cfg!(windows) {
            assert_eq!(platform.hinting, Hinting::Slight);
            assert_eq!(platform.coverage, Coverage::ThemeDefault);
        } else {
            assert_eq!(platform, TextRendering::default());
        }
        assert!(platform.subpixel_positioning);
    }

    #[test]
    fn a_tweak_sets_hinting_and_keeps_everything_else() {
        let mut tweak = FontTweak {
            scale: 1.2,
            y_offset_factor: 0.1,
            ..FontTweak::default()
        };
        TextRendering {
            hinting: Hinting::None,
            subpixel_positioning: false,
            ..TextRendering::default()
        }
        .tweak(&mut tweak);
        assert_eq!(tweak.hinting, Some(false));
        assert_eq!(tweak.subpixel_binning, Some(false));
        assert_eq!((tweak.scale, tweak.y_offset_factor), (1.2, 0.1));
    }

    #[test]
    fn coverage_follows_the_platform_in_each_theme() {
        let linear = TextRendering::default();
        assert_eq!(
            linear.color_transfer_function(true),
            FontColorTransferFunction::Off
        );
        let theme = TextRendering {
            coverage: Coverage::ThemeDefault,
            ..linear
        };
        assert_eq!(
            theme.color_transfer_function(true),
            FontColorTransferFunction::DARK_MODE_DEFAULT
        );
        assert_eq!(
            theme.color_transfer_function(false),
            FontColorTransferFunction::LIGHT_MODE_DEFAULT
        );
    }

    #[test]
    fn portal_keys_read_with_defaults_for_what_is_missing() {
        assert_eq!(
            portal::from_settings(Some("slight"), Some("grayscale")),
            Some(TextRendering::default())
        );
        let full = portal::from_settings(Some("full"), Some("none")).unwrap();
        assert_eq!((full.hinting, full.antialias), (Hinting::Full, false));
        let only = portal::from_settings(Some("none"), None).unwrap();
        assert_eq!((only.hinting, only.antialias), (Hinting::None, true));
        assert!(portal::parse_antialiasing("rgba").unwrap());
        assert_eq!(portal::from_settings(None, None), None);
        assert_eq!(portal::from_settings(Some("bogus"), Some("bogus")), None);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn portal_values_unwrap_nested_variants() {
        use zbus::zvariant::Value;
        assert_eq!(
            portal::string(&Value::from("slight")).as_deref(),
            Some("slight")
        );
        let nested = Value::Value(Box::new(Value::from("full")));
        assert_eq!(portal::string(&nested).as_deref(), Some("full"));
        assert_eq!(portal::string(&Value::from(3u32)), None);
    }

    #[test]
    fn fc_match_output_reads_with_hinting_off_winning() {
        assert_eq!(
            fontconfig::parse_fc_match("1|True|True"),
            Some(TextRendering::default())
        );
        let full = fontconfig::parse_fc_match("3|True|False").unwrap();
        assert_eq!((full.hinting, full.antialias), (Hinting::Full, false));
        let off = fontconfig::parse_fc_match("3|False|True").unwrap();
        assert_eq!(off.hinting, Hinting::None);
        let medium = fontconfig::parse_fc_match("2").unwrap();
        assert_eq!(medium.hinting, Hinting::Medium);
        assert_eq!(
            fontconfig::parse_hintstyle("hintslight"),
            Some(Hinting::Slight)
        );
        assert_eq!(fontconfig::parse_bool("DontCare"), None);
        assert_eq!(fontconfig::parse_fc_match(""), None);
        assert_eq!(fontconfig::parse_fc_match("Fontconfig error"), None);
    }
}
