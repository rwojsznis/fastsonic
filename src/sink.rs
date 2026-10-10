//! Audio output for local playback.
//!
//! Stream opening is fallible because release builds abort on panic. A machine
//! without an output device gets an error in the interface instead.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread;
use std::time::Duration;

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};

/// The backend name Settings uses for this sink.
pub const NAME: &str = "rodio";

/// Told about output failures, with a message fit for the interface.
pub type ErrorHook = Arc<dyn Fn(String) + Send + Sync>;

/// How often playback looks at which output the system calls its default.
const DEFAULT_CHECK_INTERVAL: Duration = Duration::from_secs(2);

/// Default Windows device buffer length in milliseconds.
///
/// Small platform defaults can click under load (#88). A 100 ms buffer avoids
/// these underruns while keeping controls responsive.
pub const DEFAULT_BUFFER_MS: u32 = 100;

/// Allowed Windows device buffer range. Lower values can click; higher values
/// delay playback controls.
pub const BUFFER_MS_RANGE: std::ops::RangeInclusive<u32> = 20..=500;

/// The buffer to ask the device for, in frames.
///
/// Clamp to the reported range because CoreAudio rejects unsupported sizes.
/// If a device reports no range, request the configured size; `open_stream`
/// can retry without a fixed size.
pub(crate) fn engine_buffer(
    sample_rate: u32,
    ms: u32,
    supported: cpal::SupportedBufferSize,
) -> cpal::BufferSize {
    let ms = ms.clamp(*BUFFER_MS_RANGE.start(), *BUFFER_MS_RANGE.end());
    let frames = (u64::from(sample_rate) * u64::from(ms) / 1000).max(1) as u32;
    match supported {
        cpal::SupportedBufferSize::Range { min, max } if min <= max && max > 0 => {
            cpal::BufferSize::Fixed(frames.clamp(min.max(1), max))
        }
        _ => cpal::BufferSize::Fixed(frames),
    }
}

/// The device's stream and the mixer it plays, held here rather than in
/// rodio's `OutputStream` so that Pause can stop it.
///
/// rodio keeps its cpal stream private and running until it is dropped, so
/// a paused player still had the device asking for sound and the mixer
/// making silence for it, for as long as the app stayed open. Pausing the
/// stream stops those callbacks but keeps the device open, so Play starts
/// again at once.
pub(crate) struct DeviceStream {
    stream: cpal::Stream,
    pub(crate) mixer: rodio::mixer::Mixer,
    pub(crate) sample_rate: u32,
    /// Whether the device is asking for sound.
    running: bool,
}

impl DeviceStream {
    fn open(
        device: &cpal::Device,
        config: &cpal::StreamConfig,
        format: cpal::SampleFormat,
        on_error: impl FnMut(cpal::StreamError) + Send + 'static,
    ) -> Result<Self, rodio::StreamError> {
        let (mixer, source) = rodio::mixer::mixer(
            config.channels as rodio::ChannelCount,
            config.sample_rate.0 as rodio::SampleRate,
        );
        let stream = build_stream(device, config, format, source, on_error)?;
        stream.play().map_err(rodio::StreamError::PlayStreamError)?;
        Ok(Self {
            stream,
            mixer,
            sample_rate: config.sample_rate.0,
            running: true,
        })
    }

    /// Whether the device is asking for sound.
    #[cfg(test)]
    pub(crate) fn running(&self) -> bool {
        self.running
    }

    /// Has the device ask for sound again. A device that will not is
    /// marked `failed`, so the next append reopens the output.
    pub(crate) fn resume(&mut self, failed: &AtomicBool) {
        if self.running {
            return;
        }
        match self.stream.play() {
            Ok(()) => self.running = true,
            Err(error) => {
                log::error!("cannot restart the audio output: {error}");
                failed.store(true, Ordering::Relaxed);
            }
        }
    }

    /// Stops the device asking for sound, keeping it open. Never fails: a
    /// stream that cannot pause keeps running, as it always used to.
    pub(crate) fn pause(&mut self, failed: &AtomicBool) {
        if !self.running {
            return;
        }
        match self.stream.pause() {
            Ok(()) => self.running = false,
            Err(cpal::PauseStreamError::DeviceNotAvailable) => {
                log::warn!("the audio output went away while pausing");
                failed.store(true, Ordering::Relaxed);
            }
            Err(error) => log::warn!("cannot pause the audio output: {error}"),
        }
    }
}

/// Builds a stream that plays `source` in the device's sample format, with
/// silence whenever the mixer has nothing, as rodio's own stream does.
fn build_stream(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    format: cpal::SampleFormat,
    source: rodio::mixer::MixerSource,
    on_error: impl FnMut(cpal::StreamError) + Send + 'static,
) -> Result<cpal::Stream, rodio::StreamError> {
    fn build<T>(
        device: &cpal::Device,
        config: &cpal::StreamConfig,
        mut source: rodio::mixer::MixerSource,
        on_error: impl FnMut(cpal::StreamError) + Send + 'static,
    ) -> Result<cpal::Stream, cpal::BuildStreamError>
    where
        T: cpal::SizedSample + cpal::FromSample<rodio::Sample>,
    {
        device.build_output_stream::<T, _, _>(
            config,
            move |data: &mut [T], _| {
                for out in data {
                    *out = source.next().map(T::from_sample_).unwrap_or(T::EQUILIBRIUM);
                }
            },
            on_error,
            None,
        )
    }
    use cpal::SampleFormat as Format;
    match format {
        Format::F32 => build::<f32>(device, config, source, on_error),
        Format::F64 => build::<f64>(device, config, source, on_error),
        Format::I8 => build::<i8>(device, config, source, on_error),
        Format::I16 => build::<i16>(device, config, source, on_error),
        Format::I32 => build::<i32>(device, config, source, on_error),
        Format::I64 => build::<i64>(device, config, source, on_error),
        Format::U8 => build::<u8>(device, config, source, on_error),
        Format::U16 => build::<u16>(device, config, source, on_error),
        Format::U32 => build::<u32>(device, config, source, on_error),
        Format::U64 => build::<u64>(device, config, source, on_error),
        _ => return Err(rodio::StreamError::UnsupportedSampleFormat),
    }
    .map_err(rodio::StreamError::BuildStreamError)
}

/// Opens the stream at the rate the chain would rather run at, so nothing
/// is converted, else at the device's own rate, which Windows insists on
/// for a shared device, else at any configuration the device lists.
///
/// The first two attempts request the configured buffer. The fallback lets
/// the driver choose its buffer size.
///
/// The engine prefers 44.1 kHz because that is where most of a music library
/// is, rather than because every track must have the same rate.
pub(crate) fn open_stream(
    device: &cpal::Device,
    on_error: impl FnMut(cpal::StreamError) + Send + Clone + 'static,
    buffer_ms: u32,
    channels: u16,
    preferred_rate: u32,
) -> Result<DeviceStream, rodio::StreamError> {
    let default = device
        .default_output_config()
        .map_err(rodio::StreamError::DefaultStreamConfigError)?;
    let format = default.sample_format();
    let config = |sample_rate: u32, buffer: bool| cpal::StreamConfig {
        channels: channels as cpal::ChannelCount,
        sample_rate: cpal::SampleRate(sample_rate),
        buffer_size: if buffer {
            engine_buffer(sample_rate, buffer_ms, *default.buffer_size())
        } else {
            cpal::BufferSize::Default
        },
    };
    // The fixed engine buffer addresses Windows shared-mode underruns (#88).
    // CoreAudio, ALSA, PulseAudio, and PipeWire keep their proven
    // driver-selected callback periods.
    let fixed_buffer = cfg!(windows);
    if let Ok(stream) = DeviceStream::open(
        device,
        &config(preferred_rate, fixed_buffer),
        format,
        on_error.clone(),
    ) {
        return Ok(stream);
    }
    if let Ok(stream) = DeviceStream::open(
        device,
        &config(default.sample_rate().0, fixed_buffer),
        format,
        on_error.clone(),
    ) {
        return Ok(stream);
    }
    DeviceStream::open(
        device,
        &config(preferred_rate, false),
        format,
        on_error.clone(),
    )
    .or_else(|error| {
        for supported in rodio::stream::supported_output_configs(device)? {
            if let Ok(stream) = DeviceStream::open(
                device,
                &supported.config(),
                supported.sample_format(),
                on_error.clone(),
            ) {
                return Ok(stream);
            }
        }
        Err(error)
    })
}

/// Last default-output name, polled on a worker thread because Windows device
/// enumeration can block. The thread ends when the sink is dropped.
pub(crate) struct DefaultWatch(Arc<Mutex<Option<String>>>);

impl DefaultWatch {
    pub(crate) fn start() -> Self {
        let shared = Arc::new(Mutex::new(None));
        let weak = Arc::downgrade(&shared);
        let watching = thread::Builder::new()
            .name("audio-default-watch".into())
            .spawn(move || {
                while let Some(shared) = weak.upgrade() {
                    let name = default_output_name();
                    *shared.lock().unwrap_or_else(PoisonError::into_inner) = name;
                    drop(shared);
                    thread::sleep(DEFAULT_CHECK_INTERVAL);
                }
            });
        if let Err(error) = watching {
            log::warn!("cannot watch the default audio output: {error}");
        }
        Self(shared)
    }

    /// Last polled name, or `None` before the first poll.
    pub(crate) fn name(&self) -> Option<String> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

pub(crate) fn default_output_name() -> Option<String> {
    cpal::default_host()
        .default_output_device()
        .and_then(|device| device.name().ok())
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum OpenError {
    #[error("No audio output device was found. Connect or enable one, then press play again.")]
    NoDevice,
    #[error("Cannot list the audio devices: {0}")]
    Devices(#[from] cpal::DevicesError),
    #[error("Cannot open the audio output: {0}")]
    Stream(#[from] rodio::StreamError),
}

/// The output device Settings asked for, or the system default when it
/// named none — or when the one it named is not plugged in today.
pub(crate) fn pick_device(preferred: Option<&str>) -> Result<cpal::Device, OpenError> {
    let host = cpal::default_host();
    match preferred.map(str::trim).filter(|name| !name.is_empty()) {
        Some(name) => {
            let chosen = host
                .output_devices()?
                .find(|device| device.name().is_ok_and(|found| found == name));
            match chosen {
                Some(device) => Ok(device),
                None => {
                    log::warn!("audio device {name:?} is not available; using the default");
                    host.default_output_device().ok_or(OpenError::NoDevice)
                }
            }
        }
        None => host.default_output_device().ok_or(OpenError::NoDevice),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Converts the configured buffer duration to device frames.
    #[test]
    fn the_buffer_follows_the_setting_and_the_rate() {
        let unknown = cpal::SupportedBufferSize::Unknown;
        assert_eq!(
            engine_buffer(44_100, 100, unknown),
            cpal::BufferSize::Fixed(4410),
            "a tenth of a second at 44.1 kHz"
        );
        assert_eq!(
            engine_buffer(48_000, 100, unknown),
            cpal::BufferSize::Fixed(4800),
            "the same tenth of a second at 48 kHz"
        );
        assert_eq!(
            engine_buffer(44_100, 20, unknown),
            cpal::BufferSize::Fixed(882)
        );
    }

    /// Clamps the buffer to the device range required by CoreAudio.
    #[test]
    fn a_device_that_states_its_range_is_kept_inside_it() {
        let range = cpal::SupportedBufferSize::Range { min: 64, max: 2048 };
        assert_eq!(
            engine_buffer(44_100, 100, range),
            cpal::BufferSize::Fixed(2048),
            "held down to what the device can take"
        );
        assert_eq!(
            engine_buffer(44_100, 20, range),
            cpal::BufferSize::Fixed(882),
            "and left alone when it fits"
        );
        let tiny = cpal::SupportedBufferSize::Range {
            min: 4096,
            max: 8192,
        };
        assert_eq!(
            engine_buffer(44_100, 20, tiny),
            cpal::BufferSize::Fixed(4096),
            "and brought up to a device that will not go smaller"
        );
    }

    /// Rule: a settings file with a wild number in it still opens a
    /// stream. The range is the range whoever wrote the file thought of.
    #[test]
    fn a_number_from_outside_the_range_is_brought_back_in() {
        let unknown = cpal::SupportedBufferSize::Unknown;
        assert_eq!(
            engine_buffer(44_100, 0, unknown),
            engine_buffer(44_100, *BUFFER_MS_RANGE.start(), unknown)
        );
        assert_eq!(
            engine_buffer(44_100, 100_000, unknown),
            engine_buffer(44_100, *BUFFER_MS_RANGE.end(), unknown)
        );
    }
}
