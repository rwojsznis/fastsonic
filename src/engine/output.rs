//! The audio device, and the clock that says how much of what was fed to it
//! has actually been heard.
//!
//! The device handling itself — which device, what buffer, following the
//! system default when it moves — is `src/sink.rs`'s, shared rather than
//! copied: that code was tuned per platform (`buffer_ms`, commit `41dee71`)
//! and there should not be two of it. What is new here is that the rate is
//! no longer one number for the life of the program. The old service sent one codec
//! at 44.1 kHz; a music library has 48 kHz Opus next to 44.1 kHz FLAC, so
//! the device is opened once and the engine converts each track to its rate
//! (`src/resample.rs`).
//!
//! Failure is soft, for the reason `src/sink.rs` exists: a release build
//! aborts on panic, and a machine with no sound card must get a message in
//! the interface instead.

use std::collections::VecDeque;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use cpal::traits::DeviceTrait;

use crate::sink::{DefaultWatch, ErrorHook, open_stream, pick_device};

/// The chain is stereo, from the decoder to the device.
pub(crate) const CHANNELS: usize = 2;

/// The rate to ask the device for. Not a promise about the music: it is
/// where most of a library is, so it is the rate that converts nothing most
/// of the time. The device decides in the end, and whatever it says the
/// engine converts to.
pub(crate) const PREFERRED_RATE: u32 = 44_100;

/// How much sound the engine tries to keep in front of the device. Enough
/// that a slow read does not empty it, short enough that a pause or a seek
/// is not heard late.
pub(crate) const TARGET_QUEUE: Duration = Duration::from_millis(500);

/// Which track a chunk of sound belongs to.
///
/// Two tracks are in the sink at a gapless join (P3.4), and the interface
/// must hear about the second one when the *speaker* reaches it rather than
/// when the decoder does — half a second earlier. So everything appended
/// carries a tag, and [`Output::heard`] says which tag is coming out.
pub(crate) type Token = u64;

/// What the device is playing right now: which track, and how far into it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Heard {
    pub(crate) token: Token,
    pub(crate) position: Duration,
}

/// The device, once it is open.
struct Stream {
    sink: rodio::Sink,
    stream: rodio::OutputStream,
    /// The name of the device the stream was opened on.
    device_name: Option<String>,
    /// Set from the audio thread when the stream dies (device unplugged).
    failed: Arc<AtomicBool>,
    /// The rate the device runs at. Everything appended is at this rate.
    rate: u32,
    clock: QueueClock,
    /// Whether anything has been appended since the last reset.
    fed: bool,
    last_append: Option<Instant>,
}

impl Stream {
    fn failed(&self) -> bool {
        self.failed.load(Ordering::Relaxed)
    }
}

pub(crate) struct Output {
    /// The device name from Settings; `None` means the system default.
    device: Option<String>,
    buffer_ms: u32,
    on_error: ErrorHook,
    open: Option<Stream>,
    watch: Option<DefaultWatch>,
    /// The interface's volume, 0..=`u16::MAX`.
    volume: u16,
}

impl Output {
    pub(crate) fn new(
        device: Option<String>,
        buffer_ms: u32,
        volume: u16,
        on_error: ErrorHook,
    ) -> Self {
        Self {
            device,
            buffer_ms,
            on_error,
            open: None,
            watch: None,
            volume,
        }
    }

    /// The rate everything appended has to be at, opening the device if it
    /// is not open yet.
    pub(crate) fn rate(&mut self) -> Result<u32, String> {
        Ok(self.ensure_open()?.rate)
    }

    /// The same, for a caller that must not open a device to ask — the
    /// engine reads it every time round its loop so the chain's filters
    /// follow the device when the system default moves to one running at
    /// another rate. `None` while nothing is open, when there is no rate to
    /// follow.
    pub(crate) fn current_rate(&self) -> Option<u32> {
        self.open.as_ref().map(|stream| stream.rate)
    }

    /// Opens the device if it is not open, or if it died since.
    fn ensure_open(&mut self) -> Result<&mut Stream, String> {
        self.follow_default();
        if self.open.as_ref().is_some_and(Stream::failed) {
            log::warn!("the audio output stopped working; reopening it");
            self.open = None;
        }
        if self.open.is_none() {
            let volume = attenuation(self.volume);
            match open_device(self.device.as_deref(), self.buffer_ms, volume) {
                Ok(stream) => self.open = Some(stream),
                Err(error) => {
                    let message = error.to_string();
                    log::error!("{message}");
                    (self.on_error)(message.clone());
                    return Err(message);
                }
            }
        }
        Ok(self.open.as_mut().expect("just opened"))
    }

    /// Moves playback when the system's default output changes, as long as
    /// Settings did not name a device.
    ///
    /// Windows and macOS need explicit polling; PipeWire and PulseAudio move
    /// streams themselves, and ALSA's answer does not change.
    fn follow_default(&mut self) {
        if cfg!(target_os = "linux") || self.device.is_some() {
            return;
        }
        let Some(open) = &self.open else {
            return;
        };
        let watch = self.watch.get_or_insert_with(DefaultWatch::start);
        let current = watch.name();
        if current.is_some() && current != open.device_name {
            log::info!(
                "the default audio output is now {}; moving playback to it",
                current.as_deref().unwrap_or("[unknown device]")
            );
            self.open = None;
        }
    }

    /// Hands one decoded chunk to the device and answers with how long it
    /// is. `samples` are interleaved stereo at [`Output::rate`]; `token`
    /// says which track they belong to and `from` where in that track they
    /// start, which together are what [`Output::heard`] reports back.
    pub(crate) fn append(
        &mut self,
        samples: &[f32],
        token: Token,
        from: Duration,
    ) -> Result<Duration, String> {
        if samples.is_empty() {
            return Ok(Duration::ZERO);
        }
        let stream = self.ensure_open()?;
        let frames = samples.len() / CHANNELS;
        let now = Instant::now();
        if stream.fed && stream.sink.empty() && !stream.sink.is_paused() {
            let late = stream
                .last_append
                .map(|last| now.duration_since(last).as_millis())
                .unwrap_or(0);
            log::warn!("audio queue ran dry; the next chunk arrived after {late} ms");
        }
        stream.sink.append(rodio::buffer::SamplesBuffer::new(
            CHANNELS as rodio::ChannelCount,
            stream.rate as rodio::SampleRate,
            samples,
        ));
        let dur = Duration::from_secs_f64(frames as f64 / f64::from(stream.rate));
        stream.clock.push(Chunk { dur, token, from });
        stream.fed = true;
        stream.last_append = Some(now);
        Ok(dur)
    }

    /// How much sound is waiting to be heard.
    pub(crate) fn queued(&mut self) -> Duration {
        match &mut self.open {
            Some(stream) => stream.clock.queued(stream.sink.len()),
            None => Duration::ZERO,
        }
    }

    /// The same, to the frame the device is actually on rather than to the
    /// front of the chunk it is in. This is how far in front of the music
    /// everything upstream is — what the visualisers have to look back
    /// through so that they move with the speaker (P3.8).
    pub(crate) fn ahead(&mut self) -> Duration {
        match &mut self.open {
            Some(stream) => stream.clock.ahead(stream.sink.len(), stream.sink.get_pos()),
            None => Duration::ZERO,
        }
    }

    /// What the device is playing: the track the sound belongs to and how
    /// far into it the speaker has got. It counts what the device has
    /// taken, not what the decoder has produced, so it neither runs half a
    /// second ahead of the music nor announces the next track early.
    ///
    /// `None` before anything has been appended since the last
    /// [`Output::restart`].
    pub(crate) fn heard(&mut self) -> Option<Heard> {
        let stream = self.open.as_mut()?;
        stream.clock.heard(stream.sink.len(), stream.sink.get_pos())
    }

    /// Whether everything appended has been played.
    pub(crate) fn drained(&mut self) -> bool {
        self.open.as_ref().is_none_or(|stream| stream.sink.empty())
    }

    pub(crate) fn play(&mut self) {
        if let Some(stream) = &self.open {
            stream.sink.play();
        }
    }

    pub(crate) fn pause(&mut self) {
        if let Some(stream) = &self.open {
            stream.sink.pause();
        }
    }

    /// Throws away what is queued and starts the clock again, for a seek or
    /// a new track.
    ///
    /// It replaces the sink rather than calling `rodio::Sink::clear`, which
    /// waits for the queue to empty: nothing on the audio thread may wait on
    /// the device, and a fresh sink on the same stream costs a channel.
    pub(crate) fn restart(&mut self, playing: bool) {
        let Some(stream) = &mut self.open else {
            return;
        };
        stream.sink = rodio::Sink::connect_new(stream.stream.mixer());
        stream.sink.set_volume(attenuation(self.volume));
        if !playing {
            stream.sink.pause();
        }
        stream.clock.reset();
        stream.fed = false;
        stream.last_append = None;
    }

    /// The volume applies at the output, so a change is heard at once
    /// rather than after the half second already queued.
    pub(crate) fn set_volume(&mut self, volume: u16) {
        self.volume = volume;
        if let Some(stream) = &self.open {
            stream.sink.set_volume(attenuation(volume));
        }
    }
}

/// The interface's 0..=`u16::MAX` volume as a gain.
///
/// This is librespot's `Cubic(60 dB)` curve, the one the slider was drawn
/// against: half way is about -16 dB and three quarters about -7 dB, so the
/// useful range is spread across the slider instead of crowded into its top
/// quarter. Keeping the same curve keeps the same slider.
pub(crate) fn attenuation(volume: u16) -> f32 {
    // Not only an optimisation: the curve does not reach either end by
    // itself, and zero has to be silence.
    if volume == 0 {
        return 0.0;
    }
    if volume == u16::MAX {
        return 1.0;
    }
    // 10^(-60/60), the cubic voltage-to-decibel ratio at a 60 dB range.
    const MIN_NORM: f32 = 0.1;
    let normalized = f32::from(volume) / f32::from(u16::MAX);
    (normalized * (1.0 - MIN_NORM) + MIN_NORM).powi(3)
}

/// One chunk handed to the device, and where in its track it came from.
#[derive(Clone, Copy, Debug)]
struct Chunk {
    dur: Duration,
    token: Token,
    /// The position in the track of this chunk's first frame.
    from: Duration,
}

/// What has been fed to the device, and what of it is being heard.
///
/// rodio hands back two numbers: how many chunks are still queued, and how
/// far into the one it is playing it has got. Everything appended is
/// remembered until it has been played, so the chunk at the front of this
/// list is the one at the speaker — which is what makes this a position
/// that follows the device's own clock rather than the wall clock, and what
/// lets a gapless join be noticed when it is heard rather than when it is
/// decoded.
#[derive(Debug, Default)]
struct QueueClock {
    /// The chunks appended, oldest first, trimmed to what is still queued.
    chunks: VecDeque<Chunk>,
    /// The last chunk pushed, which is what is being heard once the queue
    /// has drained: everything in it has been played to the end.
    last: Option<Chunk>,
    /// The last answer, so a position never goes backwards over the race
    /// between reading the queue length and reading the position in it.
    heard: Option<Heard>,
}

impl QueueClock {
    fn push(&mut self, chunk: Chunk) {
        self.chunks.push_back(chunk);
        self.last = Some(chunk);
    }

    fn queued(&mut self, still_queued: usize) -> Duration {
        while self.chunks.len() > still_queued {
            self.chunks.pop_front();
        }
        self.chunks.iter().map(|chunk| chunk.dur).sum()
    }

    /// What is queued, less the part of the chunk at the speaker that has
    /// already been played.
    fn ahead(&mut self, still_queued: usize, into_current: Duration) -> Duration {
        let queued = self.queued(still_queued);
        let played = match self.chunks.front() {
            Some(chunk) => into_current.min(chunk.dur),
            None => Duration::ZERO,
        };
        queued.saturating_sub(played)
    }

    /// `into_current` is rodio's position within the chunk being played,
    /// which is what gives this better resolution than one chunk.
    fn heard(&mut self, still_queued: usize, into_current: Duration) -> Option<Heard> {
        self.queued(still_queued);
        let (chunk, into) = match self.chunks.front() {
            Some(chunk) => (*chunk, into_current.min(chunk.dur)),
            // Nothing queued: the last chunk appended has been played to
            // its end.
            None => {
                let last = self.last?;
                (last, last.dur)
            }
        };
        let position = chunk.from + into;
        let heard = match self.heard {
            // Only within one track. Across a join the position starts
            // again from the top of the next one, which is not a position
            // going backwards but a different track's position.
            Some(previous) if previous.token == chunk.token => Heard {
                token: chunk.token,
                position: position.max(previous.position),
            },
            _ => Heard {
                token: chunk.token,
                position,
            },
        };
        self.heard = Some(heard);
        Some(heard)
    }

    fn reset(&mut self) {
        self.chunks.clear();
        self.last = None;
        self.heard = None;
    }
}

fn open_device(
    preferred: Option<&str>,
    buffer_ms: u32,
    volume: f32,
) -> Result<Stream, crate::sink::OpenError> {
    let device = pick_device(preferred)?;
    let device_name = device.name().ok();
    log::info!(
        "audio output: {}",
        device_name.as_deref().unwrap_or("[unknown device]")
    );

    let failed = Arc::new(AtomicBool::new(false));
    let flag = Arc::clone(&failed);
    let on_error = move |error: cpal::StreamError| {
        log::error!("audio stream error: {error}");
        flag.store(true, Ordering::Relaxed);
    };
    let mut stream = open_stream(
        &device,
        on_error,
        buffer_ms,
        CHANNELS as u16,
        PREFERRED_RATE,
    )?;
    stream.log_on_drop(false);
    let rate = stream.config().sample_rate();
    log::info!("the audio output runs at {rate} Hz");
    let sink = rodio::Sink::connect_new(stream.mixer());
    sink.set_volume(volume);
    Ok(Stream {
        sink,
        stream,
        device_name,
        failed,
        rate,
        clock: QueueClock::default(),
        fed: false,
        last_append: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ms(millis: u64) -> Duration {
        Duration::from_millis(millis)
    }

    fn heard(token: Token, position: Duration) -> Heard {
        Heard { token, position }
    }

    /// One track's chunks, back to back from the start of it.
    fn feed(clock: &mut QueueClock, token: Token, from: Duration, chunks: usize, each: Duration) {
        for chunk in 0..chunks {
            clock.push(Chunk {
                dur: each,
                token,
                from: from + each * chunk as u32,
            });
        }
    }

    /// The clock counts what the device has taken, not what the decoder has
    /// produced: five chunks appended and three still queued means two have
    /// been played.
    #[test]
    fn the_clock_counts_what_has_been_heard() {
        let mut clock = QueueClock::default();
        feed(&mut clock, 1, Duration::ZERO, 5, ms(100));
        assert_eq!(clock.heard(5, Duration::ZERO), Some(heard(1, ms(0))));
        assert_eq!(clock.heard(3, Duration::ZERO), Some(heard(1, ms(200))));
        assert_eq!(clock.queued(3), ms(300));
        // Part way into the chunk being played, which is what keeps the
        // position smooth between chunk boundaries.
        assert_eq!(clock.heard(3, ms(40)), Some(heard(1, ms(240))));
        assert_eq!(clock.heard(0, Duration::ZERO), Some(heard(1, ms(500))));
    }

    /// What is in front of the speaker is what is queued less what it has
    /// already played of the chunk it is on — the number the visualisers
    /// look back through, and the one place a whole chunk of error would
    /// be seen as the bars running ahead of the music.
    #[test]
    fn what_is_ahead_of_the_speaker_counts_the_chunk_it_is_in() {
        let mut clock = QueueClock::default();
        feed(&mut clock, 1, Duration::ZERO, 5, ms(100));
        assert_eq!(clock.ahead(5, Duration::ZERO), ms(500));
        assert_eq!(clock.ahead(5, ms(60)), ms(440));
        assert_eq!(clock.ahead(3, ms(60)), ms(240));
        // Past the end of its chunk, the same race `heard` guards against.
        assert_eq!(clock.ahead(3, ms(400)), ms(200));
        assert_eq!(clock.ahead(0, Duration::ZERO), Duration::ZERO);
    }

    /// Nothing has been appended, so there is nothing to say — as opposed
    /// to "the position is zero", which would make a seek report the top of
    /// the track until the first chunk after it landed.
    #[test]
    fn an_empty_clock_reports_nothing() {
        let mut clock = QueueClock::default();
        assert_eq!(clock.heard(0, Duration::ZERO), None);
    }

    /// A gapless join: the second track's sound is in the sink behind the
    /// first track's, and what is *heard* is whichever chunk the device has
    /// reached. That is how the interface learns about a new track when the
    /// speaker does rather than when the decoder does.
    #[test]
    fn a_join_is_heard_when_the_device_reaches_it() {
        let mut clock = QueueClock::default();
        feed(&mut clock, 1, ms(9_000), 2, ms(100));
        feed(&mut clock, 2, Duration::ZERO, 3, ms(100));
        // Still in the first track, near its end.
        assert_eq!(clock.heard(5, ms(50)), Some(heard(1, ms(9_050))));
        assert_eq!(clock.heard(4, Duration::ZERO), Some(heard(1, ms(9_100))));
        // The device has taken both of the first track's chunks: what is
        // playing is the second track, from the top.
        assert_eq!(clock.heard(3, ms(20)), Some(heard(2, ms(20))));
        assert_eq!(clock.heard(0, Duration::ZERO), Some(heard(2, ms(300))));
    }

    /// rodio's queue length and its position inside the current chunk are
    /// read one after the other, so a chunk can end between the two. The
    /// position must not jump backwards when that happens.
    #[test]
    fn a_position_never_goes_backwards() {
        let mut clock = QueueClock::default();
        feed(&mut clock, 1, Duration::ZERO, 2, ms(100));
        assert_eq!(clock.heard(1, ms(90)), Some(heard(1, ms(190))));
        // The stale reading: one fewer chunk queued, but a position from
        // the chunk before it.
        assert_eq!(clock.heard(1, ms(0)), Some(heard(1, ms(190))));
        clock.reset();
        assert_eq!(clock.heard(0, Duration::ZERO), None);
    }

    /// A position inside the current chunk that is longer than the chunk is
    /// the same race the other way around.
    #[test]
    fn a_position_past_the_end_of_its_chunk_is_held_to_it() {
        let mut clock = QueueClock::default();
        feed(&mut clock, 1, Duration::ZERO, 2, ms(100));
        assert_eq!(clock.heard(2, ms(400)), Some(heard(1, ms(100))));
    }

    /// The curve librespot's slider was drawn against: silent at the
    /// bottom, unity at the top, and about -16 dB half way up.
    #[test]
    fn the_volume_curve_matches_the_slider() {
        assert_eq!(attenuation(0), 0.0);
        assert_eq!(attenuation(u16::MAX), 1.0);
        let half = attenuation(u16::MAX / 2);
        let half_db = 20.0 * half.log10();
        assert!(
            (half_db + 16.0).abs() < 0.5,
            "half way up is {half_db} dB, not about -16 dB"
        );
        let three_quarters = 20.0 * attenuation(u16::MAX / 4 * 3).log10();
        assert!(
            (three_quarters + 7.0).abs() < 0.5,
            "three quarters up is {three_quarters} dB, not about -7 dB"
        );
        // And it only ever goes up.
        let mut last = 0.0;
        for step in 0..=64 {
            let gain = attenuation((u32::from(u16::MAX) * step / 64) as u16);
            assert!(gain >= last, "the curve dips at step {step}");
            last = gain;
        }
    }
}
