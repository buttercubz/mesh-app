//! Helmet cues (DESIGN §0 "every failure produces three things", §6.5, §7.6, §7.7).
//!
//! The session decides *when* a cue plays; the media module (WP-10C) plays it. Each cue
//! is posted on the engine's inter-module bus as
//! [`MediaRequest::Cue`](crate::media::MediaRequest) with its earcon (the guaranteed
//! cue) and its spoken prompt, whose template parameter carries the names aggregated
//! over 5 s or a rider count. A cue that reports a failure also raises its notice and
//! the lock-screen notification ([`alert`]).

use mesh_api::{NoticeKey, Params, PromptKey};
use mesh_voice::Earcon;

use crate::effects::{Effect, Notification};
use crate::engine::Ctx;
use crate::media::{self, MediaRequest};

/// Cues the session requests.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Cue {
    /// The ride started.
    RideStarted,
    /// Members joined or came back (names aggregated over 5 s, `"Ana, Bob"`).
    MemberJoined(String),
    /// One member out of range, or one member left the ride.
    MemberLeft(String),
    /// Several members out of range at once.
    OutOfRange(usize),
    /// Nobody reachable.
    Alone,
    /// Capture lost (ladder L2/L3, listen-only).
    MicLost,
    /// Capture back.
    MicBack,
    /// The headset went away while live (TX muted).
    HeadsetLost,
    /// Bluetooth off / airplane mode (repeated every 60 s while off).
    RadioOff,
    /// Battery below 15 % (once per 10 min).
    BatteryLow,
    /// The leader handed over to `name`.
    LeaderChanged(String),
}

impl Cue {
    /// Earcon, prompt and template parameter of the cue (§6.5, §7.7).
    pub fn media(&self) -> media::Cue {
        let with = media::Cue::with_prompt;
        match self {
            Cue::RideStarted => with(Some(Earcon::RideStarted), PromptKey::RideStarted),
            Cue::MemberJoined(names) => {
                with(Some(Earcon::MemberJoined), PromptKey::MemberJoined).param(names.clone())
            }
            Cue::MemberLeft(names) => {
                with(Some(Earcon::MemberLost), PromptKey::MemberLeft).param(names.clone())
            }
            Cue::OutOfRange(n) => {
                with(Some(Earcon::MemberLost), PromptKey::RidersOutOfRange).param(n.to_string())
            }
            Cue::Alone => with(Some(Earcon::Alone), PromptKey::Alone),
            Cue::MicLost => with(Some(Earcon::MicLost), PromptKey::MicPaused),
            Cue::MicBack => with(None, PromptKey::MicBack),
            Cue::HeadsetLost => with(Some(Earcon::HeadsetLost), PromptKey::HeadsetLost),
            Cue::RadioOff => with(Some(Earcon::BluetoothOff), PromptKey::BluetoothOff),
            Cue::BatteryLow => with(None, PromptKey::BatteryLow),
            Cue::LeaderChanged(name) => with(None, PromptKey::LeaderChanged).param(name.clone()),
        }
    }
}

/// Requests `cue` from the media module.
pub fn request(ctx: &mut Ctx, cue: &Cue) {
    ctx.post_media(MediaRequest::Cue(cue.media()));
}

/// Requests `cue` and raises `key` as notice and lock-screen notification.
pub fn alert(ctx: &mut Ctx, cue: &Cue, key: NoticeKey, params: Params) {
    request(ctx, cue);
    ctx.fx.push(Effect::Notify(Notification {
        key,
        params: params.clone(),
    }));
    ctx.notice(key, params);
}
