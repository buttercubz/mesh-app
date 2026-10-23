//! Scripted talkers: TALK START, then a 40 ms BLE16 VOICE frame every 40 ms with
//! burst-relative `ts`, TALK KEEP every second and TALK STOP, originated at their node
//! exactly as the media outbox does (TALK on P0, VOICE on P1 with the `VOICE_MAX_AGE`
//! deadline; the `seq` comes from the node's shared per-origin counter).

#![allow(dead_code)]

use mesh_core::Instant;
use mesh_core::links::Prio;
use mesh_wire::mesh::{
    Bandwidth, EncoderProfile, FrameDur, MeshHeader, MeshPayload, PacketType, Talk, TalkMode,
    TalkPrio, TalkState, Voice, VoiceCfg, encode_packet,
};

use crate::world::World;

pub const FRAME_US: u64 = 40_000;
pub const OPUS_BYTES: usize = 80;

/// One scripted talker.
#[derive(Debug, Clone)]
pub struct Talker {
    pub node: usize,
    pub burst: u8,
    pub leader: bool,
    started: Option<u64>,
    next: u64,
    next_keep: u64,
    pub frames: u32,
    pub stopped: bool,
}

impl Talker {
    pub fn new(node: usize, burst: u8) -> Talker {
        Talker {
            node,
            burst,
            leader: false,
            started: None,
            next: 0,
            next_keep: 0,
            frames: 0,
            stopped: false,
        }
    }

    /// The header; the origin assigns `seq` when it originates the packet.
    fn header(&self, w: &World, ptype: PacketType) -> MeshHeader {
        let mut h = MeshHeader::new(ptype, w.nodes[self.node].node_id, w.sess(self.node), 0);
        h.leader = self.leader;
        h
    }

    fn talk(&mut self, w: &mut World, state: TalkState) {
        let elapsed = self.started.map_or(0, |s| ((w.now - s) / 10_000) as u16);
        let t = Talk {
            burst: self.burst,
            state,
            prio: if self.leader {
                TalkPrio::Leader
            } else {
                TalkPrio::Member
            },
            mode: TalkMode::Ptt,
            profile: EncoderProfile::Ble16,
            elapsed_10ms: if state == TalkState::Start {
                0
            } else {
                elapsed
            },
        };
        let h = self.header(w, PacketType::Talk);
        let body = encode_packet(&h, &MeshPayload::Talk(t)).expect("talk");
        w.inject(self.node, (Prio::P0, body, None));
    }

    /// Call every simulation step: sends whatever is due now.
    pub fn tick(&mut self, w: &mut World) {
        if self.stopped {
            return;
        }
        let now = w.now;
        if self.started.is_none() {
            self.started = Some(now);
            self.next = now;
            self.next_keep = now + 1_000_000;
            self.talk(w, TalkState::Start);
        }
        if now >= self.next_keep {
            self.next_keep += 1_000_000;
            self.talk(w, TalkState::Keep);
        }
        if now >= self.next {
            self.next += FRAME_US;
            let opus = [0x5Au8; OPUS_BYTES];
            let v = Voice {
                burst: self.burst,
                ts: (self.frames * 4) as u16,
                cfg: VoiceCfg::new(
                    FrameDur::Ms40,
                    Bandwidth::Wide,
                    false,
                    false,
                    self.frames == 0,
                    false,
                ),
                opus: &opus,
            };
            let mut h = self.header(w, PacketType::Voice);
            h.ttl = 4;
            let body = encode_packet(&h, &MeshPayload::Voice(v)).expect("voice");
            w.inject(self.node, (Prio::P1, body, Some(Instant(now + 150_000))));
            self.frames += 1;
        }
    }

    pub fn stop(&mut self, w: &mut World) {
        if !self.stopped && self.started.is_some() {
            self.talk(w, TalkState::Stop);
        }
        self.stopped = true;
    }
}

/// Steps the world for `ms` while the talkers talk.
pub fn run_talking(w: &mut World, talkers: &mut [Talker], ms: u64) {
    let end = w.now + ms * 1_000;
    while w.now < end {
        for t in talkers.iter_mut() {
            t.tick(w);
        }
        w.step_once();
    }
}
