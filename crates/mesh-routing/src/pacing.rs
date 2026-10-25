//! Node-level pacing (DESIGN §5.6): one [`Pacer`] and one [`CapEstimator`] per link, the
//! voice-aware [`AirtimeBudget`] across the node's BLE links, and the cost model.
//!
//! Every 500 ms per link: the pacer runs its delay-based control law; then the node
//! recomputes the airtime grants (voice links = carried voice in the last 2 s), hands
//! each pacer its grant and feeds the grant to the link's cap estimator. Published cap
//! changes go to the router (`Router::set_link_cap`) and trigger an NBR; a sustained
//! `grant < cost(1)` asks the core to flood `PATH_HINT{BLE12}`.

use std::collections::BTreeMap;

use mesh_wire::mesh::EncoderProfile;

use crate::airtime::{AirtimeBudget, AirtimeInput};
use crate::cap::{CapEstimator, CapModels};
use crate::cost::CostModel;
use crate::limits::Limits;
use crate::neighbor::NeighborTable;
use crate::pacer::{Pacer, PacerPath};
use crate::types::{Instant, LinkId, LinkKind};

/// One link's pacing state.
#[derive(Debug, Clone)]
pub struct LinkPacing {
    pub kind: LinkKind,
    pub pacer: Pacer,
    pub cap: CapEstimator,
    pub published_cap: u8,
}

/// Result of one node tick.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PacingUpdate {
    /// Links whose published cap changed: `(link, cap_pacing)`.
    pub cap_changes: Vec<(LinkId, u8)>,
    /// Flood `PATH_HINT{BLE12}` (some link's grant stayed below `cost(1)` for 3 s).
    pub path_hint: bool,
}

/// Pacing of every link of a node.
#[derive(Debug, Clone)]
pub struct NodePacing {
    pub limits: Limits,
    pub budget: AirtimeBudget,
    /// Voice profile used on BLE links (BLE16; BLE12 after PATH_HINT).
    pub ble_profile: EncoderProfile,
    links: BTreeMap<LinkId, LinkPacing>,
}

impl NodePacing {
    pub fn new(limits: Limits) -> Self {
        NodePacing {
            budget: AirtimeBudget::from_limits(&limits),
            limits,
            ble_profile: EncoderProfile::Ble16,
            links: BTreeMap::new(),
        }
    }

    /// The cost model of a link kind.
    pub fn model(&self, kind: LinkKind) -> CostModel {
        match kind {
            LinkKind::Ble => CostModel::new(self.ble_profile),
            LinkKind::Aware | LinkKind::Tunnel => CostModel::new(EncoderProfile::Wifi20),
        }
    }

    /// The cap models of a link kind: the current profile, one stream of the lowest
    /// profile as the fallback (BLE12), and the default profile as the PATH_HINT
    /// reference (BLE16), see [`CapModels`].
    pub fn cap_models(&self, kind: LinkKind) -> CapModels {
        match kind {
            LinkKind::Ble => CapModels {
                cap: self.model(kind),
                fallback: CostModel::new(EncoderProfile::Ble12),
                hint: CostModel::new(EncoderProfile::Ble16),
            },
            LinkKind::Aware | LinkKind::Tunnel => CapModels::single(self.model(kind)),
        }
    }

    pub fn add_link(&mut self, now: Instant, link: LinkId, path: PacerPath, max_frame: u16) {
        let kind = path.kind();
        self.links.insert(
            link,
            LinkPacing {
                kind,
                pacer: Pacer::new(now, path, u32::from(max_frame)),
                cap: CapEstimator::new(),
                published_cap: self.limits.s_max(kind),
            },
        );
    }

    pub fn remove_link(&mut self, link: LinkId) {
        self.links.remove(&link);
    }

    pub fn link(&self, link: LinkId) -> Option<&LinkPacing> {
        self.links.get(&link)
    }

    pub fn link_mut(&mut self, link: LinkId) -> Option<&mut LinkPacing> {
        self.links.get_mut(&link)
    }

    /// The platform returned `WouldBlock` on `link`.
    pub fn on_would_block(&mut self, link: LinkId) {
        if let Some(l) = self.links.get_mut(&link) {
            l.pacer.on_would_block();
        }
    }

    /// Next control instant over all links.
    pub fn next_deadline(&self) -> Option<Instant> {
        self.links.values().map(|l| l.pacer.next_control()).min()
    }

    /// Runs due control steps and refreshes grants and caps.
    pub fn tick(&mut self, now: Instant, table: &NeighborTable) -> PacingUpdate {
        let mut stepped = false;
        for (id, l) in self.links.iter_mut() {
            let qd = table
                .link(*id)
                .map_or(0, |s| s.quality.rtt.queue_delay_us(now));
            stepped |= l.pacer.tick(now, qd).is_some();
        }
        if !stepped {
            return PacingUpdate::default();
        }
        self.refresh(now, table)
    }

    /// Recomputes grants and caps now (also call when an idle link starts carrying voice,
    /// so its first frames are not paced at the 8 kbps idle grant).
    ///
    /// The cap of an **idle** link is computed from its *prospective* voice grant (the
    /// grant it would get as a voice link): the 8 kbps idle grant limits what it sends
    /// while idle, not how many streams it may admit, otherwise an idle link could never
    /// become a voice link (and the §5.6 steady state "2 voice + 1 idle keep cap 2 on
    /// every link" could not hold).
    pub fn refresh(&mut self, now: Instant, table: &NeighborTable) -> PacingUpdate {
        let mut up = PacingUpdate::default();
        let inputs: Vec<AirtimeInput> = self
            .links
            .iter()
            .map(|(id, l)| AirtimeInput {
                link: *id,
                kind: l.kind,
                rate_bps: l.pacer.rate_bps(),
                voice: table.link(*id).is_some_and(|s| s.is_voice_link(now)),
            })
            .collect();
        let grants = self.budget.grants(&inputs);
        for (i, g) in grants.iter().enumerate() {
            let cap_grant = if inputs[i].voice || inputs[i].kind != LinkKind::Ble {
                g.grant_bps
            } else {
                let mut prospective = inputs.clone();
                prospective[i].voice = true;
                self.budget.grants(&prospective)[i].grant_bps
            };
            let kind = inputs[i].kind;
            let models = self.cap_models(kind);
            let s_max = self.limits.s_max(kind);
            if let Some(l) = self.links.get_mut(&g.link) {
                l.pacer.set_grant(now, g.grant_bps);
                let u = l.cap.update_with(now, cap_grant, s_max, &models);
                if u.published != l.published_cap {
                    l.published_cap = u.published;
                    up.cap_changes.push((g.link, u.published));
                }
                if kind == LinkKind::Ble && u.path_hint {
                    up.path_hint = true;
                }
            }
        }
        up
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::LinkRole;

    #[test]
    fn idle_link_keeps_its_prospective_cap() {
        let mut p = NodePacing::new(Limits::V1);
        let mut t = NeighborTable::new();
        let now = Instant::ZERO;
        t.link_up(now, 2, LinkId(1), LinkKind::Ble, LinkRole::Initiator, 1024)
            .expect("up");
        p.add_link(now, LinkId(1), PacerPath::L2cap, 1024);
        let mut changes = Vec::new();
        let mut hint = false;
        for i in 1..=10u64 {
            let u = p.tick(now.plus_ms(500 * i), &t);
            changes.extend(u.cap_changes);
            hint |= u.path_hint;
        }
        // Idle: sends at 8 kbps, but may admit 2 streams; no PATH_HINT.
        assert!(changes.is_empty());
        assert!(!hint);
        assert_eq!(p.link(LinkId(1)).map(|l| l.pacer.grant_bps()), Some(8_000));
        assert_eq!(p.link(LinkId(1)).map(|l| l.published_cap), Some(2));
        // Voice starts: the refresh grants the full rate at once.
        t.mark_voice_tx(now.plus_ms(5_000), LinkId(1));
        p.refresh(now.plus_ms(5_000), &t);
        assert_eq!(p.link(LinkId(1)).map(|l| l.pacer.grant_bps()), Some(60_000));
    }

    #[test]
    fn starved_voice_link_raises_path_hint() {
        let limits = Limits {
            node_ble_budget_bps: 20_000,
            ..Limits::V1
        };
        let mut p = NodePacing::new(limits);
        let mut t = NeighborTable::new();
        let now = Instant::ZERO;
        t.link_up(now, 2, LinkId(1), LinkKind::Ble, LinkRole::Initiator, 1024)
            .expect("up");
        p.add_link(now, LinkId(1), PacerPath::L2cap, 1024);
        let mut hint_at = None;
        let mut caps = Vec::new();
        for i in 1..=10u64 {
            let at = now.plus_ms(500 * i);
            t.mark_voice_tx(at, LinkId(1));
            let u = p.tick(at, &t);
            caps.extend(u.cap_changes);
            if u.path_hint && hint_at.is_none() {
                hint_at = Some(at);
            }
        }
        // 20 kbps < cost(1) = 26.6 kbps: cap 0 published, PATH_HINT 3 s after the start.
        assert_eq!(caps, vec![(LinkId(1), 0)]);
        assert_eq!(hint_at, Some(Instant::from_ms(3_500)));
    }
}
