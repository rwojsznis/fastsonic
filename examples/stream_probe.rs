//! Spike (P3.1): take one `stream.view` from the music server, decode it in
//! this process, and put it out of the speakers. No UI, no queue, no engine
//! contract — the questions it answers are whether the chain
//! `migration/02-audio-engine.md` proposes fits together at all, and which of
//! a real library's formats symphonia can actually decode.
//!
//! ```sh
//! cargo run --example stream_probe                # one song per format
//! cargo run --example stream_probe -- <song-id>   # that song, the whole way
//! cargo run --example stream_probe -- --list      # what the library has
//! ```
//!
//! It plays three seconds of each song, seeks to the middle by HTTP `Range`
//! (D12), plays one second more, and prints what happened. `--seconds N`
//! changes the three, `--silent` decodes without opening an output device, so
//! it runs where there is no sound card.
//!
//! `FASTSONIC_TEST_SERVER`, `FASTSONIC_TEST_USER` and
//! `FASTSONIC_TEST_PASSWORD` point it at a server other than
//! `migration/devserver` (`http://localhost:4533`, admin / fastsonic).

use std::io::{self, Read, Seek, SeekFrom};
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use symphonia::core::codecs::audio::{AudioCodecParameters, AudioDecoderOptions};
use symphonia::core::errors::Error as DecodeError;
use symphonia::core::formats::probe::Hint;
use symphonia::core::formats::{FormatOptions, SeekMode, SeekTo, TrackType};
use symphonia::core::io::{MediaSource, MediaSourceStream};
use symphonia::core::meta::MetadataOptions;
use symphonia::core::units::Time;

use fastsonic::api::NetActivity;
use fastsonic::api::subsonic::{Child, Credentials, SubsonicClient, redacted};

/// Queued rodio chunks before the decode loop waits, as `src/sink.rs` does.
const QUEUE_LIMIT: usize = 12;

fn main() -> anyhow::Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("warn")).init();

    let mut seconds = 3.0_f64;
    let mut silent = false;
    let mut list = false;
    let mut song_id = None;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--seconds" => {
                seconds = args
                    .next()
                    .ok_or_else(|| anyhow::anyhow!("--seconds wants a number"))?
                    .parse()?;
            }
            "--silent" => silent = true,
            "--list" => list = true,
            "--help" | "-h" => {
                println!("usage: stream_probe [--list] [--silent] [--seconds N] [song-id]");
                return Ok(());
            }
            other => song_id = Some(other.to_string()),
        }
    }

    let server = env("FASTSONIC_TEST_SERVER", "http://localhost:4533");
    let username = env("FASTSONIC_TEST_USER", "admin");
    let password = env("FASTSONIC_TEST_PASSWORD", "fastsonic");

    let client = SubsonicClient::new(
        fastsonic::http_client_builder().build()?,
        Arc::new(NetActivity::default()),
        20,
    );
    client.set_credentials(Some(Credentials::from_password(
        &server, &username, &password,
    )));

    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    let songs = runtime.block_on(async {
        client.ping().await?;
        match &song_id {
            Some(id) => Ok::<_, anyhow::Error>(vec![client.get_song(id).await?]),
            None => Ok(client.random_songs(200).await?),
        }
    })?;
    if songs.is_empty() {
        anyhow::bail!("{server} has no songs to play");
    }

    if list {
        for song in library_order(songs) {
            println!("{}", describe(&song));
        }
        return Ok(());
    }

    // One song per container format when nothing was asked for by name: the
    // point of the spike is the decoder set, not the music.
    let chosen = match song_id {
        Some(_) => songs,
        None => one_per_format(songs),
    };

    let http = fastsonic::blocking_http_client_builder()
        // A stream is read for as long as it plays, so only the wait for the
        // first byte is worth a deadline.
        .connect_timeout(Duration::from_secs(10))
        .build()?;

    // Opened once, and kept open across every track: reopening a device
    // between formats would measure the device, not the decoder.
    let output = if silent {
        None
    } else {
        match rodio::OutputStreamBuilder::open_default_stream() {
            Ok(mut stream) => {
                stream.log_on_drop(false);
                println!("output: {} Hz", stream.config().sample_rate());
                Some(stream)
            }
            Err(error) => {
                println!("no audio output ({error}); decoding silently");
                None
            }
        }
    };

    let mut outcomes = Vec::new();
    for song in &chosen {
        println!("\n--- {}", describe(song));
        let url = client.stream_url(&song.id)?;
        log::info!("streaming {}", redacted(&url));
        let outcome = play(&http, &url, song, output.as_ref(), seconds);
        match &outcome {
            Ok(played) => println!("    {}", played.summary()),
            Err(error) => println!("    FAILED: {error}"),
        }
        outcomes.push((song.clone(), outcome));
    }

    println!("\n{:-<96}", "");
    println!(
        "{:<10} {:<22} {:>7} {:>4} {:>7} {:>7} {:>9}  song",
        "format", "codec", "rate", "ch", "ttfa", "gets", "seek"
    );
    for (song, outcome) in &outcomes {
        let format = song.suffix.clone().unwrap_or_else(|| "?".into());
        match outcome {
            Ok(played) => println!(
                "{:<10} {:<22} {:>7} {:>4} {:>6}ms {:>7} {:>9}  {}",
                format,
                played.codec,
                played.rate,
                played.channels,
                played.first_audio.as_millis(),
                played.gets,
                played
                    .seek
                    .as_ref()
                    .map(|seek| format!("{seek:.2}s"))
                    .unwrap_or_else(|| "-".into()),
                song.title,
            ),
            Err(error) => println!("{format:<10} FAILED: {error}"),
        }
    }
    let failed = outcomes.iter().filter(|(_, done)| done.is_err()).count();
    println!(
        "\n{} of {} formats played",
        outcomes.len() - failed,
        outcomes.len()
    );
    if failed > 0 {
        anyhow::bail!("{failed} of {} formats did not play", outcomes.len());
    }
    Ok(())
}

fn env(name: &str, fallback: &str) -> String {
    std::env::var(name).unwrap_or_else(|_| fallback.to_string())
}

fn describe(song: &Child) -> String {
    format!(
        "{} — {} [{} {} Hz {} kbps {}]",
        song.artist.as_deref().unwrap_or("[no artist]"),
        song.title,
        song.suffix.as_deref().unwrap_or("?"),
        song.sampling_rate.unwrap_or_default(),
        song.bit_rate.unwrap_or_default(),
        song.content_type.as_deref().unwrap_or("?"),
    )
}

/// Names a codec the way the library talks about it, so a build without a
/// decoder for it says which one rather than printing a number. Symphonia's
/// own name comes from the decoder, which is exactly what is missing here.
fn codec_label(params: &AudioCodecParameters) -> String {
    use symphonia::core::codecs::audio::well_known::{
        CODEC_ID_AAC, CODEC_ID_ALAC, CODEC_ID_FLAC, CODEC_ID_MP3, CODEC_ID_OPUS, CODEC_ID_VORBIS,
    };
    let known = [
        (CODEC_ID_OPUS, "Opus"),
        (CODEC_ID_VORBIS, "Vorbis"),
        (CODEC_ID_FLAC, "FLAC"),
        (CODEC_ID_MP3, "MP3"),
        (CODEC_ID_AAC, "AAC"),
        (CODEC_ID_ALAC, "ALAC"),
    ];
    known
        .iter()
        .find(|(id, _)| *id == params.codec)
        .map(|(_, name)| (*name).to_string())
        .unwrap_or_else(|| format!("{}", params.codec))
}

fn library_order(mut songs: Vec<Child>) -> Vec<Child> {
    // Sorted, and sorted all the way down to the id: the two-disc fixture
    // has a track 1 on each disc under the same album name, and a probe that
    // plays a different one each run cannot be compared with the last one.
    songs.sort_by(|left, right| {
        let key = |song: &Child| {
            (
                song.suffix.clone(),
                song.album.clone(),
                song.disc_number,
                song.track,
                song.id.clone(),
            )
        };
        key(left).cmp(&key(right))
    });
    songs
}

/// The first song of each container format, in a stable order, so two runs
/// of the probe compare.
fn one_per_format(songs: Vec<Child>) -> Vec<Child> {
    let mut chosen: Vec<Child> = Vec::new();
    for song in library_order(songs) {
        if !chosen.iter().any(|kept| kept.suffix == song.suffix) {
            chosen.push(song);
        }
    }
    chosen
}

/// What one track's playback taught us.
struct Played {
    codec: String,
    rate: u32,
    channels: usize,
    /// From asking for the stream to the first decoded audio.
    first_audio: Duration,
    /// How many HTTP requests the whole track took. Two means the seek made
    /// its own; more means something reopened the stream unasked.
    gets: u32,
    /// Where the decoder landed after the seek, in seconds.
    seek: Option<f64>,
    frames: u64,
    length: Option<u64>,
    ranges: bool,
}

impl Played {
    fn summary(&self) -> String {
        format!(
            "{} {} Hz, {} ch, {} frames decoded, first audio in {} ms, {} HTTP GET(s), {}{}",
            self.codec,
            self.rate,
            self.channels,
            self.frames,
            self.first_audio.as_millis(),
            self.gets,
            match self.length {
                Some(length) => format!("{length} bytes"),
                None => "no length".into(),
            },
            if self.ranges {
                ", ranges"
            } else {
                ", no ranges"
            },
        )
    }
}

fn play(
    http: &reqwest::blocking::Client,
    url: &str,
    song: &Child,
    output: Option<&rodio::OutputStream>,
    seconds: f64,
) -> anyhow::Result<Played> {
    let started = Instant::now();
    let stats = Arc::new(Stats::default());
    let source = HttpSource::new(http.clone(), url.to_string(), Arc::clone(&stats))?;

    let stream = MediaSourceStream::new(Box::new(source), Default::default());
    let mut hint = Hint::new();
    if let Some(suffix) = &song.suffix {
        hint.with_extension(suffix);
    }
    if let Some(content_type) = &song.content_type {
        hint.mime_type(content_type);
    }
    let mut format = symphonia::default::get_probe().probe(
        &hint,
        stream,
        FormatOptions::default(),
        MetadataOptions::default(),
    )?;

    let track = format
        .first_track_known_codec(TrackType::Audio)
        .ok_or_else(|| anyhow::anyhow!("no audio track this build can decode"))?;
    let track_id = track.id;
    let time_base = track.time_base;
    let params = track
        .codec_params
        .as_ref()
        .and_then(|params| params.audio())
        .ok_or_else(|| anyhow::anyhow!("the track carries no audio codec parameters"))?
        .clone();
    // `fastsonic::opus::codecs()`, not symphonia's own registry: symphonia
    // has no Opus decoder and a music library has Opus in it.
    let mut decoder = fastsonic::opus::codecs()
        .make_audio_decoder(&params, &AudioDecoderOptions::default())
        .map_err(|error| {
            anyhow::anyhow!(
                "no {} decoder in this build ({error})",
                codec_label(&params)
            )
        })?;

    let sink = output.map(|stream| rodio::Sink::connect_new(stream.mixer()));
    let mut samples: Vec<f32> = Vec::new();
    let mut first_audio = None;
    let codec = decoder.codec_info().short_name.to_string();
    let mut rate = 0;
    let mut channels = 0;
    let mut frames = 0_u64;
    let mut seeked = None;
    let mut played = 0.0_f64;

    // Where the seek aims: three quarters in, so it lands past whatever was
    // already played and nowhere near a boundary a seek table is bound to
    // have an entry for. The server's duration is in whole seconds, which is
    // precise enough to choose a target.
    let target = song.duration.unwrap_or(0) as f64 * 0.75;

    while let Some(packet) = format.next_packet()? {
        if packet.track_id != track_id {
            continue;
        }
        let decoded = match decoder.decode(&packet) {
            Ok(decoded) => decoded,
            // A packet that will not decode is skipped, exactly as the
            // engine will have to skip one.
            Err(DecodeError::DecodeError(reason)) => {
                log::warn!("undecodable packet skipped: {reason}");
                continue;
            }
            Err(error) => return Err(error.into()),
        };
        let spec = decoded.spec();
        rate = spec.rate();
        channels = spec.channels().count();
        frames += decoded.frames() as u64;
        samples.clear();
        decoded.copy_to_vec_interleaved(&mut samples);
        if first_audio.is_none() {
            first_audio = Some(started.elapsed());
        }
        played += decoded.frames() as f64 / f64::from(rate.max(1));

        if let Some(sink) = &sink {
            sink.append(rodio::buffer::SamplesBuffer::new(
                channels as rodio::ChannelCount,
                rate as rodio::SampleRate,
                samples.as_slice(),
            ));
            // Let the device drain, or the whole track decodes into memory.
            while sink.len() > QUEUE_LIMIT {
                std::thread::sleep(Duration::from_millis(10));
            }
        }

        let position = time_base
            .and_then(|base| base.calc_time(packet.pts))
            .map(|time| time.as_secs_f64())
            .unwrap_or(played);

        // Once the asked-for seconds have played, jump ahead by byte range
        // and carry on from there. One second after the jump is enough to
        // hear whether the join is clean.
        if seeked.is_none() && played >= seconds && target > seconds {
            let time = Time::try_from_secs_f64(target)
                .ok_or_else(|| anyhow::anyhow!("{target} is not a time"))?;
            let landed = format.seek(
                SeekMode::Accurate,
                SeekTo::Time {
                    time,
                    track_id: Some(track_id),
                },
            )?;
            decoder.reset();
            seeked = Some(
                time_base
                    .and_then(|base| base.calc_time(landed.actual_ts))
                    .map(|time| time.as_secs_f64())
                    .unwrap_or(f64::NAN),
            );
            played = 0.0;
            continue;
        }
        if seeked.is_some() && played >= 1.0 {
            break;
        }
        if seeked.is_none() && target <= seconds && position >= seconds {
            break;
        }
    }

    if let Some(sink) = &sink {
        sink.sleep_until_end();
    }
    let first_audio =
        first_audio.ok_or_else(|| anyhow::anyhow!("the track decoded to no audio at all"))?;
    Ok(Played {
        codec,
        rate,
        channels,
        first_audio,
        gets: stats.gets.load(Ordering::Relaxed),
        seek: seeked,
        frames,
        length: stats.length(),
        ranges: stats.ranges(),
    })
}

/// What the HTTP side of one track did, readable after symphonia has taken
/// ownership of the reader.
#[derive(Default)]
struct Stats {
    gets: AtomicU32,
    length: AtomicU64,
    accept_ranges: Mutex<Option<String>>,
}

impl Stats {
    fn length(&self) -> Option<u64> {
        match self.length.load(Ordering::Relaxed) {
            0 => None,
            length => Some(length),
        }
    }

    /// Whether the server said it would serve byte ranges — the header D12
    /// depends on.
    fn ranges(&self) -> bool {
        self.accept_ranges
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .as_deref()
            .is_some_and(|value| value.eq_ignore_ascii_case("bytes"))
    }
}

/// A `stream.view` response read as a file: a `GET` from the current
/// position, reopened with a `Range` header whenever the decoder seeks.
struct HttpSource {
    http: reqwest::blocking::Client,
    url: String,
    /// `Content-Length` of the whole file, learned from the first response.
    len: Option<u64>,
    pos: u64,
    /// The response being read, or `None` before the first read and after a
    /// seek. Behind a `Mutex` only because a `MediaSource` must be `Sync`.
    body: Mutex<Option<reqwest::blocking::Response>>,
    stats: Arc<Stats>,
}

impl HttpSource {
    /// Opens the stream at once rather than on the first read, because
    /// symphonia asks whether the source is seekable — and the answer is the
    /// `Content-Length` of the first response — before it reads a byte.
    fn new(http: reqwest::blocking::Client, url: String, stats: Arc<Stats>) -> io::Result<Self> {
        let mut source = Self {
            http,
            url,
            len: None,
            pos: 0,
            body: Mutex::new(None),
            stats,
        };
        source.open()?;
        Ok(source)
    }

    /// Asks for the file from `pos`. The first request carries no `Range`,
    /// so it looks like what ordinary playback sends.
    fn open(&mut self) -> io::Result<()> {
        let mut request = self.http.get(&self.url);
        if self.pos > 0 {
            request = request.header(reqwest::header::RANGE, format!("bytes={}-", self.pos));
        }
        log::debug!("GET from byte {}", self.pos);
        let response = request
            .send()
            .and_then(reqwest::blocking::Response::error_for_status)
            .map_err(io::Error::other)?;
        self.stats.gets.fetch_add(1, Ordering::Relaxed);

        let headers = response.headers();
        if let Some(accept) = headers
            .get(reqwest::header::ACCEPT_RANGES)
            .and_then(|value| value.to_str().ok())
        {
            *self
                .stats
                .accept_ranges
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(accept.to_string());
        }
        if self.len.is_none() {
            // On the first request `Content-Length` is the file; on a range
            // request it is only the tail, so the whole length comes from
            // `Content-Range` instead.
            let length = if self.pos == 0 {
                headers
                    .get(reqwest::header::CONTENT_LENGTH)
                    .and_then(|value| value.to_str().ok())
                    .and_then(|value| value.parse::<u64>().ok())
            } else {
                headers
                    .get(reqwest::header::CONTENT_RANGE)
                    .and_then(|value| value.to_str().ok())
                    .and_then(|value| value.rsplit('/').next().map(str::to_string))
                    .and_then(|total| total.parse::<u64>().ok())
            };
            if let Some(length) = length {
                self.len = Some(length);
                self.stats.length.store(length, Ordering::Relaxed);
            }
        }
        *self
            .body
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(response);
        Ok(())
    }
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
            // The next read reopens the stream at the new offset. Dropping
            // the response here is what makes a seek one request rather than
            // a download of everything in between.
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
