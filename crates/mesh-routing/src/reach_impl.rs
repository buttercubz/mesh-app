//! `Reachability` behavior (see `reach.rs` for the rules).

use std::collections::{BTreeMap, BTreeSet};

use mesh_wire::mesh::Presence;
use mesh_wire::serial::peer_behind;

use super::{IslandView, MemberReach, ReachEvent, Reachability, VoiceReach};
use crate::consts::{ALONE_US, EVENT_AGGREGATION_US, MEMBER_LOST_US, PARTITION_STABLE_US};
use crate::types::{Instant, NodeId};

impl Reachability {
    pub fn new(me: NodeId, now: Instant, voice_ttl: u8, voice_max_age_us: u64) -> Self {
        Reachability {
            me,
            roster: BTreeMap::new(),
            members: BTreeMap::new(),
            my_roster_version: 0,
            my_epoch: 0,
            voice_ttl,
            voice_max_age_us,
            pending_lost: BTreeSet::new(),
            pending_back: BTreeSet::new(),
            agg_since: None,
            last_reachable: now,
            alone_sent: false,
            islands_published: None,
            islands_changed: None,
        }
    }

    /// The roster: `(node_id, roster index)` of every non-denied entry.
    pub fn set_roster(&mut self, entries: &[(NodeId, u8)]) {
        self.roster = entries.iter().copied().collect();
        for m in self.members.values_mut() {
            m.roster_index = self.roster.get(&m.node_id).copied();
        }
    }

    /// My authoritative roster version and epoch (anti-entropy comparison).
    pub fn set_my_versions(&mut self, roster_version: u32, epoch: u32) {
        self.my_roster_version = roster_version;
        self.my_epoch = epoch;
    }

    fn my_index(&self) -> Option<u8> {
        self.roster.get(&self.me).copied()
    }

    /// An authenticated, deduplicated PRESENCE from `origin` (session `sess`), received
    /// with header `hops` and path age `age_us`.
    #[allow(clippy::too_many_arguments)]
    pub fn on_presence(
        &mut self,
        now: Instant,
        origin: NodeId,
        sess: u8,
        hops: u8,
        age_us: u64,
        p: &Presence,
        all_aware: bool,
    ) -> Vec<ReachEvent> {
        let mut ev = Vec::new();
        if origin == self.me {
            return ev;
        }
        if hops == 0
            && (peer_behind(self.my_roster_version, p.roster_ver16)
                || peer_behind(self.my_epoch, p.epoch16))
        {
            ev.push(ReachEvent::PeerBehind(origin));
        }
        let idx = self.roster.get(&origin).copied();
        let entry = self.members.entry(origin).or_insert_with(|| MemberReach {
            node_id: origin,
            roster_index: idx,
            last_heard: now,
            sess,
            traversals: hops.saturating_add(1),
            age_us,
            all_aware,
            presence: *p,
            lost: false,
        });
        if entry.lost {
            entry.lost = false;
            if !self.pending_lost.remove(&origin) {
                self.pending_back.insert(origin);
                self.agg_since.get_or_insert(now);
            }
        }
        entry.last_heard = now;
        entry.sess = sess;
        entry.traversals = hops.saturating_add(1);
        entry.age_us = age_us;
        entry.all_aware = all_aware;
        entry.presence = *p;
        entry.roster_index = idx;
        self.last_reachable = now;
        self.alone_sent = false;
        ev
    }

    /// `true` when `node` was heard within 6 s.
    pub fn reachable(&self, now: Instant, node: NodeId) -> bool {
        self.members
            .get(&node)
            .is_some_and(|m| now.since(m.last_heard) < MEMBER_LOST_US)
    }

    pub fn member(&self, node: NodeId) -> Option<&MemberReach> {
        self.members.get(&node)
    }

    pub fn members(&self) -> impl Iterator<Item = &MemberReach> {
        self.members.values()
    }

    /// Voice reach of `node`.
    pub fn voice_reach(&self, now: Instant, node: NodeId) -> VoiceReach {
        match self.members.get(&node) {
            Some(m) if now.since(m.last_heard) < MEMBER_LOST_US => VoiceReach::classify(
                m.traversals,
                m.age_us,
                m.all_aware,
                self.voice_ttl,
                self.voice_max_age_us,
            ),
            _ => VoiceReach::No,
        }
    }

    /// My PRESENCE `heard` bitmap: roster members heard within 6 s.
    pub fn heard_bitmap(&self, now: Instant) -> u32 {
        self.members
            .values()
            .filter(|m| now.since(m.last_heard) < MEMBER_LOST_US)
            .filter_map(|m| m.roster_index)
            .filter(|&i| i < 32)
            .fold(0u32, |acc, i| acc | (1 << i))
    }

    /// Reachable members whose bitmap has my bit.
    pub fn who_hears_me(&self, now: Instant) -> Vec<NodeId> {
        let Some(mine) = self.my_index() else {
            return Vec::new();
        };
        self.members
            .values()
            .filter(|m| now.since(m.last_heard) < MEMBER_LOST_US && m.presence.heard_index(mine))
            .map(|m| m.node_id)
            .collect()
    }

    /// Members I hear that do not hear me.
    pub fn one_way(&self, now: Instant) -> Vec<NodeId> {
        let Some(mine) = self.my_index() else {
            return Vec::new();
        };
        self.members
            .values()
            .filter(|m| m.roster_index.is_some())
            .filter(|m| now.since(m.last_heard) < MEMBER_LOST_US && !m.presence.heard_index(mine))
            .map(|m| m.node_id)
            .collect()
    }

    /// Connected components of the "hears" graph over the roster (edges from my heard
    /// set and the bitmaps of reachable members).
    pub fn islands(&self, now: Instant) -> IslandView {
        let mut nodes: BTreeSet<NodeId> = self.roster.keys().copied().collect();
        nodes.insert(self.me);
        let by_index: BTreeMap<u8, NodeId> = self.roster.iter().map(|(n, i)| (*i, *n)).collect();
        let mut adj: BTreeMap<NodeId, BTreeSet<NodeId>> = BTreeMap::new();
        let mut link = |a: NodeId, b: NodeId| {
            if a != b {
                adj.entry(a).or_default().insert(b);
                adj.entry(b).or_default().insert(a);
            }
        };
        for m in self.members.values() {
            if now.since(m.last_heard) >= MEMBER_LOST_US || m.roster_index.is_none() {
                continue;
            }
            link(self.me, m.node_id);
            for i in 0..32u8 {
                if m.presence.heard_index(i)
                    && let Some(&v) = by_index.get(&i)
                {
                    link(m.node_id, v);
                }
            }
        }
        let mut seen: BTreeSet<NodeId> = BTreeSet::new();
        let mut islands = Vec::new();
        let order: Vec<NodeId> = std::iter::once(self.me)
            .chain(nodes.iter().copied().filter(|n| *n != self.me))
            .collect();
        for start in order {
            if !seen.insert(start) {
                continue;
            }
            let mut comp = vec![start];
            let mut stack = vec![start];
            while let Some(u) = stack.pop() {
                for &v in adj.get(&u).into_iter().flatten() {
                    if nodes.contains(&v) && seen.insert(v) {
                        comp.push(v);
                        stack.push(v);
                    }
                }
            }
            comp.sort_unstable();
            islands.push(comp);
        }
        IslandView { islands }
    }

    /// Timers: losses, aggregation flush, `alone`, `partition`.
    pub fn tick(&mut self, now: Instant) -> Vec<ReachEvent> {
        let mut ev = Vec::new();
        for m in self.members.values_mut() {
            if !m.lost && now.since(m.last_heard) >= MEMBER_LOST_US {
                m.lost = true;
                if !self.pending_back.remove(&m.node_id) {
                    self.pending_lost.insert(m.node_id);
                    self.agg_since.get_or_insert(now);
                }
            }
        }
        if let Some(since) = self.agg_since
            && now.since(since) >= EVENT_AGGREGATION_US
        {
            if !self.pending_lost.is_empty() {
                ev.push(ReachEvent::MemberLost(
                    self.pending_lost.iter().copied().collect(),
                ));
            }
            if !self.pending_back.is_empty() {
                ev.push(ReachEvent::MemberBack(
                    self.pending_back.iter().copied().collect(),
                ));
            }
            self.pending_lost.clear();
            self.pending_back.clear();
            self.agg_since = None;
        }
        let any = self
            .members
            .values()
            .any(|m| now.since(m.last_heard) < MEMBER_LOST_US);
        if any {
            self.last_reachable = now;
        } else if !self.alone_sent && now.since(self.last_reachable) >= ALONE_US {
            self.alone_sent = true;
            ev.push(ReachEvent::Alone);
        }
        let count = self.islands(now).count();
        let Some(published) = self.islands_published else {
            self.islands_published = Some(count);
            return ev;
        };
        if count == published {
            self.islands_changed = None;
        } else {
            match self.islands_changed {
                Some((since, c)) if c == count => {
                    if now.since(since) >= PARTITION_STABLE_US {
                        self.islands_published = Some(count);
                        self.islands_changed = None;
                        ev.push(ReachEvent::Partition(count));
                    }
                }
                _ => self.islands_changed = Some((now, count)),
            }
        }
        ev
    }

    /// Forget an origin entirely (left the group).
    pub fn remove(&mut self, node: NodeId) {
        self.members.remove(&node);
        self.pending_back.remove(&node);
        self.pending_lost.remove(&node);
    }
}
