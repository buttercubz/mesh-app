//! Capture gating, the degradation ladder, the audio check and Bluetooth radio state
//! (DESIGN §3.2, §7.8, §10.1.4).
//!
//! Capture gating (normative): `AudioCommand::StartCapture` is emitted only while
//! handling `goLive`, `resumeMic` or `audioCheck{start}` (and `labStart{audio: full}` in
//! the Lab module). Platform events, timers and packets never start capture; in Idle
//! only the audio check may.
//!
//! **Seam with the media module (WP-10C).** The session owns the decision (state,
//! validation, refusal codes); the media module owns the audio. One pattern per
//! command, never both:
//! - `goLive`, `resumeMic` and `audioCheck` are validated and recorded here and then
//!   passed on (the group module answers `None`): the media module, registered after
//!   it, emits `OpenSession` / `StartCapture` inside the same gesture command and starts
//!   its chains. A refusal is answered here and never reaches the media module, so no
//!   audio command is emitted for it.
//! - `setTalkMode` is answered here: the session owns the gating of the talk mode
//!   (muted in the lobby until the ride starts, §1.3 F3, and at ladder L2/L3, §7.8), so
//!   the media module only ever receives the gated mode, posted on the inter-module bus
//!   as `MediaRequest::SetTalkMode` by [`sync_talk_mode`] whenever it changes while
//!   live.
//!
//! Session teardown that is not an engine mode change (removal, group closed) emits
//! `StopCapture` + `CloseSession` here and posts `MediaRequest::EndSession`
//! ([`end_session`]); on an engine stop the media module ends its session itself.

use mesh_api::{CommandResult, ErrorCode, LadderLevel, NoticeKey, Params, TalkMode};

use super::Session;
use super::cue::{self, Cue};
use crate::effects::AudioCommand;
use crate::engine::Ctx;
use crate::links::RadioAvail;
use crate::media::MediaRequest;
use crate::{AudioIoState, AudioRouteInfo, InterruptReason};

/// Temporary audio-check session (§1.3 F0 step 3): works without a session and in the
/// lobby before Go live; refused while live (and during a Lab run, by the dispatcher).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct AudioCheck {
    pub active: bool,
}

/// What the session decided about a gesture command.
#[derive(Debug, Clone, PartialEq)]
pub enum Gesture {
    /// Accepted: the media module performs the audio part of the command.
    PassOn,
    /// Refused with this result (no audio command is emitted).
    Refuse(CommandResult),
}

impl Gesture {
    /// The module's answer to the dispatcher: `None` passes the command on.
    pub fn answer(self) -> Option<CommandResult> {
        match self {
            Gesture::PassOn => None,
            Gesture::Refuse(r) => Some(r),
        }
    }
}

fn refuse(code: ErrorCode) -> Gesture {
    Gesture::Refuse(CommandResult::error(code))
}

impl AudioCheck {
    /// `audioCheck{start}`: refused while live (`wrongMode`, as the media module does).
    pub fn start(&mut self, live: bool) -> Gesture {
        if live {
            return refuse(ErrorCode::WrongMode);
        }
        self.active = true;
        Gesture::PassOn
    }

    /// `audioCheck{stop}` (also when Go live takes over the audio session).
    pub fn stop(&mut self) -> Gesture {
        self.active = false;
        Gesture::PassOn
    }
}

/// `goLive`: the ride session opens and capture connects (muted until the ride starts).
pub fn go_live(s: &mut Session, check: &mut AudioCheck, ctx: &mut Ctx) -> Gesture {
    if s.live {
        return refuse(ErrorCode::AlreadyLive);
    }
    // The ride session replaces the temporary check session.
    check.active = false;
    s.live = true;
    s.ladder = LadderLevel::L0;
    ctx.mark_dirty();
    Gesture::PassOn
}

/// `resumeMic` (user tap after a capture loss): capture reconnects; the ladder goes to
/// L1 until the IO reports Running.
pub fn resume_mic(s: &mut Session, ctx: &mut Ctx) -> Gesture {
    if !s.live {
        return refuse(ErrorCode::NotLive);
    }
    if s.ladder != LadderLevel::L0 {
        s.ladder = LadderLevel::L1;
        ctx.fx.audio(AudioCommand::OutputOnlyFallback(false));
    }
    ctx.mark_dirty();
    Gesture::PassOn
}

/// Leaves the live state (engine stop, removal or group closed).
pub fn end_live(s: &mut Session, ctx: &mut Ctx) {
    if s.live {
        s.live = false;
        s.media_mode = None;
        ctx.fx.audio(AudioCommand::StopCapture);
        ctx.fx
            .diag(mesh_lab::ridelog::DiagEvent::CaptureStop { why: "endLive" });
        ctx.fx.audio(AudioCommand::CloseSession);
    }
}

/// Session teardown without an engine mode change (removal, group closed): leaves the
/// live state and tells the media module, which drops its session audio state.
pub fn end_session(s: &mut Session, ctx: &mut Ctx) {
    end_live(s, ctx);
    ctx.post_media(MediaRequest::EndSession);
}

/// The talk mode the media module must run: muted until live and riding, while the
/// user chose Muted and at ladder L2/L3 (capture lost); otherwise the effective mode.
pub fn media_mode(s: &Session) -> TalkMode {
    if s.talk_muted() {
        TalkMode::Muted
    } else {
        s.effective_mode()
    }
}

/// Posts `MediaRequest::SetTalkMode` when the gated talk mode changed while live (at Go
/// live, ride start/end, a `setTalkMode`, ladder changes). Not live: nothing is
/// posted, and the next Go live posts the mode again.
pub fn sync_talk_mode(s: &mut Session, ctx: &mut Ctx) {
    if !s.live {
        s.media_mode = None;
        return;
    }
    let m = media_mode(s);
    if s.media_mode != Some(m) {
        s.media_mode = Some(m);
        ctx.post_media(MediaRequest::SetTalkMode(m));
    }
}

/// The audio route changed: the headset went away while live (§6.5 "headset lost (TX
/// muted)").
pub fn on_route(s: &mut Session, ctx: &mut Ctx, r: &AudioRouteInfo) {
    let was = s.headset.replace(r.headset_present);
    if s.live && was == Some(true) && !r.headset_present {
        cue::request(ctx, &Cue::HeadsetLost);
    }
}

/// A platform sample: the battery-low cue (below 15 %, not charging, at most once per
/// 10 min, §1.3 F4).
pub fn on_battery(s: &mut Session, ctx: &mut Ctx, battery: Option<f64>, charging: Option<bool>) {
    let low = battery.is_some_and(|b| b < super::BATTERY_LOW) && charging != Some(true);
    let due = s
        .battery_cue_at
        .is_none_or(|t| ctx.now.since(t) >= super::BATTERY_CUE_INTERVAL_US);
    if s.live && low && due {
        s.battery_cue_at = Some(ctx.now);
        cue::request(ctx, &Cue::BatteryLow);
    }
}

/// Ladder transitions from the audio IO state (§7.8). Returns `true` when the ladder
/// level changed.
pub fn on_audio_io(s: &mut Session, ctx: &mut Ctx, io: AudioIoState) -> bool {
    let was = s.ladder;
    if let AudioIoState::Interrupted {
        reason: InterruptReason::Call,
    } = io
        && !s.on_call
    {
        s.on_call = true;
        ctx.notice(NoticeKey::OnCall, Params::new());
    }
    if matches!(io, AudioIoState::Running | AudioIoState::Restored) && s.on_call {
        s.on_call = false;
        ctx.notice(NoticeKey::CallEnded, Params::new());
    }
    if !s.live {
        return false;
    }
    let next = match io {
        AudioIoState::Running | AudioIoState::Restored => LadderLevel::L0,
        AudioIoState::Interrupted { .. } | AudioIoState::Restarting => LadderLevel::L1,
        AudioIoState::CaptureLost => LadderLevel::L2,
        AudioIoState::RenderLost | AudioIoState::Stopped => LadderLevel::L3,
    };
    if next == was {
        return false;
    }
    s.ladder = next;
    match next {
        LadderLevel::L2 => {
            ctx.fx.audio(AudioCommand::OutputOnlyFallback(true));
            cue::alert(ctx, &Cue::MicLost, NoticeKey::CaptureLost, Params::new());
        }
        LadderLevel::L3 => {
            cue::alert(ctx, &Cue::MicLost, NoticeKey::CaptureLost, Params::new());
        }
        LadderLevel::L0 if matches!(was, LadderLevel::L2 | LadderLevel::L3) => {
            ctx.fx.audio(AudioCommand::OutputOnlyFallback(false));
            cue::request(ctx, &Cue::MicBack);
            ctx.notice(NoticeKey::CaptureRestored, Params::new());
        }
        _ => {}
    }
    ctx.mark_dirty();
    true
}

/// Bluetooth availability changed (base engine already paused discovery and marked the
/// BLE links down). Returns `true` when the PRESENCE content changed.
pub fn on_radio_state(s: &mut Session, ctx: &mut Ctx, ble: RadioAvail) -> bool {
    let on = ble == RadioAvail::On;
    if on == s.ble_on {
        if ble == RadioAvail::Unauthorized {
            ctx.notice(NoticeKey::BluetoothDenied, Params::new());
        }
        return false;
    }
    s.ble_on = on;
    if on {
        s.radio_back_at = Some(ctx.now);
        s.radio_cue_at = None;
        ctx.notice(NoticeKey::BluetoothOn, Params::new());
    } else if ble == RadioAvail::Unauthorized {
        s.radio_cue_at = None;
        ctx.notice(NoticeKey::BluetoothDenied, Params::new());
    } else {
        s.radio_cue_at = Some(ctx.now);
        cue::alert(ctx, &Cue::RadioOff, NoticeKey::BluetoothOff, Params::new());
    }
    ctx.mark_dirty();
    true
}

/// Repeats the Bluetooth-off cue every 60 s while the radio stays off (§6.5). The
/// repeat is a deadline of the session ([`Session::next_deadline`]), so it plays on
/// time whatever the timer cadence.
pub fn radio_tick(s: &mut Session, ctx: &mut Ctx) {
    if let Some(at) = s.radio_cue_at
        && !s.ble_on
        && ctx.now.since(at) >= super::RADIO_CUE_REPEAT_US
    {
        s.radio_cue_at = Some(at.plus_us(super::RADIO_CUE_REPEAT_US).max(ctx.now));
        cue::request(ctx, &Cue::RadioOff);
    }
}
