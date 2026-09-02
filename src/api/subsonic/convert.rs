//! Turning Subsonic answers into the vocabulary the app already speaks.
//!
//! `api::models` stays canonical (D5): `Track`, `Album`, `Artist`,
//! `Playlist` and `Page<T>` are what 20 files import, and the interface is
//! written against them. Nothing here renames those types after Subsonic;
//! everything here adapts into them.
//!
//! Three seams are worth knowing about.
//!
//! **Identity.** Spotify handed every object a `spotify:kind:id` URI and the
//! app routes on it. Subsonic hands over a bare id, so the URI is built
//! here, as `sonic:kind:id`, and parsed back by [`parse_uri`].
//!
//! **Artwork.** Cover art is a request you construct, not a URL the server
//! gives you, and constructing it needs the credential. Rather than bake a
//! credential into objects that get cached to disk, art is carried as
//! `sonic:art:<size>:<cover-art-id>` and resolved by `src/images.rs` at
//! fetch time. Artist images are the exception: those really are URLs, and
//! pre-signed ones that need no credential, so they pass through unchanged.
//!
//! **Paging.** Most Subsonic list endpoints return everything at once, so
//! the `Page<T>` the app expects is synthesised here by slicing.

use crate::api::models::{
    Album, Artist, ArtistRef, Image, Owner, Page, PlayableItem, Playlist, PlaylistItem,
    SearchResults, Track, TrackCount, User,
};

use super::types::{AlbumId3, ArtistId3, ArtistInfo2, ArtistWithAlbumsId3, Child, SearchResult3};
use super::types::{Playlist as SubsonicPlaylist, User as SubsonicUser};

/// The URI scheme this app addresses its own server's objects with.
pub const URI_SCHEME: &str = "sonic";

/// What a `sonic:` URI points at.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Track,
    Album,
    Artist,
    Playlist,
    /// Everything starred — the Liked Songs page. There is one of these per
    /// server rather than one per object, so its id is a fixed word; see
    /// [`COLLECTION_URI`].
    Collection,
}

impl Kind {
    fn as_str(self) -> &'static str {
        match self {
            Self::Track => "track",
            Self::Album => "album",
            Self::Artist => "artist",
            Self::Playlist => "playlist",
            Self::Collection => "collection",
        }
    }

    fn from_str(value: &str) -> Option<Self> {
        match value {
            "track" => Some(Self::Track),
            "album" => Some(Self::Album),
            "artist" => Some(Self::Artist),
            "playlist" => Some(Self::Playlist),
            "collection" => Some(Self::Collection),
            _ => None,
        }
    }
}

/// The starred songs, as something playable. Spotify addressed this list
/// per account (`spotify:user:<id>:collection`); one server has one starred
/// list, so this is a constant rather than something built from the user.
pub const COLLECTION_URI: &str = "sonic:collection:songs";

pub fn uri(kind: Kind, id: &str) -> String {
    format!("{URI_SCHEME}:{}:{id}", kind.as_str())
}

pub fn track_uri(id: &str) -> String {
    uri(Kind::Track, id)
}

pub fn album_uri(id: &str) -> String {
    uri(Kind::Album, id)
}

pub fn artist_uri(id: &str) -> String {
    uri(Kind::Artist, id)
}

pub fn playlist_uri(id: &str) -> String {
    uri(Kind::Playlist, id)
}

/// Splits a `sonic:kind:id` URI. The id keeps every remaining colon, since
/// nothing promises a server's ids do not contain one.
pub fn parse_uri(uri: &str) -> Option<(Kind, &str)> {
    let rest = uri.strip_prefix(URI_SCHEME)?.strip_prefix(':')?;
    let (kind, id) = rest.split_once(':')?;
    if id.is_empty() {
        return None;
    }
    Some((Kind::from_str(kind)?, id))
}

/// The id out of a URI of the expected kind.
pub fn id_of(uri: &str, kind: Kind) -> Option<&str> {
    parse_uri(uri)
        .filter(|(found, _)| *found == kind)
        .map(|(_, id)| id)
}

// ---- artwork -------------------------------------------------------------

/// The sizes art is offered in, so `models::pick_image` keeps choosing the
/// way it did when Spotify offered three.
pub const ART_SIZES: [u32; 3] = [64, 300, 640];

/// A cover-art request, deferred: `src/images.rs` turns this into a real
/// `getCoverArt` URL with the current credential when it fetches.
pub fn art_url(cover_art_id: &str, size: u32) -> String {
    format!("{URI_SCHEME}:art:{size}:{cover_art_id}")
}

/// Splits an art request back into its size and cover-art id.
pub fn parse_art_url(url: &str) -> Option<(u32, &str)> {
    let rest = url.strip_prefix(URI_SCHEME)?.strip_prefix(":art:")?;
    let (size, id) = rest.split_once(':')?;
    let size = size.parse().ok()?;
    (!id.is_empty()).then_some((size, id))
}

/// Artwork for an object whose own cover-art id is not known. Navidrome
/// accepts a bare album or artist id where a cover-art id is expected, which
/// is what the native API's answers — which carry no `coverArt` — rely on.
pub fn art_images_for(id: &str) -> Vec<Image> {
    art_images(Some(&id.to_string()))
}

fn art_images(cover_art: Option<&String>) -> Vec<Image> {
    let Some(id) = cover_art.filter(|id| !id.is_empty()) else {
        return Vec::new();
    };
    ART_SIZES
        .iter()
        .map(|size| Image {
            url: art_url(id, *size),
            width: Some(*size),
            height: Some(*size),
        })
        .collect()
}

/// An artist image, which unlike cover art is a pre-signed URL the server
/// serves without any credential. One entry, because the server offers one.
fn artist_images(artist_image_url: Option<&String>) -> Vec<Image> {
    artist_image_url
        .filter(|url| !url.is_empty())
        .map(|url| {
            vec![Image {
                url: url.clone(),
                width: None,
                height: None,
            }]
        })
        .unwrap_or_default()
}

/// The three sizes `getArtistInfo2` offers, folded in beside whatever
/// `getArtist` already gave. These are pre-signed URLs too, so they cost no
/// credential; on a server with no Last.fm agent they are all the call
/// returns, which is why an empty biography is not a failed page.
pub fn info_images(info: &ArtistInfo2, existing: Vec<Image>) -> Vec<Image> {
    let offered = [
        (info.small_image_url.as_ref(), 64_u32),
        (info.medium_image_url.as_ref(), 300),
        (info.large_image_url.as_ref(), 640),
    ];
    let mut images: Vec<Image> = offered
        .into_iter()
        .filter_map(|(url, size)| {
            url.filter(|url| !url.is_empty()).map(|url| Image {
                url: url.clone(),
                width: Some(size),
                height: Some(size),
            })
        })
        .collect();
    if images.is_empty() {
        return existing;
    }
    // Keep anything `getArtist` knew that this call did not repeat.
    let kept: Vec<Image> = existing
        .into_iter()
        .filter(|image| !images.iter().any(|offered| offered.url == image.url))
        .collect();
    images.extend(kept);
    images
}

// ---- objects -------------------------------------------------------------

pub fn artist_ref(artist: &ArtistId3) -> ArtistRef {
    ArtistRef {
        id: (!artist.id.is_empty()).then(|| artist.id.clone()),
        name: artist.name.clone(),
        uri: (!artist.id.is_empty()).then(|| artist_uri(&artist.id)),
    }
}

pub fn artist(artist: &ArtistId3) -> Artist {
    Artist {
        id: artist.id.clone(),
        name: artist.name.clone(),
        uri: artist_uri(&artist.id),
        images: artist_images(artist.artist_image_url.as_ref()),
        genres: Vec::new(),
        followers: None,
        popularity: None,
        external_urls: Default::default(),
        starred: Some(artist.starred.is_some()),
    }
}

pub fn artist_with_albums(artist: &ArtistWithAlbumsId3) -> Artist {
    Artist {
        id: artist.id.clone(),
        name: artist.name.clone(),
        uri: artist_uri(&artist.id),
        images: artist_images(artist.artist_image_url.as_ref()),
        genres: Vec::new(),
        followers: None,
        popularity: None,
        external_urls: Default::default(),
        starred: Some(artist.starred.is_some()),
    }
}

/// The artists credited on a song or an album. OpenSubsonic servers send the
/// full list; the single `artist` string is the fallback, and it is the one
/// place a name containing a comma would be lost if it were split.
fn credited(artists: &[ArtistId3], name: Option<&String>, id: Option<&String>) -> Vec<ArtistRef> {
    if !artists.is_empty() {
        return artists.iter().map(artist_ref).collect();
    }
    match name {
        Some(name) if !name.is_empty() => vec![ArtistRef {
            id: id.cloned(),
            name: name.clone(),
            uri: id.map(|id| artist_uri(id)),
        }],
        _ => Vec::new(),
    }
}

pub fn album(album: &AlbumId3) -> Album {
    Album {
        id: album.id.clone(),
        name: album.name.clone(),
        uri: album_uri(&album.id),
        album_type: album_kind(album),
        album_group: None,
        total_tracks: album.song_count.map(|count| count.max(0) as u32),
        images: art_images(album.cover_art.as_ref().or(Some(&album.id))),
        artists: credited(
            &album.artists,
            album.artist.as_ref(),
            album.artist_id.as_ref(),
        ),
        release_date: album.year.map(|year| year.to_string()),
        label: album.record_labels.first().map(|label| label.name.clone()),
        genres: album_genres(album),
        popularity: None,
        tracks: None,
        external_urls: Default::default(),
        copyrights: Vec::new(),
        starred: Some(album.starred.is_some()),
    }
}

/// `Album::kind_label` reads this, and understands "single", "compilation"
/// and "appears_on". OpenSubsonic's `releaseTypes` uses the same words with
/// different capitalisation; `isCompilation` is the older signal.
fn album_kind(album: &AlbumId3) -> Option<String> {
    if let Some(kind) = album.release_types.first() {
        return Some(kind.to_lowercase());
    }
    album
        .is_compilation
        .unwrap_or_default()
        .then(|| "compilation".to_string())
}

fn album_genres(album: &AlbumId3) -> Vec<String> {
    if !album.genres.is_empty() {
        return album
            .genres
            .iter()
            .map(|genre| genre.name.clone())
            .collect();
    }
    album.genre.iter().cloned().collect()
}

/// The album a song belongs to, as much of it as the song carries. Enough
/// for a row's artwork and its "from" line; the album page loads the rest.
fn song_album(song: &Child) -> Option<Album> {
    let id = song.album_id.clone().unwrap_or_default();
    let name = song.album.clone().unwrap_or_default();
    if id.is_empty() && name.is_empty() {
        return None;
    }
    Some(Album {
        id: id.clone(),
        name,
        uri: album_uri(&id),
        images: art_images(song.cover_art.as_ref()),
        artists: credited(
            &song.album_artists,
            song.display_album_artist.as_ref(),
            None,
        ),
        release_date: song.year.map(|year| year.to_string()),
        ..Album::default()
    })
}

pub fn track(song: &Child) -> Track {
    Track {
        id: (!song.id.is_empty()).then(|| song.id.clone()),
        name: song.title.clone(),
        uri: track_uri(&song.id),
        // Subsonic durations are whole seconds. The app speaks
        // milliseconds, and the precision it is asking for is not there.
        duration_ms: song.duration.unwrap_or_default().max(0) as u32 * 1000,
        explicit: song.explicit_status.as_deref() == Some("explicit"),
        artists: credited(&song.artists, song.artist.as_ref(), song.artist_id.as_ref()),
        album: song_album(song),
        track_number: song.track.map(|number| number.max(0) as u32),
        disc_number: song.disc_number.map(|number| number.max(0) as u32),
        is_local: false,
        is_playable: Some(true),
        popularity: None,
        external_urls: Default::default(),
        starred: Some(song.starred.is_some()),
    }
}

pub fn playlist(playlist: &SubsonicPlaylist) -> Playlist {
    Playlist {
        id: playlist.id.clone(),
        name: playlist.name.clone(),
        uri: playlist_uri(&playlist.id),
        description: playlist.comment.clone(),
        images: art_images(playlist.cover_art.as_ref()),
        owner: Owner {
            id: playlist.owner.clone(),
            display_name: playlist.owner.clone(),
            uri: None,
        },
        public: playlist.public,
        collaborative: false,
        // `changed` is the nearest thing Subsonic has to a snapshot: it moves
        // whenever the contents do, so a stale view can be noticed.
        snapshot_id: playlist.changed.clone(),
        tracks: Some(TrackCount {
            total: playlist.song_count.unwrap_or_default().max(0) as u32,
        }),
        items_count: None,
        external_urls: Default::default(),
    }
}

/// A playlist's entries, as the rows the app draws. `added_at` and
/// `added_by` have no Subsonic equivalent — a playlist entry carries no
/// history — so the "added by" column has nothing to show.
pub fn playlist_items(songs: &[Child]) -> Vec<PlaylistItem> {
    songs
        .iter()
        .map(|song| PlaylistItem {
            added_at: None,
            added_by: None,
            is_local: false,
            item: Some(PlayableItem::Track(track(song))),
            track: None,
        })
        .collect()
}

pub fn user(user: &SubsonicUser) -> User {
    User {
        id: user.username.clone(),
        display_name: Some(user.username.clone()),
        images: Vec::new(),
        // No plan, no tier: every Subsonic account can stream, which is why
        // the Premium check disappears rather than being translated.
        product: None,
        country: None,
        uri: None,
    }
}

pub fn search_results(results: &SearchResult3, offset: u32, limit: u32) -> SearchResults {
    SearchResults {
        tracks: Some(page(
            results.song.iter().map(track).collect(),
            offset,
            limit,
        )),
        artists: Some(page(
            results.artist.iter().map(artist).collect(),
            offset,
            limit,
        )),
        albums: Some(page(
            results.album.iter().map(album).collect(),
            offset,
            limit,
        )),
        // search3 has no playlist, show or episode bucket.
        playlists: None,
        shows: None,
        episodes: None,
    }
}

// ---- paging --------------------------------------------------------------

/// A page whose server *did* page: a full page implies there may be more,
/// which is all a `size`/`offset` endpoint tells you.
pub fn page<T>(items: Vec<T>, offset: u32, limit: u32) -> Page<T> {
    let count = items.len() as u32;
    let more = limit > 0 && count >= limit;
    Page {
        total: offset + count + u32::from(more),
        limit,
        offset,
        next: more.then(|| (offset + count).to_string()),
        items,
    }
}

/// A page cut out of a list the server returned whole — `getStarred2`,
/// `getPlaylist`, `getArtist` — where the true total is known exactly.
pub fn slice<T: Clone>(items: &[T], offset: u32, limit: u32) -> Page<T> {
    let total = items.len() as u32;
    let start = offset.min(total) as usize;
    let end = if limit == 0 {
        items.len()
    } else {
        (start + limit as usize).min(items.len())
    };
    Page {
        items: items[start..end].to_vec(),
        total,
        limit,
        offset,
        next: ((end as u32) < total).then(|| end.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::models::pick_image;
    use crate::api::subsonic::types::{ItemGenre, ReplayGain};

    fn song() -> Child {
        Child {
            id: "s1".into(),
            title: "Signal Path".into(),
            album: Some("Blue Harvest".into()),
            album_id: Some("al1".into()),
            artist: Some("Kestrel".into()),
            artist_id: Some("ar1".into()),
            cover_art: Some("al-al1".into()),
            duration: Some(9),
            track: Some(3),
            disc_number: Some(1),
            year: Some(2019),
            replay_gain: Some(ReplayGain {
                track_gain: Some(-3.15),
                ..ReplayGain::default()
            }),
            ..Child::default()
        }
    }

    #[test]
    fn uris_round_trip() {
        assert_eq!(track_uri("s1"), "sonic:track:s1");
        assert_eq!(parse_uri("sonic:album:al1"), Some((Kind::Album, "al1")));
        assert_eq!(id_of("sonic:artist:ar1", Kind::Artist), Some("ar1"));
        assert_eq!(id_of("sonic:artist:ar1", Kind::Album), None);
        assert_eq!(parse_uri("spotify:track:x"), None);
        assert_eq!(parse_uri("sonic:track:"), None);
        assert_eq!(parse_uri("sonic:bogus:x"), None);
        // Ids are opaque: whatever a server puts after the kind is the id.
        assert_eq!(parse_uri("sonic:track:a:b"), Some((Kind::Track, "a:b")));
    }

    #[test]
    fn art_is_a_deferred_request_not_a_url() {
        let track = track(&song());
        let images = &track.album.as_ref().unwrap().images;
        assert_eq!(images.len(), ART_SIZES.len());
        // The interface still picks a size the way it always has.
        assert_eq!(pick_image(images, 100), Some("sonic:art:300:al-al1"));
        assert_eq!(parse_art_url("sonic:art:300:al-al1"), Some((300, "al-al1")));
        // Nothing here carries a credential, so nothing that gets cached does.
        assert!(!images.iter().any(|image| image.url.contains("t=")));
    }

    #[test]
    fn an_artist_image_passes_through_because_it_needs_no_credential() {
        let artist = artist(&ArtistId3 {
            id: "ar1".into(),
            name: "Kestrel".into(),
            artist_image_url: Some("http://host/share/img/JWT?size=300".into()),
            ..ArtistId3::default()
        });
        assert_eq!(
            pick_image(&artist.images, 300),
            Some("http://host/share/img/JWT?size=300")
        );
    }

    #[test]
    fn seconds_become_milliseconds() {
        assert_eq!(track(&song()).duration_ms, 9_000);
    }

    #[test]
    fn a_song_without_the_open_subsonic_artist_list_keeps_one_name_whole() {
        let mut song = song();
        song.artist = Some("Tyler, the Creator".into());
        let track = track(&song);
        assert_eq!(track.artists.len(), 1);
        assert_eq!(track.artists[0].name, "Tyler, the Creator");
        assert_eq!(track.artists[0].uri.as_deref(), Some("sonic:artist:ar1"));
    }

    #[test]
    fn every_credited_artist_survives_when_the_server_lists_them() {
        let mut song = song();
        song.artists = vec![
            ArtistId3 {
                id: "ar1".into(),
                name: "Kestrel".into(),
                ..ArtistId3::default()
            },
            ArtistId3 {
                id: "ar2".into(),
                name: "Someone Else".into(),
                ..ArtistId3::default()
            },
        ];
        assert_eq!(track(&song).artist_names(), "Kestrel, Someone Else");
    }

    #[test]
    fn an_album_carries_the_words_the_interface_labels_it_with() {
        let compilation = album(&AlbumId3 {
            id: "al2".into(),
            name: "Fastsonic Sampler".into(),
            release_types: vec!["Compilation".into()],
            genres: vec![ItemGenre {
                name: "Test Tones".into(),
            }],
            song_count: Some(4),
            year: Some(2024),
            ..AlbumId3::default()
        });
        assert_eq!(compilation.kind_label(), "Compilation");
        assert_eq!(compilation.year(), Some("2024"));
        assert_eq!(compilation.genres, vec!["Test Tones".to_string()]);
        assert_eq!(compilation.total_tracks, Some(4));
    }

    #[test]
    fn a_playlist_keeps_a_change_marker_to_notice_a_stale_view_with() {
        let converted = playlist(&SubsonicPlaylist {
            id: "p1".into(),
            name: "Mix".into(),
            owner: Some("admin".into()),
            song_count: Some(4),
            changed: Some("2026-09-01T22:02:00Z".into()),
            ..SubsonicPlaylist::default()
        });
        assert_eq!(converted.track_total(), 4);
        assert!(converted.owned_by("admin"));
        assert_eq!(
            converted.snapshot_id.as_deref(),
            Some("2026-09-01T22:02:00Z")
        );
    }

    #[test]
    fn a_whole_list_is_sliced_into_the_pages_the_interface_asks_for() {
        let items: Vec<u32> = (0..5).collect();
        let first = slice(&items, 0, 2);
        assert_eq!(first.items, vec![0, 1]);
        assert_eq!(first.total, 5);
        assert_eq!(first.next_offset(), Some(2));

        let last = slice(&items, 4, 2);
        assert_eq!(last.items, vec![4]);
        assert_eq!(last.next_offset(), None);

        // Past the end is an empty page, not a panic.
        assert!(slice(&items, 9, 2).items.is_empty());
        // A limit of zero means "all of it", which is how the callers that
        // do not page ask.
        assert_eq!(slice(&items, 0, 0).items.len(), 5);
    }

    #[test]
    fn a_server_paged_list_offers_another_page_only_while_it_is_full() {
        let full = page(vec![1, 2], 0, 2);
        assert_eq!(full.next_offset(), Some(2));
        let short = page(vec![1], 2, 2);
        assert_eq!(short.next_offset(), None);
        assert_eq!(short.total, 3);
    }
}
