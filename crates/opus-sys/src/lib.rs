//! libopus 1.6.1 built with `cc` and a fixed-arity ctl shim (owned by WP-06,
//! DESIGN §2.3, §3.1, §7.4).
//!
//! * [`ffi`]: hand-written `extern "C"` declarations (about 25 libopus symbols plus the
//!   `ms_*` shim). No bindgen and no libclang.
//! * [`Encoder`] / [`Decoder`]: safe wrappers with one setter and one getter per ctl the
//!   voice pipeline uses (`mesh-voice`, feature `opus`).
//! * The crate compiles to nothing on `wasm32` (the web build uses `SimCodec`; DESIGN §3.1).
//!
//! # Build
//!
//! `build.rs` compiles the sources fetched by `cargo xtask fetch-opus` (SHA-256 pinned in
//! `third_party/opus/SHA256SUMS`) and refuses to build without its `.ok` stamp. Float
//! build with deep PLC, DRED and OSCE; NEON presumed on aarch64, SSE2 on x86_64. On the
//! host, `cargo test -p opus-sys` also links and runs upstream `test_opus_api`,
//! `test_opus_decode` and `test_opus_encode`. `cargo xtask opus-smoke` cross-builds the
//! crate for aarch64-apple-ios, aarch64-apple-ios-sim, aarch64-linux-android and
//! x86_64-linux-android.
//!
//! # Fallback (documented, not the default; DESIGN §2.3 step 7, risk A2)
//!
//! If the `cc` build ever breaks on a target, `mesh-voice` can switch its `opus` feature
//! to `opusic-sys = { version = "=0.7.5", features = ["dred", "osce"] }`, which builds
//! the same libopus through CMake (via the `cmake` crate). No system CMake is needed:
//! `source scripts/env.sh` puts the Android SDK's CMake 3.22.1 (with ninja) first on
//! `PATH`. Keep the fixed-arity ctl wrappers when swapping (compile `src/shim.c` with `cc`
//! against the same `include/` headers) instead of declaring the variadic
//! `opus_encoder_ctl` in Rust: the Apple arm64 ABI passes variadic arguments on the
//! stack. This path is documented but not exercised by CI; after a swap, re-run
//! `cargo xtask opus-smoke` (and this crate's ctl round-trip tests) to validate it.

#![cfg(not(target_arch = "wasm32"))]

mod decoder;
mod encoder;
mod error;
#[allow(non_camel_case_types)]
pub mod ffi;

pub use decoder::Decoder;
pub use encoder::{Encoder, MAX_DRED_DURATION};
pub use error::{Error, Result};

use core::ffi::CStr;

/// Channel layout.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Channels {
    Mono = 1,
    Stereo = 2,
}

impl Channels {
    pub const fn count(self) -> usize {
        self as usize
    }
}

/// Encoder application mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Application {
    /// Speech-optimised (the only one mesh-app uses, DESIGN §7.4).
    Voip,
    Audio,
    RestrictedLowDelay,
}

impl Application {
    const fn raw(self) -> i32 {
        match self {
            Application::Voip => ffi::OPUS_APPLICATION_VOIP,
            Application::Audio => ffi::OPUS_APPLICATION_AUDIO,
            Application::RestrictedLowDelay => ffi::OPUS_APPLICATION_RESTRICTED_LOWDELAY,
        }
    }
}

/// Audio bandwidth (`OPUS_SET_MAX_BANDWIDTH`, `opus_packet_get_bandwidth`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Bandwidth {
    Narrowband,
    Mediumband,
    Wideband,
    Superwideband,
    Fullband,
}

impl Bandwidth {
    pub const fn raw(self) -> i32 {
        match self {
            Bandwidth::Narrowband => ffi::OPUS_BANDWIDTH_NARROWBAND,
            Bandwidth::Mediumband => ffi::OPUS_BANDWIDTH_MEDIUMBAND,
            Bandwidth::Wideband => ffi::OPUS_BANDWIDTH_WIDEBAND,
            Bandwidth::Superwideband => ffi::OPUS_BANDWIDTH_SUPERWIDEBAND,
            Bandwidth::Fullband => ffi::OPUS_BANDWIDTH_FULLBAND,
        }
    }

    pub const fn from_raw(raw: i32) -> Option<Self> {
        Some(match raw {
            ffi::OPUS_BANDWIDTH_NARROWBAND => Bandwidth::Narrowband,
            ffi::OPUS_BANDWIDTH_MEDIUMBAND => Bandwidth::Mediumband,
            ffi::OPUS_BANDWIDTH_WIDEBAND => Bandwidth::Wideband,
            ffi::OPUS_BANDWIDTH_SUPERWIDEBAND => Bandwidth::Superwideband,
            ffi::OPUS_BANDWIDTH_FULLBAND => Bandwidth::Fullband,
            _ => return None,
        })
    }
}

/// Signal-type hint (`OPUS_SET_SIGNAL`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Signal {
    Auto,
    Voice,
    Music,
}

impl Signal {
    const fn raw(self) -> i32 {
        match self {
            Signal::Auto => ffi::OPUS_AUTO,
            Signal::Voice => ffi::OPUS_SIGNAL_VOICE,
            Signal::Music => ffi::OPUS_SIGNAL_MUSIC,
        }
    }

    const fn from_raw(raw: i32) -> Option<Self> {
        Some(match raw {
            ffi::OPUS_AUTO => Signal::Auto,
            ffi::OPUS_SIGNAL_VOICE => Signal::Voice,
            ffi::OPUS_SIGNAL_MUSIC => Signal::Music,
            _ => return None,
        })
    }
}

/// Rate-control mode: `OPUS_SET_VBR` plus `OPUS_SET_VBR_CONSTRAINT`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Vbr {
    /// Hard CBR (`VBR = 0`).
    Off,
    /// VBR with constraint off.
    Unconstrained,
    /// Constrained VBR (DESIGN §7.4 common setting).
    Constrained,
}

/// Bitrate setting (`OPUS_SET_BITRATE`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Bitrate {
    Auto,
    Max,
    BitsPerSecond(i32),
}

impl Bitrate {
    const fn raw(self) -> i32 {
        match self {
            Bitrate::Auto => ffi::OPUS_AUTO,
            Bitrate::Max => ffi::OPUS_BITRATE_MAX,
            Bitrate::BitsPerSecond(b) => b,
        }
    }
}

/// libopus version string, e.g. `"libopus 1.6.1"`.
pub fn version() -> &'static str {
    // SAFETY: libopus returns a pointer to a static NUL-terminated string.
    let s = unsafe { CStr::from_ptr(ffi::opus_get_version_string()) };
    s.to_str().unwrap_or("libopus (non-UTF-8 version)")
}

/// Number of samples per channel in `packet` at sample rate `fs`.
pub fn packet_nb_samples(packet: &[u8], fs: u32) -> Result<usize> {
    let len = packet_len(packet)?;
    // SAFETY: `packet` is valid for `len` bytes; libopus only reads it.
    let n = unsafe { ffi::opus_packet_get_nb_samples(packet.as_ptr(), len, fs as i32) };
    error::check_count(n)
}

/// Bandwidth signalled in the TOC byte of `packet`.
pub fn packet_bandwidth(packet: &[u8]) -> Result<Bandwidth> {
    if packet.is_empty() {
        return Err(Error::from_code(ffi::OPUS_BAD_ARG));
    }
    // SAFETY: the packet has at least the TOC byte, which is all libopus reads.
    let raw = unsafe { ffi::opus_packet_get_bandwidth(packet.as_ptr()) };
    Bandwidth::from_raw(raw).ok_or(Error::from_code(raw.min(ffi::OPUS_INVALID_PACKET)))
}

/// True when `packet` carries SILK in-band FEC (LBRR) for the previous frame.
pub fn packet_has_lbrr(packet: &[u8]) -> Result<bool> {
    let len = packet_len(packet)?;
    // SAFETY: `packet` is valid for `len` bytes; libopus only reads it.
    let r = unsafe { ffi::opus_packet_has_lbrr(packet.as_ptr(), len) };
    error::check_count(r).map(|n| n != 0)
}

fn packet_len(packet: &[u8]) -> Result<i32> {
    if packet.is_empty() {
        return Err(Error::from_code(ffi::OPUS_BAD_ARG));
    }
    i32::try_from(packet.len()).map_err(|_| Error::from_code(ffi::OPUS_BAD_ARG))
}
