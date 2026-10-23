//! The session → media seam ([`MediaRequest`]): what the session module (WP-10B) tells
//! the media module. Each request maps 1:1 to a media action, so the same code runs
//! whether the request came from the session module or from this module's own fallback
//! command handling.

use mesh_api::{AudioCheckMode, PromptKey, TalkMode};
use mesh_voice::Earcon;

/// This node's identity in the open group (from the group module).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MediaIdentity {
    /// Per-group `node_id` (MeshPacket `origin`).
    pub node: u16,
    /// This node is the group leader (TALK `prio` 2, header LEADER flag).
    pub leader: bool,
    /// Current epoch (floor hash input, §6.2).
    pub epoch: u32,
    /// Group policy: leader priority on (default) or off.
    pub leader_priority: bool,
}

/// A cue to play: an earcon (the guaranteed cue) and/or a spoken prompt. A prompt that
/// is off, not rendered yet or not renderable falls back to its earcon (§7.7).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cue {
    pub earcon: Option<Earcon>,
    pub prompt: Option<PromptKey>,
    /// Template parameter (`{{name}}` or `{{n}}`), if the prompt has one.
    pub param: Option<String>,
}

impl Cue {
    /// An earcon only.
    pub const fn earcon(e: Earcon) -> Self {
        Cue {
            earcon: Some(e),
            prompt: None,
            param: None,
        }
    }

    /// An earcon plus a prompt without parameter.
    pub const fn with_prompt(e: Option<Earcon>, key: PromptKey) -> Self {
        Cue {
            earcon: e,
            prompt: Some(key),
            param: None,
        }
    }

    /// Adds the template parameter.
    pub fn param(mut self, p: impl Into<String>) -> Self {
        self.param = Some(p.into());
        self
    }
}

/// Session → media requests.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MediaRequest {
    /// The group identity is known (or changed: new epoch, leader handover).
    Identity(MediaIdentity),
    /// Go live: capture runs (the session emitted `StartCapture`); `riders` fills the
    /// `intercomLive{n}` prompt; fixed prompts and known names are rendered now.
    GoLive {
        riders: u8,
    },
    /// Capture restarted after a loss (`resumeMic`).
    ResumeMic,
    /// The session ended (left the ride, engine stop): audio state is dropped.
    EndSession,
    /// Talk mode.
    SetTalkMode(TalkMode),
    /// Audio check start/stop (the session runs its lifecycle and audio commands).
    AudioCheckStart(AudioCheckMode),
    AudioCheckStop,
    /// Play a cue.
    Cue(Cue),
    /// Display names of members known now (talkers, name prompts rendered at go-live or
    /// on demand).
    MemberNames(Vec<(u16, String)>),
    /// Demo ride started (foreground): render the `demo.*` lines for the bots (§3.4).
    DemoStart,
    /// One of my own links refused my burst (the mesh glue's relay admission, §6.3: the
    /// link already carries `cap` bursts that outrank it): handled as a FLOOR_REJECT for
    /// me, the talk FSM goes BLOCKED (§6.4).
    FloorRejected {
        burst: u8,
        reason: mesh_wire::mesh::FloorRejectReason,
    },
}
