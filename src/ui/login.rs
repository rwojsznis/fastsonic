//! The sign-in screen.
//!
//! A form, where there used to be a browser. Three fields and a button: the
//! server's address, the account on it, and the password — which is sent
//! once, exchanged for a salted token, and never written anywhere (D10).
//!
//! The errors are the interesting part. A self-hosted server fails in ways
//! a hosted one cannot, and each of them wants a different next step: a host
//! that does not answer, a host that answers but not as a music server, and
//! a server that answers and refuses the account. The client tells the three
//! apart (`ApiError::Network`, `NotSubsonic`, `Unauthorized`) and the message
//! it produces is shown as it stands.

use egui::{Align, CornerRadius, Frame, Layout, Margin, Stroke, Vec2};

use crate::app::App;
use crate::backend::AuthStatus;
use crate::model::Action;
use crate::theme;

pub fn show(app: &mut App, ui: &mut egui::Ui, connecting: bool) {
    let palette = app.palette;
    egui::CentralPanel::default()
        .frame(Frame::new().fill(palette.window))
        .show(ui, |ui| {
            let rect = ui.max_rect();
            super::titlebar_drag(ui, rect);
            let top = super::blend(palette.window, palette.accent, 0.10);
            super::widgets::paint_vertical_gradient(ui, rect, top, palette.window);
            let card_width = 440.0;
            let card_height = 430.0;
            let card = egui::Rect::from_center_size(
                rect.center() - Vec2::new(0.0, 20.0),
                Vec2::new(card_width, card_height),
            );
            let mut card_ui = ui.new_child(
                egui::UiBuilder::new()
                    .max_rect(card)
                    .layout(Layout::top_down(Align::Center)),
            );
            Frame::new()
                .fill(palette.panel)
                .stroke(Stroke::new(1.0, palette.outline))
                .corner_radius(CornerRadius::same(theme::RADIUS + 8))
                .inner_margin(Margin::same(36))
                .shadow(egui::epaint::Shadow {
                    offset: [0, 16],
                    blur: 48,
                    spread: 0,
                    color: palette.shadow,
                })
                .show(&mut card_ui, |ui| {
                    ui.set_width(card_width - 72.0);
                    ui.spacing_mut().item_spacing.y = 8.0;
                    let (logo, _) = ui.allocate_exact_size(Vec2::splat(64.0), egui::Sense::hover());
                    theme::logo(ui, logo.center(), 64.0, palette.accent, palette.on_accent);
                    ui.add_space(4.0);
                    theme::text(ui, "Fastsonic", theme::bold(28.0), palette.text);
                    theme::text(
                        ui,
                        "A native client for your own music server.",
                        theme::regular(13.5),
                        palette.secondary,
                    );
                    ui.add_space(16.0);
                    if connecting {
                        ui.horizontal(|ui| {
                            ui.add_space((ui.available_width() - 200.0).max(0.0) / 2.0);
                            theme::spinner(ui, 18.0, palette.accent);
                            theme::text(
                                ui,
                                "Connecting to your server…",
                                theme::medium(14.0),
                                palette.text,
                            );
                        });
                        ui.add_space(14.0);
                        if theme::pill_button(ui, &palette, "Cancel", false).clicked() {
                            app.actions.push(Action::CancelSignIn);
                        }
                        return;
                    }
                    let submitted = form(app, ui);
                    if let AuthStatus::Failed(message) = &app.auth {
                        let message = message.clone();
                        ui.add_space(10.0);
                        ui.add(
                            egui::Label::new(
                                egui::RichText::new(message)
                                    .font(theme::regular(13.0))
                                    .color(palette.danger),
                            )
                            .wrap(),
                        );
                    }
                    ui.add_space(14.0);
                    let ready = !app.server.trim().is_empty() && !app.server_user.trim().is_empty();
                    let pressed = big_button(ui, app, "Connect", ready);
                    if ready && (submitted || pressed) {
                        app.actions.push(Action::SignIn);
                    }
                    ui.add_space(8.0);
                    ui.add(
                        egui::Label::new(
                            egui::RichText::new(
                                "Your password is sent to this server once, in exchange for a \
                                 token. Fastsonic never stores it.",
                            )
                            .font(theme::regular(12.0))
                            .color(palette.secondary),
                        )
                        .wrap(),
                    );
                });
            ui.painter().text(
                egui::pos2(rect.center().x, rect.bottom() - 24.0),
                egui::Align2::CENTER_BOTTOM,
                format!("Fastsonic {}", env!("CARGO_PKG_VERSION")),
                theme::regular(11.5),
                palette.dim,
            );
        });
}

/// The three fields. Returns whether Enter was pressed in one of them, so
/// that the form submits the way every other form does.
fn form(app: &mut App, ui: &mut egui::Ui) -> bool {
    let mut submitted = false;
    submitted |= field(app, ui, "Server address", Field::Server);
    submitted |= field(app, ui, "Username", Field::Username);
    submitted |= field(app, ui, "Password", Field::Password);
    submitted
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Field {
    Server,
    Username,
    Password,
}

fn field(app: &mut App, ui: &mut egui::Ui, label: &str, which: Field) -> bool {
    let palette = app.palette;
    ui.with_layout(Layout::top_down(Align::Min), |ui| {
        theme::text(ui, label, theme::medium(12.0), palette.secondary);
        let hint = match which {
            Field::Server => "http://navidrome.local:4533",
            Field::Username => "",
            // A password already exchanged does not have to be typed again:
            // an empty box retries the stored token, which is what a server
            // that was merely unreachable needs.
            Field::Password => "leave empty to reuse the saved sign-in",
        };
        let value = match which {
            Field::Server => &mut app.server,
            Field::Username => &mut app.server_user,
            Field::Password => &mut app.server_password,
        };
        let edit = egui::TextEdit::singleline(value)
            .desired_width(f32::INFINITY)
            .font(theme::regular(14.0))
            .hint_text(hint)
            .password(which == Field::Password);
        let response = ui.add(edit);
        ui.add_space(6.0);
        response.lost_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter))
    })
    .inner
}

fn big_button(ui: &mut egui::Ui, app: &App, label: &str, enabled: bool) -> bool {
    let palette = app.palette;
    let text_color = if enabled {
        palette.on_accent
    } else {
        palette.dim
    };
    let galley = ui
        .painter()
        .layout_no_wrap(label.to_string(), theme::bold(15.0), text_color);
    let size = Vec2::new(ui.available_width().min(300.0), 46.0);
    let (rect, response) = ui.allocate_exact_size(size, egui::Sense::click());
    let fill = match (enabled, response.hovered()) {
        (false, _) => palette.surface,
        (true, true) => palette.accent_hover,
        (true, false) => palette.accent,
    };
    ui.painter().rect_filled(rect, 23.0, fill);
    ui.painter()
        .galley(rect.center() - galley.size() / 2.0, galley, text_color);
    enabled
        && response
            .on_hover_cursor(egui::CursorIcon::PointingHand)
            .clicked()
}
