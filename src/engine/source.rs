//! A `stream.view` response read as if it were a file on disk.
//!
//! D12: the stream is the file, unmodified, so the server serves it with
//! `Accept-Ranges: bytes` and a real `Content-Length`, and seeking is an
//! HTTP `Range` request rather than anything the server has to do. The
//! decoder above this seeks whenever it likes; each seek drops the body
//! being read and the next read reopens the stream at the new offset, which
//! is what makes a seek one request instead of a download of everything in
//! between.
//!
//! Grown from the P3.1 spike (`examples/stream_probe.rs`), with one thing
//! added: a server that answers a `Range` request with the whole file is
//! caught rather than believed.
//!
//! This is the stream itself. [`super::cache`] sits in front of it and
//! reads through it in blocks, so a track played before is read from disk
//! and this module is never asked at all.

use std::io::{self, Read, Seek, SeekFrom};
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU32, Ordering};

use reqwest::header::{ACCEPT_RANGES, CONTENT_LENGTH, CONTENT_RANGE, ETAG, LAST_MODIFIED, RANGE};
use symphonia::core::io::MediaSource;

/// How much of a range-ignoring server's answer is skipped over before
/// giving up on the seek. A tenth of a typical album track.
const MAX_SKIPPED: u64 = 4 * 1024 * 1024;

/// What the HTTP side of one track did. Readable after symphonia has taken
/// ownership of the reader, which is why it is shared rather than returned.
#[derive(Debug, Default)]
pub struct Stats {
    gets: AtomicU32,
}

impl Stats {
    /// How many requests this track has cost so far. Three before a note
    /// plays is what an uncached track costs — symphonia probes for
    /// trailing metadata — and none is what a track played before costs,
    /// which is what [`super::cache`] is for.
    pub fn gets(&self) -> u32 {
        self.gets.load(Ordering::Relaxed)
    }
}

pub struct HttpSource {
    http: reqwest::blocking::Client,
    url: String,
    /// The whole file's length, learned from the first response.
    len: Option<u64>,
    /// Whether the server offered byte ranges.
    ranges: bool,
    /// The `ETag` or `Last-Modified` of the file, from the first answer,
    /// so that [`super::cache`] can tell a file that has been replaced
    /// from the one it kept. `None` from a server that offers neither.
    validator: Option<String>,
    pos: u64,
    /// The response being read, or `None` after a seek. Behind a `Mutex`
    /// only because a `MediaSource` must be `Sync`.
    body: Mutex<Option<reqwest::blocking::Response>>,
    stats: Arc<Stats>,
}

impl HttpSource {
    /// Opens the stream at once rather than on the first read: symphonia
    /// asks whether the source is seekable before it reads a byte, and the
    /// answer is the `Content-Length` of the first response.
    pub fn new(
        http: reqwest::blocking::Client,
        url: String,
        stats: Arc<Stats>,
    ) -> io::Result<Self> {
        let mut source = Self::lazy(http, url, stats);
        source.open()?;
        Ok(source)
    }

    /// The same stream, not opened yet.
    ///
    /// For [`super::cache::CachedSource`], which may not need it at all: a
    /// track whose blocks are all on disk is played without one request,
    /// and that includes the request this type otherwise makes to learn
    /// how long the file is.
    pub fn lazy(http: reqwest::blocking::Client, url: String, stats: Arc<Stats>) -> Self {
        Self {
            http,
            url,
            len: None,
            ranges: false,
            validator: None,
            pos: 0,
            body: Mutex::new(None),
            stats,
        }
    }

    /// Opens the stream if it is not open, at wherever the reader is.
    /// Idempotent, and the one way to make the server answer without
    /// reading a byte of the answer.
    pub fn prime(&mut self) -> io::Result<()> {
        if self
            .body
            .get_mut()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .is_none()
        {
            self.open()?;
        }
        Ok(())
    }

    /// The whole file's length, once the server has said. `None` before
    /// the first answer, and from a server that will not say.
    pub fn len(&self) -> Option<u64> {
        self.len
    }

    /// What the server says would change if the file did.
    pub fn validator(&self) -> Option<&str> {
        self.validator.as_deref()
    }

    /// Asks for the file from `pos`. The first request carries no `Range`,
    /// so it looks like what ordinary playback sends.
    fn open(&mut self) -> io::Result<()> {
        let mut request = self.http.get(&self.url);
        if self.pos > 0 {
            request = request.header(RANGE, format!("bytes={}-", self.pos));
        }
        log::debug!("GET from byte {}", self.pos);
        let response = request
            .send()
            .and_then(reqwest::blocking::Response::error_for_status)
            .map_err(io::Error::other)?;
        self.stats.gets.fetch_add(1, Ordering::Relaxed);

        let headers = response.headers();
        self.ranges = headers
            .get(ACCEPT_RANGES)
            .and_then(|value| value.to_str().ok())
            .is_some_and(|value| value.eq_ignore_ascii_case("bytes"));
        if self.validator.is_none() {
            self.validator = headers
                .get(ETAG)
                .or_else(|| headers.get(LAST_MODIFIED))
                .and_then(|value| value.to_str().ok())
                .map(str::to_string);
        }
        if self.len.is_none() {
            // On the first request `Content-Length` is the file; on a range
            // request it is only the tail, so the whole length comes from
            // `Content-Range` instead.
            self.len = if self.pos == 0 {
                headers
                    .get(CONTENT_LENGTH)
                    .and_then(|value| value.to_str().ok())
                    .and_then(|value| value.parse::<u64>().ok())
            } else {
                headers
                    .get(CONTENT_RANGE)
                    .and_then(|value| value.to_str().ok())
                    .and_then(content_range_total)
            };
        }

        let mut response = response;
        // A server that ignores `Range` answers 200 with the whole file, and
        // believing it would play the track from the beginning at every
        // seek. Read forward to where the seek asked for instead, and only
        // as far as that is cheaper than being wrong.
        if self.pos > 0 && response.status() == reqwest::StatusCode::OK {
            if self.pos > MAX_SKIPPED {
                return Err(io::Error::other(
                    "the server ignored the byte range and the seek is too far in to read up to",
                ));
            }
            log::warn!(
                "the server ignored a byte range; reading forward to byte {}",
                self.pos
            );
            io::copy(&mut response.by_ref().take(self.pos), &mut io::sink())?;
        }
        *self
            .body
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(response);
        Ok(())
    }
}

/// The total length out of a `Content-Range: bytes 100-199/12345` header.
/// A server that does not know the total says `*`, and then nothing here
/// knows it either.
fn content_range_total(value: &str) -> Option<u64> {
    value.rsplit('/').next()?.trim().parse().ok()
}

impl Read for HttpSource {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if self
            .body
            .get_mut()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .is_none()
        {
            self.open()?;
        }
        let read = match self
            .body
            .get_mut()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
        {
            Some(body) => body.read(buf)?,
            None => 0,
        };
        self.pos += read as u64;
        Ok(read)
    }
}

impl Seek for HttpSource {
    fn seek(&mut self, from: SeekFrom) -> io::Result<u64> {
        let target = match from {
            SeekFrom::Start(offset) => offset,
            SeekFrom::Current(delta) => self.pos.saturating_add_signed(delta),
            SeekFrom::End(delta) => self
                .len
                .ok_or_else(|| io::Error::other("the server did not say how long the file is"))?
                .saturating_add_signed(delta),
        };
        if target != self.pos {
            // Dropping the response is what makes the seek cost one request.
            *self
                .body
                .get_mut()
                .unwrap_or_else(|poisoned| poisoned.into_inner()) = None;
            self.pos = target;
        }
        Ok(self.pos)
    }
}

impl MediaSource for HttpSource {
    /// Only with a length: symphonia seeks from the end, and a stream that
    /// cannot say how long it is cannot answer that.
    fn is_seekable(&self) -> bool {
        self.len.is_some()
    }

    fn byte_len(&self) -> Option<u64> {
        self.len
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_whole_length_comes_out_of_a_content_range() {
        assert_eq!(content_range_total("bytes 100-199/12345"), Some(12_345));
        assert_eq!(content_range_total("bytes 0-0/1"), Some(1));
        // A server that will not say. Nothing knows the length then, and
        // `is_seekable` answers no rather than guessing.
        assert_eq!(content_range_total("bytes 100-199/*"), None);
        assert_eq!(content_range_total(""), None);
    }
}
