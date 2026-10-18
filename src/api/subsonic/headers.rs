//! Headers a gate in front of the server asks every request for.
//!
//! A server behind Cloudflare Access, or any proxy like it, turns away a
//! request that does not carry the gate's own credential: a service token's
//! `CF-Access-Client-Id` and `CF-Access-Client-Secret`, a basic-auth
//! `Authorization`, a cookie. Those are typed in at sign-in, kept with the
//! Subsonic credential, and sent with every request to the server.
//!
//! To the server and to nothing else. The app's general HTTP client also
//! asks LRCLIB, GitHub and whichever host an artist image lives on, so the
//! headers are added per request, only where the URL is the server's, and
//! the clients that carry them follow no redirect to another host: reqwest
//! drops `Authorization` and `Cookie` there, but not a header it does not
//! know is a secret.

use reqwest::StatusCode;
use reqwest::header::{HeaderMap, HeaderName, HeaderValue, LOCATION, SERVER};
use serde::{Deserialize, Serialize};

use super::auth::normalize_server;

/// What the transport sets itself, or what Navidrome's own API rides on.
/// A value typed in for one of these would fight it.
const RESERVED: [&str; 10] = [
    "host",
    "content-length",
    "content-type",
    "transfer-encoding",
    "connection",
    "upgrade",
    "te",
    "range",
    "accept-encoding",
    "x-nd-authorization",
];

/// reqwest's own default.
const MAX_REDIRECTS: usize = 10;

/// One header, as it was typed.
#[derive(Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct Header {
    pub name: String,
    pub value: String,
}

/// Every extra header the server is asked with, in the order typed.
#[derive(Clone, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(transparent)]
pub struct ServerHeaders(Vec<Header>);

impl ServerHeaders {
    /// Reads the sign-in form's box: one `Name: value` per line, blank lines
    /// ignored. The error names the line and the header, never the value,
    /// because the value is usually the secret.
    pub fn parse(text: &str) -> Result<Self, String> {
        let mut headers = Vec::new();
        for (index, line) in text.lines().enumerate() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            let number = index + 1;
            let Some((name, value)) = line.split_once(':') else {
                return Err(format!(
                    "Extra headers, line {number}: write it as Name: value"
                ));
            };
            let (name, value) = (name.trim(), value.trim());
            if HeaderName::from_bytes(name.as_bytes()).is_err() {
                return Err(format!(
                    "Extra headers, line {number}: “{name}” is not a header name"
                ));
            }
            if RESERVED.contains(&name.to_ascii_lowercase().as_str()) {
                return Err(format!(
                    "Extra headers, line {number}: Fastsonic sets {name} itself"
                ));
            }
            if HeaderValue::from_str(value).is_err() {
                return Err(format!(
                    "Extra headers, line {number}: the value of {name} has characters a header cannot carry"
                ));
            }
            headers.push(Header {
                name: name.to_string(),
                value: value.to_string(),
            });
        }
        Ok(Self(headers))
    }

    /// The form's box again, as [`Self::parse`] reads it.
    pub fn to_text(&self) -> String {
        self.0
            .iter()
            .map(|header| format!("{}: {}", header.name, header.value))
            .collect::<Vec<_>>()
            .join("\n")
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// The headers as a request carries them. A file edited by hand can
    /// hold one [`Self::parse`] would have refused; it is left out, by name.
    pub fn map(&self) -> HeaderMap {
        let mut map = HeaderMap::new();
        for header in &self.0 {
            match (
                HeaderName::from_bytes(header.name.as_bytes()),
                HeaderValue::from_str(&header.value),
            ) {
                (Ok(name), Ok(mut value)) => {
                    value.set_sensitive(true);
                    map.append(name, value);
                }
                _ => log::warn!(
                    "leaving out the extra header {:?}: it is not valid",
                    header.name
                ),
            }
        }
        map
    }
}

/// Names only: the values are credentials.
impl std::fmt::Debug for ServerHeaders {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_map()
            .entries(self.0.iter().map(|header| (&header.name, "<redacted>")))
            .finish()
    }
}

/// Refuses headers that would cross the network in the clear: an `http://`
/// address sends them readable to anyone on the way, and an `https`
/// redirect after it comes too late. This computer's own address is let
/// through, since nothing is on the way there.
pub fn check_transport(server: &str, headers: &ServerHeaders) -> Result<(), String> {
    if headers.is_empty() {
        return Ok(());
    }
    let Ok(url) = reqwest::Url::parse(&normalize_server(server)) else {
        return Ok(());
    };
    let loopback = url
        .host_str()
        .map(|host| host.trim_start_matches('[').trim_end_matches(']'))
        .is_some_and(|host| {
            host.eq_ignore_ascii_case("localhost")
                || host
                    .parse::<std::net::IpAddr>()
                    .is_ok_and(|ip| ip.is_loopback())
        });
    if url.scheme() == "http" && !loopback {
        return Err(
            "Extra headers are sent only over HTTPS. Write the server address as https://…"
                .to_string(),
        );
    }
    Ok(())
}

/// Whether `url` is on the server at `server`: same scheme, host and port.
/// The headers go only where this holds.
pub fn same_origin(server: &str, url: &str) -> bool {
    let (Ok(server), Ok(url)) = (reqwest::Url::parse(server), reqwest::Url::parse(url)) else {
        return false;
    };
    server.scheme() == url.scheme()
        && server.host_str() == url.host_str()
        && server.port_or_known_default() == url.port_or_known_default()
}

/// The redirect rule of every client that talks to the server: follow one
/// that stays on the origin it came from, or that upgrades it from `http`
/// to `https` on the default ports, and stop at any other: another host,
/// another port, or a drop back to `http`. Each hop is held to the one
/// before it, so a chain cannot step off the server either. The redirect
/// is then the answer, which [`gate`] can name. A gate that wants a browser
/// sign-in redirects to one on its own domain, and nothing here can use it.
pub fn redirect_policy() -> reqwest::redirect::Policy {
    reqwest::redirect::Policy::custom(|attempt| {
        let Some(from) = attempt.previous().last() else {
            return attempt.follow();
        };
        if attempt.previous().len() > MAX_REDIRECTS {
            return attempt.error("too many redirects");
        }
        if may_follow(from, attempt.url()) {
            attempt.follow()
        } else {
            attempt.stop()
        }
    })
}

fn may_follow(from: &reqwest::Url, to: &reqwest::Url) -> bool {
    let same_host = from.host_str() == to.host_str();
    let same_origin = same_host
        && from.scheme() == to.scheme()
        && from.port_or_known_default() == to.port_or_known_default();
    let upgrade = same_host
        && from.scheme() == "http"
        && from.port_or_known_default() == Some(80)
        && to.scheme() == "https"
        && to.port_or_known_default() == Some(443);
    same_origin || upgrade
}

/// What to say when the answer is a gate's rather than the server's: a
/// redirect [`redirect_policy`] would not follow, or Cloudflare refusing
/// outright. `None` for anything the server itself could have said.
pub fn gate(status: StatusCode, headers: &HeaderMap) -> Option<String> {
    if status.is_redirection() {
        let host = headers
            .get(LOCATION)
            .and_then(|value| value.to_str().ok())
            .and_then(|location| reqwest::Url::parse(location).ok())
            .and_then(|url| url.host_str().map(str::to_string));
        return Some(match host {
            Some(host) if host.ends_with(".cloudflareaccess.com") => {
                "Cloudflare Access sent the request to its sign-in page. Add the service \
                 token's CF-Access-Client-Id and CF-Access-Client-Secret under Extra \
                 headers, and allow the token in a Service Auth policy."
                    .to_string()
            }
            Some(host) => format!(
                "The server sent the request on to {host}, which Fastsonic does not \
                 follow. A proxy in front of the server may want extra headers."
            ),
            None => format!(
                "The server answered with a redirect (HTTP {}) that Fastsonic does not follow.",
                status.as_u16()
            ),
        });
    }
    let cloudflare = headers.contains_key("cf-ray")
        || headers
            .get(SERVER)
            .and_then(|value| value.to_str().ok())
            .is_some_and(|server| server.eq_ignore_ascii_case("cloudflare"));
    (status == StatusCode::FORBIDDEN && cloudflare).then(|| {
        "Cloudflare refused the request (HTTP 403). If the server is behind Cloudflare \
         Access, check the service token under Extra headers and that a Service Auth \
         policy allows it."
            .to_string()
    })
}

#[cfg(test)]
pub(crate) mod tests {
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::sync::{Arc, Mutex};

    use super::*;
    use crate::api::NetActivity;
    use crate::api::subsonic::{ApiError, Credentials, NativeClient, NativeError, SubsonicClient};

    /// A server on a port of its own that answers every request with
    /// `answer` and keeps the head of each, lowercased, to look through.
    pub(crate) struct Listener {
        pub base: String,
        heads: Arc<Mutex<Vec<String>>>,
    }

    impl Listener {
        pub(crate) fn new(host: &str, answer: impl Fn(&str) -> String + Send + 'static) -> Self {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let base = format!("http://{host}:{}", listener.local_addr().unwrap().port());
            let heads: Arc<Mutex<Vec<String>>> = Arc::default();
            let seen = Arc::clone(&heads);
            std::thread::spawn(move || {
                for stream in listener.incoming() {
                    let Ok(mut stream) = stream else { break };
                    let mut head = Vec::new();
                    let mut byte = [0u8; 1];
                    while !head.ends_with(b"\r\n\r\n") && stream.read(&mut byte).unwrap_or(0) == 1 {
                        head.push(byte[0]);
                    }
                    let head = String::from_utf8_lossy(&head).to_ascii_lowercase();
                    let reply = answer(&head);
                    seen.lock().unwrap().push(head);
                    let _ = stream.write_all(reply.as_bytes());
                }
            });
            Self { base, heads }
        }

        pub(crate) fn heads(&self) -> Vec<String> {
            self.heads.lock().unwrap().clone()
        }
    }

    pub(crate) fn reply(status: &str, extra: &str, body: &str) -> String {
        format!(
            "HTTP/1.1 {status}\r\n{extra}Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        )
    }

    pub(crate) fn token() -> ServerHeaders {
        ServerHeaders::parse("CF-Access-Client-Id: id.access\nCF-Access-Client-Secret: s3cret")
            .unwrap()
    }

    #[test]
    fn the_box_reads_one_header_a_line() {
        let headers = ServerHeaders::parse(
            "  CF-Access-Client-Id:  abc.access \n\n\nCF-Access-Client-Secret:x:y\n",
        )
        .unwrap();
        assert_eq!(headers.len(), 2);
        assert_eq!(
            headers.to_text(),
            "CF-Access-Client-Id: abc.access\nCF-Access-Client-Secret: x:y"
        );
        let map = headers.map();
        assert_eq!(map["cf-access-client-secret"], "x:y");
        assert!(map["cf-access-client-id"].is_sensitive());
        assert!(ServerHeaders::parse(" \n").unwrap().is_empty());
        assert_eq!(ServerHeaders::parse(&headers.to_text()).unwrap(), headers);
    }

    #[test]
    fn a_line_that_is_not_a_header_names_its_line_and_never_the_value() {
        let error = ServerHeaders::parse("X-Fine: 1\njust-a-secret-value").unwrap_err();
        assert!(error.contains("line 2"), "{error}");
        assert!(!error.contains("just-a-secret-value"), "{error}");

        let error = ServerHeaders::parse("Bad Name: v").unwrap_err();
        assert!(error.contains("“Bad Name”"), "{error}");

        let error = ServerHeaders::parse("X-Token: s3cret\u{7f}").unwrap_err();
        assert!(
            error.contains("X-Token") && !error.contains("s3cret"),
            "{error}"
        );
    }

    #[test]
    fn headers_the_transport_owns_are_refused() {
        for name in ["Host", "range", "Content-Length", "X-ND-Authorization"] {
            let error = ServerHeaders::parse(&format!("{name}: x")).unwrap_err();
            assert!(error.contains("sets"), "{name}: {error}");
        }
        assert!(ServerHeaders::parse("Authorization: Basic eDp5").is_ok());
        assert!(ServerHeaders::parse("Cookie: CF_Authorization=x").is_ok());
    }

    #[test]
    fn printing_them_shows_names_only() {
        let printed = format!("{:?}", token());
        assert!(printed.contains("CF-Access-Client-Secret"), "{printed}");
        assert!(!printed.contains("s3cret"), "{printed}");
    }

    #[test]
    fn they_were_not_there_before_and_read_as_none() {
        let headers: ServerHeaders = serde_json::from_str("[]").unwrap();
        assert!(headers.is_empty());
        let stored = serde_json::to_string(&token()).unwrap();
        assert_eq!(
            stored,
            r#"[{"name":"CF-Access-Client-Id","value":"id.access"},{"name":"CF-Access-Client-Secret","value":"s3cret"}]"#
        );
    }

    #[test]
    fn headers_are_not_sent_in_the_clear() {
        // A bare host is `http://`, which is exactly the case to catch.
        for server in [
            "music.example",
            "http://music.example",
            "http://192.168.1.20:4533",
        ] {
            let error = check_transport(server, &token()).unwrap_err();
            assert!(error.contains("HTTPS"), "{server}: {error}");
        }
        for server in [
            "https://music.example",
            "http://localhost:4533",
            "http://127.0.0.1:4533",
            "http://[::1]:4533",
        ] {
            assert!(check_transport(server, &token()).is_ok(), "{server}");
        }
        assert!(check_transport("http://music.example", &ServerHeaders::default()).is_ok());
    }

    #[test]
    fn only_the_server_itself_is_the_same_origin() {
        let server = "https://music.example";
        assert!(same_origin(
            server,
            "https://music.example/rest/ping.view?u=a"
        ));
        assert!(same_origin(server, "https://music.example:443/share/img/x"));
        assert!(!same_origin(server, "http://music.example/rest/ping.view"));
        assert!(!same_origin(server, "https://music.example:8443/"));
        assert!(!same_origin(
            server,
            "https://lastfm.freetls.fastly.net/i/u/x.jpg"
        ));
        assert!(!same_origin(server, "https://music.example.evil.test/"));
        assert!(!same_origin(server, "not a url"));
    }

    #[test]
    fn a_redirect_off_the_server_is_not_followed() {
        let elsewhere = Listener::new("localhost", |_| reply("200 OK", "", "elsewhere"));
        let target = format!("{}/landing", elsewhere.base);
        let server = Listener::new("127.0.0.1", move |_| {
            reply("302 Found", &format!("Location: {target}\r\n"), "")
        });
        let http = crate::blocking_http_client_builder()
            .redirect(redirect_policy())
            .default_headers(token().map())
            .build()
            .unwrap();
        let response = http
            .get(format!("{}/rest/ping.view", server.base))
            .send()
            .unwrap();
        assert_eq!(response.status(), StatusCode::FOUND);
        assert_eq!(server.heads().len(), 1);
        assert!(server.heads()[0].contains("cf-access-client-secret: s3cret"));
        assert!(
            elsewhere.heads().is_empty(),
            "the token went to another host"
        );
    }

    #[test]
    fn a_redirect_on_the_same_origin_is_followed() {
        let server = Listener::new("127.0.0.1", |head| {
            if head.starts_with("get /music/") {
                reply("200 OK", "", "here")
            } else {
                reply(
                    "301 Moved Permanently",
                    "Location: /music/rest/ping.view\r\n",
                    "",
                )
            }
        });
        let http = crate::blocking_http_client_builder()
            .redirect(redirect_policy())
            .build()
            .unwrap();
        let response = http
            .get(format!("{}/rest/ping.view", server.base))
            .send()
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(server.heads().len(), 2);
    }

    #[test]
    fn a_redirect_to_another_port_on_the_same_host_is_not_followed() {
        let other_port = Listener::new("127.0.0.1", |_| reply("200 OK", "", "elsewhere"));
        let target = format!("{}/rest/ping.view", other_port.base);
        let server = Listener::new("127.0.0.1", move |_| {
            reply("302 Found", &format!("Location: {target}\r\n"), "")
        });
        let http = crate::blocking_http_client_builder()
            .redirect(redirect_policy())
            .default_headers(token().map())
            .build()
            .unwrap();
        let response = http
            .get(format!("{}/rest/ping.view", server.base))
            .send()
            .unwrap();
        assert_eq!(response.status(), StatusCode::FOUND);
        assert!(
            other_port.heads().is_empty(),
            "the token went to another port"
        );
    }

    #[test]
    fn only_the_origin_itself_or_its_https_upgrade_is_followed() {
        let url = |text: &str| reqwest::Url::parse(text).unwrap();
        let follows = |from: &str, to: &str| may_follow(&url(from), &url(to));
        assert!(follows(
            "https://music.example/a",
            "https://music.example/b"
        ));
        assert!(follows(
            "https://music.example/a",
            "https://music.example:443/b"
        ));
        assert!(follows(
            "http://music.example/rest",
            "https://music.example/rest"
        ));
        assert!(follows(
            "http://music.example:4533/a",
            "http://music.example:4533/b"
        ));
        // An upgrade only between the default ports: anything else is a
        // different service that happens to share the host.
        assert!(!follows(
            "http://music.example:4533/",
            "https://music.example/"
        ));
        assert!(!follows(
            "http://music.example/",
            "https://music.example:8443/"
        ));
        assert!(!follows(
            "https://music.example/",
            "https://music.example:8443/"
        ));
        assert!(!follows("https://music.example/", "http://music.example/"));
        assert!(!follows("https://music.example/", "https://other.example/"));
    }

    fn server_client() -> reqwest::Client {
        crate::http_client_builder()
            .redirect(redirect_policy())
            .build()
            .unwrap()
    }

    fn runtime() -> tokio::runtime::Runtime {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
    }

    fn carries_the_token(head: &str) -> bool {
        head.contains("cf-access-client-id: id.access")
            && head.contains("cf-access-client-secret: s3cret")
    }

    const OK: &str = r#"{"subsonic-response":{"status":"ok","version":"1.16.1"}}"#;
    const JSON: &str = "Content-Type: application/json\r\n";

    #[test]
    fn every_subsonic_call_carries_them() {
        let server = Listener::new("127.0.0.1", |_| reply("200 OK", JSON, OK));
        let client = SubsonicClient::new(server_client(), Arc::new(NetActivity::default()), 20);
        client.set_credentials(Some(
            Credentials::from_pair(&server.base, "u", "s", "t").with_headers(token()),
        ));
        let runtime = runtime();
        runtime.block_on(client.ping()).unwrap();
        runtime
            .block_on(client.post("updatePlaylist", &[("playlistId", "1".to_string())]))
            .unwrap();
        let heads = server.heads();
        assert_eq!(heads.len(), 2);
        assert!(heads[0].starts_with("get /rest/ping.view"), "{}", heads[0]);
        assert!(
            heads[1].starts_with("post /rest/updateplaylist.view"),
            "{}",
            heads[1]
        );
        assert!(
            heads.iter().all(|head| carries_the_token(head)),
            "{heads:?}"
        );
    }

    #[test]
    fn navidrome_sign_in_sends_them_and_keeps_them() {
        let server = Listener::new("127.0.0.1", |head| {
            if head.starts_with("post /auth/login") {
                reply(
                    "200 OK",
                    JSON,
                    r#"{"token":"jwt","subsonicSalt":"s","subsonicToken":"t","username":"u"}"#,
                )
            } else {
                reply("200 OK", JSON, "[]")
            }
        });
        let native = NativeClient::new(server_client(), Arc::new(NetActivity::default()));
        let runtime = runtime();
        let signed = runtime
            .block_on(native.sign_in(&server.base, "u", "p", &token()))
            .unwrap();
        assert_eq!(signed.credentials.headers, token());
        native.set_session(Some(signed.session));
        native.set_headers(token());
        runtime.block_on(native.recently_played(0, 1)).unwrap();
        let heads = server.heads();
        assert_eq!(heads.len(), 2);
        assert!(
            heads.iter().all(|head| carries_the_token(head)),
            "{heads:?}"
        );
    }

    #[test]
    fn a_gate_reads_as_a_sign_in_problem() {
        let server = Listener::new("127.0.0.1", |_| {
            reply(
                "302 Found",
                "Location: https://team.cloudflareaccess.com/cdn-cgi/access/login/x\r\n",
                "",
            )
        });
        let runtime = runtime();
        let client = SubsonicClient::new(server_client(), Arc::new(NetActivity::default()), 20);
        client.set_credentials(Some(Credentials::from_pair(&server.base, "u", "s", "t")));
        let error = runtime.block_on(client.ping()).unwrap_err();
        assert!(
            matches!(error, ApiError::Gate(_)) && error.is_auth(),
            "{error:?}"
        );
        assert!(error.to_string().contains("Cloudflare Access"), "{error}");

        // Not mistaken for a server that is not Navidrome, which would carry
        // on to a sign-in that cannot work either.
        let native = NativeClient::new(server_client(), Arc::new(NetActivity::default()));
        let error = runtime
            .block_on(native.sign_in(&server.base, "u", "p", &ServerHeaders::default()))
            .unwrap_err();
        assert!(
            matches!(&error, NativeError::Rejected(message) if message.contains("Cloudflare Access")),
            "{error:?}"
        );
    }

    #[test]
    fn artwork_on_another_host_goes_without_them() {
        let server = Listener::new("127.0.0.1", |_| reply("404 Not Found", "", ""));
        let elsewhere = Listener::new("localhost", |_| reply("404 Not Found", "", ""));
        let runtime = runtime();
        let dir =
            std::env::temp_dir().join(format!("fastsonic-art-headers-{}", std::process::id()));
        let loader = crate::images::ArtLoader::new(
            crate::http_client_builder().build().unwrap(),
            server_client(),
            runtime.handle().clone(),
            dir.clone(),
        );
        loader.set_credentials(Some(
            Credentials::from_pair(&server.base, "u", "s", "t").with_headers(token()),
        ));
        let on_server = format!("{}/share/img/artist?size=600", server.base);
        let other = format!("{}/i/u/300x300/artist.png", elsewhere.base);
        assert!(runtime.block_on(loader.fetch(&on_server)).is_err());
        assert!(runtime.block_on(loader.fetch(&other)).is_err());
        let _ = std::fs::remove_dir_all(&dir);

        assert_eq!(server.heads().len(), 1);
        assert!(carries_the_token(&server.heads()[0]));
        assert_eq!(elsewhere.heads().len(), 1);
        assert!(
            !elsewhere.heads()[0].contains("cf-access"),
            "{}",
            elsewhere.heads()[0]
        );
    }

    #[test]
    fn a_gate_is_named_for_what_it_is() {
        let mut headers = HeaderMap::new();
        headers.insert(
            LOCATION,
            HeaderValue::from_static(
                "https://team.cloudflareaccess.com/cdn-cgi/access/login/music.example",
            ),
        );
        let message = gate(StatusCode::FOUND, &headers).unwrap();
        assert!(message.contains("Cloudflare Access"), "{message}");

        headers.insert(
            LOCATION,
            HeaderValue::from_static("https://auth.example/login"),
        );
        let message = gate(StatusCode::FOUND, &headers).unwrap();
        assert!(message.contains("auth.example"), "{message}");

        let mut cloudflare = HeaderMap::new();
        cloudflare.insert("cf-ray", HeaderValue::from_static("8a1b2c3d4e5f-AMS"));
        assert!(gate(StatusCode::FORBIDDEN, &cloudflare).is_some());
        // Navidrome refusing a password through Cloudflare is the server's
        // answer, not the gate's.
        assert!(gate(StatusCode::UNAUTHORIZED, &cloudflare).is_none());
        assert!(gate(StatusCode::FORBIDDEN, &HeaderMap::new()).is_none());
        assert!(gate(StatusCode::NOT_FOUND, &cloudflare).is_none());
    }
}
