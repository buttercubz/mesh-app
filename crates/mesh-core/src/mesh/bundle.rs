//! Link bundles: which of a neighbor's links carries a packet (DESIGN §9.7, §9.8;
//! Phase 4 hooks).
//!
//! - Aware is used when its `q ≥ 60` and `srtt < 50 ms`, otherwise BLE.
//! - Downgrade triggers: Aware has no RX for 600 ms, or `q < 40`. The BLE link is always
//!   kept as the fallback and control path.
//! - For 1 s after a switch in either direction, voice goes on both links; end-to-end
//!   dedup removes the copy, so the receiver sees no gap.
//! - On a downgrade the admitted bursts are re-evaluated with the BLE cap; the caller
//!   rejects the excess with `FLOOR_REJECT{congestion}` (lowest rank first).
//! - Tunnel (v2 cost hook): used only for a neighbor that has had no radio link for
//!   more than 5 s, i.e. to bridge islands.

use std::collections::BTreeMap;

use mesh_routing::{Instant, LinkId, LinkKind, LinkState, Neighbor, NodeId};

/// Aware upgrade thresholds.
pub const AWARE_Q_UP: u8 = 60;
pub const AWARE_SRTT_MAX_US: u64 = 50_000;
/// Aware downgrade thresholds.
pub const AWARE_Q_DOWN: u8 = 40;
pub const AWARE_RX_SILENCE_US: u64 = 600_000;
/// Both links carry voice this long after a switch.
pub const OVERLAP_US: u64 = 1_000_000;
/// A Tunnel link is used only after this long without any radio link.
pub const TUNNEL_AFTER_US: u64 = 5_000_000;

/// The links a packet to one neighbor goes on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Selection {
    /// The chosen link (every class).
    pub primary: LinkId,
    /// The previous link during the 1 s overlap (voice only).
    pub overlap: Option<LinkId>,
}

/// A switch that just happened.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Switch {
    pub node: NodeId,
    pub from: LinkId,
    pub to: LinkId,
    /// Aware → BLE: admission must be re-evaluated with the BLE cap.
    pub downgrade: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Choice {
    kind: LinkKind,
    link: LinkId,
    prev: Option<LinkId>,
    switched_at: Instant,
}

/// Per-neighbor link choice.
#[derive(Debug, Clone, Default)]
pub struct Bundles {
    choice: BTreeMap<NodeId, Choice>,
    /// Since when a neighbor has had no radio (BLE/Aware) link.
    radio_lost: BTreeMap<NodeId, Instant>,
}

fn aware_up_ok(now: Instant, l: &LinkState) -> bool {
    l.q(now) >= AWARE_Q_UP
        && l.quality.rtt.srtt_us().unwrap_or(0) < AWARE_SRTT_MAX_US
        && now.since(l.last_rx) < AWARE_RX_SILENCE_US
}

fn aware_down(now: Instant, l: &LinkState) -> bool {
    now.since(l.last_rx) >= AWARE_RX_SILENCE_US || l.q(now) < AWARE_Q_DOWN
}

impl Bundles {
    pub fn new() -> Self {
        Self::default()
    }

    /// Forgets a neighbor.
    pub fn forget(&mut self, node: NodeId) {
        self.choice.remove(&node);
        self.radio_lost.remove(&node);
    }

    /// The kind the policy wants for `n` now, given the current kind.
    fn wanted(
        &mut self,
        now: Instant,
        n: &Neighbor,
        current: Option<LinkKind>,
    ) -> Option<LinkKind> {
        let ble = n.links.get(LinkKind::Ble);
        let aware = n.links.get(LinkKind::Aware);
        if ble.is_none() && aware.is_none() {
            let since = *self.radio_lost.entry(n.node_id).or_insert(now);
            let tunnel_ok = now.since(since) >= TUNNEL_AFTER_US;
            return (tunnel_ok && n.links.get(LinkKind::Tunnel).is_some())
                .then_some(LinkKind::Tunnel);
        }
        self.radio_lost.remove(&n.node_id);
        let Some(a) = aware else {
            return Some(LinkKind::Ble);
        };
        if ble.is_none() {
            return Some(LinkKind::Aware);
        }
        let keep_aware = current == Some(LinkKind::Aware) && !aware_down(now, a);
        if keep_aware || aware_up_ok(now, a) {
            Some(LinkKind::Aware)
        } else {
            Some(LinkKind::Ble)
        }
    }

    /// Chooses the link(s) for a packet to neighbor `n`. Returns the selection and, the
    /// first time a switch is seen, the switch.
    pub fn select(&mut self, now: Instant, n: &Neighbor) -> (Option<Selection>, Option<Switch>) {
        let current = self.choice.get(&n.node_id).copied();
        let Some(kind) = self.wanted(now, n, current.map(|c| c.kind)) else {
            self.choice.remove(&n.node_id);
            return (None, None);
        };
        let Some(link) = n.links.get(kind).map(|l| l.id) else {
            return (None, None);
        };
        let mut switch = None;
        let c = match current {
            Some(c) if c.link == link => c,
            Some(c) => {
                let still = n.links.iter().any(|l| l.id == c.link);
                if still {
                    switch = Some(Switch {
                        node: n.node_id,
                        from: c.link,
                        to: link,
                        downgrade: c.kind == LinkKind::Aware && kind == LinkKind::Ble,
                    });
                }
                Choice {
                    kind,
                    link,
                    prev: still.then_some(c.link),
                    switched_at: now,
                }
            }
            None => Choice {
                kind,
                link,
                prev: None,
                switched_at: now,
            },
        };
        self.choice.insert(n.node_id, c);
        let overlap = c.prev.filter(|p| {
            now.since(c.switched_at) < OVERLAP_US && n.links.iter().any(|l| l.id == *p)
        });
        (
            Some(Selection {
                primary: link,
                overlap,
            }),
            switch,
        )
    }
}

#[cfg(test)]
#[path = "bundle_tests.rs"]
mod tests;
