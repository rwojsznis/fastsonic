//! Tests that need a real server, run against `migration/devserver`.
//!
//! Every one of them is `#[ignore]`d: CI has no Navidrome, and a test that
//! silently passes when the thing it tests is absent is worse than no test.
//! Run them deliberately:
//!
//! ```sh
//! (cd migration/devserver && ./make-library.sh && docker compose up -d)
//! cargo test --lib subsonic::live -- --ignored --test-threads=1
//! ```
//!
//! `FASTSONIC_TEST_SERVER`, `FASTSONIC_TEST_USER` and
//! `FASTSONIC_TEST_PASSWORD` point them at a different server. They read the
//! library and they write playlists whose names start with `fastsonic-test-`;
//! they never touch anything else, and they clean up after themselves.

use std::sync::Arc;

use super::calls::AlbumListKind;
use super::client::SubsonicClient;
use super::convert;
use crate::api::activity::NetActivity;
use crate::api::subsonic::Credentials;

const TEST_PLAYLIST_PREFIX: &str = "fastsonic-test-";

fn client() -> SubsonicClient {
    let server = std::env::var("FASTSONIC_TEST_SERVER")
        .unwrap_or_else(|_| "http://localhost:4533".to_string());
    let username = std::env::var("FASTSONIC_TEST_USER").unwrap_or_else(|_| "admin".to_string());
    let password =
        std::env::var("FASTSONIC_TEST_PASSWORD").unwrap_or_else(|_| "fastsonic".to_string());
    let client = SubsonicClient::new(
        crate::http_client_builder().build().unwrap(),
        Arc::new(NetActivity::default()),
        20,
    );
    client.set_credentials(Some(Credentials::from_password(
        &server, &username, &password,
    )));
    client
}

#[tokio::test]
#[ignore = "needs migration/devserver"]
async fn a_derived_credential_authenticates() {
    let client = client();
    client.ping().await.unwrap();
    let me = client.me().await.unwrap();
    assert!(!me.id.is_empty());
}

#[tokio::test]
#[ignore = "needs migration/devserver"]
async fn a_wrong_password_is_told_apart_from_an_unreachable_server() {
    let client = client();
    let server = client.credentials().unwrap().server;

    client.set_credentials(Some(Credentials::from_password(
        &server,
        "admin",
        "not-the-password",
    )));
    let error = client.ping().await.unwrap_err();
    assert!(error.is_auth(), "{error:?}");

    client.set_credentials(Some(Credentials::from_password(
        "http://127.0.0.1:4",
        "admin",
        "x",
    )));
    let error = client.ping().await.unwrap_err();
    assert!(
        matches!(error, super::client::ApiError::Network(_)),
        "{error:?}"
    );
}

#[tokio::test]
#[ignore = "needs migration/devserver"]
async fn a_web_server_that_is_not_subsonic_says_so() {
    let client = client();
    // The Navidrome UI's own root: reachable, HTML, not an envelope.
    let error = client
        .get("noSuchEndpoint", "anything", &[])
        .await
        .map(|_: super::types::Songs| ())
        .unwrap_err();
    assert!(
        matches!(error, super::client::ApiError::NotSubsonic(_)),
        "{error:?}"
    );
}

#[tokio::test]
#[ignore = "needs migration/devserver"]
async fn the_library_reads_end_to_end() {
    let client = client();

    let newest = client.newest_albums(10).await.unwrap();
    assert!(!newest.is_empty(), "the fixture library has six albums");
    let album = client.album(&newest[0].id).await.unwrap();
    let tracks = album.tracks.expect("an album arrives with its songs");
    assert!(!tracks.items.is_empty());
    assert_eq!(tracks.total, tracks.items.len() as u32);

    let first = &tracks.items[0];
    assert!(first.duration_ms >= 1000, "seconds became milliseconds");
    assert!(first.uri.starts_with("sonic:track:"));
    assert!(!first.artists.is_empty());

    // The same song, fetched on its own, is the same song.
    let alone = client.track(first.id.as_ref().unwrap()).await.unwrap();
    assert_eq!(alone.name, first.name);
    assert_eq!(alone.duration_ms, first.duration_ms);

    // Its artwork is a request, not a URL, and carries no credential.
    let art = first.image(300).expect("a cover");
    let (size, art_id) = convert::parse_art_url(art).expect("a deferred art request");
    assert_eq!(size, 300);
    let real = client.cover_art_url(art_id, size).unwrap();
    assert!(real.contains("getCoverArt.view"));
    assert!(!art.contains(&client.credentials().unwrap().token));
}

#[tokio::test]
#[ignore = "needs migration/devserver"]
async fn an_artist_page_loads_and_its_lastfm_half_is_empty_not_broken() {
    let client = client();
    let newest = client.newest_albums(1).await.unwrap();
    let artist_id = newest[0].artists[0]
        .id
        .clone()
        .expect("an album names its artist");

    let artist = client.artist(&artist_id).await.unwrap();
    assert!(!artist.name.is_empty());
    let albums = client.artist_albums(&artist_id, 0, 50).await.unwrap();
    assert!(!albums.items.is_empty());

    // No Last.fm key on the development server, and none on the ordinary
    // self-hosted one either: these answer, and they answer with nothing.
    assert!(
        client
            .artist_top_tracks(&artist_id, 5)
            .await
            .unwrap()
            .is_empty()
    );
    assert!(
        client
            .related_artists(&artist_id, 5)
            .await
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
#[ignore = "needs migration/devserver"]
async fn search_finds_by_name_and_an_empty_query_is_not_sent() {
    let client = client();
    let results = client.search("signal", 0).await.unwrap();
    assert!(!results.is_empty());
    let songs = results.tracks.unwrap();
    assert!(
        songs
            .items
            .iter()
            .any(|track| track.name.to_lowercase().contains("signal"))
            || !songs.items.is_empty()
    );
    // search3 has no playlist bucket at all.
    assert!(results.playlists.is_none());

    // An empty query matches the whole library, so it never leaves here.
    assert!(client.search("   ", 0).await.unwrap().is_empty());
}

#[tokio::test]
#[ignore = "needs migration/devserver"]
async fn starring_a_song_puts_it_in_saved_tracks_and_unstarring_takes_it_out() {
    let client = client();
    let songs = client.random_tracks(1).await.unwrap();
    let uri = songs[0].uri.clone();

    client
        .set_saved(std::slice::from_ref(&uri), true)
        .await
        .unwrap();
    let saved = client.saved_tracks(0, 50).await.unwrap();
    assert!(saved.items.iter().any(|saved| saved.track.uri == uri));

    client
        .set_saved(std::slice::from_ref(&uri), false)
        .await
        .unwrap();
    let saved = client.saved_tracks(0, 50).await.unwrap();
    assert!(!saved.items.iter().any(|saved| saved.track.uri == uri));
}

/// The hearts on a page: the starred flag rides on the object, so a page
/// that has loaded its rows needs no second call to know which hearts are
/// filled (P4.2). `false` is *stated* rather than left unknown, which is
/// what lets `App::note_saved` trust it.
#[tokio::test]
#[ignore = "needs migration/devserver"]
async fn a_starred_flag_arrives_on_the_object() {
    let client = client();
    let song = client.random_tracks(1).await.unwrap().remove(0);
    let id = song.id.clone().expect("a song id");
    let uri = song.uri.clone();
    // Whatever the library holds, this test puts it back.
    let was = client.track(&id).await.unwrap().starred == Some(true);

    client
        .set_saved(std::slice::from_ref(&uri), false)
        .await
        .unwrap();
    assert_eq!(
        client.track(&id).await.unwrap().starred,
        Some(false),
        "an unstarred song says so rather than leaving it unknown"
    );
    client
        .set_saved(std::slice::from_ref(&uri), true)
        .await
        .unwrap();
    assert_eq!(client.track(&id).await.unwrap().starred, Some(true));
    if !was {
        client
            .set_saved(std::slice::from_ref(&uri), false)
            .await
            .unwrap();
    }

    // And on the other two kinds, which the same map is keyed by.
    let album = client.newest_albums(1).await.unwrap().remove(0);
    assert!(client.album(&album.id).await.unwrap().starred.is_some());
    let artist = client.all_artists(0, 1).await.unwrap().items.remove(0);
    assert!(client.artist(&artist.id).await.unwrap().starred.is_some());
}

#[tokio::test]
#[ignore = "needs migration/devserver"]
async fn a_playlist_can_be_made_added_to_reordered_and_removed_from() {
    let client = client();
    let name = format!("{TEST_PLAYLIST_PREFIX}lifecycle");
    remove_test_playlists(&client, &name).await;

    let created = client
        .create_playlist(&name, false, "written by a test")
        .await
        .unwrap();
    assert_eq!(created.name, name);
    assert!(created.uri.starts_with("sonic:playlist:"));

    let songs = client.random_tracks(3).await.unwrap();
    assert_eq!(songs.len(), 3, "the fixture library has twenty songs");
    let uris: Vec<String> = songs.iter().map(|track| track.uri.clone()).collect();
    client.add_to_playlist(&created.id, &uris).await.unwrap();

    let items = client.playlist_items(&created.id, 0, 50).await.unwrap();
    assert_eq!(items.total, 3);
    let order: Vec<String> = items
        .items
        .iter()
        .map(|item| item.playable().unwrap().uri().to_string())
        .collect();
    assert_eq!(order, uris);

    // Reorder rewrites the playlist. It must keep the id and the name.
    let reordered = client.reorder_playlist(&created.id, 0, 2).await.unwrap();
    assert_eq!(reordered.id, created.id);
    assert_eq!(reordered.name, name);
    let after = client.playlist_items(&created.id, 0, 50).await.unwrap();
    let order: Vec<String> = after
        .items
        .iter()
        .map(|item| item.playable().unwrap().uri().to_string())
        .collect();
    assert_eq!(
        order,
        vec![uris[1].clone(), uris[0].clone(), uris[2].clone()]
    );

    // Paging a playlist is local slicing, since the server sends it whole.
    let page = client.playlist_items(&created.id, 1, 1).await.unwrap();
    assert_eq!(page.items.len(), 1);
    assert_eq!(page.total, 3);
    assert_eq!(page.next_offset(), Some(2));

    let left = client
        .remove_from_playlist(&created.id, &[uris[0].clone()])
        .await
        .unwrap();
    assert_eq!(left.track_total(), 2);

    client.delete_playlist(&created.id).await.unwrap();
    let mine = client.my_playlists(0, 100).await.unwrap();
    assert!(!mine.items.iter().any(|playlist| playlist.name == name));
}

#[tokio::test]
#[ignore = "needs migration/devserver"]
async fn removing_a_duplicate_removes_every_copy_the_way_the_interface_showed() {
    // The fixture library has two songs called "Reprise" for this: index
    // arithmetic done against a stale view deletes the wrong row, and the
    // interface has already removed every matching row optimistically.
    let client = client();
    let name = format!("{TEST_PLAYLIST_PREFIX}duplicates");
    remove_test_playlists(&client, &name).await;

    let created = client.create_playlist(&name, false, "").await.unwrap();
    let songs = client.random_tracks(2).await.unwrap();
    let doomed = songs[0].uri.clone();
    let keeper = songs[1].uri.clone();
    client
        .add_to_playlist(
            &created.id,
            &[doomed.clone(), keeper.clone(), doomed.clone()],
        )
        .await
        .unwrap();
    assert_eq!(
        client
            .playlist_items(&created.id, 0, 50)
            .await
            .unwrap()
            .total,
        3
    );

    client
        .remove_from_playlist(&created.id, std::slice::from_ref(&doomed))
        .await
        .unwrap();
    let left = client.playlist_items(&created.id, 0, 50).await.unwrap();
    let uris: Vec<String> = left
        .items
        .iter()
        .map(|item| item.playable().unwrap().uri().to_string())
        .collect();
    assert_eq!(uris, vec![keeper]);

    client.delete_playlist(&created.id).await.unwrap();
}

#[tokio::test]
#[ignore = "needs migration/devserver"]
async fn a_long_playlist_is_written_by_form_post_not_by_url() {
    // Navidrome itself accepted a 30 KB GET; anything in front of it would
    // not. This is the one call whose parameters cannot fit in a URL.
    let client = client();
    let name = format!("{TEST_PLAYLIST_PREFIX}long");
    remove_test_playlists(&client, &name).await;

    let created = client.create_playlist(&name, false, "").await.unwrap();
    let songs = client.random_tracks(20).await.unwrap();
    let many: Vec<String> = std::iter::repeat_n(songs.iter(), 30)
        .flatten()
        .map(|track| track.uri.clone())
        .collect();
    assert!(many.len() > 500);
    client.add_to_playlist(&created.id, &many).await.unwrap();

    let items = client.playlist_items(&created.id, 0, 1000).await.unwrap();
    assert_eq!(items.total as usize, many.len());

    // And the rewrite path, which sends every id again.
    let reordered = client.reorder_playlist(&created.id, 0, 3).await.unwrap();
    assert_eq!(reordered.track_total() as usize, many.len());

    client.delete_playlist(&created.id).await.unwrap();
}

#[tokio::test]
#[ignore = "needs migration/devserver"]
async fn scrobbling_a_song_makes_it_recent_and_frequent() {
    let client = client();
    let songs = client.random_tracks(1).await.unwrap();
    let id = songs[0].id.clone().unwrap();
    let album_id = songs[0].album.as_ref().unwrap().id.clone();

    // Now-playing first, then the play itself. Only the second one counts.
    client.scrobble(&id, None, false).await.unwrap();
    let now = client.now_playing().await.unwrap();
    assert!(
        now.entry.iter().any(|entry| entry.child.id == id
            && entry.player_name.as_deref() == Some(super::client::CLIENT_NAME)),
        "the server reports our own client name back"
    );

    client
        .scrobble(&id, Some(epoch_millis()), true)
        .await
        .unwrap();
    let recent = client.recent_albums(20).await.unwrap();
    assert!(
        recent.iter().any(|album| album.id == album_id),
        "the recent shelf is built from what we scrobble and nothing else"
    );
    assert!(!client.frequent_albums(20).await.unwrap().is_empty());
}

#[tokio::test]
#[ignore = "needs migration/devserver"]
async fn lyrics_come_from_the_server_synced_where_the_library_has_them() {
    let client = client();
    let results = client.search("second signal", 0).await.unwrap();
    let songs = results.tracks.unwrap().items;
    assert!(!songs.is_empty(), "the MP3 fixture album has .lrc sidecars");

    let mut synced = 0;
    for song in &songs {
        let list = client.lyrics(song.id.as_ref().unwrap()).await.unwrap();
        for entry in &list.structured_lyrics {
            if entry.synced {
                synced += 1;
                assert!(entry.line.iter().any(|line| line.start.is_some()));
            }
        }
    }
    assert!(synced > 0, "one of the fixture's sidecars is synced");
}

#[tokio::test]
#[ignore = "needs migration/devserver"]
async fn the_stream_url_serves_the_file_itself_with_ranges() {
    let client = client();
    let songs = client.random_tracks(1).await.unwrap();
    let url = client.stream_url(songs[0].id.as_ref().unwrap()).unwrap();
    assert!(!url.contains("format="), "raw, so that seeking works (D12)");

    let http = crate::http_client_builder().build().unwrap();
    let head = http
        .get(&url)
        .header("Range", "bytes=0-1023")
        .send()
        .await
        .unwrap();
    assert_eq!(head.status().as_u16(), 206);
    assert!(head.headers().contains_key("content-range"));
    let bytes = head.bytes().await.unwrap();
    assert_eq!(bytes.len(), 1024);
}

#[tokio::test]
#[ignore = "needs migration/devserver"]
async fn the_extensions_the_client_leans_on_are_advertised() {
    let client = client();
    let names: Vec<String> = client
        .open_subsonic_extensions()
        .await
        .unwrap()
        .into_iter()
        .map(|extension| extension.name)
        .collect();
    for needed in ["formPost", "songLyrics"] {
        assert!(names.contains(&needed.to_string()), "{names:?}");
    }
    // The one that decided how credentials are stored: it is not there.
    assert!(
        !names.contains(&"apiKeyAuthentication".to_string()),
        "{names:?}"
    );
}

#[tokio::test]
#[ignore = "needs migration/devserver"]
async fn starred_ignores_paging_so_the_client_does_it() {
    let client = client();
    let songs = client.random_tracks(3).await.unwrap();
    for song in &songs {
        client
            .set_saved(std::slice::from_ref(&song.uri), true)
            .await
            .unwrap();
    }

    let raw = client.starred().await.unwrap();
    assert!(raw.song.len() >= 3);
    let page = client.saved_tracks(0, 2).await.unwrap();
    assert_eq!(page.items.len(), 2, "the page is cut here, not there");
    assert_eq!(page.total as usize, raw.song.len());
    assert_eq!(page.next_offset(), Some(2));

    // The date the Liked Songs table sorts by. `getStarred2` is the only
    // call that reports it, and the migration notes had it recorded as
    // absent until it was looked for.
    assert!(
        page.items.iter().all(|saved| saved.added_at.is_some()),
        "a starred song knows when it was starred"
    );
    let dates: Vec<&str> = page
        .items
        .iter()
        .filter_map(|saved| saved.added_at.as_deref())
        .collect();
    assert!(dates.windows(2).all(|pair| pair[0] >= pair[1]), "{dates:?}");
    // The three just starred are the three most recent, so they are the
    // front of the list rather than somewhere in it.
    let front: Vec<String> = client
        .saved_tracks(0, 3)
        .await
        .unwrap()
        .items
        .iter()
        .map(|saved| saved.track.uri.clone())
        .collect();
    for song in &songs {
        assert!(front.contains(&song.uri), "{front:?}");
    }

    for song in &songs {
        client
            .set_saved(std::slice::from_ref(&song.uri), false)
            .await
            .unwrap();
    }
}

/// What this whole arrangement exists for. `getStarred2` is the entire
/// starred library every time it is asked, so a page of Liked Songs that
/// asked for it was downloading the library to show fifty rows of it — and
/// sorting the table, which loads every page first, did that once per page.
#[tokio::test]
#[ignore = "needs migration/devserver"]
async fn paging_liked_songs_asks_the_server_once() {
    let client = client();
    let songs = client.random_tracks(3).await.unwrap();
    for song in &songs {
        client
            .set_saved(std::slice::from_ref(&song.uri), true)
            .await
            .unwrap();
    }
    let activity = client.activity();

    // The first page of a listing is the one that asks.
    let before = activity.made();
    let first = client.saved_tracks(0, 1).await.unwrap();
    assert_eq!(activity.made() - before, 1);
    assert_eq!(first.items.len(), 1);

    // The rest of it is cut from that same answer.
    let after_first = activity.made();
    for offset in 1..4 {
        client.saved_tracks(offset, 1).await.unwrap();
    }
    assert_eq!(
        activity.made() - after_first,
        0,
        "a later page must not re-download the starred library"
    );

    // Starting the listing over does ask again: that is what a reload and
    // what re-opening the page both do.
    let after_pages = activity.made();
    client.saved_tracks(0, 1).await.unwrap();
    assert_eq!(activity.made() - after_pages, 1);

    // And starring something makes the remembered answer wrong, so the
    // next page of any listing goes back to the server.
    client
        .set_saved(std::slice::from_ref(&songs[0].uri), false)
        .await
        .unwrap();
    let after_unstar = activity.made();
    client.saved_tracks(1, 1).await.unwrap();
    assert_eq!(
        activity.made() - after_unstar,
        1,
        "what is starred changed, so the list is asked for again"
    );

    for song in &songs {
        client
            .set_saved(std::slice::from_ref(&song.uri), false)
            .await
            .unwrap();
    }
}

#[tokio::test]
#[ignore = "needs migration/devserver"]
async fn the_album_list_really_does_page() {
    let client = client();
    let first = client
        .album_list(AlbumListKind::Newest, 2, 0)
        .await
        .unwrap();
    let second = client
        .album_list(AlbumListKind::Newest, 2, 2)
        .await
        .unwrap();
    assert_eq!(first.len(), 2);
    assert!(!second.is_empty());
    assert!(
        first
            .iter()
            .all(|album| !second.iter().any(|other| other.id == album.id)),
        "offset returns a disjoint page"
    );
}

#[tokio::test]
#[ignore = "needs migration/devserver"]
async fn cover_art_is_fetched_and_a_missing_one_is_not_cached_as_a_picture() {
    let client = client();
    let songs = client.random_tracks(1).await.unwrap();
    let art = songs[0].image(300).expect("a cover").to_string();

    let cache = std::env::temp_dir().join(format!("fastsonic-live-art-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&cache);
    let loader = crate::images::ArtLoader::new(
        crate::http_client_builder().build().unwrap(),
        tokio::runtime::Handle::current(),
        cache.clone(),
    );
    loader.set_credentials(client.credentials());

    let bytes = loader.fetch(&art).await.unwrap();
    assert!(bytes.len() > 100);
    assert!(
        image::load_from_memory(&bytes).is_ok(),
        "what came back decodes as a picture"
    );

    // The trap: an id the server does not know answers HTTP 200 with an
    // error envelope. It must be reported, and it must not reach the disk
    // cache — a cached envelope would fail to decode for ever after.
    let missing = convert::art_url("no-such-cover", 300);
    let error = loader.fetch(&missing).await.unwrap_err();
    assert!(
        error.contains("artwork") || error.contains("not a picture"),
        "{error}"
    );
    let files = std::fs::read_dir(&cache).unwrap().count();
    assert_eq!(files, 1, "only the real cover was written");

    let _ = std::fs::remove_dir_all(&cache);
}

// ---- the native API -----------------------------------------------------

fn native() -> super::native::NativeClient {
    super::native::NativeClient::new(
        crate::http_client_builder().build().unwrap(),
        Arc::new(NetActivity::default()),
    )
}

fn credentials_from_env() -> (String, String, String) {
    (
        std::env::var("FASTSONIC_TEST_SERVER")
            .unwrap_or_else(|_| "http://localhost:4533".to_string()),
        std::env::var("FASTSONIC_TEST_USER").unwrap_or_else(|_| "admin".to_string()),
        std::env::var("FASTSONIC_TEST_PASSWORD").unwrap_or_else(|_| "fastsonic".to_string()),
    )
}

#[tokio::test]
#[ignore = "needs migration/devserver"]
async fn signing_in_yields_a_credential_that_is_not_the_password() {
    // The identity D10 rests on: the pair /auth/login hands back is exactly
    // md5(password + salt), and it authenticates /rest/ on its own.
    let (server, username, password) = credentials_from_env();
    let signed_in = native()
        .sign_in(&server, &username, &password)
        .await
        .unwrap();

    assert_eq!(
        signed_in.credentials.token,
        super::auth::salted_token(&password, &signed_in.credentials.salt)
    );
    assert!(!signed_in.session.token.is_empty());

    let client = client();
    client.set_credentials(Some(signed_in.credentials.clone()));
    client.ping().await.unwrap();
}

#[tokio::test]
#[ignore = "needs migration/devserver"]
async fn a_wrong_password_is_refused_and_a_plain_subsonic_server_has_no_native_api() {
    let (server, username, _) = credentials_from_env();
    let native = native();
    let error = native
        .sign_in(&server, &username, "not-the-password")
        .await
        .unwrap_err();
    assert!(
        matches!(error, super::native::NativeError::Rejected(_)),
        "{error:?}"
    );
    assert!(!error.is_unavailable(), "a refusal is worth showing");

    // The Subsonic endpoint root is not /auth/login: standing in for a
    // server that speaks Subsonic and nothing else.
    let error = native
        .sign_in(&format!("{server}/rest"), &username, "x")
        .await
        .unwrap_err();
    assert!(error.is_unavailable(), "{error:?}");
}

#[tokio::test]
#[ignore = "needs migration/devserver"]
async fn the_native_api_answers_the_three_questions_subsonic_cannot() {
    let (server, username, password) = credentials_from_env();
    let native = native();
    let signed_in = native.sign_in(&server, &username, &password).await.unwrap();
    native.set_session(Some(signed_in.session));
    assert!(native.available());

    // Make sure there is history to read, whatever ran before this.
    let client = client();
    client.set_credentials(Some(signed_in.credentials));
    let songs = client.random_tracks(1).await.unwrap();
    let id = songs[0].id.clone().unwrap();
    client
        .scrobble(&id, Some(epoch_millis()), true)
        .await
        .unwrap();

    let recent = native.recently_played(0, 10).await.unwrap();
    assert!(
        !recent.is_empty(),
        "track-level history, which Subsonic has none of"
    );
    assert!(
        recent
            .iter()
            .any(|track| track.id.as_deref() == Some(id.as_str()))
    );
    assert!(recent[0].duration_ms > 0);

    let top = native.top_tracks(0, 10).await.unwrap();
    assert!(!top.is_empty());
    let artists = native.top_artists(0, 10).await.unwrap();
    assert!(!artists.is_empty());
    assert!(!artists[0].name.is_empty());
    // Nothing in an answer points anywhere but at the user's own server.
    assert!(
        artists
            .iter()
            .flat_map(|artist| artist.images.iter())
            .all(|image| image.url.starts_with("sonic:art:"))
    );
}

#[tokio::test]
#[ignore = "needs migration/devserver"]
async fn without_a_session_the_native_api_says_nothing_to_show() {
    let native = native();
    let error = native.recently_played(0, 10).await.unwrap_err();
    assert!(error.is_unavailable(), "{error:?}");

    // A token the server never issued is the same answer: the session is
    // dropped so nothing keeps retrying with it.
    let (server, _, _) = credentials_from_env();
    native.set_session(Some(super::native::NativeSession {
        server,
        token: "not.a.jwt".to_string(),
    }));
    let error = native.top_tracks(0, 10).await.unwrap_err();
    assert!(error.is_unavailable(), "{error:?}");
    assert!(!native.available(), "a refused token is not kept");
}

fn epoch_millis() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|since| since.as_millis() as u64)
        .unwrap_or_default()
}

/// Leaves no test playlist behind from an earlier interrupted run.
async fn remove_test_playlists(client: &SubsonicClient, name: &str) {
    let Ok(playlists) = client.get_playlists().await else {
        return;
    };
    for playlist in playlists {
        if playlist.name == name {
            let _ = client.delete_playlist(&playlist.id).await;
        }
    }
}
