//! Audio check (`audioCheck`, DESIGN §1.3, §3.2, §7.3): the tone check (1 s tone, then
//! 2 s of recorded level and route), the echo test (the captured audio played back 1 s
//! later) and the VOX meter (live `vadP` and gate). The check runs on the capture block
//! (after the capture chain computed level, `p` and gate); what it plays goes through a
//! small FIFO into the playout mixer. `audioCheckSample` events are raised at 20 Hz.

use mesh_api::{AudioCheckPhase, GateState, UiEvent};
use mesh_voice::{
    AudioCheck, AudioCheckMode, AudioCheckSample, BLOCK_SAMPLES, Block, CaptureFrame,
};

/// Blocks buffered between capture and playout.
const FIFO: usize = 8;

/// A running check.
pub(super) struct CheckRun {
    check: AudioCheck,
    fifo: [Block; FIFO],
    head: usize,
    len: usize,
    /// Samples raised on the capture path, emitted from the timer path.
    pending: [Option<AudioCheckSample>; 4],
}

fn phase(p: mesh_voice::AudioCheckPhase) -> AudioCheckPhase {
    match p {
        mesh_voice::AudioCheckPhase::Tone => AudioCheckPhase::Tone,
        mesh_voice::AudioCheckPhase::Record => AudioCheckPhase::Record,
        mesh_voice::AudioCheckPhase::Echo => AudioCheckPhase::Echo,
        mesh_voice::AudioCheckPhase::Meter => AudioCheckPhase::Meter,
    }
}

/// The voice-crate mode of an API mode.
pub(super) fn mode(m: mesh_api::AudioCheckMode) -> AudioCheckMode {
    match m {
        mesh_api::AudioCheckMode::Tone => AudioCheckMode::Tone,
        mesh_api::AudioCheckMode::Echo => AudioCheckMode::Echo,
        mesh_api::AudioCheckMode::VoxMeter => AudioCheckMode::VoxMeter,
    }
}

impl CheckRun {
    /// Starts a check (control path: the echo line is allocated here).
    pub fn new(m: mesh_api::AudioCheckMode) -> Self {
        CheckRun {
            check: AudioCheck::new(mode(m)),
            fifo: [[0; BLOCK_SAMPLES]; FIFO],
            head: 0,
            len: 0,
            pending: [None; 4],
        }
    }

    /// One captured block: `raw` is what the microphone delivered (the echo plays it back
    /// unfiltered), `frame` the capture chain's analysis of it.
    pub fn on_capture(&mut self, raw: &Block, frame: &CaptureFrame) {
        let mut out = [0i16; BLOCK_SAMPLES];
        let sample = self.check.process(raw, frame, &mut out);
        if self.len == FIFO {
            self.head = (self.head + 1) % FIFO;
            self.len -= 1;
        }
        let i = (self.head + self.len) % FIFO;
        self.fifo[i] = out;
        self.len += 1;
        if let Some(s) = sample
            && let Some(slot) = self.pending.iter_mut().find(|p| p.is_none())
        {
            *slot = Some(s);
        }
    }

    /// The next block to play; `false` when nothing is buffered.
    pub fn pop_output(&mut self, out: &mut Block) -> bool {
        if self.len == 0 {
            return false;
        }
        *out = self.fifo[self.head];
        self.head = (self.head + 1) % FIFO;
        self.len -= 1;
        out.iter().any(|&s| s != 0)
    }

    /// `true` when samples wait for the timer path.
    pub fn has_pending(&self) -> bool {
        self.pending.iter().any(Option::is_some)
    }

    /// Drains the pending samples into `audioCheckSample` events.
    pub fn drain_events(
        &mut self,
        route: mesh_api::AudioRouteKind,
        input_rate: u32,
    ) -> impl Iterator<Item = UiEvent> + '_ {
        self.pending.iter_mut().filter_map(move |p| {
            let s = p.take()?;
            Some(UiEvent::AudioCheckSample {
                level: f64::from(s.level),
                vad_p: f64::from(s.vad_p),
                gate: if s.gate_open {
                    GateState::Open
                } else {
                    GateState::Closed
                },
                route,
                input_rate,
                phase: phase(s.phase),
            })
        })
    }
}
