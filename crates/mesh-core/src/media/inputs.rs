//! Encoder-profile inputs (DESIGN §7.4) and the local talker cap, read from this node's
//! own links at burst start: link kinds, the pacer's rate (`cap_pacing`), PATH_HINT,
//! thermal state and the worst neighbour's smoothed loss (from its LSTAT reports).

use mesh_api::ThermalState;
use mesh_routing::CostModel;
use mesh_voice::ProfileInputs;
use mesh_wire::mesh::EncoderProfile;
use mesh_wire::record::Lstat;

use super::state::Media;
use crate::engine::Ctx;
use crate::links::{LinkKind, LinkState};

/// EWMA weight of a new LSTAT loss sample.
const LOSS_ALPHA: f32 = 0.2;

fn routing_kind(k: LinkKind) -> mesh_routing::LinkKind {
    match k {
        LinkKind::BleL2cap | LinkKind::BleGatt => mesh_routing::LinkKind::Ble,
        LinkKind::WifiAware => mesh_routing::LinkKind::Aware,
        LinkKind::Tunnel => mesh_routing::LinkKind::Tunnel,
    }
}

/// Loss (percent) the peer reports for what we sent in its last LSTAT window.
pub(super) fn lstat_loss_pct(l: &Lstat) -> f32 {
    let total = u32::from(l.rx_frames) + u32::from(l.gaps);
    if total == 0 {
        0.0
    } else {
        100.0 * f32::from(l.gaps) / total as f32
    }
}

impl Media {
    /// Folds the latest LSTAT of every Up link into the smoothed loss (timer path).
    pub(super) fn update_loss(&mut self, ctx: &Ctx) {
        let mut seen = Vec::new();
        for l in ctx.links.iter() {
            if l.state != LinkState::Up || l.is_lab() {
                continue;
            }
            seen.push(l.id);
            if let Some(ls) = l.peer_lstat.as_ref() {
                let x = lstat_loss_pct(ls);
                let e = self.loss_pct.entry(l.id).or_insert(x);
                *e += LOSS_ALPHA * (x - *e);
            }
        }
        self.loss_pct.retain(|k, _| seen.contains(k));
    }

    /// `cap_pacing` of one link from its pacer rate (BLE16 cost model); `None` before
    /// the pacer set a rate.
    fn cap_pacing(rate_bps: Option<u32>, s_max: u8) -> Option<u8> {
        rate_bps.map(|r| CostModel::new(EncoderProfile::Ble16).cap_pacing(r, s_max))
    }

    /// Up, non-Lab links.
    fn up_links(ctx: &Ctx) -> impl Iterator<Item = &crate::links::Link> {
        ctx.links
            .iter()
            .filter(|l| l.state == LinkState::Up && !l.is_lab())
    }

    /// Peers I hold an Up Wi-Fi Aware link to.
    fn aware_peers(ctx: &Ctx) -> Vec<u16> {
        Self::up_links(ctx)
            .filter(|l| l.info.kind == LinkKind::WifiAware)
            .filter_map(|l| l.peer_node.map(|p| p.0))
            .collect()
    }

    /// A BLE link that actually carries my voice: its peer has no Aware link with me
    /// (the link bundle sends voice to such a neighbor over Aware only, §9.7).
    fn ble_egress(l: &crate::links::Link, aware: &[u16]) -> bool {
        l.info.kind.is_ble() && !l.peer_node.is_some_and(|p| aware.contains(&p.0))
    }

    /// A BLE *edge* link: a BLE link while I hold an Aware link to another neighbor
    /// (hybrid groups, the same rule as `Router::is_edge_link`).
    fn ble_edge(l: &crate::links::Link, aware: &[u16]) -> bool {
        l.info.kind.is_ble() && aware.iter().any(|a| l.peer_node.is_none_or(|p| p.0 != *a))
    }

    /// The profile inputs at `ctx.now` (§7.4: BLE profiles when *any* of my links is
    /// BLE, Wi-Fi profiles only when all of them are Aware/Tunnel). A BLE fallback link
    /// to a neighbor I also reach over Aware counts as well, even though my voice goes
    /// to that neighbor over Aware only: relays never transcode, so a Wi-Fi-profile
    /// stream (20 ms frames, 34.4 kbps) would cross the BLE edges of a hybrid group
    /// whose admission is sized for BLE16 streams, and if my Aware link drops mid-burst
    /// the fallback link would carry 20 ms frames that the BLE12 downshift cannot
    /// re-tune. `cap_pacing_one` reads only the BLE links that carry my voice (an idle
    /// fallback link's pacer says nothing about my egress).
    pub(super) fn profile_inputs(&self, ctx: &Ctx) -> ProfileInputs {
        let lim = ctx.cfg.limits.routing;
        let aware = Self::aware_peers(ctx);
        let mut any_ble = false;
        let mut any_wifi = false;
        let mut cap_one = false;
        for l in Self::up_links(ctx) {
            if !l.info.kind.is_ble() {
                any_wifi = true;
                continue;
            }
            any_ble = true;
            if Self::ble_egress(l, &aware)
                && Self::cap_pacing(l.pace.rate_bps, lim.s_max_ble).is_some_and(|c| c <= 1)
            {
                cap_one = true;
            }
        }
        let worst = self.loss_pct.values().copied().fold(0.0f32, f32::max);
        ProfileInputs {
            // No link at all counts as BLE: the first link of a ride is BLE.
            any_ble_link: any_ble || !any_wifi,
            path_hint_ble12: self.path_hint_until.is_some_and(|t| ctx.now < t),
            cap_pacing_one: cap_one,
            thermal_serious: matches!(
                ctx.device.thermal(),
                ThermalState::Serious | ThermalState::Critical
            ),
            worst_loss_pct: worst,
            // BLE-DRED is a Phase 5 experiment gated on group-wide `dred_ok` (§4.9),
            // which the session module does not report yet.
            dred_allowed: false,
        }
    }

    /// `cap_local`: the cap of my most constrained voice link (§6.4); S_MAX_BLE when I
    /// have no link yet. Two kinds of BLE link do not count: BLE edge links of a
    /// backbone node (a full edge sheds my burst there only, it never blocks me on the
    /// Aware backbone) and BLE fallback links to a neighbor I also reach over Aware (the
    /// link bundle sends my voice there over Aware only, so the BLE cap never applies;
    /// the same rule as `Router::cap_local`).
    pub(super) fn cap_local(&self, ctx: &Ctx) -> u8 {
        let lim = ctx.cfg.limits.routing;
        let aware = Self::aware_peers(ctx);
        let mut cap: Option<u8> = None;
        for l in Self::up_links(ctx) {
            if Self::ble_edge(l, &aware) || (l.info.kind.is_ble() && !Self::ble_egress(l, &aware)) {
                continue;
            }
            let kind = routing_kind(l.info.kind);
            let pacing = Self::cap_pacing(l.pace.rate_bps, lim.s_max(kind))
                .unwrap_or(lim.s_max(kind))
                .max(1);
            let c = lim.link_cap(kind, pacing);
            cap = Some(cap.map_or(c, |x| x.min(c)));
        }
        cap.unwrap_or(lim.group_cap(true)).max(1)
    }

    /// The near-talker `group_cap` for VOICE received on `link`, per segment (§6.3):
    /// 4 when the voice comes from the Wi-Fi backbone (an Aware link, or a neighbor the
    /// mesh glue lists in `Ctx::aware_backbone`), 2 inside a BLE segment. A group-wide
    /// "any BLE link" cap would throttle an Aware island to the BLE cap.
    pub(super) fn near_talker_cap(&self, ctx: &Ctx, link: crate::LinkId) -> u8 {
        let backbone = ctx.links.get(link).is_some_and(|l| {
            l.info.kind == LinkKind::WifiAware
                || l.peer_node
                    .is_some_and(|p| ctx.aware_backbone.contains(&p.0))
        });
        ctx.cfg.limits.routing.group_cap(!backbone)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lstat_loss() {
        let l = Lstat {
            rx_frames: 97,
            rx_bytes: 0,
            gaps: 3,
            pace_kbps: 60,
            queue_ms: 0,
        };
        assert!((lstat_loss_pct(&l) - 3.0).abs() < 1e-4);
        assert_eq!(
            lstat_loss_pct(&Lstat {
                rx_frames: 0,
                gaps: 0,
                ..l
            }),
            0.0
        );
        assert_eq!(Media::cap_pacing(Some(60_000), 2), Some(2));
        assert_eq!(Media::cap_pacing(Some(40_000), 2), Some(1));
        assert_eq!(Media::cap_pacing(None, 2), None);
    }
}
