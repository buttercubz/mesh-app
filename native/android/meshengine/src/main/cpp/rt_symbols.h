// Function pointers to the RT C ABI of libmesh_ffi (contract C5, crates/mesh-ffi/include/
// mesh_rt.h), resolved with dlsym so the shim never links the Rust library directly and
// degrades to a tone when it is absent (DESIGN §10.2.1).
#pragma once
#include <cstdint>

struct MeshRtStats {
    uint64_t captured_frames, rendered_frames;
    uint32_t capture_overruns, render_underruns;
    uint32_t capture_callbacks, render_callbacks;
    float capture_rms_dbfs, render_rms_dbfs;
    uint32_t generation;
};

struct MeshRt {
    int32_t (*capture)(const int16_t *pcm, uint32_t frames, uint64_t host_time_ns) = nullptr;
    int32_t (*render)(int16_t *pcm, uint32_t frames, uint64_t host_time_ns) = nullptr;
    void (*set_flags)(uint32_t flags) = nullptr;
    void (*report)(uint32_t event_code) = nullptr;
    void (*stats)(MeshRtStats *out) = nullptr;

    bool complete() const { return capture && render && set_flags && report && stats; }
};

// Resolves the symbols once (thread-safe); later calls return the same table.
const MeshRt &mesh_rt_symbols();
