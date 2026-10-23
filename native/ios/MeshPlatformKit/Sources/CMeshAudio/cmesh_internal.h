/* Internal declarations shared by the CMeshAudio translation units. Not public API. */
#pragma once
#include <stdatomic.h>
#include <stdint.h>

#include "cmesh_audio.h"

/* Built-in sink entry points (cmesh_builtin.c); RT-safe. */
int32_t cmesh_builtin_push(const int16_t *pcm, uint32_t frames);
int32_t cmesh_builtin_pull(int16_t *pcm, uint32_t frames);

/* Block level in dBFS of `frames` Int16 samples; -120 for digital silence. RT-safe. */
float cmesh_block_dbfs(const int16_t *pcm, uint32_t frames);

/* Floats stored in atomics as their bit patterns. */
static inline uint32_t cmesh_f2u(float f) {
    union { float f; uint32_t u; } v;
    v.f = f;
    return v.u;
}
static inline float cmesh_u2f(uint32_t u) {
    union { float f; uint32_t u; } v;
    v.u = u;
    return v.f;
}

/* dBFS → linear amplitude relative to full scale (32767). */
float cmesh_dbfs_to_amplitude(float dbfs);

/* Records an AudioUnitRender failure in the input callback (cmesh_unit.c → counters). */
void cmesh_note_input_error(int32_t status);

/* Increments the stats generation (one per successful cmesh_start). */
void cmesh_bump_generation(void);
