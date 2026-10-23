//! The media part of the snapshot (DESIGN §3.7, §6.5): `talkers[]` (me first while I
//! talk, then the live bursts in floor rank order, ≤ 4), the latency estimate, the
//! TX-muted-without-headset flag and the talk state of the session block.

use mesh_api::{Snapshot, TalkState as ApiTalkState, Talker};
use mesh_voice::TalkState;

use super::state::Media;
use crate::engine::Ctx;

/// Talkers shown at most (= streams mixed, §6.3).
const MAX_TALKERS: usize = mesh_routing::consts::MIX_STREAMS;

/// Levels are quantized so the snapshot only changes on audible differences.
fn quantize(v: f32) -> f64 {
    (f64::from(v.clamp(0.0, 1.0)) * 20.0).round() / 20.0
}

pub(super) fn api_talk_state(s: TalkState) -> ApiTalkState {
    match s {
        TalkState::Muted => ApiTalkState::Muted,
        TalkState::Idle => ApiTalkState::Idle,
        TalkState::Talking => ApiTalkState::Talking,
        TalkState::Blocked => ApiTalkState::Blocked,
    }
}

impl Media {
    fn name_of(&self, node: u16) -> String {
        self.names.get(&node).cloned().unwrap_or_default()
    }

    pub(super) fn fill_snapshot(&self, ctx: &Ctx, s: &mut Snapshot) {
        if ctx.is_lab() || !self.in_session {
            return;
        }
        let now = ctx.now;
        let mut talkers = Vec::new();
        if let (Some(b), Some(me)) = (self.burst, self.identity) {
            talkers.push(Talker {
                node_id: me.node,
                name: self.name_of(me.node),
                hops: 0,
                level: quantize(ctx.device.input_level as f32),
                since_ms: (now.since(b.started) / 1_000).min(u64::from(u32::MAX)) as u32,
            });
        }
        for e in self.floor.ranked() {
            if talkers.len() >= MAX_TALKERS {
                break;
            }
            let track = self.streams.iter().find(|t| {
                t.key
                    .is_some_and(|k| k.origin == e.key.origin && k.sess == e.key.sess)
            });
            let since = (now.us() as i64 - e.t_origin_us).max(0) as u64 / 1_000;
            talkers.push(Talker {
                node_id: e.key.origin,
                name: self.name_of(e.key.origin),
                hops: track.map_or(1, |t| t.hops),
                level: quantize(track.map_or(0.0, |t| t.level)),
                since_ms: since.min(u64::from(u32::MAX)) as u32,
            });
        }
        s.talkers = talkers;
        s.latency = self.latency.info();
        s.audio.tx_muted_no_headset = self.builtin_mute_flag && !self.builtin_tx_allowed;
        if let Some(si) = s.session.as_mut() {
            // The session module owns the talk mode setting (`si.mode`) and hands this
            // module only the gated mode (muted in the lobby and at ladder L2/L3), so
            // the talk *state* is reported from here, while capture is live.
            if self.live {
                si.talk = api_talk_state(self.fsm.state());
            }
            si.live |= self.live;
        }
    }
}
