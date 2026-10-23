//! Real-time audio plane (contract C5, DESIGN §3.6): the process-global [`RT`] state
//! behind the `mesh_rt_*` C symbols, and the engine-side accessors the runtime uses.
//!
//! RT entry points must never panic, index out of bounds or unwrap; the lints below
//! make that a compile-time rule for this module (DESIGN §2.3).
#![deny(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

pub mod abi;
mod shared;

pub use abi::{MESH_RT_ABI_VERSION, MESH_RT_SAMPLE_RATE, MeshRtStats};
pub use shared::{RtClaim, RtShared};

/// Capture ring size in samples (256 ms at 16 kHz, Appendix A).
pub const CAPTURE_RING_SAMPLES: usize = 4096;
/// Render ring size in samples (Appendix A).
pub const RENDER_RING_SAMPLES: usize = 4096;
/// Render ring fill the engine keeps at least (20 ms, Appendix A).
pub const RENDER_TARGET_SAMPLES: usize = 320;
/// Fill kept above the largest render callback (10 ms: two engine ticks).
pub const RENDER_HEADROOM_SAMPLES: usize = 160;
/// Refill margin added per underrun once audio has played (5 ms).
pub const RENDER_MARGIN_STEP_SAMPLES: u32 = 80;
/// Most refill margin learnt from underruns (40 ms).
pub const MAX_RENDER_MARGIN_SAMPLES: u32 = 640;
/// Upper bound of the refill target (150 ms; the ring holds 256 ms).
pub const MAX_RENDER_TARGET_SAMPLES: usize = 2400;
/// Cross-fade into queued audio after a dry render callback (2 ms).
pub const RESUME_FADE_SAMPLES: usize = 32;
/// RMS reported for silence.
pub const RMS_FLOOR_DBFS: f32 = -120.0;

/// bit0: PTT held down.
pub const FLAG_PTT_DOWN: u32 = mesh_core::rt_flags::PTT_DOWN;
/// bit1: hard mute.
pub const FLAG_HARD_MUTE: u32 = mesh_core::rt_flags::HARD_MUTE;
/// bit2: motorcycle built-in mic mute.
pub const FLAG_BUILTIN_MIC_MUTE: u32 = mesh_core::rt_flags::BUILTIN_MIC_MUTE;
/// bit3: PTT latch toggled (edge).
pub const FLAG_LATCH_TOGGLE: u32 = mesh_core::rt_flags::LATCH_TOGGLE;

/// The process-global RT plane behind the C ABI. Const-initialised (no lazy init on an
/// RT thread); the rings are created by the first `reset_and_activate`.
pub static RT: RtShared = RtShared::new();

/// Snapshot of the RT counters (the Rust view of `MeshRtStats`).
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct RtStats {
    pub captured_frames: u64,
    pub rendered_frames: u64,
    pub capture_overruns: u32,
    pub render_underruns: u32,
    pub capture_callbacks: u32,
    pub render_callbacks: u32,
    pub capture_rms_dbfs: f32,
    pub render_rms_dbfs: f32,
    pub generation: u32,
    // Not in the C layout (contract C5 unchanged); diagnostics only.
    /// Samples decay/zero-filled because the render ring ran dry.
    pub render_underrun_samples: u64,
    /// Samples refused because the capture ring was full.
    pub capture_overrun_samples: u64,
    /// Largest render / capture callback since the last activation (samples).
    pub max_render_frames: u32,
    pub max_capture_frames: u32,
    /// Current render ring refill target (samples).
    pub render_target: u32,
}

impl RtStats {
    /// The C layout.
    pub const fn to_c(self) -> MeshRtStats {
        MeshRtStats {
            captured_frames: self.captured_frames,
            rendered_frames: self.rendered_frames,
            capture_overruns: self.capture_overruns,
            render_underruns: self.render_underruns,
            capture_callbacks: self.capture_callbacks,
            render_callbacks: self.render_callbacks,
            capture_rms_dbfs: self.capture_rms_dbfs,
            render_rms_dbfs: self.render_rms_dbfs,
            generation: self.generation,
        }
    }

    /// Overruns + underruns.
    pub const fn xruns(&self) -> u32 {
        self.capture_overruns.saturating_add(self.render_underruns)
    }
}

#[cfg(test)]
mod tests;
