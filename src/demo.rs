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

/// Invented Hebrew and Arabic titles for `--demo-show rtl`: whole lines,
/// lines mixed with English, numbers, brackets, and punctuation.
#[cfg(feature = "demo")]
const RTL_TRACKS: &[(&str, &str, &str)] = &[
    ("שיר ישן (גרסה חיה)", "להקת הים", "גלים, 2024"),
    ("Song 12 שיר ישן, part 3", "Kasia & נועה", "Sessions: חלק ב"),
    ("غيوم في السماء (Live) 2024", "فرقة الغيوم", "السماء"),
    ("ليل طويل، الجزء الأول", "نور", "رحلة 7"),
    ("Tel Aviv Nights: לילות, חלק 2", "Sam & דנה", "Nights"),
    ("مدينة [Remix]", "فرقة الغيوم", "Remixes: مدينة"),
];

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
/// How long ago the nth liked song was starred. Spread across the labels
/// `util::format_relative_date` can produce, so a screenshot of the Date
/// Added column shows the range rather than five copies of one word.
fn starred_ago(index: usize) -> SignedDuration {
    match index {
        0 => SignedDuration::from_mins(20),
        1 => SignedDuration::from_hours(6),
        2 => SignedDuration::from_hours(3 * 24),
        3 => SignedDuration::from_hours(3 * 7 * 24),
        _ => SignedDuration::from_hours((40 + index as i64 * 9) * 24),
    }
}

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
    // Screenshots look the same whatever the desktop prefers; `light` asks
    // for the other theme.
    app.settings.theme = crate::settings::ThemeChoice::Dark;
    app.auth = AuthStatus::Connected {
        username: USER.into(),
    };
    app.local_ready = true;
    app.local_playback = LocalPlayback::Ready;
    app.user = Some(User {
        id: USER.into(),
        display_name: Some(USER.into()),
        // No avatar, no plan, no country: `getUser` carries none of them,
        // and every account on a server you own can stream.
        images: Vec::new(),
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

    // Radio pages. A song's and a playlist's, as a server with a Last.fm
    // key answers them, and `art0`'s from a bare server, where
    // `getSimilarSongs2` answers `{}` as it does for every artist.
    for (seed, name, images, first) in [
        (
            track_uri("trk0"),
            &songs[0].name,
            songs[0].album.as_ref().map(|album| album.images.clone()),
            1,
        ),
        (
            playlist_uri("pl1"),
            &playlists[1].name,
            Some(playlists[1].images.clone()),
            8,
        ),
    ] {
        app.radio_pages.insert(
            seed,
            RadioPage {
                songs: Loadable::Loaded(
                    songs
                        .iter()
                        .skip(first)
                        .step_by(2)
                        .take(30)
                        .cloned()
                        .collect(),
                ),
                name: Some(name.clone()),
                images: images.unwrap_or_default(),
                ..RadioPage::default()
            },
        );
    }
    app.radio_pages.insert(
        artist_uri("art0"),
        RadioPage {
            songs: Loadable::Loaded(Vec::new()),
            name: Some(artists[0].name.clone()),
            images: artists[0].images.clone(),
            ..RadioPage::default()
        },
    );

    // Library. Starred songs carry the date they were starred, as
    // `getStarred2` reports it, newest first, and starred albums the date
    // `getAlbumList2` reports, listed in the server's own order. Starred
    // artists carry none, because the Library keeps no date for them.
    let starred_at = Timestamp::now();
    app.library.liked.absorb(
        0,
        page(
            songs
                .iter()
                .filter(|track| track.starred.unwrap_or_default())
                .enumerate()
                .map(|(index, track)| SavedTrack {
                    added_at: Some((starred_at - starred_ago(index)).to_string()),
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
                .enumerate()
                .map(|(index, album)| SavedAlbum {
                    // The last one listed was starred most recently.
                    added_at: Some((starred_at - starred_ago(RECORDS.len() - index)).to_string()),
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

    // Search. `search3` has three buckets and no playlists.
    app.search.query = "Bonobo".into();
    app.search.committed = "Bonobo".into();
    app.search.results = Loadable::Loaded(SearchResults {
        tracks: Some(page(songs.iter().take(10).cloned().collect())),
        artists: Some(page(artists.iter().take(6).cloned().collect())),
        albums: Some(page(albums.iter().take(6).cloned().collect())),
        playlists: None,
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
        track_sequence: 1,
    });
}

/// Words to go with the sample track, timed so that the one being sung
/// sits mid-panel at the demo's playback position.
#[cfg(any(test, feature = "demo"))]
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
            "playing-next" => {
                app.show_queue_panel = true;
                app.manual_queue = app
                    .queue
                    .queue
                    .iter()
                    .take(6)
                    .map(|item| item.uri().to_string())
                    .collect();
            }
            "recents" => {
                app.show_queue_panel = true;
                app.queue_tab = QueueTab::Recents;
            }
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
            "taskbar" => app.taskbar_hiding_supported = true,
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
            "lyrics" | "lyrics-fullscreen" => {
                app.lyrics_uri = app.now_playing().map(|now| now.uri);
                app.lyrics = Loadable::Loaded(Some(sample_lyrics()));
                app.lyrics_following = true;
                app.show_lyrics_panel = true;
                if surface == "lyrics-fullscreen" {
                    app.actions.push(Action::SetLyricsFullscreen(true));
                }
            }
            "player-bar-spectrum" | "player-bar-waveform" => {
                app.settings.player_bar_vis = if surface == "player-bar-spectrum" {
                    crate::settings::PlayerBarVis::Spectrum
                } else {
                    crate::settings::PlayerBarVis::Waveform
                };
                // Enough fixed sound for the tap's speaker lag and the
                // analyzer window. No server or audio device is involved.
                let samples: Vec<f64> = (0..20_000)
                    .flat_map(|frame| {
                        let phase = frame as f64 / 44_100.0 * std::f64::consts::TAU;
                        let sample = (phase * 220.0).sin() * 0.35
                            + (phase * 440.0).sin() * 0.2
                            + (phase * 880.0).sin() * 0.1;
                        [sample, sample]
                    })
                    .collect();
                app.winamp.tap.push(&samples, 1.0);
            }
            // Titles in scripts the interface font does not cover.
            "rtl" => {
                if let Some(page) = app.playlist_pages.get_mut("pl1") {
                    for (item, &(title, artist, album)) in
                        page.items.items.iter_mut().zip(RTL_TRACKS)
                    {
                        if let Some(PlayableItem::Track(track)) = &mut item.item {
                            track.name = title.into();
                            if let Some(first) = track.artists.first_mut() {
                                first.name = artist.into();
                            }
                            if let Some(album_ref) = &mut track.album {
                                album_ref.name = album.into();
                            }
                        }
                    }
                    page.items.revision += 1;
                }
            }
            // The sign-in card and the card while the session connects.
            "signed-out" => {
                app.auth = AuthStatus::SignedOut;
                app.user = None;
            }
            "connecting" => {
                app.auth = AuthStatus::Connecting;
                app.user = None;
            }
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
                    let PlayableItem::Track(track) = item;
                    rename(track, names);
                }
                if let Some(item) = &mut app.queue.currently_playing {
                    let PlayableItem::Track(track) = item;
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
    use std::sync::Arc;

    /// Only a window backend that can leave the mini player out of the
    /// taskbar offers the switch, and flipping it keeps Settings open.
    #[test]
    fn the_taskbar_setting_keeps_its_choice_without_closing_settings() {
        use egui::accesskit::Role;
        let (ctx, mut app) = accessible_app("winamp-taskbar-setting");
        app.taskbar_hiding_supported = false;
        app.open(Page::Settings);
        accessible_frame(&ctx, &mut app, vec![]);
        let tree = accessible_frame(&ctx, &mut app, vec![]);
        assert!(
            !tree
                .nodes
                .iter()
                .any(|(_, node)| node.label() == Some("Show Winamp in taskbar"))
        );
        app.taskbar_hiding_supported = true;
        accessible_frame(&ctx, &mut app, vec![]);
        let tree = accessible_frame(&ctx, &mut app, vec![]);
        let control = accessible_node(&tree, "Show Winamp in taskbar", Role::CheckBox);
        accessible_frame(
            &ctx,
            &mut app,
            vec![accessible_action(
                control,
                egui::accesskit::Action::Click,
                None,
            )],
        );
        accessible_frame(&ctx, &mut app, vec![]);
        assert!(!app.settings.winamp_show_taskbar);
        assert!(!app.settings.winamp_window && !app.switch_intent);
        assert_eq!(app.page(), &Page::Settings);
        let path = app.dirs.config.join("winamp-taskbar-choice.json");
        app.settings.save(&path);
        assert!(!Settings::load(&path).winamp_show_taskbar);
        app.backend.shutdown();
    }

    #[test]
    fn wayland_on_top_setting_is_disabled_and_does_not_look_active() {
        use egui::accesskit::{Role, Toggled};
        let (ctx, mut app) = accessible_app("wayland-on-top");
        app.window_level_supported = false;
        app.settings.winamp_on_top = true;
        app.open(Page::Settings);
        accessible_frame(&ctx, &mut app, vec![]);
        let tree = accessible_frame(&ctx, &mut app, vec![]);
        let id = accessible_node(&tree, "Always on top", Role::CheckBox);
        let node = &tree
            .nodes
            .iter()
            .find(|(node_id, _)| *node_id == id)
            .unwrap()
            .1;
        assert!(node.is_disabled());
        assert_eq!(node.toggled(), Some(Toggled::False));
        assert!(
            app.settings.winamp_on_top,
            "the saved preference is preserved"
        );
        app.backend.shutdown();
    }

    fn accessible_app(name: &str) -> (egui::Context, App) {
        let root =
            std::env::temp_dir().join(format!("fastsonic-a11y-{name}-{}", std::process::id()));
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        let waker = crate::backend::Waker::default();
        waker.attach(&ctx);
        let mut app = App::new(
            &waker,
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
        app.attach(&ctx);
        populate(&mut app);
        (ctx, app)
    }

    fn accessible_frame(
        ctx: &egui::Context,
        app: &mut App,
        events: Vec<egui::Event>,
    ) -> egui::accesskit::TreeUpdate {
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1280.0, 800.0),
                )),
                events,
                ..Default::default()
            },
            |ui| app.frame_ui(ui),
        );
        output.textures_delta.clear();
        output
            .platform_output
            .accesskit_update
            .expect("screen-reader tree")
    }

    fn accessible_node(
        tree: &egui::accesskit::TreeUpdate,
        label: &str,
        role: egui::accesskit::Role,
    ) -> egui::accesskit::NodeId {
        tree.nodes
            .iter()
            .find(|(_, node)| node.label() == Some(label) && node.role() == role)
            .unwrap_or_else(|| panic!("missing {label:?} with role {role:?}"))
            .0
    }

    fn accessible_action(
        target: egui::accesskit::NodeId,
        action: egui::accesskit::Action,
        data: Option<egui::accesskit::ActionData>,
    ) -> egui::Event {
        egui::Event::AccessKitActionRequest(egui::accesskit::ActionRequest {
            target_tree: egui::accesskit::TreeId::ROOT,
            target_node: target,
            action,
            data,
        })
    }

    fn keyboard(key: egui::Key, modifiers: egui::Modifiers) -> egui::Event {
        egui::Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers,
        }
    }

    #[test]
    fn accessible_navigation_and_pause_work_without_a_pointer() {
        use egui::accesskit::{Action, Role};
        let (ctx, mut app) = accessible_app("navigate");
        accessible_frame(&ctx, &mut app, vec![]);
        let tree = accessible_frame(&ctx, &mut app, vec![]);
        let liked = accessible_node(&tree, "Liked Songs", Role::Button);
        accessible_frame(
            &ctx,
            &mut app,
            vec![accessible_action(liked, Action::Click, None)],
        );
        assert_eq!(app.page(), &Page::LikedSongs);
        let tree = accessible_frame(&ctx, &mut app, vec![]);
        let pause = accessible_node(&tree, "Pause", Role::Button);
        assert!(app.believed_playing());
        accessible_frame(
            &ctx,
            &mut app,
            vec![accessible_action(pause, Action::Focus, None)],
        );
        let tree = accessible_frame(
            &ctx,
            &mut app,
            vec![keyboard(egui::Key::Space, egui::Modifiers::NONE)],
        );
        assert!(
            !app.believed_playing(),
            "focused Space must pause once, without firing the global shortcut too"
        );
        assert_eq!(tree.focus, pause);
        app.backend.shutdown();
    }

    /// #596: clearing the global search from another page empties the field
    /// without leaving that page.
    #[test]
    fn clearing_the_global_search_stays_on_the_current_page() {
        use egui::accesskit::{Action as AccessibleAction, Role};
        let (ctx, mut app) = accessible_app("global-search-clear");
        app.open(Page::Home);
        accessible_frame(&ctx, &mut app, vec![]);
        let tree = accessible_frame(&ctx, &mut app, vec![]);
        assert!(!app.search.query.is_empty());
        let clear = tree
            .nodes
            .iter()
            .find(|(_, node)| {
                node.label() == Some("Clear")
                    && node.role() == Role::Button
                    && node.bounds().is_some_and(|bounds| bounds.y1 < 80.0)
            })
            .expect("Clear beside the global search")
            .0;
        accessible_frame(
            &ctx,
            &mut app,
            vec![accessible_action(clear, AccessibleAction::Click, None)],
        );
        for _ in 0..3 {
            accessible_frame(&ctx, &mut app, vec![]);
        }
        assert!(app.search.query.is_empty());
        assert!(
            matches!(app.page(), Page::Home),
            "clearing must not open the search page"
        );
        assert!(
            ctx.memory(|memory| memory.has_focus(egui::Id::new("global-search"))),
            "the field stays ready for the next query"
        );
        // Typing a new query still goes to the search page.
        accessible_frame(&ctx, &mut app, vec![egui::Event::Text("Rework".into())]);
        accessible_frame(&ctx, &mut app, vec![]);
        assert_eq!(app.search.query, "Rework");
        assert!(matches!(app.page(), Page::Search));
        app.backend.shutdown();
    }

    #[test]
    fn accessible_sliders_accept_keyboard_and_screen_reader_values() {
        use crate::ui::widgets::{SliderEvent, thin_slider};
        use egui::accesskit::{Action, ActionData, Role};
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        crate::theme::install(&ctx);
        let palette = crate::theme::Palette::dark();
        let mut value = 0.5;
        let mut render = |events| {
            let mut output = ctx.run_ui(
                egui::RawInput {
                    events,
                    ..Default::default()
                },
                |ui| {
                    if let SliderEvent::Committed(next) = thin_slider(
                        ui,
                        &palette,
                        egui::Id::new("test-volume"),
                        "Volume (%)",
                        value,
                        200.0,
                        Some(0.05),
                    ) {
                        value = next;
                    }
                },
            );
            output.textures_delta.clear();
            (value, output.platform_output.accesskit_update.unwrap())
        };
        let (_, tree) = render(vec![]);
        let slider = accessible_node(&tree, "Volume (%)", Role::Slider);
        render(vec![accessible_action(slider, Action::Focus, None)]);
        let (value, _) = render(vec![keyboard(egui::Key::ArrowRight, egui::Modifiers::NONE)]);
        assert!((value - 0.55).abs() < 0.001);
        let (value, tree) = render(vec![accessible_action(
            slider,
            Action::SetValue,
            Some(ActionData::NumericValue(35.0)),
        )]);
        assert!((value - 0.35).abs() < 0.001);
        let node = &tree.nodes.iter().find(|(id, _)| *id == slider).unwrap().1;
        assert!((node.numeric_value().unwrap() - 35.0).abs() < 0.001);
        assert_eq!(node.min_numeric_value(), Some(0.0));
        assert_eq!(node.max_numeric_value(), Some(100.0));
        let (value, _) = render(vec![accessible_action(
            slider,
            Action::SetValue,
            Some(ActionData::NumericValue(200.0)),
        )]);
        assert_eq!(value, 1.0);
        let (value, _) = render(vec![accessible_action(
            slider,
            Action::SetValue,
            Some(ActionData::NumericValue(f64::NAN)),
        )]);
        assert_eq!(value, 1.0);
    }

    #[test]
    fn accessible_switch_has_a_name_state_and_keyboard_activation() {
        use egui::accesskit::{Action, Role, Toggled};
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        crate::theme::install(&ctx);
        let palette = crate::theme::Palette::dark();
        let mut on = false;
        let mut render = |events| {
            let mut output = ctx.run_ui(
                egui::RawInput {
                    events,
                    ..Default::default()
                },
                |ui| {
                    crate::ui::widgets::switch(ui, &palette, "Autoplay", &mut on);
                },
            );
            output.textures_delta.clear();
            (on, output.platform_output.accesskit_update.unwrap())
        };
        let (_, tree) = render(vec![]);
        let switch = accessible_node(&tree, "Autoplay", Role::CheckBox);
        let node = &tree.nodes.iter().find(|(id, _)| *id == switch).unwrap().1;
        assert_eq!(node.toggled(), Some(Toggled::False));
        render(vec![accessible_action(switch, Action::Focus, None)]);
        let (on, tree) = render(vec![keyboard(egui::Key::Space, egui::Modifiers::NONE)]);
        assert!(on);
        let node = &tree.nodes.iter().find(|(id, _)| *id == switch).unwrap().1;
        assert_eq!(node.toggled(), Some(Toggled::True));
    }

    #[test]
    fn accessible_song_focus_survives_visible_row_changes_and_keeps_duplicates_distinct() {
        use crate::ui::widgets::{TrackRow, track_row};
        use egui::accesskit::{Action as AccessibleAction, Role};
        let (ctx, mut app) = accessible_app("rows");
        let item = app.queue.queue[0].clone();
        let context = crate::model::RowContext::Uris(Arc::from(vec![item.uri().to_string(); 2]));
        let label = format!("Play {}, {}", item.name(), item.subtitle());
        let mut render = |first, events| {
            app.actions.clear();
            let mut output = ctx.run_ui(
                egui::RawInput {
                    events,
                    ..Default::default()
                },
                |ui| {
                    for index in first..2 {
                        track_row(
                            ui,
                            &mut app,
                            TrackRow {
                                index,
                                number: Some(index + 1),
                                item: &item,
                                context: &context,
                                show_cover: false,
                                show_album: false,
                                added_at: None,
                                added_by: None,
                                show_added_by: false,
                                compact: false,
                                thin: false,
                                shift: 0.0,
                                picked: false,
                                picked_songs: &[],
                            },
                        );
                    }
                },
            );
            output.textures_delta.clear();
            (
                output.platform_output.accesskit_update.unwrap(),
                app.actions.clone(),
            )
        };
        let (tree, _) = render(0, vec![]);
        let mut positioned_rows: Vec<_> = tree
            .nodes
            .iter()
            .filter(|(_, node)| node.label() == Some(label.as_str()) && node.role() == Role::Button)
            .map(|(id, node)| (*id, node.bounds().unwrap().y0))
            .collect();
        positioned_rows.sort_by(|a, b| a.1.total_cmp(&b.1));
        let rows: Vec<_> = positioned_rows.into_iter().map(|(id, _)| id).collect();
        assert_eq!(rows.len(), 2);
        assert_ne!(rows[0], rows[1]);
        render(
            0,
            vec![accessible_action(rows[1], AccessibleAction::Focus, None)],
        );
        let (tree, _) = render(1, vec![]);
        assert_eq!(
            accessible_node(&tree, &label, Role::Button),
            rows[1],
            "scrolling must not give a song another row's identity"
        );
        assert_eq!(tree.focus, rows[1]);
        let (_, actions) = render(1, vec![keyboard(egui::Key::Enter, egui::Modifiers::NONE)]);
        assert!(matches!(
            actions.as_slice(),
            [crate::model::Action::PlayFromRow { index: 1, .. }]
        ));
        let (tree, _) = render(1, vec![]);
        let more = accessible_node(&tree, "More", Role::Button);
        render(
            1,
            vec![accessible_action(more, AccessibleAction::Focus, None)],
        );
        let (tree, _) = render(1, vec![]);
        assert_eq!(
            tree.focus, more,
            "More must remain reachable after focus leaves the song row"
        );
        let (tree, _) = render(1, vec![keyboard(egui::Key::Enter, egui::Modifiers::NONE)]);
        assert!(
            tree.nodes
                .iter()
                .any(|(_, node)| node.label() == Some("Add to queue")),
            "the keyboard opens the song menu"
        );
        app.backend.shutdown();
    }

    #[test]
    fn accessible_tab_reaches_songs_beyond_the_visible_list() {
        use crate::ui::widgets::{TrackRow, track_row, virtual_rows};
        use egui::accesskit::{Action as AccessibleAction, Role};
        let (ctx, mut app) = accessible_app("scroll");
        let item = app.queue.queue[0].clone();
        let context = crate::model::RowContext::Uris(Arc::from(vec![item.uri().to_string(); 20]));
        let label = format!("Play {}, {}", item.name(), item.subtitle());
        let mut reached_last = false;
        let mut render = |events| {
            let mut output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(700.0, 200.0),
                    )),
                    events,
                    ..Default::default()
                },
                |ui| {
                    egui::ScrollArea::vertical().animated(false).show(ui, |ui| {
                        virtual_rows(ui, 20, crate::theme::ROW_HEIGHT, |ui, index| {
                            if index == 19 && ui.cursor().top() < ui.clip_rect().bottom() {
                                reached_last = true;
                            }
                            track_row(
                                ui,
                                &mut app,
                                TrackRow {
                                    index,
                                    number: Some(index + 1),
                                    item: &item,
                                    context: &context,
                                    show_cover: false,
                                    show_album: false,
                                    added_at: None,
                                    added_by: None,
                                    show_added_by: false,
                                    compact: false,
                                    thin: false,
                                    shift: 0.0,
                                    picked: false,
                                    picked_songs: &[],
                                },
                            );
                        });
                    });
                },
            );
            output.textures_delta.clear();
            output.platform_output.accesskit_update.unwrap()
        };
        let tree = render(vec![]);
        let first = accessible_node(&tree, &label, Role::Button);
        render(vec![accessible_action(
            first,
            AccessibleAction::Focus,
            None,
        )]);
        for _ in 0..120 {
            render(vec![keyboard(egui::Key::Tab, egui::Modifiers::NONE)]);
        }
        assert!(
            reached_last,
            "Tab must scroll through the virtual list instead of trapping focus in its first visible rows"
        );
        app.backend.shutdown();
    }

    #[test]
    fn accessible_tab_reaches_cards_beyond_the_visible_grid() {
        use crate::ui::widgets::{card, card_row_height, virtual_wrapped_cards};
        use egui::accesskit::{Action as AccessibleAction, Role};
        let (ctx, mut app) = accessible_app("card-grid");
        let mut reached_last = false;
        let mut render = |events| {
            let mut output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(400.0, 280.0),
                    )),
                    events,
                    ..Default::default()
                },
                |ui| {
                    egui::ScrollArea::vertical().animated(false).show(ui, |ui| {
                        let height = card_row_height(ui);
                        virtual_wrapped_cards(ui, 24, height, |ui, index| {
                            if index == 23 && ui.cursor().top() < ui.clip_rect().bottom() {
                                reached_last = true;
                            }
                            card(
                                ui,
                                &mut app,
                                None,
                                &format!("Album {index}"),
                                "Artist",
                                false,
                                false,
                            );
                        });
                    });
                },
            );
            output.textures_delta.clear();
            output.platform_output.accesskit_update.unwrap()
        };
        let tree = render(vec![]);
        let first = accessible_node(&tree, "Album 0, Artist", Role::Button);
        render(vec![accessible_action(
            first,
            AccessibleAction::Focus,
            None,
        )]);
        for _ in 0..80 {
            render(vec![keyboard(egui::Key::Tab, egui::Modifiers::NONE)]);
        }
        assert!(
            reached_last,
            "Tab must scroll through the virtual card grid instead of trapping focus in its first visible row"
        );
        app.backend.shutdown();
    }

    #[test]
    fn accessible_playing_and_queued_copies_target_their_own_context() {
        use crate::model::RowContext;
        use crate::ui::widgets::{TrackRow, track_row};
        use egui::accesskit::{Action as AccessibleAction, Role};
        let (ctx, mut app) = accessible_app("queued-copy");
        let item = app.queue.currently_playing.clone().unwrap();
        let contexts = [
            RowContext::Uris(Arc::from([item.uri().to_string()])),
            RowContext::Queue,
        ];
        let label = format!("Play {}, {}", item.name(), item.subtitle());
        let mut render = |events| {
            app.actions.clear();
            let mut output = ctx.run_ui(
                egui::RawInput {
                    events,
                    ..Default::default()
                },
                |ui| {
                    for context in &contexts {
                        track_row(
                            ui,
                            &mut app,
                            TrackRow {
                                index: 0,
                                number: Some(1),
                                item: &item,
                                context,
                                show_cover: false,
                                show_album: false,
                                added_at: None,
                                added_by: None,
                                show_added_by: false,
                                compact: false,
                                thin: false,
                                shift: 0.0,
                                picked: false,
                                picked_songs: &[],
                            },
                        );
                    }
                },
            );
            output.textures_delta.clear();
            (
                output.platform_output.accesskit_update.unwrap(),
                app.actions.clone(),
            )
        };
        let (tree, _) = render(vec![]);
        let mut rows: Vec<_> = tree
            .nodes
            .iter()
            .filter(|(_, node)| node.label() == Some(label.as_str()) && node.role() == Role::Button)
            .map(|(id, node)| (*id, node.bounds().unwrap().y0))
            .collect();
        rows.sort_by(|a, b| a.1.total_cmp(&b.1));
        assert_eq!(
            rows.len(),
            2,
            "the playing song and queued copy need separate controls"
        );
        let (_, actions) = render(vec![accessible_action(
            rows[1].0,
            AccessibleAction::Click,
            None,
        )]);
        assert!(matches!(
            actions.as_slice(),
            [crate::model::Action::PlayFromRow {
                context: RowContext::Queue,
                index: 0,
                ..
            }]
        ));
        let (_, actions) = render(vec![accessible_action(
            rows[0].0,
            AccessibleAction::Click,
            None,
        )]);
        assert!(matches!(
            actions.as_slice(),
            [crate::model::Action::PlayFromRow {
                context: RowContext::Uris(_),
                index: 0,
                ..
            }]
        ));
        app.backend.shutdown();
    }

    fn frame(ctx: &egui::Context, app: &mut App) {
        frame_events(ctx, app, Vec::new());
    }

    fn song(uri: &str, name: &str) -> PlayableItem {
        PlayableItem::Track(crate::api::models::Track {
            uri: uri.into(),
            name: name.into(),
            ..Default::default()
        })
    }

    fn view_frame(
        ctx: &egui::Context,
        app: &mut App,
        events: Vec<egui::Event>,
        view: fn(&mut App, &mut egui::Ui),
    ) -> Vec<(String, egui::Rect)> {
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1280.0, 2200.0),
                )),
                events,
                ..Default::default()
            },
            |ui| view(app, ui),
        );
        output.textures_delta.clear();
        fn walk(shape: &egui::epaint::Shape, text: &mut Vec<(String, egui::Rect)>) {
            match shape {
                egui::epaint::Shape::Text(shape) => text.push((
                    shape.galley.job.text.clone(),
                    shape.galley.rect.translate(shape.pos.to_vec2()),
                )),
                egui::epaint::Shape::Vec(shapes) => {
                    shapes.iter().for_each(|shape| walk(shape, text));
                }
                _ => {}
            }
        }
        let mut text = Vec::new();
        for shape in &output.shapes {
            walk(&shape.shape, &mut text);
        }
        text
    }

    fn pointer_click(pos: egui::Pos2, button: egui::PointerButton) -> Vec<egui::Event> {
        vec![
            egui::Event::PointerMoved(pos),
            egui::Event::PointerButton {
                pos,
                button,
                pressed: true,
                modifiers: egui::Modifiers::NONE,
            },
            egui::Event::PointerButton {
                pos,
                button,
                pressed: false,
                modifiers: egui::Modifiers::NONE,
            },
        ]
    }

    /// The empty space's tooltip belongs to the empty space: once shown, it
    /// closes when the pointer moves onto a control drawn over the bar.
    #[cfg(feature = "demo")]
    #[test]
    fn the_visualizer_tooltip_stays_off_the_player_bar_controls() {
        let (ctx, mut app) = accessible_app("player-bar-tooltip");
        ctx.global_style_mut(|style| style.interaction.tooltip_delay = 0.0);
        let mut time = 0.0;
        let mut draw = |app: &mut App, pos: egui::Pos2| {
            time += 1.0;
            let mut output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(1280.0, 800.0),
                    )),
                    time: Some(time),
                    events: vec![egui::Event::PointerMoved(pos)],
                    ..Default::default()
                },
                |ui| crate::ui::player_bar::show(app, ui),
            );
            output.textures_delta.clear();
            fn texts(shape: &egui::epaint::Shape, found: &mut Vec<String>) {
                match shape {
                    egui::epaint::Shape::Text(text) => found.push(text.galley.job.text.clone()),
                    egui::epaint::Shape::Vec(shapes) => {
                        shapes.iter().for_each(|shape| texts(shape, found));
                    }
                    _ => {}
                }
            }
            let mut found = Vec::new();
            output
                .shapes
                .iter()
                .for_each(|clipped| texts(&clipped.shape, &mut found));
            found
        };
        let tip = "Click to change the visualizer";

        // The tooltip shown over the empty margin beside the cover.
        let empty = egui::pos2(6.0, 796.0);
        for _ in 0..3 {
            draw(&mut app, empty);
        }
        assert!(draw(&mut app, empty).iter().any(|text| text == tip));

        // The pointer moves onto the play button.
        let play = egui::pos2(640.0, 800.0 - crate::theme::PLAYER_BAR_HEIGHT / 2.0 - 10.0);
        for _ in 0..3 {
            draw(&mut app, play);
        }

        let shown = draw(&mut app, play);
        assert!(!shown.iter().any(|text| text == tip), "{shown:?}");
        app.backend.shutdown();
    }

    #[test]
    fn the_liked_songs_menu_is_as_narrow_as_the_other_menus() {
        use egui::accesskit::Role;
        let (ctx, mut app) = accessible_app("liked-menu-width");
        let button = |tree: &egui::accesskit::TreeUpdate, label: &str| {
            let bounds = tree
                .nodes
                .iter()
                .find(|(_, node)| node.role() == Role::Button && node.label() == Some(label))
                .unwrap_or_else(|| panic!("missing button {label}"))
                .1
                .bounds()
                .unwrap();
            (
                bounds.x0 as f32,
                bounds.x1 as f32,
                bounds.y0 as f32,
                bounds.y1 as f32,
            )
        };

        let tree = accessible_frame(&ctx, &mut app, vec![]);
        let (x0, x1, y0, y1) = button(&tree, "Liked Songs");
        accessible_frame(
            &ctx,
            &mut app,
            pointer_click(
                egui::pos2((x0 + x1) / 2.0, (y0 + y1) / 2.0),
                egui::PointerButton::Secondary,
            ),
        );
        let tree = accessible_frame(&ctx, &mut app, vec![]);
        // A menu item spans its menu, so its width is the menu's.
        let (x0, x1, _, _) = button(&tree, "Play");
        assert!(x1 - x0 <= 300.0, "the menu is {} points wide", x1 - x0);
        app.backend.shutdown();
    }

    /// Library cards and an artist's discography and related-artist cards
    /// answer a right click with their own menu, as Home's and Search's do.
    #[test]
    fn library_and_discography_cards_open_their_own_menus() {
        fn albums(app: &mut App, ui: &mut egui::Ui) {
            crate::ui::library::show(app, ui, Page::Albums);
        }
        fn artists(app: &mut App, ui: &mut egui::Ui) {
            crate::ui::library::show(app, ui, Page::Artists);
        }
        fn artist_page(app: &mut App, ui: &mut egui::Ui) {
            crate::ui::artist::show(app, ui, "art2");
        }
        let (ctx, mut app) = accessible_app("library-card-menus");
        let library_album = app.library.albums.items[0].album.name.clone();
        let library_artist = app.library.artists.items[0].name.clone();
        let page = &app.artist_pages["art2"];
        let discography = page.albums[DiscographyFilter::All.groups()].items[0]
            .name
            .clone();
        let related = page.related.get().unwrap()[0].name.clone();
        type View = fn(&mut App, &mut egui::Ui);
        let cases: [(&str, String, View, &str); 4] = [
            ("library album", library_album, albums, "Add to queue"),
            ("library artist", library_artist, artists, "Copy link"),
            ("discography", discography, artist_page, "Add to queue"),
            ("related artist", related, artist_page, "Copy link"),
        ];
        for (name, title, view, item) in cases {
            view_frame(&ctx, &mut app, vec![], view);
            let text = view_frame(&ctx, &mut app, vec![], view);
            let pos = text
                .iter()
                .rev()
                .find(|(text, _)| *text == title)
                .unwrap_or_else(|| panic!("{name}: {title} is not drawn"))
                .1
                .center();
            app.actions.clear();
            view_frame(
                &ctx,
                &mut app,
                pointer_click(pos, egui::PointerButton::Secondary),
                view,
            );
            let text = view_frame(&ctx, &mut app, vec![], view);
            assert!(
                app.actions.is_empty(),
                "right-clicking a {name} must not act"
            );
            assert!(
                text.iter().any(|(text, _)| text == item),
                "the {name} menu did not open"
            );
            // Close the menu before the next case.
            view_frame(
                &ctx,
                &mut app,
                vec![keyboard(egui::Key::Escape, egui::Modifiers::NONE)],
                view,
            );
        }
        app.backend.shutdown();
    }

    /// A song's menu offers its radio, and so do an album's, an artist's
    /// and a playlist's; each opens the radio's page and plays nothing.
    #[test]
    fn radio_menu_items_open_radio_pages() {
        fn playlist(app: &mut App, ui: &mut egui::Ui) {
            crate::ui::collection::playlist(app, ui, "pl1");
        }
        fn albums(app: &mut App, ui: &mut egui::Ui) {
            crate::ui::library::show(app, ui, Page::Albums);
        }
        fn artists(app: &mut App, ui: &mut egui::Ui) {
            crate::ui::library::show(app, ui, Page::Artists);
        }
        let (ctx, mut app) = accessible_app("radio-menus");
        let song = app.playlist_pages["pl1"].items.items[0]
            .playable()
            .unwrap()
            .clone();
        let album = app.library.albums.items[0].album.clone();
        let artist = app.library.artists.items[0].clone();
        let sidebar_playlist = PLAYLISTS[1].0.to_string();
        type View = fn(&mut App, &mut egui::Ui);
        let cases: [(String, View, &str, String); 4] = [
            (
                song.name().to_string(),
                playlist,
                "Go to song radio",
                song.uri().to_string(),
            ),
            (album.name, albums, "Go to album radio", album.uri),
            (artist.name, artists, "Go to artist radio", artist.uri),
            (
                sidebar_playlist,
                crate::ui::sidebar::show,
                "Go to playlist radio",
                playlist_uri("pl1"),
            ),
        ];
        for (title, view, item, seed) in cases {
            view_frame(&ctx, &mut app, vec![], view);
            let text = view_frame(&ctx, &mut app, vec![], view);
            let at = |text: &[(String, egui::Rect)], wanted: &str| {
                text.iter()
                    .rev()
                    .find(|(text, _)| text == wanted)
                    .unwrap_or_else(|| panic!("{wanted} is not drawn"))
                    .1
                    .center()
            };
            let pos = at(&text, &title);
            view_frame(
                &ctx,
                &mut app,
                pointer_click(pos, egui::PointerButton::Secondary),
                view,
            );
            let text = view_frame(&ctx, &mut app, vec![], view);
            app.actions.clear();
            view_frame(
                &ctx,
                &mut app,
                pointer_click(at(&text, item), egui::PointerButton::Primary),
                view,
            );
            assert!(
                matches!(app.actions.as_slice(), [Action::Open(Page::Radio(asked))] if *asked == seed),
                "{item}: {:?}",
                app.actions
            );
            app.actions.clear();
        }
        app.backend.shutdown();
    }

    /// The demo's radios draw as a server answers them: a song's and a
    /// playlist's with songs, and a bare server's, which has nothing to go
    /// with anything and says so instead of looking broken.
    #[test]
    fn a_radio_page_shows_its_songs_or_why_it_has_none() {
        fn song_radio(app: &mut App, ui: &mut egui::Ui) {
            crate::ui::radio::radio(app, ui, "sonic:track:trk0");
        }
        fn bare_radio(app: &mut App, ui: &mut egui::Ui) {
            crate::ui::radio::radio(app, ui, "sonic:artist:art0");
        }
        let (ctx, mut app) = accessible_app("radio-pages");
        let first = app.radio_pages["sonic:track:trk0"].songs.get().unwrap()[0]
            .name
            .clone();
        view_frame(&ctx, &mut app, vec![], song_radio);
        let text: Vec<String> = view_frame(&ctx, &mut app, vec![], song_radio)
            .into_iter()
            .map(|(text, _)| text)
            .collect();
        let title = format!("{} Radio", RECORDS[0].songs[0].0);
        for wanted in [title.as_str(), "Based on this song", first.as_str()] {
            assert!(text.iter().any(|text| text == wanted), "{wanted}: {text:?}");
        }
        assert!(!text.iter().any(|text| text == "No similar songs"));

        view_frame(&ctx, &mut app, vec![], bare_radio);
        let text: Vec<String> = view_frame(&ctx, &mut app, vec![], bare_radio)
            .into_iter()
            .map(|(text, _)| text)
            .collect();
        let title = format!("{} Radio", ARTISTS[0]);
        for wanted in [title.as_str(), "Based on this artist", "No similar songs"] {
            assert!(text.iter().any(|text| text == wanted), "{wanted}: {text:?}");
        }
        assert!(app.actions.is_empty(), "drawing asks for nothing");
        app.backend.shutdown();
    }

    #[test]
    fn search_top_results_open_their_item_menus() {
        for (kind, results, title) in [
            {
                let item = songs().0[0].clone();
                (
                    "track",
                    SearchResults {
                        tracks: Some(page(vec![item.clone()])),
                        ..Default::default()
                    },
                    item.name,
                )
            },
            {
                let item = artist(0);
                (
                    "artist",
                    SearchResults {
                        artists: Some(page(vec![item.clone()])),
                        ..Default::default()
                    },
                    item.name,
                )
            },
            {
                let item = album(0);
                (
                    "album",
                    SearchResults {
                        albums: Some(page(vec![item.clone()])),
                        ..Default::default()
                    },
                    item.name,
                )
            },
            {
                let all_songs = songs().0;
                let item = playlist(0, &all_songs);
                (
                    "playlist",
                    SearchResults {
                        playlists: Some(page(vec![item.clone()])),
                        ..Default::default()
                    },
                    item.name,
                )
            },
        ] {
            let (ctx, mut app) = accessible_app(&format!("top-result-{kind}"));
            app.search.results = Loadable::Loaded(results);
            view_frame(&ctx, &mut app, vec![], crate::ui::search::show);
            let text = view_frame(&ctx, &mut app, vec![], crate::ui::search::show);
            let pos = text
                .iter()
                .find(|(text, _)| text == &title)
                .expect("top result title")
                .1
                .center();
            app.actions.clear();
            view_frame(
                &ctx,
                &mut app,
                pointer_click(pos, egui::PointerButton::Secondary),
                crate::ui::search::show,
            );
            let text = view_frame(&ctx, &mut app, vec![], crate::ui::search::show);
            assert!(
                app.actions.is_empty(),
                "right-clicking {kind} must not play"
            );
            assert!(
                text.iter().any(|(text, _)| text == "Copy link"),
                "{kind} menu did not open"
            );
            app.backend.shutdown();
        }
    }

    #[test]
    fn the_cross_on_a_recent_search_forgets_only_that_query() {
        use egui::accesskit::{Action as AccessibleAction, Role};
        let (ctx, mut app) = accessible_app("search-history");
        app.open(Page::Search);
        // Recent searches stand in for results only while nothing is searched.
        app.search.query.clear();
        app.search.committed.clear();
        accessible_frame(&ctx, &mut app, vec![]);
        let tree = accessible_frame(&ctx, &mut app, vec![]);
        let forget = accessible_node(&tree, "Remove ambient", Role::Button);
        accessible_frame(
            &ctx,
            &mut app,
            vec![accessible_action(forget, AccessibleAction::Click, None)],
        );
        assert_eq!(app.settings.search_history, ["Khruangbin", "immunity"]);
        assert!(
            app.search.committed.is_empty(),
            "the cross must forget a query without running it"
        );
        app.backend.shutdown();
    }

    #[test]
    fn song_top_result_artist_name_opens_the_available_profile_or_the_album() {
        for artist_id in [Some("artist-ween"), None] {
            let (ctx, mut app) = accessible_app("top-result-song-artist");
            let mut item = songs().0[0].clone();
            let album_id = item.album.as_ref().unwrap().id.clone();
            item.artists = vec![ArtistRef {
                id: artist_id.map(str::to_string),
                name: "Ween".into(),
                uri: artist_id.map(artist_uri),
            }];
            app.search.results = Loadable::Loaded(SearchResults {
                tracks: Some(page(vec![item])),
                ..Default::default()
            });
            view_frame(&ctx, &mut app, vec![], crate::ui::search::show);
            let text = view_frame(&ctx, &mut app, vec![], crate::ui::search::show);
            let songs_left = text
                .iter()
                .filter(|(text, _)| text == "Songs")
                .max_by(|(_, left), (_, right)| left.left().total_cmp(&right.left()))
                .expect("songs heading")
                .1
                .left();
            let artist = text
                .iter()
                .find(|(text, rect)| text == "Ween" && rect.left() < songs_left)
                .expect("top result artist name");
            app.actions.clear();
            view_frame(
                &ctx,
                &mut app,
                pointer_click(
                    egui::pos2(artist.1.right() - 4.0, artist.1.center().y),
                    egui::PointerButton::Primary,
                ),
                crate::ui::search::show,
            );
            if artist_id.is_some() {
                assert!(
                    matches!(app.actions.as_slice(), [Action::Open(Page::Artist(id))] if id == "artist-ween")
                );
            } else {
                assert!(
                    matches!(app.actions.as_slice(), [Action::Open(Page::Album(id))] if id == &album_id)
                );
            }
            app.backend.shutdown();
        }
    }

    #[test]
    fn home_cards_open_item_menus() {
        let (ctx, mut app) = accessible_app("home-card-menu");
        let title = songs().0[1].name.clone();
        view_frame(&ctx, &mut app, vec![], crate::ui::home::show);
        let text = view_frame(&ctx, &mut app, vec![], crate::ui::home::show);
        let below = text
            .iter()
            .find(|(text, _)| text == "Recently played")
            .expect("recent section")
            .1
            .bottom();
        let pos = text
            .iter()
            .find(|(text, rect)| text == &title && rect.top() >= below)
            .expect("recent card")
            .1
            .center();
        app.actions.clear();
        view_frame(
            &ctx,
            &mut app,
            pointer_click(pos, egui::PointerButton::Secondary),
            crate::ui::home::show,
        );
        let text = view_frame(&ctx, &mut app, vec![], crate::ui::home::show);
        assert!(app.actions.is_empty(), "right-clicking must not play");
        assert!(text.iter().any(|(text, _)| text == "Add to queue"));
        app.backend.shutdown();
    }

    #[test]
    fn playlist_filter_preserves_edit_permissions_and_keyboard_selection() {
        use egui::accesskit::Role;
        let (ctx, mut app) = accessible_app("playlist-filter");
        let owner = app.user_id().expect("demo user").to_string();
        let make = |id: &str, name: &str, owned: bool, collaborative| Playlist {
            id: id.into(),
            uri: format!("sonic:playlist:{id}"),
            name: name.into(),
            owner: Owner {
                id: Some(if owned {
                    owner.clone()
                } else {
                    "another-user".into()
                }),
                ..Default::default()
            },
            collaborative,
            ..Default::default()
        };
        app.library.playlists = Loadable::Loaded(vec![
            make("readonly", "Night locked", false, false),
            make("owned", "Night drive", true, false),
            make("shared", "Night together", false, true),
            make("day", "Daylight", true, false),
        ]);
        let selected = vec![(track_uri("trk0"), "First song".into())];
        let mut query = String::new();
        let draw = |app: &mut App, query: &mut String, focus: bool, events| {
            let mut output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(760.0, 620.0),
                    )),
                    events,
                    ..Default::default()
                },
                |ui| {
                    let field = crate::ui::widgets::playlist_picker(ui, app, &selected, query);
                    if focus {
                        field.request_focus();
                    }
                },
            );
            output.textures_delta.clear();
            output
        };
        draw(&mut app, &mut query, true, vec![]);
        let output = draw(
            &mut app,
            &mut query,
            false,
            vec![egui::Event::Text("  NiGhT  ".into())],
        );
        assert_eq!(query, "  NiGhT  ");
        let tree = output.platform_output.accesskit_update.unwrap();
        for name in ["Night locked", "Daylight"] {
            assert!(
                !tree
                    .nodes
                    .iter()
                    .any(|(_, node)| node.label() == Some(name)),
                "{name} must not be offered"
            );
        }
        let owned = accessible_node(&tree, "Night drive", Role::Button);
        accessible_node(&tree, "Night together", Role::Button);
        let mut reached = false;
        for _ in 0..6 {
            let output = draw(
                &mut app,
                &mut query,
                false,
                vec![keyboard(egui::Key::Tab, egui::Modifiers::NONE)],
            );
            if output.platform_output.accesskit_update.unwrap().focus == owned {
                reached = true;
                break;
            }
        }
        assert!(reached, "Tab must reach the filtered playlist");
        app.actions.clear();
        draw(
            &mut app,
            &mut query,
            false,
            vec![keyboard(egui::Key::Enter, egui::Modifiers::NONE)],
        );
        assert!(
            matches!(app.actions.as_slice(), [Action::AddToPlaylist { playlist_id, uris, .. }] if playlist_id == "owned" && uris == &[track_uri("trk0")])
        );
        app.backend.shutdown();
    }

    #[test]
    fn playlist_filter_arrows_choose_a_match_before_enter() {
        let (ctx, mut app) = accessible_app("playlist-filter-arrows");
        let songs = [(track_uri("trk0"), "First song".to_string())];
        let mut query = String::new();
        let draw = |app: &mut App, query: &mut String, focus, events| {
            let mut output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(760.0, 620.0),
                    )),
                    events,
                    ..Default::default()
                },
                |ui| {
                    let field = crate::ui::widgets::playlist_picker(ui, app, &songs, query);
                    if focus {
                        field.request_focus();
                    }
                },
            );
            output.textures_delta.clear();
        };
        draw(&mut app, &mut query, true, vec![]);
        draw(
            &mut app,
            &mut query,
            false,
            vec![egui::Event::Text("night".into())],
        );
        draw(&mut app, &mut query, false, vec![]);
        draw(
            &mut app,
            &mut query,
            false,
            vec![keyboard(egui::Key::ArrowDown, egui::Modifiers::NONE)],
        );
        draw(
            &mut app,
            &mut query,
            false,
            vec![keyboard(egui::Key::Enter, egui::Modifiers::NONE)],
        );
        assert_eq!(query, "night");
        assert!(
            matches!(app.actions.as_slice(), [Action::AddToPlaylist { playlist_name, .. }] if playlist_name == "Berlin nights"),
            "{:?}",
            app.actions
        );
        app.backend.shutdown();
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
        app.toast("Wish You Were Here added to queue");
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

    /// Rule: the interface zoom control puts minus on the left and plus
    /// on the right. The setting row's control is right-to-left, which
    /// used to reverse the two buttons.
    #[test]
    fn interface_zoom_puts_minus_on_the_left() {
        let root =
            std::env::temp_dir().join(format!("fastsonic-zoom-order-test-{}", std::process::id()));
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
        app.settings.zoom = 1.0;
        app.open(Page::Settings);

        let mut placed: Vec<(String, f32, f32)> = Vec::new();
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1280.0, 4000.0),
            )),
            ..Default::default()
        };
        for _ in 0..2 {
            placed.clear();
            let mut output = ctx.run_ui(input.clone(), |ui| app.frame_ui(ui));
            output.textures_delta.clear();
            fn walk(shape: &egui::epaint::Shape, placed: &mut Vec<(String, f32, f32)>) {
                match shape {
                    egui::epaint::Shape::Text(text) => {
                        placed.push((text.galley.job.text.clone(), text.pos.x, text.pos.y))
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
        let percent = placed
            .iter()
            .find(|(text, _, _)| text == "100%")
            .unwrap_or_else(|| panic!("the zoom percent was never drawn: {placed:?}"));
        let on_row = |label: &str| -> f32 {
            placed
                .iter()
                .filter(|(text, _, y)| text == label && (y - percent.2).abs() < 8.0)
                .min_by(|a, b| (a.1 - percent.1).abs().total_cmp(&(b.1 - percent.1).abs()))
                .unwrap_or_else(|| panic!("{label} was never drawn next to 100%: {placed:?}"))
                .1
        };
        let minus = on_row("-");
        let plus = on_row("+");
        assert!(
            minus < percent.1 && percent.1 < plus,
            "zoom control should read minus, percent, plus; got - at {minus}, 100% at {}, + at {plus}",
            percent.1
        );
        app.backend.shutdown();
        let _ = std::fs::remove_dir_all(root);
    }

    /// Linux offers middle-click autoscroll as a switch that starts off and
    /// is saved; Windows always autoscrolls and macOS never does, so neither
    /// shows the row.
    #[test]
    fn the_linux_autoscroll_switch_starts_off_and_is_saved() {
        use egui::accesskit::{Role, Toggled};
        let (ctx, mut app) = accessible_app("autoscroll-setting");
        app.open(Page::Settings);
        let draw = |app: &mut App, events| {
            let mut output = ctx.run_ui(
                egui::RawInput {
                    // Tall enough to lay out the whole Appearance section.
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(1280.0, 4000.0),
                    )),
                    events,
                    ..Default::default()
                },
                |ui| app.frame_ui(ui),
            );
            output.textures_delta.clear();
            output
                .platform_output
                .accesskit_update
                .expect("screen-reader tree")
        };
        let switch = |tree: &egui::accesskit::TreeUpdate, label: &str| {
            tree.nodes
                .iter()
                .find(|(_, node)| node.label() == Some(label) && node.role() == Role::CheckBox)
                .map(|(id, node)| (*id, node.toggled()))
        };
        draw(&mut app, vec![]);
        let tree = draw(&mut app, vec![]);
        assert!(
            switch(&tree, "Compact track list").is_some(),
            "the Appearance section is drawn"
        );
        let control = switch(&tree, "Middle-click autoscroll");
        assert_eq!(control.is_some(), cfg!(target_os = "linux"));
        let Some((control, toggled)) = control else {
            app.backend.shutdown();
            return;
        };
        assert_eq!(toggled, Some(Toggled::False));
        assert!(!app.settings.middle_click_autoscroll);
        draw(
            &mut app,
            vec![accessible_action(
                control,
                egui::accesskit::Action::Click,
                None,
            )],
        );
        assert!(app.settings.middle_click_autoscroll);
        let path = app.dirs.config.join("autoscroll-choice.json");
        app.settings.save(&path);
        app.settings = Settings::load(&path);
        assert!(app.settings.middle_click_autoscroll);
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

    /// Rule: side-panel headers stay on one line at their narrowest width.
    #[test]
    fn the_narrowest_panels_keep_their_headers_on_one_row() {
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
        app.settings.queue_width = crate::theme::SIDE_PANEL_MIN_WIDTH;
        app.settings.lyrics_width = crate::theme::SIDE_PANEL_MIN_WIDTH;
        app.lyrics = Loadable::Loaded(Some(sample_lyrics()));
        app.lyrics_following = false;

        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1280.0, 800.0),
            )),
            ..Default::default()
        };
        let drawn = |app: &mut App| {
            let mut placed = Vec::new();
            // A panel applies its requested width after the first frame.
            for _ in 0..2 {
                placed.clear();
                let mut output = ctx.run_ui(input.clone(), |ui| app.frame_ui(ui));
                output.textures_delta.clear();
                fn walk(shape: &egui::epaint::Shape, placed: &mut Vec<(String, egui::Rect)>) {
                    match shape {
                        egui::epaint::Shape::Text(text) => {
                            placed.push((text.galley.job.text.clone(), text.visual_bounding_rect()))
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
            placed
        };
        let assert_same_row = |placed: &[(String, egui::Rect)], left: &str, right: &str| {
            let at = |label: &str| {
                placed
                    .iter()
                    .find(|(text, _)| text == label)
                    .unwrap_or_else(|| panic!("{label} was never drawn: {placed:?}"))
                    .1
            };
            let (left_rect, right_rect) = (at(left), at(right));
            assert!(
                (left_rect.center().y - right_rect.center().y).abs() < 10.0
                    && (left_rect.right() <= right_rect.left()
                        || right_rect.right() <= left_rect.left()),
                "{left} and {right} should share a clear row at minimum width: {left_rect:?} vs {right_rect:?}"
            );
        };

        for (queue, lyrics) in [(false, false), (true, false), (false, true), (true, true)] {
            app.show_queue_panel = queue;
            app.show_lyrics_panel = lyrics;
            let placed = drawn(&mut app);
            if queue {
                assert_same_row(&placed, "Queue", "Recent");
            }
            if lyrics {
                assert_same_row(&placed, "Lyrics", "Follow");
            }
        }
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
            Page::Radio(track_uri("trk0")),
            Page::Radio(playlist_uri("pl1")),
            // A bare server's: nothing goes with anything.
            Page::Radio(artist_uri("art0")),
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
        app.settings.player_bar_vis = crate::settings::PlayerBarVis::Spectrum;
        frame(&ctx, &mut app);
        app.settings.player_bar_vis = crate::settings::PlayerBarVis::Waveform;
        frame(&ctx, &mut app);
        app.lyrics = Loadable::Loaded(Some(sample_lyrics()));
        app.lyrics_fullscreen = Some(false);
        frame(&ctx, &mut app);
        app.lyrics = Loadable::Loaded(None);
        frame(&ctx, &mut app);
        app.backend.shutdown();
        let _ = std::fs::remove_dir_all(root);
    }

    fn sidebar_text(painted: &[(String, egui::Rect)], text: &str) -> egui::Rect {
        painted
            .iter()
            .find(|(painted, _)| painted == text)
            .map(|(_, rect)| *rect)
            .unwrap_or_else(|| panic!("{text:?} is not on screen"))
    }

    fn played_contexts(app: &App) -> Vec<String> {
        app.actions
            .iter()
            .filter_map(|action| match action {
                Action::PlayContext { uri, .. } => Some(uri.clone()),
                _ => None,
            })
            .collect()
    }

    /// Double-clicking a playable Library row plays its context, in both the
    /// normal and compact sidebar modes. The first click still opens the
    /// page, and a single click only does that.
    #[test]
    fn double_clicking_a_sidebar_row_plays_its_context() {
        for compact in [false, true] {
            let (ctx, mut app) = accessible_app(&format!("sidebar-double-click-{compact}"));
            app.settings.sidebar_compact = compact;
            let view = crate::ui::sidebar::show;
            view_frame(&ctx, &mut app, vec![], view);
            let painted = view_frame(&ctx, &mut app, vec![], view);
            let name = painted
                .iter()
                .find(|(text, _)| text == "Sunday morning")
                .map(|(_, rect)| rect.center())
                .expect("the playlist in the sidebar");
            app.actions.clear();
            view_frame(
                &ctx,
                &mut app,
                pointer_click(name, egui::PointerButton::Primary),
                view,
            );
            assert!(played_contexts(&app).is_empty(), "one click only opens");
            view_frame(
                &ctx,
                &mut app,
                pointer_click(name, egui::PointerButton::Primary),
                view,
            );
            assert_eq!(played_contexts(&app), ["sonic:playlist:pl2"]);
            assert!(
                app.actions.iter().any(
                    |action| matches!(action, Action::Open(Page::Playlist(id)) if id == "pl2")
                ),
                "double click must still open the page"
            );
            app.backend.shutdown();
        }
    }

    /// Liked Songs has no URI of its own, so its row lights from the context
    /// it plays, the starred songs, as a playlist's row does from its own.
    #[test]
    fn the_liked_songs_row_lights_while_its_songs_play() {
        let (ctx, mut app) = accessible_app("liked-songs-playing");
        let lit = |ctx: &egui::Context, app: &mut App, name: &str| {
            let mut output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(1280.0, 800.0),
                    )),
                    ..Default::default()
                },
                |ui| crate::ui::sidebar::show(app, ui),
            );
            output.textures_delta.clear();
            fn walk(shape: &egui::epaint::Shape, name: &str, found: &mut Vec<egui::Color32>) {
                match shape {
                    egui::epaint::Shape::Text(text) if text.galley.job.text == name => {
                        found.extend(text.galley.job.sections.iter().map(|s| s.format.color));
                    }
                    egui::epaint::Shape::Vec(shapes) => {
                        shapes.iter().for_each(|shape| walk(shape, name, found));
                    }
                    _ => {}
                }
            }
            let mut colors = Vec::new();
            for clipped in &output.shapes {
                walk(&clipped.shape, name, &mut colors);
            }
            assert!(!colors.is_empty(), "{name:?} is not in the sidebar");
            colors.iter().all(|color| *color == app.palette.accent)
        };
        accessible_frame(&ctx, &mut app, vec![]);
        assert!(!lit(&ctx, &mut app, "Liked Songs"));
        app.actions.push(Action::PlayContext {
            uri: crate::api::subsonic::convert::COLLECTION_URI.into(),
            offset_uri: None,
            offset_index: None,
        });
        accessible_frame(&ctx, &mut app, vec![]);
        assert!(app.believed_playing());
        assert!(lit(&ctx, &mut app, "Liked Songs"));
        assert!(!lit(&ctx, &mut app, "Sunday morning"));
        app.backend.shutdown();
    }

    /// The Library's order menu works from the keyboard and a screen reader,
    /// offers only the orders the section has data for, and switching away
    /// from the custom order and back finds the arrangement where it was.
    #[test]
    fn library_sort_menu_keeps_the_custom_order_and_works_from_the_keyboard() {
        use crate::settings::{LibraryShelf, LibrarySort};
        use egui::accesskit::{Action as AccessibleAction, Role};
        let (ctx, mut app) = accessible_app("library-sort");
        app.settings.sidebar_order = vec![playlist_uri("pl4"), playlist_uri("pl1")];
        let saved = app.settings.sidebar_order.clone();
        accessible_frame(&ctx, &mut app, vec![]);
        let tree = accessible_frame(&ctx, &mut app, vec![]);
        let sort = accessible_node(&tree, "Custom order", Role::Button);
        accessible_frame(
            &ctx,
            &mut app,
            vec![accessible_action(sort, AccessibleAction::Click, None)],
        );
        let tree = accessible_frame(&ctx, &mut app, vec![]);
        let labels: Vec<_> = tree
            .nodes
            .iter()
            .filter_map(|(_, node)| node.label())
            .collect();
        assert!(labels.contains(&"Creator"));
        for absent in ["Recently added", "Library order", "Artist"] {
            assert!(!labels.contains(&absent), "playlists offered {absent}");
        }
        let name = accessible_node(&tree, "Name", Role::Button);
        accessible_frame(
            &ctx,
            &mut app,
            vec![accessible_action(name, AccessibleAction::Focus, None)],
        );
        accessible_frame(
            &ctx,
            &mut app,
            vec![keyboard(egui::Key::Enter, egui::Modifiers::NONE)],
        );
        assert_eq!(
            app.settings.library_sort.get(&LibraryShelf::Playlists),
            Some(&LibrarySort::Name)
        );
        assert_eq!(app.settings.sidebar_order, saved);
        let tree = accessible_frame(&ctx, &mut app, vec![]);
        let sort = accessible_node(&tree, "Name", Role::Button);
        accessible_frame(
            &ctx,
            &mut app,
            vec![accessible_action(sort, AccessibleAction::Click, None)],
        );
        let tree = accessible_frame(&ctx, &mut app, vec![]);
        let custom = accessible_node(&tree, "Custom order", Role::Button);
        accessible_frame(
            &ctx,
            &mut app,
            vec![accessible_action(custom, AccessibleAction::Click, None)],
        );
        assert_eq!(
            app.settings.library_sort.get(&LibraryShelf::Playlists),
            Some(&LibrarySort::Local)
        );
        assert_eq!(app.settings.sidebar_order, saved);
        let path = app.dirs.config.join("library-sort.json");
        app.settings.save(&path);
        let restored = Settings::load(&path);
        assert_eq!(restored.library_sort, app.settings.library_sort);
        assert_eq!(restored.sidebar_order, saved);
        app.backend.shutdown();
    }

    /// Albums offer the star date and sort by artist; the order on screen
    /// follows the choice.
    #[test]
    fn albums_sort_by_star_date_and_by_artist() {
        use crate::settings::{LibraryShelf, LibrarySort};
        let (ctx, mut app) = accessible_app("library-sort-albums");
        let view = crate::ui::sidebar::show;
        view_frame(&ctx, &mut app, vec![], view);
        let painted = view_frame(&ctx, &mut app, vec![], view);
        let albums = sidebar_text(&painted, "Albums").center();
        view_frame(
            &ctx,
            &mut app,
            pointer_click(albums, egui::PointerButton::Primary),
            view,
        );
        let names = |app: &App| -> Vec<String> {
            app.library
                .albums
                .items
                .iter()
                .map(|saved| saved.album.name.clone())
                .collect()
        };
        let order = |painted: &[(String, egui::Rect)], names: &[String]| {
            let mut shown: Vec<_> = painted
                .iter()
                .filter(|(text, _)| names.contains(text))
                .map(|(text, rect)| (rect.top(), text.clone()))
                .collect();
            shown.sort_by(|a, b| a.0.total_cmp(&b.0));
            shown.dedup_by(|a, b| a.1 == b.1);
            shown.into_iter().map(|(_, text)| text).collect::<Vec<_>>()
        };
        let listed = names(&app);
        assert!(listed.len() > 2);
        let painted = view_frame(&ctx, &mut app, vec![], view);
        assert_eq!(order(&painted, &listed), listed, "the server's order");

        app.settings
            .library_sort
            .insert(LibraryShelf::Albums, LibrarySort::RecentlyAdded);
        let painted = view_frame(&ctx, &mut app, vec![], view);
        let newest: Vec<_> = listed.iter().rev().cloned().collect();
        assert_eq!(order(&painted, &listed), newest, "the newest star first");
        sidebar_text(&painted, "Recently added");

        app.settings
            .library_sort
            .insert(LibraryShelf::Albums, LibrarySort::Creator);
        let painted = view_frame(&ctx, &mut app, vec![], view);
        sidebar_text(&painted, "Artist");
        let mut by_artist: Vec<_> = app
            .library
            .albums
            .items
            .iter()
            .map(|saved| {
                (
                    crate::api::models::join_names(
                        saved
                            .album
                            .artists
                            .iter()
                            .map(|artist| artist.name.as_str()),
                    )
                    .to_lowercase(),
                    saved.album.name.to_lowercase(),
                    saved.album.name.clone(),
                )
            })
            .collect();
        by_artist.sort();
        let by_artist: Vec<_> = by_artist.into_iter().map(|(_, _, name)| name).collect();
        assert_eq!(order(&painted, &listed), by_artist);
        app.backend.shutdown();
    }

    /// Any order but the server's needs the whole section, so it pages on;
    /// a page that failed is not asked for again every frame.
    #[test]
    fn library_sorts_finish_paging_without_retrying_failed_pages() {
        use crate::settings::{LibraryShelf, LibrarySort};
        for (shelf, label, page) in [
            (LibraryShelf::Albums, "Albums", Page::Albums),
            (LibraryShelf::Artists, "Artists", Page::Artists),
        ] {
            let (ctx, mut app) = accessible_app(&format!("library-sort-paging-{shelf:?}"));
            let view = crate::ui::sidebar::show;
            app.settings.library_sort.insert(shelf, LibrarySort::Name);
            app.library.albums.next_offset = Some(50);
            app.library.artists.complete = false;
            app.library.artists.after = Some("50".into());
            view_frame(&ctx, &mut app, vec![], view);
            let painted = view_frame(&ctx, &mut app, vec![], view);
            let chip = sidebar_text(&painted, label).center();
            view_frame(
                &ctx,
                &mut app,
                pointer_click(chip, egui::PointerButton::Primary),
                view,
            );
            app.actions.clear();
            view_frame(&ctx, &mut app, vec![], view);
            assert!(
                app.actions
                    .iter()
                    .any(|action| matches!(action, Action::LoadMore(found) if *found == page))
            );
            app.actions.clear();
            app.library.albums.error = Some("Try again later".into());
            app.library.artists.error = Some("Try again later".into());
            for _ in 0..3 {
                view_frame(&ctx, &mut app, vec![], view);
                assert!(
                    !app.actions
                        .iter()
                        .any(|action| matches!(action, Action::LoadMore(found) if *found == page)),
                    "failed pages must not retry every frame"
                );
            }
            app.backend.shutdown();
        }
    }

    /// Holding a dragged song at the edge of a playlist scrolls it, so a song
    /// can be moved to a row beyond the viewport; leaving the edge stops.
    #[test]
    fn dragging_at_playlist_edges_reaches_rows_beyond_the_viewport() {
        for upwards in [false, true] {
            let (ctx, mut app) = accessible_app(&format!("drag-scroll-{upwards}"));
            app.open(Page::Playlist("pl1".into()));
            let list = &mut app.playlist_pages.get_mut("pl1").unwrap().items;
            // Long enough to scroll.
            let songs = list.items.len();
            while list.items.len() < 40 {
                let again = list.items[list.items.len() % songs].clone();
                list.items.push(again);
            }
            list.revision += 1;
            let items = &mut list.items;
            let count = items.len();
            for (index, name) in [(0, "First song"), (count - 1, "Last song")] {
                if let Some(PlayableItem::Track(track)) = &mut items[index].item {
                    track.name = name.into();
                }
            }
            let mut time = 0.0;
            let mut draw = |app: &mut App, events, offset: Option<f32>| {
                time += 1.0 / 60.0;
                let mut result = None;
                let mut output = ctx.run_ui(
                    egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(
                            egui::Pos2::ZERO,
                            egui::vec2(760.0, 620.0),
                        )),
                        time: Some(time),
                        events,
                        ..Default::default()
                    },
                    |ui| {
                        let mut scroll = egui::ScrollArea::vertical()
                            .id_salt("drag-scroll-playlist")
                            .auto_shrink([false, false]);
                        if let Some(offset) = offset {
                            scroll = scroll.vertical_scroll_offset(offset);
                        }
                        let shown =
                            scroll.show(ui, |ui| crate::ui::collection::playlist(app, ui, "pl1"));
                        result = Some((shown.state.offset.y, shown.inner_rect));
                    },
                );
                output.textures_delta.clear();
                let (offset, viewport) = result.unwrap();
                (
                    offset,
                    viewport,
                    output.platform_output.accesskit_update.unwrap(),
                )
            };
            let row = |tree: &egui::accesskit::TreeUpdate, name: &str| {
                let prefix = format!("Play {name},");
                let bounds = tree
                    .nodes
                    .iter()
                    .find(|(_, node)| {
                        node.role() == egui::accesskit::Role::Button
                            && node.label().is_some_and(|label| label.starts_with(&prefix))
                    })
                    .unwrap_or_else(|| panic!("missing {name}"))
                    .1
                    .bounds()
                    .unwrap();
                egui::Rect::from_min_max(
                    egui::pos2(bounds.x0 as f32, bounds.y0 as f32),
                    egui::pos2(bounds.x1 as f32, bounds.y1 as f32),
                )
            };
            draw(
                &mut app,
                vec![],
                Some(if upwards { 100_000.0 } else { 0.0 }),
            );
            let (start, viewport, tree) = draw(&mut app, vec![], None);
            let source = row(&tree, if upwards { "Last song" } else { "First song" });
            let pos = egui::pos2(source.left() + 160.0, source.top() + 18.0);
            assert!(viewport.contains(pos));
            draw(
                &mut app,
                vec![
                    egui::Event::PointerMoved(pos),
                    egui::Event::PointerButton {
                        pos,
                        button: egui::PointerButton::Primary,
                        pressed: true,
                        modifiers: egui::Modifiers::NONE,
                    },
                ],
                None,
            );
            draw(
                &mut app,
                vec![egui::Event::PointerMoved(pos + egui::vec2(12.0, 0.0))],
                None,
            );
            assert!(
                egui::DragAndDrop::has_payload_of_type::<DragTrack>(&ctx),
                "the row must start a real drag"
            );
            let edge = egui::pos2(
                pos.x,
                if upwards {
                    viewport.top() + 2.0
                } else {
                    viewport.bottom() - 2.0
                },
            );
            draw(&mut app, vec![egui::Event::PointerMoved(edge)], None);
            for _ in 0..30 {
                draw(&mut app, vec![], None);
            }
            let (moved, _, _) = draw(&mut app, vec![], None);
            assert!(
                if upwards {
                    start - moved > 200.0
                } else {
                    moved - start > 200.0
                },
                "holding a stationary pointer at an edge must scroll ({start} to {moved})"
            );
            draw(
                &mut app,
                vec![egui::Event::PointerMoved(viewport.center())],
                None,
            );
            let (paused, _, _) = draw(&mut app, vec![], None);
            for _ in 0..10 {
                draw(&mut app, vec![], None);
            }
            let (still, _, _) = draw(&mut app, vec![], None);
            assert_eq!(paused, still, "leaving the edge stops scrolling");
            app.backend.shutdown();
        }
    }

    #[test]
    fn compact_track_rows_leave_a_gap_before_the_added_date_separator() {
        fn rows(app: &mut App, ui: &mut egui::Ui) {
            use crate::model::RowContext;
            use crate::ui::widgets::{TrackRow, track_row};
            ui.set_max_width(520.0);
            for count in 1..=2 {
                let song = Track {
                    id: Some(format!("t{count}")),
                    name: format!("Song {count}"),
                    uri: format!("sonic:track:t{count}"),
                    duration_ms: 200_000,
                    artists: (0..count)
                        .map(|index| ArtistRef {
                            id: Some(format!("artist-{index}")),
                            name: format!("Artist {index}"),
                            uri: Some(format!("sonic:artist:artist-{index}")),
                        })
                        .collect(),
                    ..Track::default()
                };
                let item = PlayableItem::Track(song);
                let context = RowContext::Uris(std::sync::Arc::from([item.uri().to_owned()]));
                track_row(
                    ui,
                    app,
                    TrackRow {
                        index: count,
                        number: Some(count),
                        item: &item,
                        context: &context,
                        show_cover: false,
                        show_album: false,
                        added_at: Some("2026-01-01T00:00:00Z"),
                        added_by: None,
                        show_added_by: false,
                        compact: false,
                        thin: true,
                        shift: 0.0,
                        picked: false,
                        picked_songs: &[],
                    },
                );
            }
        }
        let (ctx, mut app) = accessible_app("compact-artist-date-gap");
        for palette in [
            crate::theme::Palette::dark(),
            crate::theme::Palette::light(),
        ] {
            app.palette = palette;
            crate::theme::apply(&ctx, &palette);
            view_frame(&ctx, &mut app, vec![], rows);
            let text = view_frame(&ctx, &mut app, vec![], rows);
            let mut separated = 0;
            for (label, artist) in text
                .iter()
                .filter(|(label, _)| label.starts_with("Artist "))
            {
                let separator = text.iter().find(|(label, rect)| {
                    label == "•"
                        && (rect.center().y - artist.center().y).abs() < 3.0
                        && rect.left() >= artist.right() - 0.1
                });
                // Only the last artist in each row borders the date separator.
                if let Some((_, separator)) = separator {
                    separated += 1;
                    let gap = separator.left() - artist.right();
                    assert!(
                        gap >= 5.9,
                        "{label} needs a gap before the date bullet, got {gap}"
                    );
                }
            }
            assert!(separated >= 2, "each row's last artist borders the date");
        }
        app.backend.shutdown();
    }

    /// `--demo-show rtl` fills the playlist with right-to-left titles.
    #[cfg(feature = "demo")]
    #[test]
    fn the_rtl_demo_shows_right_to_left_titles() {
        fn playlist(app: &mut App, ui: &mut egui::Ui) {
            crate::ui::collection::playlist(app, ui, "pl1");
        }
        let (ctx, mut app) = accessible_app("rtl-titles");
        apply_flags(&mut app, Some("playlist:pl1"), Some("rtl"));
        view_frame(&ctx, &mut app, vec![], playlist);
        let painted = view_frame(&ctx, &mut app, vec![], playlist);
        for (title, artist, _) in RTL_TRACKS {
            let title = crate::bidi::display_text(title);
            let artist = crate::bidi::display_text(artist);
            assert!(
                painted.iter().any(|(text, _)| *text == title),
                "{title} is not drawn"
            );
            assert!(
                painted
                    .iter()
                    .any(|(text, _)| text.contains(artist.as_ref())),
                "{artist} is not drawn"
            );
        }
        app.backend.shutdown();
    }

    /// `--demo-show signed-out` and `connecting` draw the sign-in card.
    #[cfg(feature = "demo")]
    #[test]
    fn the_sign_in_card_shows_in_demo_mode() {
        fn whole(app: &mut App, ui: &mut egui::Ui) {
            app.frame_ui(ui);
        }
        let (ctx, mut app) = accessible_app("sign-in-card");
        apply_flags(&mut app, None, Some("signed-out"));
        view_frame(&ctx, &mut app, vec![], whole);
        let painted = view_frame(&ctx, &mut app, vec![], whole);
        assert!(painted.iter().any(|(text, _)| text == "Connect"));
        let tree = accessible_frame(&ctx, &mut app, vec![]);
        accessible_node(&tree, "Connect", egui::accesskit::Role::Button);
        apply_flags(&mut app, None, Some("connecting"));
        assert!(matches!(app.auth, AuthStatus::Connecting) && app.user.is_none());
        view_frame(&ctx, &mut app, vec![], whole);
        app.backend.shutdown();
    }

    /// In a wide window the cover moves aside only for words to read. While
    /// lyrics load, and when they fail or turn out to be missing, it stays
    /// centred with the reason under it, so a song without words never
    /// moves.
    #[test]
    fn full_screen_lyrics_keep_the_cover_centred_until_there_are_words() {
        let (ctx, mut app) = accessible_app("lyrics-cover-states");
        app.lyrics_fullscreen = Some(false);
        let title = app.now_playing().expect("the demo plays a song").title;
        let draw = |app: &mut App| {
            view_frame(&ctx, app, Vec::new(), crate::ui::lyrics::fullscreen);
            view_frame(&ctx, app, Vec::new(), crate::ui::lyrics::fullscreen)
        };
        let centred = |text: &[(String, egui::Rect)]| {
            text.iter()
                .filter(|(shown, _)| *shown == title)
                .any(|(_, rect)| (rect.center().x - 640.0).abs() < 2.0)
        };
        for (state, says) in [
            (Loadable::Loading, "Loading…"),
            (
                Loadable::Failed("boom".into()),
                "Couldn't fetch the lyrics: boom",
            ),
            (Loadable::Loaded(None), "No lyrics"),
        ] {
            app.lyrics = state;
            let text = draw(&mut app);
            assert!(centred(&text), "{says}: the cover stays in the middle");
            assert!(text.iter().any(|(shown, _)| shown == says), "{says}");
            assert_eq!(
                text.iter().any(|(shown, _)| shown == "Try again"),
                says.starts_with("Couldn't"),
                "{says}: only a failure offers to try again"
            );
        }
        app.lyrics = Loadable::Loaded(Some(sample_lyrics()));
        assert!(!centred(&draw(&mut app)), "words move the cover aside");
        app.backend.shutdown();
    }

    /// Virtual queue rows and library cards still draw a long list, and the
    /// library still asks for the next page when the end is near.
    #[test]
    fn a_long_virtual_queue_and_library_still_draw() {
        let root =
            std::env::temp_dir().join(format!("fastsonic-virtual-long-{}", std::process::id()));
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
        let seed = app.queue.queue.clone();
        app.queue.queue = seed.iter().cloned().cycle().take(200).collect();
        app.manual_queue = app
            .queue
            .queue
            .iter()
            .take(80)
            .map(|item| item.uri().to_string())
            .collect();
        app.show_queue_panel = true;
        for _ in 0..3 {
            frame(&ctx, &mut app);
        }
        app.library.albums.items = (0..80)
            .map(|index| SavedAlbum {
                added_at: None,
                album: album(index % 8),
            })
            .collect();
        app.library.albums.next_offset = Some(80);
        app.library.albums.loaded_once = true;
        app.open(Page::Albums);
        app.actions.clear();
        for _ in 0..3 {
            frame(&ctx, &mut app);
        }
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
                DragTrack::song(&song(&track_uri("trk0"), "Rosewood"), None),
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

    /// The cover and title in the player bar are a song source, not just
    /// links, so the sidebar receives the playing song the same way it
    /// receives a dragged table row.
    #[test]
    fn dragging_the_now_playing_song_supplies_a_playlist_row() {
        let (ctx, mut app) = accessible_app("now-playing-drag");
        for _ in 0..3 {
            frame(&ctx, &mut app);
        }
        let playing = app.now_playing().expect("the demo plays a song").uri;
        // The bar's height follows the fonts, so feel for the cover rather
        // than hardcode where it sits.
        let payload = (0..12).find_map(|step| {
            let start = egui::pos2(40.0, 800.0 - 10.0 - step as f32 * 6.0);
            frame_events(
                &ctx,
                &mut app,
                vec![
                    egui::Event::PointerMoved(start),
                    egui::Event::PointerButton {
                        pos: start,
                        button: egui::PointerButton::Primary,
                        pressed: true,
                        modifiers: egui::Modifiers::NONE,
                    },
                ],
            );
            frame_events(
                &ctx,
                &mut app,
                vec![egui::Event::PointerMoved(start + egui::vec2(20.0, -10.0))],
            );
            let payload = egui::DragAndDrop::payload::<DragTrack>(&ctx);
            egui::DragAndDrop::clear_payload(&ctx);
            frame_events(
                &ctx,
                &mut app,
                vec![egui::Event::PointerButton {
                    pos: start,
                    button: egui::PointerButton::Primary,
                    pressed: false,
                    modifiers: egui::Modifiers::NONE,
                }],
            );
            payload
        });
        let payload = payload.expect("dragging the player bar's song should carry it");
        assert_eq!(payload.uris(), vec![playing]);
        assert_eq!(payload.from, None, "this is an add, not a playlist move");
        app.backend.shutdown();
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
    /// is an insert-before position, which the handler mirrors locally
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
        let held = |from: usize, uri: &str| {
            DragTrack::song(&song(uri, "Elysian"), Some(("pl1".into(), from as u32)))
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

    /// A song from anywhere else dropped between an editable playlist's
    /// rows is copied in at that slot, at once; the playlist's own rows
    /// still move instead, and a sorted view takes nothing.
    #[test]
    fn dropping_a_song_into_an_open_playlist_inserts_it_at_the_slot() {
        let (ctx, mut app) = accessible_app("insert-drop");
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
        let total = app.playlist_pages["pl1"].items.total;
        let stranger = song(&track_uri("from-elsewhere"), "Stranger");
        let drop_at = |ctx: &egui::Context, app: &mut App, pos: egui::Pos2, payload: DragTrack| {
            egui::DragAndDrop::set_payload(ctx, payload);
            frame_events(ctx, app, vec![egui::Event::PointerMoved(pos)]);
            frame_events(
                ctx,
                app,
                vec![egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed: false,
                    modifiers: egui::Modifiers::NONE,
                }],
            );
            egui::DragAndDrop::clear_payload(ctx);
        };
        // Where the table begins depends on the loaded fonts; sweep down
        // until a drop lands.
        let landed = (0..45)
            .map(|step| egui::pos2(700.0, 120.0 + step as f32 * 15.0))
            .find(|pos| {
                drop_at(&ctx, &mut app, *pos, DragTrack::song(&stranger, None));
                app.playlist_busy
            })
            .expect("no sweep position landed inside the table");
        let now = order(&app);
        let at = now
            .iter()
            .position(|uri| *uri == track_uri("from-elsewhere"))
            .expect("the dropped song shows at once");
        let mut expected = original.clone();
        expected.insert(at, track_uri("from-elsewhere"));
        assert_eq!(now, expected, "the other rows keep their order");
        assert_eq!(
            app.playlist_pages["pl1"].items.total,
            total.map(|total| total + 1)
        );

        app.table_sorts.insert(
            Page::Playlist("pl1".into()),
            TableSort {
                column: SortColumn::Title,
                ascending: true,
            },
        );
        app.playlist_busy = false;
        frame(&ctx, &mut app);
        drop_at(&ctx, &mut app, landed, DragTrack::song(&stranger, None));
        assert!(
            !app.playlist_busy,
            "a sorted view accepted a positioned add"
        );
        assert_eq!(order(&app), expected);
        app.backend.shutdown();
    }

    /// Picking rows with Cmd/Ctrl-click and dragging one of them carries
    /// the whole selection in the order shown, however it was picked, and
    /// a selection is copied rather than moved.
    #[test]
    fn dragging_a_picked_row_carries_every_picked_song_in_table_order() {
        use egui::accesskit::Role;
        let (ctx, mut app) = accessible_app("drag-selection");
        app.open(Page::Playlist("pl1".into()));
        accessible_frame(&ctx, &mut app, vec![]);
        let tree = accessible_frame(&ctx, &mut app, vec![]);
        let mut rows: Vec<(String, egui::Pos2)> =
            tree.nodes
                .iter()
                .filter(|(_, node)| node.role() == Role::Button)
                .filter_map(|(_, node)| {
                    let label = node.label()?;
                    let bounds = node.bounds()?;
                    (label.starts_with("Play ") && label.contains(',') && bounds.width() > 400.0)
                        .then(|| {
                            (
                                label.to_string(),
                                egui::pos2(bounds.x0 as f32 + 200.0, bounds.y0 as f32 + 10.0),
                            )
                        })
                })
                .filter(|(_, pos)| pos.y > 100.0 && pos.y < 650.0)
                .collect();
        rows.sort_by(|a, b| a.1.y.total_cmp(&b.1.y));
        assert!(rows.len() >= 3, "the playlist shows its rows");
        // egui takes held modifiers from ModifiersChanged, not the click,
        // and holds the last one for the whole frame.
        let click = |pos: egui::Pos2| {
            vec![
                egui::Event::ModifiersChanged(egui::Modifiers::COMMAND),
                egui::Event::PointerMoved(pos),
                egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: egui::Modifiers::COMMAND,
                },
                egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed: false,
                    modifiers: egui::Modifiers::COMMAND,
                },
            ]
        };
        // Picked bottom row first, to show the drag uses table order.
        accessible_frame(&ctx, &mut app, click(rows[2].1));
        accessible_frame(&ctx, &mut app, click(rows[0].1));
        let start = rows[2].1;
        accessible_frame(
            &ctx,
            &mut app,
            vec![
                egui::Event::ModifiersChanged(egui::Modifiers::NONE),
                egui::Event::PointerMoved(start),
                egui::Event::PointerButton {
                    pos: start,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: egui::Modifiers::NONE,
                },
            ],
        );
        accessible_frame(
            &ctx,
            &mut app,
            vec![egui::Event::PointerMoved(start + egui::vec2(-60.0, -30.0))],
        );
        let payload = egui::DragAndDrop::payload::<DragTrack>(&ctx).expect("a song drag");
        let names: Vec<String> = payload
            .items
            .iter()
            .map(|item| format!("Play {}, {}", item.name(), item.subtitle()))
            .collect();
        assert_eq!(names, [rows[0].0.clone(), rows[2].0.clone()]);
        assert_eq!(payload.from, None, "a selection is copied, not moved");
        egui::DragAndDrop::clear_payload(&ctx);
        app.backend.shutdown();
    }

    /// Rule 10: a "Playing next" row dragged to another slot asks the
    /// engine to move it, a song from elsewhere dropped there asks for it
    /// at that slot, Next up takes nothing, and the Queue button queues at
    /// the end.
    #[test]
    fn songs_dragged_onto_playing_next_move_or_go_in_at_the_slot() {
        use crate::engine::{LocalTrack, PlayerCommand, QueueRow, QueueSnapshot};
        use egui::accesskit::Role;
        let (ctx, mut app) = accessible_app("queue-drag");
        app.show_queue_panel = true;
        let row = |id: &str| QueueRow {
            uri: track_uri(id),
            track: Some(LocalTrack {
                uri: track_uri(id),
                title: format!("Song {id}"),
                ..Default::default()
            }),
        };
        app.handle_queue(QueueSnapshot {
            current: Some(row("trk0")),
            queued: ["q1", "q2", "q3"].map(row).to_vec(),
            upcoming: vec![row("u1")],
            context_uri: Some("sonic:album:x".into()),
            context_at: Some(track_uri("trk0")),
        });
        accessible_frame(&ctx, &mut app, vec![]);
        let tree = accessible_frame(&ctx, &mut app, vec![]);
        let bounds = |label: &str| -> egui::Rect {
            tree.nodes
                .iter()
                .find(|(_, node)| {
                    node.label().is_some_and(|text| text.starts_with(label))
                        && matches!(node.role(), Role::Button)
                })
                .and_then(|(_, node)| node.bounds())
                .map(|rect| {
                    egui::Rect::from_min_max(
                        egui::pos2(rect.x0 as f32, rect.y0 as f32),
                        egui::pos2(rect.x1 as f32, rect.y1 as f32),
                    )
                })
                .unwrap_or_else(|| panic!("no {label:?} on screen"))
        };
        let (q1, q2, q3, u1) = (
            bounds("Play Song q1"),
            bounds("Play Song q2"),
            bounds("Play Song q3"),
            bounds("Play Song u1"),
        );
        // The player bar's button, not the panel's Queue tab above it.
        let queue_button = tree
            .nodes
            .iter()
            .filter(|(_, node)| node.label() == Some("Queue"))
            .filter_map(|(_, node)| node.bounds())
            .max_by(|a, b| a.y0.total_cmp(&b.y0))
            .map(|rect| {
                egui::pos2(
                    ((rect.x0 + rect.x1) / 2.0) as f32,
                    ((rect.y0 + rect.y1) / 2.0) as f32,
                )
            })
            .expect("the player bar's Queue button");
        let release = |ctx: &egui::Context, app: &mut App, at: egui::Pos2| {
            frame_events(ctx, app, vec![egui::Event::PointerMoved(at)]);
            frame_events(
                ctx,
                app,
                vec![egui::Event::PointerButton {
                    pos: at,
                    button: egui::PointerButton::Primary,
                    pressed: false,
                    modifiers: egui::Modifiers::NONE,
                }],
            );
            egui::DragAndDrop::clear_payload(ctx);
        };
        app.backend.asked();

        // A real drag of the last queued row up to the top slot.
        let start = q3.center();
        frame_events(
            &ctx,
            &mut app,
            vec![
                egui::Event::PointerMoved(start),
                egui::Event::PointerButton {
                    pos: start,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: egui::Modifiers::NONE,
                },
            ],
        );
        frame_events(
            &ctx,
            &mut app,
            vec![egui::Event::PointerMoved(start - egui::vec2(0.0, 20.0))],
        );
        let held = egui::DragAndDrop::payload::<DragTrack>(&ctx).expect("a held queue row");
        assert_eq!(held.queued, Some(2));
        release(&ctx, &mut app, egui::pos2(q1.center().x, q1.top() + 2.0));
        assert_eq!(
            app.backend.asked(),
            [PlayerCommand::MoveQueued {
                from: 2,
                to: 0,
                uri: track_uri("q3"),
            }]
        );

        // A song from a list, dropped between the first two rows.
        let stranger = song(&track_uri("new"), "Stranger");
        egui::DragAndDrop::set_payload(&ctx, DragTrack::song(&stranger, None));
        release(&ctx, &mut app, egui::pos2(q2.center().x, q2.top() + 2.0));
        assert_eq!(
            app.backend.asked(),
            [PlayerCommand::InsertQueued {
                uris: vec![track_uri("new")],
                at: 1,
            }]
        );

        // Next up plays from the album; nothing lands there.
        egui::DragAndDrop::set_payload(&ctx, DragTrack::song(&stranger, None));
        release(&ctx, &mut app, u1.center());
        assert!(app.backend.asked().is_empty());

        // The Queue button queues at the end.
        egui::DragAndDrop::set_payload(&ctx, DragTrack::song(&stranger, None));
        release(&ctx, &mut app, queue_button);
        assert_eq!(
            app.backend.asked(),
            [PlayerCommand::AddToQueue(track_uri("new"))]
        );
        app.backend.shutdown();
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

    /// Clicking the search icon in the library header reveals and focuses
    /// the sidebar search field.
    #[test]
    fn clicking_search_in_library_shelf_focuses_search_field() {
        let root = std::env::temp_dir().join(format!(
            "fastsonic-sidebar-search-focus-test-{}",
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

        // Find the Y position of the Library header.
        let mut library_y = None;
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1280.0, 800.0),
            )),
            ..Default::default()
        };
        for _ in 0..2 {
            let mut output = ctx.run_ui(input.clone(), |ui| app.frame_ui(ui));
            output.textures_delta.clear();
            fn walk(shape: &egui::epaint::Shape, found: &mut Option<f32>) {
                match shape {
                    egui::epaint::Shape::Text(text) => {
                        if text.galley.job.text == "Library" {
                            *found = Some(text.pos.y);
                        }
                    }
                    egui::epaint::Shape::Vec(shapes) => {
                        shapes.iter().for_each(|shape| walk(shape, found));
                    }
                    _ => {}
                }
            }
            for clipped in &output.shapes {
                walk(&clipped.shape, &mut library_y);
            }
        }
        let y = library_y.expect("Library label was not found");
        let search_pos = egui::pos2(168.0, y + 4.0);

        // Click on the search button in the Library shelf header.
        frame_events(
            &ctx,
            &mut app,
            vec![
                egui::Event::PointerMoved(search_pos),
                egui::Event::PointerButton {
                    pos: search_pos,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: egui::Modifiers::NONE,
                },
                egui::Event::PointerButton {
                    pos: search_pos,
                    button: egui::PointerButton::Primary,
                    pressed: false,
                    modifiers: egui::Modifiers::NONE,
                },
            ],
        );

        // Advance one frame so the focused widget processes events.
        frame(&ctx, &mut app);

        // Verify the search field is shown and has keyboard focus.
        let search_id = egui::Id::new("sidebar-search");
        let has_focus = ctx.memory(|m| m.has_focus(search_id));
        assert!(
            has_focus,
            "sidebar-search must have keyboard focus after clicking the search icon"
        );

        app.backend.shutdown();
        let _ = std::fs::remove_dir_all(root);
    }

    fn wait_for_themes(ctx: &egui::Context, app: &mut App) {
        let deadline = Instant::now() + std::time::Duration::from_secs(3);
        while app.custom_themes.loading() {
            app.poll_custom_themes(ctx);
            assert!(Instant::now() < deadline, "the theme scan never ended");
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
    }

    /// The painted rect of a label drawn below `below`.
    fn painted_below(painted: &[(String, egui::Rect)], label: &str, below: f32) -> egui::Rect {
        painted
            .iter()
            .find(|(text, rect)| text == label && rect.center().y > below)
            .map(|(_, rect)| *rect)
            .unwrap_or_else(|| panic!("{label:?} was never drawn below {below}: {painted:?}"))
    }

    /// The Theme picker lists Follow system, Light and Dark, then the files
    /// in the themes folder. Picking one shows it at once, and a screen
    /// reader hears the picker's name and what it shows.
    #[test]
    fn the_theme_picker_lists_the_built_in_themes_then_the_palette_files() {
        let (ctx, mut app) = accessible_app("custom-theme-picker");
        app.backend.shutdown();
        let themes = app.dirs.themes_dir();
        std::fs::create_dir_all(&themes).unwrap();
        std::fs::write(
            themes.join("local.json"),
            r##"{"base":"light","colors":{"accent":"#8c3fa5"}}"##,
        )
        .unwrap();
        app.open(Page::Settings);
        wait_for_themes(&ctx, &mut app);
        let view = App::frame_ui;
        for _ in 0..3 {
            view_frame(&ctx, &mut app, vec![], view);
        }
        let painted = view_frame(&ctx, &mut app, vec![], view);
        let theme_row = painted_below(&painted, "Theme", 0.0).center().y;
        let picker = painted_below(&painted, "Dark", theme_row - 20.0).center();
        view_frame(
            &ctx,
            &mut app,
            pointer_click(picker, egui::PointerButton::Primary),
            view,
        );
        assert!(
            app.custom_themes.loading(),
            "opening the picker lists the folder again"
        );
        wait_for_themes(&ctx, &mut app);
        let painted = view_frame(&ctx, &mut app, vec![], view);
        let entry = |name: &str| painted_below(&painted, name, picker.y + 1.0).center();
        assert!(entry("Follow system").y < entry("Light").y);
        assert!(entry("Light").y < entry("Dark").y);
        // Listed by name, without `.json`.
        assert!(entry("Dark").y < entry("local").y);
        view_frame(
            &ctx,
            &mut app,
            pointer_click(entry("local"), egui::PointerButton::Primary),
            view,
        );
        let mut palette = crate::theme::Palette::light();
        palette.accent = egui::Color32::from_rgb(0x8c, 0x3f, 0xa5);
        assert_eq!(app.settings.custom_theme.as_deref(), Some("local.json"));
        assert_eq!(app.palette, palette);
        assert_eq!(app.settings.cached_palette(), Some(palette));
        assert_eq!(ctx.theme(), egui::Theme::Light);

        let tree = accessible_frame(&ctx, &mut app, vec![]);
        let id = accessible_node(&tree, "Theme", egui::accesskit::Role::ComboBox);
        let node = &tree.nodes.iter().find(|(node, _)| *node == id).unwrap().1;
        assert_eq!(node.value(), Some("local"), "read out as shown");
        wait_for_themes(&ctx, &mut app);
        let _ = std::fs::remove_dir_all(themes);
    }

    /// Beside the themes folder, a button opens the guide to writing a
    /// theme. Only the page is drawn, so the click's action is collected
    /// and never opens a browser.
    #[test]
    fn the_theme_row_links_to_the_guide_to_making_a_theme() {
        let (ctx, mut app) = accessible_app("theme-guide");
        app.backend.shutdown();
        let view = crate::ui::settings::show;
        for _ in 0..3 {
            view_frame(&ctx, &mut app, vec![], view);
        }
        let painted = view_frame(&ctx, &mut app, vec![], view);
        let guide = painted_below(&painted, "How to make a theme", 0.0).center();
        let folder = painted_below(&painted, "Open themes folder", 0.0).center();
        assert!((guide.y - folder.y).abs() < 1.0, "side by side");
        assert!(
            folder.x < guide.x,
            "the guide at the edge, like Skin Museum"
        );
        app.actions.clear();
        view_frame(
            &ctx,
            &mut app,
            pointer_click(guide, egui::PointerButton::Primary),
            view,
        );
        assert!(app.actions.iter().any(|action| matches!(action,
            Action::OpenUrl(url) if url == "https://github.com/rwojsznis/fastsonic/blob/main/docs/_reference/settings-and-files.md#custom-themes")));
    }

    /// The folder button looks and reads like the Winamp skins one, and
    /// only asks for the folder: drawing never creates or opens it.
    #[test]
    fn the_themes_folder_button_asks_to_open_the_folder() {
        let (ctx, mut app) = accessible_app("themes-folder-button");
        app.backend.shutdown();
        let view = crate::ui::settings::show;
        for _ in 0..3 {
            view_frame(&ctx, &mut app, vec![], view);
        }
        let painted = view_frame(&ctx, &mut app, vec![], view);
        let button = painted_below(&painted, "Open themes folder", 0.0).center();
        app.actions.clear();
        view_frame(
            &ctx,
            &mut app,
            pointer_click(button, egui::PointerButton::Primary),
            view,
        );
        assert!(matches!(app.actions.as_slice(), [Action::OpenThemesFolder]));
        app.actions.clear();
        app.open(Page::Settings);
        accessible_frame(&ctx, &mut app, vec![]);
        let tree = accessible_frame(&ctx, &mut app, vec![]);
        accessible_node(&tree, "Open themes folder", egui::accesskit::Role::Button);
        wait_for_themes(&ctx, &mut app);
        assert!(
            !app.dirs.themes_dir().exists(),
            "drawing cannot open or create folders"
        );
    }
}
