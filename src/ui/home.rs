//! The Home page.

use std::sync::Arc;

use egui::{CornerRadius, Rect, Sense, Vec2, pos2, vec2};

use crate::api::models::{Album, PlayableItem, pick_image};
use crate::app::App;
use crate::model::{Action, Loadable, Page, RowContext};
use crate::theme::{self, Icon};

use super::widgets::{self, TrackRow};

/// Home, as a self-hosted library rather than an editorial front page.
///
/// The shelves are in the order they are useful on a server you own. What
/// you just added comes first — importing a record and playing it is the
/// commonest thing anyone does here. Then what the server knows you play,
/// which is empty until this app has scrobbled something and empty again if
/// the native session behind it has lapsed (D11, D13) — so the two shelves
/// that always have something in them, recently added and the random one,
/// are the first and the last, and Home never reads as broken between them.
pub fn show(app: &mut App, ui: &mut egui::Ui) {
    let palette = app.palette;
    ui.add_space(6.0);
    theme::text(ui, crate::util::greeting(), theme::bold(30.0), palette.text);
    ui.add_space(12.0);
    quick_access(app, ui);
    ui.add_space(16.0);

    album_shelf(app, ui, "newest", "Recently added", Shelf::Newest);
    recently_played(app, ui);
    top_tracks(app, ui);
    album_shelf(app, ui, "frequent", "Most played", Shelf::Frequent);
    top_artists(app, ui);
    album_shelf(app, ui, "random", "Something at random", Shelf::Random);
}

/// Which of Home's album shelves is being drawn. Mirrors
/// `backend::AlbumShelf`, which is what asks for it.
#[derive(Clone, Copy)]
enum Shelf {
    Newest,
    Frequent,
    Random,
}

impl Shelf {
    fn albums(self, app: &App) -> Loadable<Vec<Album>> {
        match self {
            Self::Newest => app.home.newest_albums.clone(),
            Self::Frequent => app.home.frequent_albums.clone(),
            Self::Random => app.home.random_albums.clone(),
        }
    }
}

/// A row of records: recently added, most played, or a handful at random.
///
/// An empty shelf is drawn as nothing at all rather than as an empty box.
/// "Most played" has nothing in it until the server has counted some plays,
/// and a shelf that says so would be five words of apology on every Home.
fn album_shelf(app: &mut App, ui: &mut egui::Ui, id: &str, title: &str, shelf: Shelf) {
    let palette = app.palette;
    let albums = match shelf.albums(app) {
        Loadable::Loaded(albums) => albums,
        Loadable::Loading | Loadable::NotLoaded => {
            widgets::shelf(ui, &palette, id, title, |ui| {
                widgets::loading_row(ui, &palette)
            });
            return;
        }
        Loadable::Failed(message) => {
            widgets::shelf(ui, &palette, id, title, |ui| {
                widgets::error_row(ui, app, &message, Some(Page::Home));
            });
            return;
        }
    };
    if albums.is_empty() {
        return;
    }
    widgets::shelf(ui, &palette, id, title, |ui| {
        for album in &albums {
            let names = crate::api::models::join_names(
                album.artists.iter().map(|artist| artist.name.as_str()),
            );
            let subtitle = match (names.is_empty(), album.year()) {
                (false, Some(year)) => format!("{names} • {year}"),
                (false, None) => names,
                (true, Some(year)) => year.to_string(),
                (true, None) => String::new(),
            };
            let card = widgets::card(
                ui,
                app,
                pick_image(&album.images, 300),
                &album.name,
                &subtitle,
                false,
                true,
            );
            if card.play {
                app.actions.push(Action::PlayContext {
                    uri: album.uri.clone(),
                    offset_uri: None,
                    offset_index: None,
                });
            }
            if card.clicked {
                app.actions
                    .push(Action::Open(Page::Album(album.id.clone())));
            }
        }
    });
}

struct Tile {
    image: Option<String>,
    name: String,
    page: Page,
    uri: Option<String>,
    liked: bool,
}

fn quick_access(app: &mut App, ui: &mut egui::Ui) {
    let palette = app.palette;
    let mut tiles: Vec<Tile> = vec![Tile {
        image: None,
        name: "Liked Songs".to_string(),
        page: Page::LikedSongs,
        uri: Some(crate::api::subsonic::convert::COLLECTION_URI.to_string()),
        liked: true,
    }];
    if let Some(playlists) = app.library.playlists.get() {
        for playlist in playlists.iter().take(7) {
            tiles.push(Tile {
                image: pick_image(&playlist.images, 64).map(str::to_string),
                name: playlist.name.clone(),
                page: Page::Playlist(playlist.id.clone()),
                uri: Some(playlist.uri.clone()),
                liked: false,
            });
        }
    }
    let available = ui.available_width();
    let columns = ((available / 300.0).floor() as usize).clamp(2, 4);
    let gap = 10.0;
    let tile_width = (available - gap * (columns as f32 - 1.0)) / columns as f32;
    let rows = tiles.len().div_ceil(columns);
    for row in 0..rows {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = gap;
            for column in 0..columns {
                let Some(Tile {
                    image,
                    name,
                    page,
                    uri,
                    liked,
                }) = tiles.get(row * columns + column)
                else {
                    break;
                };
                let (rect, response) =
                    ui.allocate_exact_size(vec2(tile_width, 60.0), Sense::click());
                if ui.is_rect_visible(rect) {
                    let hovered = ui.rect_contains_pointer(rect);
                    let fill = if hovered {
                        palette.surface_hover
                    } else {
                        palette.surface
                    };
                    ui.painter().rect_filled(rect, CornerRadius::same(6), fill);
                    let cover = Rect::from_min_size(rect.min, Vec2::splat(60.0));
                    if *liked {
                        super::sidebar::liked_cover(ui, cover, 6.0);
                    } else {
                        widgets::paint_cover(
                            ui,
                            &palette,
                            image.as_deref(),
                            cover,
                            6.0,
                            Icon::Music,
                            Some(app.backend.art()),
                        );
                    }
                    let play_room = if hovered && uri.is_some() { 52.0 } else { 12.0 };
                    let text_rect = Rect::from_min_max(
                        pos2(cover.right() + 12.0, rect.top()),
                        pos2(rect.right() - play_room, rect.bottom()),
                    );
                    crate::bidi::paint_line(
                        &ui.painter().with_clip_rect(text_rect),
                        text_rect.left(),
                        text_rect.right(),
                        rect.center().y,
                        name,
                        theme::bold(14.5),
                        palette.text,
                    );
                    if hovered && let Some(uri) = uri {
                        let button = Rect::from_center_size(
                            pos2(rect.right() - 28.0, rect.center().y),
                            Vec2::splat(40.0),
                        );
                        let mut child =
                            ui.new_child(egui::UiBuilder::new().max_rect(button).layout(
                                egui::Layout::centered_and_justified(egui::Direction::LeftToRight),
                            ));
                        if theme::circle_button(
                            &mut child,
                            Icon::PlayFilled,
                            40.0,
                            palette.accent,
                            palette.accent_hover,
                            palette.on_accent,
                            "Play",
                        )
                        .clicked()
                        {
                            app.actions.push(Action::PlayContext {
                                uri: uri.clone(),
                                offset_uri: None,
                                offset_index: None,
                            });
                        }
                    }
                }
                if response
                    .on_hover_cursor(egui::CursorIcon::PointingHand)
                    .clicked()
                {
                    app.actions.push(Action::Open(page.clone()));
                }
            }
        });
    }
}

fn recently_played(app: &mut App, ui: &mut egui::Ui) {
    let palette = app.palette;
    let history = match app.home.recently_played.clone() {
        Loadable::Loaded(history) => history,
        Loadable::Loading | Loadable::NotLoaded => {
            widgets::shelf(ui, &palette, "recent", "Recently played", |ui| {
                widgets::loading_row(ui, &palette)
            });
            return;
        }
        Loadable::Failed(message) => {
            widgets::shelf(ui, &palette, "recent", "Recently played", |ui| {
                widgets::error_row(ui, app, &message, Some(Page::Home));
            });
            return;
        }
    };
    let mut seen = std::collections::HashSet::new();
    let tracks: Vec<_> = history
        .into_iter()
        .filter(|entry| {
            entry
                .track
                .id
                .as_ref()
                .is_some_and(|id| seen.insert(id.clone()))
        })
        .take(16)
        .collect();
    if tracks.is_empty() {
        return;
    }
    widgets::shelf(ui, &palette, "recent", "Recently played", |ui| {
        for entry in &tracks {
            let track = &entry.track;
            let card = widgets::card(
                ui,
                app,
                track.image(300),
                &track.name,
                &track.artist_names(),
                false,
                true,
            );
            if card.play {
                app.actions.push(Action::PlayUris {
                    uris: vec![track.uri.clone()],
                    index: 0,
                });
            }
            if card.clicked
                && let Some(album) = &track.album
                && !album.id.is_empty()
            {
                app.actions
                    .push(Action::Open(Page::Album(album.id.clone())));
            }
        }
    });
}

fn top_artists(app: &mut App, ui: &mut egui::Ui) {
    let palette = app.palette;
    let artists = match app.home.top_artists.clone() {
        Loadable::Loaded(artists) => artists,
        Loadable::Loading | Loadable::NotLoaded => {
            widgets::shelf(ui, &palette, "top-artists", "Your top artists", |ui| {
                widgets::loading_row(ui, &palette)
            });
            return;
        }
        Loadable::Failed(message) => {
            widgets::shelf(ui, &palette, "top-artists", "Your top artists", |ui| {
                widgets::error_row(ui, app, &message, Some(Page::Home));
            });
            return;
        }
    };
    if artists.is_empty() {
        return;
    }
    widgets::shelf(ui, &palette, "top-artists", "Your top artists", |ui| {
        for artist in &artists {
            let card = widgets::card(
                ui,
                app,
                pick_image(&artist.images, 300),
                &artist.name,
                "Artist",
                true,
                true,
            );
            if card.play {
                app.actions.push(Action::PlayContext {
                    uri: artist.uri.clone(),
                    offset_uri: None,
                    offset_index: None,
                });
            }
            if card.clicked {
                app.actions
                    .push(Action::Open(Page::Artist(artist.id.clone())));
            }
        }
    });
}

fn track_list(
    app: &mut App,
    ui: &mut egui::Ui,
    title: &str,
    tracks: Loadable<Vec<crate::api::models::Track>>,
    limit: usize,
    title_page: Option<Page>,
    more_label: Option<&str>,
) {
    let palette = app.palette;
    let tracks = match tracks {
        Loadable::Loaded(tracks) => tracks,
        Loadable::Loading | Loadable::NotLoaded => {
            if let Some(page) = title_page {
                if theme::link(ui, title, theme::bold(17.0), palette.text).clicked() {
                    app.actions.push(Action::Open(page));
                }
            } else {
                theme::section_title(ui, &palette, title);
            }
            widgets::loading_row(ui, &palette);
            ui.add_space(12.0);
            return;
        }
        Loadable::Failed(message) => {
            theme::section_title(ui, &palette, title);
            widgets::error_row(ui, app, &message, Some(title_page.unwrap_or(Page::Home)));
            ui.add_space(12.0);
            return;
        }
    };
    if tracks.is_empty() {
        return;
    }
    if let Some(page) = title_page {
        if theme::link(ui, title, theme::bold(17.0), palette.text).clicked() {
            app.actions.push(Action::Open(page));
        }
    } else {
        theme::section_title(ui, &palette, title);
    }
    ui.add_space(4.0);
    let uris: Arc<[String]> = tracks
        .iter()
        .map(|track| track.uri.clone())
        .collect::<Vec<_>>()
        .into();
    let context = RowContext::Uris(Arc::clone(&uris));
    for (index, track) in tracks.iter().take(limit).enumerate() {
        let item = PlayableItem::Track(track.clone());
        widgets::track_row(
            ui,
            app,
            TrackRow {
                index,
                number: None,
                item: &item,
                context: &context,
                show_cover: !app.settings.tracklist_compact,
                show_album: true,
                added_at: None,
                added_by: None,
                show_added_by: false,
                compact: false,
                thin: app.settings.tracklist_compact,
                shift: 0.0,
                picked: false,
                picked_songs: &[],
            },
        );
    }
    if let Some(label) = more_label
        && tracks.len() > limit
        && theme::link(ui, label, theme::semibold(14.0), palette.secondary).clicked()
    {
        app.actions.push(Action::Open(Page::TopSongs));
    }
    ui.add_space(16.0);
}

fn top_tracks(app: &mut App, ui: &mut egui::Ui) {
    let tracks = app.home.top_tracks.clone();
    track_list(
        app,
        ui,
        "Your top songs",
        tracks,
        10,
        Some(Page::TopSongs),
        Some("Show more top songs"),
    );
}
