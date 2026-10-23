//! Per-link cap estimator with publication hysteresis and the PATH_HINT trigger
//! (DESIGN §5.6).
//!
//! `cap_pacing(L) = max{k ∈ 1..S_MAX[kind] : cost(k, profile) ≤ 0.9 × grant(L)}`. The
//! published cap drops only after the raw value has been lower for 2 s and rises only
//! after it has been higher for 5 s, so a single multiplicative decrease (60 → 51 kbps,
//! recovered by the additive increase within ~1.5 s) never flaps it. While the raw value
//! stays lower (higher), the cap published at the end of the hold is the highest
//! (lowest) raw value seen during the hold. If `grant` stays below `cost(1)` for 3 s, the
//! relay floods `PATH_HINT{BLE12}` (and re-floods every 20 s while the condition holds,
//! so the origins' 30 s BLE12 hold never lapses).

use crate::consts::{CAP_DOWN_US, CAP_UP_US, PATH_HINT_REFLOOD_US, PATH_HINT_TRIGGER_US};
use crate::cost::CostModel;
use crate::types::Instant;

/// Result of one estimator update.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CapUpdate {
    /// Raw `cap_pacing` from the current grant.
    pub raw: u8,
    /// Published cap (hysteresis applied).
    pub published: u8,
    /// The published cap changed in this update (triggers an NBR, §5.1).
    pub changed: bool,
    /// Flood `PATH_HINT{BLE12}` now.
    pub path_hint: bool,
}

/// Cap estimator of one link.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct CapEstimator {
    published: Option<u8>,
    /// Start of a run of lower raw values and the highest raw value in it.
    lower: Option<(Instant, u8)>,
    /// Start of a run of higher raw values and the lowest raw value in it.
    higher: Option<(Instant, u8)>,
    below_cost1_since: Option<Instant>,
    last_hint: Option<Instant>,
}

impl CapEstimator {
    pub fn new() -> Self {
        Self::default()
    }

    /// The published cap (S_MAX until the first update).
    pub fn published(&self) -> Option<u8> {
        self.published
    }

    /// Feeds the current grant (call every pacer period and whenever the grant changes).
    pub fn update(
        &mut self,
        now: Instant,
        grant_bps: u32,
        s_max: u8,
        model: &CostModel,
    ) -> CapUpdate {
        let raw = model.cap_pacing(grant_bps, s_max);
        let mut changed = false;
        let published = match self.published {
            None => {
                self.published = Some(raw);
                changed = true;
                raw
            }
            Some(p) if raw < p => {
                self.higher = None;
                let (since, hi) = match self.lower {
                    Some((since, hi)) => (since, hi.max(raw)),
                    None => (now, raw),
                };
                self.lower = Some((since, hi));
                if now.since(since) >= CAP_DOWN_US {
                    self.published = Some(hi);
                    self.lower = None;
                    changed = hi != p;
                    hi
                } else {
                    p
                }
            }
            Some(p) if raw > p => {
                self.lower = None;
                let (since, lo) = match self.higher {
                    Some((since, lo)) => (since, lo.min(raw)),
                    None => (now, raw),
                };
                self.higher = Some((since, lo));
                if now.since(since) >= CAP_UP_US {
                    self.published = Some(lo);
                    self.higher = None;
                    changed = lo != p;
                    lo
                } else {
                    p
                }
            }
            Some(p) => {
                self.lower = None;
                self.higher = None;
                p
            }
        };
        let path_hint = self.path_hint_due(now, grant_bps, model);
        CapUpdate {
            raw,
            published,
            changed,
            path_hint,
        }
    }

    fn path_hint_due(&mut self, now: Instant, grant_bps: u32, model: &CostModel) -> bool {
        if grant_bps >= model.cost_bps(1) {
            self.below_cost1_since = None;
            self.last_hint = None;
            return false;
        }
        let since = *self.below_cost1_since.get_or_insert(now);
        if now.since(since) < PATH_HINT_TRIGGER_US {
            return false;
        }
        match self.last_hint {
            Some(t) if now.since(t) < PATH_HINT_REFLOOD_US => false,
            _ => {
                self.last_hint = Some(now);
                true
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const G60: u32 = 60_000;
    const G51: u32 = 51_000;

    #[test]
    fn single_decrease_never_flaps() {
        let m = CostModel::default();
        let mut c = CapEstimator::new();
        let t = Instant::from_secs(1);
        assert_eq!(c.update(t, G60, 2, &m).published, 2);
        // 60 → 51 kbps for 1.5 s (raw 1), then back.
        for i in 1..=3 {
            let u = c.update(t.plus_ms(500 * i), G51, 2, &m);
            assert_eq!((u.raw, u.published), (1, 2));
        }
        let u = c.update(t.plus_ms(2_000), G60, 2, &m);
        assert_eq!((u.raw, u.published, u.changed), (2, 2, false));
    }

    #[test]
    fn down_after_two_seconds_up_after_five() {
        let m = CostModel::default();
        let mut c = CapEstimator::new();
        let t = Instant::from_secs(1);
        c.update(t, G60, 2, &m);
        let mut now = t;
        let mut dropped_at = None;
        for _ in 0..10 {
            now = now.plus_ms(500);
            let u = c.update(now, G51, 2, &m);
            if u.changed {
                dropped_at = Some(now);
                assert_eq!(u.published, 1);
                break;
            }
        }
        assert_eq!(dropped_at, Some(t.plus_ms(2_500)));
        let mut raised_at = None;
        for _ in 0..20 {
            now = now.plus_ms(500);
            let u = c.update(now, G60, 2, &m);
            if u.changed {
                raised_at = Some(now);
                assert_eq!(u.published, 2);
                break;
            }
        }
        assert_eq!(
            raised_at.map(|r| r.since(dropped_at.unwrap_or(t))),
            Some(5_500_000)
        );
    }

    #[test]
    fn path_hint_after_three_seconds_below_cost1() {
        let m = CostModel::default();
        let mut c = CapEstimator::new();
        let t = Instant::from_secs(1);
        let mut hints = Vec::new();
        for i in 0..=50u64 {
            let now = t.plus_ms(500 * i);
            if c.update(now, 20_000, 2, &m).path_hint {
                hints.push(now.since(t) / 1_000);
            }
        }
        assert_eq!(hints, vec![3_000, 23_000]);
        // Recovery re-arms the trigger.
        assert!(!c.update(t.plus_ms(30_000), G60, 2, &m).path_hint);
        assert!(!c.update(t.plus_ms(30_500), 20_000, 2, &m).path_hint);
    }
}
