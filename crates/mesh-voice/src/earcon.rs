//! Earcons (DESIGN §6.5): the guaranteed audible cue for every state, synthesized here
//! and mixed before the limiter (inside the AEC reference).

use crate::Block;
use crate::tone::{TonePlayer, ToneSegment};

/// Earcon level (the §6.5 busy level, used for every cue).
pub const EARCON_LEVEL_DBFS: f32 = -18.0;

/// Every cue of §6.5.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Earcon {
    /// 2 × (480 Hz, 150 ms on, 100 ms off).
    Busy,
    /// 660 / 550 / 440 Hz × 80 ms.
    Preempted,
    /// 880 Hz, 40 ms.
    GoAhead,
    /// 440 → 660 → 880 Hz × 80 ms.
    RideStarted,
    /// 660 → 880 Hz, 2 × 80 ms.
    MemberJoined,
    /// 880 → 660 Hz, 2 × 80 ms.
    MemberLost,
    /// 2 low pulses (300 Hz).
    Alone,
    /// 3 × 300 Hz, 150 ms (listen-only).
    MicLost,
    /// 2 × 400 Hz (TX muted).
    HeadsetLost,
    /// 3 descending tones 600 → 300 Hz × 100 ms (Bluetooth off / airplane mode).
    BluetoothOff,
}

const BUSY: &[ToneSegment] = &[
    ToneSegment::tone(480.0, 150, 100),
    ToneSegment::tone(480.0, 150, 100),
];
const PREEMPTED: &[ToneSegment] = &[
    ToneSegment::tone(660.0, 80, 0),
    ToneSegment::tone(550.0, 80, 0),
    ToneSegment::tone(440.0, 80, 0),
];
const GO_AHEAD: &[ToneSegment] = &[ToneSegment::tone(880.0, 40, 0)];
const RIDE_STARTED: &[ToneSegment] = &[
    ToneSegment::tone(440.0, 80, 0),
    ToneSegment::tone(660.0, 80, 0),
    ToneSegment::tone(880.0, 80, 0),
];
const MEMBER_JOINED: &[ToneSegment] = &[
    ToneSegment::tone(660.0, 80, 0),
    ToneSegment::tone(880.0, 80, 0),
];
const MEMBER_LOST: &[ToneSegment] = &[
    ToneSegment::tone(880.0, 80, 0),
    ToneSegment::tone(660.0, 80, 0),
];
const ALONE: &[ToneSegment] = &[
    ToneSegment::tone(300.0, 150, 150),
    ToneSegment::tone(300.0, 150, 0),
];
const MIC_LOST: &[ToneSegment] = &[
    ToneSegment::tone(300.0, 150, 100),
    ToneSegment::tone(300.0, 150, 100),
    ToneSegment::tone(300.0, 150, 0),
];
const HEADSET_LOST: &[ToneSegment] = &[
    ToneSegment::tone(400.0, 150, 100),
    ToneSegment::tone(400.0, 150, 0),
];
const BLUETOOTH_OFF: &[ToneSegment] = &[
    ToneSegment::tone(600.0, 100, 0),
    ToneSegment::tone(450.0, 100, 0),
    ToneSegment::tone(300.0, 100, 0),
];

const ONCE: u64 = u64::MAX;

impl Earcon {
    pub const ALL: [Earcon; 10] = [
        Earcon::Busy,
        Earcon::Preempted,
        Earcon::GoAhead,
        Earcon::RideStarted,
        Earcon::MemberJoined,
        Earcon::MemberLost,
        Earcon::Alone,
        Earcon::MicLost,
        Earcon::HeadsetLost,
        Earcon::BluetoothOff,
    ];

    /// The tone sequence.
    pub const fn segments(self) -> &'static [ToneSegment] {
        match self {
            Earcon::Busy => BUSY,
            Earcon::Preempted => PREEMPTED,
            Earcon::GoAhead => GO_AHEAD,
            Earcon::RideStarted => RIDE_STARTED,
            Earcon::MemberJoined => MEMBER_JOINED,
            Earcon::MemberLost => MEMBER_LOST,
            Earcon::Alone => ALONE,
            Earcon::MicLost => MIC_LOST,
            Earcon::HeadsetLost => HEADSET_LOST,
            Earcon::BluetoothOff => BLUETOOTH_OFF,
        }
    }

    /// Minimum spacing enforced by [`Earcons`] (µs). Busy is rate-limited by the talk FSM
    /// (per PTT press and every 2 s while held; VOX once per 10 s); joined/lost aggregate
    /// over 5 s; alone, mic lost and Bluetooth off repeat at most every 60 s; ride started
    /// plays once (until [`Earcons::reset_rate_limits`]).
    pub const fn min_interval_us(self) -> u64 {
        match self {
            Earcon::Busy | Earcon::Preempted | Earcon::GoAhead | Earcon::HeadsetLost => 0,
            Earcon::MemberJoined | Earcon::MemberLost => 5_000_000,
            Earcon::Alone | Earcon::MicLost | Earcon::BluetoothOff => 60_000_000,
            Earcon::RideStarted => ONCE,
        }
    }

    const fn index(self) -> usize {
        self as usize
    }
}

/// Queue capacity for pending cues.
pub const QUEUE_LEN: usize = 8;

/// Rate-limited earcon player (one per session).
#[derive(Debug, Clone)]
pub struct Earcons {
    last_us: [Option<u64>; Earcon::ALL.len()],
    queue: [Option<Earcon>; QUEUE_LEN],
    playing: Option<TonePlayer>,
    current: Option<Earcon>,
}

impl Earcons {
    pub fn new() -> Self {
        Self {
            last_us: [None; Earcon::ALL.len()],
            queue: [None; QUEUE_LEN],
            playing: None,
            current: None,
        }
    }

    /// Requests `cue` at `now_us`. Returns false when rate-limited, already queued, or the
    /// queue is full.
    pub fn trigger(&mut self, now_us: u64, cue: Earcon) -> bool {
        let i = cue.index();
        if let Some(last) = self.last_us[i] {
            let min = cue.min_interval_us();
            if min == ONCE || now_us.saturating_sub(last) < min {
                return false;
            }
        }
        if self.queue.contains(&Some(cue)) {
            return false;
        }
        let Some(slot) = self.queue.iter_mut().find(|q| q.is_none()) else {
            return false;
        };
        *slot = Some(cue);
        self.last_us[i] = Some(now_us);
        true
    }

    /// Clears the rate limiter (new ride / session).
    pub fn reset_rate_limits(&mut self) {
        self.last_us = [None; Earcon::ALL.len()];
    }

    /// Drops queued and playing cues.
    pub fn clear(&mut self) {
        self.queue = [None; QUEUE_LEN];
        self.playing = None;
        self.current = None;
    }

    /// Cue currently audible.
    pub fn current(&self) -> Option<Earcon> {
        self.current
    }

    /// True when nothing is playing or queued.
    pub fn is_idle(&self) -> bool {
        self.playing.is_none() && self.queue.iter().all(Option::is_none)
    }

    /// Renders the next 10 ms of cues (silence when idle). Returns true when `out` carries
    /// audio. Cues play one after the other in trigger order.
    pub fn render(&mut self, out: &mut Block) -> bool {
        out.fill(0);
        if self.playing.is_none() {
            self.start_next();
        }
        let Some(p) = self.playing.as_mut() else {
            return false;
        };
        let active = p.render(out);
        if p.is_done() {
            self.playing = None;
            self.current = None;
            // Back-to-back cues: the next one starts on the following block.
        }
        active
    }

    fn start_next(&mut self) {
        if let Some(cue) = self.queue[0].take() {
            self.queue.rotate_left(1);
            self.playing = Some(TonePlayer::new(cue.segments(), EARCON_LEVEL_DBFS));
            self.current = Some(cue);
        }
    }
}

impl Default for Earcons {
    fn default() -> Self {
        Self::new()
    }
}
