//! The rest of the signal chain: the equalizer, the visualiser tap and the
//! limiter, between the decoder and the device.
//!
//! ```text
//!   decode.rs -> equalizer -> the tap -> ReplayGain -> limiter -> output.rs
//!               (src/eq.rs) (src/vis.rs)             (src/limiter.rs) (volume, device)
//! ```
//!
//! Where each stage sits is a promise the interface is built on, not a
//! convenience (`AGENTS.md`): **every visualiser shows post-equalizer,
//! pre-volume audio.** Moving a slider moves the bars; turning the volume
//! down to nothing leaves them dancing. That is why the tap is between the
//! equalizer and the volume, and the volume is the device's — `output.rs`
//! sets it on the sink, so a change is heard at once rather than after the
//! half second already queued.
//!
//! ReplayGain is after the tap for the same reason the volume is: it is
//! loudness housekeeping rather than music, and the picture should not move
//! because one album was mastered louder than the next (P3.7). It is before
//! the limiter because it is a gain like any other and the limiter is the
//! one ceiling in the chain.
//!
//! The limiter is last because it is the only stage that needs to know the
//! volume. The equalizer's boost is deliberately not held back where it is
//! applied: +12 dB clips at full volume and is perfectly fine three notches
//! down, so the ceiling is worked out here from where the volume is now.
//!
//! The stages work in `f64` because the equalizer's biquads do — a
//! second-order section run at `f32` accumulates its own noise — so this is
//! where the decoder's `f32` is widened and narrowed again.

use std::sync::Arc;

use crate::eq::SharedEq;
use crate::limiter::Limiter;
use crate::vis::AudioTap;

use super::output::attenuation;

pub(super) struct Chain {
    eq: crate::eq::Processor,
    tap: Arc<AudioTap>,
    limiter: Limiter,
    /// The device's rate. Both the filters and the limiter's timings are
    /// designed for it, so both are rebuilt when it changes — which is what
    /// following the system default onto another device does.
    rate: u32,
    /// Scratch, kept across chunks so a track plays without allocating.
    work: Vec<f64>,
    out: Vec<f32>,
}

impl Chain {
    pub(super) fn new(eq: SharedEq, tap: Arc<AudioTap>, rate: u32) -> Self {
        Self {
            eq: crate::eq::Processor::new(eq, rate),
            tap,
            limiter: Limiter::new(f64::from(rate)),
            rate,
            work: Vec::new(),
            out: Vec::new(),
        }
    }

    /// Follows the device onto a new rate.
    pub(super) fn set_rate(&mut self, rate: u32) {
        if rate == self.rate {
            return;
        }
        log::info!("the chain now runs at {rate} Hz");
        self.rate = rate;
        self.eq.set_rate(rate);
        self.limiter = Limiter::new(f64::from(rate));
    }

    /// One chunk of interleaved stereo, shaped and ready for the device.
    ///
    /// `volume` is what the sink will apply *after* this, which is what
    /// decides the ceiling: at a quarter of the way up, a sample at four
    /// still comes out at one. `gain` is the track's ReplayGain, already
    /// worked out by [`replay_gain`] when the track was opened.
    pub(super) fn process(&mut self, samples: &[f32], volume: u16, gain: f32) -> &[f32] {
        self.work.clear();
        self.work
            .extend(samples.iter().map(|sample| f64::from(*sample)));
        self.eq.process(&mut self.work);
        // Post-equalizer, pre-volume, pre-ReplayGain: the sliders move the
        // bars, and neither the volume knob nor the loudness housekeeping
        // does. The gain argument is what the tap is told to undo, and
        // nothing has been applied yet, so it is one.
        self.tap.push(&self.work, 1.0);
        if gain != 1.0 {
            let gain = f64::from(gain);
            for sample in &mut self.work {
                *sample *= gain;
            }
        }
        if let Some(full_scale) = full_scale(f64::from(attenuation(volume))) {
            self.limiter.process(&mut self.work, full_scale);
        }
        self.out.clear();
        self.out
            .extend(self.work.iter().map(|sample| *sample as f32));
        &self.out
    }

    /// Everything queued has been thrown away — a seek, or a track the
    /// interface changed its mind about. What the tap holds would never
    /// have been heard, so drawing it would be showing sound that no longer
    /// exists.
    pub(super) fn clear(&mut self) {
        self.tap.clear();
    }

    /// How much of what has been pushed to the tap is still waiting at the
    /// device. Everything reading the tap looks that far back, so the
    /// visualisers move with the speaker rather than with the decoder, which
    /// runs up to half a second in front of it.
    pub(super) fn set_lead(&self, ahead: std::time::Duration) {
        self.tap
            .set_lead((ahead.as_secs_f64() * f64::from(self.rate)) as usize);
    }
}

/// How far a track's own ReplayGain may move it. A tag outside this is a
/// broken tag rather than a very quiet recording, and a broken tag must not
/// be able to make the machine shout.
const GAIN_LIMIT_DB: f64 = 24.0;

/// The gain to play a song at, from what the server says about it.
///
/// `album` asks for the album's gain rather than the song's — the two
/// differ by exactly the thing ReplayGain is for: album gain keeps a record
/// sounding like a record, with its quiet track still quiet, and track gain
/// makes every song the same loudness, which is what a shuffle wants. The
/// choice is the context's, and the numbers are the song's own either way.
///
/// The peak holds the gain down where raising it would clip, which is the
/// ordinary "prevent clipping" behaviour: without it a quiet-sounding track
/// that peaks near full scale would spend its whole length under the
/// limiter, which is a worse trade than being a decibel quieter than it
/// asked to be.
///
/// `baseGain` is deliberately not added. It is the gain a decoder has
/// already applied — an Ogg Opus output gain — and `src/opus.rs` applies it
/// because it belongs to the file; adding it here would apply it twice.
pub(super) fn replay_gain(song: &crate::api::subsonic::Child, album: bool) -> f32 {
    let Some(values) = song.replay_gain else {
        return 1.0;
    };
    let (wanted, other) = if album {
        (values.album_gain, values.track_gain)
    } else {
        (values.track_gain, values.album_gain)
    };
    let Some(db) = wanted.or(other).or(values.fallback_gain) else {
        return 1.0;
    };
    let gain = 10f64.powf(db.clamp(-GAIN_LIMIT_DB, GAIN_LIMIT_DB) / 20.0);
    let peak = if album {
        values.album_peak.or(values.track_peak)
    } else {
        values.track_peak.or(values.album_peak)
    };
    match peak {
        Some(peak) if peak > 0.0 => gain.min(1.0 / peak) as f32,
        _ => gain as f32,
    }
}

/// The level that becomes full scale once the sink has applied `volume`.
///
/// `None` at silence: no finite ceiling before a multiplication by zero
/// means anything, and there is nothing to hear either way.
fn full_scale(volume: f64) -> Option<f64> {
    (volume > f64::EPSILON).then(|| 1.0 / volume)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tone(hz: f64, frames: usize, rate: u32) -> Vec<f32> {
        (0..frames)
            .flat_map(|i| {
                let t = i as f64 / f64::from(rate);
                let sample = (0.25 * (std::f64::consts::TAU * hz * t).sin()) as f32;
                [sample, sample]
            })
            .collect()
    }

    fn peak(samples: &[f32]) -> f32 {
        samples
            .iter()
            .fold(0.0f32, |loudest, s| loudest.max(s.abs()))
    }

    fn rms(samples: &[f32]) -> f32 {
        (samples.iter().map(|s| s * s).sum::<f32>() / samples.len() as f32).sqrt()
    }

    /// The whole point of P3.8: what the visualisers read is what the
    /// equalizer made, at the level it made it, whatever the volume knob is
    /// doing. Winamp's bars dance with the volume at zero.
    #[test]
    fn the_tap_sees_the_music_and_not_the_volume() {
        let tap = AudioTap::new();
        let mut chain = Chain::new(crate::eq::shared(), Arc::clone(&tap), 44_100);
        let input = tone(440.0, 4096, 44_100);
        for volume in [u16::MAX, u16::MAX / 4, 0] {
            tap.clear();
            chain.process(&input, volume, 1.0);
            let tapped = tap.window(2048, 0);
            let level = 20.0 * (rms(&tapped) / rms(&input)).log10();
            assert!(
                level.abs() < 0.5,
                "at volume {volume} the tap is {level:.1} dB off the music"
            );
        }
    }

    /// A boosted band reaches the tap, because the tap is after the
    /// equalizer. This is the check that would fail if the stages were
    /// reordered.
    #[test]
    fn the_tap_is_after_the_equalizer() {
        let settings = crate::eq::shared();
        {
            let mut shared = settings.lock().unwrap();
            shared.on = true;
            shared.bands_db[4] = 12.0; // 1 kHz
        }
        let tap = AudioTap::new();
        let mut chain = Chain::new(settings, Arc::clone(&tap), 44_100);
        let input = tone(1000.0, 8192, 44_100);
        chain.process(&input, u16::MAX / 2, 1.0);
        // The second half, past the filters' own warm-up.
        let tapped = tap.window(4096, 0);
        let gained = 20.0 * (rms(&tapped) / rms(&input[8192..])).log10();
        assert!(
            (gained - 12.0).abs() < 1.0,
            "the boosted band reached the tap {gained:.1} dB up, not 12"
        );
    }

    /// A boost that would clip is held to the ceiling — and the ceiling is
    /// where the volume puts it, so the same boost passes untouched three
    /// notches down. The equalizer refuses to clamp for exactly this
    /// reason; if the limiter were not here, it would clip instead.
    #[test]
    fn the_ceiling_follows_the_volume() {
        let settings = crate::eq::shared();
        {
            let mut shared = settings.lock().unwrap();
            shared.on = true;
            shared.preamp_db = 12.0;
        }
        let tap = AudioTap::new();
        let loud = tone(440.0, 44_100, 44_100);

        let mut chain = Chain::new(settings.clone(), Arc::clone(&tap), 44_100);
        let mut at_full = Vec::new();
        for block in loud.chunks(4096) {
            at_full.extend_from_slice(chain.process(block, u16::MAX, 1.0));
        }
        // A quarter of full scale boosted by 12 dB is about 1.0, and the
        // limiter's threshold is a decibel below that.
        assert!(
            peak(&at_full[22_050..]) < 1.0,
            "at full volume the peak is {}",
            peak(&at_full[22_050..])
        );

        let mut chain = Chain::new(settings, tap, 44_100);
        let mut turned_down = Vec::new();
        for block in loud.chunks(4096) {
            turned_down.extend_from_slice(chain.process(block, u16::MAX / 2, 1.0));
        }
        let quiet_peak = peak(&turned_down[22_050..]);
        let full_peak = peak(&at_full[22_050..]);
        assert!(
            quiet_peak > full_peak + 0.01,
            "turned down, the same boost peaks at {quiet_peak} against {full_peak}: \
             the ceiling did not move with the volume"
        );
    }

    fn song(gain: crate::api::subsonic::types::ReplayGain) -> crate::api::subsonic::Child {
        crate::api::subsonic::Child {
            replay_gain: Some(gain),
            ..crate::api::subsonic::Child::default()
        }
    }

    /// Which of the two numbers is used is a question about the context:
    /// an album keeps its own quiet track quiet, and anything else evens
    /// every song out. The numbers are the song's own either way.
    #[test]
    fn album_gain_plays_an_album_and_track_gain_plays_a_shuffle() {
        let song = song(crate::api::subsonic::types::ReplayGain {
            track_gain: Some(-6.0),
            album_gain: Some(-3.0),
            ..Default::default()
        });
        let track = 20.0 * f64::from(replay_gain(&song, false)).log10();
        let album = 20.0 * f64::from(replay_gain(&song, true)).log10();
        assert!((track + 6.0).abs() < 0.01, "track gain came out {track}");
        assert!((album + 3.0).abs() < 0.01, "album gain came out {album}");
    }

    /// A song with only one of the two, or with neither and a fallback, or
    /// with nothing at all. A library is full of all four cases.
    #[test]
    fn a_missing_gain_falls_back_and_then_gives_up() {
        use crate::api::subsonic::types::ReplayGain;
        let only_track = song(ReplayGain {
            track_gain: Some(-6.0),
            ..Default::default()
        });
        let db = 20.0 * f64::from(replay_gain(&only_track, true)).log10();
        assert!((db + 6.0).abs() < 0.01, "the album fell back to {db}");
        let fallback = song(ReplayGain {
            fallback_gain: Some(-8.0),
            ..Default::default()
        });
        let db = 20.0 * f64::from(replay_gain(&fallback, false)).log10();
        assert!((db + 8.0).abs() < 0.01, "the fallback came out {db}");
        assert_eq!(replay_gain(&song(ReplayGain::default()), false), 1.0);
        assert_eq!(
            replay_gain(&crate::api::subsonic::Child::default(), false),
            1.0
        );
    }

    /// A quiet-sounding track that peaks near full scale asks for a boost
    /// it cannot have. Giving it would put the track under the limiter for
    /// its whole length, which is a worse trade than a decibel of loudness.
    #[test]
    fn the_peak_holds_a_boost_down() {
        use crate::api::subsonic::types::ReplayGain;
        let peaky = song(ReplayGain {
            track_gain: Some(6.0),
            track_peak: Some(0.95),
            ..Default::default()
        });
        let gain = replay_gain(&peaky, false);
        assert!(
            (f64::from(gain) - 1.0 / 0.95).abs() < 0.001,
            "the boost was not held to the peak: {gain}"
        );
        // An attenuation is never held back by a peak; there is nothing to
        // clip.
        let quiet = song(ReplayGain {
            track_gain: Some(-6.0),
            track_peak: Some(0.1),
            ..Default::default()
        });
        assert!(replay_gain(&quiet, false) < 0.51);
    }

    /// A tag that says a track is 90 dB quiet is a broken tag, and a broken
    /// tag must not be able to make the machine shout.
    #[test]
    fn an_impossible_tag_is_held_to_something_survivable() {
        use crate::api::subsonic::types::ReplayGain;
        let broken = song(ReplayGain {
            track_gain: Some(90.0),
            ..Default::default()
        });
        let db = 20.0 * f64::from(replay_gain(&broken, false)).log10();
        assert!((db - GAIN_LIMIT_DB).abs() < 0.01, "it came out at {db} dB");
    }

    /// ReplayGain is loudness housekeeping, not music: it moves what comes
    /// out of the speaker and leaves the picture where it was, exactly as
    /// the volume knob does.
    #[test]
    fn replay_gain_is_behind_the_tap() {
        let tap = AudioTap::new();
        let mut chain = Chain::new(crate::eq::shared(), Arc::clone(&tap), 44_100);
        let input = tone(440.0, 8192, 44_100);
        let out = chain.process(&input, u16::MAX / 2, 0.5).to_vec();
        let tapped = tap.window(4096, 0);
        let seen = 20.0 * (rms(&tapped) / rms(&input[8192..])).log10();
        assert!(seen.abs() < 0.5, "the tap moved by {seen:.1} dB");
        let heard = 20.0 * (rms(&out[8192..]) / rms(&input[8192..])).log10();
        assert!(
            (heard + 6.02).abs() < 0.5,
            "what is played moved by {heard:.1} dB, not -6"
        );
    }

    /// The lead is what keeps the bars with the speaker rather than with
    /// the decoder: a reader asking for the newest samples gets the ones
    /// from before what is still queued.
    #[test]
    fn the_lead_moves_the_window_back_to_the_speaker() {
        let tap = AudioTap::new();
        let chain = Chain::new(crate::eq::shared(), Arc::clone(&tap), 48_000);
        chain.set_lead(std::time::Duration::from_millis(500));
        // Half a second at 48 kHz.
        let mut marked = vec![0.0f64; 2 * 24_000];
        marked.extend([1.0; 2 * 100]);
        tap.push(&marked, 1.0);
        // The last hundred frames are still in the sink, so what is heard
        // is the silence before them.
        assert_eq!(tap.window(4, 0), [0.0; 4]);
        chain.set_lead(std::time::Duration::ZERO);
        assert_eq!(tap.window(4, 0), [1.0; 4]);
    }
}
