//! What the other people on the server are listening to, in the right panel.

use egui::{Frame, Margin, Sense, Vec2};

use crate::app::App;
use crate::listeners::Listener;
use crate::model::{Action, Loadable, Page};
use crate::theme::{self, Icon, Palette};

use super::widgets;

const AVATAR: f32 = 40.0;
const ROW_GAP: f32 = 18.0;
/// The header's title starts this far in, and the close button's glyph
/// ends this far from the right (`theme::icon_button` pads it by half of
/// `ICON_BUTTON_PAD`).
const TITLE_INSET: i8 = 4;
const CLOSE_GLYPH_INSET: i8 = 6;
const WHERE_ICON: f32 = 13.0;
pub(crate) const TITLE: &str = "Listening activity";

pub fn side_panel(app: &mut App, ui: &mut egui::Ui) {
    let palette = app.palette;
    let fit = super::yielding_panel(
        ui.ctx(),
        "listeners-panel",
        theme::SIDE_PANEL_MIN_WIDTH..=560.0,
        app.settings.queue_width,
        ui.available_width() - super::topbar::least_width(ui.ctx()),
    );
    let panel = egui::Panel::right("listeners-panel")
        .resizable(true)
        .default_size(app.settings.queue_width)
        .size_range(fit.range.clone())
        .show_separator_line(false)
        .frame(
            Frame::new()
                .fill(palette.panel)
                .inner_margin(Margin::symmetric(12, 12)),
        );
    let response = panel.show(ui, |ui| {
        // The panel sits where the Queue would, under the same controls.
        let window_controls =
            super::window_controls_reservation(ui.ctx(), true, false, ui.available_width());
        ui.add_space(window_controls.queue_top);
        let mut close = false;
        egui::Sides::new().shrink_left().show(
            ui,
            |ui| {
                ui.add_space(4.0);
                theme::text(ui, TITLE, theme::bold(16.0), palette.text);
            },
            |ui| {
                close =
                    theme::icon_button(ui, Icon::X, 18.0, palette.secondary, palette.text, "Close")
                        .clicked();
            },
        );
        if close {
            app.actions.push(Action::ToggleListenersPanel);
        }
        ui.add_space(12.0);
        egui::ScrollArea::vertical()
            .id_salt("listeners-panel-scroll")
            .auto_shrink([false, false])
            .show(ui, |ui| contents(app, ui));
    });
    // It shares the Queue panel's width, so switching between the two never
    // moves the page.
    let width = response.response.rect.width();
    if (width - app.settings.queue_width).abs() > 1.0
        && super::panel_width_chosen(ui.ctx(), "listeners-panel", &fit)
    {
        app.settings.queue_width = width;
        app.actions.push(Action::SettingsChanged);
    }
}

fn contents(app: &mut App, ui: &mut egui::Ui) {
    let palette = app.palette;
    match &app.listeners {
        Loadable::NotLoaded | Loadable::Loading => widgets::loading_row(ui, &palette),
        Loadable::Failed(error) => widgets::empty_state(
            ui,
            &palette,
            Icon::CircleAlert,
            "The server did not say who is listening",
            error,
        ),
        Loadable::Loaded(listeners) if listeners.is_empty() => widgets::empty_state(
            ui,
            &palette,
            Icon::Users,
            "Nobody else is listening",
            "What other people on your server play shows here.",
        ),
        Loadable::Loaded(listeners) => {
            let mut picked = None;
            for listener in listeners {
                // Lined up with the header: the title's left edge and the
                // close button's glyph on the right.
                let inset = Frame::new().inner_margin(Margin {
                    left: TITLE_INSET,
                    right: CLOSE_GLYPH_INSET,
                    top: 0,
                    bottom: 0,
                });
                let action = inset.show(ui, |ui| row(app, ui, &palette, listener)).inner;
                picked = picked.or(action);
                ui.add_space(ROW_GAP);
            }
            app.actions.extend(picked);
        }
    }
}

/// A person's initial beside their name and player, the song, and its
/// album. Pointing at the initial offers to play the song, or to pause it
/// when it is the song playing here; the song, its artist and its album
/// open their pages. Someone listening right now has a dot on their initial
/// and bars in place of the time.
fn row(app: &App, ui: &mut egui::Ui, palette: &Palette, listener: &Listener) -> Option<Action> {
    let mut action = None;
    let track = &listener.track;
    // Whether this song is the one here, and if so whether it plays, as the
    // player bar shows it: a song just asked for is already the one here.
    let playing_here = (app.current_track_uri().as_deref() == Some(track.uri.as_str()))
        .then(|| app.believed_playing());
    ui.horizontal_top(|ui| {
        let (rect, response) = ui.allocate_exact_size(Vec2::splat(AVATAR), Sense::click());
        if ui.is_rect_visible(rect) {
            avatar(ui, palette, rect, &listener.username);
            if response.hovered() {
                ui.painter().circle_filled(
                    rect.center(),
                    AVATAR / 2.0,
                    egui::Color32::from_black_alpha(140),
                );
                let icon = if playing_here == Some(true) {
                    Icon::PauseFilled
                } else {
                    Icon::PlayFilled
                };
                theme::paint_icon(ui, icon, rect, 16.0, egui::Color32::WHITE);
            }
            if listener.playing {
                // Inside the initial's box, so the list's edge never cuts it.
                let center = rect.right_top() + egui::vec2(-6.0, 6.0);
                ui.painter().circle_filled(center, 6.0, palette.panel);
                ui.painter().circle_filled(center, 4.5, palette.accent);
            }
        }
        let hint = if playing_here == Some(true) {
            "Pause".to_string()
        } else {
            format!("Play {}", track.name)
        };
        response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, &hint));
        theme::focus_ring(ui, &response);
        if response
            .on_hover_cursor(egui::CursorIcon::PointingHand)
            .on_hover_text(hint)
            .clicked()
        {
            // The song already here pauses or resumes; any other starts.
            action = Some(match playing_here {
                Some(_) => Action::TogglePlay,
                None => Action::PlayUris {
                    uris: vec![track.uri.clone()],
                    index: 0,
                },
            });
        }
        ui.add_space(4.0);
        ui.vertical(|ui| {
            // Lines as tall as their text, not as a button.
            ui.spacing_mut().item_spacing.y = 3.0;
            ui.spacing_mut().interact_size.y = 0.0;
            egui::Sides::new().shrink_left().show(
                ui,
                |ui| {
                    ui.spacing_mut().item_spacing.x = 6.0;
                    theme::text(ui, &listener.username, theme::semibold(13.5), palette.text);
                    if let Some(player) = &listener.player {
                        theme::text(ui, player, theme::regular(11.5), palette.secondary)
                            .on_hover_text(format!("Playing in {player}"));
                    }
                },
                |ui| {
                    if listener.playing {
                        listening_bars(ui, palette.accent).on_hover_text("Listening now");
                    } else {
                        theme::text(
                            ui,
                            crate::listeners::when(listener.minutes_ago),
                            theme::regular(11.5),
                            palette.secondary,
                        );
                    }
                },
            );
            let line = ui.fonts_mut(|fonts| fonts.row_height(&line_font()));
            let album = track.album.as_ref().filter(|album| !album.name.is_empty());
            let album_page = album
                .filter(|album| !album.id.is_empty())
                .map(|album| Page::Album(album.id.clone()));
            let artist_name = track.artist_names();
            let artist_page = track
                .artists
                .iter()
                .find_map(|artist| artist.id.clone().filter(|id| !id.is_empty()))
                .map(Page::Artist);
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 4.0;
                // The song gives way to its artist only as far as half the
                // line, so both stay readable and neither widens the panel.
                let width = ui.available_width();
                let dot = ui.fonts_mut(|fonts| {
                    fonts
                        .layout_no_wrap("•".into(), line_font(), palette.secondary)
                        .size()
                        .x
                });
                let artist = if artist_name.is_empty() {
                    0.0
                } else {
                    let natural = ui.fonts_mut(|fonts| {
                        fonts
                            .layout_no_wrap(artist_name.clone(), line_font(), palette.secondary)
                            .size()
                            .x
                    });
                    natural.min(width / 2.0)
                };
                let song_width = if artist > 0.0 {
                    (width - artist - dot - 8.0).max(0.0)
                } else {
                    width
                };
                ui.allocate_ui(egui::vec2(song_width, line), |ui| {
                    // A song has no page of its own; its title opens the album.
                    let song = link(ui, &track.name, album_page.clone(), palette.text);
                    action = action.take().or(song);
                });
                if artist > 0.0 {
                    theme::text(ui, "•", line_font(), palette.secondary);
                    ui.allocate_ui(egui::vec2(ui.available_width(), line), |ui| {
                        let artist = link(ui, &artist_name, artist_page, palette.secondary);
                        action = action.take().or(artist);
                    });
                }
            });
            if let Some(album) = album {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 5.0;
                    // Centred on the text's line, not on the icon's own box.
                    let (cell, _) =
                        ui.allocate_exact_size(egui::vec2(WHERE_ICON, line), Sense::hover());
                    theme::paint_icon(ui, Icon::Disc, cell, WHERE_ICON, palette.secondary);
                    let opened = link(ui, &album.name, album_page, palette.secondary);
                    action = action.take().or(opened);
                });
            }
        });
    });
    action
}

/// The person's initial on a disc, since Subsonic users have no picture.
fn avatar(ui: &egui::Ui, palette: &Palette, rect: egui::Rect, username: &str) {
    ui.painter()
        .circle_filled(rect.center(), AVATAR / 2.0, palette.surface_active);
    let initial: String = username
        .chars()
        .find(|c| c.is_alphanumeric())
        .map(|c| c.to_uppercase().collect())
        .unwrap_or_default();
    if initial.is_empty() {
        theme::paint_icon(ui, Icon::User, rect, 18.0, palette.secondary);
    } else {
        ui.painter().text(
            rect.center(),
            egui::Align2::CENTER_CENTER,
            initial,
            theme::semibold(16.0),
            palette.text,
        );
    }
}

/// The song, its artist and its album share one style.
fn line_font() -> egui::FontId {
    theme::regular(12.5)
}

/// Four bars of uneven height while someone listens. They hold still,
/// whatever the system's motion setting: a moving picture would redraw the
/// window every frame for as long as anyone listens, and the panel stays
/// open for hours.
fn listening_bars(ui: &mut egui::Ui, color: egui::Color32) -> egui::Response {
    const LEVELS: [f32; 4] = [0.6, 1.0, 0.45, 0.8];
    const BAR: f32 = 2.5;
    const GAP: f32 = 2.0;
    const HEIGHT: f32 = 13.0;
    let bars = LEVELS.len() as f32;
    let size = egui::vec2(bars * BAR + (bars - 1.0) * GAP, HEIGHT);
    let (rect, response) = ui.allocate_exact_size(size, Sense::hover());
    for (bar, level) in LEVELS.iter().enumerate() {
        let x = rect.left() + bar as f32 * (BAR + GAP);
        let bar_rect = egui::Rect::from_min_max(
            egui::pos2(x, rect.bottom() - HEIGHT * level),
            egui::pos2(x + BAR, rect.bottom()),
        );
        ui.painter().rect_filled(bar_rect, BAR / 2.0, color);
    }
    response
}

/// Text that opens `page` when there is one, plain text otherwise.
fn link(ui: &mut egui::Ui, text: &str, page: Option<Page>, color: egui::Color32) -> Option<Action> {
    match page {
        Some(page) => theme::link(ui, text, line_font(), color)
            .clicked()
            .then_some(Action::Open(page)),
        None => {
            theme::text(ui, text, line_font(), color);
            None
        }
    }
}
