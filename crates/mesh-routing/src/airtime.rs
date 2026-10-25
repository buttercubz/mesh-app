//! Voice-aware node BLE airtime budget (DESIGN §5.6).
//!
//! All BLE links of a node share one radio. A BLE link is a **voice link** if it carried
//! voice in the last 2 s, otherwise **idle**. Idle links are granted at most the
//! control-only floor `R_IDLE = 8 kbps` (their `R` is kept, only the grant is limited).
//! Voice links share the rest: `Σ grant(voice) ≤ NODE_BLE_BUDGET − Σ grant(idle)`; when
//! the sum of their `R` would exceed it, **only the voice links** are scaled,
//! proportionally. Non-BLE links are not in the budget (grant = `R`).
//!
//! Proportional scaling never squeezes a voice link below one stream: when it would
//! leave a voice link under `min(R, one BLE12 stream)` (25.1 kbps, the pacer's voice
//! floor), every voice link first keeps that floor and only the excess above it is
//! scaled. A link whose own pacer sits at the floor (a fading link) then still carries
//! one stream while its well-connected siblings give up a little probing headroom.

use mesh_wire::mesh::EncoderProfile;

use crate::cost::CostModel;
use crate::limits::Limits;
use crate::types::{LinkId, LinkKind};

/// One BLE12 stream (`cost(1) / 0.9`): a voice link's grant never scales below
/// `min(R, VOICE_FLOOR_BPS)`.
const VOICE_FLOOR_BPS: u32 = CostModel::new(EncoderProfile::Ble12).min_grant_bps(1);

/// One link's input to the budget.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AirtimeInput {
    pub link: LinkId,
    pub kind: LinkKind,
    /// Pacer rate `R`.
    pub rate_bps: u32,
    /// Carried voice in the last 2 s.
    pub voice: bool,
}

/// One link's grant.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Grant {
    pub link: LinkId,
    pub grant_bps: u32,
    /// The grant was limited by the budget (scaled voice link or idle floor).
    pub limited: bool,
}

/// The node airtime budget.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AirtimeBudget {
    pub node_ble_budget_bps: u32,
    pub idle_grant_bps: u32,
}

impl AirtimeBudget {
    pub const fn new(node_ble_budget_bps: u32, idle_grant_bps: u32) -> Self {
        AirtimeBudget {
            node_ble_budget_bps,
            idle_grant_bps,
        }
    }

    pub const fn from_limits(l: &Limits) -> Self {
        Self::new(l.node_ble_budget_bps, l.idle_grant_bps)
    }

    /// Grants for every link (same order as `links`).
    pub fn grants(&self, links: &[AirtimeInput]) -> Vec<Grant> {
        let idle_sum: u64 = links
            .iter()
            .filter(|l| l.kind == LinkKind::Ble && !l.voice)
            .map(|l| u64::from(l.rate_bps.min(self.idle_grant_bps)))
            .sum();
        let voice_sum: u64 = links
            .iter()
            .filter(|l| l.kind == LinkKind::Ble && l.voice)
            .map(|l| u64::from(l.rate_bps))
            .sum();
        let voice_budget = u64::from(self.node_ble_budget_bps).saturating_sub(idle_sum);
        let scale = voice_sum > voice_budget;
        let floor = |l: &AirtimeInput| u64::from(l.rate_bps.min(VOICE_FLOOR_BPS));
        let floor_sum: u64 = links
            .iter()
            .filter(|l| l.kind == LinkKind::Ble && l.voice)
            .map(floor)
            .sum();
        // Proportional unless that would put some voice link under its floor (and the
        // floors themselves fit the budget).
        let floored = scale
            && floor_sum <= voice_budget
            && links.iter().any(|l| {
                l.kind == LinkKind::Ble
                    && l.voice
                    && u64::from(l.rate_bps) * voice_budget / voice_sum.max(1) < floor(l)
            });
        let excess_sum = voice_sum - floor_sum;
        let excess_budget = voice_budget.saturating_sub(floor_sum);
        links
            .iter()
            .map(|l| {
                let (g, limited) = match (l.kind, l.voice) {
                    (LinkKind::Ble, false) => (
                        l.rate_bps.min(self.idle_grant_bps),
                        l.rate_bps > self.idle_grant_bps,
                    ),
                    (LinkKind::Ble, true) if floored => (
                        (floor(l)
                            + (u64::from(l.rate_bps) - floor(l)) * excess_budget
                                / excess_sum.max(1)) as u32,
                        true,
                    ),
                    (LinkKind::Ble, true) if scale => (
                        (u64::from(l.rate_bps) * voice_budget / voice_sum.max(1)) as u32,
                        true,
                    ),
                    _ => (l.rate_bps, false),
                };
                Grant {
                    link: l.link,
                    grant_bps: g,
                    limited,
                }
            })
            .collect()
    }
}

impl Default for AirtimeBudget {
    fn default() -> Self {
        Self::from_limits(&Limits::V1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ble(id: u32, r: u32, voice: bool) -> AirtimeInput {
        AirtimeInput {
            link: LinkId(id),
            kind: LinkKind::Ble,
            rate_bps: r,
            voice,
        }
    }

    #[test]
    fn idle_links_get_eight_kbps_and_only_voice_scales() {
        let b = AirtimeBudget::default();
        let g = b.grants(&[
            ble(1, 100_000, true),
            ble(2, 100_000, true),
            ble(3, 60_000, false),
        ]);
        assert_eq!(g[2].grant_bps, 8_000);
        // 180 − 8 = 172 shared by the two voice links in proportion to R.
        assert_eq!(g[0].grant_bps, 86_000);
        assert_eq!(g[1].grant_bps, 86_000);
        assert!(g[0].limited);
    }

    #[test]
    fn no_scaling_at_the_default_target() {
        let b = AirtimeBudget::default();
        let g = b.grants(&[ble(1, 60_000, true), ble(2, 60_000, true)]);
        assert_eq!(g[0].grant_bps, 60_000);
        assert!(!g[0].limited);
        // Three voice links at 60 kbps exactly fill 180 kbps.
        let g = b.grants(&[
            ble(1, 60_000, true),
            ble(2, 60_000, true),
            ble(3, 60_000, true),
        ]);
        assert!(g.iter().all(|x| x.grant_bps == 60_000 && !x.limited));
    }

    #[test]
    fn proportional_and_non_ble_untouched() {
        let b = AirtimeBudget::default();
        let aware = AirtimeInput {
            link: LinkId(9),
            kind: LinkKind::Aware,
            rate_bps: 2_000_000,
            voice: true,
        };
        let g = b.grants(&[
            ble(1, 120_000, true),
            ble(2, 60_000, true),
            ble(3, 60_000, true),
            aware,
        ]);
        assert_eq!(g[0].grant_bps, 90_000);
        assert_eq!(g[1].grant_bps, 45_000);
        assert_eq!(g[3].grant_bps, 2_000_000);
        let sum: u32 = g[..3].iter().map(|x| x.grant_bps).sum();
        assert!(sum <= 180_000);
        // An idle link whose R is below 8 kbps keeps its R.
        let g = b.grants(&[ble(1, 5_000, false)]);
        assert_eq!(g[0].grant_bps, 5_000);
    }

    #[test]
    fn scaling_keeps_one_stream_on_a_fading_link() {
        let b = AirtimeBudget::default();
        // A fading link at the pacer floor next to two links probing high, plus an
        // idle one: proportional scaling would give it 172 × 25.1 / 325.1 ≈ 13.3 kbps.
        let g = b.grants(&[
            ble(1, 150_000, true),
            ble(2, 150_000, true),
            ble(3, VOICE_FLOOR_BPS, true),
            ble(4, 60_000, false),
        ]);
        assert_eq!(g[2].grant_bps, VOICE_FLOOR_BPS);
        assert!(g[0].grant_bps >= VOICE_FLOOR_BPS && g[0].grant_bps == g[1].grant_bps);
        let sum: u32 = g.iter().map(|x| x.grant_bps).sum();
        assert!(sum <= 180_000, "{sum}");
        assert_eq!(VOICE_FLOOR_BPS, 25_112);
    }
}
