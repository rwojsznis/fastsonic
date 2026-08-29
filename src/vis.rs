//! The visualiser's audio: a tap on the samples on their way out, and the
//! sums that turn them into Winamp's bars and scope.
//!
//! The tap wraps whichever sink plays the music (this crate's, or one of
//! librespot's) and keeps the last half second of it, mixed to mono. The
//! analyser is a port of Winamp's classic one as Webamp reconstructed it
//! (`VisPainter.ts` and `FFTNullsoft.ts`): Nullsoft's own FFT with its sine
//! envelope and log equalisation, 75 columns spread over the spectrum on a
//! mostly logarithmic sweep and grouped four at a time into 19 bars, bars
//! that fall at a fixed rate, and peaks that hang, then drop faster and
//! faster.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use librespot_playback::audio_backend::{Sink, SinkResult};
use librespot_playback::convert::Converter;
use librespot_playback::decoder::AudioPacket;
use librespot_playback::mixer::VolumeGetter;
use librespot_playback::{NUM_CHANNELS, SAMPLE_RATE};

/// How much sound the tap keeps: half a second.
const KEPT: usize = SAMPLE_RATE as usize / 2;
/// How far behind the newest sample the visualiser looks, so that it shows
/// what the speaker is playing rather than what the sink has queued.
pub const LAG: usize = SAMPLE_RATE as usize * 3 / 20;
/// Samples that go into one spectrum.
pub const FFT_SAMPLES: usize = 1024;
const SPECTRUM_BINS: usize = FFT_SAMPLES / 2;
/// Samples the scope reads, one column every seventh.
pub const SCOPE_SAMPLES: usize = 576;
/// The visualiser's width and height in skin pixels.
pub const COLUMNS: usize = 75;
pub const ROWS: u8 = 16;
/// The bars, each three columns wide with one between.
pub const BARS: usize = 19;
/// The tallest a bar gets.
const MAX_HEIGHT: f32 = 15.0;
/// How far a bar falls each step, and how peaks pick up speed.
const FALLOFF: f32 = 12.0 / 16.0;
const PEAK_FALLOFF: f32 = 1.1;
/// Winamp's byte-scaled input: a full-scale sample is 128 in 24ths.
const INPUT_GAIN: f32 = 128.0 / 24.0;

/// The last half second of sound, shared between the player's thread and
/// the window.
pub struct AudioTap {
    samples: Mutex<VecDeque<f32>>,
}

impl std::fmt::Debug for AudioTap {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("AudioTap")
    }
}

impl Default for AudioTap {
    fn default() -> Self {
        Self {
            samples: Mutex::new(VecDeque::with_capacity(KEPT)),
        }
    }
}

impl AudioTap {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    /// Takes interleaved stereo samples, mixed down and scaled by `gain`.
    pub fn push(&self, interleaved: &[f64], gain: f32) {
        let mut samples = self.samples.lock().unwrap_or_else(|p| p.into_inner());
        let (frames, _) = interleaved.as_chunks::<{ NUM_CHANNELS as usize }>();
        for frame in frames {
            let mono = frame.iter().sum::<f64>() as f32 / frame.len() as f32 * gain;
            if samples.len() == KEPT {
                samples.pop_front();
            }
            samples.push_back(mono);
        }
    }

    /// The `count` samples ending `lag` samples before the newest, with
    /// silence where there is less than that.
    pub fn window(&self, count: usize, lag: usize) -> Vec<f32> {
        let samples = self.samples.lock().unwrap_or_else(|p| p.into_inner());
        let end = samples.len().saturating_sub(lag);
        let start = end.saturating_sub(count);
        let mut out = vec![0.0; count];
        let taken = end - start;
        for (slot, sample) in out[count - taken..]
            .iter_mut()
            .zip(samples.range(start..end))
        {
            *slot = *sample;
        }
        out
    }

    pub fn clear(&self) {
        self.samples
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .clear();
    }
}

/// A sink that runs the equalizer over every sample and hands the tap
/// the result on its way to the real one, so the bars show what is heard.
pub struct Tapped {
    inner: Box<dyn Sink>,
    tap: Arc<AudioTap>,
    eq: crate::eq::Processor,
    /// Undoes the volume the player already applied to the samples, so the
    /// bars show the music, not the volume knob. `None` when the sink
    /// applies the volume itself, after the tap.
    applied_volume: Option<Box<dyn VolumeGetter + Send>>,
}

impl Tapped {
    pub fn new(
        inner: Box<dyn Sink>,
        tap: Arc<AudioTap>,
        applied_volume: Option<Box<dyn VolumeGetter + Send>>,
        eq: crate::eq::SharedEq,
    ) -> Self {
        Self {
            inner,
            tap,
            eq: crate::eq::Processor::new(eq),
            applied_volume,
        }
    }
}

impl Sink for Tapped {
    fn start(&mut self) -> SinkResult<()> {
        self.inner.start()
    }

    fn stop(&mut self) -> SinkResult<()> {
        self.tap.clear();
        self.inner.stop()
    }

    fn write(&mut self, packet: AudioPacket, converter: &mut Converter) -> SinkResult<()> {
        let packet = match packet {
            AudioPacket::Samples(mut samples) => {
                self.eq.process(&mut samples);
                let gain = self.applied_volume.as_ref().map_or(1.0, |volume| {
                    let attenuation = volume.attenuation_factor() as f32;
                    if attenuation > 0.001 {
                        1.0 / attenuation
                    } else {
                        1.0
                    }
                });
                self.tap.push(&samples, gain);
                AudioPacket::Samples(samples)
            }
            raw => raw,
        };
        self.inner.write(packet, converter)
    }
}

/// Nullsoft's FFT: 1024 samples in, 512 magnitudes out, windowed with a
/// raised sine and equalised on a log scale so the treble holds its own.
struct Fft {
    bit_reversed: Vec<usize>,
    envelope: Vec<f32>,
    equalize: Vec<f32>,
    twiddles: Vec<(f32, f32)>,
    real: Vec<f32>,
    imaginary: Vec<f32>,
}

impl Fft {
    fn new() -> Self {
        let n = FFT_SAMPLES;
        let mut bit_reversed: Vec<usize> = (0..n).collect();
        let mut j = 0;
        for i in 0..n {
            if j > i {
                bit_reversed.swap(i, j);
            }
            let mut m = n >> 1;
            while m >= 1 && j >= m {
                j -= m;
                m >>= 1;
            }
            j += m;
        }
        let envelope = (0..n)
            .map(|i| {
                let phase = i as f32 / n as f32 * std::f32::consts::TAU;
                0.5 + 0.5 * (phase - std::f32::consts::FRAC_PI_2).sin()
            })
            .collect();
        let mut bias = 0.04f32;
        let equalize = (0..SPECTRUM_BINS)
            .map(|i| {
                let step = (9.0 - bias) / SPECTRUM_BINS as f32;
                let value = (1.0 + bias + (i + 1) as f32 * step).log10();
                bias /= 1.0025;
                value
            })
            .collect();
        let mut twiddles = Vec::new();
        let mut size = 2;
        while size <= n {
            let theta = -std::f32::consts::TAU / size as f32;
            twiddles.push((theta.cos(), theta.sin()));
            size <<= 1;
        }
        Self {
            bit_reversed,
            envelope,
            equalize,
            twiddles,
            real: vec![0.0; n],
            imaginary: vec![0.0; n],
        }
    }

    fn spectrum(&mut self, wave: &[f32], out: &mut [f32; SPECTRUM_BINS]) {
        let n = FFT_SAMPLES;
        for i in 0..n {
            let from = self.bit_reversed[i];
            self.real[i] = wave.get(from).copied().unwrap_or(0.0) * self.envelope[from];
            self.imaginary[i] = 0.0;
        }
        let mut size = 2;
        let mut stage = 0;
        while size <= n {
            let (wpr, wpi) = self.twiddles[stage];
            let (mut wr, mut wi) = (1.0f32, 0.0f32);
            let half = size >> 1;
            for m in 0..half {
                let mut i = m;
                while i < n {
                    let j = i + half;
                    let tr = wr * self.real[j] - wi * self.imaginary[j];
                    let ti = wr * self.imaginary[j] + wi * self.real[j];
                    self.real[j] = self.real[i] - tr;
                    self.imaginary[j] = self.imaginary[i] - ti;
                    self.real[i] += tr;
                    self.imaginary[i] += ti;
                    i += size;
                }
                let previous = wr;
                wr = wr * wpr - wi * wpi;
                wi = wi * wpr + previous * wpi;
            }
            size <<= 1;
            stage += 1;
        }
        for (i, slot) in out.iter_mut().enumerate() {
            *slot = (self.real[i] * self.real[i] + self.imaginary[i] * self.imaginary[i]).sqrt()
                * self.equalize[i];
        }
    }
}

/// One bar of the analyser, in rows from the bottom.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Bar {
    /// How tall the bar is, 0 to 15.
    pub height: u8,
    /// Where the peak mark sits, as the height of a bar whose top row it
    /// would be, 1 to 16; `None` while it is out of sight.
    pub peak: Option<u8>,
}

/// The spectrum analyser's memory: where each bar and its peak are.
pub struct Analyser {
    fft: Fft,
    spectrum: [f32; SPECTRUM_BINS],
    /// Where each bar is, falling at a fixed rate towards the sound.
    falloff: [f32; BARS],
    /// Each peak, in 256ths of a row, and how fast it is dropping.
    peaks: [i32; BARS],
    peak_speed: [f32; BARS],
}

impl Default for Analyser {
    fn default() -> Self {
        Self {
            fft: Fft::new(),
            spectrum: [0.0; SPECTRUM_BINS],
            falloff: [0.0; BARS],
            peaks: [0; BARS],
            peak_speed: [0.0; BARS],
        }
    }
}

impl Analyser {
    /// One frame: the spectrum of `samples` (1024 of them, mono, -1 to 1)
    /// moves the bars, and the bars are returned.
    pub fn step(&mut self, samples: &[f32]) -> [Bar; BARS] {
        let wave: Vec<f32> = samples.iter().map(|sample| sample * INPUT_GAIN).collect();
        self.fft.spectrum(&wave, &mut self.spectrum);
        let columns = self.columns();
        let mut bars = [Bar::default(); BARS];
        for (bar, slot) in bars.iter_mut().enumerate() {
            let chunk = 4 * bar;
            let sound =
                (columns[chunk] + columns[chunk + 1] + columns[chunk + 2] + columns[chunk + 3])
                    / 4.0;
            // Winamp kept the target as a whole number of rows.
            let target = sound.min(MAX_HEIGHT).trunc();
            let falloff = &mut self.falloff[bar];
            *falloff -= FALLOFF;
            if *falloff <= target {
                *falloff = target;
            }
            let peak = &mut self.peaks[bar];
            if *peak <= (*falloff * 256.0).round() as i32 {
                *peak = (*falloff * 256.0) as i32;
                self.peak_speed[bar] = 3.0;
            }
            let peak_row = *peak / 256;
            *peak -= self.peak_speed[bar].round() as i32;
            self.peak_speed[bar] *= PEAK_FALLOFF;
            if *peak <= 0 {
                *peak = 0;
            }
            slot.height = falloff.round() as u8;
            slot.peak = (peak_row >= 1).then_some((peak_row + 1) as u8);
        }
        bars
    }

    /// Whether every bar and peak has come to rest, so nothing moves until
    /// there is sound again.
    pub fn settled(&self) -> bool {
        self.falloff.iter().all(|f| *f <= 0.0) && self.peaks.iter().all(|p| *p == 0)
    }

    pub fn reset(&mut self) {
        self.falloff = [0.0; BARS];
        self.peaks = [0; BARS];
        self.peak_speed = [0.0; BARS];
    }

    /// The spectrum spread over the columns: mostly logarithmic, with a
    /// little linear mixed in, the way later Winamps swept it. One extra
    /// silent column pads the last bar's group of four.
    fn columns(&self) -> [f32; COLUMNS + 1] {
        const SCALE: f32 = 0.91;
        let max_index = SPECTRUM_BINS as f32;
        let log_max = max_index.log10();
        let mut columns = [0.0; COLUMNS + 1];
        for (x, column) in columns.iter_mut().take(COLUMNS).enumerate() {
            let along = x as f32 / (COLUMNS - 1) as f32;
            let linear = along * (max_index - 1.0);
            let logarithmic = 10f32.powf(log_max * along);
            let scaled = (1.0 - SCALE) * linear + SCALE * logarithmic;
            let low = (scaled.floor() as usize).min(SPECTRUM_BINS - 1);
            let high = (scaled.ceil() as usize).min(SPECTRUM_BINS - 1);
            *column = if low == high {
                self.spectrum[low]
            } else {
                let towards_high = scaled - low as f32;
                (1.0 - towards_high) * self.spectrum[low] + towards_high * self.spectrum[high]
            };
        }
        columns
    }
}

/// The oscilloscope's trace: a row (0 at the top) for each column, from
/// every seventh of the samples, the wave's centre at row 7.
pub fn scope(samples: &[f32]) -> [u8; COLUMNS] {
    let mut rows = [7u8; COLUMNS];
    for (column, row) in rows.iter_mut().enumerate() {
        let sample = samples.get(column * 7).copied().unwrap_or(0.0);
        let byte = ((sample * 128.0 + 128.0).round()).clamp(0.0, 255.0);
        let y = (byte / 16.0 * 2.0).round() - 9.0;
        *row = y.clamp(0.0, f32::from(ROWS) - 1.0) as u8;
    }
    rows
}

/// Which of the scope's five colours a row is drawn in: brightest at the
/// centre, darker towards the edges.
pub fn scope_shade(row: u8) -> usize {
    match row {
        14.. => 4,
        12..=13 => 3,
        10..=11 => 2,
        8..=9 => 1,
        6..=7 => 0,
        4..=5 => 1,
        2..=3 => 2,
        _ => 3,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sine(hz: f32, amplitude: f32, count: usize) -> Vec<f32> {
        (0..count)
            .map(|i| amplitude * (i as f32 / SAMPLE_RATE as f32 * hz * std::f32::consts::TAU).sin())
            .collect()
    }

    #[test]
    fn the_tap_mixes_stereo_down_and_pads_with_silence() {
        let tap = AudioTap::new();
        let interleaved: Vec<f64> = vec![0.5, -0.5, 1.0, 0.0, 0.2, 0.2];
        tap.push(&interleaved, 2.0);
        assert_eq!(tap.window(5, 0), [0.0, 0.0, 0.0, 1.0, 0.4]);
        assert_eq!(tap.window(2, 1), [0.0, 1.0]);
        tap.clear();
        assert_eq!(tap.window(2, 0), [0.0, 0.0]);
    }

    #[test]
    fn the_tap_keeps_half_a_second_at_most() {
        let tap = AudioTap::new();
        let interleaved = vec![0.25f64; 2 * (KEPT + 100)];
        tap.push(&interleaved, 1.0);
        let samples = tap.samples.lock().unwrap();
        assert_eq!(samples.len(), KEPT);
    }

    #[test]
    fn the_spectrum_peaks_where_the_tone_is() {
        let mut fft = Fft::new();
        let mut out = [0.0; SPECTRUM_BINS];
        let wave: Vec<f32> = sine(1000.0, 0.5, FFT_SAMPLES)
            .into_iter()
            .map(|s| s * INPUT_GAIN)
            .collect();
        fft.spectrum(&wave, &mut out);
        let loudest = (0..SPECTRUM_BINS)
            .max_by(|a, b| out[*a].total_cmp(&out[*b]))
            .unwrap();
        let expected = (1000.0 / SAMPLE_RATE as f32 * FFT_SAMPLES as f32).round() as usize;
        assert!(
            loudest.abs_diff(expected) <= 1,
            "peak at bin {loudest}, tone at {expected}"
        );
    }

    #[test]
    fn bars_rise_with_sound_and_fall_without() {
        let mut analyser = Analyser::default();
        let loud: Vec<f32> = (0..FFT_SAMPLES)
            .map(|i| {
                let t = i as f32 / SAMPLE_RATE as f32 * std::f32::consts::TAU;
                0.3 * ((t * 100.0).sin() + (t * 800.0).sin() + (t * 5000.0).sin())
            })
            .collect();
        let bars = analyser.step(&loud);
        let tallest = bars.iter().map(|bar| bar.height).max().unwrap();
        assert!(tallest > 0, "no bar rose to the sound");
        assert!(tallest <= 15);
        assert!(!analyser.settled());

        let silence = vec![0.0; FFT_SAMPLES];
        let after = analyser.step(&silence);
        let lower = bars
            .iter()
            .zip(after.iter())
            .all(|(before, after)| after.height <= before.height);
        assert!(lower, "bars rose in silence");
        // The peak hangs above the bar it came from.
        let with_peak = after.iter().find(|bar| bar.peak.is_some()).unwrap();
        assert!(with_peak.peak.unwrap() > with_peak.height);
        for _ in 0..400 {
            analyser.step(&silence);
        }
        assert!(analyser.settled());
        assert!(
            analyser
                .step(&silence)
                .iter()
                .all(|bar| bar.height == 0 && bar.peak.is_none())
        );
    }

    #[test]
    fn the_scope_rests_at_the_middle_and_stays_inside() {
        assert!(scope(&[0.0; SCOPE_SAMPLES]).iter().all(|row| *row == 7));
        let loud = scope(&sine(440.0, 1.0, SCOPE_SAMPLES));
        assert!(loud.iter().all(|row| *row < ROWS));
        assert!(loud.iter().any(|row| *row != 7));
        assert_eq!(scope_shade(7), 0);
        assert_eq!(scope_shade(0), 3);
        assert_eq!(scope_shade(15), 4);
    }
}
