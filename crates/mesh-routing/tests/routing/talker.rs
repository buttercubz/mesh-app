//! A scripted origin (a minimal §6.4 talk FSM) driving a [`Net`]: `local_admit` at the
//! onset, TALK START + 40 ms VOICE frames (`ts = 4n`), TALK KEEP every 1 s, TALK STOP at
//! the end, and BLOCKED on a FLOOR_REJECT for its burst (optionally retrying when
//! `local_admit` says a slot freed).

use mesh_routing::{BurstKey, Instant, NodeId, RouterAction};
use mesh_wire::mesh::{
    EncoderProfile, PacketType, Talk, TalkMode, TalkPrio, TalkState, Voice, VoiceCfg,
};

use crate::net::Net;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    Waiting,
    Talking { started: Instant, frames: u32 },
    Blocked,
    Done,
}

#[derive(Debug, Clone)]
pub struct Talker {
    pub node: NodeId,
    pub leader: bool,
    pub start: Instant,
    pub stop: Instant,
    pub retry: bool,
    /// Ignore FLOOR_REJECT (tests hard enforcement at relays).
    pub ignore_rejects: bool,
    /// Start without `local_admit` (a misbehaving origin).
    pub force: bool,
    pub burst: u8,
    pub phase: Phase,
    pub bursts_started: Vec<BurstKey>,
    pub rejected: Vec<BurstKey>,
    seen_actions: usize,
}

impl Talker {
    pub fn new(node: NodeId, start: Instant, stop: Instant) -> Self {
        Talker {
            node,
            leader: false,
            start,
            stop,
            retry: false,
            ignore_rejects: false,
            force: false,
            burst: 0,
            phase: Phase::Waiting,
            bursts_started: Vec::new(),
            rejected: Vec::new(),
            seen_actions: 0,
        }
    }

    fn prio(&self) -> TalkPrio {
        if self.leader {
            TalkPrio::Leader
        } else {
            TalkPrio::Member
        }
    }

    fn talk(&self, net: &mut Net, state: TalkState, elapsed_10ms: u16) {
        let t = Talk {
            burst: self.burst,
            state,
            prio: self.prio(),
            mode: TalkMode::Ptt,
            profile: EncoderProfile::Ble16,
            elapsed_10ms,
        };
        net.originate(self.node, PacketType::Talk, self.leader, t.encode());
    }

    fn voice(&self, net: &mut Net, n: u32) {
        let v = Voice {
            burst: self.burst,
            ts: (n * 4) as u16,
            cfg: VoiceCfg(0x80),
            opus: &[0x55; 80],
        };
        net.originate(
            self.node,
            PacketType::Voice,
            self.leader,
            v.encode().expect("voice"),
        );
    }

    pub fn key(&self) -> BurstKey {
        let sess = net_sess(self.node);
        BurstKey::new(self.node, sess, self.burst)
    }

    /// One 10 ms step at `net.now`.
    pub fn step(&mut self, net: &mut Net) {
        let now = net.now;
        // FLOOR_REJECT for my current burst?
        let mut rejected = false;
        for (_, node, a) in &net.actions[self.seen_actions..] {
            if *node == self.node
                && let RouterAction::RejectedMe { burst, .. } = a
                && *burst == self.burst
            {
                rejected = true;
            }
        }
        self.seen_actions = net.actions.len();
        match self.phase {
            Phase::Done => {}
            Phase::Waiting | Phase::Blocked if now >= self.stop => self.phase = Phase::Done,
            Phase::Waiting | Phase::Blocked => {
                if now < self.start || (self.phase == Phase::Blocked && !self.retry) {
                    return;
                }
                let prio = self.prio() as u8;
                let next = self.burst.wrapping_add(1);
                let admit = self.force || net.routers[&self.node].local_admit(now, next, prio);
                if admit {
                    self.burst = next;
                    self.bursts_started.push(self.key());
                    self.talk(net, TalkState::Start, 0);
                    self.voice(net, 0);
                    self.phase = Phase::Talking {
                        started: now,
                        frames: 1,
                    };
                } else if self.phase == Phase::Waiting {
                    self.phase = Phase::Blocked;
                }
            }
            Phase::Talking { started, frames } => {
                let elapsed = now.since(started);
                if rejected && !self.ignore_rejects {
                    self.rejected.push(self.key());
                    self.talk(net, TalkState::Stop, (elapsed / 10_000) as u16);
                    self.phase = Phase::Blocked;
                    return;
                }
                if now >= self.stop {
                    self.talk(net, TalkState::Stop, (elapsed / 10_000) as u16);
                    self.phase = Phase::Done;
                    return;
                }
                let mut frames = frames;
                while u64::from(frames) * 40_000 <= elapsed {
                    self.voice(net, frames);
                    frames += 1;
                }
                if elapsed > 0 && elapsed % 1_000_000 < 10_000 {
                    self.talk(net, TalkState::Keep, (elapsed / 10_000) as u16);
                }
                self.phase = Phase::Talking { started, frames };
            }
        }
    }
}

/// The `sess` the test [`Net`] gives a node.
pub fn net_sess(node: NodeId) -> u8 {
    (node % 250 + 1) as u8
}

/// Runs `talkers` on `net` until `until`, in 10 ms steps.
pub fn run(net: &mut Net, talkers: &mut [Talker], until: Instant) {
    while net.now < until {
        let next = net.now.plus_ms(10);
        net.run_until(next);
        for t in talkers.iter_mut() {
            t.step(net);
        }
    }
}
