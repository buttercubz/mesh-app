//! Capture chain (DESIGN §7.1-§7.3): wind guard on the raw block, 80 Hz HPF, capture
//! AGC ([`Agc`], boost-only, field fix 2026-10-23), VAD probability, VOX gate with the single 300 ms hangover, and the 40 ms pre-roll.

use crate::agc::Agc;
use crate::dsp::{HighPass80, WindGuard};
use crate::vad::{Vad, VadEvent, VadGate, VadSensitivity};
use crate::{BLOCK_SAMPLES, Block, rms_dbfs, rms_linear};

/// Pre-roll length in 10 ms blocks (40 ms, §6.4).
pub const PREROLL_BLOCKS: usize = 4;

/// Ring of the last 40 ms of filtered capture, prepended to every burst.
#[derive(Debug, Clone)]
pub struct PreRoll {
    blocks: [Block; PREROLL_BLOCKS],
    next: usize,
    len: usize,
}

impl PreRoll {
    pub fn new() -> Self {
        Self {
            blocks: [[0; BLOCK_SAMPLES]; PREROLL_BLOCKS],
            next: 0,
            len: 0,
        }
    }

    /// Stores the newest block (overwriting the oldest).
    pub fn push(&mut self, block: &Block) {
        self.blocks[self.next] = *block;
        self.next = (self.next + 1) % PREROLL_BLOCKS;
        self.len = (self.len + 1).min(PREROLL_BLOCKS);
    }

    /// Blocks held (≤ 4).
    pub fn len(&self) -> usize {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// The `i`-th held block, oldest first.
    pub fn get(&self, i: usize) -> Option<&Block> {
        if i >= self.len {
            return None;
        }
        let start = (self.next + PREROLL_BLOCKS - self.len) % PREROLL_BLOCKS;
        Some(&self.blocks[(start + i) % PREROLL_BLOCKS])
    }

    /// Calls `f` on every held block, oldest first.
    pub fn for_each(&self, mut f: impl FnMut(&Block)) {
        for i in 0..self.len {
            if let Some(b) = self.get(i) {
                f(b);
            }
        }
    }

    pub fn clear(&mut self) {
        self.len = 0;
        self.next = 0;
    }
}

impl Default for PreRoll {
    fn default() -> Self {
        Self::new()
    }
}

/// What the capture chain learned from one block.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CaptureFrame {
    /// VAD voice probability.
    pub vad_p: f32,
    /// VOX gate state after this block.
    pub gate_open: bool,
    /// Gate edge (`vad_on` / `vad_off`) for the talk FSM.
    pub event: Option<VadEvent>,
    /// Wind guard fired on the raw block.
    pub windy: bool,
    /// Level of the filtered block in dBFS.
    pub rms_dbfs: f32,
    /// Level of the filtered block, linear 0..1 (for `audioCheckSample.level`).
    pub level: f32,
}

/// The capture front end owned by the engine (one per session).
pub struct CaptureChain {
    wind: WindGuard,
    hpf: HighPass80,
    agc: Agc,
    vad: Box<dyn Vad>,
    gate: VadGate,
    preroll: PreRoll,
}

impl CaptureChain {
    /// Creates the chain around a VAD (`RnnVad` on devices, `EnergyVad` in the simulator).
    pub fn new(vad: Box<dyn Vad>, sensitivity: VadSensitivity) -> Self {
        Self {
            wind: WindGuard::new(),
            hpf: HighPass80::new(),
            agc: Agc::new(),
            vad,
            gate: VadGate::new(sensitivity),
            preroll: PreRoll::new(),
        }
    }

    /// Processes one captured block in place (the block leaves high-passed and
    /// AGC-boosted; this is what PTT and VOX encode) and returns the VAD outcome. The filtered block is also stored in the pre-roll.
    pub fn process(&mut self, block: &mut Block) -> CaptureFrame {
        let windy = self.wind.process(block);
        self.hpf.process(block);
        self.agc.process(block, windy);
        let vad_p = self.vad.process(block);
        let rms = rms_dbfs(block);
        let event = self.gate.update(vad_p, rms, windy);
        self.preroll.push(block);
        CaptureFrame {
            vad_p,
            gate_open: self.gate.is_open(),
            event,
            windy,
            rms_dbfs: rms,
            level: rms_linear(block),
        }
    }

    pub fn preroll(&self) -> &PreRoll {
        &self.preroll
    }

    pub fn gate(&self) -> &VadGate {
        &self.gate
    }

    pub fn sensitivity(&self) -> VadSensitivity {
        self.gate.sensitivity()
    }

    /// The capture AGC (gain and noise-floor diagnostics).
    pub fn agc(&self) -> &Agc {
        &self.agc
    }

    /// Enables or bypasses the capture AGC (on by default).
    pub fn set_agc_enabled(&mut self, on: bool) {
        self.agc.set_enabled(on);
    }

    /// Changes the VOX preset (settings or wind-hog raise).
    pub fn set_sensitivity(&mut self, sens: VadSensitivity) {
        self.gate.set_sensitivity(sens);
    }

    /// Clears every filter, the VAD, the gate and the pre-roll (route change, restart).
    pub fn reset(&mut self) {
        self.wind.reset();
        self.hpf.reset();
        self.agc.reset();
        self.vad.reset();
        self.gate.reset();
        self.preroll.clear();
    }
}

impl core::fmt::Debug for CaptureChain {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("CaptureChain")
            .field("gate", &self.gate)
            .finish_non_exhaustive()
    }
}
