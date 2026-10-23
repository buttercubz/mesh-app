//! Hand-written declarations of the libopus 1.6.1 API used by mesh-app, plus the
//! fixed-arity ctl shim in `shim.c` (no bindgen, no libclang; DESIGN §2.3 step 4).
//!
//! The variadic `opus_encoder_ctl` / `opus_decoder_ctl` are deliberately NOT declared:
//! every ctl goes through a `ms_*` wrapper with a fixed prototype.

use core::ffi::{c_char, c_int, c_uchar};

/// Opaque `OpusEncoder`.
#[repr(C)]
pub struct OpusEncoder {
    _private: [u8; 0],
}

/// Opaque `OpusDecoder`.
#[repr(C)]
pub struct OpusDecoder {
    _private: [u8; 0],
}

pub type opus_int16 = i16;
pub type opus_int32 = i32;

// Error codes (opus_defines.h).
pub const OPUS_OK: c_int = 0;
pub const OPUS_BAD_ARG: c_int = -1;
pub const OPUS_BUFFER_TOO_SMALL: c_int = -2;
pub const OPUS_INTERNAL_ERROR: c_int = -3;
pub const OPUS_INVALID_PACKET: c_int = -4;
pub const OPUS_UNIMPLEMENTED: c_int = -5;
pub const OPUS_INVALID_STATE: c_int = -6;
pub const OPUS_ALLOC_FAIL: c_int = -7;

// Applications.
pub const OPUS_APPLICATION_VOIP: c_int = 2048;
pub const OPUS_APPLICATION_AUDIO: c_int = 2049;
pub const OPUS_APPLICATION_RESTRICTED_LOWDELAY: c_int = 2051;

// Special values.
pub const OPUS_AUTO: opus_int32 = -1000;
pub const OPUS_BITRATE_MAX: opus_int32 = -1;

// Signal hints.
pub const OPUS_SIGNAL_VOICE: opus_int32 = 3001;
pub const OPUS_SIGNAL_MUSIC: opus_int32 = 3002;

// Bandwidths.
pub const OPUS_BANDWIDTH_NARROWBAND: opus_int32 = 1101;
pub const OPUS_BANDWIDTH_MEDIUMBAND: opus_int32 = 1102;
pub const OPUS_BANDWIDTH_WIDEBAND: opus_int32 = 1103;
pub const OPUS_BANDWIDTH_SUPERWIDEBAND: opus_int32 = 1104;
pub const OPUS_BANDWIDTH_FULLBAND: opus_int32 = 1105;

unsafe extern "C" {
    // --- library info ---
    pub fn opus_get_version_string() -> *const c_char;
    pub fn opus_strerror(error: c_int) -> *const c_char;

    // --- encoder ---
    pub fn opus_encoder_get_size(channels: c_int) -> c_int;
    pub fn opus_encoder_create(
        fs: opus_int32,
        channels: c_int,
        application: c_int,
        error: *mut c_int,
    ) -> *mut OpusEncoder;
    pub fn opus_encoder_destroy(st: *mut OpusEncoder);
    pub fn opus_encode(
        st: *mut OpusEncoder,
        pcm: *const opus_int16,
        frame_size: c_int,
        data: *mut c_uchar,
        max_data_bytes: opus_int32,
    ) -> opus_int32;

    // --- decoder ---
    pub fn opus_decoder_get_size(channels: c_int) -> c_int;
    pub fn opus_decoder_create(
        fs: opus_int32,
        channels: c_int,
        error: *mut c_int,
    ) -> *mut OpusDecoder;
    pub fn opus_decoder_destroy(st: *mut OpusDecoder);
    pub fn opus_decode(
        st: *mut OpusDecoder,
        data: *const c_uchar,
        len: opus_int32,
        pcm: *mut opus_int16,
        frame_size: c_int,
        decode_fec: c_int,
    ) -> c_int;

    // --- packet inspection ---
    pub fn opus_packet_get_bandwidth(data: *const c_uchar) -> c_int;
    pub fn opus_packet_get_nb_channels(data: *const c_uchar) -> c_int;
    pub fn opus_packet_get_nb_frames(packet: *const c_uchar, len: opus_int32) -> c_int;
    pub fn opus_packet_get_nb_samples(
        packet: *const c_uchar,
        len: opus_int32,
        fs: opus_int32,
    ) -> c_int;
    pub fn opus_packet_has_lbrr(packet: *const c_uchar, len: opus_int32) -> c_int;

    // --- fixed-arity ctl shim (src/shim.c) ---
    pub fn ms_enc_set_bitrate(st: *mut OpusEncoder, value: opus_int32) -> c_int;
    pub fn ms_enc_get_bitrate(st: *mut OpusEncoder, value: *mut opus_int32) -> c_int;
    pub fn ms_enc_set_complexity(st: *mut OpusEncoder, value: opus_int32) -> c_int;
    pub fn ms_enc_get_complexity(st: *mut OpusEncoder, value: *mut opus_int32) -> c_int;
    pub fn ms_enc_set_vbr(st: *mut OpusEncoder, value: opus_int32) -> c_int;
    pub fn ms_enc_get_vbr(st: *mut OpusEncoder, value: *mut opus_int32) -> c_int;
    pub fn ms_enc_set_vbr_constraint(st: *mut OpusEncoder, value: opus_int32) -> c_int;
    pub fn ms_enc_get_vbr_constraint(st: *mut OpusEncoder, value: *mut opus_int32) -> c_int;
    pub fn ms_enc_set_inband_fec(st: *mut OpusEncoder, value: opus_int32) -> c_int;
    pub fn ms_enc_get_inband_fec(st: *mut OpusEncoder, value: *mut opus_int32) -> c_int;
    pub fn ms_enc_set_packet_loss_perc(st: *mut OpusEncoder, value: opus_int32) -> c_int;
    pub fn ms_enc_get_packet_loss_perc(st: *mut OpusEncoder, value: *mut opus_int32) -> c_int;
    pub fn ms_enc_set_dtx(st: *mut OpusEncoder, value: opus_int32) -> c_int;
    pub fn ms_enc_get_dtx(st: *mut OpusEncoder, value: *mut opus_int32) -> c_int;
    pub fn ms_enc_set_max_bandwidth(st: *mut OpusEncoder, value: opus_int32) -> c_int;
    pub fn ms_enc_get_max_bandwidth(st: *mut OpusEncoder, value: *mut opus_int32) -> c_int;
    pub fn ms_enc_set_signal(st: *mut OpusEncoder, value: opus_int32) -> c_int;
    pub fn ms_enc_get_signal(st: *mut OpusEncoder, value: *mut opus_int32) -> c_int;
    pub fn ms_enc_set_dred_duration(st: *mut OpusEncoder, value: opus_int32) -> c_int;
    pub fn ms_enc_get_dred_duration(st: *mut OpusEncoder, value: *mut opus_int32) -> c_int;
    pub fn ms_get_lookahead(st: *mut OpusEncoder, value: *mut opus_int32) -> c_int;
    pub fn ms_enc_reset(st: *mut OpusEncoder) -> c_int;
    pub fn ms_dec_set_complexity(st: *mut OpusDecoder, value: opus_int32) -> c_int;
    pub fn ms_dec_get_complexity(st: *mut OpusDecoder, value: *mut opus_int32) -> c_int;
    pub fn ms_dec_get_last_packet_duration(st: *mut OpusDecoder, value: *mut opus_int32) -> c_int;
    pub fn ms_dec_reset(st: *mut OpusDecoder) -> c_int;
}
