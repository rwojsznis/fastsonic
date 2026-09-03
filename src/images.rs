//! Album art: fetched once, kept on disk, decoded by egui on demand.
//!
//! [`ArtLoader`] handles `http(s)` URIs in egui's image pipeline, and the
//! `sonic:art:<size>:<id>` requests the Subsonic layer leaves in its models.
//! The first request starts a background download or disk-cache read. A size
//! limit keeps long sessions from retaining unlimited textures.
//!
//! Two things are particular to a self-hosted server.
//!
//! **Cover art is a request, not a URL.** `getCoverArt` needs the credential
//! on every fetch, so the models carry `sonic:art:…` and this module builds
//! the real URL when it fetches. That keeps the credential out of anything
//! that gets cached or logged, and means a new sign-in does not invalidate
//! the art on disk: the cache is keyed by the request, not by the URL.
//! Artist images are the exception — the server hands over pre-signed
//! `/share/img/` URLs that need no credential, and those arrive here as
//! ordinary `http(s)` and are fetched as they are.
//!
//! **A `200` is not proof of an image.** Ask `getCoverArt` for an id it does
//! not have and it answers `HTTP 200` with an *error envelope* where the
//! bytes should be. Believing the status code would put that envelope in the
//! disk cache under the name of a cover, and every later request would read
//! it back and fail to decode. So the bytes are checked before they are kept.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use egui::load::{Bytes, BytesLoadResult, BytesLoader, BytesPoll, LoadError};
use sha1::{Digest, Sha1};

use crate::api::subsonic::Credentials;
use crate::api::subsonic::convert::parse_art_url;

/// Maximum artwork bytes held in memory.
///
/// Time-based eviction does not work here: after creating a texture, egui no
/// longer requests its source bytes. Visible images were therefore evicted and
/// reloaded every two and a half minutes (#129).
///
/// Size-based eviction keeps visible images stable.
const HELD_BYTES: usize = 64 * 1024 * 1024;
const MAX_ART_BYTES: usize = 8 * 1024 * 1024;

enum Entry {
    Pending,
    Ready {
        bytes: Arc<[u8]>,
        last_used: Instant,
    },
    Failed(String),
}

struct Inner {
    entries: Mutex<HashMap<String, Entry>>,
    http: reqwest::Client,
    runtime: tokio::runtime::Handle,
    cache_dir: PathBuf,
    /// Needed to turn a `sonic:art:` request into a `getCoverArt` URL.
    /// Absent until sign-in, which is also when the first cover is asked for.
    credentials: Mutex<Option<Credentials>>,
}

#[derive(Clone)]
pub struct ArtLoader {
    inner: Arc<Inner>,
}

impl ArtLoader {
    pub fn new(http: reqwest::Client, runtime: tokio::runtime::Handle, cache_dir: PathBuf) -> Self {
        let _ = std::fs::create_dir_all(&cache_dir);
        Self {
            inner: Arc::new(Inner {
                entries: Mutex::new(HashMap::new()),
                http,
                runtime,
                cache_dir,
                credentials: Mutex::new(None),
            }),
        }
    }

    /// The credential cover-art requests are built with. Set at sign-in and
    /// cleared at sign-out, like the one the API client holds.
    pub fn set_credentials(&self, credentials: Option<Credentials>) {
        *self
            .inner
            .credentials
            .lock()
            .unwrap_or_else(|p| p.into_inner()) = credentials;
    }

    /// Bytes for `uri`, from memory, disk, or the network.
    pub async fn fetch(&self, uri: &str) -> Result<Arc<[u8]>, String> {
        self.inner.fetch(uri).await
    }

    /// Evicts failed entries and the oldest artwork above the memory limit.
    pub fn evict(&self, ctx: &egui::Context) {
        let letting_go: Vec<String> = {
            let entries = self.inner.entries.lock().unwrap_or_else(|p| p.into_inner());
            let mut failed: Vec<String> = Vec::new();
            let mut held: Vec<(String, Instant, usize)> = Vec::new();
            for (url, entry) in entries.iter() {
                match entry {
                    // Forget failures so a later request can retry.
                    Entry::Failed(_) => failed.push(url.clone()),
                    Entry::Ready { bytes, last_used } => {
                        held.push((url.clone(), *last_used, bytes.len()))
                    }
                    Entry::Pending => {}
                }
            }
            failed.extend(over_budget(held, HELD_BYTES));
            failed
        };
        for url in letting_go {
            ctx.forget_image(&url);
        }
    }

    /// The disk-cache file holding `url`'s artwork, once it has been fetched.
    ///
    /// The cache is written atomically (a `.part` file, then a rename), so a
    /// file that is here at all holds a complete, successful response. The
    /// desktop media controls hand this path to the platform instead of the
    /// remote URL: macOS loads cover art itself, synchronously, inside a
    /// callback that cannot report a failure.
    pub fn cached_file(&self, url: &str) -> Option<PathBuf> {
        let path = self.inner.cache_path(url);
        std::fs::metadata(&path)
            .is_ok_and(|meta| meta.is_file() && meta.len() > 0)
            .then_some(path)
    }

    pub fn clear_disk_cache(&self) -> std::io::Result<u64> {
        let mut removed = 0;
        for entry in std::fs::read_dir(&self.inner.cache_dir)? {
            let entry = entry?;
            if entry.file_type()?.is_file() {
                removed += entry.metadata().map(|m| m.len()).unwrap_or(0);
                let _ = std::fs::remove_file(entry.path());
            }
        }
        Ok(removed)
    }
}

/// Which artwork to let go of so that what is kept fits `budget`,
/// oldest first.
///
/// "Oldest" is when egui last needed the bytes, which for a picture it
/// has already made a texture of is when it first loaded. That makes
/// this a rough order rather than a true reading of what is on screen,
/// which is why the budget is generous: being roughly right about which
/// to drop only matters once there is far more artwork than any window
/// is showing.
fn over_budget(mut held: Vec<(String, Instant, usize)>, budget: usize) -> Vec<String> {
    let mut total: usize = held.iter().map(|(_, _, bytes)| bytes).sum();
    if total <= budget {
        return Vec::new();
    }
    held.sort_by_key(|(_, last_used, _)| *last_used);
    let mut letting_go = Vec::new();
    for (url, _, bytes) in held {
        if total <= budget {
            break;
        }
        total = total.saturating_sub(bytes);
        letting_go.push(url);
    }
    letting_go
}

impl Inner {
    fn cache_path(&self, url: &str) -> PathBuf {
        let digest = Sha1::digest(url.as_bytes());
        let mut name = String::with_capacity(40);
        for byte in digest {
            use std::fmt::Write;
            let _ = write!(name, "{byte:02x}");
        }
        self.cache_dir.join(name)
    }

    /// The URL to fetch `uri` from. A `sonic:art:` request becomes a
    /// `getCoverArt` call with the current credential; anything else is
    /// already a URL. Never logged: the answer contains the credential.
    fn resolve(&self, uri: &str) -> Result<String, String> {
        let Some((size, cover_art_id)) = parse_art_url(uri) else {
            return Ok(uri.to_string());
        };
        let credentials = self
            .credentials
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .clone()
            .filter(|credentials| !credentials.is_empty())
            .ok_or_else(|| "not signed in".to_string())?;
        let mut url = format!("{}/rest/getCoverArt.view?", credentials.server);
        for (name, value) in credentials
            .params()
            .iter()
            .map(|(name, value)| (*name, value.clone()))
            .chain([
                ("v", crate::api::subsonic::client::API_VERSION.to_string()),
                ("c", crate::api::subsonic::client::CLIENT_NAME.to_string()),
                ("id", cover_art_id.to_string()),
                ("size", size.to_string()),
            ])
        {
            url.push_str(&urlencoding::encode(name));
            url.push('=');
            url.push_str(&urlencoding::encode(&value));
            url.push('&');
        }
        url.pop();
        Ok(url)
    }

    async fn fetch(self: &Arc<Self>, uri: &str) -> Result<Arc<[u8]>, String> {
        if let Some(Entry::Ready { bytes, .. }) = self
            .entries
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .get(uri)
        {
            return Ok(Arc::clone(bytes));
        }
        // Keyed by the request, not by the URL it resolves to, so that
        // signing in again — which changes the salt, and so every cover art
        // URL — does not throw away the artwork already on disk.
        let path = self.cache_path(uri);
        let cached = tokio::task::spawn_blocking({
            let path = path.clone();
            move || std::fs::read(path).ok()
        })
        .await
        .ok()
        .flatten();
        let bytes: Vec<u8> = match cached {
            Some(bytes) if !bytes.is_empty() => bytes,
            _ => {
                let url = self.resolve(uri)?;
                let response = self
                    .http
                    .get(&url)
                    .send()
                    .await
                    .map_err(|error| error.to_string())?;
                if !response.status().is_success() {
                    return Err(format!("artwork request failed: {}", response.status()));
                }
                let content_type = response
                    .headers()
                    .get(reqwest::header::CONTENT_TYPE)
                    .and_then(|value| value.to_str().ok())
                    .unwrap_or_default()
                    .to_string();
                let bytes = response.bytes().await.map_err(|error| error.to_string())?;
                if bytes.len() > MAX_ART_BYTES {
                    return Err("artwork is too large".to_string());
                }
                if !looks_like_image(&content_type, &bytes) {
                    // An error envelope where a cover should be. Say so, and
                    // above all do not write it to the cache under the name
                    // of a picture.
                    return Err(server_art_error(&bytes));
                }
                let bytes = bytes.to_vec();
                let write_path = path.clone();
                let payload = bytes.clone();
                self.runtime.spawn_blocking(move || {
                    let temporary = write_path.with_extension("part");
                    if std::fs::write(&temporary, &payload).is_ok() {
                        let _ = std::fs::rename(&temporary, &write_path);
                    }
                });
                bytes
            }
        };
        Ok(Arc::from(bytes))
    }

    fn start(self: &Arc<Self>, ctx: &egui::Context, url: String) {
        let loader = Arc::clone(self);
        let ctx = ctx.clone();
        self.runtime.spawn(async move {
            let result = loader.fetch(&url).await;
            let entry = match result {
                Ok(bytes) => Entry::Ready {
                    bytes,
                    last_used: Instant::now(),
                },
                Err(error) => Entry::Failed(error),
            };
            loader
                .entries
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .insert(url, entry);
            ctx.request_repaint();
        });
    }
}

impl BytesLoader for ArtLoader {
    fn id(&self) -> &'static str {
        "fastsonic::ArtLoader"
    }

    fn load(&self, ctx: &egui::Context, uri: &str) -> BytesLoadResult {
        if !handled(uri) {
            return Err(LoadError::NotSupported);
        }
        let mut entries = self.inner.entries.lock().unwrap_or_else(|p| p.into_inner());
        match entries.get_mut(uri) {
            Some(Entry::Ready { bytes, last_used }) => {
                *last_used = Instant::now();
                Ok(BytesPoll::Ready {
                    size: None,
                    bytes: Bytes::Shared(Arc::clone(bytes)),
                    mime: None,
                })
            }
            Some(Entry::Pending) => Ok(BytesPoll::Pending { size: None }),
            Some(Entry::Failed(error)) => Err(LoadError::Loading(error.clone())),
            None => {
                entries.insert(uri.to_string(), Entry::Pending);
                drop(entries);
                self.inner.start(ctx, uri.to_string());
                Ok(BytesPoll::Pending { size: None })
            }
        }
    }

    fn forget(&self, uri: &str) {
        self.inner
            .entries
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .remove(uri);
    }

    fn forget_all(&self) {
        self.inner
            .entries
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .clear();
    }

    fn byte_size(&self) -> usize {
        self.inner
            .entries
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .values()
            .map(|entry| match entry {
                Entry::Ready { bytes, .. } => bytes.len(),
                _ => 0,
            })
            .sum()
    }
}

/// Whether this loader answers for `uri`: a URL the server handed over, or a
/// cover-art request the Subsonic layer left for us to build.
fn handled(uri: &str) -> bool {
    uri.starts_with("https://") || uri.starts_with("http://") || parse_art_url(uri).is_some()
}

/// Whether a body is a picture. The content type answers when the server
/// gives a usable one; the leading bytes answer otherwise, because a
/// `getCoverArt` failure arrives as `HTTP 200` with an error envelope, and
/// the difference between that and a JPEG is not in the status line.
fn looks_like_image(content_type: &str, bytes: &[u8]) -> bool {
    if content_type.starts_with("image/") {
        return true;
    }
    if content_type.contains("json")
        || content_type.contains("xml")
        || content_type.contains("html")
    {
        return false;
    }
    matches!(
        bytes,
        [0xFF, 0xD8, 0xFF, ..]                                  // JPEG
            | [0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A, ..] // PNG
            | [b'G', b'I', b'F', b'8', ..]                       // GIF
            | [b'B', b'M', ..] // BMP
    ) || bytes.starts_with(b"RIFF") && bytes.get(8..12) == Some(b"WEBP")
}

/// What the server said instead of sending a picture, short enough to be an
/// error message and free of anything sensitive.
fn server_art_error(bytes: &[u8]) -> String {
    let body = String::from_utf8_lossy(&bytes[..bytes.len().min(512)]);
    let message = serde_json::from_str::<serde_json::Value>(&body)
        .ok()
        .and_then(|value| {
            value
                .get("subsonic-response")?
                .get("error")?
                .get("message")?
                .as_str()
                .map(str::to_string)
        });
    match message {
        Some(message) => format!("the server has no artwork here: {message}"),
        None => "the server answered with something that is not a picture".to_string(),
    }
}

/// A colour that represents an album cover, suitable for tinting a dark or
/// light surface: the most common saturated hue, with its lightness pulled
/// into a range that still reads as a background.
pub fn accent_color(bytes: &[u8]) -> Option<[u8; 3]> {
    let decoded = image::load_from_memory(bytes).ok()?;
    let small = decoded.thumbnail(48, 48).to_rgb8();
    let mut buckets: HashMap<(u8, u8, u8), (u64, [u64; 3])> = HashMap::new();
    for pixel in small.pixels() {
        let [r, g, b] = pixel.0;
        let (max, min) = (r.max(g).max(b) as f32, r.min(g).min(b) as f32);
        let saturation = if max == 0.0 { 0.0 } else { (max - min) / max };
        let lightness = (max + min) / 510.0;
        // Weight toward vivid mid-tones so black borders and white text lose.
        let weight = (1.0 + saturation * 6.0) * (1.0 - (lightness - 0.5).abs() * 1.4).max(0.05);
        let weight = (weight * 100.0) as u64;
        let key = (r >> 4, g >> 4, b >> 4);
        let bucket = buckets.entry(key).or_insert((0, [0, 0, 0]));
        bucket.0 += weight;
        bucket.1[0] += r as u64 * weight;
        bucket.1[1] += g as u64 * weight;
        bucket.1[2] += b as u64 * weight;
    }
    let (_, (weight, sum)) = buckets.into_iter().max_by_key(|(_, (weight, _))| *weight)?;
    if weight == 0 {
        return None;
    }
    Some([
        (sum[0] / weight) as u8,
        (sum[1] / weight) as u8,
        (sum[2] / weight) as u8,
    ])
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The media controls ask for a file rather than a URL, and have to be
    /// told "not yet" rather than handed a path to nothing: macOS loads cover
    /// art itself and dereferences a failed load without checking it, which
    /// takes the whole process with it.
    #[test]
    fn a_cached_file_is_named_only_once_it_is_really_there() {
        let dir = std::env::temp_dir().join(format!("fastpotify-art-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let runtime = tokio::runtime::Builder::new_current_thread()
            .build()
            .expect("a runtime to hand the loader");
        let loader = ArtLoader::new(
            reqwest::Client::new(),
            runtime.handle().clone(),
            dir.clone(),
        );
        let url = "https://i.scdn.co/image/abc";

        assert_eq!(loader.cached_file(url), None, "nothing downloaded yet");

        // A half-written download never appears under its real name -- the
        // cache renames one into place -- but an empty file is not artwork.
        let path = loader.inner.cache_path(url);
        std::fs::write(&path, b"").expect("an empty file");
        assert_eq!(loader.cached_file(url), None, "empty is not artwork");

        std::fs::write(&path, b"\xff\xd8\xff jpeg-ish").expect("a file with bytes");
        assert_eq!(loader.cached_file(url), Some(path));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn accent_color_finds_dominant_hue() {
        let mut image = image::RgbImage::new(16, 16);
        for (x, _, pixel) in image.enumerate_pixels_mut() {
            *pixel = if x < 12 {
                image::Rgb([20, 120, 200])
            } else {
                image::Rgb([255, 255, 255])
            };
        }
        let mut bytes = Vec::new();
        image
            .write_to(
                &mut std::io::Cursor::new(&mut bytes),
                image::ImageFormat::Png,
            )
            .unwrap();
        let color = accent_color(&bytes).unwrap();
        assert!(
            color[2] > color[0],
            "expected the blue field, got {color:?}"
        );
    }

    fn loader() -> ArtLoader {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let loader = ArtLoader::new(
            crate::http_client_builder().build().unwrap(),
            runtime.handle().clone(),
            std::env::temp_dir().join("fastsonic-art-test"),
        );
        // The runtime only has to outlive construction for these tests.
        std::mem::forget(runtime);
        loader
    }

    /// Rule: this loader answers for artwork requests as well as for the
    /// URLs the server hands over ready-made.
    #[test]
    fn art_requests_and_urls_are_both_handled() {
        assert!(handled("sonic:art:300:al-1"));
        assert!(handled("http://host/share/img/JWT?size=300"));
        assert!(handled("https://host/cover.jpg"));
        assert!(!handled("sonic:track:s1"));
        assert!(!handled("file:///cover.jpg"));
    }

    /// Rule: a cover-art request becomes a real call, credential and all —
    /// and nothing may ask for one before there is a credential to ask with.
    #[test]
    fn an_art_request_becomes_a_getcoverart_call() {
        let loader = loader();
        assert!(loader.inner.resolve("sonic:art:300:al-1").is_err());

        loader.set_credentials(Some(Credentials::from_pair(
            "http://host:4533",
            "admin",
            "salt",
            "token",
        )));
        let url = loader.inner.resolve("sonic:art:300:al-1").unwrap();
        assert!(url.starts_with("http://host:4533/rest/getCoverArt.view?"));
        for expected in ["u=admin", "t=token", "s=salt", "id=al-1", "size=300"] {
            assert!(url.contains(expected), "{url} is missing {expected}");
        }

        // A pre-signed artist image needs no credential and is not rebuilt.
        let signed = "http://host:4533/share/img/JWT?size=300";
        assert_eq!(loader.inner.resolve(signed).unwrap(), signed);
    }

    /// Rule: the disk cache is keyed by the request, so signing in again —
    /// which changes the salt, and with it every artwork URL — does not
    /// throw away artwork already fetched.
    #[test]
    fn the_cache_key_survives_a_new_sign_in() {
        let loader = loader();
        let before = loader.inner.cache_path("sonic:art:300:al-1");
        loader.set_credentials(Some(Credentials::from_password(
            "http://host",
            "admin",
            "a",
        )));
        let after = loader.inner.cache_path("sonic:art:300:al-1");
        assert_eq!(before, after);
    }

    /// Rule: an error envelope arriving where a picture should be is
    /// refused, whatever the status line said. Caching it would put a JSON
    /// document in the art cache under a cover's name for good.
    #[test]
    fn an_error_envelope_is_not_mistaken_for_a_picture() {
        let envelope = br#"{"subsonic-response":{"status":"failed","error":{"code":70,"message":"Artwork not found"}}}"#;
        assert!(!looks_like_image("application/json", envelope));
        assert!(!looks_like_image(
            "application/xml",
            b"<subsonic-response/>"
        ));
        // Navidrome answers `f=json` errors as JSON; older servers as XML.
        // Neither says so in the status line, so the body has to.
        assert!(!looks_like_image("", envelope));
        assert!(server_art_error(envelope).contains("Artwork not found"));
    }

    /// Rule: the pictures a library actually contains are accepted, whether
    /// or not the server bothered to label them.
    #[test]
    fn real_pictures_are_accepted() {
        assert!(looks_like_image("image/jpeg", &[0xFF, 0xD8, 0xFF, 0xE0]));
        assert!(looks_like_image("", &[0xFF, 0xD8, 0xFF, 0xE0]));
        assert!(looks_like_image(
            "",
            &[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A, 0]
        ));
        assert!(looks_like_image("", b"GIF89a......"));
        assert!(looks_like_image("", b"RIFF\0\0\0\0WEBPVP8 "));
        assert!(looks_like_image("image/webp", b"anything"));
        assert!(!looks_like_image("", b"not a picture at all"));
    }

    fn held(items: &[(&str, u64, usize)]) -> Vec<(String, Instant, usize)> {
        let base = Instant::now();
        items
            .iter()
            .map(|(url, age_secs, bytes)| {
                (
                    (*url).to_string(),
                    base - std::time::Duration::from_secs(*age_secs),
                    *bytes,
                )
            })
            .collect()
    }

    /// Rule: nothing is let go of while it all fits. This is the case
    /// that matters: an evening of listening never reaches the budget,
    /// so no cover ever blinks out and back (#129).
    #[test]
    fn artwork_that_fits_is_all_kept() {
        let art = held(&[("a", 600, 1000), ("b", 300, 1000), ("c", 1, 1000)]);
        assert!(over_budget(art, 10_000).is_empty());
    }

    /// Rule: over the budget, the oldest go first, and only as many as
    /// it takes to fit.
    #[test]
    fn the_oldest_go_until_the_rest_fit() {
        let art = held(&[
            ("oldest", 900, 1000),
            ("middle", 600, 1000),
            ("newest", 1, 1000),
        ]);
        assert_eq!(over_budget(art, 2000), vec!["oldest"]);
    }

    #[test]
    fn enough_go_to_get_under_the_budget() {
        let art = held(&[
            ("oldest", 900, 1000),
            ("middle", 600, 1000),
            ("newest", 1, 1000),
        ]);
        assert_eq!(over_budget(art, 900), vec!["oldest", "middle", "newest"]);
    }

    /// Rule: an empty gallery asks nothing of anyone.
    #[test]
    fn nothing_held_lets_nothing_go() {
        assert!(over_budget(Vec::new(), 0).is_empty());
    }
}
