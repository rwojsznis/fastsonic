//! Window layout: panels, overlays, keyboard shortcuts.

pub mod artist;
pub mod collection;
mod dialogs;
pub mod home;
mod keys;
pub mod library;
pub mod login;
pub mod lyrics;
pub mod player_bar;
pub mod queue;
pub mod search;
pub mod settings;
pub mod sidebar;
pub mod topbar;
pub mod widgets;
pub mod winamp;

use egui::{Align2, Color32, CornerRadius, Frame, Margin, Rect, Stroke, vec2};

use crate::api::models::pick_image;
use crate::app::App;
use crate::backend::AuthStatus;
use crate::model::{Action, Page, ToastKind};
use crate::theme::{self, Icon};

pub fn show(app: &mut App, ui: &mut egui::Ui) {
    let ctx = ui.ctx().clone();
    let ctx = &ctx;
    keys::handle(app, ctx);
    for path in winamp::dropped_skins(ctx) {
        app.actions.push(Action::InstallSkin(path));
    }
    let signed_in = app.is_connected() && app.user.is_some();
    let connecting = matches!(app.auth, AuthStatus::Connecting | AuthStatus::Starting)
        || (app.is_connected() && app.user.is_none());
    if !signed_in {
        player_bar::end_tint_session(ctx);
        login::show(app, ui, connecting);
        toasts(app, ctx, 20.0);
        window_controls(ui, &app.palette);
        window_resize(ui);
        return;
    }
    if titlebar_spans_window(cfg!(target_os = "macos"), crate::window::custom_titlebar()) {
        titlebar_drag(
            ui,
            Rect::from_min_size(
                ui.max_rect().min,
                vec2(
                    ui.max_rect().width(),
                    theme::TOP_BAR_HEIGHT + theme::titlebar_inset(ui.ctx()),
                ),
            ),
        );
    }
    player_bar::show(app, ui);
    if app.lyrics_fullscreen.is_some() {
        lyrics::fullscreen(app, ui);
    } else {
        if app.settings.sidebar_visible {
            sidebar::show(app, ui);
        }
        if app.show_queue_panel {
            queue::side_panel(app, ui);
        }
        if app.show_lyrics_panel {
            lyrics::side_panel(app, ui);
        }
        central(app, ui);
    }
    dialogs::show(app, ctx);
    widgets::drag_ghost(ctx, &app.palette);
    toasts(app, ctx, theme::PLAYER_BAR_HEIGHT + 16.0);
    window_controls(ui, &app.palette);
    window_resize(ui);
}

fn page_tint(app: &mut App) -> Option<Color32> {
    let page = app.page().clone();
    let image = match &page {
        Page::Playlist(id) => app
            .playlist_pages
            .get(id)
            .and_then(|page| page.playlist.get())
            .and_then(|playlist| pick_image(&playlist.images, 300))
            .map(str::to_string),
        Page::Album(id) => app
            .album_pages
            .get(id)
            .and_then(|page| page.album.get())
            .and_then(|album| pick_image(&album.images, 300))
            .map(str::to_string),
        Page::Artist(id) => app
            .artist_pages
            .get(id)
            .and_then(|page| page.artist.get())
            .and_then(|artist| pick_image(&artist.images, 300))
            .map(str::to_string),
        Page::LikedSongs => return Some(Color32::from_rgb(0x50, 0x38, 0xc8)),
        _ => None,
    };
    if !app.settings.accent_from_art && image.is_some() {
        return None;
    }
    match image {
        Some(url) => app.tint_for(Some(&url)),
        None => app.now_playing_tint(),
    }
}

fn central(app: &mut App, ui: &mut egui::Ui) {
    let palette = app.palette;
    let tint = page_tint(app);
    egui::CentralPanel::default()
        .frame(Frame::new().fill(palette.window))
        .show(ui, |ui| {
            let rect = ui.max_rect();
            if let Some(tint) = tint {
                let strength = if matches!(
                    app.page(),
                    Page::Home | Page::Search | Page::Settings | Page::Queue
                ) {
                    0.45
                } else {
                    0.85
                };
                let top = blend(palette.window, tint, strength);
                let header = Rect::from_min_size(rect.min, vec2(rect.width(), 340.0));
                widgets::paint_vertical_gradient(ui, header, top, palette.window);
            }
            ui.spacing_mut().item_spacing = vec2(8.0, 6.0);
            // The default edge fade blends into the panel's plain colour,
            // leaving a pale strip over a tinted page.
            ui.spacing_mut().scroll.fade.strength = 0.0;
            topbar::show(app, ui);
            let page = app.page().clone();
            let scroll = egui::ScrollArea::vertical()
                .id_salt(("page", page.encode()))
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    Frame::new()
                        .inner_margin(Margin {
                            left: widgets::PAGE_PADDING as i8,
                            right: widgets::PAGE_PADDING as i8,
                            top: 4,
                            bottom: 48,
                        })
                        .show(ui, |ui| {
                            ui.set_min_width(ui.available_width());
                            match page {
                                Page::Home => home::show(app, ui),
                                Page::TopSongs => collection::top_songs(app, ui),
                                Page::Search => search::show(app, ui),
                                Page::LikedSongs => collection::liked(app, ui),
                                Page::Albums | Page::Artists => library::show(app, ui, page),
                                Page::Playlist(id) => collection::playlist(app, ui, &id),
                                Page::Album(id) => collection::album(app, ui, &id),
                                Page::Artist(id) => artist::show(app, ui, &id),
                                Page::Queue => queue::page(app, ui),
                                Page::Settings => settings::show(app, ui),
                            }
                        });
                });
            header_shadow(ui, scroll.inner_rect, scroll.state.offset.y, palette.dark);
        });
}

fn header_shadow(ui: &egui::Ui, page: Rect, scrolled: f32, dark: bool) {
    let depth = (scrolled / 24.0).clamp(0.0, 1.0);
    if depth <= 0.0 {
        return;
    }
    let strength = if dark { 110.0 } else { 36.0 };
    let rect = Rect::from_min_size(page.min, vec2(page.width(), 14.0));
    widgets::paint_vertical_gradient(
        ui,
        rect,
        Color32::from_black_alpha((strength * depth) as u8),
        Color32::TRANSPARENT,
    );
}

#[cfg(test)]
mod header_shadow_tests {
    use super::*;

    #[test]
    fn scrolled_page_has_a_shadow_only_below_the_header() {
        let ctx = egui::Context::default();
        let page = Rect::from_min_size(egui::pos2(0.0, 80.0), vec2(800.0, 600.0));
        let draw = |scrolled, dark| {
            let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
                header_shadow(ui, page, scrolled, dark);
            });
            output.textures_delta.clear();
            output
                .shapes
                .into_iter()
                .filter_map(|shape| match shape.shape {
                    egui::Shape::Mesh(mesh) => Some(mesh.vertices.clone()),
                    _ => None,
                })
                .flatten()
                .collect::<Vec<_>>()
        };
        assert!(draw(0.0, true).is_empty());
        for dark in [true, false] {
            let vertices = draw(40.0, dark);
            assert!(vertices.iter().any(|vertex| {
                vertex.pos.y == page.top()
                    && vertex.color.a() > 0
                    && vertex.color.r() == 0
                    && vertex.color.g() == 0
                    && vertex.color.b() == 0
            }));
        }
    }
}

/// Makes `rect` drag the borderless window. Register it before child widgets so
/// they keep their clicks.
pub fn titlebar_drag(ui: &mut egui::Ui, rect: egui::Rect) {
    let fullscreen = ui
        .ctx()
        .input(|input| input.viewport().fullscreen.unwrap_or(false));
    if !titlebar_drags(
        cfg!(target_os = "macos"),
        crate::window::custom_titlebar(),
        fullscreen,
    ) {
        return;
    }
    let response = ui.interact(
        rect,
        ui.id().with("titlebar-drag"),
        egui::Sense::click_and_drag(),
    );
    if crate::window::custom_titlebar() && response.double_clicked() {
        let maximized = ui
            .ctx()
            .input(|input| input.viewport().maximized.unwrap_or(false));
        ui.ctx()
            .send_viewport_cmd(egui::ViewportCommand::Maximized(!maximized));
    } else if (crate::window::custom_titlebar() && response.drag_started())
        // macOS needs the live mouse-down event rather than egui's drag
        // threshold, and a second click is the Desktop & Dock action instead.
        || (cfg!(target_os = "macos")
            && response.is_pointer_button_down_on()
            && ui.input(|input| input.pointer.primary_pressed())
            && crate::window::macos_titlebar_should_drag())
    {
        ui.ctx().send_viewport_cmd(egui::ViewportCommand::StartDrag);
    }
}

/// Whether the interface stands in for a title bar to drag the window by:
/// macOS hides its title bar, and the Windows custom title bar replaces the
/// standard frame. Elsewhere the desktop's title bar does, and full screen
/// has none.
const fn titlebar_drags(on_macos: bool, custom_titlebar: bool, fullscreen: bool) -> bool {
    (on_macos || custom_titlebar) && !fullscreen
}

/// Without a native title bar, the top of every panel drags the window, not
/// only the page's top bar.
const fn titlebar_spans_window(on_macos: bool, custom_titlebar: bool) -> bool {
    on_macos || custom_titlebar
}

const WINDOW_RESIZE_BORDER: f32 = 5.0;
const WINDOW_RESIZE_CORNER: f32 = 10.0;
const WINDOWS_WINDOW_CONTROLS_WIDTH: f32 = 3.0 * 36.0 + WINDOW_RESIZE_BORDER;
const WINDOWS_WINDOW_CONTROLS_HEIGHT: f32 = 36.0 + WINDOW_RESIZE_BORDER;
// The 760-point minimum with the default 250-point sidebar leaves 510 points.
const WINDOWS_MIN_INLINE_TOPBAR_WIDTH: f32 = 510.0 + WINDOWS_WINDOW_CONTROLS_WIDTH;

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct WindowControlsReservation {
    pub topbar_width: f32,
    pub topbar_top: f32,
    pub queue_top: f32,
    pub lyrics_top: f32,
}

const fn windows_chrome_visible(on_windows: bool, fullscreen: bool) -> bool {
    on_windows && !fullscreen
}

fn windows_chrome_visible_here(ctx: &egui::Context) -> bool {
    let fullscreen = ctx.input(|input| input.viewport().fullscreen.unwrap_or(false));
    windows_chrome_visible(crate::window::custom_titlebar(), fullscreen)
}

const fn windows_controls_reservation(
    on_windows: bool,
    fullscreen: bool,
    queue: bool,
    lyrics: bool,
    topbar_width: f32,
) -> WindowControlsReservation {
    let mut space = WindowControlsReservation {
        topbar_width: 0.0,
        topbar_top: 0.0,
        queue_top: 0.0,
        lyrics_top: 0.0,
    };
    if windows_chrome_visible(on_windows, fullscreen) {
        if queue {
            space.queue_top = WINDOWS_WINDOW_CONTROLS_HEIGHT;
        } else if lyrics {
            space.lyrics_top = WINDOWS_WINDOW_CONTROLS_HEIGHT;
        } else if topbar_width < WINDOWS_MIN_INLINE_TOPBAR_WIDTH {
            space.topbar_top = WINDOWS_WINDOW_CONTROLS_HEIGHT;
        } else {
            space.topbar_width = WINDOWS_WINDOW_CONTROLS_WIDTH;
        }
    }
    space
}

pub(super) fn window_controls_reservation(
    ctx: &egui::Context,
    queue: bool,
    lyrics: bool,
    topbar_width: f32,
) -> WindowControlsReservation {
    let fullscreen = ctx.input(|input| input.viewport().fullscreen.unwrap_or(false));
    windows_controls_reservation(
        crate::window::custom_titlebar(),
        fullscreen,
        queue,
        lyrics,
        topbar_width,
    )
}

/// Draws the Windows caption controls over the outermost top-right header.
pub fn window_controls(ui: &mut egui::Ui, palette: &theme::Palette) {
    if !windows_chrome_visible_here(ui.ctx()) {
        return;
    }
    let maximized = ui
        .ctx()
        .input(|input| input.viewport().maximized.unwrap_or(false));
    egui::Area::new(egui::Id::new("windows-window-controls"))
        .anchor(
            Align2::RIGHT_TOP,
            vec2(-WINDOW_RESIZE_BORDER, WINDOW_RESIZE_BORDER),
        )
        .order(egui::Order::Foreground)
        .show(ui.ctx(), |ui| {
            ui.spacing_mut().item_spacing.x = 0.0;
            ui.horizontal(|ui| {
                for (icon, tooltip, command) in [
                    (
                        Icon::Minus,
                        "Minimize",
                        egui::ViewportCommand::Minimized(true),
                    ),
                    (
                        if maximized { Icon::Copy } else { Icon::Square },
                        if maximized { "Restore" } else { "Maximize" },
                        egui::ViewportCommand::Maximized(!maximized),
                    ),
                    (Icon::X, "Close", egui::ViewportCommand::Close),
                ] {
                    let image = icon.image(palette.secondary, 14.0).alt_text(tooltip);
                    let button = egui::Button::image(image).frame_when_inactive(false);
                    if ui
                        .add_sized(egui::Vec2::splat(36.0), button)
                        .on_hover_text(tooltip)
                        .clicked()
                    {
                        ui.ctx().send_viewport_cmd(command);
                    }
                }
            });
        });
}

fn window_resize(ui: &mut egui::Ui) {
    let (fullscreen, maximized) = ui.ctx().input(|input| {
        (
            input.viewport().fullscreen.unwrap_or(false),
            input.viewport().maximized.unwrap_or(false),
        )
    });
    if !window_resize_enabled(crate::window::custom_titlebar(), fullscreen, maximized) {
        return;
    }

    let Some(position) = ui.input(|input| input.pointer.hover_pos()) else {
        return;
    };
    let Some(direction) = resize_direction(ui.ctx().content_rect(), position) else {
        return;
    };
    ui.ctx().set_cursor_icon(direction.1);
    if ui.input(|input| input.pointer.primary_pressed()) {
        ui.ctx()
            .send_viewport_cmd(egui::ViewportCommand::BeginResize(direction.0));
    }
}

const fn window_resize_enabled(on_windows: bool, fullscreen: bool, maximized: bool) -> bool {
    on_windows && !fullscreen && !maximized
}

fn resize_direction(
    window: Rect,
    position: egui::Pos2,
) -> Option<(egui::ResizeDirection, egui::CursorIcon)> {
    if !window.contains(position) {
        return None;
    }
    let [left, right, top, bottom] = [
        position.x - window.left(),
        window.right() - position.x,
        position.y - window.top(),
        window.bottom() - position.y,
    ];
    let mut x = i8::from(right <= WINDOW_RESIZE_BORDER) - i8::from(left <= WINDOW_RESIZE_BORDER);
    let mut y = i8::from(bottom <= WINDOW_RESIZE_BORDER) - i8::from(top <= WINDOW_RESIZE_BORDER);
    if y != 0 {
        x = i8::from(right <= WINDOW_RESIZE_CORNER) - i8::from(left <= WINDOW_RESIZE_CORNER);
    }
    if x != 0 {
        y = i8::from(bottom <= WINDOW_RESIZE_CORNER) - i8::from(top <= WINDOW_RESIZE_CORNER);
    }
    use egui::{CursorIcon as C, ResizeDirection as D};
    match (x, y) {
        (-1, -1) => Some((D::NorthWest, C::ResizeNwSe)),
        (1, -1) => Some((D::NorthEast, C::ResizeNeSw)),
        (-1, 1) => Some((D::SouthWest, C::ResizeNeSw)),
        (1, 1) => Some((D::SouthEast, C::ResizeNwSe)),
        (-1, 0) => Some((D::West, C::ResizeHorizontal)),
        (1, 0) => Some((D::East, C::ResizeHorizontal)),
        (0, -1) => Some((D::North, C::ResizeVertical)),
        (0, 1) => Some((D::South, C::ResizeVertical)),
        _ => None,
    }
}

pub fn blend(base: Color32, tint: Color32, amount: f32) -> Color32 {
    let a = egui::Rgba::from(base);
    let b = egui::Rgba::from(tint);
    let mixed = a * (1.0 - amount) + b * amount;
    let mut color = Color32::from(mixed);
    color[3] = 255;
    color
}

fn toasts(app: &mut App, ctx: &egui::Context, bottom_offset: f32) {
    if app.toasts.is_empty() {
        return;
    }
    let palette = app.palette;
    egui::Area::new(egui::Id::new("toasts"))
        .anchor(Align2::RIGHT_BOTTOM, vec2(-20.0, -bottom_offset))
        .order(egui::Order::Tooltip)
        .interactable(false)
        .show(ctx, |ui| {
            ui.spacing_mut().item_spacing.y = 8.0;
            for toast in &app.toasts {
                let age = toast.created.elapsed().as_secs_f32();
                let alpha = if age < 0.15 {
                    age / 0.15
                } else if age > 2.8 {
                    ((3.2 - age) / 0.4).clamp(0.0, 1.0)
                } else {
                    1.0
                };
                ui.set_opacity(alpha);
                Frame::new()
                    .fill(palette.overlay)
                    .stroke(Stroke::new(1.0, palette.outline))
                    .corner_radius(CornerRadius::same(theme::RADIUS))
                    .inner_margin(Margin::symmetric(14, 10))
                    .shadow(egui::epaint::Shadow {
                        offset: [0, 4],
                        blur: 16,
                        spread: 0,
                        color: palette.shadow,
                    })
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            let (icon, color) = match toast.kind {
                                ToastKind::Info => (Icon::CircleCheck, palette.accent),
                                ToastKind::Error => (Icon::CircleAlert, palette.danger),
                            };
                            theme::icon(ui, icon, 16.0, color);
                            // Laid out at its own width. The area
                            // remembers its size, so after a short toast a
                            // label left to wrap at the area's width broke
                            // long messages on every word.
                            let galley = ui.painter().layout(
                                toast.message.clone(),
                                theme::medium(13.5),
                                palette.text,
                                280.0,
                            );
                            ui.add(egui::Label::new(galley));
                        });
                    });
            }
        });
}

#[cfg(test)]
mod window_chrome_tests {
    use super::*;

    /// A window with the custom title bar has no frame, so its top bar is the
    /// only thing to drag it by. Keying this on the macOS title-bar inset,
    /// which is zero on Windows, once left it immovable.
    #[test]
    fn the_borderless_windows_window_drags_by_its_top_bar() {
        assert!(titlebar_drags(false, true, false));
        assert!(titlebar_drags(true, false, false));
        assert!(!titlebar_drags(false, true, true));
        assert!(!titlebar_drags(true, false, true));
        assert!(!titlebar_drags(false, false, false));
    }

    #[test]
    fn the_side_panels_drag_a_window_without_a_native_title_bar() {
        assert!(titlebar_spans_window(true, false));
        assert!(titlebar_spans_window(false, true));
        assert!(!titlebar_spans_window(false, false));
    }

    #[test]
    fn chrome_visibility_matches_window_state() {
        assert!(windows_chrome_visible(true, false));
        assert!(!windows_chrome_visible(true, true));
        assert!(!windows_chrome_visible(false, false));
        assert!(window_resize_enabled(true, false, false));
        assert!(!window_resize_enabled(true, false, true));
        assert!(!window_resize_enabled(true, true, false));
        assert!(!window_resize_enabled(false, false, false));
    }

    #[test]
    fn caption_space_belongs_to_the_outermost_header() {
        let values = |queue, lyrics| {
            let space = windows_controls_reservation(true, false, queue, lyrics, f32::INFINITY);
            [
                space.topbar_width,
                space.topbar_top,
                space.queue_top,
                space.lyrics_top,
            ]
        };
        assert_eq!(
            values(false, false),
            [WINDOWS_WINDOW_CONTROLS_WIDTH, 0.0, 0.0, 0.0]
        );
        assert_eq!(
            values(true, false),
            [0.0, 0.0, WINDOWS_WINDOW_CONTROLS_HEIGHT, 0.0]
        );
        assert_eq!(
            values(false, true),
            [0.0, 0.0, 0.0, WINDOWS_WINDOW_CONTROLS_HEIGHT]
        );
        assert_eq!(
            values(true, true),
            [0.0, 0.0, WINDOWS_WINDOW_CONTROLS_HEIGHT, 0.0]
        );
        assert_eq!(
            windows_controls_reservation(true, true, true, true, f32::INFINITY),
            WindowControlsReservation {
                topbar_width: 0.0,
                topbar_top: 0.0,
                queue_top: 0.0,
                lyrics_top: 0.0,
            }
        );
    }

    #[test]
    fn minimum_windows_window_stacks_caption_space_above_the_topbar() {
        let available = 760.0 - 250.0;
        let space = windows_controls_reservation(true, false, false, false, available);
        assert_eq!(space.topbar_width, 0.0);
        assert_eq!(space.topbar_top, WINDOWS_WINDOW_CONTROLS_HEIGHT);

        let inline = windows_controls_reservation(
            true,
            false,
            false,
            false,
            WINDOWS_MIN_INLINE_TOPBAR_WIDTH,
        );
        assert_eq!(inline.topbar_width, WINDOWS_WINDOW_CONTROLS_WIDTH);
        assert_eq!(inline.topbar_top, 0.0);
    }

    #[test]
    fn resize_hit_test_covers_edges_and_corners() {
        use egui::ResizeDirection as D;

        let window = Rect::from_min_max(egui::pos2(20.0, 30.0), egui::pos2(120.0, 110.0));
        for (position, expected) in [
            (egui::pos2(21.0, 31.0), Some(D::NorthWest)),
            (egui::pos2(70.0, 31.0), Some(D::North)),
            (egui::pos2(119.0, 31.0), Some(D::NorthEast)),
            (egui::pos2(21.0, 70.0), Some(D::West)),
            (egui::pos2(70.0, 70.0), None),
            (egui::pos2(119.0, 70.0), Some(D::East)),
            (egui::pos2(21.0, 109.0), Some(D::SouthWest)),
            (egui::pos2(70.0, 109.0), Some(D::South)),
            (egui::pos2(119.0, 109.0), Some(D::SouthEast)),
        ] {
            assert_eq!(
                resize_direction(window, position).map(|hit| hit.0),
                expected
            );
        }
    }
}
