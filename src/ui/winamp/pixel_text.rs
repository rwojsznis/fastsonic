//! Text the way Winamp's playlist drew it: the small system face, hinted
//! to the pixel grid and filled with no smoothing, then scaled up pixel for
//! pixel like the rest of the skin.
//!
//! egui rasterises text smoothly for the screen it is on, which is right
//! everywhere else in the app and wrong here: a playlist of 1998 was Arial
//! at eight points on a screen with no smoothing to offer. So each line is
//! drawn once at the skin's own size, from skrifa's outlines under
//! monochrome hinting and tiny-skia's fill with anti-aliasing off, both in
//! the binary already for other reasons, and kept as a texture drawn
//! nearest-sampled at the window's scale, so its pixels are the skin's.

use std::collections::HashMap;
use std::sync::LazyLock;

use egui::{Color32, ColorImage, TextureHandle, TextureOptions};
use skrifa::MetadataProvider as _;
use skrifa::instance::{LocationRef, Size};
use skrifa::outline::{DrawSettings, HintingInstance, OutlinePen, Target};

/// Eight points at 96 dpi, as Windows rounded it.
pub const SIZE_PX: u32 = 11;
/// Lines kept before the cache is emptied; a queue is far shorter.
const CACHE_LIMIT: usize = 512;

/// A line of text as a bitmap the size it would be on a screen of skin
/// pixels: white ink on nothing, tinted when drawn.
pub struct Line {
    pub texture: TextureHandle,
    pub width: u32,
    pub height: u32,
}

/// The face: Arial or what stands in for it on this desktop, else Inter.
struct Face {
    bytes: &'static [u8],
    index: u32,
}

static FACE: LazyLock<Face> = LazyLock::new(|| match crate::system_fonts::pledit_face() {
    Some(face) => Face {
        bytes: &face.bytes,
        index: face.index,
    },
    None => Face {
        bytes: include_bytes!("../../../assets/fonts/InterVariable.ttf"),
        index: 0,
    },
});

fn font() -> Option<skrifa::FontRef<'static>> {
    skrifa::FontRef::from_index(FACE.bytes, FACE.index).ok()
}

/// Lines drawn so far, and the hinting programme they were drawn with.
#[derive(Default)]
pub struct PixelText {
    lines: HashMap<String, Line>,
    hinting: Option<HintingInstance>,
}

impl PixelText {
    /// Drops every texture, for when the window they belong to is gone.
    pub fn clear(&mut self) {
        self.lines.clear();
    }

    /// How wide a line would be, in skin pixels, from the face's advances.
    pub fn width(&self, text: &str) -> f32 {
        let Some(font) = font() else {
            return 0.0;
        };
        let size = Size::new(SIZE_PX as f32);
        let charmap = font.charmap();
        let glyphs = font.glyph_metrics(size, LocationRef::default());
        text.chars()
            .map(|character| {
                charmap
                    .map(character)
                    .and_then(|glyph| glyphs.advance_width(glyph))
                    .unwrap_or(0.0)
                    .round()
            })
            .sum()
    }

    /// A line, drawn now if it has not been.
    pub fn line(&mut self, ctx: &egui::Context, text: &str) -> &Line {
        if self.lines.len() >= CACHE_LIMIT {
            self.lines.clear();
        }
        if !self.lines.contains_key(text) {
            let image = self.rasterise(text);
            let [width, height] = image.size;
            let texture =
                ctx.load_texture(format!("pledit:{text}"), image, TextureOptions::NEAREST);
            self.lines.insert(
                text.to_string(),
                Line {
                    texture,
                    width: width as u32,
                    height: height as u32,
                },
            );
        }
        &self.lines[text]
    }

    fn rasterise(&mut self, text: &str) -> ColorImage {
        let Some(font) = font() else {
            return ColorImage::filled([1, 1], Color32::TRANSPARENT);
        };
        let size = Size::new(SIZE_PX as f32);
        let location = LocationRef::default();
        let metrics = font.metrics(size, location);
        let ascent = metrics.ascent.ceil();
        let height = (ascent + (-metrics.descent).ceil()).max(1.0) as u32;
        let charmap = font.charmap();
        let glyph_metrics = font.glyph_metrics(size, location);
        let outlines = font.outline_glyphs();
        if self.hinting.is_none() {
            self.hinting = HintingInstance::new(&outlines, size, location, Target::Mono).ok();
        }

        let mut paths = Vec::new();
        let mut x = 0.0f32;
        for character in text.chars() {
            let Some(glyph) = charmap.map(character).or_else(|| charmap.map('?')) else {
                continue;
            };
            let mut advance = glyph_metrics.advance_width(glyph).unwrap_or(0.0);
            if let Some(outline) = outlines.get(glyph) {
                let mut pen = Pen::new(x, ascent);
                let drawn = match &self.hinting {
                    Some(hinting) => outline.draw(DrawSettings::hinted(hinting, false), &mut pen),
                    None => outline.draw(DrawSettings::unhinted(size, location), &mut pen),
                };
                if let Ok(adjusted) = drawn
                    && let Some(hinted) = adjusted.advance_width
                {
                    advance = hinted;
                }
                if let Some(path) = pen.builder.finish() {
                    paths.push(path);
                }
            }
            x += advance.round();
        }

        let width = (x.ceil() as u32).max(1);
        let mut image = ColorImage::filled([width as usize, height as usize], Color32::TRANSPARENT);
        let Some(mut pixmap) = tiny_skia::Pixmap::new(width, height) else {
            return image;
        };
        let mut paint = tiny_skia::Paint {
            anti_alias: false,
            ..tiny_skia::Paint::default()
        };
        paint.set_color_rgba8(255, 255, 255, 255);
        for path in &paths {
            pixmap.fill_path(
                path,
                &paint,
                tiny_skia::FillRule::Winding,
                tiny_skia::Transform::identity(),
                None,
            );
        }
        for y in 0..height {
            for x in 0..width {
                if pixmap.pixel(x, y).is_some_and(|pixel| pixel.alpha() > 0) {
                    image[(x as usize, y as usize)] = Color32::WHITE;
                }
            }
        }
        image
    }
}

/// Collects an outline into a path, moved to its place on the line and
/// turned the right way up.
struct Pen {
    builder: tiny_skia::PathBuilder,
    x: f32,
    baseline: f32,
}

impl Pen {
    fn new(x: f32, baseline: f32) -> Self {
        Self {
            builder: tiny_skia::PathBuilder::new(),
            x,
            baseline,
        }
    }

    fn at(&self, x: f32, y: f32) -> (f32, f32) {
        (self.x + x, self.baseline - y)
    }
}

impl OutlinePen for Pen {
    fn move_to(&mut self, x: f32, y: f32) {
        let (x, y) = self.at(x, y);
        self.builder.move_to(x, y);
    }

    fn line_to(&mut self, x: f32, y: f32) {
        let (x, y) = self.at(x, y);
        self.builder.line_to(x, y);
    }

    fn quad_to(&mut self, cx0: f32, cy0: f32, x: f32, y: f32) {
        let (cx0, cy0) = self.at(cx0, cy0);
        let (x, y) = self.at(x, y);
        self.builder.quad_to(cx0, cy0, x, y);
    }

    fn curve_to(&mut self, cx0: f32, cy0: f32, cx1: f32, cy1: f32, x: f32, y: f32) {
        let (cx0, cy0) = self.at(cx0, cy0);
        let (cx1, cy1) = self.at(cx1, cy1);
        let (x, y) = self.at(x, y);
        self.builder.cubic_to(cx0, cy0, cx1, cy1, x, y);
    }

    fn close(&mut self) {
        self.builder.close();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_line_is_ink_on_nothing_at_the_skin_size() {
        let mut text = PixelText::default();
        let image = text.rasterise("Bonobo - Rosewood");
        assert!(image.size[1] >= SIZE_PX as usize && image.size[1] <= 2 * SIZE_PX as usize);
        let ink = image
            .pixels
            .iter()
            .filter(|pixel| **pixel == Color32::WHITE)
            .count();
        assert!(ink > 40, "only {ink} pixels of ink");
        assert!(
            image
                .pixels
                .iter()
                .all(|pixel| *pixel == Color32::WHITE || *pixel == Color32::TRANSPARENT),
            "a pixel is neither ink nor nothing"
        );
        let blank = text.rasterise("");
        assert_eq!(blank.size[0], 1);
        assert!(
            blank
                .pixels
                .iter()
                .all(|pixel| *pixel == Color32::TRANSPARENT)
        );
    }

    #[test]
    fn width_grows_with_the_text_and_matches_the_drawing_roughly() {
        let mut text = PixelText::default();
        let short = text.width("Otomo");
        let long = text.width("Khruangbin - Otomo");
        assert!(long > short && short > 0.0);
        let drawn = text.rasterise("Khruangbin - Otomo").size[0] as f32;
        assert!(
            (drawn - long).abs() <= long * 0.15,
            "drawn {drawn}, measured {long}"
        );
    }
}
