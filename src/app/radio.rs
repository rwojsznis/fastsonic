//! Radio pages: the songs the server picks to go with a song, playlist,
//! album or artist.
//!
//! The server picks afresh each time it is asked, so the page keeps the
//! songs it was given and its Play button plays exactly those, with the
//! radio as their context so the queue names it.

use super::*;

impl App {
    /// "`<Name>` Radio", once the seed's name is known.
    pub fn radio_name(&self, seed: &str) -> Option<String> {
        let name = self
            .radio_pages
            .get(seed)
            .and_then(|page| page.name.clone())
            .or_else(|| self.seed_name(seed))?;
        Some(format!("{name} Radio"))
    }

    /// The name of the song, playlist, album or artist, from whatever the
    /// app already holds.
    fn seed_name(&self, seed: &str) -> Option<String> {
        match convert::parse_uri(seed)? {
            (Kind::Track, id) => self.track_cache.get(id).map(|track| track.name.clone()),
            (Kind::Playlist, id) => self
                .library
                .playlists
                .get()
                .and_then(|list| list.iter().find(|playlist| playlist.id == id))
                .or_else(|| self.playlist_pages.get(id)?.playlist.get())
                .map(|playlist| playlist.name.clone()),
            (Kind::Album, id) => self
                .album_pages
                .get(id)
                .and_then(|page| page.album.get())
                .map(|album| album.name.clone()),
            (Kind::Artist, id) => self
                .artist_pages
                .get(id)
                .and_then(|page| page.artist.get())
                .map(|artist| artist.name.clone()),
            (Kind::Collection, _) => None,
        }
    }

    /// The seed's artwork, for the radio page's cover.
    pub fn radio_images(&self, seed: &str) -> Vec<Image> {
        if let Some(page) = self.radio_pages.get(seed)
            && !page.images.is_empty()
        {
            return page.images.clone();
        }
        let images = match convert::parse_uri(seed) {
            Some((Kind::Track, id)) => self
                .track_cache
                .get(id)
                .and_then(|track| track.album.as_ref())
                .map(|album| album.images.clone()),
            Some((Kind::Playlist, id)) => self
                .playlist_pages
                .get(id)
                .and_then(|page| page.playlist.get())
                .map(|playlist| playlist.images.clone()),
            Some((Kind::Album, id)) => self
                .album_pages
                .get(id)
                .and_then(|page| page.album.get())
                .map(|album| album.images.clone()),
            Some((Kind::Artist, id)) => self
                .artist_pages
                .get(id)
                .and_then(|page| page.artist.get())
                .map(|artist| artist.images.clone()),
            _ => None,
        };
        images.unwrap_or_default()
    }

    /// Asks for the seed's songs unless the page already has or awaits them.
    pub(super) fn load_radio(&mut self, seed: &str) {
        let page = self.radio_pages.entry(seed.to_string()).or_default();
        if !page.songs.needs_load() {
            return;
        }
        self.load_generation = self.load_generation.wrapping_add(1);
        page.generation = self.load_generation;
        page.songs = Loadable::Loading;
        self.backend.api(ApiRequest::Radio {
            seed: seed.to_string(),
            generation: page.generation,
        });
    }

    /// Asks for a new mix while the page keeps showing its songs. False
    /// when there are no songs to keep, so the page loads afresh.
    pub(super) fn refresh_radio(&mut self, seed: &str) -> bool {
        let Some(page) = self
            .radio_pages
            .get_mut(seed)
            .filter(|page| page.songs.get().is_some())
        else {
            return false;
        };
        if !page.refreshing {
            self.load_generation = self.load_generation.wrapping_add(1);
            page.generation = self.load_generation;
            page.refreshing = true;
            self.backend.api(ApiRequest::Radio {
                seed: seed.to_string(),
                generation: page.generation,
            });
        }
        true
    }

    pub(super) fn receive_radio(
        &mut self,
        seed: &str,
        generation: u64,
        result: crate::backend::ApiResult<crate::api::models::Radio>,
    ) {
        let Some(page) = self
            .radio_pages
            .get_mut(seed)
            .filter(|page| page.generation == generation)
        else {
            return;
        };
        let refreshing = std::mem::take(&mut page.refreshing);
        match result {
            Ok(radio) => {
                let flags = starred_flags(&radio.songs);
                if !radio.name.is_empty() {
                    page.name = Some(radio.name);
                }
                page.images = radio.images;
                // The table's cached rows follow the new songs.
                self.load_generation = self.load_generation.wrapping_add(1);
                page.generation = self.load_generation;
                let now = Instant::now();
                for track in &radio.songs {
                    if let Some(id) = &track.id {
                        self.track_cache
                            .entry(id.clone())
                            .or_insert_with(|| track.clone());
                        self.track_used.insert(id.clone(), now);
                    }
                }
                page.songs = Loadable::Loaded(radio.songs);
                self.note_saved(flags);
            }
            // A failed refresh keeps the mix on screen and says why.
            Err(error) if refreshing => {
                self.toast_error(format!("Couldn't refresh this radio: {error}"));
            }
            Err(error) => page.songs = Loadable::Failed(error.to_string()),
        }
    }

    /// Saves the radio's songs, as shown, to a new playlist named after it.
    pub(super) fn save_radio(&mut self, seed: &str) {
        let Some(songs) = self.radio_pages.get(seed).and_then(|page| page.songs.get()) else {
            return;
        };
        let add_uris: Vec<String> = songs.iter().map(|track| track.uri.clone()).collect();
        if add_uris.is_empty() {
            return;
        }
        let name = self.radio_name(seed).unwrap_or_else(|| "Radio".into());
        self.actions.push(Action::CreatePlaylist {
            name,
            public: false,
            add_uris,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::models::Radio;
    use egui::accesskit::{Action as AccessibleAction, ActionRequest, TreeId, TreeUpdate};

    fn headless_app() -> App {
        static NEXT: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
        let count = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let root =
            std::env::temp_dir().join(format!("fastsonic-radio-{}-{count}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let mut app = App::new(
            &Waker::default(),
            AppDirs {
                config: root.join("config"),
                state: root.join("state"),
                cache: root.join("cache"),
            },
            Settings::default(),
            AppOptions {
                media_controls: false,
                tray: false,
            },
        );
        app.local_ready = true;
        app.auth = AuthStatus::Connected {
            username: "test".into(),
        };
        app.backend.set_offline(true);
        app
    }

    fn song(id: &str, artist: &str) -> Track {
        Track {
            id: Some(id.into()),
            uri: convert::track_uri(id),
            name: format!("Song {id}"),
            duration_ms: 200_000,
            artists: vec![ArtistRef {
                name: artist.into(),
                ..Default::default()
            }],
            ..Default::default()
        }
    }

    fn mix(name: &str, ids: &[&str]) -> crate::backend::ApiResult<Radio> {
        Ok(Radio {
            name: name.into(),
            images: Vec::new(),
            songs: ids.iter().map(|id| song(id, "Kestrel")).collect(),
        })
    }

    fn unreachable() -> crate::backend::ApiResult<Radio> {
        Err(crate::api::subsonic::ApiError::Network(
            "unreachable".into(),
        ))
    }

    fn open(app: &mut App, ctx: &egui::Context, seed: &str) -> u64 {
        app.apply(Action::Open(Page::Radio(seed.into())), ctx);
        app.radio_pages[seed].generation
    }

    fn draw(
        ctx: &egui::Context,
        app: &mut App,
        seed: &str,
        events: Vec<egui::Event>,
    ) -> TreeUpdate {
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1240.0, 800.0),
                )),
                events,
                ..Default::default()
            },
            |ui| crate::ui::radio::radio(app, ui, seed),
        );
        output.textures_delta.clear();
        output.platform_output.accesskit_update.unwrap()
    }

    /// Draws the page and clicks the control with this label, leaving
    /// what it asked for in `app.actions`.
    fn click(ctx: &egui::Context, app: &mut App, seed: &str, label: &str) {
        let tree = draw(ctx, app, seed, Vec::new());
        let id = tree
            .nodes
            .iter()
            .find(|(_, node)| node.label() == Some(label))
            .unwrap_or_else(|| panic!("a {label} control"))
            .0;
        draw(
            ctx,
            app,
            seed,
            vec![egui::Event::AccessKitActionRequest(ActionRequest {
                target_tree: TreeId::ROOT,
                target_node: id,
                action: AccessibleAction::Click,
                data: None,
            })],
        );
    }

    fn shown(ctx: &egui::Context, app: &mut App, seed: &str) -> Vec<String> {
        draw(ctx, app, seed, Vec::new());
        app.table_rows[&Page::Radio(seed.into())]
            .items
            .iter()
            .map(|(item, _, _)| item.uri().to_string())
            .collect()
    }

    fn accessible_ctx() -> egui::Context {
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        crate::theme::install(&ctx);
        ctx
    }

    /// Go to song radio opens the radio's page and plays nothing until
    /// asked.
    #[test]
    fn a_radio_opens_its_page_without_playing() {
        let ctx = egui::Context::default();
        let mut app = headless_app();
        open(&mut app, &ctx, "sonic:track:xyz");
        assert_eq!(app.page(), &Page::Radio("sonic:track:xyz".into()));
        assert!(app.radio_pages["sonic:track:xyz"].songs.is_loading());
        assert!(app.queued_play.is_none());
        assert!(app.optimistic_playing.is_none());
        app.backend.shutdown();
    }

    /// The server picks afresh each time it is asked, so Play plays the
    /// songs on screen, shuffled or not, and the engine is handed the
    /// radio as their context so the queue names it.
    #[test]
    fn a_radio_plays_the_songs_it_shows_under_its_name() {
        let ctx = accessible_ctx();
        let seed = "sonic:playlist:pl9";
        let radio = "sonic:radio:playlist:pl9";
        for shuffle in [false, true] {
            let mut app = headless_app();
            app.shuffle_wanted = shuffle;
            let generation = open(&mut app, &ctx, seed);
            app.receive_radio(seed, generation, mix("Long Way Home", &["a", "b"]));
            click(&ctx, &mut app, seed, "Play");
            app.apply_actions(&ctx);
            let request = app.queued_play.clone().expect("a play request");
            assert_eq!(
                request.uris,
                vec!["sonic:track:a".to_string(), "sonic:track:b".into()],
                "shuffle {shuffle}: the shown songs play"
            );
            assert_eq!(request.context_uri.as_deref(), Some(radio));
            let load = local_load(&request, shuffle);
            assert_eq!(load.context_uri.as_deref(), Some(radio));
            assert_eq!(load.uris, request.uris);
            assert_eq!(
                app.playing_from(),
                Some(PlayingFrom {
                    name: "Long Way Home Radio".into(),
                    page: Page::Radio(seed.into()),
                })
            );
            // The radio is not the song, and not a playlist to remember
            // in the sidebar's order.
            assert_eq!(app.current_track_uri().as_deref(), Some("sonic:track:a"));
            assert!(!app.recent_contexts.iter().any(|uri| uri == radio));
            app.backend.shutdown();
        }
    }

    /// The engine's word on the context is enough: a paused radio is still
    /// named, after the song it is based on when that is all that is known.
    #[test]
    fn the_queue_names_a_radio_the_engine_reports() {
        let mut app = headless_app();
        app.track_cache.insert("xyz".into(), {
            let mut seed = song("xyz", "Pink Floyd");
            seed.name = "Wish You Were Here".into();
            seed
        });
        app.handle_queue(crate::engine::QueueSnapshot {
            current: Some(crate::engine::QueueRow {
                uri: "sonic:track:a".into(),
                track: None,
            }),
            context_uri: Some("sonic:radio:track:xyz".into()),
            ..Default::default()
        });
        assert_eq!(
            app.playing_from(),
            Some(PlayingFrom {
                name: "Wish You Were Here Radio".into(),
                page: Page::Radio("sonic:track:xyz".into()),
            })
        );
        app.backend.shutdown();
    }

    /// A radio's songs do not outlive the session, so the remembered song
    /// resumes on its own, still as the radio's.
    #[test]
    fn a_resumed_radio_song_plays_on_as_the_radios() {
        let ctx = egui::Context::default();
        let mut app = headless_app();
        app.resume_track = Some("sonic:track:a".into());
        app.resume_context = Some("sonic:radio:album:al1".into());
        app.apply(Action::TogglePlay, &ctx);
        let request = app.queued_play.clone().expect("a play request");
        assert_eq!(
            request.context_uri.as_deref(),
            Some("sonic:radio:album:al1")
        );
        assert_eq!(request.uris, vec!["sonic:track:a".to_string()]);
        app.backend.shutdown();
    }

    /// An answer to an earlier request does not replace the page, and a
    /// failure without songs to keep is the page's, with Retry.
    #[test]
    fn a_radio_takes_only_its_latest_answer() {
        let ctx = egui::Context::default();
        let mut app = headless_app();
        let seed = "sonic:album:alb1";
        let first = open(&mut app, &ctx, seed);
        app.receive_radio(seed, first, unreachable());
        assert!(matches!(app.radio_pages[seed].songs, Loadable::Failed(_)));
        app.apply(Action::Reload(Page::Radio(seed.into())), &ctx);
        let second = app.radio_pages[seed].generation;
        assert_ne!(first, second);
        app.receive_radio(seed, first, mix("Old", &["old"]));
        assert!(app.radio_pages[seed].songs.is_loading());
        app.receive_radio(seed, second, mix("Blue Harvest", &["new"]));
        let songs = app.radio_pages[seed].songs.get().expect("the latest mix");
        assert_eq!(songs[0].uri, "sonic:track:new");
        assert!(app.track_cache.contains_key("new"));
        assert_eq!(app.radio_name(seed).as_deref(), Some("Blue Harvest Radio"));
        app.backend.shutdown();
    }

    /// Refresh keeps the mix on screen until the new one arrives, shows the
    /// new songs once it does, and keeps the old ones if it fails.
    #[test]
    fn refreshing_a_radio_keeps_its_songs_until_the_new_mix_arrives() {
        let ctx = accessible_ctx();
        let mut app = headless_app();
        let seed = "sonic:artist:art1";
        let generation = open(&mut app, &ctx, seed);
        app.receive_radio(seed, generation, mix("Kestrel", &["old"]));
        assert_eq!(shown(&ctx, &mut app, seed), ["sonic:track:old"]);

        app.apply(Action::Reload(Page::Radio(seed.into())), &ctx);
        assert!(app.radio_pages[seed].refreshing);
        assert_eq!(
            shown(&ctx, &mut app, seed),
            ["sonic:track:old"],
            "the old mix stays"
        );
        let asked = app.radio_pages[seed].generation;
        // Asking again while one is on its way asks nothing more.
        app.apply(Action::Reload(Page::Radio(seed.into())), &ctx);
        assert_eq!(app.radio_pages[seed].generation, asked);
        app.receive_radio(seed, asked, mix("Kestrel", &["new"]));
        assert!(!app.radio_pages[seed].refreshing);
        assert_eq!(shown(&ctx, &mut app, seed), ["sonic:track:new"]);

        app.apply(Action::Reload(Page::Radio(seed.into())), &ctx);
        let asked = app.radio_pages[seed].generation;
        app.receive_radio(seed, asked, unreachable());
        assert_eq!(
            shown(&ctx, &mut app, seed),
            ["sonic:track:new"],
            "a failed refresh keeps the songs"
        );
        assert!(
            app.toasts
                .iter()
                .any(|toast| toast.message.starts_with("Couldn't refresh this radio")),
            "and says why"
        );
        app.backend.shutdown();
    }

    /// Save as playlist keeps the mix on screen, in order, as a private
    /// playlist under the radio's name.
    #[test]
    fn saving_a_radio_makes_a_private_playlist_of_its_songs() {
        let ctx = accessible_ctx();
        let mut app = headless_app();
        let seed = "sonic:track:xyz";
        let generation = open(&mut app, &ctx, seed);
        app.receive_radio(seed, generation, mix("Wish You Were Here", &["b", "a"]));
        click(&ctx, &mut app, seed, "Save as playlist");
        let saved = std::mem::take(&mut app.actions);
        assert!(matches!(saved.as_slice(), [Action::SaveRadio(uri)] if uri == seed));
        app.apply(Action::SaveRadio(seed.into()), &ctx);
        assert!(
            app.actions.iter().any(|action| matches!(
                action,
                Action::CreatePlaylist { name, public: false, add_uris }
                    if name == "Wish You Were Here Radio"
                        && add_uris == &["sonic:track:b".to_string(), "sonic:track:a".into()]
            )),
            "{:?}",
            app.actions
        );
        app.backend.shutdown();
    }
}
