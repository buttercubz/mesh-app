/*
 * cmesh_audio.h: real-time audio host of MeshPlatformKit (contract C9, DESIGN §10.1.1-§10.1.2).
 *
 * The VPIO AudioUnit callbacks call a function-pointer sink registered at runtime with
 * cmesh_set_sink(). The production sink is the Rust core's C5 ABI (mesh_rt.h): the four
 * function types below are signature-identical to mesh_rt_capture, mesh_rt_render,
 * mesh_rt_set_flags and mesh_rt_stats, so MeshEngineKit registers those symbols
 * directly (no trampoline on the RT thread). CMeshRtStats is layout-identical to
 * MeshRtStats; Swift callers pass mesh_rt_stats through unsafeBitCast to cmesh_stats_fn.
 *
 * Without a registered sink, a built-in sink (meter, loopback, tone, T5a click train and
 * capture) runs instead, so the audio host and its Lab benches work before any Rust
 * library is linked.
 *
 * Threading: cmesh_dispatch_* and every sink function run on the CoreAudio IO thread and
 * touch only atomics and preallocated static buffers. Control functions (start, stop,
 * configure, read) are for one non-RT control thread at a time (MeshPlatform serialises
 * them on its audio control queue).
 */
#pragma once
#include <stdbool.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

#define CMESH_SAMPLE_RATE 16000u        /* client format: mono, Int16, packed (MESH_RT_SAMPLE_RATE) */
#define CMESH_MAX_FRAMES_PER_SLICE 4096u /* preallocated per-callback capacity */

/* mesh_rt_set_flags bits (DESIGN §3.6). */
#define CMESH_FLAG_PTT_DOWN (1u << 0)
#define CMESH_FLAG_HARD_MUTE (1u << 1)
#define CMESH_FLAG_MOTO_BUILTIN_MIC_MUTE (1u << 2)
#define CMESH_FLAG_PTT_LATCH_TOGGLE (1u << 3)

/* Layout-identical to MeshRtStats (mesh_rt.h, contract C5). */
typedef struct CMeshRtStats {
    uint64_t captured_frames, rendered_frames;
    uint32_t capture_overruns, render_underruns;
    uint32_t capture_callbacks, render_callbacks;
    float capture_rms_dbfs, render_rms_dbfs;
    uint32_t generation;
} CMeshRtStats;

typedef int32_t (*cmesh_push_fn)(const int16_t *pcm, uint32_t frames, uint64_t host_time_ns);
typedef int32_t (*cmesh_pull_fn)(int16_t *pcm, uint32_t frames, uint64_t host_time_ns);
typedef void (*cmesh_flags_fn)(uint32_t flags);
typedef void (*cmesh_stats_fn)(CMeshRtStats *out);

/* ---- Sink registration ------------------------------------------------------------- */

/* Registers the external sink. Any pointer may be NULL (that leg then uses the built-in
 * sink: silence on render, discard on capture). `ctx` is an opaque owner token returned by
 * cmesh_sink_ctx(); cmesh_clear_sink(ctx) only clears a sink registered with that token. */
void cmesh_set_sink(cmesh_push_fn push, cmesh_pull_fn pull, cmesh_flags_fn flags,
                    cmesh_stats_fn stats, void *ctx);
/* Clears the external sink if it was registered with `ctx` (NULL clears any). Returns true
 * when a sink was cleared. */
bool cmesh_clear_sink(void *ctx);
void *cmesh_sink_ctx(void);
bool cmesh_has_external_sink(void);

/* Capture gate: while muted the input callback keeps running (the unit stays warm and the
 * IO counters advance) but nothing is pushed into the sink (DESIGN §3.2 StartCapture). */
void cmesh_set_capture_muted(bool muted);
bool cmesh_capture_muted(void);

/* RT flags: stored locally and forwarded to the registered flags function (the core's
 * mesh_rt_set_flags). Used by the PTT fast path and the Live Activity intents. */
void cmesh_set_flags(uint32_t flags);
uint32_t cmesh_flags(void);
/* Atomic read-modify-write of the flags (set, then clear, then toggle) for concurrent
 * writers (PTT button, Live Activity intents, notification actions). Forwards the new
 * value like cmesh_set_flags and returns it. Every flag writer should use this or
 * cmesh_set_flags so the stored copy and the core never diverge. */
uint32_t cmesh_update_flags(uint32_t set_mask, uint32_t clear_mask, uint32_t toggle_mask);

/* Stats from the registered stats function, or the host's own counters without one. */
void cmesh_read_stats(CMeshRtStats *out);

/* ---- Built-in sinks ----------------------------------------------------------------- */

typedef enum {
    CMESH_BUILTIN_SILENCE = 0, /* capture discarded, render zeros */
    CMESH_BUILTIN_METER = 1,   /* capture metered only, render zeros */
    CMESH_BUILTIN_LOOPBACK = 2,/* capture → ring → render (Mac mic in the simulator) */
    CMESH_BUILTIN_TONE = 3,    /* render a sine (bench tones, renderOnly anchor) */
    CMESH_BUILTIN_CLICK = 4,   /* T5a: click train out, capture recorded for onset search */
} cmesh_builtin_t;

/* Selects the built-in sink used when no external sink is registered. Resets its state. */
void cmesh_use_builtin(cmesh_builtin_t kind);
cmesh_builtin_t cmesh_builtin(void);

void cmesh_tone_configure(float frequency_hz, float level_dbfs);
/* Click train: one click every `period_frames`; capture recorded up to `capture_frames`
 * (clamped to CMESH_CLICK_MAX_CAPTURE). */
#define CMESH_CLICK_MAX_CAPTURE (16000u * 20u)
#define CMESH_CLICK_MAX_EVENTS 256u
void cmesh_click_configure(uint32_t period_frames, uint32_t capture_frames, float level_dbfs);
uint32_t cmesh_click_captured_frames(void);
uint32_t cmesh_click_emitted(void);
/* Median round-trip latency in frames over all clicks whose onset was found in the
 * capture (first |x| >= threshold after the emission). Returns -1 when none was found. */
int32_t cmesh_click_estimate_latency_frames(float threshold_dbfs, uint32_t *found_out);

/* Last computed block levels in dBFS (-120 for silence); updated for every sink. */
void cmesh_meter_read(float *capture_dbfs, float *render_dbfs);

/* ---- Dispatch (called by the AudioUnit callbacks; public for unit tests) ------------ */

int32_t cmesh_dispatch_capture(const int16_t *pcm, uint32_t frames, uint64_t host_time_ns);
int32_t cmesh_dispatch_render(int16_t *pcm, uint32_t frames, uint64_t host_time_ns);

typedef struct CMeshIoCounters {
    uint64_t input_callbacks;
    uint64_t render_callbacks;
    uint64_t captured_frames;
    uint64_t rendered_frames;
    uint32_t input_render_errors; /* AudioUnitRender failures in the input callback */
    int32_t last_input_status;    /* last OSStatus of AudioUnitRender ('!rec' etc.) */
} CMeshIoCounters;
void cmesh_io_counters(CMeshIoCounters *out);
void cmesh_reset_counters(void);

/* ---- AudioUnit lifecycle (DESIGN §10.1.2) ------------------------------------------- */

typedef enum {
    CMESH_PROFILE_VOICE = 0,             /* VPIO, input + output, VP on, AGC on */
    CMESH_PROFILE_VOICE_RENDER_ONLY = 1, /* VPIO, output only (Lab renderOnly) */
    CMESH_PROFILE_VOICE_BYPASS = 2,      /* VPIO with BypassVoiceProcessing = 1 (T5a only) */
    CMESH_PROFILE_OUTPUT_ONLY = 3,       /* RemoteIO output-only (OutputOnlyFallback, L2) */
} cmesh_profile_t;

/* Creates, configures, initialises and starts the unit. Returns 0 (noErr) or the failing
 * OSStatus. Calling it while running first stops the current unit. */
int32_t cmesh_start(cmesh_profile_t profile);
/* Stops and disposes the unit. Returns 0 or the failing OSStatus. */
int32_t cmesh_stop(void);
/* kAudioOutputUnitProperty_IsRunning of the current unit (false without a unit). */
bool cmesh_is_running(void);
bool cmesh_has_unit(void);
cmesh_profile_t cmesh_profile(void);
/* Hardware-side formats (bus 1 input scope / bus 0 output scope). Returns false without
 * a unit. Used to detect "route change, format changed" (DESIGN §10.1.4). */
bool cmesh_hw_format(double *in_rate, uint32_t *in_channels, double *out_rate,
                     uint32_t *out_channels);
/* Client formats read back after initialisation (must be 16 kHz mono Int16). */
bool cmesh_client_format_ok(void);

#ifdef __cplusplus
}
#endif
