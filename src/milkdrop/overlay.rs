//! Fading text over the picture: the keys on `?`, the song on a change.
//!
//! MilkDrop overlaid its help and the playing title on the picture and let
//! them fade; this does the same. Text is drawn on the CPU with skrifa and
//! tiny-skia, the same faces the app borrows from the system elsewhere, into
//! one bitmap with its shadow baked in, and the bitmap rides a textured quad
//! blended over the frame projectM just drew.

use std::sync::LazyLock;
use std::time::{Duration, Instant};

use eframe::glow;
use eframe::glow::HasContext as _;
use skrifa::MetadataProvider as _;
use skrifa::instance::{LocationRef, Size};
use skrifa::outline::{DrawSettings, OutlinePen};

/// How long the fade at the end of an overlay's stay takes.
const FADE: Duration = Duration::from_millis(700);
/// Space between the text and the window's edge, in logical pixels.
const MARGIN: f32 = 20.0;
/// Space around the text inside its bitmap, room for the shadow too.
const PAD: u32 = 4;
/// The shadow's offset, right and down.
const SHADOW: u32 = 2;

/// The faces, in the order they are asked: Inter, the emoji face, then
/// what the system lends for scripts those cannot draw.
static FACES: LazyLock<Vec<(&'static [u8], u32)>> = LazyLock::new(|| {
    let mut faces: Vec<(&'static [u8], u32)> = vec![
        (include_bytes!("../../assets/fonts/InterVariable.ttf"), 0),
        (include_bytes!("../../assets/fonts/NotoEmoji.ttf"), 0),
    ];
    for fallback in crate::system_fonts::fallbacks() {
        faces.push((&fallback.bytes, fallback.index));
    }
    faces
});

/// Where an overlay sits in the window.
#[derive(Clone, Copy, PartialEq)]
pub enum Place {
    TopLeft,
    BottomLeft,
}

/// A line of text and the pixel size to draw it at.
pub struct TextLine {
    pub text: String,
    pub px: f32,
}

/// White ink coverage, one byte a pixel, row zero on top.
struct Raster {
    width: u32,
    height: u32,
    alpha: Vec<u8>,
}

/// One line's coverage, anti-aliased, at `px` tall type.
fn line_raster(text: &str, px: f32) -> Raster {
    let size = Size::new(px);
    let location = LocationRef::default();
    let fonts: Vec<skrifa::FontRef<'static>> = FACES
        .iter()
        .filter_map(|(bytes, index)| skrifa::FontRef::from_index(bytes, *index).ok())
        .collect();
    let empty = Raster {
        width: 1,
        height: px.ceil() as u32,
        alpha: vec![0; px.ceil() as usize],
    };
    let Some(primary) = fonts.first() else {
        return empty;
    };
    let metrics = primary.metrics(size, location);
    let ascent = metrics.ascent.ceil();
    let height = (ascent + (-metrics.descent).ceil()).max(1.0) as u32;

    // The first face that has a character draws it; a question mark from
    // the first face stands in for what none of them have.
    let glyph_for = |character: char| {
        fonts
            .iter()
            .enumerate()
            .find_map(|(index, font)| font.charmap().map(character).map(|glyph| (index, glyph)))
            .or_else(|| fonts.first()?.charmap().map('?').map(|glyph| (0, glyph)))
    };

    let mut paths = Vec::new();
    let mut x = 0.0f32;
    for character in text.chars() {
        let Some((index, glyph)) = glyph_for(character) else {
            continue;
        };
        let font = &fonts[index];
        let advance = font
            .glyph_metrics(size, location)
            .advance_width(glyph)
            .unwrap_or(0.0);
        if let Some(outline) = font.outline_glyphs().get(glyph) {
            let mut pen = Pen::new(x, ascent);
            if outline
                .draw(DrawSettings::unhinted(size, location), &mut pen)
                .is_ok()
                && let Some(path) = pen.builder.finish()
            {
                paths.push(path);
            }
        }
        x += advance;
    }
    let width = (x.ceil() as u32).max(1);
    let Some(mut pixmap) = tiny_skia::Pixmap::new(width, height) else {
        return empty;
    };
    let mut paint = tiny_skia::Paint::default();
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
    let alpha = pixmap.pixels().iter().map(|pixel| pixel.alpha()).collect();
    Raster {
        width,
        height,
        alpha,
    }
}

/// The lines stacked into one RGBA bitmap, shadow first, ink on top.
fn block_rgba(lines: &[TextLine]) -> (u32, u32, Vec<u8>) {
    let rasters: Vec<Raster> = lines
        .iter()
        .map(|line| line_raster(&line.text, line.px))
        .collect();
    let width = rasters.iter().map(|r| r.width).max().unwrap_or(1) + PAD * 2 + SHADOW;
    let height = rasters.iter().map(|r| r.height).sum::<u32>() + PAD * 2 + SHADOW;
    let mut rgba = vec![0u8; (width * height * 4) as usize];
    // Straight alpha, composited over: the shadow's black, then the ink.
    let mut blend = |x: u32, y: u32, ink: [u8; 3], a: u8| {
        if a == 0 || x >= width || y >= height {
            return;
        }
        let at = ((y * width + x) * 4) as usize;
        let src_a = a as u32;
        let dst_a = rgba[at + 3] as u32;
        let out_a = src_a + dst_a * (255 - src_a) / 255;
        if out_a == 0 {
            return;
        }
        for channel in 0..3 {
            let src = ink[channel] as u32 * src_a;
            let dst = rgba[at + channel] as u32 * dst_a * (255 - src_a) / 255;
            rgba[at + channel] = ((src + dst) / out_a) as u8;
        }
        rgba[at + 3] = out_a as u8;
    };
    for pass in 0..2 {
        let mut top = PAD;
        for raster in &rasters {
            for y in 0..raster.height {
                for x in 0..raster.width {
                    let a = raster.alpha[(y * raster.width + x) as usize];
                    match pass {
                        0 => blend(PAD + x + SHADOW, top + y + SHADOW, [0, 0, 0], a),
                        _ => blend(PAD + x, top + y, [255, 255, 255], a),
                    }
                }
            }
            top += raster.height;
        }
    }
    (width, height, rgba)
}

/// The overlay: one bitmap at a time, drawn until its stay runs out.
pub struct Overlay {
    program: glow::Program,
    vao: glow::VertexArray,
    vbo: glow::Buffer,
    texture: glow::Texture,
    alpha_at: glow::UniformLocation,
    size: (u32, u32),
    place: Place,
    shown: Option<(Instant, Duration)>,
}

impl Overlay {
    pub fn new(gl: &glow::Context) -> Option<Self> {
        // SAFETY: the context is current; everything made here lives as
        // long as the window's context, which dies with the process.
        unsafe {
            let program = gl.create_program().ok()?;
            let sources = [
                (
                    glow::VERTEX_SHADER,
                    "#version 330 core\n\
                     layout(location = 0) in vec4 quad;\n\
                     out vec2 v_uv;\n\
                     void main() {\n\
                         v_uv = quad.zw;\n\
                         gl_Position = vec4(quad.xy, 0.0, 1.0);\n\
                     }",
                ),
                (
                    glow::FRAGMENT_SHADER,
                    "#version 330 core\n\
                     in vec2 v_uv;\n\
                     uniform sampler2D ink;\n\
                     uniform float alpha;\n\
                     out vec4 color;\n\
                     void main() {\n\
                         color = texture(ink, v_uv);\n\
                         color.a *= alpha;\n\
                     }",
                ),
            ];
            for (kind, source) in sources {
                let shader = gl.create_shader(kind).ok()?;
                gl.shader_source(shader, source);
                gl.compile_shader(shader);
                if !gl.get_shader_compile_status(shader) {
                    return None;
                }
                gl.attach_shader(program, shader);
                gl.delete_shader(shader);
            }
            gl.link_program(program);
            if !gl.get_program_link_status(program) {
                return None;
            }
            let alpha_at = gl.get_uniform_location(program, "alpha")?;
            let vao = gl.create_vertex_array().ok()?;
            let vbo = gl.create_buffer().ok()?;
            gl.bind_vertex_array(Some(vao));
            gl.bind_buffer(glow::ARRAY_BUFFER, Some(vbo));
            gl.enable_vertex_attrib_array(0);
            gl.vertex_attrib_pointer_f32(0, 4, glow::FLOAT, false, 16, 0);
            gl.bind_vertex_array(None);
            let texture = gl.create_texture().ok()?;
            Some(Self {
                program,
                vao,
                vbo,
                texture,
                alpha_at,
                size: (0, 0),
                place: Place::TopLeft,
                shown: None,
            })
        }
    }

    /// Draws the lines into the bitmap and starts its stay. `scale` is the
    /// window's pixels per logical pixel, so the type stays one size.
    pub fn show(
        &mut self,
        gl: &glow::Context,
        lines: &[TextLine],
        place: Place,
        hold: Duration,
        scale: f32,
    ) {
        let scale = scale.max(0.5);
        let scaled: Vec<TextLine> = lines
            .iter()
            .map(|line| TextLine {
                text: line.text.clone(),
                px: line.px * scale,
            })
            .collect();
        let (width, height, rgba) = block_rgba(&scaled);
        // SAFETY: the context is current; the texture is this overlay's own.
        unsafe {
            gl.bind_texture(glow::TEXTURE_2D, Some(self.texture));
            gl.pixel_store_i32(glow::UNPACK_ALIGNMENT, 1);
            gl.tex_image_2d(
                glow::TEXTURE_2D,
                0,
                glow::RGBA8 as i32,
                width as i32,
                height as i32,
                0,
                glow::RGBA,
                glow::UNSIGNED_BYTE,
                glow::PixelUnpackData::Slice(Some(&rgba)),
            );
            gl.tex_parameter_i32(
                glow::TEXTURE_2D,
                glow::TEXTURE_MIN_FILTER,
                glow::LINEAR as i32,
            );
            gl.tex_parameter_i32(
                glow::TEXTURE_2D,
                glow::TEXTURE_MAG_FILTER,
                glow::LINEAR as i32,
            );
            gl.bind_texture(glow::TEXTURE_2D, None);
        }
        self.size = (width, height);
        self.place = place;
        self.shown = Some((Instant::now(), hold));
    }

    /// Draws the overlay over the frame, fading at the end of its stay.
    /// Returns whether it is still on show, so frames keep coming.
    pub fn draw(&mut self, gl: &glow::Context, window: (u32, u32), scale: f32) -> bool {
        let Some((since, hold)) = self.shown else {
            return false;
        };
        let age = since.elapsed();
        if age >= hold + FADE {
            self.shown = None;
            return false;
        }
        let alpha = if age <= hold {
            1.0
        } else {
            1.0 - (age - hold).as_secs_f32() / FADE.as_secs_f32()
        };
        let (win_w, win_h) = (window.0.max(1) as f32, window.1.max(1) as f32);
        let (w, h) = (self.size.0 as f32, self.size.1 as f32);
        let margin = MARGIN * scale.max(0.5);
        let left = margin;
        let top = match self.place {
            Place::TopLeft => margin,
            Place::BottomLeft => (win_h - margin - h).max(0.0),
        };
        // Window pixels to clip space; row zero of the bitmap is its top.
        let x0 = left / win_w * 2.0 - 1.0;
        let x1 = (left + w) / win_w * 2.0 - 1.0;
        let y0 = 1.0 - top / win_h * 2.0;
        let y1 = 1.0 - (top + h) / win_h * 2.0;
        let quad: [f32; 16] = [
            x0, y0, 0.0, 0.0, //
            x1, y0, 1.0, 0.0, //
            x0, y1, 0.0, 1.0, //
            x1, y1, 1.0, 1.0,
        ];
        // SAFETY: the context is current; the state touched is put back.
        unsafe {
            // The engine leaves the viewport at the picture's inner size
            // when a lower resolution is on; clip space maps through the
            // viewport, so an overlay drawn into it came out small.
            gl.viewport(0, 0, window.0.max(1) as i32, window.1.max(1) as i32);
            gl.use_program(Some(self.program));
            gl.uniform_1_f32(Some(&self.alpha_at), alpha);
            gl.active_texture(glow::TEXTURE0);
            gl.bind_texture(glow::TEXTURE_2D, Some(self.texture));
            gl.bind_vertex_array(Some(self.vao));
            gl.bind_buffer(glow::ARRAY_BUFFER, Some(self.vbo));
            gl.buffer_data_u8_slice(glow::ARRAY_BUFFER, bytemuck_cast(&quad), glow::STREAM_DRAW);
            gl.enable(glow::BLEND);
            gl.blend_func(glow::SRC_ALPHA, glow::ONE_MINUS_SRC_ALPHA);
            gl.disable(glow::DEPTH_TEST);
            gl.draw_arrays(glow::TRIANGLE_STRIP, 0, 4);
            gl.disable(glow::BLEND);
            gl.bind_vertex_array(None);
            gl.use_program(None);
        }
        true
    }
}

/// The quad's floats as the bytes GL takes.
fn bytemuck_cast(quad: &[f32; 16]) -> &[u8] {
    // SAFETY: f32s reread as bytes, same length, no padding.
    unsafe { std::slice::from_raw_parts(quad.as_ptr().cast::<u8>(), std::mem::size_of_val(quad)) }
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

    /// The rasteriser inks what it is given: some coverage, more width
    /// for more text, and both lines inside the block.
    #[test]
    fn text_becomes_ink() {
        let short = line_raster("Keys", 20.0);
        let long = line_raster("N or the right arrow plays the next preset", 20.0);
        assert!(short.alpha.iter().any(|a| *a > 0), "letters leave ink");
        assert!(long.width > short.width);

        let (width, height, rgba) = block_rgba(&[
            TextLine {
                text: "Wish You Were Here".into(),
                px: 26.0,
            },
            TextLine {
                text: "Incubus — Morning View".into(),
                px: 18.0,
            },
        ]);
        assert!(width > 0 && height > 0);
        let inked = rgba.chunks(4).filter(|px| px[3] > 0).count();
        assert!(inked > 100, "the block carries the text and its shadow");
    }
}
