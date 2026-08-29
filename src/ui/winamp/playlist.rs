//! The playlist window, joined to the bottom of the main one.
//!
//! Winamp's playlist editor is a resizable frame of tiles from `pledit.bmp`
//! around a list drawn in the colours of `pledit.txt`. Here it shows the
//! queue: what is playing, then what comes next, and a double-click plays
//! from there. Fastpotify has none of the file menus along its bottom, so
//! those stay painted and quiet. The window is one with the main window,
//! which grows to hold it, since a desktop cannot be asked to keep two
//! windows together.

use egui::{Color32, Sense};

use crate::api::models::PlayableItem;
use crate::app::{App, NowPlaying};
use crate::model::{Action, RowContext};
use crate::skin::layout::{self, Area};
use crate::skin::sprites;
use crate::util;

use super::View;

/// One line of the list.
struct Row {
    uri: String,
    label: String,
    duration_ms: u32,
    current: bool,
    /// Where the song sits in the queue, when it is queued rather than on.
    queued: Option<usize>,
}

/// The whole panel, `height` skin pixels tall, drawn into a view whose
/// origin is the panel's top left.
pub(super) fn show(app: &mut App, view: &mut View, now: Option<&NowPlaying>, focused: bool) {
    let height = app
        .settings
        .playlist_height
        .max(layout::PLAYLIST_MIN_HEIGHT);
    frame(view, height, focused);

    let title = view.interact(
        Area::new(0, 0, layout::WINDOW_WIDTH, layout::PLAYLIST_TITLE_HEIGHT),
        "playlist-title",
        Sense::click_and_drag(),
    );
    if title.drag_started() {
        view.ui
            .ctx()
            .send_viewport_cmd(egui::ViewportCommand::StartDrag);
    }
    if view
        .lamp_button(
            layout::PLAYLIST_CLOSE,
            sprites::PLAYLIST_CLOSE_PRESSED,
            false,
            "playlist-close",
        )
        .clicked()
    {
        app.actions.push(Action::ToggleWinampPlaylist);
    }
    view.lamp_button(
        layout::PLAYLIST_SHADE,
        sprites::PLAYLIST_SHADE_PRESSED,
        false,
        "playlist-shade",
    );

    let (rows, queue_uris) = rows(app, now);
    list(app, view, &rows, &queue_uris, height);
    scrollbar(app, view, rows.len(), height);
    grip(app, view, height);
    times(view, now, &rows, height);
    mini_transport(app, view, now, height);
}

/// The little transport in the bottom right: the same as the big one,
/// painted into the skin, so these are only places to click.
fn mini_transport(app: &mut App, view: &mut View, now: Option<&NowPlaying>, height: u32) {
    let (x, dy) = layout::PLAYLIST_MINI_TRANSPORT;
    let y = height - layout::PLAYLIST_BOTTOM_HEIGHT + dy;
    let playing = now.is_some_and(|now| now.playing);
    for (index, name) in ["previous", "play", "pause", "stop", "next", "eject"]
        .into_iter()
        .enumerate()
    {
        let cell = Area::new(x + 10 * index as u32, y, 10, 10);
        let response = view
            .interact(cell, &format!("playlist-{name}"), Sense::click())
            .on_hover_cursor(egui::CursorIcon::PointingHand);
        if !response.clicked() {
            continue;
        }
        match name {
            "previous" => app.actions.push(Action::Previous),
            "play" => app.actions.push(if playing {
                Action::Seek(0)
            } else {
                Action::TogglePlay
            }),
            "pause" if now.is_some() => app.actions.push(Action::TogglePlay),
            "stop" if now.is_some() => {
                if playing {
                    app.actions.push(Action::TogglePlay);
                }
                app.actions.push(Action::Seek(0));
            }
            "next" => app.actions.push(Action::Next),
            "eject" => app.actions.push(Action::ToggleWinampWindow),
            _ => {}
        }
    }
}

/// The frame: corners and tiles across the top, tiles down the sides
/// however tall the list is, and the two halves of the bottom.
fn frame(view: &mut View, height: u32, focused: bool) {
    use sprites::*;
    let (top_left, title, top_tile, top_right) = if focused {
        (
            PLAYLIST_TOP_LEFT_ACTIVE,
            PLAYLIST_TITLE_ACTIVE,
            PLAYLIST_TOP_TILE_ACTIVE,
            PLAYLIST_TOP_RIGHT_ACTIVE,
        )
    } else {
        (
            PLAYLIST_TOP_LEFT,
            PLAYLIST_TITLE,
            PLAYLIST_TOP_TILE,
            PLAYLIST_TOP_RIGHT,
        )
    };
    let tile = layout::PLAYLIST_TILE_WIDTH;
    let spacers = (layout::WINDOW_WIDTH - 2 * tile - 100) / tile;
    let left_spacers = spacers / 2;
    let mut x = 0;
    view.sprite_at(top_left, x, 0);
    x += tile;
    for _ in 0..left_spacers {
        view.sprite_at(top_tile, x, 0);
        x += tile;
    }
    view.sprite_at(title, x, 0);
    x += 100;
    for _ in left_spacers..spacers {
        view.sprite_at(top_tile, x, 0);
        x += tile;
    }
    view.sprite_at(top_right, x, 0);

    let middle = Area::new(
        0,
        layout::PLAYLIST_TITLE_HEIGHT,
        layout::WINDOW_WIDTH,
        height - layout::PLAYLIST_TITLE_HEIGHT - layout::PLAYLIST_BOTTOM_HEIGHT,
    );
    let mut y = middle.y;
    while y < middle.y + middle.height {
        view.sprite_clipped(PLAYLIST_LEFT_TILE, 0, y, middle);
        view.sprite_clipped(
            PLAYLIST_RIGHT_TILE,
            layout::WINDOW_WIDTH - layout::PLAYLIST_RIGHT_WIDTH,
            y,
            middle,
        );
        y += layout::PLAYLIST_TILE_HEIGHT;
    }
    let bottom = height - layout::PLAYLIST_BOTTOM_HEIGHT;
    view.sprite_at(PLAYLIST_BOTTOM_LEFT, 0, bottom);
    view.sprite_at(PLAYLIST_BOTTOM_RIGHT, 125, bottom);
}

/// The list's own area, between the tiles.
fn list_area(height: u32) -> Area {
    Area::new(
        layout::PLAYLIST_LEFT_WIDTH,
        layout::PLAYLIST_TITLE_HEIGHT,
        layout::WINDOW_WIDTH - layout::PLAYLIST_LEFT_WIDTH - layout::PLAYLIST_RIGHT_WIDTH,
        height - layout::PLAYLIST_TITLE_HEIGHT - layout::PLAYLIST_BOTTOM_HEIGHT,
    )
}

fn rows_visible(height: u32) -> usize {
    (list_area(height).height / layout::PLAYLIST_TRACK_HEIGHT) as usize
}

/// What is playing, then the queue, numbered the way Winamp numbered a
/// playlist. The queue's URIs come along for playing from a row.
fn rows(app: &App, now: Option<&NowPlaying>) -> (Vec<Row>, Vec<String>) {
    let queue = app.queue.get();
    let current: Option<(String, String, String, u32)> = queue
        .and_then(|queue| queue.currently_playing.as_ref())
        .map(|item| {
            (
                item.uri().to_string(),
                item.subtitle(),
                item.name().to_string(),
                item.duration_ms(),
            )
        })
        .or_else(|| {
            now.map(|now| {
                (
                    now.uri.clone(),
                    now.subtitle.clone(),
                    now.title.clone(),
                    now.duration_ms,
                )
            })
        });
    let mut rows = Vec::new();
    if let Some((uri, subtitle, title, duration_ms)) = current {
        rows.push(Row {
            uri,
            label: label(1, &subtitle, &title),
            duration_ms,
            current: true,
            queued: None,
        });
    }
    let queued: &[PlayableItem] = queue.map(|queue| queue.queue.as_slice()).unwrap_or(&[]);
    for (index, item) in queued.iter().enumerate() {
        rows.push(Row {
            uri: item.uri().to_string(),
            label: label(rows.len() + 1, &item.subtitle(), item.name()),
            duration_ms: item.duration_ms(),
            current: false,
            queued: Some(index),
        });
    }
    let uris = queued.iter().map(|item| item.uri().to_string()).collect();
    (rows, uris)
}

fn label(number: usize, subtitle: &str, title: &str) -> String {
    if subtitle.is_empty() {
        format!("{number}. {title}")
    } else {
        format!("{number}. {subtitle} - {title}")
    }
}

/// The rows, in the skin's colours and a font the size Winamp's was.
fn list(app: &mut App, view: &mut View, rows: &[Row], queue_uris: &[String], height: u32) {
    let style = view.skin.playlist.clone();
    let rgb = |color: [u8; 3]| Color32::from_rgb(color[0], color[1], color[2]);
    let area = list_area(height);
    view.fill(
        area.x,
        area.y,
        area.width,
        area.height,
        rgb(style.normal_background),
    );

    let visible = rows_visible(height);
    let most = rows.len().saturating_sub(visible);
    // The wheel, over the list.
    let hovered = view
        .interact(area, "playlist-list", Sense::hover())
        .hovered();
    if hovered {
        let delta = view.ui.ctx().input(|input| input.smooth_scroll_delta.y);
        let row_points = layout::PLAYLIST_TRACK_HEIGHT as f32 * view.unit;
        app.winamp.playlist_wheel -= delta;
        let rows_moved = (app.winamp.playlist_wheel / row_points).trunc();
        if rows_moved != 0.0 {
            app.winamp.playlist_wheel -= rows_moved * row_points;
            let scroll = app.winamp.playlist_scroll as i64 + rows_moved as i64;
            app.winamp.playlist_scroll = scroll.clamp(0, most as i64) as usize;
        }
    }
    app.winamp.playlist_scroll = app.winamp.playlist_scroll.min(most);
    let scroll = app.winamp.playlist_scroll;

    let ctx = view.ui.ctx().clone();
    let clip = view.rect(area).intersect(view.ui.clip_rect());
    let painter = view.ui.painter().with_clip_rect(clip);
    let pad = 3;
    for (offset, row) in rows.iter().skip(scroll).take(visible).enumerate() {
        let line = Area::new(
            area.x,
            area.y + offset as u32 * layout::PLAYLIST_TRACK_HEIGHT,
            area.width,
            layout::PLAYLIST_TRACK_HEIGHT,
        );
        let rect = view.rect(line);
        let response = view.ui.interact(
            rect,
            egui::Id::new(("playlist-row", scroll + offset)),
            Sense::click(),
        );
        if response.clicked() {
            app.winamp.playlist_selected = Some(row.uri.clone());
        }
        if response.double_clicked()
            && let Some(index) = row.queued
        {
            app.actions.push(Action::PlayFromRow {
                context: RowContext::Uris(queue_uris.to_vec()),
                uri: row.uri.clone(),
                index: index as u32,
            });
        }
        if app.winamp.playlist_selected.as_deref() == Some(row.uri.as_str()) {
            painter.rect_filled(rect, 0.0, rgb(style.selected_background));
        }
        let color = rgb(if row.current {
            style.current
        } else {
            style.normal
        });
        let text = &mut app.winamp.playlist_text;
        let duration = util::format_duration_ms(row.duration_ms);
        let duration_width = text.width(&duration).ceil() as u32;
        let title_room = line.width.saturating_sub(3 * pad + duration_width);
        let title = fit(text, &row.label, title_room as f32);
        let duration_line = text.line(&ctx, &duration);
        let duration_at = Area::new(
            line.x + line.width - pad - duration_line.width,
            line.y + (line.height.saturating_sub(duration_line.height)) / 2,
            duration_line.width,
            duration_line.height,
        );
        painter.image(
            duration_line.texture.id(),
            view.rect(duration_at),
            egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
            color,
        );
        let title_line = text.line(&ctx, &title);
        let title_at = Area::new(
            line.x + pad,
            line.y + (line.height.saturating_sub(title_line.height)) / 2,
            title_line.width.min(title_room),
            title_line.height,
        );
        let uv_right = title_at.width as f32 / title_line.width.max(1) as f32;
        painter.image(
            title_line.texture.id(),
            view.rect(title_at),
            egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(uv_right, 1.0)),
            color,
        );
    }
}

/// Text cut to a width, with an ellipsis when it had to be.
fn fit(text: &super::PixelText, label: &str, width: f32) -> String {
    if text.width(label) <= width {
        return label.to_string();
    }
    let chars: Vec<char> = label.chars().collect();
    let mut keep = chars.len();
    while keep > 0 {
        keep -= 1;
        let candidate: String = chars[..keep].iter().collect::<String>() + "\u{2026}";
        if text.width(&candidate) <= width {
            return candidate;
        }
    }
    "\u{2026}".to_string()
}

/// The handle in the right-hand tiles, dragged to scroll.
fn scrollbar(app: &mut App, view: &mut View, rows: usize, height: u32) {
    let visible = rows_visible(height);
    let most = rows.saturating_sub(visible);
    let top = layout::PLAYLIST_TITLE_HEIGHT;
    let travel = height
        .saturating_sub(layout::PLAYLIST_TITLE_HEIGHT + layout::PLAYLIST_BOTTOM_HEIGHT)
        .saturating_sub(layout::PLAYLIST_SCROLL_HANDLE_HEIGHT);
    let fraction = if most == 0 {
        0.0
    } else {
        app.winamp.playlist_scroll as f32 / most as f32
    };
    let y = top + (fraction * travel as f32).round() as u32;
    let handle = Area::new(
        layout::PLAYLIST_SCROLL_X,
        y,
        8,
        layout::PLAYLIST_SCROLL_HANDLE_HEIGHT,
    );
    let response = view.interact(handle, "playlist-scroll", Sense::click_and_drag());
    if response.dragged()
        && most > 0
        && let Some(pos) = response.interact_pointer_pos()
    {
        let pointer = (pos.y - view.origin.y) / view.unit
            - top as f32
            - layout::PLAYLIST_SCROLL_HANDLE_HEIGHT as f32 / 2.0;
        let fraction = (pointer / travel as f32).clamp(0.0, 1.0);
        app.winamp.playlist_scroll = (fraction * most as f32).round() as usize;
    }
    let sprite = if response.dragged() || response.is_pointer_button_down_on() {
        sprites::PLAYLIST_SCROLL_HANDLE_PRESSED
    } else {
        sprites::PLAYLIST_SCROLL_HANDLE
    };
    view.sprite(sprite, handle);
}

/// The corner that stretches the list, a tile row at a time.
fn grip(app: &mut App, view: &mut View, height: u32) {
    let corner = Area::new(
        layout::WINDOW_WIDTH - layout::PLAYLIST_GRIP,
        height - layout::PLAYLIST_GRIP,
        layout::PLAYLIST_GRIP,
        layout::PLAYLIST_GRIP,
    );
    let response = view
        .interact(corner, "playlist-grip", Sense::drag())
        .on_hover_cursor(egui::CursorIcon::ResizeVertical);
    if response.dragged() {
        app.winamp.playlist_resize += response.drag_delta().y / view.unit;
        let step = layout::PLAYLIST_RESIZE_STEP as f32;
        let steps = (app.winamp.playlist_resize / step).trunc();
        if steps != 0.0 {
            app.winamp.playlist_resize -= steps * step;
            let wanted = (height as i64 + steps as i64 * step as i64).clamp(
                layout::PLAYLIST_MIN_HEIGHT as i64,
                layout::PLAYLIST_MAX_HEIGHT as i64,
            ) as u32;
            if wanted != height {
                app.actions.push(Action::SetPlaylistHeight(wanted));
            }
        }
    }
    if response.drag_stopped() {
        app.winamp.playlist_resize = 0.0;
    }
}

/// The running time of the song over the queue's total, and the song's
/// time again where Winamp kept it, in the skin's small font.
fn times(view: &mut View, now: Option<&NowPlaying>, rows: &[Row], height: u32) {
    let bottom = height - layout::PLAYLIST_BOTTOM_HEIGHT;
    let Some(now) = now else {
        return;
    };
    let total: u32 = rows.iter().map(|row| row.duration_ms).sum();
    let running = format!(
        "{}/{}",
        util::format_duration_ms(now.position_ms),
        util::format_duration_ms(total)
    );
    let (x, dy) = layout::PLAYLIST_RUNNING_TIME;
    view.text(
        &running,
        Area::new(x, bottom + dy, 5 * running.len() as u32, 6),
    );
    let (x, dy) = layout::PLAYLIST_TRACK_TIME;
    let elapsed = util::format_duration_ms(now.position_ms);
    view.text(
        &elapsed,
        Area::new(x, bottom + dy, 5 * elapsed.len() as u32, 6),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rows_are_numbered_the_way_winamp_did() {
        assert_eq!(label(1, "Bonobo", "Rosewood"), "1. Bonobo - Rosewood");
        assert_eq!(label(12, "", "Episode 12"), "12. Episode 12");
    }

    #[test]
    fn the_list_holds_whole_rows_only() {
        assert_eq!(rows_visible(116), 4);
        assert_eq!(rows_visible(174), 8);
        assert_eq!(list_area(174), Area::new(12, 20, 243, 116));
    }
}
