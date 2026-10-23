/*
 * mesh_rt.h: real-time audio C ABI of libmesh_ffi (contract C5, DESIGN §3.6).
 *
 * Hand-written and owned by WP-12. `cargo xtask gen-bindings --check`, `cargo xtask
 * build-ios` and `cargo test -p mesh-ffi` check it against crates/mesh-ffi/src/rt/abi.rs
 * with cbindgen used as a library: every function and the MeshRtStats layout must match.
 *
 * Called from the audio IO thread (CoreAudio VPIO callback / Oboe callback), directly
 * or through the cmesh_set_sink / dlsym function-pointer sink (DESIGN §2.4).
 */
#pragma once
#include <stdint.h>
#ifdef __cplusplus
extern "C" {
#endif
#define MESH_RT_ABI_VERSION 1u
#define MESH_RT_SAMPLE_RATE 16000u   /* mono, int16. v1 ceiling is wideband (16 kHz); a v2 ABI may negotiate 32 kHz */

/* mesh_rt_set_flags bits. */
#define MESH_RT_FLAG_PTT_DOWN          (1u << 0) /* PTT held down */
#define MESH_RT_FLAG_HARD_MUTE         (1u << 1) /* hard mute */
#define MESH_RT_FLAG_BUILTIN_MIC_MUTE  (1u << 2) /* motorcycle built-in mic mute */
#define MESH_RT_FLAG_LATCH_TOGGLE      (1u << 3) /* PTT latch toggled (edge, consumed by the engine) */

/* All functions: wait-free, no allocation, no locks, no syscalls, never panic. */
int32_t mesh_rt_capture(const int16_t *pcm, uint32_t frames, uint64_t host_time_ns); /* frames accepted */
int32_t mesh_rt_render(int16_t *pcm, uint32_t frames, uint64_t host_time_ns);        /* fills exactly `frames` */
void    mesh_rt_set_flags(uint32_t flags);  /* bit0 PTT down, bit1 hard mute, bit2 motorcycle-builtin-mic mute,
                                               bit3 PTT latch toggled (edge; set by LatchTalkIntent / notification action) */
void    mesh_rt_report(uint32_t event_code);/* OS-detected xrun/route glitch counter */
typedef struct MeshRtStats {
    uint64_t captured_frames, rendered_frames;
    uint32_t capture_overruns, render_underruns;
    uint32_t capture_callbacks, render_callbacks;   /* read by the native MuteWatchdog */
    float    capture_rms_dbfs, render_rms_dbfs;
    uint32_t generation;
} MeshRtStats;
void    mesh_rt_stats(MeshRtStats *out);
#ifdef __cplusplus
}
#endif
