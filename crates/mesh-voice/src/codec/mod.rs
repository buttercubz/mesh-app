//! Codec abstraction (DESIGN §7.4-§7.5): [`SimCodec`](sim::SimCodec) is always available
//! (wasm32 and the simulator); `OpusCodec` (feature `opus`) wraps libopus 1.6.1.
//!
//! Encoders and decoders are created once (engine start / decoder pool) and then used
//! from the 10 ms paths without allocating.

use core::fmt;

use crate::profile::EncoderConfig;

pub mod sim;

#[cfg(feature = "opus")]
pub mod opus;

/// Codec failure. `code` is the backend's raw error (libopus code for Opus).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CodecError {
    /// Argument rejected (frame size, empty buffer, unsupported setting).
    BadArg,
    /// Output buffer too small.
    BufferTooSmall,
    /// Undecodable packet.
    InvalidPacket,
    /// Backend-specific failure.
    Backend(i32),
}

impl fmt::Display for CodecError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CodecError::BadArg => f.write_str("codec: bad argument"),
            CodecError::BufferTooSmall => f.write_str("codec: buffer too small"),
            CodecError::InvalidPacket => f.write_str("codec: invalid packet"),
            CodecError::Backend(c) => write!(f, "codec backend error {c}"),
        }
    }
}

impl std::error::Error for CodecError {}

/// One talker's encoder (16 kHz mono).
pub trait VoiceEncoder: Send {
    /// Applies `cfg` (called right after [`VoiceEncoder::reset`] at every burst start).
    fn configure(&mut self, cfg: &EncoderConfig) -> Result<(), CodecError>;
    /// Settings currently applied.
    fn config(&self) -> &EncoderConfig;
    /// Clears the codec history (settings are kept).
    fn reset(&mut self) -> Result<(), CodecError>;
    /// Encodes exactly one frame (`pcm.len()` = the configured frame size) into `out`.
    /// Returns the payload length (1..=`out.len()`).
    fn encode(&mut self, pcm: &[i16], out: &mut [u8]) -> Result<usize, CodecError>;
    /// Algorithmic delay in samples (encoder look-ahead plus decoder delay).
    fn lookahead(&self) -> usize;
}

/// One receive stream's decoder (16 kHz mono).
pub trait VoiceDecoder: Send {
    /// Decodes `packet` into `out` (its length is the frame size). Returns samples written.
    fn decode(&mut self, packet: &[u8], out: &mut [i16]) -> Result<usize, CodecError>;
    /// Recovers the frame *before* `next` from `next`'s in-band FEC (LBRR). Falls back to
    /// concealment when `next` carries none.
    fn decode_fec(&mut self, next: &[u8], out: &mut [i16]) -> Result<usize, CodecError>;
    /// Packet-loss concealment for one missing frame of `out.len()` samples.
    fn conceal(&mut self, out: &mut [i16]) -> Result<usize, CodecError>;
    /// Clears the decoder history (new burst, or a burst ended).
    fn reset(&mut self) -> Result<(), CodecError>;
}

/// A codec family: creates encoders and decoders.
pub trait Codec: Send + Sync {
    /// Short identifier ("opus", "sim") for diagnostics and CAPS.
    fn name(&self) -> &'static str;
    /// Creates an encoder (control path; may allocate).
    fn new_encoder(&self) -> Result<Box<dyn VoiceEncoder>, CodecError>;
    /// Creates a decoder (control path; may allocate).
    fn new_decoder(&self) -> Result<Box<dyn VoiceDecoder>, CodecError>;
}
