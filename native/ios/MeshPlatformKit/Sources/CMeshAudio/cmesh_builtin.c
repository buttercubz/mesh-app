/*
 * Built-in sinks used when no external (Rust) sink is registered: silence, meter,
 * loopback, tone and the T5a click train with capture recording (DESIGN §10.1.1, §12.2).
 *
 * RT side (push/pull) uses only atomics and static buffers. Reconfiguration from the
 * control thread posts a reset request that each RT leg applies on its next callback, so
 * the two threads never write the same state.
 */
#include <math.h>
#include <stdatomic.h>
#include <stdlib.h>
#include <string.h>

#include "cmesh_internal.h"

#define LOOP_CAP 8192u /* power of two, 512 ms at 16 kHz */
#define CLICK_LEN 32u  /* 2 ms burst */

static atomic_uint g_kind = CMESH_BUILTIN_SILENCE;

/* ---- loopback (SPSC ring: capture thread produces, render thread consumes) ---------- */
static int16_t g_loop[LOOP_CAP];
static atomic_uint g_loop_head = 0; /* written by capture */
static atomic_uint g_loop_tail = 0; /* written by render */
static atomic_uint g_loop_reset = 0;

/* ---- tone ---------------------------------------------------------------------------- */
static atomic_uint g_tone_hz_bits = 0x447A0000u; /* 1000.0f */
static atomic_uint g_tone_db_bits = 0xC1A00000u; /* -20.0f dBFS */
static double g_tone_phase = 0.0; /* render thread only */

/* ---- click train (T5a) --------------------------------------------------------------- */
static atomic_uint g_click_period = 8000u;
static atomic_uint g_click_capture_max = CMESH_CLICK_MAX_CAPTURE;
static atomic_uint g_click_db_bits = 0xC0C00000u; /* -6.0f dBFS */
static atomic_uint g_click_reset_render = 0;
static atomic_uint g_click_reset_capture = 0;
static uint64_t g_click_render_pos = 0; /* render thread only */
static int16_t g_click_capture[CMESH_CLICK_MAX_CAPTURE];
static atomic_uint g_click_captured = 0;
static uint32_t g_click_events[CMESH_CLICK_MAX_EVENTS];
static atomic_uint g_click_emitted = 0;

void cmesh_use_builtin(cmesh_builtin_t kind) {
    atomic_store(&g_loop_reset, 1);
    atomic_store(&g_click_reset_render, 1);
    atomic_store(&g_click_reset_capture, 1);
    atomic_store(&g_kind, (unsigned)kind);
}

cmesh_builtin_t cmesh_builtin(void) { return (cmesh_builtin_t)atomic_load(&g_kind); }

void cmesh_tone_configure(float frequency_hz, float level_dbfs) {
    atomic_store(&g_tone_hz_bits, cmesh_f2u(frequency_hz));
    atomic_store(&g_tone_db_bits, cmesh_f2u(level_dbfs));
}

void cmesh_click_configure(uint32_t period_frames, uint32_t capture_frames, float level_dbfs) {
    if (period_frames < CLICK_LEN * 2u) {
        period_frames = CLICK_LEN * 2u;
    }
    if (capture_frames > CMESH_CLICK_MAX_CAPTURE) {
        capture_frames = CMESH_CLICK_MAX_CAPTURE;
    }
    atomic_store(&g_click_period, period_frames);
    atomic_store(&g_click_capture_max, capture_frames);
    atomic_store(&g_click_db_bits, cmesh_f2u(level_dbfs));
    atomic_store(&g_click_reset_render, 1);
    atomic_store(&g_click_reset_capture, 1);
}

uint32_t cmesh_click_captured_frames(void) {
    return atomic_load(&g_click_reset_capture) != 0 ? 0 : atomic_load(&g_click_captured);
}
uint32_t cmesh_click_emitted(void) {
    return atomic_load(&g_click_reset_render) != 0 ? 0 : atomic_load(&g_click_emitted);
}

static int32_t loop_push(const int16_t *pcm, uint32_t frames) {
    unsigned head = atomic_load_explicit(&g_loop_head, memory_order_relaxed);
    unsigned tail = atomic_load_explicit(&g_loop_tail, memory_order_acquire);
    unsigned room = LOOP_CAP - (head - tail);
    uint32_t n = frames < room ? frames : room; /* overrun: newest samples dropped */
    for (uint32_t i = 0; i < n; i++) {
        g_loop[(head + i) & (LOOP_CAP - 1u)] = pcm[i];
    }
    atomic_store_explicit(&g_loop_head, head + n, memory_order_release);
    return (int32_t)n;
}

static int32_t loop_pull(int16_t *pcm, uint32_t frames) {
    if (atomic_exchange(&g_loop_reset, 0) != 0) {
        atomic_store(&g_loop_tail, atomic_load(&g_loop_head));
    }
    unsigned tail = atomic_load_explicit(&g_loop_tail, memory_order_relaxed);
    unsigned head = atomic_load_explicit(&g_loop_head, memory_order_acquire);
    unsigned avail = head - tail;
    uint32_t n = frames < avail ? frames : avail;
    for (uint32_t i = 0; i < n; i++) {
        pcm[i] = g_loop[(tail + i) & (LOOP_CAP - 1u)];
    }
    if (n < frames) {
        memset(pcm + n, 0, (size_t)(frames - n) * sizeof(int16_t)); /* underrun → silence */
    }
    atomic_store_explicit(&g_loop_tail, tail + n, memory_order_release);
    return (int32_t)frames;
}

static int32_t tone_pull(int16_t *pcm, uint32_t frames) {
    float hz = cmesh_u2f(atomic_load(&g_tone_hz_bits));
    float amp = cmesh_dbfs_to_amplitude(cmesh_u2f(atomic_load(&g_tone_db_bits)));
    if (!(hz > 0.0f) || hz >= (float)CMESH_SAMPLE_RATE / 2.0f) {
        memset(pcm, 0, (size_t)frames * sizeof(int16_t));
        return (int32_t)frames;
    }
    double step = 2.0 * M_PI * (double)hz / (double)CMESH_SAMPLE_RATE;
    for (uint32_t i = 0; i < frames; i++) {
        pcm[i] = (int16_t)lrint((double)amp * sin(g_tone_phase));
        g_tone_phase += step;
        if (g_tone_phase >= 2.0 * M_PI) {
            g_tone_phase -= 2.0 * M_PI;
        }
    }
    return (int32_t)frames;
}

/* One 2 ms burst: 4 kHz carrier under a Hann window, sample `k` of CLICK_LEN. */
static int16_t click_sample(uint32_t k, float amp) {
    double w = 0.5 - 0.5 * cos(2.0 * M_PI * (double)k / (double)(CLICK_LEN - 1u));
    double c = sin(2.0 * M_PI * 4000.0 * (double)k / (double)CMESH_SAMPLE_RATE + M_PI / 4.0);
    return (int16_t)lrint((double)amp * w * c);
}

static int32_t click_pull(int16_t *pcm, uint32_t frames) {
    if (atomic_exchange(&g_click_reset_render, 0) != 0) {
        g_click_render_pos = 0;
        atomic_store(&g_click_emitted, 0);
    }
    uint32_t period = atomic_load(&g_click_period);
    float amp = cmesh_dbfs_to_amplitude(cmesh_u2f(atomic_load(&g_click_db_bits)));
    for (uint32_t i = 0; i < frames; i++) {
        uint64_t pos = g_click_render_pos + i;
        uint32_t k = (uint32_t)(pos % period);
        if (k == 0) {
            unsigned e = atomic_load_explicit(&g_click_emitted, memory_order_relaxed);
            if (e < CMESH_CLICK_MAX_EVENTS && pos <= UINT32_MAX) {
                g_click_events[e] = (uint32_t)pos;
                atomic_store_explicit(&g_click_emitted, e + 1, memory_order_release);
            }
        }
        pcm[i] = k < CLICK_LEN ? click_sample(k, amp) : 0;
    }
    g_click_render_pos += frames;
    return (int32_t)frames;
}

static int32_t click_push(const int16_t *pcm, uint32_t frames) {
    if (atomic_exchange(&g_click_reset_capture, 0) != 0) {
        atomic_store(&g_click_captured, 0);
    }
    unsigned have = atomic_load_explicit(&g_click_captured, memory_order_relaxed);
    unsigned max = atomic_load(&g_click_capture_max);
    uint32_t n = have >= max ? 0 : (frames < max - have ? frames : max - have);
    memcpy(g_click_capture + have, pcm, (size_t)n * sizeof(int16_t));
    atomic_store_explicit(&g_click_captured, have + n, memory_order_release);
    return (int32_t)frames;
}

int32_t cmesh_builtin_push(const int16_t *pcm, uint32_t frames) {
    switch ((cmesh_builtin_t)atomic_load(&g_kind)) {
    case CMESH_BUILTIN_LOOPBACK:
        return loop_push(pcm, frames);
    case CMESH_BUILTIN_CLICK:
        return click_push(pcm, frames);
    case CMESH_BUILTIN_SILENCE:
    case CMESH_BUILTIN_METER:
    case CMESH_BUILTIN_TONE:
    default:
        return (int32_t)frames; /* metered by the dispatcher, then discarded */
    }
}

int32_t cmesh_builtin_pull(int16_t *pcm, uint32_t frames) {
    switch ((cmesh_builtin_t)atomic_load(&g_kind)) {
    case CMESH_BUILTIN_LOOPBACK:
        return loop_pull(pcm, frames);
    case CMESH_BUILTIN_TONE:
        return tone_pull(pcm, frames);
    case CMESH_BUILTIN_CLICK:
        return click_pull(pcm, frames);
    case CMESH_BUILTIN_SILENCE:
    case CMESH_BUILTIN_METER:
    default:
        memset(pcm, 0, (size_t)frames * sizeof(int16_t));
        return (int32_t)frames;
    }
}

static int cmp_i32(const void *a, const void *b) {
    int32_t x = *(const int32_t *)a, y = *(const int32_t *)b;
    return (x > y) - (x < y);
}

int32_t cmesh_click_estimate_latency_frames(float threshold_dbfs, uint32_t *found_out) {
    static int32_t lags[CMESH_CLICK_MAX_EVENTS]; /* control thread only */
    if (found_out != NULL) {
        *found_out = 0;
    }
    if (atomic_load(&g_click_reset_render) != 0 || atomic_load(&g_click_reset_capture) != 0) {
        return -1; /* reconfigured, the RT side has not started the new run yet */
    }
    unsigned emitted = atomic_load_explicit(&g_click_emitted, memory_order_acquire);
    unsigned captured = atomic_load_explicit(&g_click_captured, memory_order_acquire);
    uint32_t period = atomic_load(&g_click_period);
    float thr = cmesh_dbfs_to_amplitude(threshold_dbfs);
    uint32_t found = 0;
    for (unsigned e = 0; e < emitted && e < CMESH_CLICK_MAX_EVENTS; e++) {
        uint32_t start = g_click_events[e];
        uint64_t end = (uint64_t)start + period;
        if (end > captured) {
            end = captured;
        }
        for (uint64_t i = start; i < end; i++) {
            if ((float)abs((int)g_click_capture[i]) >= thr) {
                lags[found++] = (int32_t)(i - start);
                break;
            }
        }
    }
    if (found_out != NULL) {
        *found_out = found;
    }
    if (found == 0) {
        return -1;
    }
    qsort(lags, found, sizeof(int32_t), cmp_i32);
    return lags[found / 2];
}
