//! Album art: fetched once, kept on disk, and drawn from a copy scaled to
//! the pixels each cover covers.
//!
//! [`ArtLoader`] handles `http(s)` URIs in egui's image pipeline, and the
//! `sonic:art:<size>:<id>` requests the Subsonic layer leaves in its models.
//! The first request starts a background download or disk-cache read. A size
//! limit keeps long sessions from retaining unlimited textures.
//!
//! A cover is not drawn from what the server sent. [`CoverScaler`] decodes
//! it off the UI thread and scales it to the pixels it covers on the display,
//! and that copy is what the GPU holds. Once its texture is made the
//! downloaded and decoded copies are let go of; another size of the same
//! cover reads them back from the disk cache.
//!
//! Two things are particular to a self-hosted server.
//!
//! **Cover art is a request, not a URL.** `getCoverArt` needs the credential
//! on every fetch, so the models carry `sonic:art:…` and this module builds
//! the real URL when it fetches. That keeps the credential out of anything
//! that gets cached or logged, and means a new sign-in does not invalidate
//! the art on disk: the cache is keyed by the request, not by the URL.
//! The size in a request is chosen where the cover is drawn, by
//! [`request_for`]. Artist images are the exception — the server hands over
//! pre-signed `/share/img/` URLs that need no credential, and those arrive
//! here as ordinary `http(s)` and are fetched as they are.
//!
//! **A `200` is not proof of an image.** Ask `getCoverArt` for an id it does
//! not have and it answers `HTTP 200` with an *error envelope* where the
//! bytes should be. Believing the status code would put that envelope in the
//! disk cache under the name of a cover, and every later request would read
//! it back and fail to decode. So the bytes are checked before they are kept.

use std::borrow::Cow;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use egui::load::{
    Bytes, BytesLoadResult, BytesLoader, BytesPoll, ImageLoadResult, ImageLoader, ImagePoll,
    LoadError, SizeHint,
};
use sha1::{Digest, Sha1};

use crate::api::subsonic::Credentials;
use crate::api::subsonic::auth::without_credentials;
use crate::api::subsonic::convert::{art_request_size, art_url, parse_art_url};

/// Maximum artwork bytes held in memory.
///
/// Time-based eviction does not work here: after creating a texture, egui no
/// longer requests its source bytes. Visible images were therefore evicted and
/// reloaded every two and a half minutes (#129).
///
/// Size-based eviction keeps visible images stable.
///
/// It counts the textures of painted covers, which are on the GPU, and the
/// encoded bytes of covers still waiting to be painted. A card's cover is
/// scaled to the 148 or so pixels it covers and holds about 120 KB, so a page
/// of cards stays well inside it.
const HELD_BYTES: usize = 64 * 1024 * 1024;
const MAX_ART_BYTES: usize = 8 * 1024 * 1024;

/// How long another copy of the same artwork goes on being painted without
/// a copy of a cover before that copy is let go: a panel being resized leaves
/// a copy behind at each step. It is measured from the artwork's latest
/// paint, not the clock, so copies painted together stay together however
/// long the window then sits idle.
const SUPERSEDED: Duration = Duration::from_secs(5);

/// How covers are sampled. Each is scaled to the pixels it covers before it
/// reaches the GPU, but sizes come in steps and a cover can be drawn smaller
/// for a moment, from a larger copy standing in while its own is made; a
/// plain linear filter then skips texels, so edges stair-step and fine detail
/// turns to grain. Mipmaps let the GPU read a prefiltered level instead.
/// Every painter of a cover must use these same options: egui keys its
/// textures by URI and options, so a second set would upload the cover twice.
pub const COVER_TEXTURE: egui::TextureOptions =
    egui::TextureOptions::LINEAR.with_mipmap_mode(Some(egui::TextureFilter::Linear));

/// A cover's GPU texture, RGBA, with its mipmap chain adding a third of the
/// base level. Once the texture exists the decoded image is let go (see
/// [`forget_decoded`]), so this is all a painted cover holds.
fn texture_bytes(width: usize, height: usize) -> usize {
    width
        .saturating_mul(height)
        .saturating_mul(4)
        .saturating_mul(4)
        / 3
}

/// The side, in pixels, of the copy of a cover drawn `pixels` wide: the next
/// step of 32 up, so a panel being resized makes a few copies rather than one
/// per pixel.
pub fn cover_pixels(pixels: f32) -> u32 {
    let pixels = pixels.ceil().max(1.0) as u32;
    pixels.div_ceil(32) * 32
}

/// What to fetch for `url` drawn `pixels` wide on the display. A cover-art
/// request asks the server for the size that suits those pixels (see
/// [`art_request_size`]), whatever size the model named; any other URL is
/// fetched as it is.
pub fn request_for(url: &str, pixels: f32) -> Cow<'_, str> {
    match parse_art_url(url) {
        Some((size, id)) => {
            let wanted = art_request_size(pixels);
            if wanted == size {
                Cow::Borrowed(url)
            } else {
                Cow::Owned(art_url(id, wanted))
            }
        }
        None => Cow::Borrowed(url),
    }
}

/// The URI of `url`'s artwork scaled to `pixels` on its longer side. The
/// suffix is a URL fragment, so it never reaches the server; [`CoverScaler`]
/// answers for it.
pub fn scaled_uri(url: &str, pixels: u32) -> String {
    format!("{url}{SCALED}{pixels}")
}

const SCALED: &str = "#fastsonic-px=";

/// The artwork URL and pixel size in a [`scaled_uri`].
fn parse_scaled(uri: &str) -> Option<(&str, u32)> {
    let (url, pixels) = uri.rsplit_once(SCALED)?;
    let pixels = pixels.parse().ok().filter(|pixels| *pixels > 0)?;
    handled(url).then_some((url, pixels))
}

/// The picture a request or URL is of, whatever size it asks for: copies made
/// from a cover's 128 and its 300 pixel request are copies of one picture,
/// and stand in for each other.
fn artwork(url: &str) -> Cow<'_, str> {
    match parse_art_url(url) {
        Some((_, id)) => Cow::Owned(format!("sonic:cover:{id}")),
        None => Cow::Borrowed(url),
    }
}

/// Lets go of the decoded copy of `uri` that egui's image loaders keep in
/// system memory after its texture is on the GPU. egui keeps the texture,
/// and asks for the image again only if it needs another texture of it;
/// the bytes then come back from the disk cache, off the UI thread.
///
/// Only the image loaders forget: egui's own `reduce_texture_memory` would
/// also forget the bytes loader's entry, and with it the size the artwork
/// budget evicts by, leaving the texture held for good.
pub fn forget_decoded(ctx: &egui::Context, uri: &str) {
    for loader in ctx.loaders().image.lock().iter() {
        loader.forget(uri);
    }
}

enum Entry {
    Pending,
    Ready {
        bytes: Option<Arc<[u8]>>,
        last_used: Instant,
        /// The source bytes still held.
        retained: usize,
    },
    Failed(String),
}

/// A copy of an artwork on the GPU: the video memory it holds, and when it
/// was last painted.
struct PaintedCopy {
    bytes: usize,
    painted: Instant,
}

/// The scaled copies on the GPU, by [`artwork`] and copy URI.
type Copies = HashMap<String, HashMap<String, PaintedCopy>>;

struct Inner {
    entries: Mutex<HashMap<String, Entry>>,
    /// Kept apart from `entries`, which drops a request's entry to reload
    /// its bytes, so a texture is never forgotten while it is still held.
    textures: Mutex<Copies>,
    /// For artwork on any other host: an artist image the server only
    /// names. It never carries the server's extra headers.
    http: reqwest::Client,
    /// For artwork on the server, with its extra headers, and no redirect
    /// off it (`api::subsonic::headers`).
    server_http: reqwest::Client,
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
    pub fn new(
        http: reqwest::Client,
        server_http: reqwest::Client,
        runtime: tokio::runtime::Handle,
        cache_dir: PathBuf,
    ) -> Self {
        let _ = std::fs::create_dir_all(&cache_dir);
        Self {
            inner: Arc::new(Inner {
                entries: Mutex::new(HashMap::new()),
                textures: Mutex::new(HashMap::new()),
                http,
                server_http,
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

    /// The image loader that scales this loader's artwork for painting.
    pub fn scaler(&self) -> CoverScaler {
        CoverScaler::new(self.inner.runtime.clone())
    }

    /// Bytes for `uri`, from memory, disk, or the network.
    pub async fn fetch(&self, uri: &str) -> Result<Arc<[u8]>, String> {
        self.inner.fetch(uri).await
    }

    /// Whether artwork has finished loading from disk or the network.
    pub fn is_ready(&self, url: &str) -> bool {
        matches!(
            self.inner
                .entries
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .get(url),
            Some(Entry::Ready { .. })
        )
    }

    /// Marks artwork as visible so size-based eviction keeps it stable.
    pub fn touch(&self, url: &str) {
        if let Some(Entry::Ready { last_used, .. }) = self
            .inner
            .entries
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .get_mut(url)
        {
            *last_used = Instant::now();
        }
    }

    /// Evicts failed entries, copies of a cover no longer painted at their
    /// size, and the oldest artwork above the memory limit, with every size
    /// of it asked for and every scaled copy of it on the GPU.
    pub fn evict(&self, ctx: &egui::Context) {
        let superseded = superseded(
            &mut self
                .inner
                .textures
                .lock()
                .unwrap_or_else(|p| p.into_inner()),
        );
        for uri in superseded {
            ctx.forget_image(&uri);
        }
        let (failed, letting_go) = {
            let entries = self.inner.entries.lock().unwrap_or_else(|p| p.into_inner());
            let textures = self
                .inner
                .textures
                .lock()
                .unwrap_or_else(|p| p.into_inner());
            let mut failed: Vec<String> = Vec::new();
            // Each artwork's latest use, what it holds, and its requests.
            let mut held: HashMap<String, (Instant, usize, Vec<String>)> = HashMap::new();
            for (url, entry) in entries.iter() {
                match entry {
                    // Forget failures so a later request can retry.
                    Entry::Failed(_) => failed.push(url.clone()),
                    Entry::Ready {
                        last_used,
                        retained,
                        ..
                    } => {
                        let artwork = held.entry(artwork(url).into_owned()).or_insert((
                            *last_used,
                            0,
                            Vec::new(),
                        ));
                        artwork.0 = artwork.0.max(*last_used);
                        artwork.1 += *retained;
                        artwork.2.push(url.clone());
                    }
                    Entry::Pending => {}
                }
            }
            for (key, (last_used, bytes, _)) in held.iter_mut() {
                for copy in textures.get(key).into_iter().flat_map(HashMap::values) {
                    *bytes += copy.bytes;
                    *last_used = (*last_used).max(copy.painted);
                }
            }
            let over = over_budget(
                held.iter()
                    .map(|(key, (last_used, bytes, _))| (key.clone(), *last_used, *bytes))
                    .collect(),
                HELD_BYTES,
            );
            let letting_go: Vec<(String, Vec<String>)> = over
                .into_iter()
                .filter_map(|key| {
                    let (_, _, requests) = held.remove(&key)?;
                    Some((key, requests))
                })
                .collect();
            (failed, letting_go)
        };
        for url in failed {
            ctx.forget_image(&url);
            self.forget(&url);
        }
        for (key, requests) in letting_go {
            let copies = self
                .inner
                .textures
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .remove(&key);
            for uri in copies.into_iter().flat_map(HashMap::into_keys) {
                ctx.forget_image(&uri);
            }
            for url in requests {
                ctx.forget_image(&url);
                self.forget(&url);
            }
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

    /// Starts the download for `url` while nothing is drawing it, so the
    /// media controls have a file to hand the platform, and answers whether
    /// this call is what started it.
    ///
    /// Artwork already held, already on its way, or addressed by a scheme
    /// this loader does not answer for is left alone.
    pub fn prefetch(&self, ctx: &egui::Context, url: &str) -> bool {
        if !handled(url) {
            return false;
        }
        let mut entries = self.inner.entries.lock().unwrap_or_else(|p| p.into_inner());
        if entries.contains_key(url) {
            return false;
        }
        entries.insert(url.to_string(), Entry::Pending);
        drop(entries);
        self.inner.start(ctx, url.to_string());
        true
    }

    /// Drops the held source bytes once egui has made a texture. The disk
    /// cache remains for later reloads. Answers whether any bytes were still
    /// held, which is true once per load: the frame the texture first
    /// appeared.
    pub fn release_bytes(&self, url: &str) -> bool {
        self.inner.drop_bytes(url)
    }

    /// Records that the copy at `uri` of `url`'s artwork was painted, and
    /// answers whether this is the first time: the frame egui made its
    /// texture, and its decoded copy can be let go of.
    pub fn note_texture(&self, url: &str, uri: &str, width: usize, height: usize) -> bool {
        let now = Instant::now();
        let key = artwork(url);
        let mut textures = self
            .inner
            .textures
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        if let Some(copy) = textures
            .get_mut(key.as_ref())
            .and_then(|copies| copies.get_mut(uri))
        {
            copy.painted = now;
            return false;
        }
        textures.entry(key.into_owned()).or_default().insert(
            uri.to_string(),
            PaintedCopy {
                bytes: texture_bytes(width, height),
                painted: now,
            },
        );
        true
    }

    /// Forgets which copies are on the GPU, for a new window whose egui
    /// holds none of them yet: each is then let go of as it is made again.
    pub fn forget_textures(&self) {
        self.inner
            .textures
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .clear();
    }

    /// The copy of `url`'s artwork already on the GPU that best stands in
    /// for one `pixels` wide, while that one is made: a cover changing size,
    /// one asked of the server at a new size, or one whose bytes are coming
    /// back from the disk for another size, shows its last picture instead
    /// of the placeholder.
    pub fn nearest_copy(&self, url: &str, pixels: u32) -> Option<String> {
        let textures = self
            .inner
            .textures
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        nearest(
            textures
                .get(artwork(url).as_ref())?
                .keys()
                .map(String::as_str),
            pixels,
        )
        .map(str::to_string)
    }

    /// Whether a copy of `url`'s artwork is on the GPU.
    pub fn has_copy(&self, url: &str) -> bool {
        self.inner
            .textures
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .get(artwork(url).as_ref())
            .is_some_and(|copies| !copies.is_empty())
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

/// Takes out of `textures` the copies of a cover last painted more than
/// [`SUPERSEDED`] before another copy of it, and answers their URIs. A cover
/// with a single copy is left to the memory budget.
fn superseded(textures: &mut Copies) -> Vec<String> {
    let mut letting_go = Vec::new();
    for copies in textures.values_mut() {
        let Some(latest) = copies.values().map(|copy| copy.painted).max() else {
            continue;
        };
        copies.retain(|uri, copy| {
            let keep = latest.duration_since(copy.painted) < SUPERSEDED;
            if !keep {
                letting_go.push(uri.clone());
            }
            keep
        });
    }
    letting_go
}

/// The pixels a scaled copy really has: the size it was made at, or the
/// size the server was asked for when that is smaller, since a copy is
/// never enlarged.
fn copy_pixels(uri: &str) -> Option<u32> {
    let (url, pixels) = parse_scaled(uri)?;
    Some(parse_art_url(url).map_or(pixels, |(asked, _)| pixels.min(asked)))
}

/// Of the scaled copies at `uris`, the smallest at least `pixels` wide, so
/// no detail is missing, or else the largest there is. A copy with fewer
/// than half the pixels is none: enlarged that far it is blocky, and a page
/// header has its softened thumbnail to show instead.
fn nearest<'a>(uris: impl Iterator<Item = &'a str>, pixels: u32) -> Option<&'a str> {
    uris.filter_map(|uri| Some((copy_pixels(uri)?, uri)))
        .filter(|&(size, _)| size.saturating_mul(2) >= pixels)
        .min_by_key(|&(size, _)| {
            if size >= pixels {
                (false, size)
            } else {
                (true, u32::MAX - size)
            }
        })
        .map(|(_, uri)| uri)
}

/// Which artwork to let go of so that what is kept fits `budget`,
/// oldest first.
///
/// "Oldest" is when a cover was last painted, or when egui last needed
/// its bytes for one still waiting to be. Being roughly right about which
/// to drop only matters once there is far more artwork than any window is
/// showing, which is why the budget is generous.
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

    /// A request on the server goes out with its extra headers, through the
    /// client that keeps them there; one anywhere else goes without.
    fn request(&self, url: &str) -> reqwest::RequestBuilder {
        let credentials = self
            .credentials
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .clone();
        match credentials.filter(|credentials| credentials.serves(url)) {
            Some(credentials) => self.server_http.get(url).headers(credentials.headers.map()),
            None => self.http.get(url),
        }
    }

    async fn fetch(self: &Arc<Self>, uri: &str) -> Result<Arc<[u8]>, String> {
        if let Some(Entry::Ready {
            bytes: Some(bytes), ..
        }) = self
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
        let bytes: Arc<[u8]> = match cached {
            Some(bytes) if !bytes.is_empty() => Arc::from(bytes),
            _ => {
                let url = self.resolve(uri)?;
                let mut response = self
                    .request(&url)
                    .send()
                    .await
                    .map_err(|error| without_credentials(error).to_string())?;
                if !response.status().is_success() {
                    return Err(format!("artwork request failed: {}", response.status()));
                }
                let content_type = response
                    .headers()
                    .get(reqwest::header::CONTENT_TYPE)
                    .and_then(|value| value.to_str().ok())
                    .unwrap_or_default()
                    .to_string();
                // Refuse oversized artwork as it arrives, not once it is all
                // in memory.
                if response
                    .content_length()
                    .is_some_and(|length| length > MAX_ART_BYTES as u64)
                {
                    return Err("artwork is too large".to_string());
                }
                let mut bytes = Vec::new();
                while let Some(chunk) = response
                    .chunk()
                    .await
                    .map_err(|error| without_credentials(error).to_string())?
                {
                    if bytes.len() + chunk.len() > MAX_ART_BYTES {
                        return Err("artwork is too large".to_string());
                    }
                    bytes.extend_from_slice(&chunk);
                }
                if !looks_like_image(&content_type, &bytes) {
                    // An error envelope where a cover should be. Say so, and
                    // above all do not write it to the cache under the name
                    // of a picture.
                    return Err(server_art_error(&bytes));
                }
                // The loader and the file worker share one payload.
                let bytes: Arc<[u8]> = Arc::from(bytes.as_ref());
                let write_path = path.clone();
                let payload = Arc::clone(&bytes);
                self.runtime.spawn_blocking(move || {
                    let temporary = write_path.with_extension("part");
                    if std::fs::write(&temporary, &payload).is_ok() {
                        let _ = std::fs::rename(&temporary, &write_path);
                    }
                });
                bytes
            }
        };
        Ok(bytes)
    }

    fn start(self: &Arc<Self>, ctx: &egui::Context, url: String) {
        let loader = Arc::clone(self);
        let ctx = ctx.clone();
        self.runtime.spawn(async move {
            let result = loader.fetch(&url).await;
            let entry = match result {
                Ok(bytes) => Entry::Ready {
                    retained: bytes.len(),
                    bytes: Some(bytes),
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

    fn drop_bytes(&self, url: &str) -> bool {
        if let Some(Entry::Ready {
            bytes, retained, ..
        }) = self
            .entries
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .get_mut(url)
            && let Some(held) = bytes.take()
        {
            *retained = retained.saturating_sub(held.len());
            return true;
        }
        false
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
            Some(Entry::Ready {
                bytes: Some(bytes),
                last_used,
                ..
            }) => {
                *last_used = Instant::now();
                Ok(BytesPoll::Ready {
                    size: None,
                    bytes: Bytes::Shared(Arc::clone(bytes)),
                    mime: None,
                })
            }
            Some(Entry::Ready {
                bytes: None,
                last_used,
                ..
            }) => {
                *last_used = Instant::now();
                entries.insert(uri.to_string(), Entry::Pending);
                drop(entries);
                self.inner.start(ctx, uri.to_string());
                Ok(BytesPoll::Pending { size: None })
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
            .textures
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .clear();
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
                Entry::Ready {
                    bytes: Some(bytes), ..
                } => bytes.len(),
                _ => 0,
            })
            .sum()
    }
}

/// Whether this loader answers for `uri`: a URL the server handed over, or a
/// cover-art request the Subsonic layer left for us to build.
pub fn handled(uri: &str) -> bool {
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

/// Blur applied to the full-window lyrics backdrop.
const LYRICS_BLUR: f32 = 9.0;

/// Blur applied to a library thumbnail standing in for a cover.
///
/// Softening happens at the same bounded size as the backdrop, so this is
/// weaker than the backdrop's radius: the thumbnail already carries little
/// detail, and a heavier blur smears it into flat colour.
const COVER_BLUR: f32 = 1.5;

/// Softened covers held at once, for library artwork seen recently.
const HELD_COVERS: usize = 64;

#[derive(Default)]
pub struct LyricsBackdrop {
    uri: Option<String>,
    requested: bool,
    retry_at: Option<Instant>,
    pending: Option<std::sync::mpsc::Receiver<Result<Option<egui::ColorImage>, String>>>,
    texture: Option<egui::TextureHandle>,
}

/// How long a backdrop whose artwork failed to download waits to ask again.
const BACKDROP_RETRY: Duration = Duration::from_secs(3);

impl LyricsBackdrop {
    /// The blurred backdrop for `uri`.
    ///
    /// The artwork is fetched here, from the disk cache or the network,
    /// rather than read from the bytes the image loader holds: a cover
    /// painted on the same screen releases those bytes as soon as its
    /// texture exists, and a backdrop waiting on them could wait forever.
    /// Until the new song's backdrop is ready the previous one stays, so a
    /// change of song never flashes black.
    pub fn texture(
        &mut self,
        ctx: &egui::Context,
        loader: &ArtLoader,
        uri: Option<&str>,
    ) -> Option<&egui::TextureHandle> {
        if self.uri.as_deref() != uri {
            *self = Self {
                uri: uri.map(str::to_owned),
                texture: self.texture.take().filter(|_| uri.is_some()),
                ..Default::default()
            };
        }
        let uri = uri?;
        let due = self.retry_at.is_none_or(|at| Instant::now() >= at);
        if !self.requested && due && handled(uri) {
            self.requested = true;
            self.retry_at = None;
            let (tx, rx) = std::sync::mpsc::channel();
            self.pending = Some(rx);
            let ctx = ctx.clone();
            let inner = Arc::clone(&loader.inner);
            let uri = uri.to_string();
            loader.inner.runtime.spawn(async move {
                let result = match inner.fetch(&uri).await {
                    Ok(bytes) => inner
                        .runtime
                        .spawn_blocking(move || blurred_background(&bytes, LYRICS_BLUR))
                        .await
                        .map_err(|error| error.to_string()),
                    Err(error) => Err(error),
                };
                let _ = tx.send(result);
                ctx.request_repaint();
            });
        }
        if let Some(receiver) = &self.pending {
            match receiver.try_recv() {
                Ok(Ok(image)) => {
                    self.texture = image.map(|image| {
                        ctx.load_texture("lyrics-backdrop", image, egui::TextureOptions::LINEAR)
                    });
                    self.pending = None;
                }
                Ok(Err(_)) | Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    self.pending = None;
                    self.requested = false;
                    self.retry_at = Some(Instant::now() + BACKDROP_RETRY);
                    ctx.request_repaint_after(BACKDROP_RETRY);
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => {}
            }
        }
        self.texture.as_ref()
    }
}

/// Softened artwork for covers that are still loading, one entry per URL.
///
/// A library row's small thumbnail stands in for the cover its page opens
/// with: enlarged and blurred until the full artwork arrives. Blurring
/// happens off the UI thread and the result is kept, so a cover that is
/// opened, left, and opened again shows softened art at once.
struct SoftenedCover {
    texture: egui::TextureHandle,
    last_used: Instant,
}

/// A softened cover, `None` when the artwork could not be decoded, or why it
/// could not be read for now.
type SoftenedResult = (String, Result<Option<egui::ColorImage>, String>);

pub struct SoftenedCovers {
    textures: HashMap<String, SoftenedCover>,
    pending: HashSet<String>,
    failed: HashSet<String>,
    /// Artwork that could not be read, and when to ask for it again.
    retry_at: HashMap<String, Instant>,
    ready_tx: std::sync::mpsc::Sender<SoftenedResult>,
    ready_rx: std::sync::mpsc::Receiver<SoftenedResult>,
}

impl Default for SoftenedCovers {
    fn default() -> Self {
        let (ready_tx, ready_rx) = std::sync::mpsc::channel();
        Self {
            textures: HashMap::new(),
            pending: HashSet::new(),
            failed: HashSet::new(),
            retry_at: HashMap::new(),
            ready_tx,
            ready_rx,
        }
    }
}

impl SoftenedCovers {
    fn receive_ready(&mut self, ctx: &egui::Context) {
        while let Ok((ready_uri, result)) = self.ready_rx.try_recv() {
            self.pending.remove(&ready_uri);
            let image = match result {
                Ok(image) => image,
                Err(_) => {
                    self.retry_at
                        .insert(ready_uri, Instant::now() + BACKDROP_RETRY);
                    ctx.request_repaint_after(BACKDROP_RETRY);
                    continue;
                }
            };
            let Some(image) = image else {
                self.failed.insert(ready_uri);
                continue;
            };
            if self.textures.len() >= HELD_COVERS
                && let Some(oldest) = self
                    .textures
                    .iter()
                    .min_by_key(|(_, cover)| cover.last_used)
                    .map(|(uri, _)| uri.clone())
            {
                self.textures.remove(&oldest);
            }
            let texture = ctx.load_texture("softened-cover", image, egui::TextureOptions::LINEAR);
            self.textures.insert(
                ready_uri,
                SoftenedCover {
                    texture,
                    last_used: Instant::now(),
                },
            );
        }
    }

    /// The softened cover for `uri`, blurring it in the background the first
    /// time it is asked for.
    pub fn texture(
        &mut self,
        ctx: &egui::Context,
        loader: &ArtLoader,
        uri: &str,
    ) -> Option<egui::TextureHandle> {
        self.receive_ready(ctx);
        if let Some(cover) = self.textures.get_mut(uri) {
            cover.last_used = Instant::now();
            return Some(cover.texture.clone());
        }
        if self.failed.contains(uri)
            || self.pending.contains(uri)
            || self.pending.len() >= HELD_COVERS
            || self
                .retry_at
                .get(uri)
                .is_some_and(|at| Instant::now() < *at)
        {
            return None;
        }
        if !handled(uri) {
            // No loader will ever answer for it, so it is not asked again.
            self.failed.insert(uri.to_string());
            return None;
        }
        // Read from the disk cache, or the network, rather than from egui's
        // decoded image: painted covers let that go once their texture
        // exists, and their encoded bytes along with it.
        self.retry_at.remove(uri);
        self.pending.insert(uri.to_string());
        let ready_tx = self.ready_tx.clone();
        let ready_uri = uri.to_string();
        let ctx = ctx.clone();
        let inner = Arc::clone(&loader.inner);
        loader.inner.runtime.spawn(async move {
            let result = match inner.fetch(&ready_uri).await {
                Ok(bytes) => inner
                    .runtime
                    .spawn_blocking(move || blurred_background(&bytes, COVER_BLUR))
                    .await
                    .map_err(|error| error.to_string()),
                Err(error) => Err(error),
            };
            let _ = ready_tx.send((ready_uri, result));
            ctx.request_repaint();
        });
        None
    }
}

fn blurred_background(bytes: &[u8], sigma: f32) -> Option<egui::ColorImage> {
    Some(blurred_image(decode(bytes).ok()?, sigma))
}

fn blurred_image(image: image::DynamicImage, sigma: f32) -> egui::ColorImage {
    let image = image.thumbnail(256, 256).blur(sigma).to_rgba8();
    egui::ColorImage::from_rgba_unmultiplied(
        [image.width() as usize, image.height() as usize],
        image.as_raw(),
    )
}

/// Decodes artwork, within the same bounds as every other decode here.
fn decode(bytes: &[u8]) -> Result<image::DynamicImage, String> {
    if bytes.len() > MAX_ART_BYTES {
        return Err("artwork is too large".to_string());
    }
    let mut reader = image::ImageReader::new(std::io::Cursor::new(bytes))
        .with_guessed_format()
        .map_err(|error| error.to_string())?;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(8192);
    limits.max_image_height = Some(8192);
    limits.max_alloc = Some(64 * 1024 * 1024);
    reader.limits(limits);
    reader.decode().map_err(|error| error.to_string())
}

/// `image` scaled so its longer side is `pixels`, with a Lanczos filter:
/// every source pixel counts towards the result, so fine detail such as a
/// logo stays sharp instead of breaking up. Artwork already that small or
/// smaller is left as it is rather than enlarged.
///
/// A large reduction first averages the artwork down to twice the size
/// wanted, which is cheap: Lanczos then weighs a few source pixels for each
/// pixel rather than dozens, for the same picture.
fn scaled(image: image::DynamicImage, pixels: u32) -> egui::ColorImage {
    let longer = image.width().max(image.height());
    let image = if longer > pixels {
        let scale = pixels as f32 / longer as f32;
        let width = ((image.width() as f32 * scale).round() as u32).max(1);
        let height = ((image.height() as f32 * scale).round() as u32).max(1);
        let image = if longer > pixels * 2 {
            image.thumbnail(width * 2, height * 2)
        } else {
            image
        };
        // Through DynamicImage, whose resize is compiled in the image crate
        // and so optimised even in a development build of this one.
        image
            .resize_exact(width, height, image::imageops::FilterType::Lanczos3)
            .to_rgba8()
    } else {
        image.to_rgba8()
    };
    egui::ColorImage::from_rgba_unmultiplied(
        [image.width() as usize, image.height() as usize],
        image.as_raw(),
    )
}

/// How many covers are scaled at once. Each takes a core while it lasts. A
/// fast scroll asks for dozens in a moment, and scaling them all at once
/// left the interface and the rest of the machine short of cores.
const SCALING_AT_ONCE: usize = 2;

/// How long a copy waits its turn after it was last asked for. A cover
/// scrolled past before its turn came is not made at all, and one made after
/// it was scrolled past is let go of rather than held.
const SCALING_STALE: Duration = Duration::from_millis(500);

/// A copy of a cover in the scaler: waiting its turn, with the artwork's
/// bytes and when a frame last asked for it, being made, or made, with when
/// it was made or last asked for. egui asks for a made copy once, to make
/// its texture, and the painter then lets it go; one nobody asks for is a
/// cover that left the screen while it was made.
enum Slot {
    Waiting {
        asked: Instant,
        bytes: Bytes,
        pixels: u32,
    },
    Working,
    Ready {
        result: Result<Arc<egui::ColorImage>, String>,
        asked: Instant,
    },
}

#[derive(Default)]
struct Scaling {
    slots: HashMap<String, Slot>,
    /// Threads making copies now, at most [`SCALING_AT_ONCE`].
    workers: usize,
}

/// Makes the copy of a cover that egui uploads: the artwork from
/// [`ArtLoader`], decoded and scaled to the pixels it is drawn at, off the
/// UI thread. The GPU then holds a texture of that size, not of the source.
///
/// A few copies are made at a time, those asked for most recently first.
pub struct CoverScaler {
    runtime: tokio::runtime::Handle,
    state: Arc<Mutex<Scaling>>,
}

impl CoverScaler {
    pub fn new(runtime: tokio::runtime::Handle) -> Self {
        Self {
            runtime,
            state: Arc::default(),
        }
    }
}

/// Makes the waiting copies, the most recently asked for first, until none
/// is left, and drops those nobody has asked for lately.
fn scale_waiting(state: &Mutex<Scaling>, ctx: &egui::Context) {
    loop {
        let (uri, bytes, pixels) = {
            let mut state = state.lock().unwrap_or_else(|p| p.into_inner());
            let now = Instant::now();
            let before = state.slots.len();
            state.slots.retain(|_, slot| match slot {
                Slot::Waiting { asked, .. } => now.duration_since(*asked) < SCALING_STALE,
                _ => true,
            });
            if state.slots.len() < before {
                // A cover still on screen is asked for again on the next
                // frame, and waits its turn once more.
                ctx.request_repaint();
            }
            let next = state
                .slots
                .iter()
                .filter_map(|(uri, slot)| match slot {
                    Slot::Waiting { asked, .. } => Some((*asked, uri)),
                    _ => None,
                })
                .max_by_key(|(asked, _)| *asked)
                .map(|(_, uri)| uri.clone());
            let Some(uri) = next else {
                state.workers -= 1;
                return;
            };
            match state.slots.insert(uri.clone(), Slot::Working) {
                Some(Slot::Waiting { bytes, pixels, .. }) => (uri, bytes, pixels),
                _ => unreachable!("the copy chosen was waiting"),
            }
        };
        // A copy that cannot be made is an error, never a copy that stays
        // pending for good.
        let result = std::panic::catch_unwind(|| {
            decode(&bytes).map(|image| Arc::new(scaled(image, pixels)))
        })
        .unwrap_or_else(|_| Err("the cover could not be scaled".to_string()));
        let mut state = state.lock().unwrap_or_else(|p| p.into_inner());
        // A copy forgotten while it was being made stays forgotten.
        if let Some(slot @ Slot::Working) = state.slots.get_mut(&uri) {
            *slot = Slot::Ready {
                result,
                asked: Instant::now(),
            };
            drop(state);
            ctx.request_repaint();
        }
    }
}

impl ImageLoader for CoverScaler {
    fn id(&self) -> &str {
        "fastsonic::CoverScaler"
    }

    fn load(&self, ctx: &egui::Context, uri: &str, _: SizeHint) -> ImageLoadResult {
        let Some((url, pixels)) = parse_scaled(uri) else {
            return Err(LoadError::NotSupported);
        };
        match self
            .state
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .slots
            .get_mut(uri)
        {
            Some(Slot::Ready { result, asked }) => {
                *asked = Instant::now();
                return match result {
                    Ok(image) => Ok(ImagePoll::Ready {
                        image: Arc::clone(image),
                    }),
                    Err(error) => Err(LoadError::Loading(error.clone())),
                };
            }
            Some(Slot::Waiting { asked, .. }) => {
                *asked = Instant::now();
                return Ok(ImagePoll::Pending { size: None });
            }
            Some(Slot::Working) => return Ok(ImagePoll::Pending { size: None }),
            None => {}
        }
        let bytes = match ctx.try_load_bytes(url)? {
            BytesPoll::Pending { .. } => return Ok(ImagePoll::Pending { size: None }),
            BytesPoll::Ready { bytes, .. } => bytes,
        };
        let mut state = self.state.lock().unwrap_or_else(|p| p.into_inner());
        state.slots.insert(
            uri.to_string(),
            Slot::Waiting {
                asked: Instant::now(),
                bytes,
                pixels,
            },
        );
        if state.workers < SCALING_AT_ONCE {
            state.workers += 1;
            drop(state);
            let state = Arc::clone(&self.state);
            let ctx = ctx.clone();
            self.runtime
                .spawn_blocking(move || scale_waiting(&state, &ctx));
        }
        Ok(ImagePoll::Pending { size: None })
    }

    fn forget(&self, uri: &str) {
        self.state
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .slots
            .remove(uri);
    }

    fn forget_all(&self) {
        self.state
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .slots
            .clear();
    }

    fn byte_size(&self) -> usize {
        self.state
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .slots
            .values()
            .map(|slot| match slot {
                Slot::Ready {
                    result: Ok(image), ..
                } => image.pixels.len() * 4,
                _ => 0,
            })
            .sum()
    }

    fn has_pending(&self) -> bool {
        self.state
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .slots
            .values()
            .any(|slot| !matches!(slot, Slot::Ready { .. }))
    }

    fn end_pass(&self, _: u64) {
        let now = Instant::now();
        self.state
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .slots
            .retain(|_, slot| match slot {
                Slot::Ready { asked, .. } => now.duration_since(*asked) < SCALING_STALE,
                _ => true,
            });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn cached_png(loader: &ArtLoader, uri: &str, rgb: [u8; 3]) {
        let mut encoded = std::io::Cursor::new(Vec::new());
        image::RgbImage::from_pixel(64, 64, image::Rgb(rgb))
            .write_to(&mut encoded, image::ImageFormat::Png)
            .unwrap();
        std::fs::write(loader.inner.cache_path(uri), encoded.into_inner()).unwrap();
    }

    #[test]
    fn prefetching_starts_one_download_and_not_another() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .build()
            .expect("a runtime to hand the loader");
        let dir = std::env::temp_dir().join(format!("fastsonic-prefetch-{}", std::process::id()));
        let loader = backdrop_loader(&runtime, dir.clone());
        let ctx = egui::Context::default();
        let url = "sonic:art:640:never-drawn";
        assert!(loader.prefetch(&ctx, url), "nobody has asked for it yet");
        assert!(!loader.prefetch(&ctx, url), "it is already on its way");
        // A scheme the loader does not answer for is refused, as in `load`,
        // and nothing is remembered about it.
        let local = "file:///tmp/cover.jpg";
        assert!(!loader.prefetch(&ctx, local));
        assert!(
            !loader
                .inner
                .entries
                .lock()
                .expect("the entries")
                .contains_key(local)
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn backdrop_loader(runtime: &tokio::runtime::Runtime, dir: PathBuf) -> ArtLoader {
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        ArtLoader::new(
            crate::http_client_builder().build().unwrap(),
            crate::http_client_builder().build().unwrap(),
            runtime.handle().clone(),
            dir,
        )
    }

    /// Calls `texture` until `ready` accepts its answer, as frames would.
    fn backdrop_frames(
        runtime: &tokio::runtime::Runtime,
        backdrop: &mut LyricsBackdrop,
        ctx: &egui::Context,
        loader: &ArtLoader,
        uri: Option<&str>,
        mut each_frame: impl FnMut(),
        ready: impl Fn(Option<&egui::TextureHandle>) -> bool,
    ) {
        runtime.block_on(async {
            tokio::time::timeout(Duration::from_secs(10), async {
                loop {
                    each_frame();
                    if ready(backdrop.texture(ctx, loader, uri)) {
                        break;
                    }
                    tokio::time::sleep(Duration::from_millis(1)).await;
                }
            })
            .await
            .expect("the backdrop settles");
        });
    }

    /// A cover painted beside the backdrop releases the loader's bytes every
    /// frame once its texture exists. The backdrop must still appear.
    #[test]
    fn lyrics_backdrop_appears_while_a_cover_releases_the_same_artwork() {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let dir = std::env::temp_dir().join(format!("fastsonic-backdrop-{}", std::process::id()));
        let loader = backdrop_loader(&runtime, dir.clone());
        let ctx = egui::Context::default();
        let uri = "https://music.example/share/img/backdrop";
        cached_png(&loader, uri, [200, 40, 40]);
        loader.inner.entries.lock().unwrap().insert(
            uri.into(),
            Entry::Ready {
                retained: 0,
                bytes: None,
                last_used: Instant::now(),
            },
        );
        let mut backdrop = LyricsBackdrop::default();
        backdrop_frames(
            &runtime,
            &mut backdrop,
            &ctx,
            &loader,
            Some(uri),
            || {
                loader.release_bytes(uri);
            },
            |texture| texture.is_some(),
        );
        std::fs::remove_dir_all(dir).unwrap();
    }

    /// A new song keeps the old backdrop until its own is ready, a failed
    /// download is asked for again, and no song clears the backdrop.
    #[test]
    fn lyrics_backdrop_holds_the_previous_song_and_retries_failures() {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let dir =
            std::env::temp_dir().join(format!("fastsonic-backdrop-retry-{}", std::process::id()));
        let loader = backdrop_loader(&runtime, dir.clone());
        let ctx = egui::Context::default();
        let first = "https://music.example/share/img/first";
        // Nothing listens here, so the first attempt at this song fails.
        let second = "http://127.0.0.1:9/second";
        cached_png(&loader, first, [200, 40, 40]);
        let mut backdrop = LyricsBackdrop::default();
        backdrop_frames(
            &runtime,
            &mut backdrop,
            &ctx,
            &loader,
            Some(first),
            || {},
            |t| t.is_some(),
        );
        let shown = backdrop.texture.as_ref().unwrap().id();

        backdrop_frames(
            &runtime,
            &mut backdrop,
            &ctx,
            &loader,
            Some(second),
            || {},
            |t| {
                assert_eq!(t.map(|t| t.id()), Some(shown), "the old backdrop stays");
                true
            },
        );
        runtime.block_on(async {
            tokio::time::timeout(Duration::from_secs(10), async {
                while backdrop.retry_at.is_none() {
                    backdrop.texture(&ctx, &loader, Some(second));
                    tokio::time::sleep(Duration::from_millis(1)).await;
                }
            })
            .await
            .expect("the failed download is noticed");
        });
        assert_eq!(backdrop.texture.as_ref().map(|t| t.id()), Some(shown));

        cached_png(&loader, second, [40, 40, 200]);
        backdrop.retry_at = Some(Instant::now());
        backdrop_frames(
            &runtime,
            &mut backdrop,
            &ctx,
            &loader,
            Some(second),
            || {},
            |t| t.is_some_and(|t| t.id() != shown),
        );

        assert!(backdrop.texture(&ctx, &loader, None).is_none());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn blurred_background_rejects_oversized_decode_before_thumbnailing() {
        let image = image::RgbImage::from_pixel(8193, 1, image::Rgb([20, 30, 40]));
        let mut bytes = std::io::Cursor::new(Vec::new());
        image.write_to(&mut bytes, image::ImageFormat::Png).unwrap();
        assert!(blurred_background(bytes.get_ref(), LYRICS_BLUR).is_none());
    }

    #[test]
    fn blurred_background_blurs_edges_and_bounds_texture_size() {
        let image = image::RgbImage::from_fn(640, 320, |x, _| {
            if x < 320 {
                image::Rgb([255, 0, 0])
            } else {
                image::Rgb([0, 0, 255])
            }
        });
        let mut bytes = std::io::Cursor::new(Vec::new());
        image.write_to(&mut bytes, image::ImageFormat::Png).unwrap();
        let background = blurred_background(bytes.get_ref(), LYRICS_BLUR).expect("valid artwork");
        assert_eq!(background.size, [256, 128]);
        let center = background.pixels[64 * 256 + 128];
        assert!(
            center.r() > 40 && center.b() > 40,
            "the edge must be blurred: {center:?}"
        );
        assert!(
            background.pixels[0].r() > 240,
            "the cover's colors remain recognizable"
        );
        assert!(blurred_background(b"broken artwork", LYRICS_BLUR).is_none());
    }

    fn softened_test_image() -> egui::ColorImage {
        egui::ColorImage::filled([1, 1], egui::Color32::WHITE)
    }

    /// A painted cover keeps only its texture: the encoded bytes and the
    /// decoded image both go, and a softened cover of the same artwork is
    /// still made, from the disk cache.
    #[test]
    fn painted_covers_keep_only_their_texture() {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let dir =
            std::env::temp_dir().join(format!("fastsonic-painted-texture-{}", std::process::id()));
        let loader = backdrop_loader(&runtime, dir.clone());
        let ctx = egui::Context::default();
        egui_extras::install_image_loaders(&ctx);
        ctx.add_bytes_loader(Arc::new(loader.clone()));
        let uri = "sonic:art:64:painted";
        cached_png(&loader, uri, [20, 30, 40]);
        let bytes: Arc<[u8]> = std::fs::read(loader.inner.cache_path(uri)).unwrap().into();
        loader.inner.entries.lock().unwrap().insert(
            uri.into(),
            Entry::Ready {
                retained: bytes.len(),
                bytes: Some(bytes),
                last_used: Instant::now(),
            },
        );
        let decoded_bytes = |ctx: &egui::Context| -> usize {
            ctx.loaders()
                .image
                .lock()
                .iter()
                .map(|loader| loader.byte_size())
                .sum()
        };
        let texture_ready = |ctx: &egui::Context| {
            matches!(
                ctx.try_load_texture(uri, COVER_TEXTURE, Default::default()),
                Ok(egui::load::TexturePoll::Ready { .. })
            )
        };
        runtime.block_on(async {
            tokio::time::timeout(Duration::from_secs(10), async {
                while !texture_ready(&ctx) {
                    tokio::time::sleep(Duration::from_millis(1)).await;
                }
                assert!(decoded_bytes(&ctx) > 0, "egui decoded the cover");
                // What painting the cover does on the frame its texture appears.
                assert!(loader.release_bytes(uri));
                forget_decoded(&ctx, uri);
                assert!(!loader.release_bytes(uri), "only the first frame releases");
                assert_eq!(decoded_bytes(&ctx), 0, "the decoded copy is let go");
                assert!(texture_ready(&ctx), "the texture stays");

                let mut covers = SoftenedCovers::default();
                loop {
                    let texture = covers.texture(&ctx, &loader, uri);
                    assert!(
                        matches!(
                            loader.inner.entries.lock().unwrap().get(uri),
                            Some(Entry::Ready { bytes: None, .. })
                        ),
                        "softening must not restart the byte loader"
                    );
                    if let Some(texture) = texture {
                        assert_eq!(texture.size(), [256, 256]);
                        break;
                    }
                    tokio::time::sleep(Duration::from_millis(1)).await;
                }
                assert!(covers.pending.is_empty());
            })
            .await
            .expect("a painted cover is softened from the disk cache");
        });
        std::fs::remove_dir_all(dir).unwrap();
    }

    /// Artwork that cannot be read right now, offline say, is asked for
    /// again a little later rather than given up on.
    #[test]
    fn softened_cover_retries_artwork_it_could_not_read() {
        let ctx = egui::Context::default();
        let mut covers = SoftenedCovers::default();
        covers.pending.insert("offline".to_string());
        covers
            .ready_tx
            .send(("offline".to_string(), Err("no network".to_string())))
            .unwrap();
        covers.receive_ready(&ctx);
        assert!(covers.pending.is_empty());
        assert!(!covers.failed.contains("offline"));
        assert!(covers.retry_at.contains_key("offline"));
    }

    #[test]
    fn softened_cover_pending_requests_are_deduplicated() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .build()
            .expect("a runtime to hand the loader");
        let dir =
            std::env::temp_dir().join(format!("fastsonic-softened-pending-{}", std::process::id()));
        let loader = backdrop_loader(&runtime, dir.clone());
        let ctx = egui::Context::default();
        let mut covers = SoftenedCovers::default();
        covers.pending.insert("pending".to_string());

        assert!(covers.texture(&ctx, &loader, "pending").is_none());
        assert_eq!(covers.pending.len(), 1);

        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn softened_cover_completion_and_failure_clear_pending_state() {
        let ctx = egui::Context::default();
        let mut covers = SoftenedCovers::default();
        covers.pending.insert("ready".to_string());
        covers.pending.insert("broken".to_string());
        covers
            .ready_tx
            .send(("ready".to_string(), Ok(Some(softened_test_image()))))
            .unwrap();
        covers
            .ready_tx
            .send(("broken".to_string(), Ok(None)))
            .unwrap();

        covers.receive_ready(&ctx);

        assert!(covers.pending.is_empty());
        assert!(covers.textures.contains_key("ready"));
        assert!(covers.failed.contains("broken"));
    }

    #[test]
    fn softened_cover_completion_evicts_the_oldest_texture() {
        let ctx = egui::Context::default();
        let mut covers = SoftenedCovers::default();
        let now = Instant::now();
        for index in 0..HELD_COVERS {
            covers.textures.insert(
                format!("cover-{index}"),
                SoftenedCover {
                    texture: ctx.load_texture(
                        format!("cover-{index}"),
                        softened_test_image(),
                        egui::TextureOptions::LINEAR,
                    ),
                    last_used: now + Duration::from_secs(index as u64),
                },
            );
        }
        covers.pending.insert("fresh".to_string());
        covers
            .ready_tx
            .send(("fresh".to_string(), Ok(Some(softened_test_image()))))
            .unwrap();

        covers.receive_ready(&ctx);

        assert_eq!(covers.textures.len(), HELD_COVERS);
        assert!(!covers.textures.contains_key("cover-0"));
        assert!(covers.textures.contains_key("fresh"));
    }

    /// A scheme the artwork loader does not answer for never softens, and
    /// is not asked about again.
    #[test]
    fn softened_cover_gives_up_on_unsupported_art() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .build()
            .expect("a runtime to hand the loader");
        let dir = std::env::temp_dir().join(format!(
            "fastsonic-softened-unsupported-{}",
            std::process::id()
        ));
        let loader = backdrop_loader(&runtime, dir.clone());
        let ctx = egui::Context::default();
        let mut covers = SoftenedCovers::default();

        assert!(covers.texture(&ctx, &loader, "nowhere:cover").is_none());
        assert!(covers.failed.contains("nowhere:cover"));
        assert!(covers.pending.is_empty());

        let _ = std::fs::remove_dir_all(dir);
    }

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

    /// Without a declared length the body runs until the connection closes.
    async fn serve_artwork(
        bytes: Vec<u8>,
        declare_length: bool,
    ) -> (String, tokio::task::JoinHandle<()>) {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/cover", listener.local_addr().unwrap());
        let length = if declare_length {
            format!("Content-Length: {}\r\n", bytes.len())
        } else {
            String::new()
        };
        let header = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: image/jpeg\r\n{length}Connection: close\r\n\r\n"
        );
        let server = tokio::spawn(async move {
            tokio::time::timeout(Duration::from_secs(10), async move {
                let (mut socket, _) = listener.accept().await.unwrap();
                let mut request = Vec::new();
                while !request.ends_with(b"\r\n\r\n") {
                    request.push(socket.read_u8().await.unwrap());
                    assert!(request.len() < 8192);
                }
                socket.write_all(header.as_bytes()).await.unwrap();
                // A client that refuses the body hangs up partway through.
                let _ = socket.write_all(&bytes).await;
            })
            .await
            .expect("the artwork request completes");
        });
        (url, server)
    }

    #[test]
    fn oversized_artwork_is_refused_as_it_arrives_and_never_cached() {
        let dir =
            std::env::temp_dir().join(format!("fastsonic-art-oversized-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let client = || {
            crate::http_client_builder()
                .no_proxy()
                .timeout(Duration::from_secs(10))
                .build()
                .unwrap()
        };
        let loader = ArtLoader::new(client(), client(), runtime.handle().clone(), dir.clone());
        runtime.block_on(async {
            for declare_length in [true, false] {
                let (url, server) = serve_artwork(vec![1; MAX_ART_BYTES + 1], declare_length).await;
                let error = loader.fetch(&url).await.unwrap_err();
                assert_eq!(error, "artwork is too large", "declared: {declare_length}");
                server.await.unwrap();
            }
        });
        drop(loader);
        runtime.shutdown_timeout(Duration::from_secs(10));
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 0);
        std::fs::remove_dir_all(dir).unwrap();
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

    /// A server that does not resize a cover hands over the library's own
    /// file, which can be WebP. The decoder behind both the page tint and
    /// egui's image loaders must accept it, or every size of that cover
    /// shows the placeholder.
    #[test]
    fn webp_cover_bytes_tint_and_decode_through_the_image_loaders() {
        let image = image::RgbImage::from_pixel(16, 16, image::Rgb([20, 120, 200]));
        let mut bytes = Vec::new();
        image
            .write_to(
                &mut std::io::Cursor::new(&mut bytes),
                image::ImageFormat::WebP,
            )
            .expect("the image crate encodes WebP");
        let color = accent_color(&bytes).expect("a WebP cover tints the page");
        assert!(
            color[2] > color[0],
            "expected the blue field, got {color:?}"
        );

        let runtime = tokio::runtime::Runtime::new().unwrap();
        let dir = std::env::temp_dir().join(format!("fastsonic-art-webp-{}", std::process::id()));
        let loader = backdrop_loader(&runtime, dir.clone());
        let ctx = egui::Context::default();
        egui_extras::install_image_loaders(&ctx);
        ctx.add_bytes_loader(Arc::new(loader.clone()));
        let uri = "sonic:art:300:webp-cover";
        let payload: Arc<[u8]> = bytes.into();
        loader.inner.entries.lock().unwrap().insert(
            uri.into(),
            Entry::Ready {
                retained: payload.len(),
                bytes: Some(payload),
                last_used: Instant::now(),
            },
        );
        runtime.block_on(async {
            tokio::time::timeout(Duration::from_secs(10), async {
                loop {
                    match ctx.try_load_texture(uri, Default::default(), Default::default()) {
                        Ok(egui::load::TexturePoll::Ready { .. }) => break,
                        Ok(egui::load::TexturePoll::Pending { .. }) => {}
                        Err(error) => panic!("the WebP cover failed to decode: {error}"),
                    }
                    tokio::time::sleep(Duration::from_millis(1)).await;
                }
            })
            .await
            .expect("the WebP cover produces a texture");
        });
        std::fs::remove_dir_all(dir).unwrap();
    }

    fn loader() -> ArtLoader {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let loader = ArtLoader::new(
            crate::http_client_builder().build().unwrap(),
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

    fn retained_total(loader: &ArtLoader) -> usize {
        let encoded: usize = loader
            .inner
            .entries
            .lock()
            .expect("lock")
            .values()
            .map(|entry| match entry {
                Entry::Ready { retained, .. } => *retained,
                _ => 0,
            })
            .sum();
        let textures: usize = loader
            .inner
            .textures
            .lock()
            .expect("lock")
            .values()
            .flat_map(HashMap::values)
            .map(|copy| copy.bytes)
            .sum();
        encoded + textures
    }

    #[test]
    fn large_covers_evict_using_texture_sizes() {
        let one = texture_bytes(640, 640);
        assert_eq!(one, 640 * 640 * 4 * 4 / 3);
        let jpeg = 50_000usize;
        let dir = std::env::temp_dir().join(format!(
            "fastsonic-art-budget-{}-{}",
            std::process::id(),
            Instant::now().elapsed().as_nanos()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(1)
            .enable_all()
            .build()
            .expect("a runtime for eviction");
        let loader = ArtLoader::new(
            reqwest::Client::new(),
            reqwest::Client::new(),
            runtime.handle().clone(),
            dir.clone(),
        );
        let now = Instant::now();
        let url = |i: u64| format!("sonic:art:640:al-{i}");
        for i in 0..40 {
            loader.inner.entries.lock().expect("lock").insert(
                url(i),
                Entry::Ready {
                    bytes: Some(Arc::from(vec![0u8; jpeg])),
                    last_used: now - Duration::from_secs(40 - i),
                    retained: jpeg,
                },
            );
        }
        let jpeg_total = retained_total(&loader);
        assert!(
            jpeg_total < HELD_BYTES,
            "JPEG-only covers must still fit the budget: {jpeg_total}"
        );
        for i in 0..40 {
            loader.release_bytes(&url(i));
            assert!(loader.note_texture(&url(i), &scaled_uri(&url(i), 640), 640, 640));
        }
        // The oldest cover was also painted from its 128 pixel request, an
        // entry of its own that goes with it.
        let small = "sonic:art:128:al-0";
        loader.inner.entries.lock().expect("lock").insert(
            small.into(),
            Entry::Ready {
                bytes: None,
                last_used: now - Duration::from_secs(60),
                retained: 0,
            },
        );
        let before = retained_total(&loader);
        assert_eq!(before, 40 * one);
        assert!(
            before > HELD_BYTES,
            "640×640 cover textures must exceed 64 MiB: {before}"
        );
        // Copies were painted in the order the covers were last used.
        for i in 0..40 {
            let mut textures = loader.inner.textures.lock().expect("lock");
            let copy = textures
                .get_mut(artwork(&url(i)).as_ref())
                .and_then(|copies| copies.get_mut(&scaled_uri(&url(i), 640)))
                .expect("a copy");
            copy.painted = now - Duration::from_secs(40 - i);
        }
        let ctx = egui::Context::default();
        loader.evict(&ctx);
        let after = retained_total(&loader);
        assert!(
            after <= HELD_BYTES,
            "eviction must bring retained texture bytes under budget: after={after}"
        );
        assert!(
            after < before,
            "a long scroll of large covers must free memory: before={before} after={after}"
        );
        let entries = loader.inner.entries.lock().expect("lock");
        assert!(
            !entries.contains_key(&url(0)),
            "the oldest scrolled-away cover must go first"
        );
        assert!(!entries.contains_key(small), "with every size asked for");
        assert!(
            entries.contains_key(&url(39)),
            "the cover just scrolled into view must stay"
        );
        drop(entries);
        assert!(
            !loader
                .inner
                .textures
                .lock()
                .expect("lock")
                .contains_key(artwork(&url(0)).as_ref()),
            "its scaled copies go with it"
        );
        loader.forget_all();
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn scaled_cover_uris_round_trip_and_step_by_32_pixels() {
        let url = "sonic:art:300:al-1";
        let uri = scaled_uri(url, 96);
        assert_eq!(parse_scaled(&uri), Some((url, 96)));
        assert_eq!(parse_scaled(url), None, "the source itself is not scaled");
        let signed = "http://host/share/img/JWT?size=300";
        assert_eq!(parse_scaled(&scaled_uri(signed, 64)), Some((signed, 64)));
        assert_eq!(parse_scaled("bytes://cover#fastsonic-px=96"), None);
        assert_eq!(cover_pixels(44.0), 64);
        assert_eq!(cover_pixels(64.0), 64);
        assert_eq!(cover_pixels(65.5), 96);
        assert_eq!(cover_pixels(0.0), 32);
    }

    /// Rule: a cover-art request is asked of the server at the size the
    /// cover's pixels want, whatever size the model named, and the
    /// requests of one cover at every size are one picture. A URL the
    /// server handed over is fetched as it is.
    #[test]
    fn a_request_follows_the_pixels_and_stays_the_same_picture() {
        assert_eq!(request_for("sonic:art:64:al-1", 44.0), "sonic:art:128:al-1");
        assert_eq!(
            request_for("sonic:art:640:al-1", 148.0),
            "sonic:art:192:al-1"
        );
        assert!(matches!(
            request_for("sonic:art:300:al-1", 212.0),
            Cow::Borrowed(_)
        ));
        let signed = "http://host/share/img/JWT?size=300";
        assert_eq!(request_for(signed, 44.0), signed);
        assert_eq!(artwork("sonic:art:64:al-1"), artwork("sonic:art:640:al-1"));
        assert_ne!(artwork("sonic:art:64:al-1"), artwork("sonic:art:64:al-2"));
        assert_eq!(artwork(signed), signed);
    }

    /// A cover is scaled down with a filter that keeps fine detail, and
    /// never up.
    #[test]
    fn covers_are_scaled_down_to_their_pixels_and_never_up() {
        let stripes = image::RgbImage::from_fn(300, 150, |x, _| {
            if x % 2 == 0 {
                image::Rgb([0, 0, 0])
            } else {
                image::Rgb([255, 255, 255])
            }
        });
        let small = scaled(stripes.clone().into(), 60);
        assert_eq!(small.size, [60, 30]);
        // Five stripes to a pixel average out to grey rather than landing on
        // whichever stripe a plain sample would hit.
        let [r, _, _, _] = small.pixels[15 * 60 + 30].to_srgba_unmultiplied();
        assert!((100..=160).contains(&r), "{r}");
        let same = scaled(stripes.into(), 640);
        assert_eq!(same.size, [300, 150]);
    }

    /// Averaging a large cover down before the Lanczos filter changes the
    /// picture by less than a level or two, on artwork full of fine lines
    /// and edges, and the copy is the size asked for.
    #[test]
    fn a_large_reduction_averages_first_for_the_same_picture() {
        let artwork = image::RgbImage::from_fn(640, 480, |x, y| {
            let line = ((x + 2 * y) / 3) % 7 < 2;
            let edge = x > 320 && y < 200;
            let shade = (x * 255 / 640) as u8;
            match (line, edge) {
                (true, _) => image::Rgb([255, 255, 255]),
                (false, true) => image::Rgb([200, 20, shade]),
                (false, false) => image::Rgb([shade, 60, 90]),
            }
        });
        let averaged = scaled(artwork.clone().into(), 64);
        assert_eq!(averaged.size, [64, 48]);
        let direct = image::DynamicImage::from(artwork)
            .resize_exact(64, 48, image::imageops::FilterType::Lanczos3)
            .to_rgba8();
        let difference: f64 = averaged
            .pixels
            .iter()
            .zip(direct.pixels())
            .map(|(averaged, direct)| {
                let averaged = averaged.to_srgba_unmultiplied();
                (0..3)
                    .map(|channel| {
                        (f64::from(averaged[channel]) - f64::from(direct[channel])).abs()
                    })
                    .sum::<f64>()
            })
            .sum::<f64>()
            / (64.0 * 48.0 * 3.0);
        assert!(difference < 2.0, "mean difference {difference}");
    }

    fn scaling_ctx(loader: &ArtLoader) -> egui::Context {
        let ctx = egui::Context::default();
        ctx.add_bytes_loader(Arc::new(loader.clone()));
        ctx.add_image_loader(Arc::new(loader.scaler()));
        ctx
    }

    /// The same artwork wanted at two sizes, a sidebar row's and a page
    /// header's: the row's copy, painted every frame, must not keep taking
    /// the bytes the header's copy is reloading from the disk, or the
    /// header stays blurry for good.
    #[test]
    fn a_second_size_of_painted_artwork_is_still_made() {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let dir = std::env::temp_dir().join(format!("fastsonic-two-sizes-{}", std::process::id()));
        let loader = backdrop_loader(&runtime, dir.clone());
        let ctx = scaling_ctx(&loader);
        let url = "http://music.example/share/img/two-sizes";
        cached_png(&loader, url, [20, 30, 40]);
        let palette = crate::theme::Palette::dark();
        let small = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(32.0, 32.0));
        let large = egui::Rect::from_min_size(egui::pos2(100.0, 0.0), egui::vec2(64.0, 64.0));
        let frame = |ctx: &egui::Context, rects: &[egui::Rect]| {
            let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
                for rect in rects {
                    crate::ui::widgets::paint_cover(
                        ui,
                        &palette,
                        Some(url),
                        *rect,
                        6.0,
                        crate::theme::Icon::Music,
                        Some(&loader),
                    );
                }
            });
            output.textures_delta.clear();
        };
        runtime.block_on(async {
            tokio::time::timeout(Duration::from_secs(10), async {
                while texture_named(&ctx, &scaled_uri(url, 32)).is_none() {
                    frame(&ctx, &[small]);
                    tokio::time::sleep(Duration::from_millis(1)).await;
                }
                while texture_named(&ctx, &scaled_uri(url, 64)).is_none() {
                    frame(&ctx, &[small, large]);
                    tokio::time::sleep(Duration::from_millis(1)).await;
                }
            })
            .await
            .expect("both sizes are made");
        });
        let _ = std::fs::remove_dir_all(dir);
    }

    /// A panel resized from one size to the next leaves the copy at the
    /// old size unpainted; it goes, even when the new size asked the server
    /// for a new size too. Two sizes painted together, a row's and a page
    /// header's, both stay, even after the window has sat idle for minutes,
    /// as does a cover's only copy.
    #[test]
    fn copies_no_longer_painted_at_their_size_are_let_go() {
        let now = Instant::now();
        let copy = |ago: u64| PaintedCopy {
            bytes: 1,
            painted: now - Duration::from_secs(ago),
        };
        let resized = "sonic:art:300:resized";
        let zoomed = "sonic:art:300:zoomed";
        let shared = "sonic:art:300:shared";
        let alone = "sonic:art:300:alone";
        let idle = "sonic:art:300:idle";
        let mut textures: Copies = HashMap::new();
        textures.insert(
            artwork(resized).into_owned(),
            HashMap::from([
                (scaled_uri(resized, 224), copy(30)),
                (scaled_uri(resized, 256), copy(0)),
            ]),
        );
        textures.insert(
            artwork(zoomed).into_owned(),
            HashMap::from([
                (scaled_uri(zoomed, 224), copy(30)),
                (scaled_uri("sonic:art:640:zoomed", 448), copy(0)),
            ]),
        );
        textures.insert(
            artwork(shared).into_owned(),
            HashMap::from([
                (scaled_uri("sonic:art:128:shared", 64), copy(1)),
                (scaled_uri(shared, 224), copy(0)),
            ]),
        );
        textures.insert(
            artwork(alone).into_owned(),
            HashMap::from([(scaled_uri(alone, 64), copy(600))]),
        );
        textures.insert(
            artwork(idle).into_owned(),
            HashMap::from([
                (scaled_uri(idle, 64), copy(601)),
                (scaled_uri(idle, 288), copy(600)),
            ]),
        );
        let mut gone = superseded(&mut textures);
        gone.sort();
        assert_eq!(gone, [scaled_uri(resized, 224), scaled_uri(zoomed, 224)]);
        assert_eq!(textures[artwork(resized).as_ref()].len(), 1);
        assert_eq!(textures[artwork(zoomed).as_ref()].len(), 1);
        assert_eq!(textures[artwork(shared).as_ref()].len(), 2);
        assert_eq!(textures[artwork(alone).as_ref()].len(), 1);
        assert_eq!(
            textures[artwork(idle).as_ref()].len(),
            2,
            "painted together, then idle"
        );
    }

    /// A new window's egui holds none of the old one's textures, so each
    /// copy counts as new again, and its decoded image is let go of again.
    #[test]
    fn a_new_window_starts_without_copies() {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let dir = std::env::temp_dir().join(format!("fastsonic-new-window-{}", std::process::id()));
        let loader = backdrop_loader(&runtime, dir.clone());
        let url = "sonic:art:128:window";
        let uri = scaled_uri(url, 64);
        assert!(loader.note_texture(url, &uri, 64, 64));
        assert!(!loader.note_texture(url, &uri, 64, 64));
        assert!(loader.has_copy("sonic:art:640:window"), "any size of it");
        loader.forget_textures();
        assert!(!loader.has_copy(url));
        assert!(loader.note_texture(url, &uri, 64, 64));
        let _ = std::fs::remove_dir_all(dir);
    }

    /// The copy that stands in has the pixels asked for, or is the largest;
    /// a copy counts at the size the server was asked for when that is
    /// smaller, as it was never enlarged.
    #[test]
    fn the_nearest_copy_has_the_pixels_asked_for_or_is_the_largest() {
        let url = "sonic:art:1280:nearest";
        let copies = [64, 224, 320].map(|pixels| scaled_uri(url, pixels));
        let nearest_to = |pixels| nearest(copies.iter().map(String::as_str), pixels);
        assert_eq!(nearest_to(200), Some(copies[1].as_str()));
        assert_eq!(nearest_to(224), Some(copies[1].as_str()));
        assert_eq!(nearest_to(32), Some(copies[0].as_str()));
        assert_eq!(nearest_to(640), Some(copies[2].as_str()), "none is enough");
        assert_eq!(nearest_to(641), None, "the largest has under half");
        assert_eq!(nearest(std::iter::empty(), 64), None);

        let thumbnail = scaled_uri("sonic:art:64:nearest", 288);
        let sharp = scaled_uri("sonic:art:300:nearest", 224);
        let both = [thumbnail.as_str(), sharp.as_str()];
        assert_eq!(
            nearest(both.into_iter(), 288),
            Some(sharp.as_str()),
            "an enlarged thumbnail has only its own pixels"
        );
        assert_eq!(
            nearest([thumbnail.as_str()].into_iter(), 288),
            None,
            "the softened thumbnail does better"
        );
    }

    /// The textures `shapes` are painted with, besides the font atlas.
    fn painted_textures(shapes: &[egui::epaint::ClippedShape]) -> Vec<egui::TextureId> {
        shapes
            .iter()
            .filter_map(|clipped| match &clipped.shape {
                egui::Shape::Rect(rect) => rect.brush.as_ref().map(|brush| brush.fill_texture_id),
                egui::Shape::Mesh(mesh) => Some(mesh.texture_id),
                _ => None,
            })
            .filter(|texture| *texture != egui::TextureId::default())
            .collect()
    }

    /// The texture egui made of the copy at `uri`, once it has.
    fn texture_named(ctx: &egui::Context, uri: &str) -> Option<egui::TextureId> {
        ctx.tex_manager()
            .read()
            .allocated()
            .find(|(_, meta)| meta.name == uri)
            .map(|(texture, _)| *texture)
    }

    /// Paints frames until egui has made each of `uris`, painting `paint`.
    fn paint_until_made(
        runtime: &tokio::runtime::Runtime,
        ctx: &egui::Context,
        uris: &[String],
        mut paint: impl FnMut(&mut egui::Ui),
    ) {
        runtime.block_on(async {
            tokio::time::timeout(Duration::from_secs(10), async {
                while uris.iter().any(|uri| texture_named(ctx, uri).is_none()) {
                    let mut output = ctx.run_ui(egui::RawInput::default(), &mut paint);
                    output.textures_delta.clear();
                    tokio::time::sleep(Duration::from_millis(1)).await;
                }
            })
            .await
            .expect("the copies are made");
        });
    }

    /// A cover drawn at a new size, which asks the server for a new size,
    /// shows its copy at the old one while the new copy is made, rather
    /// than the placeholder.
    #[test]
    fn a_cover_changing_size_keeps_its_picture_while_the_new_copy_is_made() {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let dir = std::env::temp_dir().join(format!("fastsonic-resized-{}", std::process::id()));
        let loader = backdrop_loader(&runtime, dir.clone());
        let ctx = scaling_ctx(&loader);
        let url = "sonic:art:300:resized";
        // A 96 point cover asks for 128 pixels, a 148 point one for 192.
        cached_png(&loader, "sonic:art:128:resized", [20, 30, 40]);
        let palette = crate::theme::Palette::dark();
        let at =
            |size: f32| egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(size, size));
        let cover = |ui: &mut egui::Ui, size: f32| {
            crate::ui::widgets::paint_cover(
                ui,
                &palette,
                Some(url),
                at(size),
                6.0,
                crate::theme::Icon::Music,
                Some(&loader),
            );
        };
        let small = scaled_uri("sonic:art:128:resized", 96);
        paint_until_made(&runtime, &ctx, std::slice::from_ref(&small), |ui| {
            cover(ui, 96.0)
        });
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| cover(ui, 148.0));
        output.textures_delta.clear();
        assert!(texture_named(&ctx, &scaled_uri("sonic:art:192:resized", 160)).is_none());
        assert_eq!(
            painted_textures(&output.shapes),
            [texture_named(&ctx, &small).unwrap()],
            "the small copy stands in"
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    /// A new song's cover still being made leaves the last song's showing.
    #[test]
    fn a_new_songs_cover_keeps_the_last_ones_place_until_it_is_made() {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let dir = std::env::temp_dir().join(format!("fastsonic-next-song-{}", std::process::id()));
        let loader = backdrop_loader(&runtime, dir.clone());
        let ctx = scaling_ctx(&loader);
        let [first, next] = ["sonic:art:64:first", "sonic:art:64:next"];
        // A 56 point cover asks for 128 pixels.
        cached_png(&loader, "sonic:art:128:first", [20, 30, 40]);
        cached_png(&loader, "sonic:art:128:next", [200, 30, 40]);
        let palette = crate::theme::Palette::dark();
        let rect = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(56.0, 56.0));
        let cover = |ui: &mut egui::Ui, url: &str| {
            crate::ui::widgets::playing_cover(ui, &palette, &loader, "bar", Some(url), rect, 6.0);
        };
        let shown = scaled_uri("sonic:art:128:first", 64);
        paint_until_made(&runtime, &ctx, std::slice::from_ref(&shown), |ui| {
            cover(ui, first)
        });
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| cover(ui, next));
        output.textures_delta.clear();
        assert_eq!(
            painted_textures(&output.shapes),
            [texture_named(&ctx, &shown).unwrap()],
            "the last song's cover stays"
        );
        paint_until_made(
            &runtime,
            &ctx,
            &[scaled_uri("sonic:art:128:next", 64)],
            |ui| cover(ui, next),
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    fn png(rgb: [u8; 3]) -> Bytes {
        let mut encoded = std::io::Cursor::new(Vec::new());
        image::RgbImage::from_pixel(64, 64, image::Rgb(rgb))
            .write_to(&mut encoded, image::ImageFormat::Png)
            .unwrap();
        Bytes::Shared(Arc::from(encoded.into_inner()))
    }

    /// A fast scroll asks for many copies at once; only a few are made at
    /// a time.
    #[test]
    fn only_a_few_covers_are_scaled_at_once() {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let dir = std::env::temp_dir().join(format!("fastsonic-at-once-{}", std::process::id()));
        let loader = backdrop_loader(&runtime, dir.clone());
        let ctx = egui::Context::default();
        ctx.add_bytes_loader(Arc::new(loader.clone()));
        let scaler = loader.scaler();
        let uris: Vec<String> = (0..8)
            .map(|index| {
                let url = format!("sonic:art:64:scroll-{index}");
                cached_png(&loader, &url, [index * 20, 30, 40]);
                scaled_uri(&url, 32)
            })
            .collect();
        runtime.block_on(async {
            tokio::time::timeout(Duration::from_secs(10), async {
                loop {
                    let mut ready = 0;
                    for uri in &uris {
                        if let Ok(ImagePoll::Ready { .. }) =
                            scaler.load(&ctx, uri, SizeHint::default())
                        {
                            ready += 1;
                        }
                    }
                    let (workers, working) = {
                        let state = scaler.state.lock().unwrap();
                        let working = state
                            .slots
                            .values()
                            .filter(|slot| matches!(slot, Slot::Working))
                            .count();
                        (state.workers, working)
                    };
                    assert!(workers <= SCALING_AT_ONCE, "{workers}");
                    assert!(working <= SCALING_AT_ONCE, "{working}");
                    if ready == uris.len() {
                        break;
                    }
                    tokio::time::sleep(Duration::from_millis(1)).await;
                }
            })
            .await
            .expect("every copy is made in the end");
        });
        let _ = std::fs::remove_dir_all(dir);
    }

    /// A cover scrolled past before its turn is dropped rather than made,
    /// while one still asked for is made. Asked for again, the dropped one
    /// is made too.
    #[test]
    fn a_copy_nobody_asks_for_any_more_is_not_made() {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let ctx = egui::Context::default();
        let scaler = CoverScaler::new(runtime.handle().clone());
        let [gone, shown] = [
            scaled_uri("sonic:art:64:gone", 32),
            scaled_uri("sonic:art:64:shown", 32),
        ];
        {
            let mut state = scaler.state.lock().unwrap();
            let waiting = |ago: Duration| Slot::Waiting {
                asked: Instant::now() - ago,
                bytes: png([20, 30, 40]),
                pixels: 32,
            };
            state.slots.insert(gone.clone(), waiting(SCALING_STALE * 2));
            state.slots.insert(shown.clone(), waiting(Duration::ZERO));
            state.workers = 1;
        }
        scale_waiting(&scaler.state, &ctx);
        let state = scaler.state.lock().unwrap();
        assert!(!state.slots.contains_key(&gone), "dropped, not made");
        assert!(matches!(
            state.slots.get(&shown),
            Some(Slot::Ready { result: Ok(_), .. })
        ));
        assert_eq!(state.workers, 0, "the worker is done");
        drop(state);

        let dir =
            std::env::temp_dir().join(format!("fastsonic-asked-again-{}", std::process::id()));
        let loader = backdrop_loader(&runtime, dir.clone());
        ctx.add_bytes_loader(Arc::new(loader.clone()));
        cached_png(&loader, "sonic:art:64:gone", [20, 30, 40]);
        runtime.block_on(async {
            tokio::time::timeout(Duration::from_secs(10), async {
                while !matches!(
                    scaler.load(&ctx, &gone, SizeHint::default()),
                    Ok(ImagePoll::Ready { .. })
                ) {
                    tokio::time::sleep(Duration::from_millis(1)).await;
                }
            })
            .await
            .expect("asked for again, it is made");
        });
        let _ = std::fs::remove_dir_all(dir);
    }

    /// A copy made after its cover left the screen, which no frame asks
    /// for, is let go of at the end of a pass; one just made, or asked for,
    /// is kept for egui to make its texture.
    #[test]
    fn a_made_copy_nobody_asks_for_is_let_go() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap();
        let scaler = CoverScaler::new(runtime.handle().clone());
        let made = |ago: Duration| Slot::Ready {
            result: Err("made".to_string()),
            asked: Instant::now() - ago,
        };
        let [gone, fresh] = [
            scaled_uri("sonic:art:64:gone", 32),
            scaled_uri("sonic:art:64:fresh", 32),
        ];
        {
            let mut state = scaler.state.lock().unwrap();
            state.slots.insert(gone.clone(), made(SCALING_STALE * 2));
            state.slots.insert(fresh.clone(), made(Duration::ZERO));
        }
        scaler.end_pass(1);
        let state = scaler.state.lock().unwrap();
        assert!(!state.slots.contains_key(&gone));
        assert!(state.slots.contains_key(&fresh));
    }

    /// The scaler makes the copy off the UI thread from the artwork
    /// loader's bytes, and holds it only until it is forgotten.
    #[test]
    fn the_scaler_makes_a_copy_from_the_artwork_loader() {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let dir = std::env::temp_dir().join(format!("fastsonic-scaler-{}", std::process::id()));
        let loader = backdrop_loader(&runtime, dir.clone());
        let ctx = egui::Context::default();
        ctx.add_bytes_loader(Arc::new(loader.clone()));
        let scaler = loader.scaler();
        let url = "sonic:art:64:scaled";
        cached_png(&loader, url, [20, 30, 40]);
        let uri = scaled_uri(url, 32);
        assert!(matches!(
            scaler.load(&ctx, url, SizeHint::default()),
            Err(LoadError::NotSupported)
        ));
        runtime.block_on(async {
            tokio::time::timeout(Duration::from_secs(10), async {
                loop {
                    match scaler.load(&ctx, &uri, SizeHint::default()) {
                        Ok(ImagePoll::Ready { image }) => {
                            assert_eq!(image.size, [32, 32]);
                            break;
                        }
                        Ok(ImagePoll::Pending { .. }) => {}
                        Err(error) => panic!("{error}"),
                    }
                    tokio::time::sleep(Duration::from_millis(1)).await;
                }
            })
            .await
            .expect("the scaled copy is made");
        });
        assert_eq!(scaler.byte_size(), 32 * 32 * 4);
        scaler.forget(&uri);
        assert_eq!(scaler.byte_size(), 0);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn releasing_bytes_reloads_from_disk_off_the_ui_thread() {
        use egui::load::BytesLoader;
        use std::time::Duration as StdDuration;

        let dir = std::env::temp_dir().join(format!(
            "fastsonic-art-reload-{}-{}",
            std::process::id(),
            Instant::now().elapsed().as_nanos()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(1)
            .enable_all()
            .build()
            .expect("a runtime for disk reload");
        let loader = ArtLoader::new(
            reqwest::Client::new(),
            reqwest::Client::new(),
            runtime.handle().clone(),
            dir.clone(),
        );
        let url = "https://i.scdn.co/image/reload";
        let path = loader.inner.cache_path(url);
        std::fs::create_dir_all(path.parent().expect("cache dir")).expect("cache dir");
        std::fs::write(&path, b"\xff\xd8\xff jpeg-ish").expect("cached jpeg");
        loader.inner.entries.lock().expect("lock").insert(
            url.to_string(),
            Entry::Ready {
                bytes: None,
                last_used: Instant::now(),
                retained: 0,
            },
        );
        let ctx = egui::Context::default();
        let first = loader.load(&ctx, url).expect("load");
        assert!(
            matches!(first, BytesPoll::Pending { .. }),
            "disk reload must not block the UI thread"
        );
        let deadline = Instant::now() + StdDuration::from_secs(2);
        loop {
            std::thread::sleep(StdDuration::from_millis(20));
            match loader.load(&ctx, url) {
                Ok(BytesPoll::Ready { .. }) => break,
                Ok(BytesPoll::Pending { .. }) if Instant::now() < deadline => continue,
                _ => panic!("reload did not finish"),
            }
        }
        loader.forget_all();
        assert!(loader.inner.entries.lock().expect("lock").is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
