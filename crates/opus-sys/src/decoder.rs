//! Safe `OpusDecoder` wrapper. Every ctl goes through the fixed-arity `ms_dec_*` shim.

use core::ptr::NonNull;

use crate::error::{self, Error, Result};
use crate::{Channels, ffi};

/// An owned libopus decoder (a pool of six per engine, DESIGN §7.5).
///
/// `Send` but not `Sync`, like [`crate::Encoder`].
#[derive(Debug)]
pub struct Decoder {
    st: NonNull<ffi::OpusDecoder>,
    sample_rate: u32,
    channels: Channels,
}

// SAFETY: an OpusDecoder has no thread affinity; `&mut self` serialises every mutation.
unsafe impl Send for Decoder {}

impl Decoder {
    /// Creates a decoder at `sample_rate` (8000, 12000, 16000, 24000 or 48000 Hz).
    pub fn new(sample_rate: u32, channels: Channels) -> Result<Self> {
        let fs = i32::try_from(sample_rate).map_err(|_| Error::from_code(ffi::OPUS_BAD_ARG))?;
        let mut err = ffi::OPUS_OK;
        // SAFETY: `err` is a valid out-pointer; libopus validates the other arguments.
        let st = unsafe { ffi::opus_decoder_create(fs, channels.count() as i32, &mut err) };
        error::check(err)?;
        let st = NonNull::new(st).ok_or(Error::from_code(ffi::OPUS_ALLOC_FAIL))?;
        Ok(Self {
            st,
            sample_rate,
            channels,
        })
    }

    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    pub fn channels(&self) -> Channels {
        self.channels
    }

    /// Decodes into `out` (interleaved; its length sets the frame size, which must be a
    /// multiple of 2.5 ms). Returns the number of samples per channel written.
    ///
    /// * `packet = Some(p), fec = false`: normal decode of `p`.
    /// * `packet = Some(p), fec = true`: recovers the *previous* frame from `p`'s LBRR
    ///   data (falls back to PLC when `p` carries none).
    /// * `packet = None`: packet-loss concealment (deep PLC at complexity ≥ 5).
    pub fn decode(&mut self, packet: Option<&[u8]>, out: &mut [i16], fec: bool) -> Result<usize> {
        let ch = self.channels.count();
        if out.is_empty() || !out.len().is_multiple_of(ch) {
            return Err(Error::from_code(ffi::OPUS_BAD_ARG));
        }
        let frame =
            i32::try_from(out.len() / ch).map_err(|_| Error::from_code(ffi::OPUS_BAD_ARG))?;
        let (ptr, len) = match packet {
            Some([]) => return Err(Error::from_code(ffi::OPUS_BAD_ARG)),
            Some(p) => (
                p.as_ptr(),
                i32::try_from(p.len()).map_err(|_| Error::from_code(ffi::OPUS_BAD_ARG))?,
            ),
            None => (core::ptr::null(), 0),
        };
        // SAFETY: `ptr` is null (PLC) or valid for `len` bytes; `out` holds `frame * ch`
        // writable samples.
        let n = unsafe {
            ffi::opus_decode(
                self.st.as_ptr(),
                ptr,
                len,
                out.as_mut_ptr(),
                frame,
                i32::from(fec),
            )
        };
        error::check_count(n)
    }

    /// `OPUS_SET_COMPLEXITY` on the decoder (0..=10): ≥ 5 enables deep PLC, ≥ 6 LACE and
    /// ≥ 7 NoLACE (OSCE).
    pub fn set_complexity(&mut self, complexity: u8) -> Result<()> {
        // SAFETY: `self.st` is a live decoder owned by `self`.
        error::check(unsafe { ffi::ms_dec_set_complexity(self.st.as_ptr(), i32::from(complexity)) })
    }

    pub fn complexity(&self) -> Result<u8> {
        let mut v = 0i32;
        // SAFETY: `self.st` is live and `v` is a valid out-pointer.
        error::check(unsafe { ffi::ms_dec_get_complexity(self.st.as_ptr(), &mut v) })?;
        Ok(u8::try_from(v.max(0)).unwrap_or(u8::MAX))
    }

    /// `OPUS_GET_LAST_PACKET_DURATION`: samples per channel of the last decode or PLC call.
    pub fn last_packet_duration(&self) -> Result<usize> {
        let mut v = 0i32;
        // SAFETY: `self.st` is live and `v` is a valid out-pointer.
        error::check(unsafe { ffi::ms_dec_get_last_packet_duration(self.st.as_ptr(), &mut v) })?;
        error::check_count(v)
    }

    /// `OPUS_RESET_STATE` (a new burst from the same origin). Complexity is kept.
    pub fn reset(&mut self) -> Result<()> {
        // SAFETY: `self.st` is a live decoder owned by `self`.
        error::check(unsafe { ffi::ms_dec_reset(self.st.as_ptr()) })
    }
}

impl Drop for Decoder {
    fn drop(&mut self) {
        // SAFETY: created by opus_decoder_create and destroyed exactly once.
        unsafe { ffi::opus_decoder_destroy(self.st.as_ptr()) }
    }
}
