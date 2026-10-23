//! Floor table (DESIGN §6.2): `FloorTable { active: ≤ 8 bursts }`. An entry is created by
//! the first TALK START or VOICE packet of a burst (TALK KEEP too, so a node that missed
//! START joins within one KEEP period) and removed on TALK STOP, on FLOOR_REJECT for it,
//! or `FLOOR_IDLE_RELEASE` after its last packet.
//!
//! A rejected burst stays in the table as a **tombstone** until STOP or idle release: it
//! no longer ranks, is admitted on no link and is never mixed, but a late VOICE frame of
//! the (stopping) burst cannot re-create it and re-trigger rejection.

use mesh_wire::mesh::{EncoderProfile, Talk, TalkMode, TalkState};

use super::rank::{BurstKey, Rank, t_origin_us};
use crate::consts::{FLOOR_IDLE_RELEASE_US, FLOOR_TABLE_MAX, PRIO_LEADER, PRIO_MEMBER};
use crate::types::{Instant, NodeId};

/// Why a burst left the live set.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EndReason {
    Stop,
    Rejected,
    Idle,
    /// A newer burst of the same origin replaced it.
    Replaced,
    /// Evicted from a full table by a higher-ranked burst.
    Evicted,
}

/// Floor table change.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FloorEvent {
    Started(BurstKey),
    Ended(BurstKey, EndReason),
}

/// One burst.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BurstEntry {
    pub key: BurstKey,
    pub rank: Rank,
    pub t_origin_us: i64,
    pub first_seen: Instant,
    pub last_packet: Instant,
    pub mode: Option<TalkMode>,
    pub profile: Option<EncoderProfile>,
    /// Rejected (tombstone): not live.
    pub rejected: bool,
    /// This node already flooded a FLOOR_REJECT{CAP|CONGESTION} for it.
    pub rejected_by_me: bool,
    /// This node already sent its near-talker FLOOR_REJECT{OUTRANKED}.
    pub near_rejected: bool,
}

impl BurstEntry {
    pub fn live(&self) -> bool {
        !self.rejected
    }
}

/// The floor table of one node.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FloorTable {
    pub epoch: u32,
    pub t_tie_us: u64,
    pub leader_priority: bool,
    entries: Vec<BurstEntry>,
}

/// Priority of a burst: leader 2 when the policy is on, else member 1.
pub fn prio_of(leader: bool, leader_priority: bool) -> u8 {
    if leader && leader_priority {
        PRIO_LEADER
    } else {
        PRIO_MEMBER
    }
}

impl FloorTable {
    pub fn new(epoch: u32, t_tie_us: u64, leader_priority: bool) -> Self {
        FloorTable {
            epoch,
            t_tie_us,
            leader_priority,
            entries: Vec::with_capacity(FLOOR_TABLE_MAX),
        }
    }

    /// Every entry (live and tombstones).
    pub fn entries(&self) -> &[BurstEntry] {
        &self.entries
    }

    pub fn get(&self, key: BurstKey) -> Option<&BurstEntry> {
        self.entries.iter().find(|e| e.key == key)
    }

    pub(crate) fn get_mut(&mut self, key: BurstKey) -> Option<&mut BurstEntry> {
        self.entries.iter_mut().find(|e| e.key == key)
    }

    /// Live bursts, best first.
    pub fn ranked(&self) -> Vec<&BurstEntry> {
        let mut v: Vec<&BurstEntry> = self.entries.iter().filter(|e| e.live()).collect();
        v.sort_by_key(|a| a.rank);
        v
    }

    /// The top `n` live bursts (receivers mix at most 4, chosen by rank, §6.3).
    pub fn top(&self, n: usize) -> Vec<BurstKey> {
        self.ranked().into_iter().take(n).map(|e| e.key).collect()
    }

    /// `true` when `key` is live and among the top `n`.
    pub fn in_top(&self, key: BurstKey, n: usize) -> bool {
        self.top(n).contains(&key)
    }

    /// The rank this node would give a burst seen first now.
    pub fn rank_for(&self, key: BurstKey, prio: u8, t_origin_us: i64) -> Rank {
        Rank::new(prio, t_origin_us, key, self.epoch, self.t_tie_us)
    }

    /// A TALK packet (TALK carries its own `prio`; header `age_us`).
    pub fn on_talk(
        &mut self,
        now: Instant,
        key: BurstKey,
        talk: &Talk,
        age_us: u64,
    ) -> Vec<FloorEvent> {
        let prio = if self.leader_priority {
            talk.prio as u8
        } else {
            PRIO_MEMBER
        };
        match talk.state {
            TalkState::Stop => match self.remove(key) {
                Some(_) => vec![FloorEvent::Ended(key, EndReason::Stop)],
                None => Vec::new(),
            },
            TalkState::Start | TalkState::Keep => {
                let rel = u64::from(talk.elapsed_10ms) * 10_000;
                let t0 = t_origin_us(now, age_us, rel);
                let ev = self.observe(now, key, prio, t0);
                if let Some(e) = self.get_mut(key) {
                    e.mode = Some(talk.mode);
                    e.profile = Some(talk.profile);
                }
                ev
            }
        }
    }

    /// A VOICE frame with burst-relative `ts` (10 ms ticks) and header `age_us`.
    pub fn on_voice(
        &mut self,
        now: Instant,
        key: BurstKey,
        leader: bool,
        ts: u16,
        age_us: u64,
    ) -> Vec<FloorEvent> {
        let prio = prio_of(leader, self.leader_priority);
        let t0 = t_origin_us(now, age_us, u64::from(ts) * 10_000);
        self.observe(now, key, prio, t0)
    }

    /// FLOOR_REJECT for `key` (from anyone, or decided locally): tombstone it.
    pub fn on_reject(&mut self, key: BurstKey) -> Option<FloorEvent> {
        let e = self.get_mut(key)?;
        if e.rejected {
            return None;
        }
        e.rejected = true;
        Some(FloorEvent::Ended(key, EndReason::Rejected))
    }

    /// Idle release: bursts without voice/TALK for 600 ms leave the table.
    pub fn expire(&mut self, now: Instant) -> Vec<FloorEvent> {
        let mut ev = Vec::new();
        self.entries.retain(|e| {
            let keep = now.since(e.last_packet) < FLOOR_IDLE_RELEASE_US;
            if !keep && e.live() {
                ev.push(FloorEvent::Ended(e.key, EndReason::Idle));
            }
            keep
        });
        ev
    }

    fn remove(&mut self, key: BurstKey) -> Option<BurstEntry> {
        let i = self.entries.iter().position(|e| e.key == key)?;
        let e = self.entries.remove(i);
        e.live().then_some(e)
    }

    /// Creates or refreshes a burst. The rank is fixed by the first packet seen.
    fn observe(&mut self, now: Instant, key: BurstKey, prio: u8, t0: i64) -> Vec<FloorEvent> {
        if let Some(e) = self.get_mut(key) {
            e.last_packet = now;
            return Vec::new();
        }
        let mut ev = Vec::new();
        // One burst per origin: a newer burst (or a new sess) replaces the old one.
        let older: Vec<BurstKey> = self
            .entries
            .iter()
            .filter(|e| e.key.origin == key.origin)
            .map(|e| e.key)
            .collect();
        for k in older {
            if self.remove(k).is_some() {
                ev.push(FloorEvent::Ended(k, EndReason::Replaced));
            }
        }
        let rank = self.rank_for(key, prio, t0);
        if self.entries.len() >= FLOOR_TABLE_MAX {
            // Drop a tombstone first, else the lowest-ranked live burst if we outrank it.
            if let Some(i) = self.entries.iter().position(|e| !e.live()) {
                self.entries.remove(i);
            } else {
                let worst = self
                    .entries
                    .iter()
                    .enumerate()
                    .max_by(|a, b| a.1.rank.cmp(&b.1.rank))
                    .map(|(i, e)| (i, e.rank));
                match worst {
                    Some((i, w)) if rank.outranks(&w) => {
                        let e = self.entries.remove(i);
                        ev.push(FloorEvent::Ended(e.key, EndReason::Evicted));
                    }
                    _ => return ev,
                }
            }
        }
        self.entries.push(BurstEntry {
            key,
            rank,
            t_origin_us: t0,
            first_seen: now,
            last_packet: now,
            mode: None,
            profile: None,
            rejected: false,
            rejected_by_me: false,
            near_rejected: false,
        });
        ev.push(FloorEvent::Started(key));
        ev
    }

    /// The origin's `local_admit` (§6.4): my prospective burst `key` (priority `prio`,
    /// `eff_start = now + h·T_TIE`) ranks inside `cap_local` given my table. Bursts of my
    /// own origin are ignored.
    pub fn local_admit(&self, now: Instant, key: BurstKey, prio: u8, cap_local: u8) -> bool {
        let mine = self.rank_for(key, prio, now.us() as i64);
        let above = self
            .entries
            .iter()
            .filter(|e| e.live() && e.key.origin != key.origin && e.rank.outranks(&mine))
            .count();
        above < usize::from(cap_local)
    }

    /// Near-talker rejection (§6.3): VOICE with `hops ≤ 1` for a live burst outside the
    /// global top `group_cap` ⇒ one FLOOR_REJECT{OUTRANKED} (TTL 4) per burst. Returns
    /// `true` when this node must send it now.
    pub fn near_talker_reject(&mut self, key: BurstKey, hops: u8, group_cap: u8) -> bool {
        if hops > 1 || self.in_top(key, usize::from(group_cap)) {
            return false;
        }
        match self.get_mut(key) {
            Some(e) if e.live() && !e.near_rejected => {
                e.near_rejected = true;
                true
            }
            _ => false,
        }
    }

    /// Live bursts of `origin` (for the origin FSM and the snapshot `talkers[]`).
    pub fn bursts_of(&self, origin: NodeId) -> impl Iterator<Item = &BurstEntry> {
        self.entries
            .iter()
            .filter(move |e| e.live() && e.key.origin == origin)
    }
}
