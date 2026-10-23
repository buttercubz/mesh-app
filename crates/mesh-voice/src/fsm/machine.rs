//! [`TalkFsm`] transitions (§6.4).

use super::*;

/// The origin talk state machine.
#[derive(Debug, Clone)]
pub struct TalkFsm {
    state: TalkState,
    mode: TalkMode,
    hard_mute: bool,
    builtin_mute: bool,
    vad_on: bool,
    ptt_held: bool,
    latched: bool,
    next_burst: u8,
    burst: Option<u8>,
    burst_start_us: u64,
    p_sum: f32,
    p_blocks: u32,
    wind_checked: bool,
    speech_since_us: Option<u64>,
    last_busy_us: Option<u64>,
    last_vox_busy_us: Option<u64>,
    holdoff_until_us: Option<u64>,
    denials: [Option<u64>; WIND_DENIALS],
}

impl TalkFsm {
    /// `first_burst` is the first burst id (random per session, from the engine RNG).
    pub fn new(mode: TalkMode, first_burst: u8) -> Self {
        Self {
            state: if mode == TalkMode::Muted {
                TalkState::Muted
            } else {
                TalkState::Idle
            },
            mode,
            hard_mute: false,
            builtin_mute: false,
            vad_on: false,
            ptt_held: false,
            latched: false,
            next_burst: first_burst,
            burst: None,
            burst_start_us: 0,
            p_sum: 0.0,
            p_blocks: 0,
            wind_checked: false,
            speech_since_us: None,
            last_busy_us: None,
            last_vox_busy_us: None,
            holdoff_until_us: None,
            denials: [None; WIND_DENIALS],
        }
    }

    pub fn state(&self) -> TalkState {
        self.state
    }

    pub fn mode(&self) -> TalkMode {
        self.mode
    }

    /// Burst being transmitted.
    pub fn current_burst(&self) -> Option<u8> {
        self.burst
    }

    /// PTT latch state (Live Activity shows it).
    pub fn latched(&self) -> bool {
        self.latched
    }

    /// Time since the current burst started (`TALK.elapsed_10ms` source).
    pub fn burst_elapsed_us(&self, now_us: u64) -> Option<u64> {
        self.burst
            .map(|_| now_us.saturating_sub(self.burst_start_us))
    }

    fn muted(&self) -> bool {
        self.mode == TalkMode::Muted || self.hard_mute || self.builtin_mute
    }

    fn ptt_active(&self) -> bool {
        self.ptt_held || self.latched
    }

    /// The rider currently asks to talk in the active mode.
    fn wants_talk(&self) -> bool {
        match self.mode {
            TalkMode::Vox => self.vad_on,
            TalkMode::Ptt => self.ptt_active(),
            TalkMode::Muted => false,
        }
    }

    /// Applies one input and returns the effects.
    pub fn handle(&mut self, now_us: u64, input: TalkInput, env: &mut dyn TalkEnv) -> TalkOutputs {
        let mut out = TalkOutputs::new();
        match input {
            TalkInput::VadOn => {
                self.vad_on = true;
                if self.mode == TalkMode::Vox {
                    match self.state {
                        TalkState::Idle if !self.in_holdoff(now_us) => {
                            self.try_start(now_us, env, &mut out)
                        }
                        TalkState::Blocked => self.speech_since_us = Some(now_us),
                        _ => {}
                    }
                }
            }
            TalkInput::VadOff => {
                self.vad_on = false;
                self.speech_since_us = None;
                if self.mode == TalkMode::Vox {
                    match self.state {
                        TalkState::Talking => {
                            self.stop(StopReason::VoxRelease, true, false, &mut out)
                        }
                        TalkState::Blocked => {
                            self.state = TalkState::Idle;
                            self.holdoff_until_us = Some(
                                now_us + VOX_HOLDOFF_US + env.holdoff_jitter_us().min(500_000),
                            );
                        }
                        _ => {}
                    }
                }
            }
            TalkInput::PttDown => {
                let was = self.ptt_active();
                self.ptt_held = true;
                self.ptt_changed(was, now_us, env, &mut out);
            }
            TalkInput::PttUp => {
                let was = self.ptt_active();
                self.ptt_held = false;
                self.ptt_changed(was, now_us, env, &mut out);
            }
            TalkInput::LatchToggle => {
                let was = self.ptt_active();
                self.latched = !self.latched;
                self.ptt_changed(was, now_us, env, &mut out);
            }
            TalkInput::FloorReject { burst, reason } => {
                if self.state == TalkState::Talking && self.burst == Some(burst) {
                    self.stop(StopReason::FloorReject, false, false, &mut out);
                    out.push(TalkOutput::Earcon(if reason == RejectReason::Preempted {
                        Earcon::Preempted
                    } else {
                        Earcon::Busy
                    }));
                    self.enter_blocked(now_us, &mut out);
                }
            }
            TalkInput::SlotFreed => {
                if self.state == TalkState::Blocked && self.wants_talk() {
                    if self.mode == TalkMode::Ptt {
                        out.push(TalkOutput::Earcon(Earcon::GoAhead));
                    }
                    self.start(now_us, &mut out);
                }
            }
            TalkInput::SetMode(mode) => {
                if mode != self.mode {
                    if self.state == TalkState::Talking {
                        self.stop(StopReason::ModeChange, true, false, &mut out);
                    }
                    self.mode = mode;
                    self.latched = false;
                    if self.state == TalkState::Blocked {
                        self.state = TalkState::Idle;
                    }
                }
            }
            TalkInput::HardMute(on) => self.hard_mute = on,
            TalkInput::BuiltinMicMute(on) => self.builtin_mute = on,
            TalkInput::Block { vad_p } => self.tick(now_us, vad_p, env, &mut out),
        }
        self.apply_mute(now_us, env, &mut out);
        out
    }

    fn in_holdoff(&self, now_us: u64) -> bool {
        self.holdoff_until_us.is_some_and(|t| now_us < t)
    }

    fn ptt_changed(
        &mut self,
        was: bool,
        now_us: u64,
        env: &mut dyn TalkEnv,
        out: &mut TalkOutputs,
    ) {
        if self.mode != TalkMode::Ptt {
            return;
        }
        let now_active = self.ptt_active();
        if now_active && !was {
            if self.state == TalkState::Idle {
                self.try_start(now_us, env, out);
            }
        } else if !now_active && was {
            match self.state {
                TalkState::Talking => self.stop(StopReason::PttUp, true, true, out),
                TalkState::Blocked => self.state = TalkState::Idle,
                _ => {}
            }
        }
    }

    fn apply_mute(&mut self, now_us: u64, env: &mut dyn TalkEnv, out: &mut TalkOutputs) {
        if self.muted() {
            if self.state == TalkState::Talking {
                self.stop(StopReason::Muted, true, false, out);
            }
            self.state = TalkState::Muted;
        } else if self.state == TalkState::Muted {
            self.state = TalkState::Idle;
            if self.wants_talk() && !self.in_holdoff(now_us) {
                self.try_start(now_us, env, out);
            }
        }
    }

    fn try_start(&mut self, now_us: u64, env: &mut dyn TalkEnv, out: &mut TalkOutputs) {
        if env.local_admit(now_us) {
            self.start(now_us, out);
        } else {
            self.enter_blocked(now_us, out);
        }
    }

    fn start(&mut self, now_us: u64, out: &mut TalkOutputs) {
        let burst = self.next_burst;
        self.next_burst = self.next_burst.wrapping_add(1);
        self.burst = Some(burst);
        self.burst_start_us = now_us;
        self.state = TalkState::Talking;
        self.p_sum = 0.0;
        self.p_blocks = 0;
        self.wind_checked = false;
        self.holdoff_until_us = None;
        out.push(TalkOutput::StartBurst {
            burst,
            mode: self.mode,
        });
    }

    fn stop(&mut self, reason: StopReason, flush: bool, stop_twice: bool, out: &mut TalkOutputs) {
        if let Some(burst) = self.burst.take() {
            out.push(TalkOutput::StopBurst {
                burst,
                reason,
                flush,
                stop_twice,
            });
        }
        self.state = TalkState::Idle;
    }

    fn enter_blocked(&mut self, now_us: u64, out: &mut TalkOutputs) {
        self.state = TalkState::Blocked;
        out.push(TalkOutput::TalkDenied);
        match self.mode {
            TalkMode::Ptt => {
                if !out.contains(|o| matches!(o, TalkOutput::Earcon(_))) {
                    out.push(TalkOutput::Earcon(Earcon::Busy));
                }
                self.last_busy_us = Some(now_us);
            }
            TalkMode::Vox => {
                self.speech_since_us = Some(now_us);
                self.record_denial(now_us, out);
            }
            TalkMode::Muted => {}
        }
    }

    fn record_denial(&mut self, now_us: u64, out: &mut TalkOutputs) {
        for d in &mut self.denials {
            if d.is_some_and(|t| now_us.saturating_sub(t) >= WIND_DENIAL_WINDOW_US) {
                *d = None;
            }
        }
        if let Some(slot) = self.denials.iter_mut().find(|d| d.is_none()) {
            *slot = Some(now_us);
        }
        if self.denials.iter().all(Option::is_some) {
            self.denials = [None; WIND_DENIALS];
            out.push(TalkOutput::RaiseVoxThreshold);
        }
    }

    fn tick(&mut self, now_us: u64, vad_p: f32, env: &mut dyn TalkEnv, out: &mut TalkOutputs) {
        match self.state {
            TalkState::Talking => {
                let elapsed = now_us.saturating_sub(self.burst_start_us);
                let max = if self.mode == TalkMode::Ptt {
                    MAX_BURST_PTT_US
                } else {
                    MAX_BURST_VOX_US
                };
                if elapsed >= max {
                    self.stop(StopReason::MaxBurst, true, true, out);
                    out.push(TalkOutput::Earcon(Earcon::Busy));
                    // A stuck PTT or forgotten latch needs a fresh press; VOX a fresh onset.
                    self.latched = false;
                    if self.mode == TalkMode::Vox {
                        self.holdoff_until_us =
                            Some(now_us + VOX_HOLDOFF_US + env.holdoff_jitter_us().min(500_000));
                    }
                    return;
                }
                if self.mode == TalkMode::Vox {
                    self.p_sum += vad_p;
                    self.p_blocks += 1;
                    if !self.wind_checked && elapsed >= WIND_CHECK_AFTER_US {
                        self.wind_checked = true;
                        if self.p_sum / (self.p_blocks.max(1) as f32) < WIND_MIN_MEAN_P {
                            self.stop(StopReason::WindHog, true, false, out);
                            out.push(TalkOutput::RaiseVoxThreshold);
                            self.holdoff_until_us = Some(
                                now_us + VOX_HOLDOFF_US + env.holdoff_jitter_us().min(500_000),
                            );
                        }
                    }
                }
            }
            TalkState::Blocked => match self.mode {
                TalkMode::Ptt if self.ptt_active() => {
                    if self
                        .last_busy_us
                        .is_none_or(|t| now_us.saturating_sub(t) >= PTT_BUSY_REPEAT_US)
                    {
                        self.last_busy_us = Some(now_us);
                        out.push(TalkOutput::Earcon(Earcon::Busy));
                    }
                }
                TalkMode::Vox if self.vad_on => {
                    let sustained = self
                        .speech_since_us
                        .is_some_and(|t| now_us.saturating_sub(t) >= VOX_BUSY_DELAY_US);
                    let allowed = self
                        .last_vox_busy_us
                        .is_none_or(|t| now_us.saturating_sub(t) >= VOX_BUSY_INTERVAL_US);
                    if sustained && allowed {
                        self.last_vox_busy_us = Some(now_us);
                        out.push(TalkOutput::Earcon(Earcon::Busy));
                    }
                }
                _ => {}
            },
            TalkState::Idle => {
                if self.mode == TalkMode::Vox && self.holdoff_until_us.is_some_and(|t| now_us >= t)
                {
                    self.holdoff_until_us = None;
                    if self.vad_on {
                        self.try_start(now_us, env, out);
                    }
                }
            }
            TalkState::Muted => {}
        }
    }
}
