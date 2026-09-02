//! Subsonic / OpenSubsonic response shapes.
//!
//! Vendored from the `opensubsonic` crate 0.4.0 (`src/data/`), MIT OR
//! Apache-2.0, Copyright (c) 2026 Gianluca Boiano — see `THIRD-PARTY.md`.
//! Only the types this client reads are kept; jukebox, chat,
//! bookmarks, internet radio, sharing and video are dropped.
//!
//! Two changes to the vendored shapes, both from measuring Navidrome 0.63.2
//! (`migration/01-api-mapping.md` Findings):
//!
//! - Every struct is `#[serde(default)]` and derives `Default`. Navidrome
//!   omits keys rather than sending nulls or empty arrays — an empty
//!   collection arrives as `{}` with the inner `song`/`album`/`artist` key
//!   *absent* — so a missing field must never fail a page.
//! - `Serialize` is kept only where something is written back to disk or to
//!   the server; the rest deserialize only.

use serde::Deserialize;

// ---- shared -------------------------------------------------------------

/// A genre tag on a media item.
#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
#[serde(default, rename_all = "camelCase")]
pub struct ItemGenre {
    pub name: String,
}

/// A date that may be partial: year only, year and month, or a full date.
#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq)]
#[serde(default, rename_all = "camelCase")]
pub struct ItemDate {
    pub year: Option<i32>,
    pub month: Option<i32>,
    pub day: Option<i32>,
}

/// A disc title for a multi-disc album.
#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
#[serde(default, rename_all = "camelCase")]
pub struct DiscTitle {
    pub disc: i32,
    pub title: String,
    pub cover_art: Option<String>,
}

/// A record label for an album.
#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
#[serde(default, rename_all = "camelCase")]
pub struct RecordLabel {
    pub name: String,
}

/// ReplayGain values, served with every song rather than read from tags.
#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq)]
#[serde(default, rename_all = "camelCase")]
pub struct ReplayGain {
    pub track_gain: Option<f64>,
    pub album_gain: Option<f64>,
    pub track_peak: Option<f64>,
    pub album_peak: Option<f64>,
    /// Base gain in dB, such as an Ogg Opus output gain.
    pub base_gain: Option<f64>,
    /// Used when neither track nor album gain is present.
    pub fallback_gain: Option<f64>,
}

/// A supported OpenSubsonic extension. `getOpenSubsonicExtensions` answers
/// with a bare array of these rather than an object wrapping one.
#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
#[serde(default, rename_all = "camelCase")]
pub struct OpenSubsonicExtension {
    pub name: String,
    pub versions: Vec<i32>,
}

// ---- artists ------------------------------------------------------------

/// An artist from ID3 tags.
#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
#[serde(default, rename_all = "camelCase")]
pub struct ArtistId3 {
    pub id: String,
    pub name: String,
    pub cover_art: Option<String>,
    /// A pre-signed `/share/img/<JWT>` URL that needs no auth parameters.
    pub artist_image_url: Option<String>,
    pub album_count: Option<i64>,
    /// When the artist was starred, ISO 8601. Absent when it is not.
    pub starred: Option<String>,
    pub music_brainz_id: Option<String>,
    pub sort_name: Option<String>,
    pub roles: Vec<String>,
}

/// `getArtist`: an artist together with its albums.
#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
#[serde(default, rename_all = "camelCase")]
pub struct ArtistWithAlbumsId3 {
    pub id: String,
    pub name: String,
    pub cover_art: Option<String>,
    pub artist_image_url: Option<String>,
    pub album_count: Option<i64>,
    pub starred: Option<String>,
    pub music_brainz_id: Option<String>,
    pub sort_name: Option<String>,
    pub roles: Vec<String>,
    pub album: Vec<AlbumId3>,
}

/// `getArtists`: the artist index, grouped by first letter.
#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
#[serde(default, rename_all = "camelCase")]
pub struct ArtistsId3 {
    pub ignored_articles: Option<String>,
    pub index: Vec<IndexId3>,
}

/// One letter of the artist index.
#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
#[serde(default, rename_all = "camelCase")]
pub struct IndexId3 {
    pub name: String,
    pub artist: Vec<ArtistId3>,
}

/// `getArtistInfo2`. Without a Last.fm agent configured the server fills in
/// the image URLs and nothing else, which is the ordinary case for a
/// self-hoster (D11).
#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
#[serde(default, rename_all = "camelCase")]
pub struct ArtistInfo2 {
    pub biography: Option<String>,
    pub music_brainz_id: Option<String>,
    pub last_fm_url: Option<String>,
    pub small_image_url: Option<String>,
    pub medium_image_url: Option<String>,
    pub large_image_url: Option<String>,
    pub similar_artist: Vec<ArtistId3>,
}

// ---- albums -------------------------------------------------------------

/// An album from ID3 tags.
#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
#[serde(default, rename_all = "camelCase")]
pub struct AlbumId3 {
    pub id: String,
    pub name: String,
    /// "Remastered", "Deluxe Edition", and the like.
    pub version: Option<String>,
    pub artist: Option<String>,
    pub artist_id: Option<String>,
    pub cover_art: Option<String>,
    pub song_count: Option<i64>,
    /// Seconds, not milliseconds.
    pub duration: Option<i64>,
    pub play_count: Option<i64>,
    pub created: Option<String>,
    pub starred: Option<String>,
    pub year: Option<i32>,
    pub genre: Option<String>,
    pub played: Option<String>,
    pub user_rating: Option<i32>,
    pub record_labels: Vec<RecordLabel>,
    pub music_brainz_id: Option<String>,
    pub genres: Vec<ItemGenre>,
    pub artists: Vec<ArtistId3>,
    pub display_artist: Option<String>,
    /// "album", "compilation", "ep", "single", …
    pub release_types: Vec<String>,
    pub original_release_date: Option<ItemDate>,
    pub release_date: Option<ItemDate>,
    pub is_compilation: Option<bool>,
    pub sort_name: Option<String>,
    pub disc_titles: Vec<DiscTitle>,
    pub explicit_status: Option<String>,
    pub moods: Vec<String>,
}

/// `getAlbum`: an album together with its songs. There is no second call
/// and no paging — the tracks always come with the album.
#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
#[serde(default, rename_all = "camelCase")]
pub struct AlbumWithSongsId3 {
    #[serde(flatten)]
    pub album: AlbumId3,
    pub song: Vec<Child>,
}

/// `getAlbumInfo2`, the album's Last.fm notes. Empty without a key.
#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
#[serde(default, rename_all = "camelCase")]
pub struct AlbumInfo {
    pub notes: Option<String>,
    pub music_brainz_id: Option<String>,
    pub last_fm_url: Option<String>,
    pub small_image_url: Option<String>,
    pub medium_image_url: Option<String>,
    pub large_image_url: Option<String>,
}

// ---- songs --------------------------------------------------------------

/// A song. `Child` is the Subsonic name and the type nearly every listing
/// endpoint returns, so it keeps that name here rather than becoming a
/// second `Track` next to `api::models::Track`.
#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
#[serde(default, rename_all = "camelCase")]
pub struct Child {
    pub id: String,
    /// The album this song belongs to, for directory-style browsing.
    pub parent: Option<String>,
    pub is_dir: bool,
    pub title: String,
    pub album: Option<String>,
    pub artist: Option<String>,
    pub track: Option<i32>,
    pub year: Option<i32>,
    pub genre: Option<String>,
    pub cover_art: Option<String>,
    pub size: Option<i64>,
    pub content_type: Option<String>,
    pub suffix: Option<String>,
    pub transcoded_content_type: Option<String>,
    pub transcoded_suffix: Option<String>,
    /// Seconds, integer. The app speaks milliseconds, so the adapter
    /// multiplies and the sub-second precision simply is not there.
    pub duration: Option<i64>,
    pub bit_rate: Option<i32>,
    pub bit_depth: Option<i32>,
    pub sampling_rate: Option<i32>,
    pub channel_count: Option<i32>,
    pub path: Option<String>,
    pub is_video: Option<bool>,
    pub user_rating: Option<i32>,
    pub average_rating: Option<f64>,
    pub play_count: Option<i64>,
    pub disc_number: Option<i32>,
    pub created: Option<String>,
    pub starred: Option<String>,
    pub album_id: Option<String>,
    pub artist_id: Option<String>,
    #[serde(rename = "type")]
    pub media_type_generic: Option<String>,
    pub media_type: Option<String>,
    pub bookmark_position: Option<i64>,
    pub played: Option<String>,
    pub bpm: Option<i32>,
    pub comment: Option<String>,
    pub sort_name: Option<String>,
    pub music_brainz_id: Option<String>,
    pub isrc: Vec<String>,
    pub genres: Vec<ItemGenre>,
    /// Every credited artist, where `artist` is only the first.
    pub artists: Vec<ArtistId3>,
    pub display_artist: Option<String>,
    pub album_artists: Vec<ArtistId3>,
    pub display_album_artist: Option<String>,
    pub moods: Vec<String>,
    pub replay_gain: Option<ReplayGain>,
    /// "explicit", "clean", or empty.
    pub explicit_status: Option<String>,
}

/// A `getNowPlaying` entry: a song plus who is playing it and where.
#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
#[serde(default, rename_all = "camelCase")]
pub struct NowPlayingEntry {
    #[serde(flatten)]
    pub child: Child,
    pub username: Option<String>,
    pub minutes_ago: Option<i64>,
    pub player_id: Option<i64>,
    pub player_name: Option<String>,
    /// The `playbackReport` extension fills these three in.
    pub state: Option<String>,
    pub position_ms: Option<i64>,
    pub playback_rate: Option<f64>,
}

// ---- playlists ----------------------------------------------------------

/// A playlist without its entries.
#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
#[serde(default, rename_all = "camelCase")]
pub struct Playlist {
    pub id: String,
    pub name: String,
    pub comment: Option<String>,
    pub owner: Option<String>,
    pub public: Option<bool>,
    pub song_count: Option<i64>,
    pub duration: Option<i64>,
    pub created: Option<String>,
    pub changed: Option<String>,
    pub cover_art: Option<String>,
    pub allowed_user: Vec<String>,
    pub readonly: Option<bool>,
    pub valid_until: Option<String>,
}

/// `getPlaylist`: a playlist with **every** entry in one response. There is
/// no offset or limit; the paging the app does is local slicing.
#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
#[serde(default, rename_all = "camelCase")]
pub struct PlaylistWithSongs {
    #[serde(flatten)]
    pub playlist: Playlist,
    pub entry: Vec<Child>,
}

// ---- search -------------------------------------------------------------

/// `search3`. Artists, albums and songs only: there is no playlist bucket.
#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
#[serde(default, rename_all = "camelCase")]
pub struct SearchResult3 {
    pub artist: Vec<ArtistId3>,
    pub album: Vec<AlbumId3>,
    pub song: Vec<Child>,
}

// ---- lyrics -------------------------------------------------------------

/// One line of lyrics. `start` is milliseconds, and present only when the
/// lyrics are synced.
#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
#[serde(default, rename_all = "camelCase")]
pub struct Line {
    pub value: String,
    pub start: Option<f64>,
}

/// One set of lyrics for a song, in one language.
#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
#[serde(default, rename_all = "camelCase")]
pub struct StructuredLyrics {
    pub lang: String,
    pub synced: bool,
    pub line: Vec<Line>,
    pub display_artist: Option<String>,
    pub display_title: Option<String>,
    /// Milliseconds to shift every line by.
    pub offset: Option<f64>,
    /// "main", "translation" or "pronunciation" (songLyrics v2).
    pub kind: Option<String>,
}

/// `getLyricsBySongId`, which may carry several languages.
#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
#[serde(default, rename_all = "camelCase")]
pub struct LyricsList {
    pub structured_lyrics: Vec<StructuredLyrics>,
}

/// `getLyrics`, the flat pre-OpenSubsonic shape.
#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
#[serde(default, rename_all = "camelCase")]
pub struct Lyrics {
    pub value: Option<String>,
    pub artist: Option<String>,
    pub title: Option<String>,
}

// ---- session ------------------------------------------------------------

/// `getUser`. There is no plan, tier or product here — every Subsonic user
/// can stream, which is why the old plan check goes away.
#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
#[serde(default, rename_all = "camelCase")]
pub struct User {
    pub username: String,
    pub scrobbling_enabled: Option<bool>,
    pub max_bit_rate: Option<i32>,
    pub admin_role: Option<bool>,
    pub settings_role: Option<bool>,
    pub download_role: Option<bool>,
    pub upload_role: Option<bool>,
    pub playlist_role: Option<bool>,
    pub cover_art_role: Option<bool>,
    pub comment_role: Option<bool>,
    pub stream_role: Option<bool>,
    pub jukebox_role: Option<bool>,
    pub share_role: Option<bool>,
    pub video_conversion_role: Option<bool>,
    pub avatar_last_changed: Option<String>,
    pub folder: Vec<i64>,
    pub email: Option<String>,
}

// ---- list wrappers ------------------------------------------------------
//
// The response keys the transport unwraps. Each is the object Subsonic puts
// beside `status` in the envelope, and each inner list is absent — not
// empty — when there is nothing to return.

/// `getAlbumList2`.
#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
#[serde(default, rename_all = "camelCase")]
pub struct AlbumList2 {
    pub album: Vec<AlbumId3>,
}

/// `getStarred2`. Ignores `size` and `offset`: it returns everything.
#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
#[serde(default, rename_all = "camelCase")]
pub struct Starred2 {
    pub song: Vec<Child>,
    pub album: Vec<AlbumId3>,
    pub artist: Vec<ArtistId3>,
}

/// `getPlaylists`. No paging; returns all of them.
#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
#[serde(default, rename_all = "camelCase")]
pub struct Playlists {
    pub playlist: Vec<Playlist>,
}

/// `getRandomSongs`, `getSongsByGenre`.
#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
#[serde(default, rename_all = "camelCase")]
pub struct Songs {
    pub song: Vec<Child>,
}

/// `getTopSongs`. Last.fm-backed, so `{}` on a server without a key.
#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
#[serde(default, rename_all = "camelCase")]
pub struct TopSongs {
    pub song: Vec<Child>,
}

/// `getSimilarSongs2`. Last.fm-backed, so `{}` on a server without a key.
#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
#[serde(default, rename_all = "camelCase")]
pub struct SimilarSongs2 {
    pub song: Vec<Child>,
}

/// `getNowPlaying`.
#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
#[serde(default, rename_all = "camelCase")]
pub struct NowPlaying {
    pub entry: Vec<NowPlayingEntry>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_empty_collection_is_an_empty_object() {
        // Navidrome answers `topSongs: {}` rather than `topSongs: []` or
        // `{"song": []}` when there is nothing to return.
        let empty: TopSongs = serde_json::from_str("{}").unwrap();
        assert!(empty.song.is_empty());
        let starred: Starred2 = serde_json::from_str("{}").unwrap();
        assert!(starred.song.is_empty() && starred.album.is_empty());
        let playlists: Playlists = serde_json::from_str("{}").unwrap();
        assert!(playlists.playlist.is_empty());
    }

    #[test]
    fn a_song_keeps_the_fields_the_engine_needs() {
        let json = r#"{
            "id": "s1", "title": "Signal Path", "album": "Blue Harvest",
            "artist": "Kestrel", "albumId": "a1", "artistId": "ar1",
            "duration": 9, "suffix": "flac", "contentType": "audio/flac",
            "samplingRate": 44100, "bitDepth": 16, "channelCount": 2,
            "size": 145677, "coverArt": "al-a1", "track": 3, "discNumber": 1,
            "replayGain": {"trackGain": -3.15, "albumGain": -4.02, "trackPeak": 0.98}
        }"#;
        let song: Child = serde_json::from_str(json).unwrap();
        assert_eq!(song.duration, Some(9));
        assert_eq!(song.sampling_rate, Some(44100));
        assert_eq!(song.replay_gain.unwrap().track_gain, Some(-3.15));
        assert!(song.starred.is_none());
        assert!(song.artists.is_empty());
    }

    #[test]
    fn an_album_flattens_into_its_song_list() {
        let json = r#"{"id":"a1","name":"Blue Harvest","artist":"Kestrel",
            "song":[{"id":"s1","title":"Signal Path"}]}"#;
        let album: AlbumWithSongsId3 = serde_json::from_str(json).unwrap();
        assert_eq!(album.album.name, "Blue Harvest");
        assert_eq!(album.song.len(), 1);
        assert_eq!(album.song[0].title, "Signal Path");
    }

    #[test]
    fn a_playlist_flattens_into_its_entries() {
        let json = r#"{"id":"p1","name":"Mix","songCount":2,
            "entry":[{"id":"s1","title":"Approach"},{"id":"s2","title":"Reprise"}]}"#;
        let playlist: PlaylistWithSongs = serde_json::from_str(json).unwrap();
        assert_eq!(playlist.playlist.song_count, Some(2));
        assert_eq!(playlist.entry.len(), 2);
    }

    #[test]
    fn synced_lyrics_carry_millisecond_starts() {
        let json = r#"{"structuredLyrics":[{"lang":"eng","synced":true,
            "line":[{"start":0,"value":"one"},{"start":2500,"value":"two"}]}]}"#;
        let lyrics: LyricsList = serde_json::from_str(json).unwrap();
        let first = &lyrics.structured_lyrics[0];
        assert!(first.synced);
        assert_eq!(first.line[1].start, Some(2500.0));
    }
}
