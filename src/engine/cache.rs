//! The on-disk audio cache: the bytes of a stream, kept so that playing a
//! track again does not ask the server for it.
//!
//! librespot had one of these and it leaves with librespot (P3.9), so this
//! is the fork's own. It is **not** a store of whole tracks. Opening a
//! track costs three HTTP requests before a note plays — symphonia probes
//! for trailing metadata whenever the source can seek and knows its length,
//! so it reads the head, jumps to the tail, and comes back — and on a home
//! connection to a server in the next room that is three round trips of
//! silence. A cache of whole files would still pay all three for a track
//! that was skipped through and never finished. So the unit here is a
//! **block** of the file, [`BLOCK`] bytes of it, and the head and the tail
//! of a track are cached as readily as the middle.
//!
//! ```text
//!   <cache>/audio/<sha1 of the song id>/
//!       meta            what the file is: its length, and how to notice it changing
//!       00000000        the first BLOCK bytes
//!       00000001        the next, if they have been read
//! ```
//!
//! One file per block rather than one sparse file per track, because a
//! budget has to be measurable: a sparse file reports the length of the
//! whole track whatever is actually in it, and on Windows writing past the
//! end of a file allocates the space rather than leaving a hole. A block
//! that exists is a block that has been paid for, on every platform.
//!
//! **Nothing here may stop the music.** A cache is disposable by
//! definition: a directory that cannot be read, a block that cannot be
//! written, a budget that cannot be measured all mean one thing — go to the
//! server instead. Every failure in this module is logged and swallowed,
//! and the only errors that come out of a cached stream are the stream's
//! own.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::io::{self, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::SystemTime;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use sha1::{Digest, Sha1};
use symphonia::core::io::MediaSource;

use super::source::{HttpSource, Stats};

/// How much of a file one cached block holds. About a second and a half of
/// FLAC, six of a 320 kbps MP3: big enough that a track streams in a
/// handful of files, small enough that a track opened and abandoned costs
/// the disk almost nothing.
pub const BLOCK: usize = 256 * 1024;

/// The layout of an entry. Anything written by another version is deleted
/// rather than read, which is what makes changing the layout free.
const VERSION: u32 = 1;

/// What the cache knows about one file, written beside its blocks.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
struct Meta {
    version: u32,
    /// The song id, so that a person can tell what a directory of numbers
    /// is. Nothing reads it.
    id: String,
    /// The whole file's length in bytes, which is what makes block
    /// arithmetic possible without asking the server.
    len: u64,
    /// The block size the entry was cut into.
    block: u32,
    /// The server's `ETag` or `Last-Modified` for the file, so that a file
    /// replaced on the server is noticed rather than served from here for
    /// ever. `None` from a server that offers neither.
    validator: Option<String>,
}

/// What one entry costs and when it was last played from, in memory so
/// that the budget does not need a walk of the disk to enforce.
#[derive(Debug)]
struct Held {
    bytes: u64,
    /// The order entries were last opened in. Ticks rather than clock
    /// time: what eviction needs is an order, and a counter cannot go
    /// backwards when the machine's clock does.
    used: u64,
}

#[derive(Debug, Default)]
struct Index {
    entries: BTreeMap<String, Held>,
    total: u64,
    tick: u64,
    /// The entries with a source open on them, and how many. Never
    /// evicted: the track being played and the track being opened for the
    /// next join are both in here, and deleting the blocks under a reader
    /// would turn a cache miss into a stutter.
    busy: BTreeMap<String, u32>,
}

/// What the cache holds and how it has been doing, for the settings window
/// and for the probe that has to prove a second play makes no request.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CacheStats {
    pub entries: usize,
    pub bytes: u64,
    pub budget: u64,
    /// Blocks served from disk.
    pub hits: u64,
    /// Blocks that had to be fetched.
    pub misses: u64,
}

/// The cache itself: a directory, a budget, and what is in it.
///
/// Shared between the audio thread and the runtime, because the track
/// after this one is opened on the runtime (P3.4) and reads through the
/// same cache as the one playing.
#[derive(Debug)]
pub struct Cache {
    root: PathBuf,
    block: usize,
    budget: u64,
    index: Mutex<Index>,
    hits: AtomicU64,
    misses: AtomicU64,
}

impl Cache {
    /// Opens the cache at `root`, keeping it under `budget` bytes.
    ///
    /// The directory is read once here so that the budget can be enforced
    /// without walking the disk again, and anything in it that is not an
    /// entry of this version is removed. This directory is dedicated to audio
    /// blocks, so entries without our metadata are leftovers from the player
    /// replaced at P3.9 or interrupted writes.
    pub fn open(root: PathBuf, budget: u64) -> Result<Arc<Self>> {
        Self::with_block(root, budget, BLOCK)
    }

    fn with_block(root: PathBuf, budget: u64, block: usize) -> Result<Arc<Self>> {
        std::fs::create_dir_all(&root)
            .with_context(|| format!("unable to make the audio cache at {}", root.display()))?;
        let cache = Arc::new(Self {
            root,
            block,
            budget,
            index: Mutex::new(Index::default()),
            hits: AtomicU64::new(0),
            misses: AtomicU64::new(0),
        });
        cache.scan();
        let stats = cache.stats();
        log::info!(
            "audio cache: {} track(s), {:.1} MiB of {:.0} MiB, in {}",
            stats.entries,
            stats.bytes as f64 / (1024.0 * 1024.0),
            stats.budget as f64 / (1024.0 * 1024.0),
            cache.root.display()
        );
        cache.trim();
        Ok(cache)
    }

    /// Reads what is on disk into the index, oldest first.
    ///
    /// Recency survives a restart because [`Cache::entry`] rewrites an
    /// entry's `meta` when it opens it, so the file's own timestamp is
    /// when the track was last played. The ticks handed out here put those
    /// timestamps in order and nothing needs the clock again.
    fn scan(&self) {
        let listing = match std::fs::read_dir(&self.root) {
            Ok(listing) => listing,
            Err(error) => {
                log::warn!("cannot read the audio cache: {error}");
                return;
            }
        };
        let mut found: Vec<(SystemTime, String, u64)> = Vec::new();
        for entry in listing.flatten() {
            if !entry.file_type().is_ok_and(|kind| kind.is_dir()) {
                continue;
            }
            let Some(key) = entry.file_name().to_str().map(str::to_string) else {
                continue;
            };
            let dir = entry.path();
            // The directory is dedicated to audio cache entries. Anything
            // without readable metadata is either from the retired player or
            // an interrupted write, and would otherwise sit outside the
            // configured budget forever.
            let Some(meta) = read_meta(&dir) else {
                log::debug!(
                    "dropping an unrecognised audio cache entry: {}",
                    dir.display()
                );
                let _ = std::fs::remove_dir_all(&dir);
                continue;
            };
            if meta.version != VERSION || meta.block as usize != self.block {
                log::debug!(
                    "dropping a cache entry of an older layout: {}",
                    dir.display()
                );
                let _ = std::fs::remove_dir_all(&dir);
                continue;
            }
            let used = std::fs::metadata(dir.join("meta"))
                .and_then(|meta| meta.modified())
                .unwrap_or(SystemTime::UNIX_EPOCH);
            found.push((used, key, blocks_size(&dir)));
        }
        found.sort_by_key(|(used, _, _)| *used);

        let mut index = self.lock();
        for (tick, (_, key, bytes)) in found.into_iter().enumerate() {
            index.total += bytes;
            index.entries.insert(
                key,
                Held {
                    bytes,
                    used: tick as u64,
                },
            );
        }
        index.tick = index.entries.len() as u64;
    }

    /// A handle on one song's blocks.
    ///
    /// `expected` is the file size the server described the song with. A
    /// cached entry whose length disagrees with it is a file that changed
    /// on the server — a re-tag, a re-encode — and it is thrown away here
    /// rather than played back as somebody else's bytes.
    pub(crate) fn entry(self: &Arc<Self>, id: &str, expected: Option<u64>) -> Entry {
        let key = key_for(id);
        let dir = self.root.join(&key);
        {
            let mut index = self.lock();
            *index.busy.entry(key.clone()).or_default() += 1;
        }
        let mut entry = Entry {
            cache: Arc::clone(self),
            key,
            dir,
            meta: None,
        };
        let meta = read_meta(&entry.dir).filter(|meta| {
            meta.version == VERSION
                && meta.block as usize == self.block
                && expected.is_none_or(|size| size == meta.len)
        });
        match meta {
            Some(meta) => {
                // Rewriting it is how the entry says it was used today,
                // which is what a later `scan` sorts by.
                entry.write_meta(&meta);
                entry.meta = Some(meta);
                self.touch(&entry.key);
            }
            // Either nothing is here, or what is here cannot be trusted.
            None if entry.dir.exists() => entry.wipe(),
            None => {}
        }
        entry
    }

    /// Everything, gone. The settings window's Clear button, and how a
    /// test starts from nothing. Returns how many bytes were freed, which
    /// is what the button says afterwards.
    pub fn clear(&self) -> u64 {
        let mut index = self.lock();
        for key in index.entries.keys() {
            let _ = std::fs::remove_dir_all(self.root.join(key));
        }
        let freed = index.total;
        index.entries.clear();
        index.total = 0;
        freed
    }

    pub fn stats(&self) -> CacheStats {
        let index = self.lock();
        CacheStats {
            entries: index.entries.len(),
            bytes: index.total,
            budget: self.budget,
            hits: self.hits.load(Ordering::Relaxed),
            misses: self.misses.load(Ordering::Relaxed),
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Index> {
        self.index
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn touch(&self, key: &str) {
        let mut index = self.lock();
        index.tick += 1;
        let tick = index.tick;
        if let Some(held) = index.entries.get_mut(key) {
            held.used = tick;
        }
    }

    /// A block has been written. The budget is enforced from here, because
    /// this is the only moment the cache grows.
    fn grew(&self, key: &str, bytes: u64) {
        {
            let mut index = self.lock();
            index.total += bytes;
            index.tick += 1;
            let tick = index.tick;
            let held = index.entries.entry(key.to_string()).or_insert(Held {
                bytes: 0,
                used: tick,
            });
            held.bytes += bytes;
            held.used = tick;
        }
        self.trim();
    }

    fn forget(&self, key: &str) {
        let mut index = self.lock();
        if let Some(held) = index.entries.remove(key) {
            index.total = index.total.saturating_sub(held.bytes);
        }
    }

    fn release(&self, key: &str) {
        let mut index = self.lock();
        if let Some(count) = index.busy.get_mut(key) {
            *count -= 1;
            if *count == 0 {
                index.busy.remove(key);
            }
        }
    }

    /// Deletes least-recently-played tracks until the cache is inside its
    /// budget, and never the ones being played from.
    fn trim(&self) {
        let mut index = self.lock();
        while index.total > self.budget {
            let oldest = index
                .entries
                .iter()
                .filter(|(key, _)| !index.busy.contains_key(*key))
                .min_by_key(|(_, held)| held.used)
                .map(|(key, held)| (key.clone(), held.bytes));
            // Everything left is open. Over budget for as long as it takes
            // to finish playing is the right answer; stuttering is not.
            let Some((key, bytes)) = oldest else {
                log::debug!("the audio cache is over its budget and everything in it is in use");
                break;
            };
            log::debug!("evicting {key} from the audio cache to stay inside its budget");
            let _ = std::fs::remove_dir_all(self.root.join(&key));
            index.entries.remove(&key);
            index.total = index.total.saturating_sub(bytes);
        }
    }
}

/// One song's place in the cache, held for as long as something is reading
/// or writing it.
pub(crate) struct Entry {
    cache: Arc<Cache>,
    key: String,
    dir: PathBuf,
    /// What the file is, if the cache already knew. `None` until the
    /// stream says.
    meta: Option<Meta>,
}

impl Entry {
    /// The file's length, known without asking the server if this track
    /// has been played before. That is what makes a second play cost no
    /// requests at all: symphonia asks whether the source can seek before
    /// it reads a byte, and the answer is a length.
    fn len(&self) -> Option<u64> {
        self.meta.as_ref().map(|meta| meta.len)
    }

    fn validator(&self) -> Option<&str> {
        self.meta
            .as_ref()
            .and_then(|meta| meta.validator.as_deref())
    }

    /// What the stream turned out to be, recorded so the next play does
    /// not have to open it to find out.
    fn describe(&mut self, id: &str, len: u64, validator: Option<String>) {
        let meta = Meta {
            version: VERSION,
            id: id.to_string(),
            len,
            block: self.cache.block as u32,
            validator,
        };
        self.write_meta(&meta);
        self.meta = Some(meta);
    }

    fn write_meta(&self, meta: &Meta) {
        if let Err(error) = std::fs::create_dir_all(&self.dir)
            .and_then(|()| serde_json::to_vec(meta).map_err(io::Error::other))
            .and_then(|json| std::fs::write(self.dir.join("meta"), json))
        {
            log::debug!("cannot write to the audio cache: {error}");
        }
    }

    /// This song's blocks, gone: what is here is not the file the server
    /// has, or not a file this build can read.
    fn wipe(&mut self) {
        log::debug!("dropping the cached copy of {}", self.key);
        let _ = std::fs::remove_dir_all(&self.dir);
        self.cache.forget(&self.key);
        self.meta = None;
    }

    /// One block, if it has been read before.
    fn block(&self, index: u64) -> Option<Vec<u8>> {
        let bytes = std::fs::read(self.dir.join(block_name(index))).ok()?;
        self.cache.hits.fetch_add(1, Ordering::Relaxed);
        Some(bytes)
    }

    /// Keeps one block. Written to a temporary name and renamed, so a
    /// block that exists is a block that is whole — two copies of the app,
    /// or a crash in the middle of a write, cannot leave a half a block to
    /// be played as if it were music.
    fn put(&self, index: u64, bytes: &[u8]) {
        let name = block_name(index);
        let temp = self.dir.join(format!(".{name}.{}", std::process::id()));
        let written = std::fs::create_dir_all(&self.dir)
            .and_then(|()| std::fs::write(&temp, bytes))
            .and_then(|()| std::fs::rename(&temp, self.dir.join(&name)));
        match written {
            Ok(()) => self.cache.grew(&self.key, bytes.len() as u64),
            Err(error) => {
                log::debug!("cannot keep a block of audio: {error}");
                let _ = std::fs::remove_file(&temp);
            }
        }
    }
}

impl Drop for Entry {
    fn drop(&mut self) {
        self.cache.release(&self.key);
    }
}

/// The block being read, in memory. A block is fetched or read whole and
/// handed out in whatever sizes the reader above asks for.
struct Loaded {
    index: u64,
    bytes: Vec<u8>,
}

/// A stream read through the cache: blocks from disk where they are
/// there, from the server where they are not, and every one that comes
/// from the server kept.
///
/// This is a [`MediaSource`] like [`HttpSource`] is, and the decoder above
/// cannot tell the difference — except in how long the first packet takes
/// and, on a track played before, in there being no request at all.
pub(crate) struct CachedSource {
    http: HttpSource,
    entry: Entry,
    /// The whole file's length. `None` from a server that will not say,
    /// and then nothing is cached: block arithmetic needs an end, and
    /// [`HttpSource`] already refuses to seek without one.
    len: Option<u64>,
    block: u64,
    pos: u64,
    loaded: Option<Loaded>,
    id: String,
}

impl CachedSource {
    /// Opens a track through the cache.
    ///
    /// A track the cache knows costs nothing here: the length comes out of
    /// its `meta` and the stream is not opened until a block is wanted
    /// that is not on disk. A track it does not know is opened at once,
    /// exactly as [`HttpSource::new`] would, because its length is the
    /// first thing symphonia asks for.
    pub(crate) fn new(
        http: reqwest::blocking::Client,
        url: String,
        stats: Arc<Stats>,
        mut entry: Entry,
        id: &str,
    ) -> io::Result<Self> {
        let block = entry.cache.block as u64;
        let mut source = HttpSource::lazy(http, url, stats);
        let len = match entry.len() {
            Some(len) => Some(len),
            None => {
                source.prime()?;
                let len = source.len();
                if let Some(len) = len {
                    entry.describe(id, len, source.validator().map(str::to_string));
                }
                len
            }
        };
        Ok(Self {
            http: source,
            entry,
            len,
            block,
            pos: 0,
            loaded: None,
            id: id.to_string(),
        })
    }

    /// Reads one block: from disk, or from the server and then onto disk.
    fn load(&mut self, index: u64) -> io::Result<()> {
        if let Some(bytes) = self.entry.block(index) {
            self.loaded = Some(Loaded { index, bytes });
            return Ok(());
        }
        self.entry.cache.misses.fetch_add(1, Ordering::Relaxed);

        let start = index * self.block;
        // Seeking to where the reader already is costs nothing and keeps
        // the body open, so a track that plays through from the start is
        // one request however many blocks it is cut into.
        self.http.seek(SeekFrom::Start(start))?;
        self.http.prime()?;
        // Only now, with an answer from the server in hand: the length it
        // gives may not be the length the cache believed.
        self.agrees_with_the_server()?;
        let len = self.len.unwrap_or(0);
        let want = len.min(start + self.block).saturating_sub(start) as usize;
        let mut bytes = vec![0_u8; want];
        let mut got = 0;
        while got < want {
            match self.http.read(&mut bytes[got..]) {
                Ok(0) => break,
                Ok(read) => got += read,
                Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
                Err(error) => return Err(error),
            }
        }
        bytes.truncate(got);
        // A short read is a file that is not what its length said, and
        // keeping it would serve the same short block for ever.
        if got == want {
            self.entry.put(index, &bytes);
        } else {
            log::debug!(
                "the stream ran out {} byte(s) into block {index} of {}",
                got,
                self.id
            );
        }
        self.loaded = Some(Loaded { index, bytes });
        Ok(())
    }

    /// The check that needs the server: the file behind a cached entry may
    /// have been replaced since it was cached, and the first answer from
    /// the server is where that shows.
    ///
    /// A track that is *wholly* cached makes no request and so gets no
    /// check here — the song's `size`, which the server describes it with
    /// before anything is opened, is what covers that case in
    /// [`Cache::entry`]. What is left uncovered is a file replaced by one
    /// of exactly the same length while a complete copy of the old one is
    /// cached, and the way out of that is Clear.
    fn agrees_with_the_server(&mut self) -> io::Result<()> {
        let Some(fresh) = self.http.len() else {
            return Ok(());
        };
        let stale = self.len != Some(fresh)
            || (self.http.validator().is_some()
                && self.entry.validator().is_some()
                && self.http.validator() != self.entry.validator());
        if !stale {
            return Ok(());
        }
        self.entry.wipe();
        self.loaded = None;
        self.entry
            .describe(&self.id, fresh, self.http.validator().map(str::to_string));
        self.len = Some(fresh);
        Ok(())
    }
}

impl Read for CachedSource {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        // A server that would not say how long the file is gets no cache
        // and no seeking, which is the same trade `HttpSource` makes.
        let Some(len) = self.len else {
            let read = self.http.read(buf)?;
            self.pos += read as u64;
            return Ok(read);
        };
        if buf.is_empty() || self.pos >= len {
            return Ok(0);
        }
        let index = self.pos / self.block;
        if self.loaded.as_ref().map(|loaded| loaded.index) != Some(index) {
            self.load(index)?;
        }
        let Some(loaded) = &self.loaded else {
            return Ok(0);
        };
        let offset = (self.pos - index * self.block) as usize;
        // The block came up short: the file ends here, whatever its length
        // claimed.
        if offset >= loaded.bytes.len() {
            return Ok(0);
        }
        let take = buf.len().min(loaded.bytes.len() - offset);
        buf[..take].copy_from_slice(&loaded.bytes[offset..offset + take]);
        self.pos += take as u64;
        Ok(take)
    }
}

impl Seek for CachedSource {
    fn seek(&mut self, from: SeekFrom) -> io::Result<u64> {
        let Some(len) = self.len else {
            self.pos = self.http.seek(from)?;
            return Ok(self.pos);
        };
        self.pos = match from {
            SeekFrom::Start(offset) => offset,
            SeekFrom::Current(delta) => self.pos.saturating_add_signed(delta),
            SeekFrom::End(delta) => len.saturating_add_signed(delta),
        };
        // The loaded block is deliberately kept: symphonia's check for
        // trailing metadata seeks to the end and back again, and both ends
        // of a short file are the same block.
        Ok(self.pos)
    }
}

impl MediaSource for CachedSource {
    fn is_seekable(&self) -> bool {
        self.len.is_some()
    }

    fn byte_len(&self) -> Option<u64> {
        self.len
    }
}

/// The directory one song's blocks live in. A hash rather than the id
/// itself: an id is the server's to choose, and Subsonic says nothing
/// about it being usable as a file name.
fn key_for(id: &str) -> String {
    let digest = Sha1::digest(id.as_bytes());
    let mut hex = String::with_capacity(digest.len() * 2);
    for byte in digest {
        let _ = write!(hex, "{byte:02x}");
    }
    hex
}

/// Blocks are named by their index, zero-padded so that a listing reads in
/// order and so that no block is ever called `meta`.
fn block_name(index: u64) -> String {
    format!("{index:08}")
}

fn read_meta(dir: &Path) -> Option<Meta> {
    let json = std::fs::read(dir.join("meta")).ok()?;
    serde_json::from_slice(&json).ok()
}

/// What one entry costs on disk: its blocks, not its `meta`, which is
/// noise beside them.
fn blocks_size(dir: &Path) -> u64 {
    let Ok(listing) = std::fs::read_dir(dir) else {
        return 0;
    };
    listing
        .flatten()
        .filter(|entry| {
            entry
                .file_name()
                .to_str()
                .is_some_and(|name| name.bytes().all(|byte| byte.is_ascii_digit()))
        })
        .filter_map(|entry| entry.metadata().ok())
        .map(|meta| meta.len())
        .sum()
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::io::Write;
    use std::net::{TcpListener, TcpStream};
    use std::sync::atomic::AtomicUsize;

    /// A directory of its own per test, removed when the test ends however
    /// it ends.
    struct Temp(PathBuf);

    impl Temp {
        fn new(name: &str) -> Self {
            static COUNT: AtomicUsize = AtomicUsize::new(0);
            let dir = std::env::temp_dir().join(format!(
                "fastsonic-cache-{name}-{}-{}",
                std::process::id(),
                COUNT.fetch_add(1, Ordering::Relaxed)
            ));
            let _ = std::fs::remove_dir_all(&dir);
            Self(dir)
        }

        fn path(&self) -> PathBuf {
            self.0.clone()
        }
    }

    impl Drop for Temp {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    /// Enough of a server to answer what the reader asks: the whole file,
    /// or a range of it, and it counts what it was asked. `stream.view` is
    /// no more than this from here.
    struct Server {
        url: String,
        requests: Arc<AtomicUsize>,
        body: Arc<Mutex<Vec<u8>>>,
        modified: Arc<Mutex<String>>,
        length: Arc<Mutex<bool>>,
    }

    impl Server {
        fn new(body: Vec<u8>) -> Self {
            let listener = TcpListener::bind("127.0.0.1:0").expect("a port");
            let url = format!(
                "http://{}/stream",
                listener.local_addr().expect("an address")
            );
            let requests = Arc::new(AtomicUsize::new(0));
            let body = Arc::new(Mutex::new(body));
            let modified = Arc::new(Mutex::new("Tue, 01 Sep 2026 21:49:45 GMT".to_string()));
            let length = Arc::new(Mutex::new(true));
            let served = Self {
                url,
                requests: Arc::clone(&requests),
                body: Arc::clone(&body),
                modified: Arc::clone(&modified),
                length: Arc::clone(&length),
            };
            std::thread::spawn(move || {
                for stream in listener.incoming() {
                    let Ok(stream) = stream else { break };
                    requests.fetch_add(1, Ordering::Relaxed);
                    let body = body.lock().unwrap_or_else(|p| p.into_inner()).clone();
                    let modified = modified.lock().unwrap_or_else(|p| p.into_inner()).clone();
                    let length = *length.lock().unwrap_or_else(|p| p.into_inner());
                    answer(stream, &body, &modified, length);
                }
            });
            served
        }

        fn requests(&self) -> usize {
            self.requests.load(Ordering::Relaxed)
        }

        fn replace(&self, body: Vec<u8>, modified: &str) {
            *self.body.lock().unwrap_or_else(|p| p.into_inner()) = body;
            *self.modified.lock().unwrap_or_else(|p| p.into_inner()) = modified.to_string();
        }

        /// A server that streams without saying how long the file is —
        /// what a transcoding Navidrome does, and what D12 refuses.
        fn hide_the_length(&self) {
            *self.length.lock().unwrap_or_else(|p| p.into_inner()) = false;
        }
    }

    fn answer(mut stream: TcpStream, body: &[u8], modified: &str, length: bool) {
        let mut request = Vec::new();
        let mut byte = [0_u8; 1];
        // Up to the blank line, which is where the headers end.
        while !request.ends_with(b"\r\n\r\n") {
            match stream.read(&mut byte) {
                Ok(0) | Err(_) => return,
                Ok(_) => request.push(byte[0]),
            }
        }
        // Lowercased before it is read: a header name is case-insensitive,
        // and hyper writes them in lower case — which this server believed
        // otherwise once, and then served whole files where a range was
        // asked for.
        let request = String::from_utf8_lossy(&request).to_lowercase();
        let from = request
            .lines()
            .find_map(|line| line.strip_prefix("range: bytes="))
            .and_then(|range| range.split('-').next()?.parse::<u64>().ok());
        let part = match from {
            Some(from) => &body[(from as usize).min(body.len())..],
            None => body,
        };
        let mut head = String::new();
        let status = if from.is_some() {
            "206 Partial Content"
        } else {
            "200 OK"
        };
        let _ = write!(head, "HTTP/1.1 {status}\r\n");
        let _ = write!(head, "Accept-Ranges: bytes\r\n");
        let _ = write!(head, "Last-Modified: {modified}\r\n");
        // One request per connection, so that what the server counted is
        // exactly what the reader asked for.
        let _ = write!(head, "Connection: close\r\n");
        if length {
            let _ = write!(head, "Content-Length: {}\r\n", part.len());
            if let Some(from) = from {
                let _ = write!(
                    head,
                    "Content-Range: bytes {from}-{}/{}\r\n",
                    body.len().saturating_sub(1),
                    body.len()
                );
            }
        }
        head.push_str("\r\n");
        let _ = stream.write_all(head.as_bytes());
        let _ = stream.write_all(part);
        let _ = stream.flush();
    }

    fn tones(len: usize) -> Vec<u8> {
        (0..len).map(|index| (index % 251) as u8).collect()
    }

    fn cache_at(temp: &Temp, budget: u64, block: usize) -> Arc<Cache> {
        Cache::with_block(temp.path(), budget, block).expect("a cache")
    }

    fn http() -> reqwest::blocking::Client {
        // Through the crate's builder rather than reqwest's: it installs
        // the rustls provider that reqwest panics without.
        crate::blocking_http_client_builder()
            .build()
            .expect("a client")
    }

    /// Reads a source the way symphonia does: the head, then the tail,
    /// then back to the beginning, then all of it.
    fn read_all(source: &mut CachedSource) -> Vec<u8> {
        let mut all = Vec::new();
        source.read_to_end(&mut all).expect("the whole stream");
        all
    }

    fn open(cache: &Arc<Cache>, server: &Server, id: &str, expected: Option<u64>) -> CachedSource {
        let entry = cache.entry(id, expected);
        CachedSource::new(
            http(),
            server.url.clone(),
            Arc::new(Stats::default()),
            entry,
            id,
        )
        .expect("a source")
    }

    #[test]
    fn a_second_play_makes_no_request() {
        let temp = Temp::new("second-play");
        let cache = cache_at(&temp, 8 * 1024 * 1024, 1024);
        let body = tones(4096);
        let server = Server::new(body.clone());

        let mut first = open(&cache, &server, "song-1", None);
        assert!(first.is_seekable());
        assert_eq!(first.byte_len(), Some(4096));
        assert_eq!(read_all(&mut first), body);
        drop(first);
        let after_one = server.requests();
        // Four blocks, read in order, on one connection.
        assert_eq!(after_one, 1);

        // P3.6's done-when, from the outside: the same track again, and
        // the server hears nothing about it.
        let mut again = open(&cache, &server, "song-1", Some(4096));
        assert_eq!(again.byte_len(), Some(4096));
        assert_eq!(read_all(&mut again), body);
        assert_eq!(server.requests(), after_one);
        assert!(cache.stats().hits >= 4);
    }

    #[test]
    fn the_head_and_the_tail_are_cached_without_the_middle() {
        let temp = Temp::new("head-and-tail");
        let cache = cache_at(&temp, 8 * 1024 * 1024, 1024);
        let body = tones(8192);
        let server = Server::new(body.clone());

        // What opening a track costs: symphonia reads the head, looks for
        // trailing metadata, and comes back.
        let mut source = open(&cache, &server, "song-2", None);
        let mut head = [0_u8; 16];
        source.read_exact(&mut head).expect("the head");
        source.seek(SeekFrom::End(-16)).expect("the tail");
        let mut tail = [0_u8; 16];
        source.read_exact(&mut tail).expect("the tail");
        assert_eq!(head, body[..16]);
        assert_eq!(tail, body[8176..]);
        drop(source);

        let blocks = cache.stats();
        assert_eq!(blocks.entries, 1);
        // The first block and the last, and nothing in between: a track
        // opened and abandoned costs two blocks rather than a whole file.
        assert_eq!(blocks.bytes, 2048);

        // Opening it again asks the server for nothing, which is the three
        // round trips this cache exists to remove.
        let before = server.requests();
        let mut again = open(&cache, &server, "song-2", Some(8192));
        let mut head = [0_u8; 16];
        again.read_exact(&mut head).expect("the head");
        again.seek(SeekFrom::End(-16)).expect("the tail");
        let mut tail = [0_u8; 16];
        again.read_exact(&mut tail).expect("the tail");
        assert_eq!(server.requests(), before);
        assert_eq!(head, body[..16]);
        assert_eq!(tail, body[8176..]);

        // And the middle still comes from the server, once.
        again.seek(SeekFrom::Start(4096)).expect("the middle");
        let mut middle = [0_u8; 16];
        again.read_exact(&mut middle).expect("the middle");
        assert_eq!(middle, body[4096..4112]);
        assert_eq!(server.requests(), before + 1);
    }

    #[test]
    fn a_file_whose_size_changed_on_the_server_is_recached_before_a_request() {
        let temp = Temp::new("resized");
        let cache = cache_at(&temp, 8 * 1024 * 1024, 1024);
        let first = tones(2048);
        let server = Server::new(first.clone());
        let mut source = open(&cache, &server, "song-3", None);
        assert_eq!(read_all(&mut source), first);
        drop(source);

        // A re-encode: the same song id, a longer file. The size the
        // server describes the song with is what catches it, before any
        // request is made — which matters, because a wholly cached track
        // otherwise makes none.
        let second = tones(3072);
        server.replace(second.clone(), "Wed, 02 Sep 2026 09:00:00 GMT");
        let mut source = open(&cache, &server, "song-3", Some(3072));
        assert_eq!(source.byte_len(), Some(3072));
        assert_eq!(read_all(&mut source), second);
        drop(source);
        assert_eq!(cache.stats().bytes, 3072);
    }

    #[test]
    fn a_file_replaced_at_the_same_size_is_noticed_at_the_first_request() {
        let temp = Temp::new("retagged");
        let cache = cache_at(&temp, 8 * 1024 * 1024, 1024);
        let first = tones(4096);
        let server = Server::new(first.clone());
        // Only the head, so the entry is incomplete and the next play has
        // to ask the server for the rest of it.
        let mut source = open(&cache, &server, "song-4", None);
        let mut head = [0_u8; 16];
        source.read_exact(&mut head).expect("the head");
        drop(source);

        // A re-tag that happens to keep the length: `size` cannot see it,
        // so what sees it is `Last-Modified` on the answer to the first
        // request.
        let second: Vec<u8> = first.iter().map(|byte| byte ^ 0xff).collect();
        server.replace(second.clone(), "Thu, 03 Sep 2026 09:00:00 GMT");
        let mut source = open(&cache, &server, "song-4", Some(4096));
        source
            .seek(SeekFrom::Start(1024))
            .expect("the second block");
        let mut rest = Vec::new();
        source.read_to_end(&mut rest).expect("the rest");
        assert_eq!(rest, second[1024..]);
        drop(source);

        // The stale head went with it, so the next play is the new file
        // whole rather than the old file's first block and the new file's
        // rest.
        let mut source = open(&cache, &server, "song-4", Some(4096));
        assert_eq!(read_all(&mut source), second);
    }

    #[test]
    fn the_budget_evicts_the_least_recently_played() {
        let temp = Temp::new("budget");
        // Room for two tracks of two blocks.
        let cache = cache_at(&temp, 4096, 1024);
        let body = tones(2048);
        let server = Server::new(body.clone());
        for id in ["song-a", "song-b"] {
            let mut source = open(&cache, &server, id, None);
            assert_eq!(read_all(&mut source), body);
        }
        assert_eq!(cache.stats().entries, 2);
        assert_eq!(cache.stats().bytes, 4096);

        // Playing the first again makes the second the oldest.
        let mut again = open(&cache, &server, "song-a", Some(2048));
        assert_eq!(read_all(&mut again), body);
        drop(again);

        let mut third = open(&cache, &server, "song-c", None);
        assert_eq!(read_all(&mut third), body);
        drop(third);
        assert_eq!(cache.stats().bytes, 4096);
        assert!(temp.path().join(key_for("song-a")).exists());
        assert!(!temp.path().join(key_for("song-b")).exists());
        assert!(temp.path().join(key_for("song-c")).exists());
    }

    #[test]
    fn the_track_being_played_is_never_evicted() {
        let temp = Temp::new("busy");
        let cache = cache_at(&temp, 1024, 1024);
        let body = tones(2048);
        let server = Server::new(body.clone());
        // One track, twice its budget, still being read: over budget for
        // as long as it plays is right, and a stutter is not.
        let mut playing = open(&cache, &server, "song-d", None);
        assert_eq!(read_all(&mut playing), body);
        assert_eq!(cache.stats().bytes, 2048);
        assert!(temp.path().join(key_for("song-d")).exists());
        // Once it is no longer playing, the next thing to arrive collects
        // it.
        drop(playing);
        let mut next = open(&cache, &server, "song-e", None);
        assert_eq!(read_all(&mut next), body);
        assert!(!temp.path().join(key_for("song-d")).exists());
    }

    #[test]
    fn what_was_played_last_survives_a_restart() {
        let temp = Temp::new("restart");
        let block = 1024;
        let body = tones(2048);
        let server = Server::new(body.clone());
        {
            let cache = cache_at(&temp, 8 * 1024 * 1024, block);
            for id in ["song-f", "song-g"] {
                let mut source = open(&cache, &server, id, None);
                assert_eq!(read_all(&mut source), body);
            }
            // `meta`'s timestamp is what the order survives in, and a
            // file system's timestamps are not always finer than a
            // second — so make the difference visible rather than hope.
            let older = std::fs::metadata(temp.path().join(key_for("song-f")).join("meta"))
                .expect("meta")
                .modified()
                .expect("a timestamp");
            let file = std::fs::OpenOptions::new()
                .write(true)
                .open(temp.path().join(key_for("song-f")).join("meta"))
                .expect("meta");
            file.set_times(
                std::fs::FileTimes::new().set_modified(older - std::time::Duration::from_secs(60)),
            )
            .expect("an older timestamp");
        }
        // A new run, a budget for one track, and the one played longer
        // ago is the one that goes.
        let cache = cache_at(&temp, 2048, block);
        assert_eq!(cache.stats().entries, 1);
        assert!(!temp.path().join(key_for("song-f")).exists());
        assert!(temp.path().join(key_for("song-g")).exists());
    }

    #[test]
    fn a_stream_of_unknown_length_is_not_cached() {
        let temp = Temp::new("no-length");
        let cache = cache_at(&temp, 8 * 1024 * 1024, 1024);
        let body = tones(2048);
        let server = Server::new(body.clone());
        server.hide_the_length();
        let mut source = open(&cache, &server, "song-h", None);
        // The trade `HttpSource` already makes, kept: no length, no
        // seeking, and nothing worth keeping either.
        assert!(!source.is_seekable());
        assert_eq!(source.byte_len(), None);
        assert_eq!(read_all(&mut source), body);
        drop(source);
        assert_eq!(cache.stats().bytes, 0);
        assert_eq!(cache.stats().entries, 0);
    }

    #[test]
    fn entries_without_the_current_layout_are_thrown_away() {
        let temp = Temp::new("layouts");
        let cache = cache_at(&temp, 8 * 1024 * 1024, 1024);
        let body = tones(1024);
        let server = Server::new(body.clone());
        let mut source = open(&cache, &server, "song-i", None);
        assert_eq!(read_all(&mut source), body);
        drop(source);

        // The retired player's entries had no metadata in our format.
        let stranger = temp.path().join("aa");
        std::fs::create_dir_all(&stranger).expect("a stranger");
        std::fs::write(stranger.join("some-file-id"), b"not ours").expect("a stranger");

        // An entry written by a build that cut files up differently is
        // ours, and unusable.
        let mine = temp.path().join(key_for("song-i"));
        let meta: Meta = read_meta(&mine).expect("meta");
        std::fs::write(
            mine.join("meta"),
            serde_json::to_vec(&Meta { block: 64, ..meta }).expect("json"),
        )
        .expect("meta");

        let cache = cache_at(&temp, 8 * 1024 * 1024, 1024);
        assert_eq!(cache.stats().entries, 0);
        assert!(!mine.exists());
        assert!(!stranger.exists());
    }

    #[test]
    fn clearing_it_leaves_the_directory_empty() {
        let temp = Temp::new("clear");
        let cache = cache_at(&temp, 8 * 1024 * 1024, 1024);
        let body = tones(2048);
        let server = Server::new(body.clone());
        let mut source = open(&cache, &server, "song-j", None);
        assert_eq!(read_all(&mut source), body);
        drop(source);
        assert_eq!(cache.stats().entries, 1);
        assert_eq!(cache.clear(), body.len() as u64, "it says what it freed");
        assert_eq!(cache.stats().entries, 0);
        assert_eq!(cache.stats().bytes, 0);
        assert!(!temp.path().join(key_for("song-j")).exists());
    }

    #[test]
    fn a_block_is_named_after_its_index_and_never_after_the_meta() {
        assert_eq!(block_name(0), "00000000");
        assert_eq!(block_name(41), "00000041");
        assert_ne!(block_name(0), "meta");
        // The id is the server's to choose; the directory name is ours.
        assert_eq!(key_for("song-1").len(), 40);
        assert_ne!(key_for("song-1"), key_for("song-2"));
        assert_eq!(key_for("a/b?c=d"), key_for("a/b?c=d"));
        assert!(
            key_for("a/b?c=d")
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit())
        );
    }
}
