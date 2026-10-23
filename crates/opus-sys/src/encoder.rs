//! Safe `OpusEncoder` wrapper. Every ctl goes through the fixed-arity `ms_enc_*` shim.

use core::ptr::NonNull;

use crate::error::{self, Error, Result};
use crate::{Application, Bandwidth, Bitrate, Channels, Signal, Vbr, ffi};

/// Largest DRED duration libopus accepts, in 10 ms units (`DRED_MAX_FRAMES`).
pub const MAX_DRED_DURATION: u16 = 104;

/// An owned libopus encoder (one per talking origin; DESIGN §7.4).
///
/// The state is plain heap memory owned by this value, so it may move between threads
/// (`Send`), but it is not shareable (`!Sync`): every call mutates it.
#[derive(Debug)]
pub struct Encoder {
    st: NonNull<ffi::OpusEncoder>,
    sample_rate: u32,
    channels: Channels,
}

// SAFETY: an OpusEncoder has no thread affinity and no interior sharing; `&mut self` on
// every call serialises access.
unsafe impl Send for Encoder {}

impl Encoder {
    /// Creates an encoder. `sample_rate` must be 8000, 12000, 16000, 24000 or 48000 Hz
    /// (mesh-app uses 16000, mono, [`Application::Voip`]).
    pub fn new(sample_rate: u32, channels: Channels, application: Application) -> Result<Self> {
        let fs = i32::try_from(sample_rate).map_err(|_| Error::from_code(ffi::OPUS_BAD_ARG))?;
        let mut err = ffi::OPUS_OK;
        // SAFETY: `err` is a valid out-pointer; libopus validates the other arguments.
        let st = unsafe {
            ffi::opus_encoder_create(fs, channels.count() as i32, application.raw(), &mut err)
        };
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

    fn set(
        &mut self,
        f: unsafe extern "C" fn(*mut ffi::OpusEncoder, i32) -> i32,
        v: i32,
    ) -> Result<()> {
        // SAFETY: `self.st` is a live encoder owned by `self`; the shim has a fixed prototype.
        error::check(unsafe { f(self.st.as_ptr(), v) })
    }

    fn get(&self, f: unsafe extern "C" fn(*mut ffi::OpusEncoder, *mut i32) -> i32) -> Result<i32> {
        let mut v = 0i32;
        // SAFETY: `self.st` is live and `v` is a valid out-pointer. GET ctls do not
        // modify the encoder state, so a shared borrow is sufficient.
        error::check(unsafe { f(self.st.as_ptr(), &mut v) })?;
        Ok(v)
    }

    /// `OPUS_SET_BITRATE` (bits per second, or auto/max).
    pub fn set_bitrate(&mut self, bitrate: Bitrate) -> Result<()> {
        self.set(ffi::ms_enc_set_bitrate, bitrate.raw())
    }

    /// `OPUS_GET_BITRATE`: the bitrate in use, in bits per second.
    pub fn bitrate(&self) -> Result<i32> {
        self.get(ffi::ms_enc_get_bitrate)
    }

    /// `OPUS_SET_COMPLEXITY` (0..=10).
    pub fn set_complexity(&mut self, complexity: u8) -> Result<()> {
        self.set(ffi::ms_enc_set_complexity, i32::from(complexity))
    }

    pub fn complexity(&self) -> Result<u8> {
        self.get(ffi::ms_enc_get_complexity).map(clamp_u8)
    }

    /// `OPUS_SET_VBR` and `OPUS_SET_VBR_CONSTRAINT` together.
    pub fn set_vbr(&mut self, vbr: Vbr) -> Result<()> {
        let (on, constrained) = match vbr {
            Vbr::Off => (0, 0),
            Vbr::Unconstrained => (1, 0),
            Vbr::Constrained => (1, 1),
        };
        self.set(ffi::ms_enc_set_vbr, on)?;
        self.set(ffi::ms_enc_set_vbr_constraint, constrained)
    }

    /// Current rate-control mode. With VBR off the constraint flag is irrelevant.
    pub fn vbr(&self) -> Result<Vbr> {
        if self.get(ffi::ms_enc_get_vbr)? == 0 {
            return Ok(Vbr::Off);
        }
        Ok(if self.get(ffi::ms_enc_get_vbr_constraint)? != 0 {
            Vbr::Constrained
        } else {
            Vbr::Unconstrained
        })
    }

    /// `OPUS_SET_INBAND_FEC` (SILK LBRR). Used only by the Wi-Fi 24 kbps profile.
    pub fn set_inband_fec(&mut self, on: bool) -> Result<()> {
        self.set(ffi::ms_enc_set_inband_fec, i32::from(on))
    }

    pub fn inband_fec(&self) -> Result<bool> {
        self.get(ffi::ms_enc_get_inband_fec).map(|v| v != 0)
    }

    /// `OPUS_SET_PACKET_LOSS_PERC` (0..=100). Must stay 0 whenever LBRR is off (§7.4).
    pub fn set_packet_loss_perc(&mut self, percent: u8) -> Result<()> {
        self.set(ffi::ms_enc_set_packet_loss_perc, i32::from(percent))
    }

    pub fn packet_loss_perc(&self) -> Result<u8> {
        self.get(ffi::ms_enc_get_packet_loss_perc).map(clamp_u8)
    }

    /// `OPUS_SET_DTX`. Off in mesh-app: the external VAD gates bursts (§7.4).
    pub fn set_dtx(&mut self, on: bool) -> Result<()> {
        self.set(ffi::ms_enc_set_dtx, i32::from(on))
    }

    pub fn dtx(&self) -> Result<bool> {
        self.get(ffi::ms_enc_get_dtx).map(|v| v != 0)
    }

    /// `OPUS_SET_MAX_BANDWIDTH`: follows the sender's capture route (NB on 8 kHz routes).
    pub fn set_max_bandwidth(&mut self, bw: Bandwidth) -> Result<()> {
        self.set(ffi::ms_enc_set_max_bandwidth, bw.raw())
    }

    pub fn max_bandwidth(&self) -> Result<Bandwidth> {
        let raw = self.get(ffi::ms_enc_get_max_bandwidth)?;
        Bandwidth::from_raw(raw).ok_or(Error::from_code(ffi::OPUS_INTERNAL_ERROR))
    }

    /// `OPUS_SET_SIGNAL`.
    pub fn set_signal(&mut self, signal: Signal) -> Result<()> {
        self.set(ffi::ms_enc_set_signal, signal.raw())
    }

    /// `OPUS_SET_SIGNAL(OPUS_SIGNAL_VOICE)`, the mesh-app setting.
    pub fn set_signal_voice(&mut self) -> Result<()> {
        self.set_signal(Signal::Voice)
    }

    pub fn signal(&self) -> Result<Signal> {
        let raw = self.get(ffi::ms_enc_get_signal)?;
        Signal::from_raw(raw).ok_or(Error::from_code(ffi::OPUS_INTERNAL_ERROR))
    }

    /// `OPUS_SET_DRED_DURATION` in 10 ms units (0 = off, the v1 default; max
    /// [`MAX_DRED_DURATION`]).
    pub fn set_dred_duration(&mut self, frames_10ms: u16) -> Result<()> {
        self.set(ffi::ms_enc_set_dred_duration, i32::from(frames_10ms))
    }

    pub fn dred_duration(&self) -> Result<u16> {
        let v = self.get(ffi::ms_enc_get_dred_duration)?;
        u16::try_from(v).map_err(|_| Error::from_code(ffi::OPUS_INTERNAL_ERROR))
    }

    /// `OPUS_GET_LOOKAHEAD`: algorithmic delay in samples at the API rate.
    pub fn lookahead(&self) -> Result<usize> {
        self.get(ffi::ms_get_lookahead).and_then(error::check_count)
    }

    /// `OPUS_RESET_STATE` (every burst start, §7.4). Settings are kept.
    pub fn reset(&mut self) -> Result<()> {
        // SAFETY: `self.st` is a live encoder owned by `self`.
        error::check(unsafe { ffi::ms_enc_reset(self.st.as_ptr()) })
    }

    /// Encodes one frame of interleaved PCM (`pcm.len()` = samples per channel ×
    /// channels; 2.5/5/10/20/40/60/80/100/120 ms) into `out`. Returns the packet length.
    pub fn encode(&mut self, pcm: &[i16], out: &mut [u8]) -> Result<usize> {
        let ch = self.channels.count();
        if pcm.is_empty() || !pcm.len().is_multiple_of(ch) || out.is_empty() {
            return Err(Error::from_code(ffi::OPUS_BAD_ARG));
        }
        let frame =
            i32::try_from(pcm.len() / ch).map_err(|_| Error::from_code(ffi::OPUS_BAD_ARG))?;
        let cap = i32::try_from(out.len()).unwrap_or(i32::MAX);
        // SAFETY: `pcm` holds `frame * ch` samples and `out` is writable for `cap` bytes.
        let n = unsafe {
            ffi::opus_encode(self.st.as_ptr(), pcm.as_ptr(), frame, out.as_mut_ptr(), cap)
        };
        error::check_count(n)
    }
}

impl Drop for Encoder {
    fn drop(&mut self) {
        // SAFETY: created by opus_encoder_create and destroyed exactly once.
        unsafe { ffi::opus_encoder_destroy(self.st.as_ptr()) }
    }
}

fn clamp_u8(v: i32) -> u8 {
    u8::try_from(v.max(0)).unwrap_or(u8::MAX)
}
