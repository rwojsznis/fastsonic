//! The credential every request to the music server carries.
//!
//! Subsonic authenticates with a salted token: `u=user`, `s=salt`,
//! `t=md5(password + salt)`. The salt is the client's to choose and does not
//! have to change per request — the server holds the password and recomputes
//! — so the app asks for the password once, derives a pair, and stores that.
//! The plaintext is never written to disk (D10 in `migration/00-decisions.md`).
//!
//! The pair is still a password-equivalent credential for one server: anyone
//! holding it can act as the user. It gets the same care the old tokens
//! got, and one thing more, because with token auth the credential rides in
//! every URL's query string — so nothing here may be logged without going
//! through [`redacted`].

use md5::{Digest, Md5};
use rand::Rng;
use serde::{Deserialize, Serialize};

/// The query parameters whose values are credentials, in any URL this app
/// builds: the salted token, its salt, a plaintext password (which this
/// client never sends, but a hand-built URL might), and an OpenSubsonic API
/// key (which Navidrome 0.63 does not offer, but a later one may).
const SECRET_PARAMS: [&str; 4] = ["t", "s", "p", "apiKey"];

/// Everything needed to talk to one server as one user.
#[derive(Clone, Default, Deserialize, Serialize, PartialEq, Eq)]
pub struct Credentials {
    /// Base URL with no trailing slash and no `/rest` suffix.
    pub server: String,
    pub username: String,
    pub salt: String,
    pub token: String,
}

impl Credentials {
    /// Derives the stored pair from a password typed at sign-in. The
    /// password is not kept: only `md5(password + salt)` and the salt are.
    pub fn from_password(server: &str, username: &str, password: &str) -> Self {
        let salt = random_salt();
        let token = salted_token(password, &salt);
        Self {
            server: normalize_server(server),
            username: username.to_string(),
            salt,
            token,
        }
    }

    /// Adopts the `subsonicSalt` / `subsonicToken` pair Navidrome's
    /// `/auth/login` hands back, which authenticates `/rest/` calls just as
    /// a locally derived pair does.
    pub fn from_pair(server: &str, username: &str, salt: &str, token: &str) -> Self {
        Self {
            server: normalize_server(server),
            username: username.to_string(),
            salt: salt.to_string(),
            token: token.to_string(),
        }
    }

    /// The authentication half of every request's query string.
    pub fn params(&self) -> [(&'static str, String); 3] {
        [
            ("u", self.username.clone()),
            ("t", self.token.clone()),
            ("s", self.salt.clone()),
        ]
    }

    pub fn is_empty(&self) -> bool {
        self.server.is_empty() || self.username.is_empty() || self.token.is_empty()
    }
}

/// Redacted, so that a credential cannot reach a log by being printed.
impl std::fmt::Debug for Credentials {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Credentials")
            .field("server", &self.server)
            .field("username", &self.username)
            .field("salt", &"<redacted>")
            .field("token", &"<redacted>")
            .finish()
    }
}

/// A URL safe to log: every credential parameter keeps its name and loses
/// its value. Cover art URLs carry the credential too, so `src/images.rs` is
/// as much a logging site as this module.
pub fn redacted(url: &str) -> String {
    let Some((path, query)) = url.split_once('?') else {
        return url.to_string();
    };
    let mut out = String::with_capacity(url.len());
    out.push_str(path);
    out.push('?');
    for (index, pair) in query.split('&').enumerate() {
        if index > 0 {
            out.push('&');
        }
        let name = pair.split('=').next().unwrap_or(pair);
        if SECRET_PARAMS.contains(&name) {
            out.push_str(name);
            out.push_str("=<redacted>");
        } else {
            out.push_str(pair);
        }
    }
    out
}

/// `http://` is assumed for a bare host, because a self-hosted server on a
/// LAN commonly has no certificate. A trailing slash is dropped so that
/// paths can be appended without doubling it.
pub fn normalize_server(server: &str) -> String {
    let trimmed = server.trim().trim_end_matches('/');
    if trimmed.is_empty() {
        return String::new();
    }
    if trimmed.starts_with("http://") || trimmed.starts_with("https://") {
        trimmed.to_string()
    } else {
        format!("http://{trimmed}")
    }
}

/// `md5(password + salt)`, hex encoded — the Subsonic `t` parameter.
pub fn salted_token(password: &str, salt: &str) -> String {
    let mut hasher = Md5::new();
    hasher.update(password.as_bytes());
    hasher.update(salt.as_bytes());
    hex(&hasher.finalize())
}

/// Twelve hex characters, the length Subsonic clients conventionally use.
fn random_salt() -> String {
    let bytes: [u8; 6] = rand::rng().random();
    hex(&bytes)
}

fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write;
    bytes.iter().fold(String::new(), |mut out, byte| {
        let _ = write!(out, "{byte:02x}");
        out
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_token_is_md5_of_password_and_salt() {
        // The pair Navidrome's /auth/login returned for the development
        // server, recomputed here: this is the identity D10 rests on.
        assert_eq!(
            salted_token("fastsonic", "abcdef012345"),
            "d19747a82ab779a2e87514556f662177"
        );
    }

    #[test]
    fn credentials_from_a_password_do_not_keep_it() {
        let credentials = Credentials::from_password("localhost:4533", "admin", "fastsonic");
        assert_eq!(credentials.server, "http://localhost:4533");
        assert_eq!(credentials.salt.len(), 12);
        assert_eq!(
            credentials.token,
            salted_token("fastsonic", &credentials.salt)
        );
        let printed = format!("{credentials:?}");
        assert!(!printed.contains("fastsonic") || printed.contains("<redacted>"));
        assert!(!printed.contains(&credentials.token));
    }

    #[test]
    fn two_salts_differ() {
        let one = Credentials::from_password("host", "u", "p");
        let two = Credentials::from_password("host", "u", "p");
        assert_ne!(one.salt, two.salt);
        assert_ne!(one.token, two.token);
    }

    #[test]
    fn server_urls_are_normalized() {
        assert_eq!(normalize_server(" music.example/ "), "http://music.example");
        assert_eq!(
            normalize_server("https://music.example/"),
            "https://music.example"
        );
        assert_eq!(normalize_server(""), "");
    }

    #[test]
    fn logging_a_url_strips_the_credential() {
        let url = "http://host/rest/getAlbum.view?u=admin&t=deadbeef&s=abc123&id=7&f=json";
        assert_eq!(
            redacted(url),
            "http://host/rest/getAlbum.view?u=admin&t=<redacted>&s=<redacted>&id=7&f=json"
        );
        assert_eq!(
            redacted("http://host/rest/ping.view"),
            "http://host/rest/ping.view"
        );
        // The names that only a hand-built URL would carry are stripped too.
        assert_eq!(
            redacted("http://host/x?p=enc:6162&apiKey=zzz"),
            "http://host/x?p=<redacted>&apiKey=<redacted>"
        );
    }
}
