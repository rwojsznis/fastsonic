//! One track, open: the HTTP body, the container reader, the decoder, and
//! the conversion to what the output takes.
//!
//! Everything above this asks for the next chunk of interleaved stereo at
//! the device's rate and does not care what the file was. That is the whole
//! point of the arrangement: a library holds FLAC beside MP3 beside 48 kHz
//! Opus, and only this module knows.
//!
//! Decoding goes through [`crate::opus::codecs`] rather than symphonia's own
//! registry, which has no Opus decoder in it (D14).
//!
//! The bytes come from [`super::source`], or from [`super::cache`] reading
//! through it — which one is a setting, and the difference is only in what
//! opening a track costs.

use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result, anyhow};
use symphonia::core::codecs::audio::{AudioCodecParameters, AudioDecoder, AudioDecoderOptions};
use symphonia::core::errors::Error as SymphoniaError;
use symphonia::core::formats::probe::Hint;
use symphonia::core::formats::{FormatOptions, FormatReader, SeekMode, SeekTo, TrackType};
use symphonia::core::io::{MediaSourceStream, MediaSourceStreamOptions};
use symphonia::core::meta::MetadataOptions;
use symphonia::core::units::{Time, TimeBase};

use crate::resample::Resampler;

use super::cache::{Cache, CachedSource};
use super::output::CHANNELS;
use super::source::{HttpSource, Stats};

/// How much of one packet a warm-up or a padding trim may take before the
/// sums behind it are not to be believed. No encoder's warm-up is a second
/// long; if the arithmetic says it is, this build has the file's timeline
/// wrong and cutting a second of music would be the worse mistake.
const MAX_TRIM_SECONDS: u32 = 1;

/// The frames of a track that are music, in the track's own timeline.
///
/// A block encoder writes more frames than the music has: a warm-up at the
/// start, padding at the end. Playing them is the click at a gapless join,
/// and 6.5 ms of Opus warm-up at every track of an album is what P3.4 is
/// about. Symphonia reports the two in two different currencies and no
/// container uses both — see [`window`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Window {
    /// The first frame that is music.
    start: i64,
    /// One past the last, where the track says how long it is.
    end: Option<i64>,
}

/// What one decoded packet is worth keeping, in frames, and where the first
/// of them sits in the track.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Trim {
    /// Frames to drop from the front of the decoded buffer.
    head: usize,
    /// Frames to drop from the back of it.
    tail: usize,
}

pub(crate) struct Decoder {
    reader: Box<dyn FormatReader>,
    decoder: Box<dyn AudioDecoder>,
    track_id: u32,
    time_base: Option<TimeBase>,
    /// The music inside the encoder's frames, in the track's timeline —
    /// used only once the rate is known to be the timeline's unit.
    window: Option<Window>,
    /// Whether the track's timestamps count frames at the rate being
    /// decoded, which is what makes [`Decoder::window`] comparable with a
    /// packet's timestamp.
    timeline: bool,
    /// The rate the device wants. Everything handed out is at this rate.
    out_rate: u32,
    /// The rate the file decodes at, known once a packet has been decoded.
    source_rate: u32,
    resampler: Option<Resampler>,
    stats: Arc<Stats>,
    /// Scratch, kept across packets so a track decodes without allocating.
    raw: Vec<f32>,
    stereo: Vec<f32>,
    chunk: Vec<f32>,
}

/// One track to open: where its bytes are, what the server said they are,
/// and where they may be kept.
///
/// Owned rather than borrowed because the track after this one is opened on
/// the runtime and moved onto a blocking task to do it (P3.4).
#[derive(Clone)]
pub(crate) struct Stream {
    /// The `stream.view` URL, credential and all.
    pub(crate) url: String,
    /// The song's id, which is what the cache keys on: the URL carries a
    /// per-request salt, and the same file must not be cached twice.
    pub(crate) id: String,
    /// How long the server says the file is, so that a copy in the cache
    /// of a file that has since been re-encoded is noticed before it is
    /// played.
    pub(crate) size: Option<u64>,
    /// What the server said the file is. symphonia takes both as a hint
    /// and still checks for itself.
    pub(crate) suffix: Option<String>,
    pub(crate) mime: Option<String>,
    /// Where bytes read from the server are kept, if the cache is on.
    pub(crate) cache: Option<Arc<Cache>>,
}

impl Decoder {
    /// Opens a stream and gets as far as a decoder for its audio track.
    ///
    /// With the cache on, a track played before opens without asking the
    /// server anything at all — which is three requests and three round
    /// trips saved before the first note.
    pub(crate) fn open(
        http: reqwest::blocking::Client,
        stream: Stream,
        out_rate: u32,
    ) -> Result<Self> {
        let stats = Arc::new(Stats::default());
        let source: Box<dyn symphonia::core::io::MediaSource> = match &stream.cache {
            Some(cache) => Box::new(
                CachedSource::new(
                    http,
                    stream.url.clone(),
                    Arc::clone(&stats),
                    cache.entry(&stream.id, stream.size),
                    &stream.id,
                )
                .context("unable to open the audio stream")?,
            ),
            None => Box::new(
                HttpSource::new(http, stream.url.clone(), Arc::clone(&stats))
                    .context("unable to open the audio stream")?,
            ),
        };
        let stream_source = MediaSourceStream::new(source, MediaSourceStreamOptions::default());

        let mut hint = Hint::new();
        if let Some(suffix) = &stream.suffix {
            hint.with_extension(suffix);
        }
        if let Some(mime) = &stream.mime {
            hint.mime_type(mime);
        }
        let reader = symphonia::default::get_probe()
            .probe(
                &hint,
                stream_source,
                FormatOptions::default(),
                MetadataOptions::default(),
            )
            .context("this file is not in a format the app can read")?;

        let track = reader
            .first_track_known_codec(TrackType::Audio)
            .ok_or_else(|| anyhow!("the file holds no audio track this build can decode"))?;
        let track_id = track.id;
        let time_base = track.time_base;
        let window = window(track);
        let params: AudioCodecParameters = track
            .codec_params
            .as_ref()
            .and_then(|params| params.audio())
            .ok_or_else(|| anyhow!("the audio track carries no codec parameters"))?
            .clone();
        let decoder = crate::opus::codecs()
            .make_audio_decoder(&params, &AudioDecoderOptions::default())
            .map_err(|error| anyhow!("this build cannot decode the file ({error})"))?;

        Ok(Self {
            reader,
            decoder,
            track_id,
            time_base,
            window,
            timeline: false,
            out_rate,
            source_rate: 0,
            resampler: None,
            stats,
            raw: Vec::new(),
            stereo: Vec::new(),
            chunk: Vec::new(),
        })
    }

    /// The name of the codec being decoded, for the log.
    pub(crate) fn codec(&self) -> &'static str {
        self.decoder.codec_info().short_name
    }

    /// How many HTTP requests this track has cost so far.
    pub(crate) fn requests(&self) -> u32 {
        self.stats.gets()
    }

    /// The next chunk of interleaved stereo at the output's rate, or `None`
    /// at the end of the track.
    ///
    /// A packet that will not decode is skipped rather than ending the
    /// track: one bad frame in the middle of a file is a glitch, not the
    /// end of the music.
    pub(crate) fn next(&mut self) -> Result<Option<&[f32]>> {
        loop {
            let packet = match self.reader.next_packet() {
                Ok(Some(packet)) => packet,
                Ok(None) => return Ok(None),
                // Some readers still signal the end of a stream this way.
                Err(SymphoniaError::IoError(error))
                    if error.kind() == std::io::ErrorKind::UnexpectedEof =>
                {
                    return Ok(None);
                }
                Err(error) => return Err(anyhow!("the audio stream stopped: {error}")),
            };
            if packet.track_id != self.track_id {
                continue;
            }
            // Read before the packet is consumed by the decoder: what
            // frames of it are music depends on all four of these.
            let timing = Timing {
                pts: packet.pts.get(),
                dur: packet.dur.get(),
                trim_start: packet.trim_start.get(),
                trim_end: packet.trim_end.get(),
            };
            // The scratch buffer is moved out for the length of the decode:
            // what comes back borrows the decoder, and the rate it reports
            // is needed before anything can be done with the samples.
            let mut raw = std::mem::take(&mut self.raw);
            let decoded = match self.decoder.decode(&packet) {
                Ok(decoded) => Some(decoded),
                Err(SymphoniaError::DecodeError(reason)) => {
                    log::warn!("skipped a packet that would not decode: {reason}");
                    None
                }
                Err(error) => {
                    self.raw = raw;
                    return Err(anyhow!("the audio stopped decoding: {error}"));
                }
            };
            let spec = match decoded {
                Some(decoded) if decoded.frames() > 0 => {
                    let spec = (decoded.spec().rate(), decoded.spec().channels().count());
                    decoded.copy_to_vec_interleaved(&mut raw);
                    Some(spec)
                }
                _ => None,
            };
            self.raw = raw;
            let Some((rate, channels)) = spec else {
                continue;
            };
            self.prepare(rate);
            // The encoder's warm-up and padding, dropped here rather than
            // played as a click at the join into the next track.
            let frames = self.raw.len().checked_div(channels).unwrap_or(0);
            let trim = trim(
                timing,
                frames,
                self.timeline.then_some(self.window).flatten(),
                rate * MAX_TRIM_SECONDS,
            );
            if trim.head + trim.tail >= frames {
                continue;
            }
            let kept = &self.raw[trim.head * channels..(frames - trim.tail) * channels];
            to_stereo(kept, channels, &mut self.stereo);
            match &mut self.resampler {
                Some(resampler) => {
                    self.chunk = resampler.process(&self.stereo);
                }
                None => std::mem::swap(&mut self.chunk, &mut self.stereo),
            }
            if self.chunk.is_empty() {
                // The resampler holds the last few frames back until the
                // next chunk arrives; there is simply nothing to play yet.
                continue;
            }
            return Ok(Some(&self.chunk));
        }
    }

    /// Builds the resampler when the rate is first known, and again if a
    /// file changes rate part way through — which is legal in Ogg and which
    /// nothing else in the chain would survive.
    fn prepare(&mut self, rate: u32) {
        if rate == self.source_rate {
            return;
        }
        if self.source_rate != 0 {
            log::info!(
                "the track changed rate from {} to {rate} Hz",
                self.source_rate
            );
        }
        self.source_rate = rate;
        // A packet's timestamp is in the track's time base, and the window
        // of music inside the encoder's frames is in frames. The two are
        // the same thing only when the time base is the reciprocal of the
        // rate being decoded — which is the ordinary case, and is checked
        // rather than assumed because a container is free to count in
        // anything (an MP4 timescale, most obviously).
        self.timeline = self
            .time_base
            .is_some_and(|base| base.numer.get() == 1 && base.denom.get() == rate);
        match self.window.filter(|_| self.timeline) {
            Some(window) => log::debug!(
                "the music is frames {}..{:?} of what the file decodes to",
                window.start,
                window.end
            ),
            None => log::debug!(
                "the track does not say which of its frames are music, so all of them play"
            ),
        }
        self.resampler = Resampler::new(rate, self.out_rate, CHANNELS);
        if self.resampler.is_some() {
            log::info!(
                "decoding at {rate} Hz for an output at {} Hz",
                self.out_rate
            );
        }
    }

    /// Jumps to a point in the track, by HTTP `Range` under the container
    /// (D12), and answers with where it actually landed — a seek lands on
    /// the packet boundary before the time asked for.
    pub(crate) fn seek(&mut self, to: Duration) -> Result<Duration> {
        let time = Time::try_from_secs_f64(to.as_secs_f64())
            .ok_or_else(|| anyhow!("{to:?} is not a time in this track"))?;
        let landed = self
            .reader
            .seek(
                SeekMode::Accurate,
                SeekTo::Time {
                    time,
                    track_id: Some(self.track_id),
                },
            )
            .map_err(|error| anyhow!("this track cannot be seeked: {error}"))?;
        self.decoder.reset();
        // The resampler carries the tail of the audio before the jump, and
        // playing that after it would be a click.
        if self.source_rate != 0 {
            self.resampler = Resampler::new(self.source_rate, self.out_rate, CHANNELS);
        }
        self.chunk.clear();
        let landed = self
            .time_base
            .and_then(|base| base.calc_time(landed.actual_ts))
            .map(time_to_duration)
            .unwrap_or(to);
        Ok(landed)
    }
}

fn time_to_duration(time: Time) -> Duration {
    Duration::from_secs_f64(time.as_secs_f64().max(0.0))
}

/// What one packet says about itself, in the track's time base.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Timing {
    /// The timestamp of the first frame the decoder will produce for it.
    pts: i64,
    /// How many of those frames are music.
    dur: u64,
    trim_start: u64,
    trim_end: u64,
}

/// The music inside the frames a track's encoder wrote.
///
/// Symphonia reports the encoder's warm-up and padding twice over, and no
/// container fills in both:
///
/// - **On the packet** (`trim_start` / `trim_end`). Ogg computes them from
///   the granule positions; the MP3 and Vorbis *decoders* then apply them
///   themselves, and no other decoder does — including `src/opus.rs`, which
///   is why the Opus fixture arrives with its 312-frame pre-skip intact.
/// - **On the track** (`delay` / `padding`), which nothing applies. MP3
///   puts the LAME delay here and nowhere else, and numbers its frames from
///   `-delay` so that the music starts at zero; Ogg Opus puts the pre-skip
///   here and starts at zero, so the music starts at `delay`.
///
/// `start_ts + delay` is the first frame of music under both conventions.
/// The end is `num_frames` later, which is exact for the containers that
/// count only playable frames (MP3, FLAC, MP4) and an over-estimate for
/// Ogg, which counts the encoder's as well — harmless, because Ogg is also
/// the one that reports its padding on the packets.
fn window(track: &symphonia::core::formats::Track) -> Option<Window> {
    let delay = i64::from(track.delay.unwrap_or(0));
    let start = track.start_ts.get().checked_add(delay)?;
    let end = track
        .num_frames
        .and_then(|frames| i64::try_from(frames).ok())
        .and_then(|frames| start.checked_add(frames));
    Some(Window { start, end })
}

/// What to drop from one decoded packet: the encoder's frames, and nothing
/// else.
///
/// `frames` is what the decoder produced, `most` the largest trim to
/// believe. A packet whose own trims have already been applied by its
/// decoder is recognised by its frame count, which is the only honest way
/// to tell: symphonia leaves the choice to each decoder and they disagree.
fn trim(packet: Timing, frames: usize, window: Option<Window>, most: u32) -> Trim {
    let mut head = 0;
    let mut tail = 0;
    let trims = packet.trim_start + packet.trim_end;
    // Where the frame at index 0 sits in the track. A decoder that trimmed
    // for us handed back the music, which starts `trim_start` in.
    let mut first = packet.pts;
    if trims > 0 && frames as u64 == packet.dur {
        first += packet.trim_start as i64;
    } else if trims > 0 {
        head = (packet.trim_start as usize).min(frames);
        tail = (packet.trim_end as usize).min(frames - head);
        first += head as i64;
    }
    let Some(window) = window else {
        return Trim { head, tail };
    };
    let held = frames.saturating_sub(head + tail) as i64;
    // How far this packet reaches outside the music, before being held to
    // the packet's own length: a warm-up the length of a second is this
    // build reading the file's timeline wrong rather than an encoder being
    // strange, and the check has to see the number it distrusts.
    let warm_up = window.start - first;
    let padding = match window.end {
        Some(end) => first + held - end,
        None => 0,
    };
    if warm_up > i64::from(most) || padding > i64::from(most) {
        log::warn!(
            "{warm_up} frames before the music and {padding} after it are not an encoder's; not trimming them"
        );
        return Trim { head, tail };
    }
    let warm_up = warm_up.clamp(0, held);
    let padding = padding.clamp(0, held - warm_up);
    Trim {
        head: head + warm_up as usize,
        tail: tail + padding as usize,
    }
}

/// Whatever the file holds, as interleaved stereo.
///
/// Mono plays on both sides. More than two channels keeps the first two,
/// which are the front pair in every layout symphonia reports: a proper
/// downmix of a surround file is a job this does not pretend to do, and a
/// surround file in a music library is rare enough to be a curiosity.
fn to_stereo(samples: &[f32], channels: usize, out: &mut Vec<f32>) {
    out.clear();
    match channels {
        0 => {}
        1 => {
            out.reserve(samples.len() * 2);
            for sample in samples {
                out.push(*sample);
                out.push(*sample);
            }
        }
        2 => out.extend_from_slice(samples),
        more => {
            let frames = samples.len() / more;
            out.reserve(frames * CHANNELS);
            for frame in 0..frames {
                out.push(samples[frame * more]);
                out.push(samples[frame * more + 1]);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::num::NonZero;

    use symphonia::core::formats::Track;
    use symphonia::core::units::Timestamp;

    /// One second of frames at 48 kHz, the cap the real decoder passes.
    const MOST: u32 = 48_000;

    fn track(delay: Option<u32>, start_ts: i64, num_frames: Option<u64>) -> Track {
        let mut track = Track::new(0);
        track.with_start_ts(Timestamp::new(start_ts));
        if let Some(delay) = delay {
            track.with_delay(delay);
        }
        if let Some(frames) = num_frames {
            track.with_num_frames(frames);
        }
        track
    }

    /// Opus in Ogg, as symphonia describes the fixture: the pre-skip is on
    /// the track and nothing applies it, the padding is on the last page's
    /// packets, and `num_frames` counts the encoder's frames as well as the
    /// music's. 8 seconds at 48 kHz, 312 frames of warm-up, 648 of padding.
    #[test]
    fn an_opus_track_loses_its_pre_skip() {
        let music = 384_000;
        let window = window(&track(Some(312), 0, Some(music + 312 + 648))).expect("a window");
        assert_eq!(
            window,
            Window {
                start: 312,
                end: Some(385_272)
            }
        );
        // The first packet: 960 frames decoded, the first 312 of them the
        // encoder warming up.
        let first = Timing {
            pts: 0,
            dur: 960,
            ..Timing::default()
        };
        assert_eq!(
            trim(first, 960, Some(window), MOST),
            Trim { head: 312, tail: 0 }
        );
        // The middle of the track is untouched.
        let middle = Timing {
            pts: 96_000,
            dur: 960,
            ..Timing::default()
        };
        assert_eq!(trim(middle, 960, Some(window), MOST), Trim::default());
        // The last packet carries its padding on the packet, and the
        // decoder did not apply it: 960 decoded, 312 of them music.
        let last = Timing {
            pts: 384_000,
            dur: 312,
            trim_start: 0,
            trim_end: 648,
        };
        assert_eq!(
            trim(last, 960, Some(window), MOST),
            Trim { head: 0, tail: 648 }
        );
    }

    /// MP3, where the LAME delay is on the track and nowhere else, and the
    /// frames are numbered from `-delay` so that the music starts at zero.
    /// Symphonia applies none of it, so a gapless album of MP3s needs this
    /// as much as an Opus one does.
    #[test]
    fn an_mp3_starts_where_its_music_starts() {
        let music = 441_000;
        let window = window(&track(Some(1_105), -1_105, Some(music))).expect("a window");
        assert_eq!(
            window,
            Window {
                start: 0,
                end: Some(441_000)
            }
        );
        let first = Timing {
            pts: -1_105,
            dur: 1_152,
            ..Timing::default()
        };
        assert_eq!(
            trim(first, 1_152, Some(window), MOST),
            Trim {
                head: 1_105,
                tail: 0
            }
        );
        // The last packet runs past the end of the music by its padding.
        let last = Timing {
            pts: 440_000,
            dur: 1_152,
            ..Timing::default()
        };
        assert_eq!(
            trim(last, 1_152, Some(window), MOST),
            Trim { head: 0, tail: 152 }
        );
    }

    /// Vorbis and MP3 packets are trimmed by their own decoders. Trimming
    /// them again here would cut the first frames of the music, so the
    /// frame count is what decides: a decoder that trimmed handed back
    /// `dur` frames, one that did not handed back all of them.
    #[test]
    fn a_decoder_that_trimmed_for_us_is_not_trimmed_twice() {
        let window = window(&track(Some(64), -64, Some(352_800))).expect("a window");
        assert_eq!(window.start, 0);
        let packet = Timing {
            pts: -64,
            dur: 960,
            trim_start: 64,
            trim_end: 0,
        };
        // 960 frames back for a packet whose music is 960 frames long: the
        // decoder dropped the warm-up already.
        assert_eq!(trim(packet, 960, Some(window), MOST), Trim::default());
        // The same packet from a decoder that did not: 1024 frames back,
        // and the first 64 are the encoder's.
        assert_eq!(
            trim(packet, 1_024, Some(window), MOST),
            Trim { head: 64, tail: 0 }
        );
    }

    /// FLAC has no encoder delay and its frame count is the music's, so
    /// every frame of every packet is kept — including the short last one.
    #[test]
    fn a_track_without_an_encoder_delay_keeps_every_frame() {
        let whole = window(&track(None, 0, Some(441_000))).expect("a window");
        for pts in [0, 220_500, 436_904] {
            let packet = Timing {
                pts,
                dur: 4_096,
                ..Timing::default()
            };
            assert_eq!(
                trim(packet, 4_096, Some(whole), MOST),
                Trim::default(),
                "the packet at {pts} lost frames"
            );
        }
        // A track that will not say how long it is keeps its tail.
        assert_eq!(window(&track(None, 0, None)).expect("a window").end, None);
    }

    /// The guard: if the sums say a second of the track is the encoder
    /// warming up, the sums are wrong about the file and a click at the
    /// join is the lesser mistake.
    #[test]
    fn a_trim_that_looks_wrong_is_not_applied() {
        let window = Window {
            start: 100_000,
            end: Some(200_000),
        };
        let packet = Timing {
            pts: 0,
            dur: 1_024,
            ..Timing::default()
        };
        assert_eq!(trim(packet, 1_024, Some(window), MOST), Trim::default());
        // And with no window at all — a container that counts time in
        // something other than frames — nothing is trimmed either.
        assert_eq!(trim(packet, 1_024, None, MOST), Trim::default());
    }

    /// A time base that is not one over the rate makes the window
    /// meaningless, which is what the decoder checks before using it.
    #[test]
    fn only_a_frame_counting_time_base_is_a_frame_count() {
        let frames = TimeBase::new(
            NonZero::new(1).expect("one"),
            NonZero::new(44_100).expect("44100"),
        );
        assert!(frames.numer.get() == 1 && frames.denom.get() == 44_100);
        let milliseconds = TimeBase::new(
            NonZero::new(1).expect("one"),
            NonZero::new(1_000).expect("1000"),
        );
        assert!(milliseconds.denom.get() != 44_100);
    }

    #[test]
    fn mono_plays_on_both_sides() {
        let mut out = Vec::new();
        to_stereo(&[0.5, -0.25], 1, &mut out);
        assert_eq!(out, vec![0.5, 0.5, -0.25, -0.25]);
    }

    #[test]
    fn stereo_passes_through() {
        let mut out = Vec::new();
        to_stereo(&[0.1, 0.2, 0.3, 0.4], 2, &mut out);
        assert_eq!(out, vec![0.1, 0.2, 0.3, 0.4]);
    }

    /// A surround file keeps its front pair rather than refusing to play.
    #[test]
    fn more_channels_keep_the_front_pair() {
        let mut out = Vec::new();
        let frames = [
            [1.0, 2.0, 3.0, 4.0, 5.0, 6.0],
            [7.0, 8.0, 9.0, 10.0, 11.0, 12.0],
        ]
        .concat();
        to_stereo(&frames, 6, &mut out);
        assert_eq!(out, vec![1.0, 2.0, 7.0, 8.0]);
    }

    /// Scratch buffers are reused between packets, so what was in them
    /// before must never leak into the next chunk.
    #[test]
    fn the_scratch_buffer_does_not_leak_between_chunks() {
        let mut out = vec![9.0; 16];
        to_stereo(&[0.1, 0.2], 2, &mut out);
        assert_eq!(out, vec![0.1, 0.2]);
    }
}
