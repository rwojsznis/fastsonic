//! The words of the playing track, in a side panel that follows the song.

use egui::{Align, Frame, Layout, Margin, Sense};

use crate::app::App;
use crate::model::{Action, Loadable};
use crate::theme::{self, Icon};

use super::widgets;

pub fn side_panel(app: &mut App, ui: &mut egui::Ui) {
    let palette = app.palette;
    egui::Panel::right("lyrics-panel")
        .exact_size(360.0)
        .resizable(false)
        .show_separator_line(false)
        .frame(
            Frame::new()
                .fill(palette.panel)
                .inner_margin(Margin::symmetric(12, 12)),
        )
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.add_space(4.0);
                theme::text(ui, "Lyrics", theme::bold(18.0), palette.text);
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if theme::icon_button(
                        ui,
                        Icon::X,
                        18.0,
                        palette.secondary,
                        palette.text,
                        "Close",
                    )
                    .clicked()
                    {
                        app.actions.push(Action::ToggleLyricsPanel);
                    }
                    let synced =
                        matches!(&app.lyrics, Loadable::Loaded(Some(lyrics)) if lyrics.synced);
                    if synced
                        && !app.lyrics_following
                        && theme::pill_button(ui, &palette, "Follow", false).clicked()
                    {
                        app.lyrics_following = true;
                    }
                });
            });
            ui.add_space(8.0);
            contents(app, ui);
        });
}

fn contents(app: &mut App, ui: &mut egui::Ui) {
    let palette = app.palette;
    let Some(now) = app.now_playing() else {
        widgets::empty_state(
            ui,
            &palette,
            Icon::Mic,
            "Nothing playing",
            "Play something and its words show up here.",
        );
        return;
    };
    let lyrics = match &app.lyrics {
        Loadable::NotLoaded | Loadable::Loading => {
            widgets::loading_row(ui, &palette);
            return;
        }
        Loadable::Failed(error) => {
            let message = format!("Couldn't fetch the lyrics: {error}");
            ui.add_space(8.0);
            theme::text(ui, message, theme::regular(13.0), palette.secondary);
            ui.add_space(8.0);
            if theme::pill_button(ui, &palette, "Try again", false).clicked() {
                app.request_lyrics();
            }
            return;
        }
        Loadable::Loaded(None) => {
            widgets::empty_state(
                ui,
                &palette,
                Icon::Mic,
                "No lyrics",
                "Nobody has transcribed this one yet.",
            );
            return;
        }
        Loadable::Loaded(Some(lyrics)) if lyrics.instrumental => {
            widgets::empty_state(
                ui,
                &palette,
                Icon::Music,
                "Instrumental",
                "No words to follow on this one.",
            );
            return;
        }
        Loadable::Loaded(Some(lyrics)) => lyrics.clone(),
    };

    let active = lyrics.active_line(now.position_ms);
    let follow = app.lyrics_following && app.lyrics_line_shown != active;
    let scroll = egui::ScrollArea::vertical()
        .id_salt("lyrics-scroll")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            ui.add_space(12.0);
            for (index, line) in lyrics.lines.iter().enumerate() {
                let is_active = active == Some(index);
                let sung = lyrics.synced && active.is_some_and(|active| index < active);
                let color = if is_active {
                    palette.text
                } else if sung {
                    palette.dim
                } else {
                    palette.secondary
                };
                let font = if is_active {
                    theme::bold(21.0)
                } else {
                    theme::semibold(21.0)
                };
                // A timed line with no words is the band playing on.
                let text = if line.text.is_empty() && lyrics.synced {
                    "♪"
                } else {
                    line.text.as_str()
                };
                let sense = if lyrics.synced {
                    Sense::click()
                } else {
                    Sense::hover()
                };
                let response = ui.add(
                    egui::Label::new(egui::RichText::new(text).font(font).color(color))
                        .sense(sense),
                );
                let rect = response.rect;
                if lyrics.synced {
                    let response = response.on_hover_cursor(egui::CursorIcon::PointingHand);
                    if response.clicked()
                        && let Some(at_ms) = line.at_ms
                    {
                        app.actions.push(Action::Seek(at_ms));
                        app.lyrics_following = true;
                    }
                }
                if is_active && follow {
                    ui.scroll_to_rect(rect, Some(Align::Center));
                }
                ui.add_space(8.0);
            }
            ui.add_space(60.0);
        });
    // Scrolling by hand means the reader wants to look elsewhere; the
    // Follow button in the header picks the song back up.
    if lyrics.synced
        && ui.rect_contains_pointer(scroll.inner_rect)
        && ui.input(|input| input.smooth_scroll_delta.y != 0.0)
    {
        app.lyrics_following = false;
    }
    app.lyrics_line_shown = active;
}
