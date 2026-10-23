//! Global assertions checked in every scenario (DESIGN §13.2): no `(key, nonce)` pair
//! is ever used twice (all modes, Lab included); no Lab or node id collision survives;
//! no link exceeds its cap; no panic (caught by the suite runner); plus the §9.2 role
//! rule (no Android-initiated link to an iPhone) and the §3.2 capture rule
//! (`StartCapture` never without an open session).
//!
//! The nonce check taps every unit a node puts on air (after the OS accepted it),
//! reassembles LinkFrames with the production framing and tracks SEC `kp_ctr` per
//! sending key. Each link direction derives fresh keys from fresh Noise ephemerals
//! (§8.4), so a key is identified by `(node, link, key-phase generation)`; within one
//! key the counter must strictly increase, which implies nonce uniqueness. LAB frames
//! carry no key and HS frames no AEAD nonce; they are counted.

use std::collections::BTreeMap;

use mesh_core::links::framing::{Framer, Inbound};
use mesh_core::{Instant, LinkUnit};
use mesh_wire::frame::{KpCtr, LinkFrame};

/// What went wrong.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ViolationKind {
    NonceReuse,
    IdCollision,
    LinkCap,
    StreamCap,
    RoleRule,
    CaptureWithoutSession,
    MalformedOwnFrame,
}

/// One failed global assertion.
#[derive(Debug, Clone, PartialEq)]
pub struct Violation {
    pub t: Instant,
    pub kind: ViolationKind,
    pub detail: String,
}

#[derive(Debug)]
struct Tap {
    framer: Framer,
    phase: Option<bool>,
    generation: u32,
}

#[derive(Debug, Clone, Copy, Default)]
struct KeyUse {
    last_ctr: Option<u32>,
    frames: u64,
}

/// Frame counts seen on air.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct FrameCounts {
    pub hs: u64,
    pub sec: u64,
    pub lab: u64,
    pub reject: u64,
}

/// The global assertion state of a world.
#[derive(Debug, Default)]
pub struct Audit {
    taps: BTreeMap<(usize, u32), Tap>,
    keys: BTreeMap<(usize, u32, u32), KeyUse>,
    pub frames: FrameCounts,
    id_collisions: BTreeMap<(u16, usize, usize), u64>,
    over_cap: BTreeMap<usize, u64>,
    over_episode: std::collections::BTreeSet<usize>,
    pub violations: Vec<Violation>,
    /// Highest number of simultaneous streams seen on any link, by kind (BLE, other).
    pub max_streams: (usize, usize),
    /// Episodes of a node above its BLE link max (see [`Audit::check_link_cap`]).
    pub over_max_episodes: u64,
}

impl Audit {
    pub fn new() -> Self {
        Self::default()
    }

    fn violate(&mut self, t: Instant, kind: ViolationKind, detail: String) {
        // Keep reports bounded: at most 64 violations of each kind.
        if self.violations.iter().filter(|v| v.kind == kind).count() < 64 {
            self.violations.push(Violation { t, kind, detail });
        }
    }

    /// A unit `node` put on air on its `link`.
    pub fn on_air(
        &mut self,
        t: Instant,
        node: usize,
        link: u32,
        unit: LinkUnit,
        att: u16,
        bytes: &[u8],
    ) {
        let tap = self.taps.entry((node, link)).or_insert_with(|| Tap {
            framer: Framer::new(unit, Some(att)),
            phase: None,
            generation: 0,
        });
        let frames = match tap.framer.inbound(bytes) {
            Inbound::Frames(f) => f,
            Inbound::Dropped => Vec::new(),
            Inbound::Fatal(e) => {
                let d = format!("node {node} link {link}: own stream broken: {e:?}");
                self.violate(t, ViolationKind::MalformedOwnFrame, d);
                return;
            }
        };
        for f in frames {
            self.on_frame(t, node, link, &f);
        }
    }

    fn on_frame(&mut self, t: Instant, node: usize, link: u32, frame: &[u8]) {
        let Ok(parsed) = LinkFrame::decode(frame) else {
            let d = format!("node {node} link {link}: undecodable own frame");
            self.violate(t, ViolationKind::MalformedOwnFrame, d);
            return;
        };
        match parsed {
            LinkFrame::Hs { .. } => self.frames.hs += 1,
            LinkFrame::Lab { .. } => self.frames.lab += 1,
            LinkFrame::Reject { .. } => self.frames.reject += 1,
            LinkFrame::Sec { kp_ctr, .. } => {
                self.frames.sec += 1;
                self.on_sec(t, node, link, kp_ctr);
            }
        }
    }

    fn on_sec(&mut self, t: Instant, node: usize, link: u32, kp: KpCtr) {
        let Some(tap) = self.taps.get_mut(&(node, link)) else {
            return;
        };
        match tap.phase {
            Some(p) if p != kp.phase() => tap.generation += 1,
            _ => {}
        }
        tap.phase = Some(kp.phase());
        let generation = tap.generation;
        let k = self.keys.entry((node, link, generation)).or_default();
        k.frames += 1;
        let ctr = kp.ctr();
        let prev = k.last_ctr;
        k.last_ctr = Some(prev.map_or(ctr, |l| l.max(ctr)));
        if let Some(last) = prev.filter(|last| ctr <= *last) {
            let d = format!(
                "node {node} link {link} key generation {generation}: counter {ctr} after {last}"
            );
            self.violate(t, ViolationKind::NonceReuse, d);
        }
    }

    /// Distinct sending keys observed.
    pub fn keys_seen(&self) -> usize {
        self.keys.len()
    }

    /// The link of `node` is gone: its framer state is dropped (ids are never reused).
    pub fn forget_link(&mut self, node: usize, link: u32) {
        self.taps.remove(&(node, link));
    }

    /// Current same-id pairs `(id, a, b)` inside one connected component; a pair that
    /// lasts longer than `grace_us` is a violation (reported once per episode).
    pub fn check_ids(&mut self, t: Instant, pairs: &[(u16, usize, usize)], grace_us: u64) {
        self.id_collisions.retain(|k, _| pairs.contains(k));
        for p in pairs {
            let since = *self.id_collisions.entry(*p).or_insert(t.0);
            if since != u64::MAX && t.0.saturating_sub(since) > grace_us {
                let d = format!(
                    "id {} shared by nodes {} and {} for more than {grace_us} µs",
                    p.0, p.1, p.2
                );
                self.violate(t, ViolationKind::IdCollision, d);
                self.id_collisions.insert(*p, u64::MAX);
            }
        }
    }

    /// A node's active BLE links against its max. Every episode over the max is
    /// counted in `over_max_episodes`. §5.7 closes a link over the max only when one
    /// is *redundant* (its neighbor appears in another neighbor's NBR) and its removal
    /// cuts no reachable node off in the 2-hop graph; `closable` says whether such a
    /// link exists now (the caller computes it from the world graph). With
    /// `grace_us = Some(g)`, an over-max state that stays closable for longer than `g`
    /// is a violation (reported once per episode). `None` makes the check
    /// report-only, for worlds whose engines have no over-max policy (Lab mode: two
    /// initiators may read one `accepting` HELLO at once).
    pub fn check_link_cap(
        &mut self,
        t: Instant,
        node: usize,
        links: usize,
        max: usize,
        closable: bool,
        grace_us: Option<u64>,
    ) {
        if links <= max {
            self.over_cap.remove(&node);
            self.over_episode.remove(&node);
            return;
        }
        if self.over_episode.insert(node) {
            self.over_max_episodes += 1;
        }
        if !closable {
            // Nothing §5.7 may close: the episode goes on, without a deadline.
            if self.over_cap.get(&node) != Some(&u64::MAX) {
                self.over_cap.remove(&node);
            }
            return;
        }
        let since = *self.over_cap.entry(node).or_insert(t.0);
        let Some(grace_us) = grace_us else {
            return;
        };
        if since != u64::MAX && t.0.saturating_sub(since) > grace_us {
            let d = format!("node {node}: {links} BLE links > max {max} with a redundant one");
            self.violate(t, ViolationKind::LinkCap, d);
            self.over_cap.insert(node, u64::MAX);
        }
    }

    /// Streams currently carried by one link (fed by a stream tap); over `cap` is a
    /// violation.
    pub fn observe_streams(
        &mut self,
        t: Instant,
        node: usize,
        link: u32,
        ble: bool,
        n: usize,
        cap: usize,
    ) {
        if ble {
            self.max_streams.0 = self.max_streams.0.max(n);
        } else {
            self.max_streams.1 = self.max_streams.1.max(n);
        }
        if n > cap {
            let d = format!("node {node} link {link}: {n} streams > cap {cap}");
            self.violate(t, ViolationKind::StreamCap, d);
        }
    }

    /// Records a rule violation found by the world (role rule, capture rule).
    pub fn report(&mut self, t: Instant, kind: ViolationKind, detail: String) {
        self.violate(t, kind, detail);
    }
}
