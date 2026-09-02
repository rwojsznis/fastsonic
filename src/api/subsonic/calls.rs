//! Every call the app makes, in two layers.
//!
//! The first layer is one method per Subsonic endpoint, returning the
//! server's own shapes. The second is the calls `src/backend.rs` makes,
//! returning `api::models` (D5) — that is the layer the `ApiRequest`
//! variants in `migration/01-api-mapping.md` map onto.
//!
//! The awkward parts of the protocol live here rather than in the callers:
//!
//! - **Most list endpoints do not page.** `getStarred2`, `getPlaylist` and
//!   `getArtist` return everything they have, so the `Page<T>` the interface
//!   asks for is cut locally. `getAlbumList2` and `search3` really do page.
//! - **A playlist entry is removed by index, not by id**, so a removal
//!   re-reads the playlist first. Removing while working from a stale view
//!   deletes the wrong row — with two songs of the same name in a playlist,
//!   silently.
//! - **There is no reorder endpoint.** Reordering rewrites the playlist:
//!   `createPlaylist` with the existing `playlistId` and the full ordered
//!   list of song ids, which keeps the id and the name.

use crate::api::models::{Album, Artist, Page, Playlist, PlaylistItem, SearchResults, Track, User};

use super::client::{ApiError, Result, SubsonicClient};
use super::convert;
use super::types::{
    AlbumId3, AlbumList2, ArtistInfo2, ArtistWithAlbumsId3, ArtistsId3, Child, LyricsList,
    NowPlaying, OpenSubsonicExtension, PlaylistWithSongs, Playlists, SearchResult3, Songs,
    Starred2, TopSongs, User as SubsonicUser,
};

/// `getAlbumList2`'s orderings, as far as this app uses them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AlbumListKind {
    /// Recently added to the library.
    Newest,
    /// Most played — driven entirely by what this app scrobbles.
    Frequent,
    /// Most recently played — likewise.
    Recent,
    Random,
    Starred,
    AlphabeticalByName,
    AlphabeticalByArtist,
}

impl AlbumListKind {
    fn as_str(self) -> &'static str {
        match self {
            Self::Newest => "newest",
            Self::Frequent => "frequent",
            Self::Recent => "recent",
            Self::Random => "random",
            Self::Starred => "starred",
            Self::AlphabeticalByName => "alphabeticalByName",
            Self::AlphabeticalByArtist => "alphabeticalByArtist",
        }
    }
}

/// What a `star` or `unstar` call is being asked about. Subsonic names the
/// parameter after the kind, so the caller has to know which it holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum StarKind {
    Song,
    Album,
    Artist,
}

impl StarKind {
    fn parameter(self) -> &'static str {
        match self {
            Self::Song => "id",
            Self::Album => "albumId",
            Self::Artist => "artistId",
        }
    }
}

fn param(name: &'static str, value: impl ToString) -> (&'static str, String) {
    (name, value.to_string())
}

// ---- one method per endpoint --------------------------------------------

impl SubsonicClient {
    /// Whether the server is there and the credential works. The sign-in
    /// screen's "test connection", and the first call of every session.
    pub async fn ping(&self) -> Result<()> {
        self.act("ping", &[]).await
    }

    pub async fn get_user(&self, username: &str) -> Result<SubsonicUser> {
        self.get("getUser", "user", &[param("username", username)])
            .await
    }

    /// What the server admits to supporting. `formPost` and `songLyrics`
    /// are the two this client leans on.
    pub async fn open_subsonic_extensions(&self) -> Result<Vec<OpenSubsonicExtension>> {
        self.get("getOpenSubsonicExtensions", "openSubsonicExtensions", &[])
            .await
    }

    /// Everything starred, in one answer: `size` and `offset` are accepted
    /// and ignored, which was measured, not assumed.
    pub async fn starred(&self) -> Result<Starred2> {
        self.get("getStarred2", "starred2", &[]).await
    }

    pub async fn album_list(
        &self,
        kind: AlbumListKind,
        size: u32,
        offset: u32,
    ) -> Result<Vec<AlbumId3>> {
        let list: AlbumList2 = self
            .get(
                "getAlbumList2",
                "albumList2",
                &[
                    param("type", kind.as_str()),
                    param("size", size),
                    param("offset", offset),
                ],
            )
            .await?;
        Ok(list.album)
    }

    /// The whole artist index. Album artists only: a performer who appears
    /// on no album of their own has a working artist page but is not in
    /// here.
    pub async fn artists(&self) -> Result<ArtistsId3> {
        self.get("getArtists", "artists", &[]).await
    }

    pub async fn get_artist(&self, id: &str) -> Result<ArtistWithAlbumsId3> {
        self.get("getArtist", "artist", &[param("id", id)]).await
    }

    /// Biography, images and similar artists. Navidrome fills the biography
    /// and the similar artists from Last.fm, so on a server with no key
    /// configured — the ordinary self-hosted case — only the images come
    /// back (D11).
    pub async fn artist_info(&self, id: &str, count: u32) -> Result<ArtistInfo2> {
        self.get(
            "getArtistInfo2",
            "artistInfo2",
            &[param("id", id), param("count", count)],
        )
        .await
    }

    pub async fn get_album(&self, id: &str) -> Result<PlaylistOrAlbumSongs> {
        self.get("getAlbum", "album", &[param("id", id)]).await
    }

    pub async fn get_song(&self, id: &str) -> Result<Child> {
        self.get("getSong", "song", &[param("id", id)]).await
    }

    /// Takes the artist's **name**, not its id — the one endpoint that does.
    /// Last.fm-backed, so empty without a key.
    pub async fn top_songs(&self, artist_name: &str, count: u32) -> Result<Vec<Child>> {
        let songs: TopSongs = self
            .get(
                "getTopSongs",
                "topSongs",
                &[param("artist", artist_name), param("count", count)],
            )
            .await?;
        Ok(songs.song)
    }

    pub async fn random_songs(&self, size: u32) -> Result<Vec<Child>> {
        let songs: Songs = self
            .get("getRandomSongs", "randomSongs", &[param("size", size)])
            .await?;
        Ok(songs.song)
    }

    /// An empty query matches the entire library, so callers must guard the
    /// empty search box rather than let it pull everything.
    pub async fn search3(&self, query: &str, count: u32, offset: u32) -> Result<SearchResult3> {
        self.get(
            "search3",
            "searchResult3",
            &[
                param("query", query),
                param("artistCount", count),
                param("artistOffset", offset),
                param("albumCount", count),
                param("albumOffset", offset),
                param("songCount", count),
                param("songOffset", offset),
            ],
        )
        .await
    }

    pub async fn get_playlists(&self) -> Result<Vec<super::types::Playlist>> {
        let playlists: Playlists = self.get("getPlaylists", "playlists", &[]).await?;
        Ok(playlists.playlist)
    }

    /// A playlist and **every** entry it has; there is no paging to ask for.
    pub async fn get_playlist(&self, id: &str) -> Result<PlaylistWithSongs> {
        self.get("getPlaylist", "playlist", &[param("id", id)])
            .await
    }

    /// Creates a playlist, or — with `playlist_id` — replaces the contents
    /// of one that exists, which is how reordering is done.
    async fn write_playlist(
        &self,
        playlist_id: Option<&str>,
        name: Option<&str>,
        song_ids: &[String],
    ) -> Result<PlaylistWithSongs> {
        let mut params = Vec::with_capacity(song_ids.len() + 2);
        if let Some(id) = playlist_id {
            params.push(param("playlistId", id));
        }
        if let Some(name) = name {
            params.push(param("name", name));
        }
        params.extend(song_ids.iter().map(|id| param("songId", id)));
        self.post_for("createPlaylist", "playlist", &params).await
    }

    pub async fn update_playlist_details(
        &self,
        id: &str,
        name: Option<&str>,
        comment: Option<&str>,
        public: Option<bool>,
    ) -> Result<()> {
        let mut params = vec![param("playlistId", id)];
        if let Some(name) = name {
            params.push(param("name", name));
        }
        if let Some(comment) = comment {
            params.push(param("comment", comment));
        }
        if let Some(public) = public {
            params.push(param("public", public));
        }
        self.post("updatePlaylist", &params).await
    }

    pub async fn delete_playlist(&self, id: &str) -> Result<()> {
        self.act("deletePlaylist", &[param("id", id)]).await
    }

    async fn star(&self, kind: StarKind, id: &str, starred: bool) -> Result<()> {
        let endpoint = if starred { "star" } else { "unstar" };
        self.act(endpoint, &[param(kind.parameter(), id)]).await
    }

    /// Tells the server what is being played. `submission=false` is
    /// now-playing; `submission=true` is the play itself, and the only
    /// thing that moves a play count. `time` is milliseconds since the
    /// epoch, and Navidrome's "recent" and "frequent" shelves are built
    /// from nothing else.
    pub async fn scrobble(&self, id: &str, time_ms: Option<u64>, submission: bool) -> Result<()> {
        let mut params = vec![param("id", id), param("submission", submission)];
        if let Some(time) = time_ms {
            params.push(param("time", time));
        }
        self.act("scrobble", &params).await
    }

    /// Who else is listening, and where this app's own playback is showing
    /// up. `c=fastsonic` is the name that appears.
    pub async fn now_playing(&self) -> Result<NowPlaying> {
        self.get("getNowPlaying", "nowPlaying", &[]).await
    }

    /// Lyrics from the server, synced where the library has an `.lrc`
    /// beside the file. LRCLIB stays as the fallback for everything else.
    pub async fn lyrics(&self, song_id: &str) -> Result<LyricsList> {
        self.get("getLyricsBySongId", "lyricsList", &[param("id", song_id)])
            .await
    }

    // ---- URLs for bytes rather than JSON --------------------------------

    /// The audio itself, raw: no `format`, so the server sends the file it
    /// has, with `Accept-Ranges: bytes` for seeking (D12).
    pub fn stream_url(&self, song_id: &str) -> Result<String> {
        self.url("stream", &[param("id", song_id)])
    }

    /// Cover art at a size. This URL carries the credential, so it is never
    /// logged and never stored — `src/images.rs` builds it at fetch time
    /// from the deferred request `convert::art_url` left in the model.
    pub fn cover_art_url(&self, cover_art_id: &str, size: u32) -> Result<String> {
        self.url(
            "getCoverArt",
            &[param("id", cover_art_id), param("size", size)],
        )
    }
}

/// `getAlbum` answers with an album that has its songs inside it.
pub type PlaylistOrAlbumSongs = super::types::AlbumWithSongsId3;

// ---- the calls the app makes --------------------------------------------

impl SubsonicClient {
    /// `ApiRequest::Me`. A ping first, so an unreachable server is reported
    /// as unreachable rather than as a strange user record.
    pub async fn me(&self) -> Result<User> {
        let username = self
            .credentials()
            .map(|credentials| credentials.username)
            .ok_or(ApiError::NotSignedIn)?;
        self.ping().await?;
        match self.get_user(&username).await {
            Ok(user) if !user.username.is_empty() => Ok(convert::user(&user)),
            // Not every Subsonic server lets an ordinary account read its
            // own record. The name is already known, so this is not a
            // failure worth showing anyone.
            Ok(_) | Err(_) => Ok(User {
                id: username.clone(),
                display_name: Some(username),
                ..User::default()
            }),
        }
    }

    /// `ApiRequest::SavedTracks`. `getStarred2` returns every starred song
    /// at once, so the page is cut here and the total is exact.
    pub async fn saved_tracks(&self, offset: u32, limit: u32) -> Result<Page<Track>> {
        let starred = self.starred().await?;
        let tracks: Vec<Track> = starred.song.iter().map(convert::track).collect();
        Ok(convert::slice(&tracks, offset, limit))
    }

    /// `ApiRequest::SavedAlbums`. `getAlbumList2 type=starred` pages
    /// properly, unlike `getStarred2`, so this one asks the server.
    pub async fn saved_albums(&self, offset: u32, limit: u32) -> Result<Page<Album>> {
        let albums = self
            .album_list(AlbumListKind::Starred, limit, offset)
            .await?;
        Ok(convert::page(
            albums.iter().map(convert::album).collect(),
            offset,
            limit,
        ))
    }

    /// `ApiRequest::FollowedArtists`. There is nothing to follow on a server
    /// you own, so this is the starred artists.
    pub async fn saved_artists(&self, offset: u32, limit: u32) -> Result<Page<Artist>> {
        let starred = self.starred().await?;
        let artists: Vec<Artist> = starred.artist.iter().map(convert::artist).collect();
        Ok(convert::slice(&artists, offset, limit))
    }

    /// Every artist in the library, for the artists page.
    pub async fn all_artists(&self, offset: u32, limit: u32) -> Result<Page<Artist>> {
        let index = self.artists().await?;
        let artists: Vec<Artist> = index
            .index
            .iter()
            .flat_map(|letter| letter.artist.iter())
            .map(convert::artist)
            .collect();
        Ok(convert::slice(&artists, offset, limit))
    }

    /// `ApiRequest::SetSaved`. The parameter is named after the kind, so
    /// the URI says which call to make.
    pub async fn set_saved(&self, uris: &[String], saved: bool) -> Result<()> {
        for uri in uris {
            let Some((kind, id)) = convert::parse_uri(uri) else {
                log::warn!("cannot star an unrecognised uri");
                continue;
            };
            let kind = match kind {
                convert::Kind::Track => StarKind::Song,
                convert::Kind::Album => StarKind::Album,
                convert::Kind::Artist => StarKind::Artist,
                // A playlist is owned or it is not, and the starred list
                // is not an object; there is nothing to star in either.
                convert::Kind::Playlist | convert::Kind::Collection => continue,
            };
            self.star(kind, id, saved).await?;
        }
        Ok(())
    }

    /// `ApiRequest::MyPlaylists`. Returns all of them; the page is local.
    pub async fn my_playlists(&self, offset: u32, limit: u32) -> Result<Page<Playlist>> {
        let playlists: Vec<Playlist> = self
            .get_playlists()
            .await?
            .iter()
            .map(convert::playlist)
            .collect();
        Ok(convert::slice(&playlists, offset, limit))
    }

    /// `ApiRequest::Playlist`.
    pub async fn playlist(&self, id: &str) -> Result<Playlist> {
        let playlist = self.get_playlist(id).await?;
        Ok(convert::playlist(&playlist.playlist))
    }

    /// `ApiRequest::PlaylistItems` and `PlaylistSample`, which are the same
    /// call with a different slice.
    pub async fn playlist_items(
        &self,
        id: &str,
        offset: u32,
        limit: u32,
    ) -> Result<Page<PlaylistItem>> {
        let playlist = self.get_playlist(id).await?;
        let items = convert::playlist_items(&playlist.entry);
        Ok(convert::slice(&items, offset, limit))
    }

    /// `ApiRequest::CreatePlaylist`. Subsonic creates a playlist with its
    /// songs; the name is all `createPlaylist` takes, so the description
    /// and visibility follow in a second call.
    pub async fn create_playlist(
        &self,
        name: &str,
        public: bool,
        description: &str,
    ) -> Result<Playlist> {
        let created = self.write_playlist(None, Some(name), &[]).await?;
        let id = created.playlist.id.clone();
        if public || !description.is_empty() {
            self.update_playlist_details(
                &id,
                None,
                (!description.is_empty()).then_some(description),
                public.then_some(true),
            )
            .await?;
            return self.playlist(&id).await;
        }
        Ok(convert::playlist(&created.playlist))
    }

    /// `ApiRequest::UpdatePlaylist`. Subsonic has a comment where Spotify
    /// had a description, and no image of its own.
    pub async fn update_playlist(
        &self,
        id: &str,
        name: Option<&str>,
        description: Option<&str>,
        public: Option<bool>,
    ) -> Result<()> {
        self.update_playlist_details(id, name, description, public)
            .await
    }

    /// `ApiRequest::AddToPlaylist`.
    pub async fn add_to_playlist(&self, id: &str, uris: &[String]) -> Result<()> {
        let mut params = vec![param("playlistId", id)];
        params.extend(
            uris.iter()
                .filter_map(|uri| convert::id_of(uri, convert::Kind::Track))
                .map(|id| param("songIdToAdd", id)),
        );
        if params.len() == 1 {
            return Ok(());
        }
        self.post("updatePlaylist", &params).await
    }

    /// `ApiRequest::RemoveFromPlaylist`. Removal is by index, so the
    /// playlist is re-read first and the indices are computed from what the
    /// server has *now*: working from a stale view removes the wrong row,
    /// and with two songs of the same name in a playlist it does so
    /// silently. Every occurrence of a URI goes, which is what the
    /// interface already did optimistically.
    ///
    /// Returns the playlist as it stands afterwards.
    pub async fn remove_from_playlist(&self, id: &str, uris: &[String]) -> Result<Playlist> {
        let current = self.get_playlist(id).await?;
        let doomed: Vec<&str> = uris
            .iter()
            .filter_map(|uri| convert::id_of(uri, convert::Kind::Track))
            .collect();
        let mut params = vec![param("playlistId", id)];
        params.extend(
            current
                .entry
                .iter()
                .enumerate()
                .filter(|(_, song)| doomed.contains(&song.id.as_str()))
                .map(|(index, _)| param("songIndexToRemove", index)),
        );
        if params.len() > 1 {
            self.post("updatePlaylist", &params).await?;
        }
        self.playlist(id).await
    }

    /// `ApiRequest::ReorderPlaylist`. There is no reorder endpoint, so the
    /// whole playlist is rewritten in the new order — the same
    /// `playlistId`, the same name, the full ordered list of song ids. It
    /// keeps the id and moves `changed`. The rewrite is racy against
    /// another editor, which is a price worth paying on a server with one
    /// user; the read immediately beforehand is what keeps it honest.
    pub async fn reorder_playlist(
        &self,
        id: &str,
        range_start: u32,
        insert_before: u32,
    ) -> Result<Playlist> {
        let current = self.get_playlist(id).await?;
        let mut song_ids: Vec<String> = current.entry.iter().map(|song| song.id.clone()).collect();
        if !reorder(&mut song_ids, range_start as usize, insert_before as usize) {
            return Ok(convert::playlist(&current.playlist));
        }
        let name = current.playlist.name.clone();
        let rewritten = self
            .write_playlist(Some(id), Some(&name), &song_ids)
            .await?;
        Ok(convert::playlist(&rewritten.playlist))
    }

    /// `ApiRequest::Search`. An empty query matches the whole library, so
    /// it is answered with nothing instead of being sent.
    pub async fn search(&self, query: &str, offset: u32) -> Result<SearchResults> {
        let limit = self.search_limit();
        if query.trim().is_empty() {
            return Ok(SearchResults::default());
        }
        let results = self.search3(query, limit, offset).await?;
        Ok(convert::search_results(&results, offset, limit))
    }

    /// `ApiRequest::Artist`. The artist page wants an image and a biography
    /// as well as the name, and those are a second call.
    pub async fn artist(&self, id: &str) -> Result<Artist> {
        let artist = self.get_artist(id).await?;
        let mut converted = convert::artist_with_albums(&artist);
        if let Ok(info) = self.artist_info(id, 0).await {
            converted.images = convert::info_images(&info, converted.images);
        }
        Ok(converted)
    }

    /// `ApiRequest::ArtistAlbums`. The albums came with the artist, so this
    /// is the same call and a local slice. There is no album-group filter:
    /// singles and compilations are not a separate query.
    pub async fn artist_albums(&self, id: &str, offset: u32, limit: u32) -> Result<Page<Album>> {
        let artist = self.get_artist(id).await?;
        let albums: Vec<Album> = artist.album.iter().map(convert::album).collect();
        Ok(convert::slice(&albums, offset, limit))
    }

    /// `ApiRequest::ArtistTopTracks`. Last.fm-backed: on a server with no
    /// key this is empty, which is the ordinary case and not an error.
    pub async fn artist_top_tracks(&self, id: &str, count: u32) -> Result<Vec<Track>> {
        let artist = self.get_artist(id).await?;
        let songs = self.top_songs(&artist.name, count).await?;
        Ok(songs.iter().map(convert::track).collect())
    }

    /// `ApiRequest::RelatedArtists`. Last.fm-backed, and empty for the same
    /// reason.
    pub async fn related_artists(&self, id: &str, count: u32) -> Result<Vec<Artist>> {
        let info = self.artist_info(id, count).await?;
        Ok(info.similar_artist.iter().map(convert::artist).collect())
    }

    /// `ApiRequest::Album`, with its tracks already filled in, because the
    /// server sends them together and a second call would only be a way of
    /// throwing them away.
    pub async fn album(&self, id: &str) -> Result<Album> {
        let album = self.get_album(id).await?;
        let mut converted = convert::album(&album.album);
        let tracks: Vec<Track> = album.song.iter().map(convert::track).collect();
        converted.tracks = Some(convert::slice(&tracks, 0, 0));
        Ok(converted)
    }

    /// `ApiRequest::AlbumTracks`.
    pub async fn album_tracks(&self, id: &str, offset: u32, limit: u32) -> Result<Page<Track>> {
        let album = self.get_album(id).await?;
        let tracks: Vec<Track> = album.song.iter().map(convert::track).collect();
        Ok(convert::slice(&tracks, offset, limit))
    }

    /// `ApiRequest::Track`.
    pub async fn track(&self, id: &str) -> Result<Track> {
        Ok(convert::track(&self.get_song(id).await?))
    }

    // ---- what Home is made of now ---------------------------------------
    //
    // Recommendations, Discover and Made-for-you have no equivalent and are
    // cut. What is left is the library's own shape: what arrived recently,
    // what gets played, what was starred, and something at random. The
    // "recent" and "frequent" shelves are built from what this app
    // scrobbles and from nothing else, so they are empty until it does.

    pub async fn newest_albums(&self, count: u32) -> Result<Vec<Album>> {
        self.shelf(AlbumListKind::Newest, count).await
    }

    pub async fn frequent_albums(&self, count: u32) -> Result<Vec<Album>> {
        self.shelf(AlbumListKind::Frequent, count).await
    }

    pub async fn recent_albums(&self, count: u32) -> Result<Vec<Album>> {
        self.shelf(AlbumListKind::Recent, count).await
    }

    pub async fn random_albums(&self, count: u32) -> Result<Vec<Album>> {
        self.shelf(AlbumListKind::Random, count).await
    }

    async fn shelf(&self, kind: AlbumListKind, count: u32) -> Result<Vec<Album>> {
        let albums = self.album_list(kind, count, 0).await?;
        Ok(albums.iter().map(convert::album).collect())
    }

    /// A shelf of songs to start from when nothing else is known yet.
    pub async fn random_tracks(&self, count: u32) -> Result<Vec<Track>> {
        let songs = self.random_songs(count).await?;
        Ok(songs.iter().map(convert::track).collect())
    }
}

/// Moves one entry, in the coordinates `ApiRequest::ReorderPlaylist` uses:
/// `insert_before` is the index the entry should sit *before* in the list as
/// it was, so moving down by one is `to == from + 2`. Returns whether
/// anything moved.
fn reorder(ids: &mut Vec<String>, from: usize, insert_before: usize) -> bool {
    if from >= ids.len() || insert_before > ids.len() {
        return false;
    }
    let target = if insert_before > from {
        insert_before - 1
    } else {
        insert_before
    };
    if target == from {
        return false;
    }
    let moved = ids.remove(from);
    ids.insert(target.min(ids.len()), moved);
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ids(names: &[&str]) -> Vec<String> {
        names.iter().map(|name| (*name).to_string()).collect()
    }

    #[test]
    fn moving_a_row_down_puts_it_after_its_neighbour() {
        // What the "Move down" menu item sends: from 0, insert_before 2.
        let mut list = ids(&["a", "b", "c"]);
        assert!(reorder(&mut list, 0, 2));
        assert_eq!(list, ids(&["b", "a", "c"]));
    }

    #[test]
    fn moving_a_row_up_puts_it_before_its_neighbour() {
        let mut list = ids(&["a", "b", "c"]);
        assert!(reorder(&mut list, 2, 1));
        assert_eq!(list, ids(&["a", "c", "b"]));
    }

    #[test]
    fn a_move_to_where_it_already_is_changes_nothing() {
        let mut list = ids(&["a", "b", "c"]);
        assert!(!reorder(&mut list, 1, 1));
        assert!(!reorder(&mut list, 1, 2));
        assert_eq!(list, ids(&["a", "b", "c"]));
    }

    #[test]
    fn a_move_off_the_end_is_refused_rather_than_panicking() {
        let mut list = ids(&["a", "b"]);
        assert!(!reorder(&mut list, 5, 0));
        assert!(!reorder(&mut list, 0, 9));
        assert_eq!(list, ids(&["a", "b"]));
    }

    #[test]
    fn moving_a_row_to_the_end_is_allowed() {
        let mut list = ids(&["a", "b", "c"]);
        assert!(reorder(&mut list, 0, 3));
        assert_eq!(list, ids(&["b", "c", "a"]));
    }
}
