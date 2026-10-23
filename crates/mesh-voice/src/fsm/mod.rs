//! Origin talk state machine for VOX and PTT (DESIGN §6.4, §6.5).
//!
//! Pure: every call carries `now_us`, floor decisions and randomness come from a
//! caller-supplied [`TalkEnv`], and effects are returned as [`TalkOutputs`].
//!
//! * There is **no FSM hangover**: `VadOff` (emitted by the VAD after its single 300 ms
//!   hangover) releases the burst at once.
//! * The lock-screen latch (RT flag bit3) arrives as [`TalkInput::LatchToggle`] and acts
//!   exactly like a latch tap.
//! * [`TalkInput::Block`] is fed every 10 ms; it drives MAX_BURST, busy-tone repeats, the
//!   VOX denied holdoff and the wind-hog check.

use crate::earcon::Earcon;

mod machine;

pub use machine::TalkFsm;

/// Talk mode selected by the rider (`setTalkMode`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum TalkMode {
    #[default]
    Vox,
    Ptt,
    Muted,
}

/// FSM state (exposed in the snapshot and PRESENCE `talk`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TalkState {
    Muted,
    Idle,
    Talking,
    Blocked,
}

/// FLOOR_REJECT reason (DESIGN §4.6).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RejectReason {
    Cap,
    Preempted,
    Congestion,
    Outranked,
}

/// FSM inputs.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum TalkInput {
    /// VOX gate opened.
    VadOn,
    /// VOX gate closed (after the VAD's 300 ms hangover).
    VadOff,
    /// PTT pressed (RT flag bit0 rising).
    PttDown,
    /// PTT released (RT flag bit0 falling).
    PttUp,
    /// Latch tap or lock-screen latch (RT flag bit3 edge).
    LatchToggle,
    /// FLOOR_REJECT addressed to my burst.
    FloorReject { burst: u8, reason: RejectReason },
    /// The floor table freed a slot I could use.
    SlotFreed,
    /// Talk mode change.
    SetMode(TalkMode),
    /// Hard mute (RT flag bit1).
    HardMute(bool),
    /// Built-in-mic mute on a motorcycle without headset mic (RT flag bit2, unless the
    /// rider overrides it with `setBuiltinMicTx`).
    BuiltinMicMute(bool),
    /// 10 ms tick with this block's VAD probability.
    Block { vad_p: f32 },
}

/// Why a burst ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum StopReason {
    VoxRelease,
    PttUp,
    MaxBurst,
    FloorReject,
    Muted,
    ModeChange,
    WindHog,
}

/// FSM effects, in the order the engine must apply them.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum TalkOutput {
    /// New burst: send TALK{START, profile}, encode pre-roll + live (FIRST on frame 0).
    StartBurst { burst: u8, mode: TalkMode },
    /// End of burst: `flush` encodes the partial frame (false = stop TX at once);
    /// `stop_twice` sends TALK{STOP} twice, 60 ms apart (PTT up, MAX_BURST).
    StopBurst {
        burst: u8,
        reason: StopReason,
        flush: bool,
        stop_twice: bool,
    },
    /// Play a cue.
    Earcon(Earcon),
    /// Talk denied: `talkDenied` event for the UI banner.
    TalkDenied,
    /// Wind-hog protection: raise the VOX threshold one level for 2 min and say
    /// `voxLessSensitive`.
    RaiseVoxThreshold,
}

/// Floor decisions and randomness supplied by the engine.
pub trait TalkEnv {
    /// My prospective burst ranks inside `cap_local` given my floor table (§6.4).
    fn local_admit(&mut self, now_us: u64) -> bool;
    /// `rand(0..500 ms)` for the VOX denied holdoff, from the injected ChaCha20Rng.
    fn holdoff_jitter_us(&mut self) -> u64;
}

/// Fixed-capacity output list (no allocation).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TalkOutputs {
    items: [Option<TalkOutput>; Self::CAP],
    len: usize,
}

impl TalkOutputs {
    pub const CAP: usize = 6;

    pub const fn new() -> Self {
        Self {
            items: [None; Self::CAP],
            len: 0,
        }
    }

    pub(crate) fn push(&mut self, o: TalkOutput) {
        if self.len < Self::CAP {
            self.items[self.len] = Some(o);
            self.len += 1;
        }
    }

    pub fn len(&self) -> usize {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    pub fn iter(&self) -> impl Iterator<Item = &TalkOutput> {
        self.items[..self.len].iter().flatten()
    }

    pub fn contains(&self, pred: impl Fn(&TalkOutput) -> bool) -> bool {
        self.iter().any(pred)
    }
}

impl Default for TalkOutputs {
    fn default() -> Self {
        Self::new()
    }
}

/// MAX_BURST for PTT (§6.1).
pub const MAX_BURST_PTT_US: u64 = 60_000_000;
/// MAX_BURST for VOX.
pub const MAX_BURST_VOX_US: u64 = 45_000_000;
/// VOX: busy tone only after this much sustained speech while blocked.
pub const VOX_BUSY_DELAY_US: u64 = 600_000;
/// VOX: busy at most once per 10 s.
pub const VOX_BUSY_INTERVAL_US: u64 = 10_000_000;
/// PTT: busy repeated every 2 s while held.
pub const PTT_BUSY_REPEAT_US: u64 = 2_000_000;
/// VOX holdoff after a denied burst ends (plus `rand(0..500 ms)`).
pub const VOX_HOLDOFF_US: u64 = 1_500_000;
/// Wind-hog: denials counted in this window.
pub const WIND_DENIAL_WINDOW_US: u64 = 30_000_000;
/// Wind-hog: denials that trigger the threshold raise.
pub const WIND_DENIALS: usize = 3;
/// Wind-hog: mean `p` is checked this long after burst start.
pub const WIND_CHECK_AFTER_US: u64 = 1_500_000;
/// Wind-hog: minimum mean `p` of a genuine burst.
pub const WIND_MIN_MEAN_P: f32 = 0.5;
