//! The exported C symbols of contract C5 (`include/mesh_rt.h`, DESIGN §3.6).
//!
//! This file is also the input of the cbindgen check (`cargo xtask gen-bindings
//! --check`, `cargo test -p mesh-ffi`): the functions, the `MeshRtStats` layout and the
//! two `#define`s generated from it must match the hand-written header. Keep it limited
//! to the C ABI.
//!
//! Every function is wait-free, allocation-free and lock-free and never panics: null
//! pointers and zero lengths are tolerated, and no Rust code below can unwind.

use super::RT;

/// RT ABI version (`MESH_RT_ABI_VERSION`).
pub const MESH_RT_ABI_VERSION: u32 = 1;
/// Sample rate of every RT buffer, mono int16 (`MESH_RT_SAMPLE_RATE`).
pub const MESH_RT_SAMPLE_RATE: u32 = 16000;

/// Counters of the RT plane (`MeshRtStats`).
#[repr(C)]
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct MeshRtStats {
    pub captured_frames: u64,
    pub rendered_frames: u64,
    pub capture_overruns: u32,
    pub render_underruns: u32,
    pub capture_callbacks: u32,
    pub render_callbacks: u32,
    pub capture_rms_dbfs: f32,
    pub render_rms_dbfs: f32,
    pub generation: u32,
}

/// Queues `frames` captured samples (16 kHz mono int16). Returns the frames accepted.
///
/// # Safety
/// `pcm` must be null or point to `frames` readable `int16_t`. Only one thread (the OS
/// capture callback) may call this at a time; a concurrent second caller is refused.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mesh_rt_capture(pcm: *const i16, frames: u32, host_time_ns: u64) -> i32 {
    let pcm: &[i16] = if pcm.is_null() || frames == 0 {
        &[]
    } else {
        // SAFETY: the caller guarantees `frames` readable samples at `pcm`.
        unsafe { std::slice::from_raw_parts(pcm, frames as usize) }
    };
    i32::try_from(RT.capture_rt(pcm, host_time_ns)).unwrap_or(i32::MAX)
}

/// Fills exactly `frames` samples of playout (queued audio, then silence). Returns
/// `frames` (0 for a null buffer).
///
/// # Safety
/// `pcm` must be null or point to `frames` writable `int16_t`. Only one thread (the OS
/// render callback) may call this at a time; a concurrent second caller gets silence.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mesh_rt_render(pcm: *mut i16, frames: u32, host_time_ns: u64) -> i32 {
    if pcm.is_null() || frames == 0 {
        return 0;
    }
    // SAFETY: the caller guarantees `frames` writable samples at `pcm`.
    let out = unsafe { std::slice::from_raw_parts_mut(pcm, frames as usize) };
    RT.render_rt(out, host_time_ns);
    i32::try_from(frames).unwrap_or(i32::MAX)
}

/// Sets the RT flags: bit0 PTT down, bit1 hard mute, bit2 built-in mic mute, bit3 PTT
/// latch toggled (edge, consumed by the engine on its next tick).
#[unsafe(no_mangle)]
pub extern "C" fn mesh_rt_set_flags(flags: u32) {
    RT.set_flags(flags);
}

/// Counts an OS-detected xrun or route glitch.
#[unsafe(no_mangle)]
pub extern "C" fn mesh_rt_report(event_code: u32) {
    RT.report(event_code);
}

/// Copies the RT counters into `*out` (no-op for null).
///
/// # Safety
/// `out` must be null or point to a writable `MeshRtStats`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mesh_rt_stats(out: *mut MeshRtStats) {
    if out.is_null() {
        return;
    }
    let s = RT.stats().to_c();
    // SAFETY: the caller guarantees `out` is writable.
    unsafe { out.write(s) };
}
