//! Navidrome's own API, for the three questions Subsonic cannot answer.
//!
//! Subsonic knows what is starred and what is in the library; it does not
//! know what *you* have listened to. There is no track-level play history,
//! no "your top songs" and no "your top artists" in the protocol — the
//! nearest thing, `getTopSongs`, is per artist and Last.fm-backed. Navidrome
//! answers all three from its own database, so Home, the top-songs shelf and
//! the Recents tab survive instead of being cut (D7, D11).
//!
//! Everything that depends on this server being Navidrome specifically is in
//! this file, so the degraded path is obvious: against Gonic, or any other
//! Subsonic server, [`NativeClient::sign_in`] fails and those three sections
//! are empty. They must read as *this server does not keep that*, not as a
//! broken page.
//!
//! ## The session
//!
//! `POST /auth/login` takes the password once and answers with three things:
//! a JWT for `/api/*`, and the `subsonicSalt` / `subsonicToken` pair that
//! authenticates `/rest/` for good. The pair is what gets stored (D10); the
//! password is not.
//!
//! The JWT is the awkward one. It expires — twenty-four hours after issue on
//! a default Navidrome (`ND_SESSIONTIMEOUT`) — and the server re-issues it
//! through an `X-Nd-Authorization` response header, which every call here
//! reads back into the stored token. So an app opened regularly keeps its
//! session indefinitely, and an app left closed for longer loses it. When
//! that happens there is no way to get another without the password, which
//! is deliberately not stored: the personalisation sections go empty until
//! the next sign-in, and nothing else is affected. See D13.

use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::api::activity::{ActivityGuard, NetActivity};
use crate::api::models::{Album, Artist, ArtistRef, Track};

use super::auth::{Credentials, normalize_server};
use super::convert;

/// Navidrome's re-issued token comes back in this header, on every call.
const TOKEN_HEADER: &str = "x-nd-authorization";

#[derive(Clone, Debug, thiserror::Error)]
pub enum NativeError {
    /// No session yet, or the one there was has run out. The caller shows
    /// an empty section, not an error.
    #[error("this server keeps no listening history for us yet")]
    NoSession,
    /// A server that answered, but not as Navidrome. Gonic and friends.
    #[error("this server does not have Navidrome's own API")]
    NotNavidrome,
    #[error("{0}")]
    Rejected(String),
    #[error("network error: {0}")]
    Network(String),
    #[error("unexpected response from the server: {0}")]
    Decode(String),
}

impl NativeError {
    /// Whether this simply means "nothing to show", which is the ordinary
    /// state on a server that is not Navidrome or after a session lapses.
    pub fn is_unavailable(&self) -> bool {
        matches!(self, Self::NoSession | Self::NotNavidrome)
    }
}

pub type Result<T> = std::result::Result<T, NativeError>;

/// The JWT for `/api/*`, and which server it belongs to. Persisted beside
/// the Subsonic credential; expiring, and replaceable only by signing in.
#[derive(Clone, Default, Deserialize, Serialize, PartialEq, Eq)]
pub struct NativeSession {
    pub server: String,
    pub token: String,
}

impl std::fmt::Debug for NativeSession {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("NativeSession")
            .field("server", &self.server)
            .field("token", &"<redacted>")
            .finish()
    }
}

/// What `POST /auth/login` gives back: everything needed for both APIs.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
struct LoginAnswer {
    token: String,
    subsonic_salt: String,
    subsonic_token: String,
    username: String,
    name: String,
}

/// A completed sign-in: the credential to keep, and the session to use
/// until it lapses.
#[derive(Clone, Debug)]
pub struct SignIn {
    pub credentials: Credentials,
    pub session: NativeSession,
    /// The display name the server has for this account, where it has one.
    pub display_name: String,
}

pub struct NativeClient {
    http: reqwest::Client,
    session: Mutex<Option<NativeSession>>,
    activity: Arc<NetActivity>,
}

impl NativeClient {
    pub fn new(http: reqwest::Client, activity: Arc<NetActivity>) -> Self {
        Self {
            http,
            session: Mutex::new(None),
            activity,
        }
    }

    pub fn set_session(&self, session: Option<NativeSession>) {
        *self.session.lock().unwrap_or_else(|p| p.into_inner()) = session;
    }

    pub fn session(&self) -> Option<NativeSession> {
        self.session
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .clone()
    }

    /// Whether there is a session to ask with, so callers can skip a
    /// request they know will fail.
    pub fn available(&self) -> bool {
        self.session()
            .is_some_and(|session| !session.token.is_empty())
    }

    /// Exchanges the password for both credentials. The one call in the app
    /// that a password passes through.
    pub async fn sign_in(&self, server: &str, username: &str, password: &str) -> Result<SignIn> {
        let server = normalize_server(server);
        let response = self
            .http
            .post(format!("{server}/auth/login"))
            .json(&serde_json::json!({ "username": username, "password": password }))
            .send()
            .await
            .map_err(|error| NativeError::Network(error.to_string()))?;

        let status = response.status();
        let body = response
            .text()
            .await
            .map_err(|error| NativeError::Network(error.to_string()))?;
        if status == reqwest::StatusCode::UNAUTHORIZED {
            return Err(NativeError::Rejected(
                "wrong username or password".to_string(),
            ));
        }
        if !status.is_success() {
            // A Subsonic server that is not Navidrome has no /auth/login at
            // all, which is a 404 rather than a refusal.
            return Err(NativeError::NotNavidrome);
        }
        let answer: LoginAnswer =
            serde_json::from_str(&body).map_err(|_| NativeError::NotNavidrome)?;
        if answer.subsonic_token.is_empty() {
            return Err(NativeError::NotNavidrome);
        }
        Ok(SignIn {
            credentials: Credentials::from_pair(
                &server,
                username,
                &answer.subsonic_salt,
                &answer.subsonic_token,
            ),
            session: NativeSession {
                server,
                token: answer.token,
            },
            display_name: if answer.name.is_empty() {
                answer.username
            } else {
                answer.name
            },
        })
    }

    /// One `/api/` call. Reads the re-issued token out of the response
    /// before anything else, so that a long-running app keeps its session.
    async fn get(&self, path: &str, params: &[(&str, String)]) -> Result<Vec<Value>> {
        let session = self
            .session()
            .filter(|session| !session.token.is_empty())
            .ok_or(NativeError::NoSession)?;

        self.activity.begin();
        let _activity = ActivityGuard(&self.activity);
        let response = self
            .http
            .get(format!("{}/api/{path}", session.server))
            .header(TOKEN_HEADER, format!("Bearer {}", session.token))
            .query(params)
            .send()
            .await
            .map_err(|error| NativeError::Network(error.to_string()))?;

        if let Some(reissued) = response
            .headers()
            .get(TOKEN_HEADER)
            .and_then(|value| value.to_str().ok())
            .map(|value| value.trim_start_matches("Bearer ").to_string())
            .filter(|token| !token.is_empty() && *token != session.token)
        {
            self.set_session(Some(NativeSession {
                server: session.server.clone(),
                token: reissued,
            }));
        }

        let status = response.status();
        if status == reqwest::StatusCode::UNAUTHORIZED {
            // Twenty-four hours since the last sign-in, or a server that
            // never issued this token. Either way there is no way back
            // without the password, which is not kept.
            log::info!(
                "the server's own session has expired; personalisation is empty until the next sign-in"
            );
            self.set_session(None);
            return Err(NativeError::NoSession);
        }
        if status == reqwest::StatusCode::NOT_FOUND {
            return Err(NativeError::NotNavidrome);
        }
        if !status.is_success() {
            return Err(NativeError::Rejected(format!("HTTP {}", status.as_u16())));
        }

        let body = response
            .text()
            .await
            .map_err(|error| NativeError::Network(error.to_string()))?;
        serde_json::from_str(&body).map_err(|error| NativeError::Decode(error.to_string()))
    }

    fn window(offset: u32, limit: u32) -> Vec<(&'static str, String)> {
        vec![
            ("_start", offset.to_string()),
            ("_end", (offset + limit).to_string()),
        ]
    }

    /// Track-level play history, newest first — the Recents tab and the
    /// Home shelf. Rows past the last played one carry `playDate: null`, and
    /// the filter that would have removed them server-side does not work, so
    /// the list stops at the first unplayed row.
    pub async fn recently_played(&self, offset: u32, limit: u32) -> Result<Vec<Track>> {
        let mut params = Self::window(offset, limit);
        params.push(("_sort", "playDate".to_string()));
        params.push(("_order", "DESC".to_string()));
        let songs = self.get("song", &params).await?;
        Ok(songs
            .iter()
            .take_while(|song| !song["playDate"].is_null())
            .map(track)
            .collect())
    }

    /// Your most played songs. Unplayed rows sort last and are dropped for
    /// the same reason.
    pub async fn top_tracks(&self, offset: u32, limit: u32) -> Result<Vec<Track>> {
        let mut params = Self::window(offset, limit);
        params.push(("_sort", "playCount".to_string()));
        params.push(("_order", "DESC".to_string()));
        let songs = self.get("song", &params).await?;
        Ok(songs
            .iter()
            .take_while(|song| song["playCount"].as_i64().unwrap_or_default() > 0)
            .map(track)
            .collect())
    }

    /// Your most played artists.
    pub async fn top_artists(&self, offset: u32, limit: u32) -> Result<Vec<Artist>> {
        let mut params = Self::window(offset, limit);
        params.push(("_sort", "playCount".to_string()));
        params.push(("_order", "DESC".to_string()));
        let artists = self.get("artist", &params).await?;
        Ok(artists
            .iter()
            .take_while(|artist| artist["playCount"].as_i64().unwrap_or_default() > 0)
            .map(artist)
            .collect())
    }
}

fn text(value: &Value, key: &str) -> String {
    value[key].as_str().unwrap_or_default().to_string()
}

fn optional(value: &Value, key: &str) -> Option<String> {
    value[key]
        .as_str()
        .filter(|text| !text.is_empty())
        .map(str::to_string)
}

/// A native-API song, in the app's vocabulary.
///
/// Two differences from the Subsonic shape, both worth knowing:
/// `duration` is a **float** in seconds here, so the millisecond the
/// Subsonic integer threw away is back; and there is no `coverArt` id, but
/// Navidrome accepts a bare `albumId` as one, which is what the artwork
/// request is built from.
fn track(song: &Value) -> Track {
    let id = text(song, "id");
    let album_id = text(song, "albumId");
    let artist_id = optional(song, "artistId");
    let album_name = text(song, "album");
    Track {
        id: (!id.is_empty()).then(|| id.clone()),
        name: text(song, "title"),
        uri: convert::track_uri(&id),
        duration_ms: (song["duration"].as_f64().unwrap_or_default() * 1000.0).max(0.0) as u32,
        explicit: song["explicitStatus"].as_str() == Some("explicit"),
        artists: vec![ArtistRef {
            id: artist_id.clone(),
            name: text(song, "artist"),
            uri: artist_id.as_deref().map(convert::artist_uri),
        }],
        album: (!album_id.is_empty() || !album_name.is_empty()).then(|| Album {
            id: album_id.clone(),
            name: album_name,
            uri: convert::album_uri(&album_id),
            images: convert::art_images_for(&album_id),
            release_date: song["year"]
                .as_i64()
                .filter(|year| *year > 0)
                .map(|year| year.to_string()),
            ..Album::default()
        }),
        track_number: song["trackNumber"].as_u64().map(|number| number as u32),
        disc_number: song["discNumber"].as_u64().map(|number| number as u32),
        is_local: false,
        is_playable: Some(true),
        popularity: None,
        external_urls: Default::default(),
        starred: song["starred"].as_bool(),
    }
}

/// A native-API artist, in the app's vocabulary.
///
/// The image URLs this endpoint returns are *not* used: unlike Subsonic's
/// `artistImageUrl`, which the server proxies, these point straight at an
/// external image CDN, and this app does not fetch from anywhere but the
/// user's own server. The artwork request is built from the artist id
/// instead, which Navidrome serves through `getCoverArt` like any other.
fn artist(artist: &Value) -> Artist {
    let id = text(artist, "id");
    Artist {
        name: text(artist, "name"),
        uri: convert::artist_uri(&id),
        images: convert::art_images_for(&id),
        genres: Vec::new(),
        followers: None,
        popularity: None,
        external_urls: Default::default(),
        starred: artist["starred"].as_bool(),
        id,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::models::pick_image;

    fn song() -> Value {
        serde_json::json!({
            "id": "4JzXNrkZk04PWxzTBgYXFW",
            "title": "Opener",
            "album": "Fastsonic Sampler",
            "albumId": "7f45yPTAMk0VAp25JNQbUh",
            "artist": "Blue Harvest",
            "artistId": "6j1iXoRgVulaaWUzHovlgx",
            "trackNumber": 1,
            "discNumber": 0,
            "year": 2024,
            "duration": 7.02,
            "playCount": 1,
            "playDate": "2026-09-02T06:03:17.677Z",
            "explicitStatus": ""
        })
    }

    /// Rule: the native API's float duration keeps the precision the
    /// Subsonic integer threw away.
    #[test]
    fn a_native_song_keeps_its_fractional_seconds() {
        assert_eq!(track(&song()).duration_ms, 7_020);
    }

    /// Rule: a native song has no cover-art id, so the artwork request is
    /// built from the album id — which Navidrome accepts as one.
    #[test]
    fn artwork_is_built_from_the_album_id() {
        let track = track(&song());
        let art = track.image(300).expect("a cover");
        assert_eq!(
            convert::parse_art_url(art),
            Some((300, "7f45yPTAMk0VAp25JNQbUh"))
        );
    }

    /// Rule: nothing in this app fetches from anywhere but the user's own
    /// server. The native API offers external CDN image URLs; they are not
    /// taken.
    #[test]
    fn an_external_image_url_is_never_used() {
        let value = serde_json::json!({
            "id": "6j1iXoRgVulaaWUzHovlgx",
            "name": "Blue Harvest",
            "playCount": 3,
            "smallImageUrl": "https://cdn-images.dzcdn.net/images/artist/x/250x250.jpg",
            "mediumImageUrl": "https://cdn-images.dzcdn.net/images/artist/x/500x500.jpg"
        });
        let artist = artist(&value);
        assert!(
            !artist
                .images
                .iter()
                .any(|image| image.url.contains("dzcdn"))
        );
        assert_eq!(
            pick_image(&artist.images, 300),
            Some("sonic:art:300:6j1iXoRgVulaaWUzHovlgx")
        );
    }

    /// Rule: a lapsed session and a server that never had this API are both
    /// "nothing to show", not errors to put in front of anyone.
    #[test]
    fn an_absent_session_is_not_a_failure_to_report() {
        assert!(NativeError::NoSession.is_unavailable());
        assert!(NativeError::NotNavidrome.is_unavailable());
        assert!(!NativeError::Network("refused".into()).is_unavailable());
    }

    /// Rule: the token is never printed, even by a debug format.
    #[test]
    fn the_session_token_is_not_printable() {
        let session = NativeSession {
            server: "http://host:4533".into(),
            token: "ey.secret.jwt".into(),
        };
        let printed = format!("{session:?}");
        assert!(printed.contains("http://host:4533"));
        assert!(!printed.contains("secret"));
    }

    /// Rule: a window is `_start` and `_end`, and `_end` is exclusive.
    #[test]
    fn a_page_is_a_half_open_window() {
        let window = NativeClient::window(20, 10);
        assert_eq!(window[0], ("_start", "20".to_string()));
        assert_eq!(window[1], ("_end", "30".to_string()));
    }
}
