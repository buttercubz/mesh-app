/*
 * Fixed-arity wrappers around the variadic opus_encoder_ctl / opus_decoder_ctl
 * (owned by WP-06, DESIGN §2.3 step 3).
 *
 * Rust never calls a variadic C function: the Apple arm64 ABI passes variadic
 * arguments on the stack while a non-variadic prototype passes them in registers,
 * so a Rust `extern "C" fn(..., ...)` declaration would silently pass garbage there.
 * Every ctl the engine uses gets a setter and a getter (the getter lets the tests
 * prove each setting round-trips). All functions return the libopus error code
 * (OPUS_OK == 0 on success).
 */

#include "opus.h"
#include "opus_defines.h"

#define MS_ENC_SET(name, REQ)                                                  \
    int ms_enc_set_##name(OpusEncoder *st, opus_int32 value) {                 \
        return opus_encoder_ctl(st, REQ(value));                               \
    }
#define MS_ENC_GET(name, REQ)                                                  \
    int ms_enc_get_##name(OpusEncoder *st, opus_int32 *value) {                \
        return opus_encoder_ctl(st, REQ(value));                               \
    }

MS_ENC_SET(bitrate, OPUS_SET_BITRATE)
MS_ENC_GET(bitrate, OPUS_GET_BITRATE)
MS_ENC_SET(complexity, OPUS_SET_COMPLEXITY)
MS_ENC_GET(complexity, OPUS_GET_COMPLEXITY)
MS_ENC_SET(vbr, OPUS_SET_VBR)
MS_ENC_GET(vbr, OPUS_GET_VBR)
MS_ENC_SET(vbr_constraint, OPUS_SET_VBR_CONSTRAINT)
MS_ENC_GET(vbr_constraint, OPUS_GET_VBR_CONSTRAINT)
MS_ENC_SET(inband_fec, OPUS_SET_INBAND_FEC)
MS_ENC_GET(inband_fec, OPUS_GET_INBAND_FEC)
MS_ENC_SET(packet_loss_perc, OPUS_SET_PACKET_LOSS_PERC)
MS_ENC_GET(packet_loss_perc, OPUS_GET_PACKET_LOSS_PERC)
MS_ENC_SET(dtx, OPUS_SET_DTX)
MS_ENC_GET(dtx, OPUS_GET_DTX)
MS_ENC_SET(max_bandwidth, OPUS_SET_MAX_BANDWIDTH)
MS_ENC_GET(max_bandwidth, OPUS_GET_MAX_BANDWIDTH)
MS_ENC_SET(signal, OPUS_SET_SIGNAL)
MS_ENC_GET(signal, OPUS_GET_SIGNAL)
MS_ENC_SET(dred_duration, OPUS_SET_DRED_DURATION)
MS_ENC_GET(dred_duration, OPUS_GET_DRED_DURATION)

/* Algorithmic delay of the encoder in samples at its API rate. */
int ms_get_lookahead(OpusEncoder *st, opus_int32 *value) {
    return opus_encoder_ctl(st, OPUS_GET_LOOKAHEAD(value));
}

/* Resets the encoder state (done at every burst start, DESIGN §7.4). */
int ms_enc_reset(OpusEncoder *st) {
    return opus_encoder_ctl(st, OPUS_RESET_STATE);
}

/* Decoder complexity gates deep PLC (>= 5) and OSCE LACE/NoLACE (>= 6/7). */
int ms_dec_set_complexity(OpusDecoder *st, opus_int32 value) {
    return opus_decoder_ctl(st, OPUS_SET_COMPLEXITY(value));
}

int ms_dec_get_complexity(OpusDecoder *st, opus_int32 *value) {
    return opus_decoder_ctl(st, OPUS_GET_COMPLEXITY(value));
}

/* Duration in samples of the last decoded (or concealed) packet. */
int ms_dec_get_last_packet_duration(OpusDecoder *st, opus_int32 *value) {
    return opus_decoder_ctl(st, OPUS_GET_LAST_PACKET_DURATION(value));
}

int ms_dec_reset(OpusDecoder *st) {
    return opus_decoder_ctl(st, OPUS_RESET_STATE);
}
