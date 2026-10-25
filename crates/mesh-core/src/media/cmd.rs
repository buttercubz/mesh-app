//! UI commands and session requests.
//!
//! Media-owned commands: `ptt`, `setPttStyle`, `setVoxSensitivity`, `setBuiltinMicTx`,
//! `setPrompts`. The session module's commands (`goLive`, `resumeMic`, `setTalkMode`,
//! `audioCheck`) reach this module only when no earlier module answered them (a build
//! without the session module); it then answers them itself, with the same capture
//! gating (`StartCapture` only while handling `goLive`, `resumeMic` or
//! `audioCheck{start}`), and turns them into the [`MediaRequest`] the session module
//! would have sent.

use mesh_api::{
    AudioCheckOp, CommandResult, ErrorCode, TalkMode as ApiTalkMode, UiCommand, VoxSensitivity,
};
use mesh_lab::ridelog::DiagEvent;
use mesh_voice::{TalkInput, TalkMode, VadSensitivity};

use super::bus::MediaRequest;
use super::check::CheckRun;
use super::state::Media;
use crate::effects::{AudioCommand, AudioSessionKind, AudioSessionProfile};
use crate::engine::{Ctx, EngineMode};
use crate::links::LinkState;

/// Voice-crate talk mode of an API talk mode.
pub(super) fn talk_mode(m: ApiTalkMode) -> TalkMode {
    match m {
        ApiTalkMode::Vox => TalkMode::Vox,
        ApiTalkMode::Ptt => TalkMode::Ptt,
        ApiTalkMode::Muted => TalkMode::Muted,
    }
}

fn sensitivity(v: VoxSensitivity) -> VadSensitivity {
    match v {
        VoxSensitivity::Low => VadSensitivity::Low,
        VoxSensitivity::Medium => VadSensitivity::Medium,
        VoxSensitivity::High => VadSensitivity::High,
    }
}

fn profile(kind: AudioSessionKind) -> AudioSessionProfile {
    AudioSessionProfile {
        kind,
        capture: true,
        mixable: false,
        location_anchor: false,
        bypass_voice_processing: false,
    }
}

impl Media {
    /// Riders reachable now, me included (`intercomLive{n}`).
    fn riders(ctx: &Ctx) -> u8 {
        let n = ctx
            .links
            .iter()
            .filter(|l| l.state == LinkState::Up && !l.is_lab())
            .count();
        (n + 1).min(255) as u8
    }

    /// A UI command no earlier module answered.
    pub(super) fn on_command(&mut self, ctx: &mut Ctx, cmd: &UiCommand) -> Option<CommandResult> {
        let session = matches!(ctx.mode, EngineMode::Session { .. });
        let done = Some(CommandResult::done());
        let refuse = |c| Some(CommandResult::error(c));
        match cmd {
            UiCommand::SetPrompts {
                lang,
                enabled,
                templates,
            } => {
                self.prompts.lang = *lang;
                self.prompts.enabled = *enabled;
                self.prompts.templates = templates.clone();
                if !enabled {
                    self.player.clear();
                }
                done
            }
            UiCommand::SetVoxSensitivity { level } => {
                self.vox_base = sensitivity(*level);
                self.apply_sensitivity(ctx.now);
                done
            }
            UiCommand::SetPttStyle { style } => {
                if *style != self.ptt_style && self.fsm.latched() && self.in_session {
                    // Leaving latch style releases a latched burst.
                    self.talk_input(ctx, TalkInput::LatchToggle);
                }
                self.ptt_style = *style;
                done
            }
            UiCommand::SetBuiltinMicTx { allowed } => {
                self.builtin_tx_allowed = *allowed;
                if self.in_session {
                    let on = self.builtin_mute_flag && !allowed;
                    self.talk_input(ctx, TalkInput::BuiltinMicMute(on));
                }
                self.ui_dirty = true;
                done
            }
            UiCommand::Ptt { down } if session => {
                if !self.live {
                    return refuse(ErrorCode::NotLive);
                }
                self.ptt(ctx, *down);
                done
            }
            UiCommand::GoLive {} if session => {
                if self.live {
                    return refuse(ErrorCode::AlreadyLive);
                }
                ctx.fx.audio(AudioCommand::OpenSession {
                    profile: profile(AudioSessionKind::Ride),
                });
                ctx.fx.audio(AudioCommand::StartCapture);
                ctx.fx.diag(DiagEvent::CaptureStart { why: "goLive" });
                let riders = Self::riders(ctx);
                self.request(ctx, MediaRequest::GoLive { riders });
                done
            }
            UiCommand::ResumeMic {} if session => {
                if !self.live {
                    return refuse(ErrorCode::NotLive);
                }
                ctx.fx.audio(AudioCommand::StartCapture);
                ctx.fx.diag(DiagEvent::CaptureStart { why: "resumeMic" });
                self.request(ctx, MediaRequest::ResumeMic);
                done
            }
            UiCommand::SetTalkMode { mode } if session => {
                self.request(ctx, MediaRequest::SetTalkMode(*mode));
                done
            }
            UiCommand::AudioCheck(op) => {
                if ctx.is_lab() || self.live {
                    return refuse(ErrorCode::WrongMode);
                }
                match op {
                    AudioCheckOp::Start { mode } => {
                        ctx.fx.audio(AudioCommand::OpenSession {
                            profile: profile(AudioSessionKind::Check),
                        });
                        ctx.fx.audio(AudioCommand::StartCapture);
                        ctx.fx.diag(DiagEvent::CaptureStart { why: "audioCheck" });
                        self.request(ctx, MediaRequest::AudioCheckStart(*mode));
                    }
                    AudioCheckOp::Stop {} => {
                        if self.check.is_some() {
                            ctx.fx.audio(AudioCommand::StopCapture);
                            ctx.fx.diag(DiagEvent::CaptureStop { why: "audioCheck" });
                            ctx.fx.audio(AudioCommand::CloseSession);
                        }
                        self.request(ctx, MediaRequest::AudioCheckStop);
                    }
                }
                done
            }
            _ => None,
        }
    }

    /// Applies one session request (from the session module or the fallback above).
    pub(super) fn request(&mut self, ctx: &mut Ctx, req: MediaRequest) {
        match req {
            MediaRequest::Identity(id) => self.set_identity(id),
            MediaRequest::GoLive { riders } => {
                if !self.in_session {
                    self.begin_session(ctx);
                }
                self.ensure_codec(ctx);
                self.live = true;
                self.check = None;
                self.chain.reset();
                self.identity(ctx);
                self.render_at_go_live(ctx, riders);
                self.ui_dirty = true;
            }
            MediaRequest::ResumeMic => {
                self.chain.reset();
                self.cues.capture_lost = false;
            }
            MediaRequest::EndSession => self.end_session(),
            MediaRequest::SetTalkMode(m) => {
                self.talk_input(ctx, TalkInput::SetMode(talk_mode(m)));
                self.ui_dirty = true;
            }
            // A check never runs while live (§3.2): capture belongs to the ride.
            MediaRequest::AudioCheckStart(_) if self.live => {}
            MediaRequest::AudioCheckStart(m) => {
                self.ensure_codec(ctx);
                self.chain.reset();
                self.check = Some(CheckRun::new(m));
            }
            MediaRequest::AudioCheckStop => self.check = None,
            MediaRequest::Cue(c) => self.play_cue(ctx, &c),
            MediaRequest::MemberNames(v) => {
                let fresh: Vec<String> = v
                    .iter()
                    .filter(|(id, n)| self.names.get(id) != Some(n))
                    .map(|(_, n)| n.clone())
                    .collect();
                self.names = v.into_iter().collect();
                if self.live && self.prompts.enabled {
                    let items: Vec<_> = fresh
                        .into_iter()
                        .flat_map(|n| {
                            [
                                (mesh_api::PromptKey::MemberJoined, Some(n.clone()), false),
                                (mesh_api::PromptKey::MemberLeft, Some(n), false),
                            ]
                        })
                        .collect();
                    self.render_prompts(ctx, &items);
                }
                self.ui_dirty = true;
            }
            MediaRequest::DemoStart => self.render_demo_lines(ctx),
            MediaRequest::FloorRejected { burst, reason } => {
                if self.in_session {
                    self.reject_me(ctx, burst, reason);
                }
            }
        }
    }
}
