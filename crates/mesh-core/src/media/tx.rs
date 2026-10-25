//! Capture side (DESIGN §6.4, §7.2-§7.4): the 10 ms capture block runs the capture chain
//! (wind guard, HPF, VAD + gate), the talk FSM (VOX / PTT / latch, mutes, floor
//! decisions from the local floor table) and, while talking, the packetizer. TALK START
//! leaves in the same instant as VOICE frame 0 (the 40 ms pre-roll, FIRST flag); TALK
//! KEEP every 1 s carries `elapsed_10ms`; TALK STOP is sent twice 60 ms apart after PTT
//! up and MAX_BURST.

use mesh_api::{NoticeKey, PromptKey, PttStyle, TalkDeniedReason};
use mesh_routing::BurstKey;
use mesh_voice::{
    Block, EncoderConfig, Profile, TalkEnv, TalkInput, TalkMode, TalkOutput, VadEvent,
    fsm::StopReason,
};
use mesh_wire::mesh::{EncoderProfile, Talk, TalkPrio, TalkState};
use rand_chacha::ChaCha20Rng;
use rand_core::RngCore;

use super::bus::MediaIdentity;
use super::outbox::OutKind;
use super::state::{BurstTx, Media, StopAgain};
use super::{STOP_REPEAT_US, TALK_KEEP_US, VOX_RAISE_US};
use crate::engine::Ctx;
use crate::{Instant, rt_flags};

/// The FSM's view of the floor (`local_admit`) and of the RNG (VOX holdoff jitter).
struct Env<'a> {
    floor: &'a mesh_routing::FloorTable,
    key: BurstKey,
    prio: u8,
    cap: u8,
    rng: &'a mut ChaCha20Rng,
}

impl TalkEnv for Env<'_> {
    fn local_admit(&mut self, now_us: u64) -> bool {
        self.floor
            .local_admit(mesh_routing::Instant(now_us), self.key, self.prio, self.cap)
    }

    fn holdoff_jitter_us(&mut self) -> u64 {
        u64::from(self.rng.next_u32() % 500_000)
    }
}

/// Wire TALK mode of a voice talk mode.
pub(super) fn wire_mode(m: TalkMode) -> mesh_wire::mesh::TalkMode {
    match m {
        TalkMode::Ptt => mesh_wire::mesh::TalkMode::Ptt,
        TalkMode::Vox | TalkMode::Muted => mesh_wire::mesh::TalkMode::Vox,
    }
}

/// Wire profile of a voice profile.
pub(super) fn wire_profile(p: mesh_voice::Profile) -> EncoderProfile {
    match p {
        mesh_voice::Profile::Ble16 => EncoderProfile::Ble16,
        mesh_voice::Profile::Ble12 => EncoderProfile::Ble12,
        mesh_voice::Profile::Wifi20 => EncoderProfile::Wifi20,
        mesh_voice::Profile::Wifi24Fec => EncoderProfile::Wifi24Fec,
        mesh_voice::Profile::BleDred => EncoderProfile::BleDred,
    }
}

fn prio(me: &MediaIdentity) -> TalkPrio {
    if me.leader && me.leader_priority {
        TalkPrio::Leader
    } else {
        TalkPrio::Member
    }
}

impl Media {
    /// One 10 ms capture block (no allocation in steady state).
    pub(super) fn on_capture(&mut self, ctx: &mut Ctx, pcm: &Block) {
        if ctx.is_lab() || (!self.live && self.check.is_none()) {
            return;
        }
        let now = ctx.now;
        self.apply_sensitivity(now);
        let mut block = *pcm;
        let frame = self.chain.process(&mut block);
        if let Some(c) = self.check.as_mut() {
            c.on_capture(pcm, &frame);
        }
        if !self.live || !self.in_session {
            return;
        }
        let mut consumed = false;
        self.diag.vad_p = frame.vad_p;
        if let Some(ev) = frame.event {
            self.diag.vad = ev == VadEvent::On;
            let input = match ev {
                VadEvent::On => TalkInput::VadOn,
                VadEvent::Off => TalkInput::VadOff,
            };
            consumed |= self.talk_input(ctx, input);
        }
        consumed |= self.talk_input(ctx, TalkInput::Block { vad_p: frame.vad_p });
        if self.burst.is_some()
            && !consumed
            && let Some(p) = self.packetizer.as_mut()
        {
            let outbox = &mut self.outbox;
            let _ = p.push_block(&block, &mut |f| outbox.push_voice(now, f));
        }
    }

    /// Feeds the FSM and applies its outputs. Returns `true` when a burst started (the
    /// current capture block is already inside the pre-roll).
    pub(super) fn talk_input(&mut self, ctx: &mut Ctx, input: TalkInput) -> bool {
        let now = ctx.now;
        let me = self.identity(ctx);
        let cap = self.cap_local(ctx);
        let outs = {
            let mut env = Env {
                floor: &self.floor,
                key: BurstKey::new(me.node, ctx.sess, self.next_burst),
                prio: if prio(&me) == TalkPrio::Leader { 2 } else { 1 },
                cap,
                rng: &mut ctx.rng,
            };
            self.fsm.handle(now.us(), input, &mut env)
        };
        let mut started = false;
        for o in outs.iter() {
            match *o {
                TalkOutput::StartBurst { burst, mode } => {
                    self.start_burst(ctx, me, burst, mode);
                    started = true;
                }
                TalkOutput::StopBurst {
                    burst,
                    reason,
                    flush,
                    stop_twice,
                } => self.stop_burst(ctx, me, burst, reason, flush, stop_twice),
                TalkOutput::Earcon(e) => {
                    self.earcons.trigger(now.us(), e);
                }
                TalkOutput::TalkDenied => {
                    self.pending_denied = Some(self.last_reject.unwrap_or(TalkDeniedReason::Cap));
                }
                TalkOutput::RaiseVoxThreshold => {
                    self.vox_raised_until = Some(now.plus_us(VOX_RAISE_US));
                    self.apply_sensitivity(now);
                    self.defer_notice(NoticeKey::VoxLessSensitive);
                    self.defer_prompt(PromptKey::VoxLessSensitive);
                }
            }
        }
        if !outs.is_empty() {
            self.ui_dirty = true;
        }
        started
    }

    fn start_burst(&mut self, ctx: &mut Ctx, me: MediaIdentity, burst: u8, mode: TalkMode) {
        let now = ctx.now;
        let inputs = self.profile_inputs(ctx);
        let profile = self.selector.select(now.us(), &inputs);
        let rate = match ctx.device.route.input_rate {
            0 => mesh_voice::SAMPLE_RATE,
            r => r,
        };
        let cfg = EncoderConfig::for_profile(profile, rate, inputs.worst_loss_pct)
            .with_edge_fec(self.selector.edge_fec(&inputs), inputs.worst_loss_pct)
            .with_thermal(inputs.thermal_serious);
        self.outbox.push(
            now,
            OutKind::Talk(Talk {
                burst,
                state: TalkState::Start,
                prio: prio(&me),
                mode: wire_mode(mode),
                profile: wire_profile(profile),
                elapsed_10ms: 0,
            }),
        );
        if let Some(p) = self.packetizer.as_mut() {
            let outbox = &mut self.outbox;
            let _ = p.start_burst(burst, &cfg, self.chain.preroll(), &mut |f| {
                outbox.push_voice(now, f)
            });
        }
        self.next_burst = burst.wrapping_add(1);
        self.last_reject = None;
        self.burst = Some(BurstTx {
            burst,
            mode,
            profile,
            started: now,
            next_keep: now.plus_us(TALK_KEEP_US),
        });
    }

    fn stop_burst(
        &mut self,
        ctx: &mut Ctx,
        me: MediaIdentity,
        burst: u8,
        reason: StopReason,
        flush: bool,
        stop_twice: bool,
    ) {
        let now = ctx.now;
        if let Some(p) = self.packetizer.as_mut() {
            if flush {
                let outbox = &mut self.outbox;
                let _ = p.finish_burst(&mut |f| outbox.push_voice(now, f));
            } else {
                p.abort_burst();
            }
        }
        let Some(b) = self.burst.take().filter(|b| b.burst == burst) else {
            return;
        };
        let elapsed = (now.since(b.started) / 10_000).min(u64::from(u16::MAX)) as u16;
        let talk = Talk {
            burst,
            state: TalkState::Stop,
            prio: prio(&me),
            mode: wire_mode(b.mode),
            profile: wire_profile(b.profile),
            elapsed_10ms: elapsed,
        };
        self.outbox.push(now, OutKind::Talk(talk));
        if stop_twice {
            self.stop_again = Some(StopAgain {
                at: now.plus_us(STOP_REPEAT_US),
                burst,
                mode: b.mode,
                profile: b.profile,
                elapsed_10ms: elapsed,
            });
        }
        if reason == StopReason::MaxBurst {
            self.defer_notice(NoticeKey::TalkLimit);
            self.defer_prompt(PromptKey::TalkLimit);
        }
    }

    /// TALK KEEP and the second STOP (timer path).
    pub(super) fn tx_timer(&mut self, ctx: &mut Ctx) {
        let now = ctx.now;
        let Some(me) = self.identity else {
            return;
        };
        self.edge_downshift(ctx);
        if let Some(b) = self.burst.as_mut()
            && now >= b.next_keep
        {
            let elapsed = (now.since(b.started) / 10_000).min(u64::from(u16::MAX)) as u16;
            let talk = Talk {
                burst: b.burst,
                state: TalkState::Keep,
                prio: prio(&me),
                mode: wire_mode(b.mode),
                profile: wire_profile(b.profile),
                elapsed_10ms: elapsed,
            };
            b.next_keep = now.plus_us(TALK_KEEP_US);
            self.outbox.push(now, OutKind::Talk(talk));
        }
        if let Some(s) = self.stop_again
            && now >= s.at
        {
            self.stop_again = None;
            self.outbox.push(
                now,
                OutKind::Talk(Talk {
                    burst: s.burst,
                    state: TalkState::Stop,
                    prio: prio(&me),
                    mode: wire_mode(s.mode),
                    profile: wire_profile(s.profile),
                    elapsed_10ms: s.elapsed_10ms,
                }),
            );
        }
    }

    /// Mid-burst BLE16 → BLE12 downshift (field test 2026-10-24): at the edge of range
    /// the pacer rate can fall below one BLE16 stream during a burst, and a burst kept
    /// at 16 kbps then loses frames in the link queue. The profile is chosen at burst
    /// start (§7.4), so the running burst re-tunes its encoder to BLE12 (same 40 ms
    /// grid, no reset) and sends a TALK KEEP at once so relays and listeners account the
    /// lower cost. Never upshifts mid-burst (the next burst re-selects), so it cannot
    /// flap.
    fn edge_downshift(&mut self, ctx: &Ctx) {
        let Some(b) = self.burst.filter(|b| b.profile == Profile::Ble16) else {
            return;
        };
        let inputs = self.profile_inputs(ctx);
        if !(inputs.any_ble_link && (inputs.cap_pacing_one || inputs.path_hint_ble12)) {
            return;
        }
        let rate = match ctx.device.route.input_rate {
            0 => mesh_voice::SAMPLE_RATE,
            r => r,
        };
        let cfg = EncoderConfig::for_profile(Profile::Ble12, rate, inputs.worst_loss_pct)
            .with_thermal(inputs.thermal_serious);
        let ok = self
            .packetizer
            .as_mut()
            .is_some_and(|p| p.burst() == b.burst && p.reconfigure(&cfg).is_ok());
        if ok && let Some(t) = self.burst.as_mut() {
            t.profile = Profile::Ble12;
            t.next_keep = ctx.now;
        }
    }

    /// Next TX deadline (KEEP, second STOP).
    pub(super) fn tx_deadline(&self) -> Option<Instant> {
        [
            self.burst.map(|b| b.next_keep),
            self.stop_again.map(|s| s.at),
        ]
        .into_iter()
        .flatten()
        .min()
    }

    /// RT flags (§3.6): PTT (hold or latch style), hard mute, built-in-mic mute and the
    /// lock-screen latch bit3, whose rising edge acts exactly like a latch tap.
    pub(super) fn on_rt_flags(&mut self, ctx: &mut Ctx, prev: u32, flags: u32) {
        if !self.in_session {
            return;
        }
        let changed = prev ^ flags;
        if changed & rt_flags::HARD_MUTE != 0 {
            self.talk_input(ctx, TalkInput::HardMute(flags & rt_flags::HARD_MUTE != 0));
        }
        if changed & rt_flags::BUILTIN_MIC_MUTE != 0 {
            self.builtin_mute_flag = flags & rt_flags::BUILTIN_MIC_MUTE != 0;
            let on = self.builtin_mute_flag && !self.builtin_tx_allowed;
            self.talk_input(ctx, TalkInput::BuiltinMicMute(on));
        }
        if !self.live {
            return;
        }
        if changed & rt_flags::PTT_DOWN != 0 {
            let down = flags & rt_flags::PTT_DOWN != 0;
            self.ptt(ctx, down);
        }
        // Bit3 is an edge (mesh_rt.h): the runtime delivers it set for one tick per
        // lock-screen Talk tap and consumes it. Every delivery that carries it is one
        // latch tap; the engine re-delivers an unchanged value when bit3 is set, so two
        // taps in consecutive ticks are two toggles.
        if flags & rt_flags::LATCH_TOGGLE != 0 {
            self.talk_input(ctx, TalkInput::LatchToggle);
        }
    }

    /// A PTT press or release (RT flag or `ptt` command), honouring the PTT style.
    pub(super) fn ptt(&mut self, ctx: &mut Ctx, down: bool) {
        let held = match self.ptt_style {
            PttStyle::Latch if down => !self.diag.ptt,
            PttStyle::Latch => self.diag.ptt,
            PttStyle::Hold => down,
        };
        if held != self.diag.ptt {
            self.diag.ptt = held;
            ctx.fx.diag(mesh_lab::ridelog::DiagEvent::Ptt { on: held });
        }
        match (self.ptt_style, down) {
            (PttStyle::Latch, true) => {
                self.talk_input(ctx, TalkInput::LatchToggle);
            }
            (PttStyle::Latch, false) => {}
            (PttStyle::Hold, true) => {
                self.talk_input(ctx, TalkInput::PttDown);
            }
            (PttStyle::Hold, false) => {
                self.talk_input(ctx, TalkInput::PttUp);
            }
        }
    }
}
