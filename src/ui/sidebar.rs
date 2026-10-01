//! The left panel: navigation and Your Library.

use egui::{Align, CornerRadius, Frame, Layout, Margin, Rect, Sense, Vec2, pos2, vec2};

use crate::api::models::pick_image;
use crate::app::App;
use crate::model::{Action, Dialog, DragEntry, DragTrack, Loadable, Page};
use crate::settings::{LIKED_SONGS_KEY, LibraryShelf as Filter, LibrarySort};
use crate::theme::{self, Icon, Palette};

const DEFAULT_ROW_HEIGHT: f32 = 60.0;
const COMPACT_ROW_HEIGHT: f32 = 32.0;

struct Entry {
    image: Option<String>,
    /// Larger art for a grid card.
    grid_image: Option<String>,
    name: String,
    subtitle: String,
    /// The shorter line under a grid card's name.
    grid_subtitle: String,
    /// Who made it, for the Creator sort: a playlist's owner, an album's
    /// artists.
    creator: String,
    page: Page,
    uri: String,
    round: bool,
    liked: bool,
    owned: bool,
    playlist_index: Option<usize>,
    /// A folder row: its rootlist id, whether it is rolled up, and how
    /// many playlists it holds.
    folder: Option<(String, bool, usize)>,
    /// How deep inside folders the row sits, for the indent.
    depth: u8,
    /// When it was starred, in milliseconds, for the Recently added sort.
    added_at: Option<i64>,
}

impl Entry {
    /// The row's key among the pins and in the custom order. Liked Songs
    /// has no URI of its own, so it goes by the starred songs' context.
    fn ordering_key(&self) -> &str {
        if self.liked {
            LIKED_SONGS_KEY
        } else {
            &self.uri
        }
    }
}

fn liked_entry(app: &App) -> Entry {
    Entry {
        image: None,
        grid_image: None,
        name: "Liked Songs".into(),
        subtitle: match app.library.liked.total {
            Some(total) => format!("Playlist • {total} songs"),
            None => "Playlist".into(),
        },
        grid_subtitle: match app.library.liked.total {
            Some(1) => "1 song".into(),
            Some(total) => format!("{total} songs"),
            None => String::new(),
        },
        creator: String::new(),
        page: Page::LikedSongs,
        uri: String::new(),
        round: false,
        liked: true,
        owned: false,
        playlist_index: None,
        folder: None,
        depth: 0,
        added_at: None,
    }
}

/// The order a section shows: the one chosen for it, or what it showed
/// before orders could be chosen. Playlists keep a saved arrangement, or
/// else put the recently played first; albums and artists keep the
/// server's order.
fn selected_sort(app: &App, shelf: Filter) -> LibrarySort {
    if let Some(sort) = app.settings.library_sort.get(&shelf).copied()
        && sort.supports(shelf)
    {
        return sort;
    }
    if shelf != Filter::Playlists {
        LibrarySort::Library
    } else if !app.settings.sidebar_order.is_empty() {
        LibrarySort::Local
    } else {
        LibrarySort::RecentlyPlayed
    }
}

fn sort_label(shelf: Filter, sort: LibrarySort) -> &'static str {
    match sort {
        LibrarySort::Library => "Library order",
        LibrarySort::RecentlyPlayed => "Recently played",
        LibrarySort::Name => "Name",
        LibrarySort::Creator if shelf == Filter::Albums => "Artist",
        LibrarySort::Creator => "Creator",
        LibrarySort::RecentlyAdded => "Recently added",
        LibrarySort::Local => "Custom order",
    }
}

fn sort_menu(app: &mut App, ui: &mut egui::Ui, shelf: Filter, selected: LibrarySort) {
    // Playlists leave out the server's order, which nobody chose, and the
    // custom order until a row has been dragged into one.
    let choices: Vec<LibrarySort> = [
        LibrarySort::Library,
        LibrarySort::RecentlyPlayed,
        LibrarySort::Name,
        LibrarySort::Creator,
        LibrarySort::RecentlyAdded,
        LibrarySort::Local,
    ]
    .into_iter()
    .filter(|sort| {
        sort.supports(shelf)
            && !(*sort == LibrarySort::Library && shelf == Filter::Playlists)
            && !(*sort == LibrarySort::Local && app.settings.sidebar_order.is_empty())
    })
    .collect();
    ui.add_space(4.0);
    let response = ui.add(
        egui::Button::image_and_text(
            Icon::ChevronDown.image(app.palette.text, 15.0),
            egui::RichText::new(sort_label(shelf, selected)).font(theme::medium(13.0)),
        )
        .wrap()
        .fill(app.palette.surface)
        .corner_radius(12)
        .min_size(vec2(0.0, 28.0)),
    );
    egui::Popup::menu(&response)
        .frame(super::widgets::menu_frame(&app.palette))
        .show(|ui| {
            ui.set_min_width(200.0);
            ui.set_max_width(300.0);
            for sort in choices {
                if super::widgets::menu_item(
                    ui,
                    &app.palette,
                    (sort == selected).then_some(Icon::Check),
                    sort_label(shelf, sort),
                ) {
                    app.actions.push(Action::SetLibrarySort { shelf, sort });
                }
            }
        });
}

fn saved_time(value: Option<&str>) -> Option<i64> {
    value
        .and_then(|text| text.parse::<jiff::Timestamp>().ok())
        .map(|time| time.as_millisecond())
}

/// Orders the entries by `sort`, then lifts the pins, Liked Songs among
/// them while it is pinned, above the rest. Sorts are stable, so ties keep
/// the server's order.
fn order_entries(app: &App, sort: LibrarySort, entries: &mut [Entry]) {
    match sort {
        LibrarySort::Name => {
            entries.sort_by_cached_key(|entry| (entry.name.to_lowercase(), entry.uri.clone()));
        }
        // Nameless creators last, as undated entries are below.
        LibrarySort::Creator => entries.sort_by_cached_key(|entry| {
            (
                entry.creator.is_empty(),
                entry.creator.to_lowercase(),
                entry.name.to_lowercase(),
            )
        }),
        LibrarySort::RecentlyPlayed => entries.sort_by_key(|entry| {
            app.recent_contexts
                .iter()
                .position(|held| held == entry.ordering_key())
                .unwrap_or(usize::MAX)
        }),
        LibrarySort::RecentlyAdded => entries
            .sort_by_key(|entry| (entry.added_at.is_none(), std::cmp::Reverse(entry.added_at))),
        // Playlists the saved arrangement has not met yet come first.
        LibrarySort::Local => entries.sort_by_key(|entry| {
            match app
                .settings
                .sidebar_order
                .iter()
                .position(|held| held == entry.ordering_key())
            {
                Some(rank) => (1, rank),
                None => (0, entry.playlist_index.unwrap_or(0)),
            }
        }),
        LibrarySort::Library => {}
    }
    let pins = app.settings.library_pins();
    entries.sort_by_key(|entry| {
        pins.iter()
            .position(|held| held == entry.ordering_key())
            .unwrap_or(usize::MAX)
    });
}

fn playlist_entry(playlist: &crate::api::models::Playlist, index: usize, user_id: &str) -> Entry {
    Entry {
        image: pick_image(&playlist.images, 64).map(str::to_string),
        grid_image: pick_image(&playlist.images, super::GRID_ART_TARGET_WIDTH).map(str::to_string),
        name: playlist.name.clone(),
        subtitle: format!("Playlist • {}", playlist.owner_name()),
        grid_subtitle: playlist.owner_name().to_string(),
        creator: playlist.owner_name().to_string(),
        page: Page::Playlist(playlist.id.clone()),
        uri: playlist.uri.clone(),
        round: false,
        liked: false,
        owned: playlist.owned_by(user_id),
        playlist_index: Some(index),
        folder: None,
        depth: 0,
        added_at: None,
    }
}

/// The context a library row plays, matching the cover play button and the
/// right-click menu. Liked Songs plays the starred songs.
fn entry_play_uri(entry: &Entry) -> Option<String> {
    if entry.liked {
        Some(crate::api::subsonic::convert::COLLECTION_URI.to_string())
    } else if entry.uri.is_empty() {
        None
    } else {
        Some(entry.uri.clone())
    }
}

/// Whether `context`, the one playing, is the one this row plays. Liked
/// Songs has no URI of its own, so it lights from the starred songs.
fn entry_is_playing_context(entry: &Entry, context: Option<&str>) -> bool {
    context.is_some() && entry_play_uri(entry).as_deref() == context
}

/// What a screen reader calls the row or card.
fn entry_label(entry: &Entry) -> String {
    if let Some((_, collapsed, _)) = &entry.folder {
        format!(
            "{}, folder, {}",
            entry.name,
            if *collapsed { "collapsed" } else { "expanded" }
        )
    } else {
        entry.name.clone()
    }
}

/// A list row's cover offers to play the row while it is hovered. Returns
/// whether it took the click, so the row does not act on it too.
fn cover_play_button(
    app: &mut App,
    ui: &mut egui::Ui,
    entry: &Entry,
    index: usize,
    cover_rect: Rect,
    parent: &egui::Response,
) -> bool {
    let Some(uri) = entry_play_uri(entry) else {
        return false;
    };
    let play = ui.interact(
        cover_rect,
        ui.id().with(("sidebar-play", index)),
        Sense::click(),
    );
    let play_hover = play.hovered();
    if play_hover || parent.hovered() {
        ui.painter().rect_filled(
            cover_rect,
            CornerRadius::same(if entry.round { 22 } else { 6 }),
            egui::Color32::from_black_alpha(120),
        );
        Icon::PlayFilled
            .image(
                if play_hover {
                    app.palette.accent
                } else {
                    egui::Color32::WHITE
                },
                18.0,
            )
            .paint_at(
                ui,
                Rect::from_center_size(
                    cover_rect.center() + theme::play_glyph_offset(Icon::PlayFilled, 18.0),
                    Vec2::splat(18.0),
                ),
            );
    }
    if !play.clicked() {
        return false;
    }
    // The first click of a double click plays; the second must not play again.
    if !play.double_clicked() {
        app.actions.push(Action::PlayContext {
            uri,
            offset_uri: None,
            offset_index: None,
        });
    }
    true
}

fn grid_play_rect(cover_rect: Rect) -> Rect {
    let size = (cover_rect.width() * 0.3).clamp(32.0, 44.0);
    let inset = LIBRARY_ITEM_PADDING + size / 2.0;
    Rect::from_center_size(
        pos2(cover_rect.right() - inset, cover_rect.bottom() - inset),
        Vec2::splat(size),
    )
}

fn grid_pin_rect(cover_rect: Rect) -> Rect {
    Rect::from_center_size(cover_rect.left_top() + Vec2::splat(12.0), Vec2::splat(20.0))
}

/// A grid card's corner button: it plays the card, or pauses and resumes
/// it while it is the one playing. Returns whether it took the click.
fn grid_play_button(
    app: &mut App,
    ui: &mut egui::Ui,
    entry: &Entry,
    cover_rect: Rect,
    parent: &egui::Response,
    playing_here: bool,
) -> bool {
    let Some(uri) = entry_play_uri(entry) else {
        return false;
    };
    let rect = grid_play_rect(cover_rect);
    let size = rect.width();
    let button = ui.interact(rect, ui.id().with("library-grid-play"), Sense::click());
    let playing = playing_here && app.believed_playing();
    let label = format!("{} {}", if playing { "Pause" } else { "Play" }, entry.name);
    button.widget_info(|| {
        egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), label.clone())
    });
    if parent.hovered() || playing_here || button.has_focus() {
        let fill = if button.hovered() {
            app.palette.accent_hover
        } else {
            app.palette.accent
        };
        ui.painter().add(
            egui::epaint::Shadow {
                offset: [0, 4],
                blur: 12,
                spread: 0,
                color: egui::Color32::from_black_alpha(90),
            }
            .as_shape(rect, egui::CornerRadius::same(127)),
        );
        ui.painter().circle_filled(rect.center(), size / 2.0, fill);
        let icon = if playing {
            Icon::PauseFilled
        } else {
            Icon::PlayFilled
        };
        let icon_size = size * 0.42;
        icon.image(app.palette.on_accent, icon_size).paint_at(
            ui,
            Rect::from_center_size(
                rect.center() + theme::play_glyph_offset(icon, icon_size),
                Vec2::splat(icon_size),
            ),
        );
    }
    theme::focus_ring(ui, &button);
    if !button.clicked() {
        return false;
    }
    if playing_here {
        app.actions.push(Action::TogglePlay);
    } else {
        app.actions.push(Action::PlayContext {
            uri,
            offset_uri: None,
            offset_index: None,
        });
    }
    true
}

/// What a row and a card share once drawn: dragging it, dropping a song
/// on it, opening it, its menu and middle-click autoscroll. A list row
/// also plays on a double click; a card has its play button for that.
fn finish_entry_interaction(
    app: &mut App,
    ui: &mut egui::Ui,
    response: &egui::Response,
    entry: &Entry,
    cover_took_click: bool,
    custom_order: bool,
    drop_allowed: bool,
) {
    // Start reordering after the drag threshold.
    if !entry.ordering_key().is_empty() && response.drag_started_by(egui::PointerButton::Primary) {
        egui::DragAndDrop::set_payload(
            ui.ctx(),
            DragEntry {
                uri: entry.ordering_key().to_string(),
                title: entry.name.clone(),
                image: entry.image.clone(),
            },
        );
    }
    if drop_allowed
        && (entry.liked || entry.owned)
        && egui::DragAndDrop::has_payload_of_type::<DragTrack>(ui.ctx())
        && let Some(track) = response.dnd_release_payload::<DragTrack>()
    {
        if entry.liked {
            // Dropping on Liked Songs saves every dragged song; songs
            // already saved stay saved.
            app.actions.push(Action::SetSavedMany {
                uris: track.uris(),
                saved: true,
            });
        } else if let Page::Playlist(id) = &entry.page {
            app.actions.push(Action::AddToPlaylist {
                playlist_id: id.clone(),
                playlist_name: entry.name.clone(),
                uris: track.uris(),
            });
        }
    }
    theme::focus_ring(ui, response);
    if response.clicked() && !cover_took_click {
        app.actions.push(Action::Open(entry.page.clone()));
    }
    // A double click plays the row's context; the cover button handled its
    // own click.
    if !app.settings.sidebar_grid
        && response.double_clicked()
        && !cover_took_click
        && let Some(uri) = entry_play_uri(entry)
    {
        app.actions.push(Action::PlayContext {
            uri,
            offset_uri: None,
            offset_index: None,
        });
    }
    entry_menu(app, response, entry, custom_order);
    crate::autoscroll::row(ui, response);
}

pub fn show(app: &mut App, ui: &mut egui::Ui) {
    let palette = app.palette;
    let expanded_art = has_expanded_art(app);
    let floating_art = app.settings.sidebar_grid && expanded_art;
    // The traffic lights float over the top-left of the sidebar now, so the
    // first nav row has to start below them.
    let top = 12 + theme::titlebar_inset(ui.ctx()) as i8;
    let panel = egui::Panel::left("sidebar")
        .resizable(true)
        .default_size(app.settings.sidebar_width)
        .size_range(210.0..=440.0)
        .show_separator_line(false)
        .frame(Frame::new().fill(palette.panel).inner_margin(Margin {
            left: 12,
            right: 8,
            top,
            bottom: if expanded_art { 0 } else { 8 },
        }));
    let response = panel.show(ui, |ui| {
        // The list stops above the artwork; the grid scrolls on under it.
        let art_rect = expanded_art.then(|| expanded_art_rect(ui));
        if let Some(rect) = art_rect.filter(|_| !floating_art) {
            reserve_expanded_art(ui, rect);
        }
        contents(app, ui, art_rect.filter(|_| floating_art));
        if let Some(rect) = art_rect {
            if floating_art {
                paint_grid_art_mask(app, ui, rect);
            }
            paint_expanded_art(app, ui, rect);
        }
    });
    let width = response.response.rect.width();
    if (width - app.settings.sidebar_width).abs() > 1.0 {
        app.settings.sidebar_width = width;
        app.actions.push(Action::SettingsChanged);
    }
}

fn expanded_art_rect(ui: &egui::Ui) -> Rect {
    let side = expanded_art_side(ui);
    Rect::from_min_size(
        pos2(
            ui.max_rect().left(),
            ui.max_rect().bottom() - EXPANDED_ART_GAP - side,
        ),
        Vec2::splat(side),
    )
}

fn reserve_expanded_art(ui: &mut egui::Ui, rect: Rect) {
    egui::Panel::bottom("sidebar-art-space")
        .exact_size(rect.height() + EXPANDED_ART_GAP)
        .resizable(false)
        .show_separator_line(false)
        .frame(Frame::new())
        .show(ui, |_| {});
}

/// In grid mode the cards continue behind the artwork, fading out above it
/// and hidden in the gap beneath it.
fn paint_grid_art_mask(app: &App, ui: &egui::Ui, rect: Rect) {
    let bottom_fade_rect = Rect::from_min_max(
        pos2(ui.max_rect().left(), rect.bottom() - 64.0),
        pos2(ui.max_rect().right(), rect.bottom()),
    );
    super::widgets::paint_vertical_gradient(
        ui,
        bottom_fade_rect,
        egui::Color32::TRANSPARENT,
        app.palette.panel,
    );
    ui.painter().rect_filled(
        Rect::from_min_max(
            pos2(ui.max_rect().left(), rect.bottom()),
            ui.max_rect().right_bottom(),
        ),
        0.0,
        app.palette.panel,
    );
}

fn has_expanded_art(app: &App) -> bool {
    app.settings.art_expanded
        && app
            .now_playing()
            .is_some_and(|now| now.art_url.is_some() || now.art_small.is_some())
}

fn expanded_art_side(ui: &egui::Ui) -> f32 {
    ui.max_rect()
        .width()
        .min(ui.max_rect().height() * 0.45)
        .max(80.0)
}

/// Expanded album art at the bottom of the sidebar (#92).
fn paint_expanded_art(app: &mut App, ui: &mut egui::Ui, rect: Rect) {
    let Some(now) = app.now_playing() else {
        return;
    };
    let Some(url) = now.art_url.clone().or_else(|| now.art_small.clone()) else {
        return;
    };
    let album_id = now.album_id.clone();
    let palette = app.palette;
    ui.painter().add(
        egui::epaint::Shadow {
            offset: [0, 0],
            blur: 28,
            spread: 0,
            color: egui::Color32::from_black_alpha(if palette.dark { 120 } else { 40 }),
        }
        .as_shape(rect, egui::CornerRadius::same(8)),
    );
    super::widgets::paint_cover(
        ui,
        &palette,
        Some(&url),
        rect,
        8.0,
        Icon::Music,
        Some(app.backend.art()),
    );
    let art = ui.interact(rect, egui::Id::new("sidebar-art"), Sense::click());
    let chevron_rect = Rect::from_center_size(
        pos2(rect.right() - 16.0, rect.top() + 16.0),
        Vec2::splat(20.0),
    );
    let over_chevron = ui.rect_contains_pointer(chevron_rect);
    if art.hovered() || over_chevron {
        let chevron = ui.interact(
            chevron_rect,
            egui::Id::new("sidebar-art-collapse"),
            Sense::click(),
        );
        ui.painter().circle_filled(
            chevron_rect.center(),
            10.0,
            palette.panel.gamma_multiply(0.9),
        );
        Icon::ChevronDown.image(palette.text, 14.0).paint_at(
            ui,
            Rect::from_center_size(chevron_rect.center(), Vec2::splat(14.0)),
        );
        if chevron.clicked() {
            app.settings.art_expanded = false;
            app.actions.push(Action::SettingsChanged);
        }
    }
    if art.clicked()
        && !over_chevron
        && let Some(id) = album_id
    {
        app.actions.push(Action::Open(Page::Album(id)));
    }
}

fn nav_row(
    ui: &mut egui::Ui,
    palette: &Palette,
    icon: Icon,
    label: &str,
    active: bool,
) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), 40.0), Sense::click());
    if ui.is_rect_visible(rect) {
        let color = if active || response.hovered() {
            palette.text
        } else {
            palette.secondary
        };
        let icon_rect =
            Rect::from_center_size(pos2(rect.left() + 22.0, rect.center().y), Vec2::splat(22.0));
        icon.image(color, 22.0).paint_at(ui, icon_rect);
        ui.painter().text(
            pos2(rect.left() + 46.0, rect.center().y),
            egui::Align2::LEFT_CENTER,
            label,
            theme::bold(15.0),
            color,
        );
    }
    response.widget_info(|| {
        egui::WidgetInfo::selected(egui::WidgetType::Button, ui.is_enabled(), active, label)
    });
    theme::focus_ring(ui, &response);
    response
}

fn contents(app: &mut App, ui: &mut egui::Ui, grid_art: Option<Rect>) {
    let palette = app.palette;
    let page = app.page().clone();
    ui.add_space(4.0);
    if nav_row(ui, &palette, Icon::House, "Home", page == Page::Home).clicked() {
        app.actions.push(Action::Open(Page::Home));
    }
    if nav_row(ui, &palette, Icon::Search, "Search", page == Page::Search).clicked() {
        app.actions.push(Action::FocusSearch);
    }
    ui.add_space(10.0);
    ui.painter().hline(
        ui.max_rect().x_range().shrink(4.0),
        ui.cursor().top(),
        egui::Stroke::new(1.0, palette.outline),
    );
    ui.add_space(10.0);

    let filter_id = egui::Id::new("sidebar-filter");
    let mut filter = ui
        .data(|data| data.get_temp::<Filter>(filter_id))
        .unwrap_or_default();
    let show_search_id = egui::Id::new("sidebar-show-search");
    let mut show_search = ui
        .data(|data| data.get_temp::<bool>(show_search_id))
        .unwrap_or(false);

    let mut focus_search = false;

    ui.horizontal(|ui| {
        ui.add_space(6.0);
        theme::icon(ui, Icon::Library, 22.0, palette.secondary);
        ui.add_space(2.0);
        theme::text(ui, "Library", theme::bold(15.0), palette.text);
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            if theme::icon_button(
                ui,
                Icon::PanelLeft,
                16.0,
                palette.secondary,
                palette.text,
                super::keys::platform_shortcut("Hide sidebar (Ctrl+B)", "Hide sidebar (Cmd+B)"),
            )
            .clicked()
            {
                app.actions.push(Action::ToggleSidebar);
            }
            let grid = app.settings.sidebar_grid;
            let (icon, label) = if grid {
                (Icon::LayoutList, "Show as list")
            } else {
                (Icon::LayoutGrid, "Show as grid")
            };
            if theme::icon_button(ui, icon, 16.0, palette.secondary, palette.text, label).clicked()
            {
                app.actions.push(Action::SetLibraryGrid(!grid));
            }
            // One item never deserved a menu: the plus creates directly.
            if theme::icon_button(
                ui,
                Icon::Plus,
                16.0,
                palette.secondary,
                palette.text,
                "Create a playlist",
            )
            .clicked()
            {
                app.actions.push(Action::ShowDialog(Dialog::CreatePlaylist {
                    name: String::new(),
                    public: false,
                    add_uris: Vec::new(),
                }));
            }
            if theme::icon_button(
                ui,
                Icon::Search,
                16.0,
                palette.secondary,
                palette.text,
                "Search Your Library",
            )
            .clicked()
            {
                show_search = !show_search;
                if show_search {
                    focus_search = true;
                } else {
                    app.library.filter.clear();
                }
            }
        });
    });
    ui.add_space(6.0);

    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = vec2(6.0, 6.0);
        for (value, label) in [
            (Filter::Playlists, "Playlists"),
            (Filter::Albums, "Albums"),
            (Filter::Artists, "Artists"),
        ] {
            if theme::soft_button(ui, &palette, None, label, filter == value).clicked() {
                filter = value;
            }
        }
    });
    let sort = selected_sort(app, filter);
    sort_menu(app, ui, filter, sort);
    ui.data_mut(|data| {
        data.insert_temp(filter_id, filter);
        data.insert_temp(show_search_id, show_search);
    });
    if show_search {
        ui.add_space(4.0);
        let response = super::widgets::search_field(
            ui,
            &palette,
            egui::Id::new("sidebar-search"),
            &mut app.library.filter,
            "Search in Your Library",
            ui.available_width() - 4.0,
        );
        if focus_search {
            response.request_focus();
        }
    }
    ui.add_space(6.0);

    // Make sure the selected shelf is loading. Any order but the server's
    // needs the whole shelf, so it loads the rest a page at a time; a page
    // that failed waits for the order to be chosen again.
    match filter {
        Filter::Playlists => {}
        Filter::Albums => {
            let albums = &app.library.albums;
            if !albums.loading
                && albums.error.is_none()
                && (!albums.loaded_once || (sort != LibrarySort::Library && albums.can_load_more()))
            {
                app.actions.push(Action::LoadMore(Page::Albums));
            }
        }
        Filter::Artists => {
            let artists = &app.library.artists;
            if !artists.loading
                && artists.error.is_none()
                && (!artists.loaded_once
                    || (sort != LibrarySort::Library && artists.can_load_more()))
            {
                app.actions.push(Action::LoadMore(Page::Artists));
            }
        }
    }

    let needle = app.library.filter.trim().to_lowercase();
    let user_id = app.user_id().unwrap_or("").to_string();
    let mut entries: Vec<Entry> = Vec::new();
    let mut loading = false;
    let mut error: Option<String> = None;
    let mut more_page: Option<Page> = None;
    match filter {
        Filter::Playlists => {
            let liked = liked_entry(app);
            if needle.is_empty() || liked.name.to_lowercase().contains(&needle) {
                entries.push(liked);
            }
            match &app.library.playlists {
                Loadable::Loaded(playlists) => {
                    for (index, playlist) in playlists.iter().enumerate() {
                        if !needle.is_empty() && !playlist.name.to_lowercase().contains(&needle) {
                            continue;
                        }
                        entries.push(playlist_entry(playlist, index, &user_id));
                    }
                }
                Loadable::Loading | Loadable::NotLoaded => loading = true,
                Loadable::Failed(message) => error = Some(message.clone()),
            }
        }
        Filter::Albums => {
            for saved in &app.library.albums.items {
                let album = &saved.album;
                if !needle.is_empty()
                    && !album.name.to_lowercase().contains(&needle)
                    && !album
                        .artists
                        .iter()
                        .any(|a| a.name.to_lowercase().contains(&needle))
                {
                    continue;
                }
                let artists = crate::api::models::join_names(
                    album.artists.iter().map(|artist| artist.name.as_str()),
                );
                entries.push(Entry {
                    image: pick_image(&album.images, 64).map(str::to_string),
                    grid_image: pick_image(&album.images, super::GRID_ART_TARGET_WIDTH)
                        .map(str::to_string),
                    name: album.name.clone(),
                    subtitle: format!("{} • {artists}", album.kind_label()),
                    grid_subtitle: artists.clone(),
                    creator: artists,
                    page: Page::Album(album.id.clone()),
                    uri: album.uri.clone(),
                    round: false,
                    liked: false,
                    owned: false,
                    playlist_index: None,
                    folder: None,
                    depth: 0,
                    added_at: saved_time(saved.added_at.as_deref()),
                });
            }
            loading = app.library.albums.loading && app.library.albums.items.is_empty();
            error = app.library.albums.error.clone();
            if app.library.albums.error.is_none() && app.library.albums.can_load_more() {
                more_page = Some(Page::Albums);
            }
        }
        Filter::Artists => {
            for artist in &app.library.artists.items {
                if !needle.is_empty() && !artist.name.to_lowercase().contains(&needle) {
                    continue;
                }
                entries.push(Entry {
                    image: pick_image(&artist.images, 64).map(str::to_string),
                    grid_image: pick_image(&artist.images, super::GRID_ART_TARGET_WIDTH)
                        .map(str::to_string),
                    name: artist.name.clone(),
                    subtitle: "Artist".into(),
                    grid_subtitle: String::new(),
                    creator: String::new(),
                    page: Page::Artist(artist.id.clone()),
                    uri: artist.uri.clone(),
                    round: true,
                    liked: false,
                    owned: false,
                    playlist_index: None,
                    folder: None,
                    depth: 0,
                    added_at: None,
                });
            }
            loading = app.library.artists.loading && app.library.artists.items.is_empty();
            error = app.library.artists.error.clone();
            if app.library.artists.error.is_none() && app.library.artists.can_load_more() {
                more_page = Some(Page::Artists);
            }
        }
    }

    order_entries(app, sort, &mut entries);
    let custom_order = filter == Filter::Playlists && sort == LibrarySort::Local;
    // The pins form one block at the top; a dragged row lands in it or
    // below it.
    let pins = app.settings.library_pins();
    let pinned_rows = entries
        .iter()
        .take_while(|entry| pins.iter().any(|key| key == entry.ordering_key()))
        .count();
    let playing_context = app.playing_context_uri();
    let context_playing = app.believed_playing();
    let current_page = app.page().clone();

    crate::autoscroll::show(
        ui,
        egui::ScrollArea::vertical()
            .id_salt("sidebar-list")
            .auto_shrink([false, false]),
        egui::Vec2b::new(false, true),
        |ui| {
            if egui::DragAndDrop::has_payload_of_type::<DragTrack>(ui.ctx())
                || egui::DragAndDrop::has_payload_of_type::<DragEntry>(ui.ctx())
            {
                super::widgets::scroll_during_drag(ui);
            }
            if loading {
                super::widgets::loading_row(ui, &palette);
            }
            if let Some(error) = &error {
                super::widgets::error_row(ui, app, error, None);
            }
            if entries.is_empty() && !loading && error.is_none() {
                ui.add_space(12.0);
                theme::subtle(
                    ui,
                    &palette,
                    if needle.is_empty() {
                        "Nothing here yet."
                    } else {
                        "No matches."
                    },
                );
            }
            if app.settings.sidebar_grid {
                library_grid(
                    app,
                    ui,
                    &entries,
                    filter,
                    custom_order,
                    pinned_rows,
                    grid_art,
                );
                if let Some(page) = more_page {
                    super::widgets::load_more_when_near_end(ui, app, page, true);
                }
                // Room to scroll the last cards out from under the artwork.
                ui.add_space(grid_art.map_or(0.0, |rect| ui.max_rect().bottom() - rect.top()));
                return;
            }
            let compact = app.settings.sidebar_compact;
            let row_height = if compact {
                COMPACT_ROW_HEIGHT
            } else {
                DEFAULT_ROW_HEIGHT
            };
            // Calculate drop positions from fixed row height because rows shift
            // before drawing.
            let list_top = ui.cursor().top();
            let pointer = ui.ctx().pointer_latest_pos().filter(|pos| {
                ui.clip_rect().contains(*pos) && ui.rect_contains_pointer(ui.clip_rect())
            });
            // Tracks may drop on Liked Songs or owned playlists.
            let dragging_song = egui::DragAndDrop::has_payload_of_type::<DragTrack>(ui.ctx());
            let drop_target = dragging_song
                .then_some(pointer)
                .flatten()
                .map(|pos| ((pos.y - list_top) / row_height).floor())
                .filter(|row| *row >= 0.0 && *row < entries.len() as f32)
                .map(|row| row as usize)
                .filter(|row| entries[*row].liked || entries[*row].owned);
            // Sidebar entries, Liked Songs among them, drop between rows.
            let reordering = egui::DragAndDrop::has_payload_of_type::<DragEntry>(ui.ctx());
            let reorder_slot = reordering.then_some(pointer).flatten().map(|pos| {
                (((pos.y - list_top) / row_height).round().max(0.0) as usize).min(entries.len())
            });
            super::widgets::virtual_rows(ui, entries.len(), row_height, |ui, index| {
                let entry = &entries[index];
                let droppable = entry.liked || entry.owned;
                let drop_hover = drop_target == Some(index);
                let active = entry.folder.is_none() && entry.page == current_page;
                let playing =
                    context_playing && entry_is_playing_context(entry, playing_context.as_deref());
                let pinned = pins.iter().any(|key| key == entry.ordering_key());
                let (_, rect) = ui.allocate_space(vec2(ui.available_width(), row_height));
                let id = ui.id().with((
                    "library-row",
                    &entry.uri,
                    entry.liked,
                    entry.folder.as_ref().map(|(id, _, _)| id),
                ));
                let response = ui.interact(rect, id, Sense::click_and_drag());
                response.widget_info(|| {
                    egui::WidgetInfo::selected(
                        egui::WidgetType::Button,
                        ui.is_enabled(),
                        active,
                        entry_label(entry),
                    )
                });
                // Animate rows around the current track or entry drop target.
                let shift = ui.ctx().animate_value_with_time(
                    ui.id().with(("drop-shift", index)),
                    if let Some(slot) = reorder_slot {
                        if index < slot { -4.0 } else { 4.0 }
                    } else {
                        match drop_target {
                            Some(target) if index < target => -4.0,
                            Some(target) if index > target => 4.0,
                            _ => 0.0,
                        }
                    },
                    0.12,
                );
                let rect = rect.translate(vec2(0.0, shift));
                // Set when the cover play button takes a click, so a double
                // click on it does not also play from the row.
                let mut cover_took_click = false;
                if ui.is_rect_visible(rect) {
                    if active {
                        ui.painter()
                            .rect_filled(rect, CornerRadius::same(6), palette.surface);
                    } else if response.hovered() {
                        ui.painter().rect_filled(
                            rect,
                            CornerRadius::same(6),
                            palette.surface_hover.gamma_multiply(0.6),
                        );
                    }
                    if drop_hover {
                        ui.painter().rect_filled(
                            rect,
                            CornerRadius::same(6),
                            palette.accent.gamma_multiply(0.18),
                        );
                        ui.painter().rect_stroke(
                            rect,
                            CornerRadius::same(6),
                            egui::Stroke::new(1.5, palette.accent),
                            egui::StrokeKind::Inside,
                        );
                    }
                    let name_color = if playing {
                        palette.accent
                    } else {
                        palette.text
                    };
                    let indent = f32::from(entry.depth) * 14.0;
                    if let Some((_, collapsed, _)) = &entry.folder {
                        let chevron = if *collapsed {
                            Icon::ChevronRight
                        } else {
                            Icon::ChevronDown
                        };
                        let left = rect.left() + 8.0 + indent;
                        chevron.image(palette.secondary, 16.0).paint_at(
                            ui,
                            Rect::from_center_size(
                                pos2(left + 8.0, rect.center().y),
                                Vec2::splat(16.0),
                            ),
                        );
                        Icon::Library.image(palette.secondary, 20.0).paint_at(
                            ui,
                            Rect::from_center_size(
                                pos2(left + 30.0, rect.center().y),
                                Vec2::splat(20.0),
                            ),
                        );
                        let text_left = left + 46.0;
                        let text_right = rect.right() - 8.0;
                        let painter = ui.painter().with_clip_rect(Rect::from_min_max(
                            pos2(text_left, rect.top()),
                            pos2(text_right, rect.bottom()),
                        ));
                        crate::bidi::paint_line(
                            &painter,
                            text_left,
                            text_right,
                            rect.center().y - if compact { 0.0 } else { 9.0 },
                            &entry.name,
                            theme::medium(if compact { 13.5 } else { 14.0 }),
                            name_color,
                        );
                        if !compact {
                            crate::bidi::paint_line(
                                &painter,
                                text_left,
                                text_right,
                                rect.center().y + 10.0,
                                &entry.subtitle,
                                theme::regular(12.5),
                                palette.secondary,
                            );
                        }
                    } else if compact {
                        let text_left = rect.left() + 8.0 + indent;
                        let text_right = rect.right() - if playing || pinned { 28.0 } else { 8.0 };
                        let painter = ui.painter().with_clip_rect(Rect::from_min_max(
                            pos2(text_left, rect.top()),
                            pos2(text_right, rect.bottom()),
                        ));
                        crate::bidi::paint_line(
                            &painter,
                            text_left,
                            text_right,
                            rect.center().y,
                            &entry.name,
                            theme::medium(13.5),
                            name_color,
                        );
                    } else {
                        let cover_rect = Rect::from_center_size(
                            pos2(rect.left() + 8.0 + indent + 22.0, rect.center().y),
                            Vec2::splat(44.0),
                        );
                        if entry.liked {
                            liked_cover(ui, cover_rect, 6.0);
                        } else {
                            super::widgets::paint_cover(
                                ui,
                                &palette,
                                entry.image.as_deref(),
                                cover_rect,
                                if entry.round { 22.0 } else { 6.0 },
                                if entry.round { Icon::User } else { Icon::Music },
                                Some(app.backend.art()),
                            );
                        }
                        let text_left = cover_rect.right() + 12.0;
                        let text_right = rect.right() - if playing || pinned { 28.0 } else { 8.0 };
                        let painter = ui.painter().with_clip_rect(Rect::from_min_max(
                            pos2(text_left, rect.top()),
                            pos2(text_right, rect.bottom()),
                        ));
                        crate::bidi::paint_line(
                            &painter,
                            text_left,
                            text_right,
                            rect.center().y - 9.0,
                            &entry.name,
                            theme::medium(14.0),
                            name_color,
                        );
                        crate::bidi::paint_line(
                            &painter,
                            text_left,
                            text_right,
                            rect.center().y + 10.0,
                            &entry.subtitle,
                            theme::regular(12.5),
                            palette.secondary,
                        );
                        // Hovering the art offers to play right from here.
                        cover_took_click =
                            cover_play_button(app, ui, entry, index, cover_rect, &response);
                    }
                    if playing {
                        let icon_rect = Rect::from_center_size(
                            pos2(rect.right() - 16.0, rect.center().y),
                            Vec2::splat(16.0),
                        );
                        Icon::Volume2
                            .image(palette.accent, 16.0)
                            .paint_at(ui, icon_rect);
                    } else if pinned {
                        let icon_rect = Rect::from_center_size(
                            pos2(rect.right() - 16.0, rect.center().y),
                            Vec2::splat(13.0),
                        );
                        Icon::Pin
                            .image(palette.secondary, 13.0)
                            .paint_at(ui, icon_rect);
                    }
                    // Rows that cannot take the song step back a little.
                    if dragging_song && !droppable {
                        ui.painter().rect_filled(
                            rect,
                            CornerRadius::same(6),
                            palette.panel.gamma_multiply(0.5),
                        );
                    }
                }
                finish_entry_interaction(
                    app,
                    ui,
                    &response,
                    entry,
                    cover_took_click,
                    custom_order,
                    true,
                );
            });
            if let Some(slot) = reorder_slot {
                // A line in the gap the rows opened, so the eye lands
                // where the row will.
                let y = list_top + slot as f32 * row_height;
                ui.painter().hline(
                    ui.max_rect().x_range().shrink(6.0),
                    y,
                    egui::Stroke::new(2.0, palette.accent),
                );
                if ui.input(|input| input.pointer.button_released(egui::PointerButton::Primary))
                    && let Some(drag) = egui::DragAndDrop::take_payload::<DragEntry>(ui.ctx())
                {
                    if filter == Filter::Playlists {
                        drop_playlist_row(app, &entries, pinned_rows, slot, &drag.uri);
                    } else {
                        drop_row(app, &entries, pinned_rows, slot, &drag.uri);
                    }
                }
            }
            if let Some(page) = more_page {
                super::widgets::load_more_when_near_end(ui, app, page, true);
            }
        },
    );
}

const EXPANDED_ART_GAP: f32 = 10.0;
const LIBRARY_ITEM_PADDING: f32 = 8.0;
const GRID_MIN_CARD_WIDTH: f32 = 108.0;
const GRID_MAX_COLUMNS: usize = 4;
const GRID_TEXT_HEIGHT: f32 = 44.0;

#[derive(Clone, Copy)]
struct GridLayout {
    columns: usize,
    card_width: f32,
    card_height: f32,
    row_height: f32,
}

/// As many columns as cards of the smallest width fit, two at least, and
/// cards that share out the whole width.
fn grid_layout(width: f32) -> GridLayout {
    let columns = ((width / GRID_MIN_CARD_WIDTH).floor() as usize).clamp(2, GRID_MAX_COLUMNS);
    let card_width = (width / columns as f32).max(1.0);
    let card_height = card_width + GRID_TEXT_HEIGHT;
    GridLayout {
        columns,
        card_width,
        card_height,
        row_height: card_height,
    }
}

fn paint_grid_text(
    ui: &egui::Ui,
    rect: Rect,
    text: &str,
    font: egui::FontId,
    color: egui::Color32,
) {
    let galley = crate::bidi::layout(
        ui.painter(),
        text,
        font,
        color,
        rect.width(),
        1,
        Some(crate::bidi::ELLIPSIS),
    );
    ui.painter()
        .galley(crate::bidi::galley_pos(rect, &galley), galley, color);
}

/// The gap a card dragged to `pointer` lands in, counted in reading order:
/// before the card under the pointer, or after it past the card's middle.
fn grid_reorder_slot(
    pointer: egui::Pos2,
    origin: egui::Pos2,
    layout: GridLayout,
    count: usize,
) -> usize {
    let row = ((pointer.y - origin.y) / layout.row_height)
        .floor()
        .max(0.0) as usize;
    let column = ((pointer.x - origin.x) / layout.card_width)
        .floor()
        .max(0.0)
        .min((layout.columns - 1) as f32) as usize;
    let after = pointer.x - origin.x - column as f32 * layout.card_width > layout.card_width / 2.0;
    (row * layout.columns + column + usize::from(after)).min(count)
}

/// The Library as cover cards, row by row so only the visible ones are
/// laid out. `grid_art` is the expanded artwork floating over the cards,
/// which takes no drops meant for the card beneath it.
fn library_grid(
    app: &mut App,
    ui: &mut egui::Ui,
    entries: &[Entry],
    filter: Filter,
    custom_order: bool,
    pinned_rows: usize,
    grid_art: Option<Rect>,
) {
    if entries.is_empty() {
        return;
    }
    let palette = app.palette;
    let pins = app.settings.library_pins();
    let playing_context = app.playing_context_uri();
    let context_playing = app.believed_playing();
    let current_page = app.page().clone();
    let layout = grid_layout(ui.available_width());
    let origin = ui.cursor().min;
    let pointer = ui.ctx().pointer_latest_pos().filter(|pos| {
        ui.clip_rect().contains(*pos)
            && ui.rect_contains_pointer(ui.clip_rect())
            && !grid_art.is_some_and(|art| art.contains(*pos))
    });
    let dragging_song = egui::DragAndDrop::has_payload_of_type::<DragTrack>(ui.ctx());
    let reordering = egui::DragAndDrop::has_payload_of_type::<DragEntry>(ui.ctx());
    let reorder_slot = reordering
        .then_some(pointer)
        .flatten()
        .map(|pointer| grid_reorder_slot(pointer, origin, layout, entries.len()));
    let row_count = entries.len().div_ceil(layout.columns);
    let grid_id = ui.unique_id().with("library-grid");

    super::widgets::virtual_rows(ui, row_count, layout.row_height, |ui, row| {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 0.0;
            let start = row * layout.columns;
            let end = (start + layout.columns).min(entries.len());
            for index in start..end {
                let entry = &entries[index];
                let entry_id = (entry.ordering_key(), entry.liked);
                ui.scope_builder(egui::UiBuilder::new().id(grid_id.with(entry_id)), |ui| {
                    let active = entry.page == current_page;
                    let playing_here = entry_is_playing_context(entry, playing_context.as_deref());
                    let playing = context_playing && playing_here;
                    let pinned = pins.iter().any(|key| key == entry.ordering_key());
                    let droppable = entry.liked || entry.owned;
                    let (rect, response) = ui.allocate_exact_size(
                        vec2(layout.card_width, layout.card_height),
                        Sense::click_and_drag(),
                    );
                    response.widget_info(|| {
                        egui::WidgetInfo::selected(
                            egui::WidgetType::Button,
                            ui.is_enabled(),
                            active,
                            entry_label(entry),
                        )
                    });
                    let cover_rect = Rect::from_min_size(
                        rect.min + Vec2::splat(LIBRARY_ITEM_PADDING),
                        Vec2::splat(layout.card_width - LIBRARY_ITEM_PADDING * 2.0),
                    );
                    let mut cover_took_click = false;
                    if ui.is_rect_visible(rect) {
                        if active {
                            ui.painter()
                                .rect_filled(rect, CornerRadius::same(6), palette.surface);
                        } else if response.hovered() {
                            ui.painter().rect_filled(
                                rect,
                                CornerRadius::same(6),
                                palette.surface_hover.gamma_multiply(0.6),
                            );
                        }
                        if entry.liked {
                            liked_cover(ui, cover_rect, 6.0);
                        } else {
                            super::widgets::paint_cover(
                                ui,
                                &palette,
                                entry.grid_image.as_deref().or(entry.image.as_deref()),
                                cover_rect,
                                if entry.round {
                                    layout.card_width / 2.0
                                } else {
                                    6.0
                                },
                                if entry.round { Icon::User } else { Icon::Music },
                                Some(app.backend.art()),
                            );
                        }

                        cover_took_click =
                            grid_play_button(app, ui, entry, cover_rect, &response, playing_here);

                        let text_left = rect.left() + LIBRARY_ITEM_PADDING;
                        let text_width = (rect.width() - LIBRARY_ITEM_PADDING * 2.0).max(0.0);
                        paint_grid_text(
                            ui,
                            Rect::from_min_size(
                                pos2(text_left, cover_rect.bottom() + 4.0),
                                vec2(text_width, 18.0),
                            ),
                            &entry.name,
                            theme::medium(13.5),
                            if playing {
                                palette.accent
                            } else {
                                palette.text
                            },
                        );
                        paint_grid_text(
                            ui,
                            Rect::from_min_size(
                                pos2(text_left, cover_rect.bottom() + 23.0),
                                vec2(text_width, 17.0),
                            ),
                            &entry.grid_subtitle,
                            theme::regular(12.0),
                            palette.secondary,
                        );

                        if pinned {
                            let pin_rect = grid_pin_rect(cover_rect);
                            ui.painter().circle_filled(
                                pin_rect.center(),
                                pin_rect.width() / 2.0,
                                egui::Color32::from_black_alpha(170),
                            );
                            Icon::Pin.image(egui::Color32::WHITE, 12.0).paint_at(
                                ui,
                                Rect::from_center_size(pin_rect.center(), Vec2::splat(12.0)),
                            );
                        }
                        // Cards that cannot take the song step back; the one
                        // under the pointer that can is outlined. A drag keeps
                        // egui from reporting hovers, so this follows the
                        // pointer instead.
                        if dragging_song && !droppable {
                            ui.painter().rect_filled(
                                rect,
                                CornerRadius::same(6),
                                palette.panel.gamma_multiply(0.5),
                            );
                        } else if dragging_song
                            && droppable
                            && pointer.is_some_and(|pos| rect.contains(pos))
                        {
                            ui.painter().rect_stroke(
                                cover_rect,
                                CornerRadius::same(6),
                                egui::Stroke::new(2.0, palette.accent),
                                egui::StrokeKind::Inside,
                            );
                        }
                        if reorder_slot == Some(index) {
                            ui.painter().vline(
                                rect.left(),
                                rect.y_range(),
                                egui::Stroke::new(2.0, palette.accent),
                            );
                        }
                        if index + 1 == entries.len() && reorder_slot == Some(entries.len()) {
                            ui.painter().vline(
                                rect.right(),
                                rect.y_range(),
                                egui::Stroke::new(2.0, palette.accent),
                            );
                        }
                    }

                    finish_entry_interaction(
                        app,
                        ui,
                        &response,
                        entry,
                        cover_took_click,
                        custom_order,
                        pointer.is_some(),
                    );
                });
            }
        });
    });

    if let Some(slot) = reorder_slot
        && ui.input(|input| input.pointer.button_released(egui::PointerButton::Primary))
        && let Some(drag) = egui::DragAndDrop::take_payload::<DragEntry>(ui.ctx())
    {
        if filter == Filter::Playlists {
            drop_playlist_row(app, entries, pinned_rows, slot, &drag.uri);
        } else {
            drop_row(app, entries, pinned_rows, slot, &drag.uri);
        }
    }
}

fn entry_menu(app: &mut App, response: &egui::Response, entry: &Entry, custom_order: bool) {
    if !entry.uri.is_empty() {
        let owned_playlist = entry
            .owned
            .then_some(entry.playlist_index)
            .flatten()
            .and_then(|index| {
                app.library
                    .playlists
                    .get()
                    .and_then(|list| list.get(index))
                    .cloned()
            });
        egui::Popup::context_menu(response)
            .frame(super::widgets::menu_frame(&app.palette))
            .show(|ui| {
                super::widgets::context_menu_items(
                    ui,
                    app,
                    &entry.uri,
                    &entry.name,
                    owned_playlist.as_ref(),
                );
                pin_menu(app, ui, entry.ordering_key());
                if custom_order
                    && super::widgets::menu_item(
                        ui,
                        &app.palette,
                        Some(Icon::Clock),
                        "Sort by recently played",
                    )
                {
                    // The arrangement is kept for choosing again.
                    app.actions.push(Action::SetLibrarySort {
                        shelf: Filter::Playlists,
                        sort: LibrarySort::RecentlyPlayed,
                    });
                }
            });
    } else if entry.liked {
        egui::Popup::context_menu(response)
            .frame(super::widgets::menu_frame(&app.palette))
            .show(|ui| {
                // The same width as every other menu; without it the menu
                // stretches as wide as the window.
                ui.set_min_width(200.0);
                ui.set_max_width(300.0);
                if super::widgets::menu_item(ui, &app.palette, Some(Icon::Play), "Play") {
                    app.actions.push(Action::PlayContext {
                        uri: crate::api::subsonic::convert::COLLECTION_URI.to_string(),
                        offset_uri: None,
                        offset_index: None,
                    });
                }
                pin_menu(app, ui, entry.ordering_key());
            });
    }
}

fn pin_menu(app: &mut App, ui: &mut egui::Ui, key: &str) {
    let mut pins = app.settings.library_pins();
    let pinned = pins.iter().any(|held| held == key);
    if super::widgets::menu_item(
        ui,
        &app.palette,
        Some(if pinned { Icon::PinOff } else { Icon::Pin }),
        if pinned { "Unpin" } else { "Pin to top" },
    ) {
        if pinned {
            pins.retain(|held| held != key);
        } else {
            pins.push(key.to_string());
        }
        app.actions.push(Action::ArrangeLibrary {
            pinned: pins,
            playlist_order: None,
        });
    }
}

/// A dropped playlist row lands in one of two worlds. Inside the pin
/// block it pins, or reorders the pins, exactly where it fell; a pin
/// dropped on the block's lower edge stays pinned. Below the block it
/// unpins and orders the rest: the drop snapshots the whole order on
/// screen so nothing jumps, and rows then sit where they are put.
fn drop_playlist_row(app: &mut App, entries: &[Entry], pinned_rows: usize, slot: usize, key: &str) {
    if key != LIKED_SONGS_KEY
        && !app
            .library
            .playlists
            .get()
            .is_some_and(|playlists| playlists.iter().any(|playlist| playlist.uri == key))
    {
        return;
    }
    let was_pinned = app.settings.library_pins().iter().any(|held| held == key);
    if slot < pinned_rows || (was_pinned && slot == pinned_rows) {
        drop_row(app, entries, pinned_rows, slot, key);
        return;
    }
    let mut pins = app.settings.library_pins();
    pins.retain(|held| held != key);
    let mut order = full_playlist_order(app);
    let anchor = entries
        .iter()
        .skip(slot)
        .map(Entry::ordering_key)
        .find(|held| !held.is_empty() && *held != key)
        .map(str::to_string);
    order.retain(|held| held != key);
    let at = anchor
        .and_then(|anchor| order.iter().position(|held| *held == anchor))
        .unwrap_or(order.len());
    order.insert(at, key.to_string());
    app.actions.push(Action::ArrangeLibrary {
        pinned: pins,
        playlist_order: Some(order),
    });
}

/// Every unpinned playlist, and Liked Songs while it is unpinned, in the
/// order the shelf shows them, including rows a search hides. The first
/// drag snapshots this whole arrangement, so nothing on screen jumps and
/// the saved order covers the library rather than the rows that happened
/// to be visible.
fn full_playlist_order(app: &App) -> Vec<String> {
    let Some(playlists) = app.library.playlists.get() else {
        return Vec::new();
    };
    let user_id = app.user_id().unwrap_or("");
    let mut entries: Vec<_> = playlists
        .iter()
        .enumerate()
        .map(|(index, playlist)| playlist_entry(playlist, index, user_id))
        .collect();
    entries.push(liked_entry(app));
    order_entries(app, selected_sort(app, Filter::Playlists), &mut entries);
    let pins = app.settings.library_pins();
    entries
        .iter()
        .map(|entry| entry.ordering_key().to_string())
        // Pins live in their own list; the saved order holds the rest.
        .filter(|key| !pins.contains(key))
        .collect()
}

/// Arranges the pin block, or unpins a row dropped below it, without
/// disturbing the pins of another shelf.
fn drop_row(app: &mut App, entries: &[Entry], pinned_rows: usize, slot: usize, key: &str) {
    let mut pins = app.settings.library_pins();
    pins.retain(|held| held != key);
    if slot <= pinned_rows {
        // The pinned entry the drop lands in front of anchors the new
        // position, so entries pinned from another shelf keep theirs.
        let anchor = entries[..pinned_rows]
            .iter()
            .skip(slot)
            .map(Entry::ordering_key)
            .find(|held| *held != key);
        let at = anchor
            .and_then(|anchor| pins.iter().position(|held| held == anchor))
            .unwrap_or(pins.len());
        pins.insert(at, key.to_string());
    }
    app.actions.push(Action::ArrangeLibrary {
        pinned: pins,
        playlist_order: None,
    });
}

/// The purple-to-blue Liked Songs tile.
pub fn liked_cover(ui: &egui::Ui, rect: Rect, radius: f32) {
    let texture_id = egui::Id::new("liked-cover-gradient");
    let texture = ui
        .data(|data| data.get_temp::<egui::TextureHandle>(texture_id))
        .unwrap_or_else(|| {
            let size = 64;
            let lerp = |a: u8, b: u8, t: f32| (a as f32 + (b as f32 - a as f32) * t) as u8;
            let top_left = [0x45, 0x0a, 0xf5];
            let top_right = [0x6a, 0x3a, 0xe8];
            let bottom_left = [0x8e, 0x9f, 0xe5];
            let bottom_right = [0xc4, 0xef, 0xd9];
            let pixels = (0..size)
                .flat_map(|y| {
                    let y = y as f32 / (size - 1) as f32;
                    (0..size).map(move |x| {
                        let x = x as f32 / (size - 1) as f32;
                        egui::Color32::from_rgb(
                            lerp(
                                lerp(top_left[0], top_right[0], x),
                                lerp(bottom_left[0], bottom_right[0], x),
                                y,
                            ),
                            lerp(
                                lerp(top_left[1], top_right[1], x),
                                lerp(bottom_left[1], bottom_right[1], x),
                                y,
                            ),
                            lerp(
                                lerp(top_left[2], top_right[2], x),
                                lerp(bottom_left[2], bottom_right[2], x),
                                y,
                            ),
                        )
                    })
                })
                .collect();
            let texture = ui.ctx().load_texture(
                "liked-cover-gradient",
                egui::ColorImage::new([size, size], pixels),
                egui::TextureOptions::LINEAR,
            );
            ui.data_mut(|data| data.insert_temp(texture_id, texture.clone()));
            texture
        });
    egui::Image::new(&texture)
        .corner_radius(CornerRadius::same(radius.min(127.0) as u8))
        .paint_at(ui, rect);
    let size = rect.width() * 0.45;
    let icon_rect = Rect::from_center_size(rect.center(), Vec2::splat(size));
    Icon::HeartFilled
        .image(egui::Color32::WHITE, size)
        .paint_at(ui, icon_rect);
}

#[cfg(all(test, feature = "demo"))]
mod ordering_tests {
    use super::*;
    use crate::api::models::{Owner, Playlist};
    use crate::settings::Settings;

    fn app(name: &str) -> App {
        let root =
            std::env::temp_dir().join(format!("fastsonic-order-{name}-{}", std::process::id()));
        let mut app = App::new(
            &crate::backend::Waker::default(),
            crate::paths::AppDirs {
                config: root.join("config"),
                state: root.join("state"),
                cache: root.join("cache"),
            },
            Settings::default(),
            crate::app::AppOptions {
                media_controls: false,
                tray: false,
            },
        );
        crate::demo::populate(&mut app);
        app.library.playlists = Loadable::Loaded(
            [
                ("a", "Zebra", "Mona"),
                ("b", "Alpha", "Kai"),
                ("c", "alpha", "Mona"),
                ("d", "Beta", "Ada"),
            ]
            .into_iter()
            .map(|(id, name, owner)| Playlist {
                id: id.into(),
                name: name.into(),
                uri: uri(id),
                owner: Owner {
                    display_name: Some(owner.into()),
                    ..Default::default()
                },
                ..Default::default()
            })
            .collect(),
        );
        app.recent_contexts = vec![uri("d"), uri("a")];
        app
    }

    fn uri(id: &str) -> String {
        format!("sonic:playlist:{id}")
    }

    fn apply_actions(app: &mut App) {
        for action in std::mem::take(&mut app.actions) {
            app.apply(action, &egui::Context::default());
        }
    }

    fn rows(app: &App) -> Vec<Entry> {
        app.library
            .playlists
            .get()
            .unwrap()
            .iter()
            .enumerate()
            .map(|(index, playlist)| playlist_entry(playlist, index, ""))
            .collect()
    }

    fn ids(entries: &[Entry]) -> Vec<&str> {
        entries
            .iter()
            .map(|entry| entry.uri.rsplit(':').next().unwrap())
            .collect()
    }

    /// A grid card asks for the 300-pixel cover a page header uses rather
    /// than the 640-pixel one, and a list row keeps its 64-pixel thumbnail.
    #[test]
    fn grid_cards_use_the_page_header_art_size() {
        let playlist = Playlist {
            images: crate::api::subsonic::convert::art_images_for("pl-1"),
            ..Default::default()
        };
        let entry = playlist_entry(&playlist, 0, "");
        assert_eq!(entry.image.as_deref(), Some("sonic:art:64:pl-1"));
        assert_eq!(entry.grid_image.as_deref(), Some("sonic:art:300:pl-1"));
        assert_eq!(
            entry.grid_image.as_deref(),
            pick_image(&playlist.images, 300),
            "the header's cover"
        );
    }

    /// The sidebar's frame takes 20 points of its width; the grid gets the
    /// rest. Two columns at the narrowest, three from a middling width up
    /// to the widest, and the cards always fill the row.
    #[test]
    fn library_grid_adds_columns_as_the_sidebar_grows() {
        for (sidebar, columns) in [(210.0, 2), (230.0, 2), (380.0, 3), (440.0, 3)] {
            let width = sidebar - 20.0;
            let layout = grid_layout(width);
            assert_eq!(layout.columns, columns, "a {sidebar} point sidebar");
            let filled = layout.card_width * layout.columns as f32;
            assert!((filled - width).abs() < 0.01);
            assert!(layout.card_width >= GRID_MIN_CARD_WIDTH || columns == 2);
        }
        assert_eq!(grid_layout(600.0).columns, GRID_MAX_COLUMNS);
    }

    #[test]
    fn grid_pin_and_play_controls_keep_opposite_corners() {
        let narrowest = grid_layout(190.0).card_width - LIBRARY_ITEM_PADDING * 2.0;
        for side in [narrowest, GRID_MIN_CARD_WIDTH, 124.0] {
            let cover = Rect::from_min_size(pos2(10.0, 20.0), Vec2::splat(side));
            let pin = grid_pin_rect(cover);
            let play = grid_play_rect(cover);
            assert!(pin.center().x < cover.center().x && pin.center().y < cover.center().y);
            assert!(play.center().x > cover.center().x && play.center().y > cover.center().y);
            assert!(!pin.intersects(play), "a {side} point cover");
            assert!(cover.contains_rect(play));
        }
    }

    #[test]
    fn grid_drop_positions_follow_visual_row_order() {
        let layout = grid_layout(360.0);
        let origin = pos2(20.0, 40.0);
        assert_eq!(grid_reorder_slot(pos2(21.0, 41.0), origin, layout, 8), 0);
        assert_eq!(
            grid_reorder_slot(
                pos2(origin.x + layout.card_width, origin.y + 1.0),
                origin,
                layout,
                8,
            ),
            1
        );
        assert_eq!(
            grid_reorder_slot(
                pos2(origin.x + layout.card_width * 0.75, origin.y + 1.0),
                origin,
                layout,
                8,
            ),
            1,
            "past a card's middle is after it"
        );
        assert_eq!(
            grid_reorder_slot(
                pos2(origin.x + 1.0, origin.y + layout.row_height + 1.0),
                origin,
                layout,
                8,
            ),
            3
        );
        assert_eq!(
            grid_reorder_slot(pos2(origin.x + 1.0, 4000.0), origin, layout, 8),
            8,
            "below the last row is the end"
        );
    }

    /// A pin dropped on the lower edge of the pin block, between the last
    /// pin and the first unpinned row, stays pinned.
    #[test]
    fn a_pin_dropped_on_the_edge_of_the_pins_stays_pinned() {
        let mut app = app("pin-edge");
        app.settings.pinned_contexts = vec![uri("a"), uri("b")];
        let entries = ordered(&app);
        assert_eq!(entries[2].uri, uri("b"));
        drop_playlist_row(&mut app, &entries, 3, 3, &uri("a"));
        apply_actions(&mut app);
        assert_eq!(
            app.settings.library_pins(),
            [LIKED_SONGS_KEY.into(), uri("b"), uri("a")]
        );
        assert!(app.settings.sidebar_order.is_empty());
        // An unpinned row dropped there joins the rest instead.
        let entries = ordered(&app);
        drop_playlist_row(&mut app, &entries, 3, 3, &uri("d"));
        apply_actions(&mut app);
        assert_eq!(app.settings.library_pins().len(), 3);
        assert_eq!(app.settings.sidebar_order[0], uri("d"));
        app.backend.shutdown();
    }

    #[test]
    fn name_creator_and_recent_sorts_keep_pins_and_leave_the_library_alone() {
        let mut app = app("sorts");
        let mut entries = rows(&app);
        order_entries(&app, LibrarySort::RecentlyPlayed, &mut entries);
        assert_eq!(ids(&entries), ["d", "a", "b", "c"]);
        order_entries(&app, LibrarySort::Name, &mut entries);
        assert_eq!(ids(&entries), ["b", "c", "d", "a"]);
        order_entries(&app, LibrarySort::Creator, &mut entries);
        assert_eq!(ids(&entries), ["d", "b", "c", "a"], "by owner, then name");
        app.settings.pinned_contexts = vec![uri("a")];
        order_entries(&app, LibrarySort::Name, &mut entries);
        assert_eq!(ids(&entries), ["a", "b", "c", "d"]);
        assert_eq!(ids(&rows(&app)), ["a", "b", "c", "d"]);
        app.backend.shutdown();
    }

    #[test]
    fn switching_sort_and_a_filtered_drag_keep_the_whole_custom_order() {
        let mut app = app("saved");
        app.settings.sidebar_order = ["c", "a", "b", "d"].map(uri).to_vec();
        let saved = app.settings.sidebar_order.clone();
        assert_eq!(selected_sort(&app, Filter::Playlists), LibrarySort::Local);
        assert_eq!(full_playlist_order(&app), saved);
        app.settings
            .library_sort
            .insert(Filter::Playlists, LibrarySort::Name);
        assert_eq!(full_playlist_order(&app), ["b", "c", "d", "a"].map(uri));
        assert_eq!(app.settings.sidebar_order, saved, "another order keeps it");
        // A search shows only two rows; the drop still saves all four.
        let filtered: Vec<_> = rows(&app)
            .into_iter()
            .filter(|row| row.uri == uri("c") || row.uri == uri("a"))
            .rev()
            .collect();
        drop_playlist_row(&mut app, &filtered, 0, 0, &uri("a"));
        apply_actions(&mut app);
        assert_eq!(app.settings.sidebar_order, ["b", "a", "c", "d"].map(uri));
        assert_eq!(selected_sort(&app, Filter::Playlists), LibrarySort::Local);
        app.settings
            .library_sort
            .insert(Filter::Playlists, LibrarySort::Name);
        drop_playlist_row(&mut app, &filtered, 0, 0, &uri("a"));
        apply_actions(&mut app);
        assert_eq!(app.settings.sidebar_order, ["b", "a", "c", "d"].map(uri));
        assert_eq!(
            selected_sort(&app, Filter::Playlists),
            LibrarySort::Local,
            "recreating the saved arrangement still chooses it"
        );
        app.backend.shutdown();
    }

    #[test]
    fn a_pinned_row_dropped_below_the_pins_unpins_where_it_fell() {
        let mut app = app("unpin-drop");
        app.settings
            .library_sort
            .insert(Filter::Playlists, LibrarySort::Name);
        app.settings.pinned_contexts = vec![uri("a")];
        let mut entries = rows(&app);
        order_entries(&app, LibrarySort::Name, &mut entries);
        assert_eq!(ids(&entries), ["a", "b", "c", "d"]);
        drop_playlist_row(&mut app, &entries, 1, 2, &uri("a"));
        apply_actions(&mut app);
        assert_eq!(app.settings.library_pins(), [LIKED_SONGS_KEY]);
        assert_eq!(app.settings.sidebar_order, ["b", "a", "c", "d"].map(uri));
        assert_eq!(selected_sort(&app, Filter::Playlists), LibrarySort::Local);
        app.backend.shutdown();
    }

    fn ordered(app: &App) -> Vec<Entry> {
        let mut entries = rows(app);
        entries.push(liked_entry(app));
        order_entries(app, selected_sort(app, Filter::Playlists), &mut entries);
        entries
    }

    #[test]
    fn liked_songs_moves_among_pins_and_keeps_its_place_once_unpinned() {
        let mut app = app("liked-position");
        app.settings.pinned_contexts = [uri("d"), uri("a")].to_vec();
        let entries = ordered(&app);
        assert_eq!(entries[0].ordering_key(), LIKED_SONGS_KEY);
        drop_playlist_row(&mut app, &entries, 3, 2, LIKED_SONGS_KEY);
        assert_eq!(
            app.settings.library_pins()[0],
            LIKED_SONGS_KEY,
            "the view only asks; the change waits for the action"
        );
        apply_actions(&mut app);
        assert_eq!(
            app.settings.library_pins(),
            [uri("d"), LIKED_SONGS_KEY.into(), uri("a")]
        );
        assert!(app.settings.sidebar_order.is_empty());

        // Below the pins it unpins, and takes its place in the custom order.
        let entries = ordered(&app);
        drop_playlist_row(&mut app, &entries, 3, 4, LIKED_SONGS_KEY);
        apply_actions(&mut app);
        assert!(!app.settings.liked_songs_pinned);
        let saved = [uri("b"), LIKED_SONGS_KEY.into(), uri("c")];
        assert_eq!(full_playlist_order(&app), saved);
        app.settings
            .library_sort
            .insert(Filter::Playlists, LibrarySort::Name);
        assert_eq!(
            full_playlist_order(&app),
            [uri("b"), uri("c"), LIKED_SONGS_KEY.into()],
            "unpinned, it sorts by name like any playlist"
        );
        app.settings
            .library_sort
            .insert(Filter::Playlists, LibrarySort::Local);
        app.settings =
            serde_json::from_str(&serde_json::to_string(&app.settings).unwrap()).unwrap();
        assert_eq!(full_playlist_order(&app), saved, "a restart keeps it");

        // A search hides it; dragging what is shown keeps it where it was.
        let filtered: Vec<_> = ordered(&app)
            .into_iter()
            .filter(|entry| entry.uri == uri("c") || entry.uri == uri("b"))
            .collect();
        drop_playlist_row(&mut app, &filtered, 0, 0, &uri("c"));
        apply_actions(&mut app);
        assert_eq!(
            full_playlist_order(&app),
            [uri("c"), uri("b"), LIKED_SONGS_KEY.into()]
        );
        app.backend.shutdown();
    }

    #[test]
    fn unpinned_liked_songs_follows_recent_plays_and_the_drop_position() {
        let mut app = app("liked-sorts");
        app.settings.liked_songs_pinned = false;
        app.recent_contexts = vec![uri("a"), LIKED_SONGS_KEY.into(), uri("c")];
        app.settings
            .library_sort
            .insert(Filter::Playlists, LibrarySort::RecentlyPlayed);
        assert_eq!(
            full_playlist_order(&app),
            [
                uri("a"),
                LIKED_SONGS_KEY.into(),
                uri("c"),
                uri("b"),
                uri("d")
            ]
        );
        // By creator it has none, so it goes last.
        app.settings
            .library_sort
            .insert(Filter::Playlists, LibrarySort::Creator);
        assert_eq!(full_playlist_order(&app).last().unwrap(), LIKED_SONGS_KEY);
        let entries = ordered(&app);
        drop_playlist_row(&mut app, &entries, 0, 0, LIKED_SONGS_KEY);
        apply_actions(&mut app);
        assert_eq!(full_playlist_order(&app)[0], LIKED_SONGS_KEY);
        let entries = ordered(&app);
        let end = entries.len();
        drop_playlist_row(&mut app, &entries, 0, end, LIKED_SONGS_KEY);
        apply_actions(&mut app);
        assert_eq!(full_playlist_order(&app).last().unwrap(), LIKED_SONGS_KEY);
        assert!(!app.settings.liked_songs_pinned);
        app.backend.shutdown();
    }

    /// Only Liked Songs and playlists the Library lists can join the
    /// playlists' order; a stale drag of something else changes nothing.
    #[test]
    fn a_drop_of_an_unknown_row_leaves_the_arrangement_alone() {
        let mut app = app("unknown-drop");
        let entries = ordered(&app);
        app.actions.clear();
        drop_playlist_row(&mut app, &entries, 1, 2, "sonic:album:elsewhere");
        assert!(app.actions.is_empty());
        app.backend.shutdown();
    }

    #[test]
    fn recently_added_uses_real_instants_and_puts_missing_dates_last() {
        let mut app = app("dates");
        let mut entries = rows(&app);
        for (row, value) in entries.iter_mut().zip([
            Some("2026-09-09T09:00:00Z"),
            Some("2026-09-09T10:00:00+02:00"),
            None,
            Some("unknown"),
        ]) {
            row.added_at = saved_time(value);
        }
        entries.swap(0, 1);
        order_entries(&app, LibrarySort::RecentlyAdded, &mut entries);
        assert_eq!(ids(&entries), ["a", "b", "c", "d"]);
        assert!(LibrarySort::RecentlyAdded.supports(Filter::Albums));
        assert!(!LibrarySort::RecentlyAdded.supports(Filter::Playlists));
        assert!(!LibrarySort::RecentlyAdded.supports(Filter::Artists));
        assert!(!LibrarySort::Creator.supports(Filter::Artists));
        assert!(!LibrarySort::Local.supports(Filter::Albums));
        app.backend.shutdown();
    }

    #[test]
    fn older_settings_keep_their_order_and_new_playlists_precede_it() {
        let mut app = app("migration");
        app.settings = serde_json::from_str(
            r#"{"sidebar_order":["sonic:playlist:c","sonic:playlist:a","sonic:playlist:b"]}"#,
        )
        .unwrap();
        assert_eq!(selected_sort(&app, Filter::Playlists), LibrarySort::Local);
        assert_eq!(selected_sort(&app, Filter::Albums), LibrarySort::Library);
        assert_eq!(selected_sort(&app, Filter::Artists), LibrarySort::Library);
        assert_eq!(full_playlist_order(&app), ["d", "c", "a", "b"].map(uri));
        app.settings.sidebar_order.clear();
        assert_eq!(
            selected_sort(&app, Filter::Playlists),
            LibrarySort::RecentlyPlayed
        );
        // A sort saved for a section that cannot use it is not used.
        app.settings
            .library_sort
            .insert(Filter::Artists, LibrarySort::RecentlyAdded);
        assert_eq!(selected_sort(&app, Filter::Artists), LibrarySort::Library);
        app.backend.shutdown();
    }
}
