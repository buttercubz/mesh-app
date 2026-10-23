//! Per-platform limits handed in by the core's `EngineConfig::limits` (DESIGN §3.2,
//! Appendix A). Defaults are the compiled-in v1 values; Lab benches may change the
//! [TUNE] ones (Appendix B: `links_max × cost(2) ≤ 0.9 × NODE_BLE_BUDGET` must keep
//! holding when the budget or the link max change).

use crate::consts;
use crate::types::LinkKind;

/// Link targets, caps, budget and timers used by mesh-routing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    /// BLE links target / max per node (§5.7).
    pub ble_links_target: u8,
    pub ble_links_max: u8,
    /// Wi-Fi Aware links max (further bounded by available data paths).
    pub aware_links_max: u8,
    /// Distinct neighbors max (all kinds).
    pub neighbors_max: u8,
    /// Node BLE airtime budget and the idle-link grant (§5.6).
    pub node_ble_budget_bps: u32,
    pub idle_grant_bps: u32,
    /// Per-kind stream caps (§6.1).
    pub s_max_ble: u8,
    pub s_max_aware: u8,
    pub s_max_tunnel: u8,
    /// Group cap override (policy), applied on top of S_MAX and cap_pacing.
    pub group_cap_override: Option<u8>,
    /// Voice TTL at the origin and the relay age cap (policy ranges 3-6 and 100-250 ms).
    pub voice_ttl: u8,
    pub voice_max_age_us: u64,
    /// Floor smear and stickiness [TUNE].
    pub t_tie_us: u64,
    pub t_sticky_us: u64,
    /// Leader priority (group policy, default on).
    pub leader_priority: bool,
}

impl Limits {
    /// Compiled-in v1 defaults.
    pub const V1: Limits = Limits {
        ble_links_target: consts::BLE_LINKS_TARGET,
        ble_links_max: consts::BLE_LINKS_MAX,
        aware_links_max: consts::AWARE_LINKS_MAX,
        neighbors_max: consts::NEIGHBORS_MAX,
        node_ble_budget_bps: consts::NODE_BLE_BUDGET_BPS,
        idle_grant_bps: consts::R_IDLE_BPS,
        s_max_ble: consts::S_MAX_BLE,
        s_max_aware: consts::S_MAX_AWARE,
        s_max_tunnel: consts::S_MAX_TUNNEL,
        group_cap_override: None,
        voice_ttl: consts::VOICE_TTL,
        voice_max_age_us: consts::VOICE_MAX_AGE_US,
        t_tie_us: consts::T_TIE_US,
        t_sticky_us: consts::T_STICKY_US,
        leader_priority: true,
    };

    /// `S_MAX[kind]`.
    pub const fn s_max(&self, kind: LinkKind) -> u8 {
        match kind {
            LinkKind::Ble => self.s_max_ble,
            LinkKind::Aware => self.s_max_aware,
            LinkKind::Tunnel => self.s_max_tunnel,
        }
    }

    /// `cap(L) = min(S_MAX[kind], cap_pacing, group_cap_override)` (§6.3).
    pub fn link_cap(&self, kind: LinkKind, cap_pacing: u8) -> u8 {
        let c = self.s_max(kind).min(cap_pacing);
        match self.group_cap_override {
            Some(o) => c.min(o),
            None => c,
        }
    }

    /// The near-talker `group_cap`: 2 if any BLE link exists in the group, else 4
    /// (§6.3); the override applies too.
    pub fn group_cap(&self, any_ble: bool) -> u8 {
        let c = if any_ble {
            self.s_max_ble
        } else {
            self.s_max_aware
        };
        match self.group_cap_override {
            Some(o) => c.min(o),
            None => c,
        }
    }

    /// The Appendix B invariant `links_max × cost(2) ≤ 0.9 × NODE_BLE_BUDGET` for BLE16.
    pub fn budget_consistent(&self) -> bool {
        let cost2 = u64::from(mesh_wire::budget::cost_bps(
            2,
            mesh_wire::mesh::EncoderProfile::Ble16,
        ));
        u64::from(self.ble_links_max) * cost2 * 10 <= u64::from(self.node_ble_budget_bps) * 9
    }
}

impl Default for Limits {
    fn default() -> Self {
        Self::V1
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults() {
        let l = Limits::default();
        assert_eq!((l.ble_links_target, l.ble_links_max), (2, 3));
        assert_eq!(l.link_cap(LinkKind::Ble, 4), 2);
        assert_eq!(l.link_cap(LinkKind::Aware, 3), 3);
        assert_eq!(l.group_cap(true), 2);
        assert_eq!(l.group_cap(false), 4);
        assert!(l.budget_consistent());
        let o = Limits {
            group_cap_override: Some(1),
            ..Limits::V1
        };
        assert_eq!(o.link_cap(LinkKind::Aware, 4), 1);
        let bad = Limits {
            ble_links_max: 4,
            ..Limits::V1
        };
        assert!(!bad.budget_consistent());
    }
}
