//! Sample data for screenshots and headless rendering tests.
//!
//! Nothing here talks to a server: the backend is switched offline, the
//! session is marked signed in, and every page is filled with the shapes a
//! Navidrome library hands over — `sonic:` URIs, whole-second durations, a
//! starred flag carried on each object, and nothing in the fields Subsonic
//! has no answer for (no follower counts, no popularity, no copyright
//! line, no date or name against a playlist row).
//!
//! Three departures from what the client really receives, each deliberate:
//!
//! **Cover art is a placeholder URL.** Real models carry
//! `sonic:art:<size>:<id>` requests that `src/images.rs` turns into
//! `getCoverArt` calls with the current credential; demo mode has neither
//! credential nor server, so covers point at a public placeholder service.
//! That is also what keeps the artwork pipeline — download, disk cache,
//! accent colour — exercised, which is why it was a URL here before.
//!
//! **The ids are readable.** A Navidrome id is an opaque 22-character
//! string; `alb0`, `trk3` and `pl1` are what `--demo-page`, the screenshot
//! commands in `docs/` and the tests below address, so they stay as they
//! are. Nothing in the app reads meaning into an id.
//!
//! **Playback is published, not played.** There is no engine, so
//! [`populate`] hands the app one `LocalState` and one `QueueSnapshot`
//! through the same two doors `src/engine/` publishes them through. The
//! interface then draws the queue, the player bar, the context and the
//! hearts the way it draws the real thing — see `docs/_reference/queue.md`.

use std::time::Instant;

use jiff::{SignedDuration, Timestamp};

use crate::api::models::{
    Album, Artist, ArtistRef, Image, Owner, Page as ApiPage, PlayHistory, PlayableItem, Playlist,
    PlaylistItem, SavedAlbum, SavedTrack, SearchResults, Track, TrackCount, User,
};
use crate::api::subsonic::convert::{ART_SIZES, album_uri, artist_uri, playlist_uri, track_uri};
use crate::app::App;
use crate::backend::{AuthStatus, LocalPlayback};
use crate::engine::{LocalState, LocalTrack, Playback, QueueRow, QueueSnapshot, RepeatMode};
use crate::model::*;

/// The account signed in. A Subsonic user is a username and nothing else:
/// no display name of its own, no avatar, no plan.
const USER: &str = "demo";

/// Another account on the same server. Its playlist is public, so this one
/// can read it and cannot edit it.
const OTHER_USER: &str = "kasia";

/// The device id local playback reports (`backend::LOCAL_DEVICE_ID`). There
/// is one player and it is this process.
const DEVICE: &str = "local";

/// Cover art, in the three sizes `convert::art_images` offers. A real one
/// is a `sonic:art:` request; see the note at the top of the module.
fn cover(seed: u32) -> Vec<Image> {
    ART_SIZES
        .iter()
        .map(|size| Image {
            url: format!("https://picsum.photos/seed/fastsonic{seed}/{size}/{size}"),
            width: Some(*size),
            height: Some(*size),
        })
        .collect()
}

/// An artist's picture, in the three sizes a bare server offers: the
/// artist page gets them from `getArtistInfo2` as pre-signed share URLs
/// (`convert::info_images`), and Home's shelf from the artist id
/// (`convert::art_images_for`). Three either way.
fn artist_image(seed: u32) -> Vec<Image> {
    cover(seed)
}

/// The artists in the library. The index is the id: `art3` is `ARTISTS[3]`.
/// The last is the album artist a compilation is filed under.
const ARTISTS: &[&str] = &[
    "Bonobo",
    "Khruangbin",
    "Nils Frahm",
    "Little Simz",
    "Floating Points",
    "Jon Hopkins",
    "Sault",
    "Four Tet",
    "Various Artists",
];

/// One song: title, the disc it is on, its length in **whole seconds** —
/// which is all a Subsonic duration carries — the performer where it
/// differs from the album artist, and whether the tag says explicit.
type Song = (&'static str, u32, u32, Option<usize>, bool);

/// One album, and the songs on it. Ordinary tags, in other words: a
/// scanned library's albums own their songs and the numbers add up.
struct Record {
    title: &'static str,
    /// The album artist, as an index into [`ARTISTS`].
    artist: usize,
    year: u32,
    /// One genre, as `getAlbum` returns it from the tag.
    genre: &'static str,
    /// `recordLabels`, which most libraries have no tag for.
    label: Option<&'static str>,
    /// OpenSubsonic `releaseTypes`, which `convert::album_kind` lowercases
    /// and `Album::kind_label` draws. Untagged on an ordinary album.
    kind: Option<&'static str>,
    songs: &'static [Song],
}

/// The library, in the order a scan found it. The index is the id: `alb0`
/// is the first, and its songs are `trk0` onwards.
const RECORDS: &[Record] = &[
    Record {
        title: "Fragments",
        artist: 0,
        year: 2022,
        genre: "Electronic",
        label: Some("Ninja Tune"),
        kind: None,
        // Two songs called "Reprise", deliberately: a playlist holding
        // both is the fixture for removing a row by its index rather than
        // by its id (`migration/01-api-mapping.md`).
        songs: &[
            ("Rosewood", 1, 214, None, false),
            ("Otomo", 1, 249, None, false),
            ("Reprise", 1, 96, None, false),
            ("Tides", 1, 305, None, false),
            ("Elysian", 1, 271, None, false),
            ("Reprise", 1, 88, None, false),
            ("Sapien", 1, 232, None, false),
            ("Day by Day", 1, 258, None, false),
        ],
    },
    Record {
        title: "Mordechai",
        artist: 1,
        year: 2020,
        genre: "Psychedelic",
        label: Some("Dead Oceans"),
        kind: None,
        songs: &[
            ("First Class", 1, 262, None, false),
            ("Time Moves Slow", 1, 233, None, false),
            ("Pelota", 1, 198, None, false),
            ("So We Won't Forget", 1, 274, None, false),
            ("Dearest Alfredo", 1, 219, None, false),
            ("If There Is No Question", 1, 205, None, false),
            ("Shida", 1, 241, None, false),
        ],
    },
    Record {
        title: "All Melody",
        artist: 2,
        year: 2018,
        genre: "Ambient",
        label: Some("Erased Tapes"),
        kind: None,
        songs: &[
            (
                "The Whole Universe Wants to Be Touched",
                1,
                260,
                None,
                false,
            ),
            ("Sunson", 1, 543, None, false),
            ("A Place", 1, 289, None, false),
            ("My Friend the Forest", 1, 293, None, false),
            ("Human Range", 1, 372, None, false),
            ("Kaleidoscope", 1, 316, None, false),
        ],
    },
    Record {
        title: "Sometimes I Might Be Introvert",
        artist: 3,
        year: 2021,
        genre: "Hip-Hop",
        label: Some("Age 101"),
        kind: None,
        songs: &[
            ("Introvert", 1, 285, None, false),
            ("Woman", 1, 227, None, false),
            ("Two Worlds Apart", 1, 200, None, false),
            ("I Love You, I Hate You", 1, 246, None, true),
            ("Little Q", 1, 232, None, false),
            ("Speed", 1, 154, None, false),
            ("Point and Kill", 1, 214, None, false),
        ],
    },
    Record {
        title: "Ritual",
        artist: 4,
        year: 2024,
        genre: "Electronic",
        label: None,
        // A tagged release type, which is the only way the "Single" label
        // under a title gets there.
        kind: Some("single"),
        songs: &[
            ("Ritual", 1, 402, None, false),
            ("Ritual (Edit)", 1, 218, None, false),
        ],
    },
    Record {
        title: "Immunity",
        artist: 5,
        year: 2013,
        genre: "Electronic",
        label: Some("Domino"),
        kind: None,
        // Two discs, and the track numbers start again on the second.
        songs: &[
            ("We Disappear", 1, 249, None, false),
            ("Open Eye Signal", 1, 469, None, false),
            ("Breathe This Air", 1, 320, None, false),
            ("Collider", 1, 508, None, false),
            ("Abandon Window", 2, 254, None, false),
            ("Form by Firelight", 2, 292, None, false),
            ("Immunity", 2, 561, None, false),
        ],
    },
    Record {
        title: "Untitled (Black Is)",
        artist: 6,
        year: 2020,
        genre: "Soul",
        label: None,
        kind: None,
        songs: &[
            ("Out of the Lies", 1, 210, None, false),
            ("Hard Life", 1, 195, None, false),
            ("Bow", 1, 178, None, false),
            ("Wildfires", 1, 202, None, false),
            ("Black", 1, 231, None, false),
        ],
    },
    Record {
        title: "There Is Love in You",
        artist: 7,
        year: 2010,
        genre: "Electronic",
        label: Some("Domino"),
        kind: None,
        songs: &[
            ("Angel Echoes", 1, 199, None, false),
            ("Love Cry", 1, 542, None, false),
            ("Circling", 1, 269, None, false),
            ("Pablo's Heart", 1, 88, None, false),
            ("Sing", 1, 379, None, false),
            ("This Unfolds", 1, 429, None, false),
        ],
    },
    Record {
        title: "Late Night Tapes",
        artist: 8,
        year: 2024,
        genre: "Compilation",
        label: None,
        kind: Some("compilation"),
        // A compilation: every row's performer differs from the album's.
        songs: &[
            ("Counterpart", 1, 244, Some(7), false),
            ("From You", 1, 218, Some(1), false),
            ("Age of Phase", 1, 262, Some(5), false),
            ("Polyghost", 1, 233, Some(2), false),
            ("Encores", 1, 251, Some(0), false),
            ("So Rare", 1, 207, Some(6), false),
        ],
    },
];

/// The playlists the server lists: this account's own, and one public one
/// belonging to another. `pl1` is the one the docs screenshot and the
/// tests below drag rows around in, so it is owned and it is long.
const PLAYLISTS: &[(&str, &str, Option<&str>)] = &[
    (
        "Everything ambient",
        OTHER_USER,
        Some("Long records for long evenings. Shared with the house."),
    ),
    ("Late night focus", USER, None),
    ("Sunday morning", USER, None),
    ("Running 2026", USER, Some("Nothing under 120bpm.")),
    ("New this month", USER, None),
    ("Berlin nights", USER, None),
    ("Dinner party", USER, None),
    ("Deep work", USER, None),
    ("Road trip", USER, None),
    ("Kitchen jams", USER, None),
];

/// Which objects the server says are starred. Deterministic, and spread so
/// that every page has some hearts filled and some not.
fn starred_song(index: usize) -> bool {
    index.is_multiple_of(3)
}

fn starred_album(index: usize) -> bool {
    matches!(index, 0 | 3 | 7)
}

fn starred_artist(index: usize) -> bool {
    matches!(index, 0 | 2)
}

fn artist_ref(index: usize) -> ArtistRef {
    let id = format!("art{index}");
    ArtistRef {
        name: ARTISTS[index].to_string(),
        uri: Some(artist_uri(&id)),
        id: Some(id),
    }
}

fn artist(index: usize) -> Artist {
    let id = format!("art{index}");
    Artist {
        name: ARTISTS[index].to_string(),
        uri: artist_uri(&id),
        images: artist_image(100 + index as u32),
        // None of these exist in this protocol: an artist has a name, a
        // picture, and the albums filed under it.
        genres: Vec::new(),
        followers: None,
        popularity: None,
        external_urls: Default::default(),
        starred: Some(starred_artist(index)),
        id,
    }
}

fn album(index: usize) -> Album {
    let record = &RECORDS[index];
    let id = format!("alb{index}");
    Album {
        name: record.title.to_string(),
        uri: album_uri(&id),
        album_type: record.kind.map(str::to_string),
        album_group: None,
        total_tracks: Some(record.songs.len() as u32),
        images: cover(200 + index as u32),
        artists: vec![artist_ref(record.artist)],
        // A year is all the tag carries; there is no release day.
        release_date: Some(record.year.to_string()),
        label: record.label.map(str::to_string),
        genres: vec![record.genre.to_string()],
        // Neither a popularity score nor a copyright line is anything a
        // Subsonic server knows about.
        popularity: None,
        tracks: None,
        external_urls: Default::default(),
        copyrights: Vec::new(),
        starred: Some(starred_album(index)),
        id,
    }
}

/// The album a *song* carries: its id, name, artists, year and artwork,
/// which is as much as `convert::song_album` can build out of the song's
/// own tags. The album page loads the rest.
fn song_album(index: usize) -> Album {
    let record = &RECORDS[index];
    let id = format!("alb{index}");
    Album {
        name: record.title.to_string(),
        uri: album_uri(&id),
        images: cover(200 + index as u32),
        artists: vec![artist_ref(record.artist)],
        release_date: Some(record.year.to_string()),
        id,
        ..Album::default()
    }
}

/// Every song in the library, in album order: `songs()[3]` is `trk3`.
/// The second value is where each album's songs begin.
fn songs() -> (Vec<Track>, Vec<usize>) {
    let mut songs = Vec::new();
    let mut starts = Vec::new();
    for (album_index, record) in RECORDS.iter().enumerate() {
        starts.push(songs.len());
        for (position, song) in record.songs.iter().enumerate() {
            let (title, disc, seconds, performer, explicit) = *song;
            let index = songs.len();
            let id = format!("trk{index}");
            // Numbering restarts on each disc, as the tags do.
            let number = record.songs[..position]
                .iter()
                .filter(|other| other.1 == disc)
                .count() as u32
                + 1;
            songs.push(Track {
                name: title.to_string(),
                uri: track_uri(&id),
                // Whole seconds: `convert::track` multiplies the only
                // number the server sends, so every duration here is a
                // multiple of a thousand.
                duration_ms: seconds * 1000,
                explicit,
                artists: vec![artist_ref(performer.unwrap_or(record.artist))],
                album: Some(song_album(album_index)),
                track_number: Some(number),
                disc_number: Some(disc),
                is_local: false,
                is_playable: Some(true),
                popularity: None,
                external_urls: Default::default(),
                starred: Some(starred_song(index)),
                id: Some(id),
            });
        }
    }
    (songs, starts)
}

/// The songs on one album, out of the whole library.
fn album_songs(songs: &[Track], starts: &[usize], album: usize) -> Vec<Track> {
    let start = starts[album];
    songs[start..start + RECORDS[album].songs.len()].to_vec()
}

fn playlist(index: usize, songs: &[Track]) -> Playlist {
    let (name, owner, comment) = PLAYLISTS[index];
    let id = format!("pl{index}");
    Playlist {
        name: name.to_string(),
        uri: playlist_uri(&id),
        // A playlist's `comment`, which most have none of.
        description: comment.map(str::to_string),
        images: cover(300 + index as u32),
        // One string for both: a Subsonic playlist's owner is a username.
        owner: Owner {
            id: Some(owner.to_string()),
            display_name: Some(owner.to_string()),
            uri: None,
        },
        public: Some(owner != USER || index.is_multiple_of(2)),
        collaborative: false,
        // `changed`, which is the nearest thing to a snapshot id: it moves
        // whenever the contents do.
        snapshot_id: Some("2026-08-30T18:12:04Z".into()),
        tracks: Some(TrackCount {
            total: playlist_songs(index, songs).len() as u32,
        }),
        items_count: None,
        external_urls: Default::default(),
        id,
    }
}

/// What is on a playlist: songs from across the library, deterministic.
fn playlist_songs(index: usize, songs: &[Track]) -> Vec<Track> {
    match index {
        // The one the screenshots and the drag tests use. The first two
        // albums in order, so it carries both songs called "Reprise".
        1 => songs.iter().take(15).cloned().collect(),
        _ => songs
            .iter()
            .skip(index)
            .step_by(1 + index % 4)
            .take(20)
            .cloned()
            .collect(),
    }
}

/// A playlist's rows. Subsonic keeps no history against an entry, so
/// nothing here has a date or a name against it (`convert::playlist_items`).
fn playlist_rows(songs: Vec<Track>) -> Vec<PlaylistItem> {
    songs
        .into_iter()
        .map(|track| PlaylistItem {
            added_at: None,
            added_by: None,
            is_local: false,
            item: Some(PlayableItem::Track(track)),
            track: None,
        })
        .collect()
}

/// A page of a list the server sent whole, which is most of them: Subsonic
/// answers with everything and `convert` slices it.
fn page<T>(items: Vec<T>) -> ApiPage<T> {
    let total = items.len() as u32;
    ApiPage {
        items,
        total,
        limit: total,
        offset: 0,
        next: None,
    }
}

/// One song as the engine describes what it is playing.
fn local_track(track: &Track) -> LocalTrack {
    LocalTrack {
        uri: track.uri.clone(),
        title: track.name.clone(),
        artists: track
            .artists
            .iter()
            .map(|artist| artist.name.clone())
            .collect(),
        album: track
            .album
            .as_ref()
            .map(|album| album.name.clone())
            .unwrap_or_default(),
        art_url: track.image(640).map(str::to_string),
        art_small_url: track.image(64).map(str::to_string),
        duration_ms: track.duration_ms,
        starred: track.starred,
    }
}

fn queue_row(track: &Track) -> QueueRow {
    QueueRow {
        uri: track.uri.clone(),
        track: Some(local_track(track)),
    }
}

/// How long ago the *n*th row of Recents was played. The first rows cover
/// each relative-date unit; the rest are days.
fn played_ago(index: usize) -> SignedDuration {
    match index {
        0 => SignedDuration::from_secs(30),
        1 => SignedDuration::from_mins(5),
        2 => SignedDuration::from_hours(3),
        3 => SignedDuration::from_hours(2 * 24),
        4 => SignedDuration::from_hours(2 * 7 * 24),
        _ => SignedDuration::from_hours((35 + index as i64) * 24),
    }
}

/// The play history the server reports: songs, in the order they were
/// last played, and **no times at all** — the native API sends none the
/// interface can use, so the order is the only fact (`backend::
/// history_page`). The times in Recents come from this app's own plays.
fn history(songs: &[Track]) -> Vec<PlayHistory> {
    songs
        .iter()
        .map(|track| PlayHistory {
            track: track.clone(),
            played_at: None,
            context: None,
        })
        .collect()
}

pub fn populate(app: &mut App) {
    app.backend.set_offline(true);
    app.offline = true;
    app.auth = AuthStatus::Connected {
        username: USER.into(),
    };
    app.local_device_id = Some(DEVICE.into());
    app.local_ready = true;
    app.local_playback = LocalPlayback::Ready {
        device_id: DEVICE.into(),
    };
    app.user = Some(User {
        id: USER.into(),
        display_name: Some(USER.into()),
        // No avatar, no plan, no country: `getUser` carries none of them,
        // and every account on a server you own can stream.
        images: Vec::new(),
        product: None,
        country: None,
        uri: None,
    });

    let artists: Vec<Artist> = (0..ARTISTS.len()).map(artist).collect();
    let albums: Vec<Album> = (0..RECORDS.len()).map(album).collect();
    let (songs, starts) = songs();
    let playlists: Vec<Playlist> = (0..PLAYLISTS.len())
        .map(|index| playlist(index, &songs))
        .collect();

    // What a loaded page already knows: every object arrives with the
    // server's starred flag on it, and `App::note_saved` copies it here as
    // the page lands — no page asks (P4.2).
    for track in &songs {
        app.saved
            .insert(track.uri.clone(), track.starred.unwrap_or_default());
    }
    for album in &albums {
        app.saved
            .insert(album.uri.clone(), album.starred.unwrap_or_default());
    }
    for artist in &artists {
        app.saved
            .insert(artist.uri.clone(), artist.starred.unwrap_or_default());
    }
    // A playlist has nothing to star: it is yours or it is public. Every
    // one the server listed is on the sidebar.
    for playlist in &playlists {
        app.saved.insert(playlist.uri.clone(), true);
    }
    app.library.playlists = Loadable::Loaded(playlists.clone());

    // Playlist pages: the long owned one the screenshots use, and the
    // public one belonging to somebody else, which offers no editing.
    for index in [1, 0] {
        let mut playlist_page = PlaylistPage {
            playlist: Loadable::Loaded(playlists[index].clone()),
            ..PlaylistPage::default()
        };
        playlist_page
            .items
            .absorb(0, page(playlist_rows(playlist_songs(index, &songs))));
        app.playlist_pages
            .insert(format!("pl{index}"), playlist_page);
    }

    // Album pages: the first album, and the two-disc one.
    for index in [0, 5] {
        let mut album_page = AlbumPage {
            album: Loadable::Loaded(albums[index].clone()),
            ..AlbumPage::default()
        };
        album_page
            .tracks
            .absorb(0, page(album_songs(&songs, &starts, index)));
        app.album_pages.insert(format!("alb{index}"), album_page);
    }

    // Artist pages. Both halves of D11: `art0` is a bare self-hosted
    // server, where Popular and Fans also like are empty because both are
    // Last.fm-backed and there is no key — verified against the
    // development server, `getTopSongs` answers `{}` — and `art2` is a
    // server with a key, where they have something on them.
    for index in [0, 2] {
        let mine: Vec<usize> = (0..RECORDS.len())
            .filter(|album| RECORDS[*album].artist == index)
            .collect();
        let top: Vec<Track> = mine
            .iter()
            .flat_map(|album| album_songs(&songs, &starts, *album))
            .take(10)
            .collect();
        let lastfm = index == 2;
        let mut artist_page = ArtistPage {
            artist: Loadable::Loaded(artists[index].clone()),
            top_tracks: Loadable::Loaded(if lastfm { top } else { Vec::new() }),
            related: Loadable::Loaded(if lastfm {
                artists.iter().skip(3).take(5).cloned().collect()
            } else {
                Vec::new()
            }),
            ..ArtistPage::default()
        };
        let mut discography = PagedList::default();
        discography.absorb(
            0,
            page(mine.iter().map(|album| albums[*album].clone()).collect()),
        );
        artist_page
            .albums
            .insert(DiscographyFilter::All.groups().to_string(), discography);
        app.artist_pages.insert(format!("art{index}"), artist_page);
    }

    // Library. Starred songs, albums and artists, with no date against
    // them: `getStarred2` says what is starred, not when it was.
    app.library.liked.absorb(
        0,
        page(
            songs
                .iter()
                .filter(|track| track.starred.unwrap_or_default())
                .map(|track| SavedTrack {
                    added_at: None,
                    track: track.clone(),
                })
                .collect(),
        ),
    );
    app.library.albums.absorb(
        0,
        page(
            albums
                .iter()
                .filter(|album| album.starred.unwrap_or_default())
                .map(|album| SavedAlbum {
                    added_at: None,
                    album: album.clone(),
                })
                .collect(),
        ),
    );
    app.library.artists.items = artists
        .iter()
        .filter(|artist| artist.starred.unwrap_or_default())
        .cloned()
        .collect();
    app.library.artists.loaded_once = true;
    app.library.artists.complete = true;

    // Home. Recently added and something at random come off any library;
    // the four in between are the ones the native API answers once
    // something has been played (D11, D13).
    app.home.requested = true;
    app.home.loaded_at = Some(Instant::now());
    app.home.newest_albums = Loadable::Loaded(albums.iter().rev().cloned().collect());
    app.home.frequent_albums = Loadable::Loaded(albums.iter().skip(2).cloned().collect());
    app.home.random_albums = Loadable::Loaded(albums.iter().rev().skip(3).cloned().collect());
    // One song from each album, so the shelf is a shelf rather than one
    // cover eight times.
    app.home.recently_played = Loadable::Loaded(history(
        &(0..RECORDS.len())
            .filter_map(|album| album_songs(&songs, &starts, album).into_iter().nth(1))
            .collect::<Vec<_>>(),
    ));
    app.home.top_artists = Loadable::Loaded(artists.iter().take(8).cloned().collect());
    app.home.top_tracks = Loadable::Loaded(songs.iter().skip(10).take(20).cloned().collect());
    app.home.top_songs = Loadable::Loaded(songs.iter().skip(10).cloned().collect());
    app.home.top_songs_complete = true;

    // Recents. Two halves, as the tab really has them: the server's rows,
    // which carry no time, and this app's own plays, which do — and which
    // sort above them (`history::merged`).
    app.plays.clear();
    let now = Timestamp::now();
    for (index, track) in songs.iter().skip(2).take(6).enumerate().rev() {
        app.plays.record(track.clone(), now - played_ago(index));
    }
    app.recents.items = history(&songs.iter().skip(8).take(18).cloned().collect::<Vec<_>>());
    app.recents.loaded_once = true;
    app.recents.loading = false;
    app.recents.error = None;
    // There is more to load: the cursor is an offset, being the number of
    // rows read so far.
    app.recents.after = Some(app.recents.items.len().to_string());
    app.recents.complete = false;
    app.rebuild_recents();

    // Search. `search3` has three buckets and no more: no playlists, no
    // podcasts (`convert::search_results`).
    app.search.query = "Bonobo".into();
    app.search.committed = "Bonobo".into();
    app.search.results = Loadable::Loaded(SearchResults {
        tracks: Some(page(songs.iter().take(10).cloned().collect())),
        artists: Some(page(artists.iter().take(6).cloned().collect())),
        albums: Some(page(albums.iter().take(6).cloned().collect())),
        playlists: None,
        shows: None,
        episodes: None,
    });
    app.settings.search_history = vec!["Khruangbin".into(), "ambient".into(), "immunity".into()];

    for track in &songs {
        if let Some(id) = &track.id {
            app.track_cache.insert(id.clone(), track.clone());
        }
    }

    // Playback: this computer is playing the first album, with one song
    // queued by hand over the top of it. Both of these go in the way the
    // engine sends them, so nothing about them is drawn specially.
    let playing = &songs[0];
    let queued = &songs[starts[2] + 1];
    app.handle_queue(QueueSnapshot {
        current: Some(queue_row(playing)),
        queued: vec![queue_row(queued)],
        upcoming: album_songs(&songs, &starts, 0)
            .iter()
            .skip(1)
            .map(queue_row)
            .collect(),
        context_uri: Some(albums[0].uri.clone()),
        context_at: Some(playing.uri.clone()),
    });
    app.handle_local(LocalState {
        playback: Playback::Playing,
        track: Some(local_track(playing)),
        position_ms: 83_000,
        position_at: Some(Instant::now()),
        volume: crate::app::percent_to_volume(70),
        shuffle: false,
        repeat: RepeatMode::Off,
        connected: true,
        username: USER.into(),
        error: None,
        seek_sequence: 0,
    });
}

/// Words to go with the sample track, timed so that the one being sung
/// sits mid-panel at the demo's playback position.
#[cfg(feature = "demo")]
fn sample_lyrics() -> crate::lyrics::Lyrics {
    let lines = [
        (40_000, "Streetlights blinking down the river road"),
        (46_500, "Every window holding someone's evening"),
        (53_000, "I keep the radio low so you can sleep"),
        (59_500, "Counting mile markers like a rosary"),
        (66_000, "We left the city with the tank half full"),
        (72_500, "And a map that only shows the way back"),
        (79_000, "But the night is wide and the road is long"),
        (85_500, "And there's nowhere I would rather be"),
        (92_000, "Coffee going cold in the cup holder"),
        (98_500, "Your hand asleep on the gear stick"),
        (105_000, "Somewhere past the county line"),
        (111_500, "The stars come out to see us through"),
        (118_000, "Still the night is wide and the road is long"),
        (124_500, "And there's nowhere I would rather be"),
    ];
    crate::lyrics::Lyrics {
        lines: lines
            .iter()
            .map(|(at_ms, text)| crate::lyrics::Line {
                at_ms: Some(*at_ms),
                text: (*text).to_string(),
            })
            .collect(),
        synced: true,
        instrumental: false,
    }
}

/// Applies `--demo-page` and `--demo-show`.
#[cfg(feature = "demo")]
pub fn apply_flags(app: &mut App, page: Option<&str>, show: Option<&str>) {
    // Default screenshots to the main window regardless of saved settings.
    app.settings.winamp_window = false;
    if let Some(page) = page.and_then(Page::decode) {
        app.open(page);
    }
    for surface in show.unwrap_or("").split(',').map(str::trim) {
        match surface {
            "queue" => app.show_queue_panel = true,
            "recents" => {
                app.show_queue_panel = true;
                app.queue_tab = QueueTab::Recents;
            }
            "devices" => app.show_devices = true,
            "shortcuts" => app.dialog = Some(Dialog::Shortcuts),
            "create" => {
                app.dialog = Some(Dialog::CreatePlaylist {
                    name: "Autumn drives".into(),
                    public: false,
                    add_uris: vec![track_uri("trk1")],
                })
            }
            "light" => {
                app.settings.theme = crate::settings::ThemeChoice::Light;
                app.actions.push(Action::SettingsChanged);
            }
            "focus" => app.settings.sidebar_visible = false,
            // A cold start: nothing is playing, and all the app has is
            // what the last session left it — the song, the place the
            // playlist had got to, and the rows that were queued behind
            // it (rule 9).
            "resume" => resume(app),
            // The same cold start, one press of Next in: the song moved on
            // and nothing started playing.
            "resume-next" => {
                resume(app);
                app.actions.push(Action::Next);
            }
            // Use the built-in skin for deterministic screenshots.
            "winamp" => {
                app.settings.winamp_window = true;
                app.settings.skin = None;
            }
            "playlist" => app.settings.playlist_open = true,
            "shade" => app.settings.winamp_shaded = true,
            "playlist-shade" => app.settings.playlist_shaded = true,
            "eq" => {
                app.settings.eq_open = true;
                app.settings.eq_on = true;
                app.settings.eq_bands_db = crate::eq::PRESETS[13].bands_db;
            }
            "presets" => app.winamp.open_presets = true,
            "art" => app.settings.art_expanded = true,
            "small" => app.settings.skin_scale = Some(1),
            "compact" => {
                app.settings.sidebar_compact = true;
                app.settings.tracklist_compact = true;
            }
            "eq-shade" => {
                app.settings.eq_open = true;
                app.settings.eq_shaded = true;
            }
            "milkdrop" => app.settings.milkdrop_open = true,
            "pins" => {
                app.settings.pinned_contexts = vec![playlist_uri("pl2"), playlist_uri("pl4")];
            }
            // By title: a playlist row carries no date to sort by, so the
            // Added column has nothing in it to order.
            "sorted" => {
                app.table_sorts.insert(
                    Page::Playlist("pl1".into()),
                    crate::model::TableSort {
                        column: crate::model::SortColumn::Title,
                        ascending: false,
                    },
                );
            }
            "lyrics" => {
                app.lyrics_uri = app.now_playing().map(|now| now.uri);
                app.lyrics = Loadable::Loaded(Some(sample_lyrics()));
                app.lyrics_following = true;
                app.show_lyrics_panel = true;
            }
            // Titles in scripts the interface font does not cover.
            "scripts" => {
                let titles = [
                    ("\u{591c}\u{306b}\u{99c6}\u{3051}\u{308b}", "YOASOBI"),
                    (
                        "\u{8d77}\u{98ce}\u{4e86}",
                        "\u{4e70}\u{8fa3}\u{6912}\u{4e5f}\u{7528}\u{5238}",
                    ),
                    (
                        "\u{bd04}\u{c5ec}\u{b984}\u{ac00}\u{c744}\u{aca8}\u{c6b8} (Still Life)",
                        "BIGBANG",
                    ),
                    (
                        "\u{6253}\u{4e0a}\u{82b1}\u{706b}",
                        "DAOKO, \u{7c73}\u{6d25}\u{7384}\u{5e2b}",
                    ),
                    (
                        "\u{5149}\u{5e74}\u{4e4b}\u{5916}",
                        "G.E.M. \u{9093}\u{7d2b}\u{68cb}",
                    ),
                    ("\u{bc24}\u{d3b8}\u{c9c0}", "IU"),
                    ("Lemon", "\u{7c73}\u{6d25}\u{7384}\u{5e2b}"),
                    (
                        "\u{7ea2}\u{8272}\u{9ad8}\u{8ddf}\u{978b}",
                        "\u{8521}\u{5065}\u{96c5}",
                    ),
                ];
                let rename = |track: &mut Track, (title, artist): (&str, &str)| {
                    track.name = title.to_string();
                    track.artists = vec![ArtistRef {
                        id: None,
                        name: artist.to_string(),
                        uri: None,
                    }];
                };
                if let Some(page) = app.playlist_pages.get_mut("pl1") {
                    for (entry, names) in page.items.items.iter_mut().zip(titles) {
                        if let Some(PlayableItem::Track(track)) = &mut entry.item {
                            rename(track, names);
                        }
                    }
                }
                for (item, names) in app.queue.queue.iter_mut().zip(titles) {
                    if let PlayableItem::Track(track) = item {
                        rename(track, names);
                    }
                }
                if let Some(item) = &mut app.queue.currently_playing
                    && let PlayableItem::Track(track) = item
                {
                    rename(track, titles[0]);
                }
                if let Some(track) = &mut app.local.track {
                    let (title, artist) = titles[0];
                    track.title = title.to_string();
                    track.artists = vec![artist.to_string()];
                }
                if let Some(track) = app.track_cache.get_mut("trk0") {
                    rename(track, titles[0]);
                }
                if let Loadable::Loaded(playlists) = &mut app.library.playlists {
                    let names = [
                        "\u{901a}\u{52e4}\u{306e}BGM",
                        "\u{7761}\u{524d}\u{6b4c}\u{5355}",
                        "\u{cd9c}\u{adfc}\u{ae38} \u{d50c}\u{b808}\u{c774}\u{b9ac}\u{c2a4}\u{d2b8}",
                    ];
                    for (playlist, name) in playlists.iter_mut().skip(3).zip(names) {
                        playlist.name = name.to_string();
                    }
                }
            }
            _ => {}
        }
    }
}

/// A session picked up where the last one left off, before a first press:
/// nothing is playing, and what the app has is the remembered song, the
/// place the playlist had reached, and the rows queued behind it. This is
/// rule 9 as the interface sees it — `App::new` builds the same thing out
/// of the saved session, and demo mode has no session file to read.
#[cfg(feature = "demo")]
fn resume(app: &mut App) {
    // The engine has nothing on. The volume is a setting rather than
    // something the engine remembers, so it survives (`App::new`).
    app.local = LocalState {
        volume: app.local.volume,
        ..LocalState::default()
    };
    app.resume_context = Some(playlist_uri("pl1"));
    app.resume_track = Some(track_uri("trk0"));
    app.resume_position_ms = 19_566;
    let remembered: Vec<PlayableItem> = ["trk18", "trk1", "trk2", "trk3"]
        .iter()
        .filter_map(|id| app.track_cache.get(*id).cloned())
        .map(PlayableItem::Track)
        .collect();
    // The first row was queued by hand; the rest is where the playlist
    // had got to.
    app.set_remembered_queue(remembered, 1);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::AppOptions;
    use crate::paths::AppDirs;
    use crate::settings::Settings;

    fn frame(ctx: &egui::Context, app: &mut App) {
        frame_events(ctx, app, Vec::new());
    }

    fn frame_events(ctx: &egui::Context, app: &mut App, events: Vec<egui::Event>) {
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1280.0, 800.0),
            )),
            events,
            ..Default::default()
        };
        let mut output = ctx.run_ui(input, |ui| {
            app.frame_ui(ui);
        });
        output.textures_delta.clear();
    }

    /// A toast is wide enough to avoid wrapping every word.
    #[test]
    fn a_toast_is_wide_enough_to_read() {
        let root =
            std::env::temp_dir().join(format!("fastsonic-toast-test-{}", std::process::id()));
        let dirs = AppDirs {
            config: root.join("config"),
            state: root.join("state"),
            cache: root.join("cache"),
        };
        let ctx = egui::Context::default();
        let waker = crate::backend::Waker::default();
        waker.attach(&ctx);
        let mut app = App::new(
            &waker,
            dirs,
            Settings::default(),
            AppOptions {
                media_controls: false,
                tray: false,
            },
        );
        app.attach(&ctx);
        populate(&mut app);
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1280.0, 800.0),
            )),
            ..Default::default()
        };
        // A short toast first: the toasts area remembers its size, and a
        // long toast used to inherit the narrow width and wrap inside it.
        app.toast("Saved");
        for _ in 0..2 {
            frame(&ctx, &mut app);
        }
        app.toasts.clear();
        app.toast("Wish You Were Here will play next");
        // Two frames: an area sizes itself on its first one.
        let mut first = ctx.run_ui(input.clone(), |ui| app.frame_ui(ui));
        first.textures_delta.clear();
        let mut output = ctx.run_ui(input, |ui| app.frame_ui(ui));
        output.textures_delta.clear();

        fn widest_toast_text(shape: &egui::epaint::Shape) -> Option<f32> {
            match shape {
                egui::epaint::Shape::Text(text)
                    if text.galley.job.text.contains("Wish You Were Here") =>
                {
                    Some(text.galley.rect.width())
                }
                egui::epaint::Shape::Vec(shapes) => {
                    shapes.iter().filter_map(widest_toast_text).next()
                }
                _ => None,
            }
        }
        let width = output
            .shapes
            .iter()
            .filter_map(|clipped| widest_toast_text(&clipped.shape))
            .next()
            .expect("the toast's text is painted");
        assert!(
            width > 150.0,
            "one word per line again: the toast text is only {width}px wide"
        );
        app.backend.shutdown();
    }

    /// The shortcuts are longer than a small window is tall, so the
    /// dialog scrolls them rather than running off the bottom with the
    /// Done button somewhere past the edge of the screen.
    #[test]
    fn the_shortcuts_dialog_fits_a_small_window() {
        let root =
            std::env::temp_dir().join(format!("fastsonic-shortcuts-test-{}", std::process::id()));
        let dirs = AppDirs {
            config: root.join("config"),
            state: root.join("state"),
            cache: root.join("cache"),
        };
        let ctx = egui::Context::default();
        let waker = crate::backend::Waker::default();
        waker.attach(&ctx);
        let mut app = App::new(
            &waker,
            dirs,
            Settings::default(),
            AppOptions {
                media_controls: false,
                tray: false,
            },
        );
        app.attach(&ctx);
        populate(&mut app);
        app.dialog = Some(Dialog::Shortcuts);

        let height = 420.0;
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(900.0, height),
            )),
            ..Default::default()
        };
        // Two frames: the dialog sizes itself on the first.
        let mut first = ctx.run_ui(input.clone(), |ui| app.frame_ui(ui));
        first.textures_delta.clear();
        let mut output = ctx.run_ui(input, |ui| app.frame_ui(ui));
        output.textures_delta.clear();

        let dialog = app.dialog_rect.expect("the dialog drew itself");
        let bottom = dialog.max.y;
        assert!(
            bottom <= height + 1.0,
            "the dialog runs {} pixels past the bottom of a {height}-tall window",
            bottom - height
        );
        app.backend.shutdown();
    }

    /// The frame rate is a dial with detents: it stops at the rates
    /// worth having, names the one it is on, and moving it one notch
    /// lands on the next of them rather than somewhere in between.
    #[cfg(feature = "milkdrop")]
    #[test]
    fn the_frame_rate_dial_steps_between_its_stops() {
        let root =
            std::env::temp_dir().join(format!("fastsonic-fps-dial-test-{}", std::process::id()));
        let dirs = AppDirs {
            config: root.join("config"),
            state: root.join("state"),
            cache: root.join("cache"),
        };
        let ctx = egui::Context::default();
        let waker = crate::backend::Waker::default();
        waker.attach(&ctx);
        let mut app = App::new(
            &waker,
            dirs,
            Settings::default(),
            AppOptions {
                media_controls: false,
                tray: false,
            },
        );
        app.attach(&ctx);
        populate(&mut app);
        app.settings.milkdrop_screen_hz = 144;
        app.settings.milkdrop_fps = 60;
        app.open(Page::Settings);

        // Read labels from the real Settings page.
        let drawn = |app: &mut App, ctx: &egui::Context| -> Vec<String> {
            let input = egui::RawInput {
                // Draw the full Settings page, including MilkDrop.
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1280.0, 4000.0),
                )),
                ..Default::default()
            };
            let mut output = ctx.run_ui(input, |ui| app.frame_ui(ui));
            output.textures_delta.clear();
            let mut said = Vec::new();
            fn walk(shape: &egui::epaint::Shape, said: &mut Vec<String>) {
                match shape {
                    egui::epaint::Shape::Text(text) => said.push(text.galley.job.text.clone()),
                    egui::epaint::Shape::Vec(shapes) => {
                        shapes.iter().for_each(|shape| walk(shape, said))
                    }
                    _ => {}
                }
            }
            for clipped in &output.shapes {
                walk(&clipped.shape, &mut said);
            }
            said
        };

        for _ in 0..3 {
            let said = drawn(&mut app, &ctx);
            assert!(
                said.iter().any(|text| text.contains("60 fps")),
                "the dial names the rate it is on: {said:?}"
            );
        }

        // Every stop can be reached, and each names itself.
        for (rate, expected) in [
            (144, "144 fps, your screen"),
            (0, "Uncapped"),
            (30, "30 fps"),
        ] {
            app.settings.milkdrop_fps = rate;
            let said = drawn(&mut app, &ctx);
            assert!(
                said.iter().any(|text| text == expected),
                "the dial on {rate} should read {expected}: {said:?}"
            );
        }
        app.backend.shutdown();
    }

    /// Rule: at its narrowest the queue panel still puts its header on
    /// one line. The chips used to wrap under the buttons, and then,
    /// once the buttons were given their room first, onto a second row,
    /// which is a lot of panel spent on saying what two words already
    /// said.
    #[test]
    fn the_narrowest_panel_keeps_its_header_on_one_row() {
        let root = std::env::temp_dir().join(format!(
            "fastsonic-queue-header-test-{}",
            std::process::id()
        ));
        let dirs = AppDirs {
            config: root.join("config"),
            state: root.join("state"),
            cache: root.join("cache"),
        };
        let ctx = egui::Context::default();
        let waker = crate::backend::Waker::default();
        waker.attach(&ctx);
        let mut app = App::new(
            &waker,
            dirs,
            Settings::default(),
            AppOptions {
                media_controls: false,
                tray: false,
            },
        );
        app.attach(&ctx);
        populate(&mut app);
        app.show_queue_panel = true;
        app.settings.queue_width = crate::theme::SIDE_PANEL_MIN_WIDTH;

        // Where each piece of text was actually drawn.
        let mut placed: Vec<(String, f32)> = Vec::new();
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1280.0, 800.0),
            )),
            ..Default::default()
        };
        // The panel applies its width after the first frame.
        for _ in 0..2 {
            placed.clear();
            let mut output = ctx.run_ui(input.clone(), |ui| app.frame_ui(ui));
            output.textures_delta.clear();
            fn walk(shape: &egui::epaint::Shape, placed: &mut Vec<(String, f32)>) {
                match shape {
                    egui::epaint::Shape::Text(text) => {
                        placed.push((text.galley.job.text.clone(), text.pos.y))
                    }
                    egui::epaint::Shape::Vec(shapes) => {
                        shapes.iter().for_each(|shape| walk(shape, placed))
                    }
                    _ => {}
                }
            }
            for clipped in &output.shapes {
                walk(&clipped.shape, &mut placed);
            }
        }
        let at = |label: &str| -> f32 {
            placed
                .iter()
                .find(|(text, _)| text == label)
                .unwrap_or_else(|| panic!("{label} was never drawn: {placed:?}"))
                .1
        };
        let (queue, recents) = (at("Queue"), at("Recent"));
        assert!(
            (queue - recents).abs() < 1.0,
            "both chips sit on one line at {} wide: Queue at {queue}, Recent at {recents}",
            crate::theme::SIDE_PANEL_MIN_WIDTH
        );
        app.backend.shutdown();
    }

    /// Every page, panel, and dialog lays out without panicking.
    #[test]
    fn every_surface_renders_headless() {
        let root =
            std::env::temp_dir().join(format!("fastsonic-render-test-{}", std::process::id()));
        let dirs = AppDirs {
            config: root.join("config"),
            state: root.join("state"),
            cache: root.join("cache"),
        };
        let ctx = egui::Context::default();
        let waker = crate::backend::Waker::default();
        waker.attach(&ctx);
        let mut app = App::new(
            &waker,
            dirs,
            Settings::default(),
            AppOptions {
                media_controls: false,
                tray: false,
            },
        );
        app.attach(&ctx);
        populate(&mut app);

        let pages = [
            Page::Home,
            Page::TopSongs,
            Page::Search,
            Page::LikedSongs,
            Page::Albums,
            Page::Artists,
            // Nothing on this server has podcasts on it, so these two draw
            // their empty state until P5.3 takes the pages out.
            Page::Podcasts,
            Page::Episodes,
            Page::Playlist("pl1".into()),
            // Somebody else's public playlist, which offers no editing.
            Page::Playlist("pl0".into()),
            Page::Playlist("missing".into()),
            Page::Album("alb0".into()),
            // Two discs.
            Page::Album("alb5".into()),
            // A bare server's artist page: no Popular, no Fans also like.
            Page::Artist("art0".into()),
            // One with a Last.fm key, which has both.
            Page::Artist("art2".into()),
            Page::Queue,
            Page::Settings,
        ];
        for page in pages {
            app.open(page.clone());
            for _ in 0..3 {
                frame(&ctx, &mut app);
            }
            assert_eq!(app.page(), &page);
        }
        app.settings.sidebar_visible = false;
        frame(&ctx, &mut app);
        app.settings.sidebar_visible = true;
        app.show_queue_panel = true;
        app.show_devices = true;
        frame(&ctx, &mut app);
        // The panel's other shape. `populate` leaves a hand-queued row on
        // top of the album; this is the album on its own, so there is no
        // Playing next section and nothing to empty.
        app.handle_queue(QueueSnapshot {
            current: Some(QueueRow {
                uri: track_uri("trk0"),
                track: None,
            }),
            queued: Vec::new(),
            upcoming: (1..8)
                .map(|index| QueueRow {
                    uri: track_uri(&format!("trk{index}")),
                    track: None,
                })
                .collect(),
            context_uri: Some(album_uri("alb0")),
            context_at: Some(track_uri("trk0")),
        });
        frame(&ctx, &mut app);
        for dialog in [
            Dialog::Shortcuts,
            Dialog::CreatePlaylist {
                name: "x".into(),
                public: true,
                add_uris: vec![],
            },
            Dialog::EditPlaylist {
                id: "pl1".into(),
                name: "x".into(),
                description: String::new(),
                public: false,
            },
            Dialog::ConfirmDeletePlaylist {
                id: "pl1".into(),
                name: "x".into(),
                owned: true,
            },
        ] {
            app.dialog = Some(dialog);
            frame(&ctx, &mut app);
        }
        app.settings.theme = crate::settings::ThemeChoice::Light;
        app.actions.push(Action::SettingsChanged);
        app.open(Page::Home);
        for _ in 0..3 {
            frame(&ctx, &mut app);
        }
        assert!(!app.palette.dark);
        app.backend.shutdown();
        let _ = std::fs::remove_dir_all(root);
    }

    /// A drag in flight renders, and releasing it over an owned playlist
    /// row lands in the same add-to-playlist plumbing the row menu uses.
    #[test]
    fn dropping_a_song_on_a_sidebar_playlist_adds_it() {
        let root = std::env::temp_dir().join(format!("fastsonic-drag-test-{}", std::process::id()));
        let dirs = AppDirs {
            config: root.join("config"),
            state: root.join("state"),
            cache: root.join("cache"),
        };
        let ctx = egui::Context::default();
        let waker = crate::backend::Waker::default();
        waker.attach(&ctx);
        let mut app = App::new(
            &waker,
            dirs,
            Settings::default(),
            AppOptions {
                media_controls: false,
                tray: false,
            },
        );
        app.attach(&ctx);
        populate(&mut app);
        app.open(Page::Playlist("pl1".into()));
        for _ in 0..3 {
            frame(&ctx, &mut app);
        }

        // Sweep a held track down the sidebar; somewhere along the sweep
        // the pointer crosses an owned playlist row, and releasing there
        // must mark the playlist edit busy through the existing plumbing.
        // Where exactly the rows sit depends on the loaded fonts, so the
        // sweep does not hardcode a row position.
        let mut dropped = false;
        for step in 0..40 {
            let pos = egui::pos2(120.0, 120.0 + step as f32 * 15.0);
            egui::DragAndDrop::set_payload(
                &ctx,
                DragTrack {
                    uri: track_uri("trk0"),
                    title: "Rosewood".into(),
                    image: None,
                    from: None,
                },
            );
            frame_events(&ctx, &mut app, vec![egui::Event::PointerMoved(pos)]);
            frame_events(
                &ctx,
                &mut app,
                vec![egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed: false,
                    modifiers: egui::Modifiers::NONE,
                }],
            );
            assert!(!egui::DragAndDrop::has_any_payload(&ctx));
            if app.playlist_busy {
                dropped = true;
                break;
            }
        }
        assert!(dropped, "no sweep position landed on an owned playlist row");
        app.backend.shutdown();
        let _ = std::fs::remove_dir_all(root);
    }

    /// Pins are pins: dropping a pinned row at the top of the block
    /// reorders the pins themselves, and the rest of the shelf stays in
    /// its automatic order.
    #[test]
    fn dragging_within_the_pinned_block_reorders_it() {
        let root =
            std::env::temp_dir().join(format!("fastsonic-reorder-test-{}", std::process::id()));
        let dirs = AppDirs {
            config: root.join("config"),
            state: root.join("state"),
            cache: root.join("cache"),
        };
        let ctx = egui::Context::default();
        let waker = crate::backend::Waker::default();
        waker.attach(&ctx);
        let mut app = App::new(
            &waker,
            dirs,
            Settings::default(),
            AppOptions {
                media_controls: false,
                tray: false,
            },
        );
        app.attach(&ctx);
        populate(&mut app);
        app.settings.pinned_contexts = vec![playlist_uri("pl2"), playlist_uri("pl4")];
        for _ in 0..3 {
            frame(&ctx, &mut app);
        }

        // Sweep from the top: the first slot inside the list drops the
        // dragged row right under Liked Songs. Where the list begins
        // depends on the loaded fonts, so the sweep does not hardcode it.
        let mut dropped = false;
        for step in 0..40 {
            let pos = egui::pos2(120.0, 100.0 + step as f32 * 10.0);
            egui::DragAndDrop::set_payload(
                &ctx,
                DragEntry {
                    uri: playlist_uri("pl4"),
                    title: "New this month".into(),
                    image: None,
                },
            );
            frame_events(&ctx, &mut app, vec![egui::Event::PointerMoved(pos)]);
            frame_events(
                &ctx,
                &mut app,
                vec![egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed: false,
                    modifiers: egui::Modifiers::NONE,
                }],
            );
            egui::DragAndDrop::clear_payload(&ctx);
            if app.settings.pinned_contexts.first().map(String::as_str)
                == Some(playlist_uri("pl4").as_str())
            {
                dropped = true;
                break;
            }
        }
        assert!(dropped, "no sweep position landed in the pinned block");
        assert_eq!(
            app.settings.pinned_contexts,
            vec![playlist_uri("pl4"), playlist_uri("pl2")],
        );
        assert!(app.settings.sidebar_order.is_empty());
        app.backend.shutdown();
        let _ = std::fs::remove_dir_all(root);
    }

    /// Reordering unpinned playlists creates a custom sidebar order.
    #[test]
    fn dropping_between_unpinned_playlists_creates_the_custom_order() {
        let root =
            std::env::temp_dir().join(format!("fastsonic-unpinned-test-{}", std::process::id()));
        let dirs = AppDirs {
            config: root.join("config"),
            state: root.join("state"),
            cache: root.join("cache"),
        };
        let ctx = egui::Context::default();
        let waker = crate::backend::Waker::default();
        waker.attach(&ctx);
        let mut app = App::new(
            &waker,
            dirs,
            Settings::default(),
            AppOptions {
                media_controls: false,
                tray: false,
            },
        );
        app.attach(&ctx);
        populate(&mut app);
        assert!(app.settings.pinned_contexts.is_empty());
        assert!(app.settings.sidebar_order.is_empty());
        for _ in 0..3 {
            frame(&ctx, &mut app);
        }

        // Sweep from the top; the first slot inside the list is the one
        // right under Liked Songs, between what were the first two
        // unpinned playlists.
        let mut dropped = false;
        for step in 0..40 {
            let pos = egui::pos2(120.0, 100.0 + step as f32 * 10.0);
            egui::DragAndDrop::set_payload(
                &ctx,
                DragEntry {
                    uri: playlist_uri("pl4"),
                    title: "New this month".into(),
                    image: None,
                },
            );
            frame_events(&ctx, &mut app, vec![egui::Event::PointerMoved(pos)]);
            frame_events(
                &ctx,
                &mut app,
                vec![egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed: false,
                    modifiers: egui::Modifiers::NONE,
                }],
            );
            egui::DragAndDrop::clear_payload(&ctx);
            if !app.settings.sidebar_order.is_empty() {
                dropped = true;
                break;
            }
        }
        assert!(dropped, "no sweep position landed below Liked Songs");
        let expected: Vec<String> = std::iter::once(4)
            .chain((0..PLAYLISTS.len()).filter(|index| *index != 4))
            .map(|index| playlist_uri(&format!("pl{index}")))
            .collect();
        assert_eq!(app.settings.sidebar_order, expected);
        assert!(app.settings.pinned_contexts.is_empty());
        app.backend.shutdown();
        let _ = std::fs::remove_dir_all(root);
    }

    /// Dragging a row within an owned playlist's table moves it through
    /// the same MoveInPlaylist action the menu's move items use: the slot
    /// is Spotify's insert-before, which the handler mirrors locally
    /// before asking the server.
    #[test]
    fn dragging_a_row_within_a_playlist_reorders_it() {
        let root = std::env::temp_dir().join(format!("fastsonic-move-test-{}", std::process::id()));
        let dirs = AppDirs {
            config: root.join("config"),
            state: root.join("state"),
            cache: root.join("cache"),
        };
        let ctx = egui::Context::default();
        let waker = crate::backend::Waker::default();
        waker.attach(&ctx);
        let mut app = App::new(
            &waker,
            dirs,
            Settings::default(),
            AppOptions {
                media_controls: false,
                tray: false,
            },
        );
        app.attach(&ctx);
        populate(&mut app);
        app.open(Page::Playlist("pl1".into()));
        for _ in 0..3 {
            frame(&ctx, &mut app);
        }
        let order = |app: &App| -> Vec<String> {
            app.playlist_pages["pl1"]
                .items
                .items
                .iter()
                .filter_map(|item| item.playable().map(|playable| playable.uri().to_string()))
                .collect()
        };
        let original = order(&app);
        let from = 5usize;
        let held = |from: usize, uri: &str| DragTrack {
            uri: uri.to_string(),
            title: "Elysian".into(),
            image: None,
            from: Some(("pl1".into(), from as u32)),
        };

        // Sweep the held row down the page; above the table nothing
        // bites, and the first slot inside it lands the row above its old
        // place. Where the table begins depends on the loaded fonts, so
        // the sweep does not hardcode it.
        let mut landed = None;
        for step in 0..45 {
            let pos = egui::pos2(700.0, 120.0 + step as f32 * 15.0);
            egui::DragAndDrop::set_payload(&ctx, held(from, &original[from]));
            frame_events(&ctx, &mut app, vec![egui::Event::PointerMoved(pos)]);
            frame_events(
                &ctx,
                &mut app,
                vec![egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed: false,
                    modifiers: egui::Modifiers::NONE,
                }],
            );
            egui::DragAndDrop::clear_payload(&ctx);
            if app.playlist_busy {
                landed = Some(pos);
                break;
            }
        }
        let landed = landed.expect("no sweep position landed inside the table");
        let drop_at = |ctx: &egui::Context, app: &mut App, payload: DragTrack| {
            egui::DragAndDrop::set_payload(ctx, payload);
            frame_events(ctx, app, vec![egui::Event::PointerMoved(landed)]);
            frame_events(
                ctx,
                app,
                vec![egui::Event::PointerButton {
                    pos: landed,
                    button: egui::PointerButton::Primary,
                    pressed: false,
                    modifiers: egui::Modifiers::NONE,
                }],
            );
            egui::DragAndDrop::clear_payload(ctx);
        };
        // The handler mirrored the move locally: the dragged row moved
        // up, everything else kept its order.
        let now = order(&app);
        let to = now
            .iter()
            .position(|uri| *uri == original[from])
            .expect("the dragged row vanished");
        assert!(to < from, "the row should have moved up, not to {to}");
        let mut expected = original.clone();
        let moved = expected.remove(from);
        expected.insert(to, moved);
        assert_eq!(now, expected);

        // Dropping the row on the same slot again moves nothing: the slot
        // is insert-before, so a row's own edges are a no-op. A slot sent
        // one row out would move it here.
        app.playlist_busy = false;
        drop_at(&ctx, &mut app, held(to, &expected[to]));
        assert!(!app.playlist_busy, "a row dropped on its own slot moved");
        assert_eq!(order(&app), expected);

        // A sorted view refuses the move: positions on screen no longer
        // match the server's.
        app.table_sorts.insert(
            Page::Playlist("pl1".into()),
            TableSort {
                column: SortColumn::Title,
                ascending: true,
            },
        );
        frame(&ctx, &mut app);
        drop_at(&ctx, &mut app, held(to, &expected[to]));
        assert!(!app.playlist_busy, "a sorted view accepted a move");
        assert_eq!(order(&app), expected);
        app.backend.shutdown();
        let _ = std::fs::remove_dir_all(root);
    }

    /// The custom order is a setting like any other: it survives the trip
    /// through the settings file, and older files without it stay in the
    /// automatic order.
    #[test]
    fn custom_sidebar_order_round_trips_through_settings() {
        let settings = Settings {
            sidebar_order: vec![playlist_uri("pl4"), playlist_uri("pl0")],
            ..Settings::default()
        };
        let json = serde_json::to_string(&settings).unwrap();
        let restored: Settings = serde_json::from_str(&json).unwrap();
        assert_eq!(restored.sidebar_order, settings.sidebar_order);
        let older: Settings = serde_json::from_str("{}").unwrap();
        assert!(older.sidebar_order.is_empty());
    }
}
