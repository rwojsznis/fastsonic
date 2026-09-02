//! Opus decoding, which symphonia does not carry.
//!
//! Symphonia demuxes Ogg Opus and seeks inside it, but it has no Opus decoder
//! in any released version — and a self-hosted library has Opus in it. This
//! module is the missing half: an [`OpusDecoder`] over `opus-pure`, and
//! [`codecs`], the registry the audio engine decodes with, which is
//! symphonia's own set plus this one.
//!
//! Nothing else in the app names `opus_pure`. Swapping the implementation —
//! for libopus, or for symphonia's own once it has one — is a change to this
//! file and no other. See Q7 in `migration/00-decisions.md`.
//!
//! Two details of the format that live here rather than in the engine:
//!
//! - **The stream decodes at 48 kHz whatever it was made from.** The rate in
//!   the header is the *input's* rate and is informational; Opus itself only
//!   speaks 8, 12, 16, 24 and 48 kHz, and 48 is the one that never resamples.
//! - **The header's output gain is applied here**, because it belongs to the
//!   file rather than to playback. What is *not* applied here is the encoder
//!   delay: symphonia reports it as the track's `delay` and `padding`, in
//!   frames, and trimming it is the engine's job at the seams between tracks.

use std::sync::LazyLock;

use symphonia::core::audio::{
    AsGenericAudioBufferRef, AudioBuffer, AudioMut, AudioSpec, Channels, GenericAudioBufferRef,
};
use symphonia::core::codecs::CodecInfo;
use symphonia::core::codecs::audio::well_known::CODEC_ID_OPUS;
use symphonia::core::codecs::audio::{
    AudioCodecParameters, AudioDecoder, AudioDecoderOptions, FinalizeResult,
};
use symphonia::core::codecs::registry::{
    CodecRegistry, RegisterableAudioDecoder, SupportedAudioCodec,
};
use symphonia::core::errors::{Result, decode_error, unsupported_error};
use symphonia::core::packet::PacketRef;

/// Every Opus stream decodes at one of five rates; this is the one that is
/// never a resample of the encoder's own.
const DECODE_RATE: u32 = 48_000;

/// The longest packet Opus allows, per channel: 120 ms at 48 kHz.
const MAX_FRAMES_PER_PACKET: usize = 5_760;

/// Channel mapping families this decoder handles: 0 is mono or stereo, 1 is
/// Vorbis-order surround. 255 (discrete channels) needs the mapping table
/// from the header and no music file uses it.
const MAPPING_MONO_STEREO: u8 = 0;
const MAPPING_SURROUND: u8 = 1;

/// The codec registry the audio engine decodes with: everything symphonia's
/// features enable, plus Opus.
///
/// Use this rather than `symphonia::default::get_codecs()`, which has no
/// Opus in it.
pub fn codecs() -> &'static CodecRegistry {
    static REGISTRY: LazyLock<CodecRegistry> = LazyLock::new(|| {
        let mut registry = CodecRegistry::new();
        symphonia::default::register_enabled_codecs(&mut registry);
        registry.register_audio_decoder::<OpusDecoder>();
        registry
    });
    &REGISTRY
}

/// What the identification header says about the stream (RFC 7845 §5.1).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Head {
    channels: u8,
    /// Frames of encoder delay at 48 kHz. Symphonia reports the same number
    /// as the track's `delay`; it is read here only to be logged.
    pre_skip: u16,
    /// Playback gain in Q7.8 decibels. 0 is unity, and almost every file
    /// says 0.
    output_gain_q8: i16,
    mapping_family: u8,
}

impl Head {
    /// Reads the header symphonia carried through as the track's extra data.
    fn parse(data: &[u8]) -> Result<Self> {
        // 8 bytes of magic, version, channels, pre-skip, input rate, gain,
        // mapping family: 19 bytes before any channel mapping table.
        if data.len() < 19 || &data[..8] != b"OpusHead" {
            return decode_error("opus: not an identification header");
        }
        // The version's high nibble is the incompatible half: 0x0f and below
        // is this mapping, whatever the low nibble says.
        if data[8] & 0xf0 != 0 {
            return unsupported_error("opus: newer header version");
        }
        Ok(Self {
            channels: data[9],
            pre_skip: u16::from_le_bytes([data[10], data[11]]),
            output_gain_q8: i16::from_le_bytes([data[16], data[17]]),
            mapping_family: data[18],
        })
    }

    /// The header's gain as a factor to multiply samples by.
    fn gain(&self) -> f32 {
        match self.output_gain_q8 {
            0 => 1.0,
            gain => 10.0_f32.powf(f32::from(gain) / 256.0 / 20.0),
        }
    }
}

/// The decoders `opus-pure` offers: one stream for mono or stereo, and the
/// multistream decoder for surround.
enum Engine {
    /// Boxed because a single decoder's state is ten kilobytes and the
    /// multistream one holds its streams on the heap already.
    Single(Box<opus_pure::OpusDecoder>),
    Multi(opus_pure::OpusMSDecoder),
}

/// An Opus decoder for symphonia, over `opus-pure`.
pub struct OpusDecoder {
    engine: Engine,
    params: AudioCodecParameters,
    channels: usize,
    gain: f32,
    /// The decoded packet, planar, which is what the rest of symphonia
    /// speaks.
    buf: AudioBuffer<f32>,
    /// `opus-pure` writes interleaved, so one packet lands here first.
    interleaved: Vec<f32>,
}

impl OpusDecoder {
    pub fn try_new(params: &AudioCodecParameters, _options: &AudioDecoderOptions) -> Result<Self> {
        // The header is the authority on the channel count and the mapping;
        // the codec parameters carry it verbatim as extra data. A container
        // that lost it leaves the parameters, and mono or stereo.
        let head = match params.extra_data.as_deref() {
            Some(data) => Some(Head::parse(data)?),
            None => None,
        };
        let channels = match (&head, &params.channels) {
            (Some(head), _) => usize::from(head.channels),
            (None, Some(channels)) => channels.count(),
            (None, None) => return decode_error("opus: the stream does not say how many channels"),
        };
        if channels == 0 || channels > 255 {
            return decode_error("opus: impossible channel count");
        }
        let family = head.map_or(MAPPING_MONO_STEREO, |head| head.mapping_family);
        let engine = match family {
            MAPPING_MONO_STEREO if channels <= 2 => Engine::Single(Box::new(
                opus_pure::OpusDecoder::new(DECODE_RATE as i32, channels).map_err(|error| {
                    symphonia::core::errors::Error::DecodeError(leak(error.to_string()))
                })?,
            )),
            MAPPING_MONO_STEREO => return decode_error("opus: mapping family 0 is mono or stereo"),
            MAPPING_SURROUND => Engine::Multi(
                opus_pure::OpusMSDecoder::new(DECODE_RATE as i32, channels, family).map_err(
                    |error| symphonia::core::errors::Error::DecodeError(leak(error.to_string())),
                )?,
            ),
            other => {
                log::warn!("opus: channel mapping family {other} is not supported");
                return unsupported_error("opus: unsupported channel mapping family");
            }
        };

        if let Some(head) = head {
            log::debug!(
                "opus: {channels} channel(s), mapping family {family}, {} frames of encoder delay, {:+.1} dB header gain",
                head.pre_skip,
                20.0 * head.gain().log10(),
            );
        }

        // Symphonia's own parameters, corrected to what comes out of the
        // decoder rather than what the file was made from.
        let mut corrected = params.clone();
        corrected.with_sample_rate(DECODE_RATE);
        if corrected.channels.is_none() {
            corrected.with_channels(default_channels(channels));
        }

        let spec = AudioSpec::new(
            DECODE_RATE,
            corrected
                .channels
                .clone()
                .unwrap_or_else(|| default_channels(channels)),
        );
        Ok(Self {
            engine,
            params: corrected,
            channels,
            gain: head.map_or(1.0, |head| head.gain()),
            buf: AudioBuffer::new(spec, MAX_FRAMES_PER_PACKET),
            interleaved: vec![0.0; MAX_FRAMES_PER_PACKET * channels],
        })
    }

    fn decode_inner(&mut self, packet: &PacketRef<'_>) -> Result<()> {
        let frames = match &mut self.engine {
            Engine::Single(decoder) => {
                decoder.decode(packet.data, MAX_FRAMES_PER_PACKET, &mut self.interleaved)
            }
            Engine::Multi(decoder) => {
                decoder.decode(packet.data, MAX_FRAMES_PER_PACKET, &mut self.interleaved)
            }
        }
        .map_err(|error| {
            symphonia::core::errors::Error::DecodeError(leak(format!("opus: {error}")))
        })?;

        self.buf.clear();
        self.buf.render_uninit(Some(frames));
        for channel in 0..self.channels {
            let Some(plane) = self.buf.plane_mut(channel) else {
                return decode_error("opus: fewer planes than channels");
            };
            for (frame, sample) in plane[..frames].iter_mut().enumerate() {
                *sample = self.interleaved[frame * self.channels + channel] * self.gain;
            }
        }
        Ok(())
    }
}

impl AudioDecoder for OpusDecoder {
    /// Opus recovers from a discontinuity on its own — the next packet is
    /// decoded with no history — but the state has to be dropped first or the
    /// join carries the sound of wherever the last packet came from.
    fn reset(&mut self) {
        let reset = match &mut self.engine {
            Engine::Single(decoder) => decoder.reset_state(),
            Engine::Multi(decoder) => decoder.reset_state(),
        };
        if let Err(error) = reset {
            log::warn!("opus: the decoder would not reset ({error})");
        }
    }

    fn codec_info(&self) -> &CodecInfo {
        &Self::supported_codecs()[0].info
    }

    fn codec_params(&self) -> &AudioCodecParameters {
        &self.params
    }

    fn decode_ref(&mut self, packet: &PacketRef<'_>) -> Result<GenericAudioBufferRef<'_>> {
        match self.decode_inner(packet) {
            Ok(()) => Ok(self.buf.as_generic_audio_buffer_ref()),
            Err(error) => {
                // The contract: a failed decode leaves nothing behind.
                self.buf.clear();
                Err(error)
            }
        }
    }

    fn finalize(&mut self) -> FinalizeResult {
        FinalizeResult::default()
    }

    fn last_decoded(&self) -> GenericAudioBufferRef<'_> {
        self.buf.as_generic_audio_buffer_ref()
    }
}

impl RegisterableAudioDecoder for OpusDecoder {
    fn try_registry_new(
        params: &AudioCodecParameters,
        options: &AudioDecoderOptions,
    ) -> Result<Box<dyn AudioDecoder>> {
        Ok(Box::new(Self::try_new(params, options)?))
    }

    fn supported_codecs() -> &'static [SupportedAudioCodec] {
        &[SupportedAudioCodec {
            id: CODEC_ID_OPUS,
            info: CodecInfo {
                short_name: "opus",
                long_name: "Opus",
                profiles: &[],
            },
        }]
    }
}

/// Speaker positions for a channel count the header did not describe.
fn default_channels(count: usize) -> Channels {
    match count {
        1 => Channels::Positioned(symphonia::core::audio::Position::FRONT_LEFT),
        2 => Channels::Positioned(
            symphonia::core::audio::Position::FRONT_LEFT
                | symphonia::core::audio::Position::FRONT_RIGHT,
        ),
        other => Channels::Discrete(other as u16),
    }
}

/// Symphonia's decode errors carry a `&'static str`, so a message built at
/// runtime has to outlive the call. Decode failures are rare and each one is
/// a few dozen bytes; a mangled file cannot make this grow without bound
/// because the engine gives up on a track long before.
fn leak(message: String) -> &'static str {
    Box::leak(message.into_boxed_str())
}

#[cfg(test)]
mod tests {
    use super::*;
    use symphonia::core::packet::Packet;
    use symphonia::core::units::{Duration, Timestamp};

    /// A header the way `opusenc` writes one: stereo, 312 frames of delay,
    /// unity gain, mapping family 0.
    fn head_bytes(channels: u8, pre_skip: u16, gain_q8: i16, family: u8) -> Vec<u8> {
        let mut data = b"OpusHead".to_vec();
        data.push(1); // version
        data.push(channels);
        data.extend_from_slice(&pre_skip.to_le_bytes());
        data.extend_from_slice(&48_000_u32.to_le_bytes());
        data.extend_from_slice(&gain_q8.to_le_bytes());
        data.push(family);
        data
    }

    fn params(extra: Option<Vec<u8>>) -> AudioCodecParameters {
        let mut params = AudioCodecParameters::new();
        params.for_codec(CODEC_ID_OPUS).with_sample_rate(48_000);
        if let Some(extra) = extra {
            params.with_extra_data(extra.into_boxed_slice());
        }
        params
    }

    /// One packet of real Opus, from this crate's own encoder, so the test
    /// needs no fixture file.
    fn one_packet(channels: usize, frames: usize) -> Vec<u8> {
        let mut encoder =
            opus_pure::OpusEncoder::new(48_000, channels, opus_pure::Application::Audio).unwrap();
        let pcm: Vec<f32> = (0..frames * channels)
            .map(|index| {
                let time = (index / channels) as f32 / 48_000.0;
                (time * 440.0 * std::f32::consts::TAU).sin() * 0.5
            })
            .collect();
        let mut packet = vec![0u8; opus_pure::MAX_PACKET_BYTES];
        let written = encoder.encode(&pcm, frames, &mut packet).unwrap();
        packet.truncate(written);
        packet
    }

    fn packet(data: Vec<u8>, frames: u64) -> Packet {
        Packet::new(0, Timestamp::ZERO, Duration::new(frames), data)
    }

    /// The identification header is where the channel count and the mapping
    /// come from, so reading it wrong means decoding the wrong stream.
    #[test]
    fn the_identification_header_is_read_field_by_field() {
        let head = Head::parse(&head_bytes(2, 312, 0, 0)).unwrap();
        assert_eq!(head.channels, 2);
        assert_eq!(head.pre_skip, 312);
        assert_eq!(head.output_gain_q8, 0);
        assert_eq!(head.mapping_family, 0);
        assert_eq!(head.gain(), 1.0, "no gain field means no gain applied");

        // -6 dB in Q7.8 is -1536, and halves the samples.
        let quiet = Head::parse(&head_bytes(2, 312, -1536, 0)).unwrap();
        assert!((quiet.gain() - 0.501).abs() < 0.001, "{}", quiet.gain());
    }

    /// Rule: anything that is not a header this decoder understands is an
    /// error, not a guess. A truncated one is the shape a broken file has.
    #[test]
    fn a_header_that_is_not_one_is_refused() {
        assert!(Head::parse(b"OpusHead").is_err(), "too short");
        assert!(Head::parse(&[0; 19]).is_err(), "no magic");
        let mut future = head_bytes(2, 312, 0, 0);
        future[8] = 0x10;
        assert!(Head::parse(&future).is_err(), "a later header version");
    }

    /// The point of the module: a packet in, planar `f32` out, at 48 kHz.
    #[test]
    fn a_packet_decodes_to_planar_audio_at_48_khz() {
        let mut decoder =
            OpusDecoder::try_new(&params(Some(head_bytes(2, 312, 0, 0))), &Default::default())
                .unwrap();
        let decoded = decoder
            .decode(&packet(one_packet(2, 960), 960))
            .expect("the packet decodes");
        assert_eq!(decoded.spec().rate(), 48_000);
        assert_eq!(decoded.spec().channels().count(), 2);
        assert_eq!(decoded.frames(), 960, "20 ms at 48 kHz");

        let mut samples: Vec<f32> = Vec::new();
        decoded.copy_to_vec_interleaved(&mut samples);
        assert_eq!(samples.len(), 1920);
        let loudest = samples
            .iter()
            .fold(0.0_f32, |peak, sample| peak.max(sample.abs()));
        assert!(loudest > 0.1, "a 440 Hz tone came back silent ({loudest})");
    }

    /// The header's gain belongs to the file, so it is applied before the
    /// audio reaches the equalizer or the visualisers.
    #[test]
    fn the_header_gain_is_applied_to_the_samples() {
        let data = one_packet(2, 960);
        let peak = |gain_q8: i16| {
            let mut decoder = OpusDecoder::try_new(
                &params(Some(head_bytes(2, 312, gain_q8, 0))),
                &Default::default(),
            )
            .unwrap();
            let decoded = decoder.decode(&packet(data.clone(), 960)).unwrap();
            let mut samples = Vec::new();
            decoded.copy_to_vec_interleaved::<f32>(&mut samples);
            samples
                .iter()
                .fold(0.0_f32, |peak, sample| peak.max(sample.abs()))
        };
        let unity = peak(0);
        let half = peak(-1536); // -6 dB
        assert!(
            (half / unity - 0.501).abs() < 0.01,
            "-6 dB should halve the peak: {unity} -> {half}"
        );
    }

    /// A packet that is not Opus must leave the decoder usable and its
    /// buffer empty, because the engine skips the packet and carries on.
    #[test]
    fn a_bad_packet_is_an_error_and_leaves_nothing_behind() {
        let mut decoder =
            OpusDecoder::try_new(&params(Some(head_bytes(2, 312, 0, 0))), &Default::default())
                .unwrap();
        assert!(decoder.decode(&packet(vec![0xff; 8], 960)).is_err());
        assert_eq!(decoder.last_decoded().frames(), 0);
        // And the next real packet still decodes.
        assert!(decoder.decode(&packet(one_packet(2, 960), 960)).is_ok());
    }

    /// The registry the engine uses has to have both halves in it: Opus,
    /// which symphonia lacks, and everything symphonia brings.
    #[test]
    fn the_registry_holds_opus_beside_symphonias_own_codecs() {
        use symphonia::core::codecs::audio::well_known::{CODEC_ID_FLAC, CODEC_ID_OPUS};
        assert!(codecs().get_audio_decoder(CODEC_ID_OPUS).is_some(), "opus");
        assert!(codecs().get_audio_decoder(CODEC_ID_FLAC).is_some(), "flac");
        assert!(
            symphonia::default::get_codecs()
                .get_audio_decoder(CODEC_ID_OPUS)
                .is_none(),
            "and symphonia's own registry still has no opus, which is why this module exists"
        );
    }

    /// Rule: a mapping family this decoder cannot lay out is refused with a
    /// message, not decoded into noise.
    #[test]
    fn an_unsupported_channel_mapping_is_refused() {
        let discrete = params(Some(head_bytes(4, 312, 0, 255)));
        assert!(OpusDecoder::try_new(&discrete, &Default::default()).is_err());
        // Family 0 is defined for one or two channels only.
        let wrong = params(Some(head_bytes(3, 312, 0, 0)));
        assert!(OpusDecoder::try_new(&wrong, &Default::default()).is_err());
    }
}
