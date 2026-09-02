//! The transport to the music server: one `GET` per call, a Subsonic
//! envelope back.
//!
//! Three things about this protocol decide the shape of everything here, and
//! all three were measured against Navidrome 0.63.2 rather than read off the
//! specification:
//!
//! 1. **A failure is `HTTP 200`.** `status: "failed"` and an `error.code`
//!    live inside the body. Reading the status line instead of the envelope
//!    is the most common way to get a Subsonic client wrong.
//! 2. **The converse is not true.** An unknown endpoint is a real `404` with
//!    a plain-text body, so a non-2xx or a non-JSON answer must be reported
//!    as "this is not a Subsonic server" rather than fed to serde.
//! 3. **The credential is in the query string**, so no URL may reach a log
//!    without passing through [`auth::redacted`].
//!
//! There is no rate limiting and no `Retry-After` handling: those existed to
//! survive Spotify and are dead against your own server (D8). The
//! concurrency cap stays — a Raspberry Pi serving FLAC will thank you.

use std::sync::{Arc, Mutex};
use std::time::Instant;

use serde::de::DeserializeOwned;
use serde_json::{Map, Value};
use thiserror::Error;
use tokio::sync::Semaphore;

use super::auth::{self, Credentials};
use crate::api::activity::{ActivityGuard, NetActivity};

/// The protocol version this client claims. 1.16.1 is the last Subsonic
/// release and what every OpenSubsonic server implements.
pub const API_VERSION: &str = "1.16.1";
/// The `c=` parameter. It is what `getNowPlaying` shows other clients, and
/// what Navidrome files play history under, so it is a name, not a detail.
pub const CLIENT_NAME: &str = "fastsonic";

const MAX_IN_FLIGHT: usize = 6;

// Subsonic error codes worth telling apart. The rest are reported verbatim.
const CODE_MISSING_PARAMETER: i32 = 10;
const CODE_CLIENT_TOO_OLD: i32 = 20;
const CODE_SERVER_TOO_OLD: i32 = 30;
const CODE_WRONG_CREDENTIALS: i32 = 40;
const CODE_TOKEN_AUTH_UNSUPPORTED: i32 = 41;
const CODE_AUTH_MECHANISM_UNSUPPORTED: i32 = 42;
const CODE_CONFLICTING_AUTH: i32 = 43;
const CODE_INVALID_API_KEY: i32 = 44;
const CODE_NOT_AUTHORIZED: i32 = 50;
const CODE_NOT_FOUND: i32 = 70;

#[derive(Clone, Debug, Error)]
pub enum ApiError {
    #[error("not signed in")]
    NotSignedIn,
    /// The credential was refused. The sign-in screen words this one itself.
    #[error("{0}")]
    Unauthorized(String),
    /// The server understood the call and refused or could not answer it.
    #[error("{message}")]
    Server { code: i32, message: String },
    /// Reachable, but not speaking Subsonic — a wrong URL far more often
    /// than a broken server.
    #[error("{0} does not answer as a Subsonic server")]
    NotSubsonic(String),
    #[error("network error: {0}")]
    Network(String),
    #[error("unexpected response from the server: {0}")]
    Decode(String),
}

impl ApiError {
    /// The Subsonic error code, where the server gave one.
    pub fn code(&self) -> Option<i32> {
        match self {
            Self::Server { code, .. } => Some(*code),
            _ => None,
        }
    }

    /// Whether the credential is the problem, so the caller knows to send
    /// the user back to the sign-in screen rather than retry.
    pub fn is_auth(&self) -> bool {
        matches!(self, Self::NotSignedIn | Self::Unauthorized(_))
    }

    /// Whether the thing asked for is simply not there, which several
    /// callers treat as an empty page rather than an error.
    pub fn is_not_found(&self) -> bool {
        self.code() == Some(CODE_NOT_FOUND)
    }

    fn from_envelope(code: i32, message: String) -> Self {
        match code {
            CODE_WRONG_CREDENTIALS => Self::Unauthorized(message),
            CODE_TOKEN_AUTH_UNSUPPORTED => Self::Unauthorized(format!(
                "{message} — this account cannot use token authentication"
            )),
            CODE_AUTH_MECHANISM_UNSUPPORTED | CODE_CONFLICTING_AUTH | CODE_INVALID_API_KEY => {
                Self::Unauthorized(message)
            }
            CODE_NOT_AUTHORIZED => Self::Unauthorized(format!(
                "{message} — this account is not allowed to do that"
            )),
            CODE_CLIENT_TOO_OLD => Self::Server {
                code,
                message: format!("{message} — this server speaks a newer Subsonic protocol"),
            },
            CODE_SERVER_TOO_OLD => Self::Server {
                code,
                message: format!("{message} — this server is too old for Fastsonic"),
            },
            CODE_MISSING_PARAMETER | CODE_NOT_FOUND => Self::Server { code, message },
            _ => Self::Server { code, message },
        }
    }
}

impl From<reqwest::Error> for ApiError {
    fn from(error: reqwest::Error) -> Self {
        if error.is_decode() {
            Self::Decode(error.to_string())
        } else {
            Self::Network(error.to_string())
        }
    }
}

pub type Result<T> = std::result::Result<T, ApiError>;

/// How a call's parameters travel.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Transport {
    /// A `GET` with everything in the query string. Every read.
    Query,
    /// An `application/x-www-form-urlencoded` `POST`, the `formPost`
    /// extension. Rewriting a long playlist is the one call whose parameters
    /// will not fit in a URL: Navidrome itself accepted a 30 KB one, but
    /// 8 KB is the common default in front of it.
    Form,
}

pub struct SubsonicClient {
    http: reqwest::Client,
    credentials: Mutex<Option<Credentials>>,
    limiter: Semaphore,
    activity: Arc<NetActivity>,
    /// How many of each kind `search3` asks for.
    search_limit: u32,
}

impl SubsonicClient {
    pub fn new(http: reqwest::Client, activity: Arc<NetActivity>, search_limit: u32) -> Self {
        Self {
            http,
            credentials: Mutex::new(None),
            limiter: Semaphore::new(MAX_IN_FLIGHT),
            activity,
            search_limit,
        }
    }

    pub fn set_credentials(&self, credentials: Option<Credentials>) {
        *self.credentials.lock().unwrap_or_else(|p| p.into_inner()) = credentials;
    }

    pub fn credentials(&self) -> Option<Credentials> {
        self.credentials
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .clone()
    }

    fn require_credentials(&self) -> Result<Credentials> {
        self.credentials()
            .filter(|credentials| !credentials.is_empty())
            .ok_or(ApiError::NotSignedIn)
    }

    pub fn search_limit(&self) -> u32 {
        self.search_limit
    }

    /// The busy indicator's counter, shared with everything else that talks
    /// to the network, so a second client built for a sign-in attempt shows
    /// up in the same place.
    pub fn activity(&self) -> Arc<NetActivity> {
        Arc::clone(&self.activity)
    }

    // ---- URL building ----------------------------------------------------

    /// The full URL for an endpoint, credential and all. Public because the
    /// audio engine and the artwork loader fetch bytes rather than JSON and
    /// need the same URL this transport would have built.
    pub fn url(&self, endpoint: &str, params: &[(&str, String)]) -> Result<String> {
        let credentials = self.require_credentials()?;
        Ok(build_url(&credentials, endpoint, params))
    }

    // ---- transport -------------------------------------------------------

    /// Sends one call and returns the envelope's payload: everything beside
    /// `status`, `version`, `type`, `serverVersion`, `openSubsonic` and
    /// `error`.
    async fn send(
        &self,
        endpoint: &str,
        params: &[(&str, String)],
        transport: Transport,
    ) -> Result<Map<String, Value>> {
        let credentials = self.require_credentials()?;
        let started = Instant::now();

        let permit = self
            .limiter
            .acquire()
            .await
            .map_err(|_| ApiError::NotSignedIn)?;
        self.activity.begin();
        let _activity = ActivityGuard(&self.activity);

        let request = match transport {
            Transport::Query => self.http.get(build_url(&credentials, endpoint, params)),
            Transport::Form => self
                .http
                .post(format!("{}/rest/{endpoint}.view", credentials.server))
                .form(&form_body(&credentials, params)),
        };
        let response = request.send().await?;
        let status = response.status();
        let content_type = response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .unwrap_or_default()
            .to_string();
        let body = response.text().await?;
        drop(permit);

        log::debug!(
            "server request endpoint={endpoint} status={} duration_ms={}",
            status.as_u16(),
            started.elapsed().as_millis()
        );

        if !status.is_success() || !looks_like_json(&content_type, &body) {
            // A `404` with `404 page not found` is what an endpoint this
            // server does not have looks like — and what any non-Subsonic
            // web server answers for `/rest/…`.
            return Err(ApiError::NotSubsonic(format!(
                "{} (HTTP {})",
                credentials.server,
                status.as_u16()
            )));
        }
        parse_envelope(&body)
    }

    /// One call whose answer is a single named object in the envelope.
    /// A missing key is the empty value, not an error: a Navidrome with
    /// nothing to say leaves the key out entirely.
    pub(super) async fn get<T: DeserializeOwned + Default>(
        &self,
        endpoint: &str,
        key: &str,
        params: &[(&str, String)],
    ) -> Result<T> {
        let mut payload = self.send(endpoint, params, Transport::Query).await?;
        take(&mut payload, key)
    }

    /// A call whose answer is only whether it worked.
    pub(super) async fn act(&self, endpoint: &str, params: &[(&str, String)]) -> Result<()> {
        self.send(endpoint, params, Transport::Query).await?;
        Ok(())
    }

    /// A change whose parameters may be too long for a URL.
    pub(super) async fn post(&self, endpoint: &str, params: &[(&str, String)]) -> Result<()> {
        self.send(endpoint, params, Transport::Form).await?;
        Ok(())
    }

    /// A change that posts and reads its answer back, so the caller sees
    /// what the server actually stored.
    pub(super) async fn post_for<T: DeserializeOwned + Default>(
        &self,
        endpoint: &str,
        key: &str,
        params: &[(&str, String)],
    ) -> Result<T> {
        let mut payload = self.send(endpoint, params, Transport::Form).await?;
        take(&mut payload, key)
    }
}

/// The parameters every call carries, in front of its own.
fn common_params(credentials: &Credentials) -> Vec<(&'static str, String)> {
    let mut params: Vec<(&'static str, String)> = credentials.params().into();
    params.push(("v", API_VERSION.to_string()));
    params.push(("c", CLIENT_NAME.to_string()));
    params.push(("f", "json".to_string()));
    params
}

fn build_url(credentials: &Credentials, endpoint: &str, params: &[(&str, String)]) -> String {
    let mut url = format!("{}/rest/{endpoint}.view?", credentials.server);
    let mut first = true;
    for (name, value) in common_params(credentials)
        .iter()
        .map(|(name, value)| (*name, value.as_str()))
        .chain(params.iter().map(|(name, value)| (*name, value.as_str())))
    {
        if !first {
            url.push('&');
        }
        first = false;
        url.push_str(&urlencoding::encode(name));
        url.push('=');
        url.push_str(&urlencoding::encode(value));
    }
    url
}

fn form_body(credentials: &Credentials, params: &[(&str, String)]) -> Vec<(String, String)> {
    common_params(credentials)
        .into_iter()
        .map(|(name, value)| (name.to_string(), value))
        .chain(
            params
                .iter()
                .map(|(name, value)| ((*name).to_string(), value.clone())),
        )
        .collect()
}

/// Whether a body is worth handing to serde. The content type decides when
/// there is one, because `getCoverArt` proves a `200` can carry XML where
/// bytes were expected; the first non-space character decides otherwise.
fn looks_like_json(content_type: &str, body: &str) -> bool {
    if content_type.contains("json") {
        return true;
    }
    if !content_type.is_empty() && !content_type.starts_with("text/plain") {
        return false;
    }
    body.trim_start().starts_with('{')
}

/// Unwraps `{"subsonic-response": {...}}`, turning `status: "failed"` into
/// an error and everything else into the payload.
fn parse_envelope(body: &str) -> Result<Map<String, Value>> {
    let mut envelope: Map<String, Value> =
        serde_json::from_str(body).map_err(|error| ApiError::Decode(error.to_string()))?;
    let response = envelope
        .remove("subsonic-response")
        .ok_or_else(|| ApiError::Decode("no subsonic-response in the answer".to_string()))?;
    let Value::Object(mut response) = response else {
        return Err(ApiError::Decode(
            "subsonic-response was not an object".to_string(),
        ));
    };

    let ok = response
        .get("status")
        .and_then(Value::as_str)
        .is_some_and(|status| status == "ok");
    if !ok {
        let error = response.remove("error").unwrap_or(Value::Null);
        let code = error
            .get("code")
            .and_then(Value::as_i64)
            .map_or(0, |code| code as i32);
        let message = error
            .get("message")
            .and_then(Value::as_str)
            .filter(|message| !message.is_empty())
            .unwrap_or("the server refused the request")
            .to_string();
        return Err(ApiError::from_envelope(code, message));
    }

    for key in [
        "status",
        "version",
        "type",
        "serverVersion",
        "openSubsonic",
        "error",
    ] {
        response.remove(key);
    }
    Ok(response)
}

/// Reads one named object out of a payload, defaulting when it is absent.
fn take<T: DeserializeOwned + Default>(payload: &mut Map<String, Value>, key: &str) -> Result<T> {
    match payload.remove(key) {
        None | Some(Value::Null) => Ok(T::default()),
        Some(value) => {
            serde_json::from_value(value).map_err(|error| ApiError::Decode(error.to_string()))
        }
    }
}

/// Re-exported so callers that log a URL cannot forget where the redaction
/// lives.
pub use auth::redacted;

#[cfg(test)]
mod tests {
    use super::*;

    fn credentials() -> Credentials {
        Credentials::from_pair("http://host:4533", "admin", "abc123", "deadbeef")
    }

    #[test]
    fn every_call_carries_the_protocol_parameters() {
        let url = build_url(&credentials(), "getAlbum", &[("id", "al-1".to_string())]);
        assert!(url.starts_with("http://host:4533/rest/getAlbum.view?"));
        for expected in [
            "u=admin",
            "t=deadbeef",
            "s=abc123",
            "v=1.16.1",
            "c=fastsonic",
            "f=json",
            "id=al-1",
        ] {
            assert!(url.contains(expected), "{url} is missing {expected}");
        }
    }

    #[test]
    fn parameters_are_escaped() {
        let url = build_url(
            &credentials(),
            "search3",
            &[("query", "tyler, the creator & co".to_string())],
        );
        assert!(
            url.contains("query=tyler%2C%20the%20creator%20%26%20co"),
            "{url}"
        );
    }

    #[test]
    fn a_form_post_carries_the_same_parameters() {
        let body = form_body(&credentials(), &[("songId", "s1".to_string())]);
        assert!(body.contains(&("u".to_string(), "admin".to_string())));
        assert!(body.contains(&("f".to_string(), "json".to_string())));
        assert!(body.contains(&("songId".to_string(), "s1".to_string())));
    }

    #[test]
    fn a_successful_envelope_is_unwrapped_to_its_payload() {
        let body = r#"{"subsonic-response":{"status":"ok","version":"1.16.1",
            "type":"navidrome","serverVersion":"0.63.2","openSubsonic":true,
            "album":{"id":"al-1","name":"Blue Harvest"}}}"#;
        let mut payload = parse_envelope(body).unwrap();
        assert_eq!(payload.len(), 1);
        let album: super::super::types::AlbumWithSongsId3 = take(&mut payload, "album").unwrap();
        assert_eq!(album.album.name, "Blue Harvest");
    }

    #[test]
    fn a_failed_envelope_is_an_error_despite_http_200() {
        let body = r#"{"subsonic-response":{"status":"failed","version":"1.16.1",
            "error":{"code":40,"message":"Wrong username or password"}}}"#;
        let error = parse_envelope(body).unwrap_err();
        assert!(error.is_auth(), "{error:?}");
        assert_eq!(error.to_string(), "Wrong username or password");
    }

    #[test]
    fn a_not_found_envelope_is_reported_by_code() {
        let body = r#"{"subsonic-response":{"status":"failed",
            "error":{"code":70,"message":"Album not found"}}}"#;
        let error = parse_envelope(body).unwrap_err();
        assert!(error.is_not_found());
        assert!(!error.is_auth());
    }

    #[test]
    fn an_absent_key_reads_as_empty_rather_than_failing() {
        // `getStarred2` with nothing starred: the key is there, its contents
        // are not. `getTopSongs` on a server with no Last.fm agent: neither.
        let mut payload =
            parse_envelope(r#"{"subsonic-response":{"status":"ok","starred2":{}}}"#).unwrap();
        let starred: super::super::types::Starred2 = take(&mut payload, "starred2").unwrap();
        assert!(starred.song.is_empty());

        let mut empty = parse_envelope(r#"{"subsonic-response":{"status":"ok"}}"#).unwrap();
        let top: super::super::types::TopSongs = take(&mut empty, "topSongs").unwrap();
        assert!(top.song.is_empty());
    }

    #[test]
    fn a_plain_text_404_is_not_fed_to_serde() {
        // The body an unknown endpoint — or any web server that is not a
        // Subsonic one — answers with.
        assert!(!looks_like_json(
            "text/plain; charset=utf-8",
            "404 page not found\n"
        ));
        // getCoverArt's trap: HTTP 200, an error envelope, and a content
        // type that says XML.
        assert!(!looks_like_json("application/xml", "<subsonic-response/>"));
        assert!(looks_like_json("application/json; charset=utf-8", "{}"));
        // Navidrome labels some answers only by their shape.
        assert!(looks_like_json("", "  {\"subsonic-response\":{}}"));
    }

    #[test]
    fn an_answer_that_is_not_an_envelope_is_a_decode_error() {
        let error = parse_envelope(r#"{"something":"else"}"#).unwrap_err();
        assert!(matches!(error, ApiError::Decode(_)), "{error:?}");
    }

    #[test]
    fn calls_without_a_credential_do_not_reach_the_network() {
        let client = SubsonicClient::new(
            crate::http_client_builder().build().unwrap(),
            Arc::new(NetActivity::default()),
            20,
        );
        assert!(matches!(
            client.url("ping", &[]).unwrap_err(),
            ApiError::NotSignedIn
        ));
        client.set_credentials(Some(credentials()));
        assert!(client.url("ping", &[]).unwrap().contains("/rest/ping.view"));
    }
}
