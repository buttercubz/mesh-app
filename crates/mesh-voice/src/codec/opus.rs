//! libopus 1.6.1 backend (feature `opus`, DESIGN §7.4-§7.5).

use opus_sys::{Application, Bandwidth, Bitrate, Channels, Decoder, Encoder, Vbr};

use super::{Codec, CodecError, VoiceDecoder, VoiceEncoder};
use crate::SAMPLE_RATE;
use crate::profile::{AudioBandwidth, EncoderConfig, Profile};

/// Decoder complexity: ≥ 5 deep PLC, ≥ 6 LACE, 7 NoLACE (DESIGN §7.5).
pub const DECODER_COMPLEXITY: u8 = 7;

/// The libopus codec family.
#[derive(Debug, Clone, Copy, Default)]
pub struct OpusCodec;

impl OpusCodec {
    pub fn new() -> Self {
        OpusCodec
    }

    /// Creates a concrete Opus encoder (for tests and diagnostics that read ctls back).
    pub fn encoder(&self) -> Result<OpusVoiceEncoder, CodecError> {
        OpusVoiceEncoder::new()
    }

    /// Creates a concrete Opus decoder.
    pub fn decoder(&self) -> Result<OpusVoiceDecoder, CodecError> {
        OpusVoiceDecoder::new()
    }

    /// libopus version string.
    pub fn version() -> &'static str {
        opus_sys::version()
    }
}

impl Codec for OpusCodec {
    fn name(&self) -> &'static str {
        "opus"
    }

    fn new_encoder(&self) -> Result<Box<dyn VoiceEncoder>, CodecError> {
        Ok(Box::new(OpusVoiceEncoder::new()?))
    }

    fn new_decoder(&self) -> Result<Box<dyn VoiceDecoder>, CodecError> {
        Ok(Box::new(OpusVoiceDecoder::new()?))
    }
}

fn map_err(e: opus_sys::Error) -> CodecError {
    match e.code() {
        opus_sys::ffi::OPUS_BAD_ARG => CodecError::BadArg,
        opus_sys::ffi::OPUS_BUFFER_TOO_SMALL => CodecError::BufferTooSmall,
        opus_sys::ffi::OPUS_INVALID_PACKET => CodecError::InvalidPacket,
        c => CodecError::Backend(c),
    }
}

const fn opus_bw(bw: AudioBandwidth) -> Bandwidth {
    match bw {
        AudioBandwidth::Nb => Bandwidth::Narrowband,
        AudioBandwidth::Mb => Bandwidth::Mediumband,
        AudioBandwidth::Wb => Bandwidth::Wideband,
    }
}

/// Opus encoder applying an [`EncoderConfig`].
#[derive(Debug)]
pub struct OpusVoiceEncoder {
    enc: Encoder,
    cfg: EncoderConfig,
    lookahead: usize,
}

impl OpusVoiceEncoder {
    pub fn new() -> Result<Self, CodecError> {
        let enc = Encoder::new(SAMPLE_RATE, Channels::Mono, Application::Voip).map_err(map_err)?;
        let lookahead = enc.lookahead().map_err(map_err)?;
        let mut s = Self {
            enc,
            cfg: EncoderConfig::for_profile(Profile::Ble16, SAMPLE_RATE, 0.0),
            lookahead,
        };
        let cfg = s.cfg;
        s.configure(&cfg)?;
        Ok(s)
    }

    /// The underlying libopus encoder (read-only, to verify ctls).
    pub fn inner(&self) -> &Encoder {
        &self.enc
    }
}

impl VoiceEncoder for OpusVoiceEncoder {
    fn configure(&mut self, cfg: &EncoderConfig) -> Result<(), CodecError> {
        let e = &mut self.enc;
        e.set_signal_voice().map_err(map_err)?;
        e.set_bitrate(Bitrate::BitsPerSecond(cfg.bitrate_bps as i32))
            .map_err(map_err)?;
        e.set_complexity(cfg.complexity).map_err(map_err)?;
        e.set_vbr(if cfg.constrained_vbr {
            Vbr::Constrained
        } else {
            Vbr::Unconstrained
        })
        .map_err(map_err)?;
        e.set_dtx(cfg.dtx).map_err(map_err)?;
        e.set_max_bandwidth(opus_bw(cfg.max_bandwidth))
            .map_err(map_err)?;
        e.set_inband_fec(cfg.inband_fec).map_err(map_err)?;
        e.set_packet_loss_perc(cfg.packet_loss_perc)
            .map_err(map_err)?;
        e.set_dred_duration(cfg.dred_duration_10ms)
            .map_err(map_err)?;
        self.cfg = *cfg;
        Ok(())
    }

    fn config(&self) -> &EncoderConfig {
        &self.cfg
    }

    fn reset(&mut self) -> Result<(), CodecError> {
        self.enc.reset().map_err(map_err)
    }

    fn encode(&mut self, pcm: &[i16], out: &mut [u8]) -> Result<usize, CodecError> {
        if pcm.len() != self.cfg.frame.samples() {
            return Err(CodecError::BadArg);
        }
        self.enc.encode(pcm, out).map_err(map_err)
    }

    fn lookahead(&self) -> usize {
        self.lookahead
    }
}

/// Opus decoder at complexity 7 (deep PLC + NoLACE).
#[derive(Debug)]
pub struct OpusVoiceDecoder {
    dec: Decoder,
}

impl OpusVoiceDecoder {
    pub fn new() -> Result<Self, CodecError> {
        let mut dec = Decoder::new(SAMPLE_RATE, Channels::Mono).map_err(map_err)?;
        dec.set_complexity(DECODER_COMPLEXITY).map_err(map_err)?;
        Ok(Self { dec })
    }

    /// The underlying libopus decoder (read-only).
    pub fn inner(&self) -> &Decoder {
        &self.dec
    }
}

impl VoiceDecoder for OpusVoiceDecoder {
    fn decode(&mut self, packet: &[u8], out: &mut [i16]) -> Result<usize, CodecError> {
        self.dec.decode(Some(packet), out, false).map_err(map_err)
    }

    fn decode_fec(&mut self, next: &[u8], out: &mut [i16]) -> Result<usize, CodecError> {
        self.dec.decode(Some(next), out, true).map_err(map_err)
    }

    fn conceal(&mut self, out: &mut [i16]) -> Result<usize, CodecError> {
        self.dec.decode(None, out, false).map_err(map_err)
    }

    fn reset(&mut self) -> Result<(), CodecError> {
        self.dec.reset().map_err(map_err)
    }
}
