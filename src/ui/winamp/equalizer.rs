//! The equalizer window, between the main window and the playlist.
//!
//! Winamp's graphic equalizer: a preamp and ten bands on sliders cut from
//! `eqmain.bmp`, the ON switch, a PRESETS button, and the little graph of
//! the curve. AUTO loaded a preset per song from a file Fastpotify has no
//! equivalent of, so it stays painted and off. The sound itself is shaped
//! in `eq`, on the player's thread; this only moves the numbers.

use egui::Sense;

use crate::app::App;
use crate::eq::{self, EqSettings, RANGE_DB};
use crate::model::Action;
use crate::skin::layout::{self, Area};
use crate::skin::{Sheet, sprites};

use super::View;

/// A slider's thumb is eleven pixels in a track of sixty-three.
const THUMB: u32 = 11;
const TRAVEL: u32 = layout::EQ_PREAMP.height - THUMB;
/// The graph's bands sit twelve pixels apart, two in from its edge.
const GRAPH_STEP: u32 = 12;
const GRAPH_PAD: u32 = 2;

pub(super) fn show(app: &mut App, view: &mut View, focused: bool) {
    view.sprite(
        sprites::EQ_BACKGROUND,
        Area::new(0, 0, layout::WINDOW_WIDTH, layout::EQ_HEIGHT),
    );
    let bar = if focused {
        sprites::EQ_TITLE_BAR_ACTIVE
    } else {
        sprites::EQ_TITLE_BAR_INACTIVE
    };
    view.sprite(bar, layout::EQ_TITLE_BAR);
    let title = view.interact(layout::EQ_TITLE_BAR, "eq-title", Sense::click_and_drag());
    if title.drag_started() {
        view.ui
            .ctx()
            .send_viewport_cmd(egui::ViewportCommand::StartDrag);
    }
    if view
        .button(
            layout::EQ_CLOSE,
            sprites::EQ_CLOSE_BUTTON,
            sprites::EQ_CLOSE_BUTTON_PRESSED,
            "eq-close",
        )
        .clicked()
    {
        app.actions.push(Action::ToggleWinampEq);
    }

    let settings = current(app);
    let (normal, pressed) = if settings.on {
        (sprites::EQ_ON_ON, sprites::EQ_ON_ON_PRESSED)
    } else {
        (sprites::EQ_ON_OFF, sprites::EQ_ON_OFF_PRESSED)
    };
    if view
        .button(layout::EQ_ON, normal, pressed, "eq-on")
        .clicked()
    {
        app.actions.push(Action::ToggleEq);
    }
    view.sprite(sprites::EQ_AUTO_OFF, layout::EQ_AUTO);
    let presets = view.button(
        layout::EQ_PRESETS_BUTTON,
        sprites::EQ_PRESETS,
        sprites::EQ_PRESETS_PRESSED,
        "eq-presets",
    );
    let unit = view.unit;
    egui::Popup::menu(&presets).show(|ui| presets_menu(app, ui, unit));

    graph(view, &settings);
    let preamp = fraction(settings.preamp_db);
    if let Some(value) = slider(view, layout::EQ_PREAMP, "eq-preamp", preamp) {
        // The preamp never boosts: the top half of its track is dead.
        app.actions
            .push(Action::SetEqPreamp(decibels(value).min(0.0)));
    }
    for (band, gain_db) in settings.bands_db.into_iter().enumerate() {
        let area = layout::eq_band(band);
        if let Some(value) = slider(view, area, &format!("eq-band-{band}"), fraction(gain_db)) {
            app.actions.push(Action::SetEqBand(band, decibels(value)));
        }
    }
}

fn current(app: &App) -> EqSettings {
    EqSettings {
        on: app.settings.eq_on,
        preamp_db: app.settings.eq_preamp_db,
        bands_db: app.settings.eq_bands_db,
    }
    .clamped()
}

/// A gain as a fraction of the slider, 0 at the bottom.
fn fraction(gain_db: f32) -> f32 {
    ((gain_db + RANGE_DB) / (2.0 * RANGE_DB)).clamp(0.0, 1.0)
}

fn decibels(fraction: f32) -> f32 {
    fraction * 2.0 * RANGE_DB - RANGE_DB
}

/// A vertical slider drawn from the skin's frames, with the pointer's new
/// value while it is held.
fn slider(view: &mut View, area: Area, id: &str, value: f32) -> Option<f32> {
    let response = view
        .interact(area, id, Sense::click_and_drag())
        .on_hover_cursor(egui::CursorIcon::PointingHand);
    let frame = (value * (sprites::EQ_SLIDER_FRAMES - 1) as f32).round() as u32;
    view.sprite(sprites::eq_slider_frame(frame), area);
    let held = response.dragged() || response.is_pointer_button_down_on();
    let thumb = if held {
        sprites::EQ_THUMB_PRESSED
    } else {
        sprites::EQ_THUMB
    };
    let thumb_y = area.y + ((1.0 - value) * TRAVEL as f32).round() as u32;
    view.sprite_at(thumb, area.x + 1, thumb_y);
    if !(response.dragged() || response.clicked()) {
        return None;
    }
    let pos = response.interact_pointer_pos()?;
    let along = (pos.y - view.origin.y) / view.unit - area.y as f32 - THUMB as f32 / 2.0;
    Some((1.0 - along / TRAVEL as f32).clamp(0.0, 1.0))
}

/// The curve through the bands, in the skin's colours row by row, with the
/// preamp's line under it.
fn graph(view: &mut View, settings: &EqSettings) {
    let area = layout::EQ_GRAPH;
    view.sprite(sprites::EQ_GRAPH, area);
    let rows = area.height;
    let row_of = |gain_db: f32| ((1.0 - fraction(gain_db)) * (rows - 1) as f32).round() as u32;
    view.sprite_at(
        sprites::EQ_PREAMP_LINE,
        area.x,
        area.y + row_of(settings.preamp_db),
    );
    let sheet = view.skin.sheet(Sheet::EqMain);
    let line = sprites::EQ_GRAPH_LINE;
    let color_at = |row: u32| {
        sheet
            .pixel(line.x, line.y + row.min(line.height - 1))
            .map(|[r, g, b, _]| egui::Color32::from_rgb(r, g, b))
            .unwrap_or(egui::Color32::WHITE)
    };
    let points: Vec<f32> = settings.bands_db.iter().map(|db| fraction(*db)).collect();
    let width = GRAPH_STEP * (eq::BANDS.len() as u32 - 1);
    let mut last = row_of(settings.bands_db[0]);
    for x in 0..=width {
        let value = spline(&points, x as f32 / GRAPH_STEP as f32);
        let row = ((1.0 - value) * (rows - 1) as f32).round() as u32;
        let (top, bottom) = if row < last { (row, last) } else { (last, row) };
        for r in top..=bottom {
            view.fill(area.x + GRAPH_PAD + x, area.y + r, 1, 1, color_at(r));
        }
        last = row;
    }
}

/// A smooth curve through evenly spaced points (Catmull-Rom), at `t`
/// measured in points.
fn spline(points: &[f32], t: f32) -> f32 {
    let last = points.len() - 1;
    let i = (t.floor() as usize).min(last.saturating_sub(1));
    let u = (t - i as f32).clamp(0.0, 1.0);
    let at = |index: isize| points[index.clamp(0, last as isize) as usize];
    let (p0, p1, p2, p3) = (
        at(i as isize - 1),
        at(i as isize),
        at(i as isize + 1),
        at(i as isize + 2),
    );
    let value = 0.5
        * (2.0 * p1
            + (-p0 + p2) * u
            + (2.0 * p0 - 5.0 * p1 + 4.0 * p2 - p3) * u * u
            + (-p0 + 3.0 * p1 - 3.0 * p2 + p3) * u * u * u);
    value.clamp(0.0, 1.0)
}

/// Winamp's presets, in a menu sized to the skin.
fn presets_menu(app: &mut App, ui: &mut egui::Ui, unit: f32) {
    let font = (5.0 * unit).clamp(9.0, 14.0);
    for style in [egui::TextStyle::Body, egui::TextStyle::Button] {
        ui.style_mut()
            .text_styles
            .insert(style, egui::FontId::proportional(font));
    }
    ui.spacing_mut().item_spacing = egui::vec2(4.0, 1.0);
    ui.spacing_mut().button_padding = egui::vec2(6.0, 1.0);
    let current = app.settings.eq_bands_db;
    for (index, preset) in eq::PRESETS.iter().enumerate() {
        let chosen = preset.bands_db == current;
        if ui.selectable_label(chosen, preset.name).clicked() {
            app.actions.push(Action::ApplyEqPreset(index));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_slider_maps_the_range_and_back() {
        assert_eq!(fraction(-RANGE_DB), 0.0);
        assert_eq!(fraction(0.0), 0.5);
        assert_eq!(fraction(RANGE_DB), 1.0);
        assert!((decibels(fraction(3.0)) - 3.0).abs() < 1e-5);
    }

    #[test]
    fn the_curve_passes_through_the_bands_and_stays_inside() {
        let points = [0.5, 1.0, 0.0, 0.5, 0.5, 0.5, 0.5, 0.5, 0.5, 0.5];
        assert!((spline(&points, 1.0) - 1.0).abs() < 1e-5);
        assert!((spline(&points, 2.0) - 0.0).abs() < 1e-5);
        for step in 0..=90 {
            let value = spline(&points, step as f32 / 10.0);
            assert!((0.0..=1.0).contains(&value));
        }
    }
}
