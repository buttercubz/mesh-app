/*
 * Sink registry, capture gate, RT flags, IO counters and the two dispatch entry points
 * called by the AudioUnit callbacks. Everything reachable from cmesh_dispatch_* is
 * wait-free: atomics and static storage only (DESIGN §3.4: no allocation, locks or
 * logging on the audio RT thread).
 */
#include <math.h>
#include <stdatomic.h>
#include <string.h>

#include "cmesh_internal.h"

static _Atomic(cmesh_push_fn) g_push = NULL;
static _Atomic(cmesh_pull_fn) g_pull = NULL;
static _Atomic(cmesh_flags_fn) g_flags_fn = NULL;
static _Atomic(cmesh_stats_fn) g_stats_fn = NULL;
static _Atomic(void *) g_ctx = NULL;
static atomic_bool g_external = false;

static atomic_bool g_capture_muted = true; /* the unit always starts muted (§3.2) */
static atomic_uint g_flags = 0;

static atomic_ullong g_input_callbacks = 0;
static atomic_ullong g_render_callbacks = 0;
static atomic_ullong g_captured_frames = 0;
static atomic_ullong g_rendered_frames = 0;
static atomic_uint g_input_errors = 0;
static atomic_int g_last_input_status = 0;
static atomic_uint g_capture_db_bits = 0;
static atomic_uint g_render_db_bits = 0;
static atomic_uint g_generation = 0;

void cmesh_set_sink(cmesh_push_fn push, cmesh_pull_fn pull, cmesh_flags_fn flags,
                    cmesh_stats_fn stats, void *ctx) {
    atomic_store(&g_push, push);
    atomic_store(&g_pull, pull);
    atomic_store(&g_flags_fn, flags);
    atomic_store(&g_stats_fn, stats);
    atomic_store(&g_ctx, ctx);
    atomic_store(&g_external, true);
    if (flags != NULL) {
        flags(atomic_load(&g_flags)); /* the core sees the current latch/mute state */
    }
}

bool cmesh_clear_sink(void *ctx) {
    if (!atomic_load(&g_external)) {
        return false;
    }
    if (ctx != NULL && atomic_load(&g_ctx) != ctx) {
        return false;
    }
    atomic_store(&g_external, false);
    atomic_store(&g_push, NULL);
    atomic_store(&g_pull, NULL);
    atomic_store(&g_flags_fn, NULL);
    atomic_store(&g_stats_fn, NULL);
    atomic_store(&g_ctx, NULL);
    return true;
}

void *cmesh_sink_ctx(void) { return atomic_load(&g_ctx); }
bool cmesh_has_external_sink(void) { return atomic_load(&g_external); }

void cmesh_set_capture_muted(bool muted) { atomic_store(&g_capture_muted, muted); }
bool cmesh_capture_muted(void) { return atomic_load(&g_capture_muted); }

void cmesh_set_flags(uint32_t flags) {
    atomic_store(&g_flags, flags);
    cmesh_flags_fn fn = atomic_load(&g_flags_fn);
    if (fn != NULL) {
        fn(flags);
    }
}

uint32_t cmesh_flags(void) { return atomic_load(&g_flags); }

uint32_t cmesh_update_flags(uint32_t set_mask, uint32_t clear_mask, uint32_t toggle_mask) {
    unsigned old = atomic_load(&g_flags);
    unsigned next;
    do {
        next = ((old | set_mask) & ~clear_mask) ^ toggle_mask;
    } while (!atomic_compare_exchange_weak(&g_flags, &old, next));
    cmesh_flags_fn fn = atomic_load(&g_flags_fn);
    if (fn != NULL) {
        fn(next);
    }
    return next;
}

void cmesh_note_input_error(int32_t status) {
    atomic_fetch_add(&g_input_errors, 1);
    atomic_store(&g_last_input_status, status);
}

void cmesh_bump_generation(void) { atomic_fetch_add(&g_generation, 1); }

float cmesh_block_dbfs(const int16_t *pcm, uint32_t frames) {
    if (pcm == NULL || frames == 0) {
        return -120.0f;
    }
    double acc = 0.0;
    for (uint32_t i = 0; i < frames; i++) {
        double s = (double)pcm[i];
        acc += s * s;
    }
    double rms = sqrt(acc / (double)frames);
    if (rms < 1e-3) {
        return -120.0f;
    }
    double db = 20.0 * log10(rms / 32768.0);
    return db < -120.0 ? -120.0f : (float)db;
}

float cmesh_dbfs_to_amplitude(float dbfs) {
    if (dbfs > 0.0f) {
        dbfs = 0.0f;
    }
    return 32767.0f * powf(10.0f, dbfs / 20.0f);
}

int32_t cmesh_dispatch_capture(const int16_t *pcm, uint32_t frames, uint64_t host_time_ns) {
    atomic_fetch_add(&g_input_callbacks, 1);
    atomic_fetch_add(&g_captured_frames, frames);
    atomic_store(&g_capture_db_bits, cmesh_f2u(cmesh_block_dbfs(pcm, frames)));
    if (atomic_load(&g_capture_muted)) {
        return 0;
    }
    if (atomic_load(&g_external)) {
        cmesh_push_fn push = atomic_load(&g_push);
        return push != NULL ? push(pcm, frames, host_time_ns) : (int32_t)frames;
    }
    return cmesh_builtin_push(pcm, frames);
}

int32_t cmesh_dispatch_render(int16_t *pcm, uint32_t frames, uint64_t host_time_ns) {
    atomic_fetch_add(&g_render_callbacks, 1);
    atomic_fetch_add(&g_rendered_frames, frames);
    int32_t n;
    if (atomic_load(&g_external)) {
        cmesh_pull_fn pull = atomic_load(&g_pull);
        if (pull != NULL) {
            n = pull(pcm, frames, host_time_ns);
        } else {
            memset(pcm, 0, (size_t)frames * sizeof(int16_t));
            n = (int32_t)frames;
        }
    } else {
        n = cmesh_builtin_pull(pcm, frames);
    }
    atomic_store(&g_render_db_bits, cmesh_f2u(cmesh_block_dbfs(pcm, frames)));
    return n;
}

void cmesh_io_counters(CMeshIoCounters *out) {
    if (out == NULL) {
        return;
    }
    out->input_callbacks = atomic_load(&g_input_callbacks);
    out->render_callbacks = atomic_load(&g_render_callbacks);
    out->captured_frames = atomic_load(&g_captured_frames);
    out->rendered_frames = atomic_load(&g_rendered_frames);
    out->input_render_errors = atomic_load(&g_input_errors);
    out->last_input_status = atomic_load(&g_last_input_status);
}

void cmesh_reset_counters(void) {
    atomic_store(&g_input_callbacks, 0);
    atomic_store(&g_render_callbacks, 0);
    atomic_store(&g_captured_frames, 0);
    atomic_store(&g_rendered_frames, 0);
    atomic_store(&g_input_errors, 0);
    atomic_store(&g_last_input_status, 0);
    atomic_store(&g_capture_db_bits, cmesh_f2u(-120.0f));
    atomic_store(&g_render_db_bits, cmesh_f2u(-120.0f));
}

void cmesh_meter_read(float *capture_dbfs, float *render_dbfs) {
    if (capture_dbfs != NULL) {
        uint32_t b = atomic_load(&g_capture_db_bits);
        *capture_dbfs = b == 0 ? -120.0f : cmesh_u2f(b);
    }
    if (render_dbfs != NULL) {
        uint32_t b = atomic_load(&g_render_db_bits);
        *render_dbfs = b == 0 ? -120.0f : cmesh_u2f(b);
    }
}

void cmesh_read_stats(CMeshRtStats *out) {
    if (out == NULL) {
        return;
    }
    cmesh_stats_fn fn = atomic_load(&g_stats_fn);
    if (atomic_load(&g_external) && fn != NULL) {
        fn(out);
        return;
    }
    memset(out, 0, sizeof(*out));
    out->captured_frames = atomic_load(&g_captured_frames);
    out->rendered_frames = atomic_load(&g_rendered_frames);
    out->capture_callbacks = (uint32_t)atomic_load(&g_input_callbacks);
    out->render_callbacks = (uint32_t)atomic_load(&g_render_callbacks);
    cmesh_meter_read(&out->capture_rms_dbfs, &out->render_rms_dbfs);
    out->generation = atomic_load(&g_generation);
}
