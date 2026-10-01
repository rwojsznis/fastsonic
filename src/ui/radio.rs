//! A radio page: the songs the server picks to go with a song, playlist,
//! album or artist, laid out as a playlist, with the songs it shows being
//! the songs it plays.

use std::sync::Arc;

use crate::api::models::{PlayableItem, Track, pick_image};
use crate::api::subsonic::convert::{self, Kind};
use crate::app::App;
use crate::model::{Loadable, Page, RowContext};
use crate::theme::Icon;
use crate::util;

use super::collection::{
    Actions, Hero, Table, actions_row, hero, remember_table_items, table, table_items_hit,
};
use super::widgets;

pub fn radio(app: &mut App, ui: &mut egui::Ui, seed: &str) {
    let key = Page::Radio(seed.to_string());
    let Some(radio_uri) = convert::radio_uri(seed) else {
        return;
    };
    if !app.radio_pages.contains_key(seed) {
        app.ensure_loaded(key.clone());
    }
    let Some(page) = app.radio_pages.get(seed) else {
        return;
    };
    let name = app.radio_name(seed).unwrap_or_else(|| "Radio".into());
    let generation = page.generation;
    let refreshing = page.refreshing;
    let state = match &page.songs {
        Loadable::Loaded(songs) => Ok(Some((
            songs.len(),
            songs
                .iter()
                .map(|song| song.duration_ms as u64)
                .sum::<u64>(),
            featuring(songs),
        ))),
        Loadable::Failed(error) => Err(error.clone()),
        Loadable::Loading | Loadable::NotLoaded => Ok(None),
    };

    let mut byline = vec![based_on(seed)];
    let mut description = None;
    if let Ok(Some((count, duration, artists))) = &state
        && *count > 0
    {
        byline.push((
            format!("{count} songs, {}", util::format_total_ms(*duration)),
            None,
        ));
        description.clone_from(artists);
    }
    let images = app.radio_images(seed);
    hero(
        app,
        ui,
        Hero {
            image: pick_image(&images, 300),
            liked: false,
            kind: "Radio",
            title: &name,
            description,
            byline,
            round: false,
        },
    );

    let count = match state {
        Ok(Some((count, _, _))) => count,
        Ok(None) => {
            ui.add_enabled_ui(false, |ui| {
                actions_row(
                    app,
                    ui,
                    radio_actions(seed, &radio_uri, &name, None, true),
                    None,
                )
            });
            widgets::loading_row(ui, &app.palette);
            return;
        }
        Err(error) => {
            ui.add_enabled_ui(false, |ui| {
                actions_row(
                    app,
                    ui,
                    radio_actions(seed, &radio_uri, &name, None, false),
                    None,
                )
            });
            widgets::error_row(ui, app, &error, Some(key));
            return;
        }
    };
    let names = app.user_names_revision;
    let items = match table_items_hit(app, &key, generation, generation, names) {
        Some(items) => items,
        None => {
            let rows = app
                .radio_pages
                .get(seed)
                .and_then(|page| page.songs.get())
                .into_iter()
                .flatten()
                .map(|track| (PlayableItem::Track(track.clone()), None, None))
                .collect();
            remember_table_items(app, key.clone(), generation, generation, names, rows)
        }
    };
    let uris: Arc<[String]> = items
        .iter()
        .map(|(item, _, _)| item.uri().to_string())
        .collect::<Vec<_>>()
        .into();
    actions_row(
        app,
        ui,
        radio_actions(seed, &radio_uri, &name, Some(Arc::clone(&uris)), refreshing),
        None,
    );
    if count == 0 {
        // Not a failure: the server answered, and had nothing to say.
        widgets::empty_state(
            ui,
            &app.palette,
            Icon::Radio,
            "No similar songs",
            "Your server finds them through Last.fm or another agent, and has none without one.",
        );
        return;
    }
    table(
        app,
        ui,
        Table {
            items: &items,
            context: RowContext::View {
                uris,
                context_uri: radio_uri,
                editable_playlist: None,
            },
            show_album: true,
            show_cover: true,
            show_added: false,
            show_added_by: false,
            page: key,
            loading: false,
            error: None,
            can_load_more: false,
            filter: "",
            items_revision: generation,
        },
    );
}

fn radio_actions<'a>(
    seed: &str,
    radio_uri: &str,
    name: &'a str,
    view: Option<Arc<[String]>>,
    loading: bool,
) -> Actions<'a> {
    Actions {
        play_uri: Some(radio_uri.to_string()),
        view,
        saved: None,
        saved_icons: (Icon::CirclePlus, Icon::CircleCheck),
        saved_tooltips: ("", ""),
        owned_playlist: None,
        reload: Some((Page::Radio(seed.to_string()), loading)),
        name,
    }
}

/// What the radio is based on, linking to the seed's page where it has one.
fn based_on(seed: &str) -> (String, Option<Page>) {
    let (text, page) = match convert::parse_uri(seed) {
        Some((Kind::Playlist, id)) => ("Based on this playlist", Some(Page::Playlist(id.into()))),
        Some((Kind::Album, id)) => ("Based on this album", Some(Page::Album(id.into()))),
        Some((Kind::Artist, id)) => ("Based on this artist", Some(Page::Artist(id.into()))),
        _ => ("Based on this song", None),
    };
    (text.to_string(), page)
}

/// "With A, B and C": the first few artists the radio plays.
fn featuring(songs: &[Track]) -> Option<String> {
    let mut names: Vec<&str> = Vec::new();
    for artist in songs.iter().flat_map(|song| &song.artists) {
        if !artist.name.is_empty() && !names.contains(&artist.name.as_str()) {
            names.push(&artist.name);
        }
        if names.len() == 4 {
            break;
        }
    }
    match names.as_slice() {
        [] => None,
        [one] => Some(format!("With {one}")),
        [first, second] => Some(format!("With {first} and {second}")),
        [first, second, third] => Some(format!("With {first}, {second} and {third}")),
        [first, second, third, ..] => Some(format!("With {first}, {second}, {third} and more")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::models::ArtistRef;

    fn by(names: &[&str]) -> Track {
        Track {
            artists: names
                .iter()
                .map(|name| ArtistRef {
                    name: (*name).into(),
                    ..Default::default()
                })
                .collect(),
            ..Default::default()
        }
    }

    #[test]
    fn a_radio_names_its_first_artists() {
        assert_eq!(featuring(&[]), None);
        assert_eq!(featuring(&[by(&["Björk"])]).as_deref(), Some("With Björk"));
        assert_eq!(
            featuring(&[by(&["Björk", "Arca"]), by(&["Björk"])]).as_deref(),
            Some("With Björk and Arca")
        );
        assert_eq!(
            featuring(&[by(&["A"]), by(&["B"]), by(&["C"]), by(&["D"]), by(&["E"])]).as_deref(),
            Some("With A, B, C and more")
        );
    }

    #[test]
    fn a_radio_links_to_what_it_is_based_on() {
        assert_eq!(
            based_on("sonic:album:a:1"),
            (
                "Based on this album".into(),
                Some(Page::Album("a:1".into()))
            )
        );
        assert_eq!(
            based_on("sonic:track:s1"),
            ("Based on this song".into(), None)
        );
    }
}
