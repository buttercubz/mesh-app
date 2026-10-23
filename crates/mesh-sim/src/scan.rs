//! What discovery costs the links (DESIGN §5.7, §9.5, first field test): a phone's
//! controller serves scan windows, advertising events and every connection event of
//! its links from one radio. The more it scans, the more connection events it misses:
//! units need more retransmissions (latency), and once the scheduler is oversubscribed
//! long enough a link misses its supervision timeout and drops. Without this cost a
//! policy that scans more looks free in the simulator and drops links on real phones
//! (three phones idle at a table split the group because Android scanned
//! `LOW_LATENCY` whenever it was `Forming`).
//!
//! Model [TUNE: T0c / field logs]:
//! - each node has a radio occupancy `c` from its platform, app state, scan intensity
//!   and advertising rate (Android `LOW_LATENCY` 100 %, `BALANCED` 25 %, `LOW_POWER`
//!   10 %; iOS scans are filtered and coalesced by the OS, so they cost less);
//! - a link between `a` and `b` has the miss weight `m = 1 − (1 − c_a)(1 − c_b)`,
//!   and `x = (m − threshold) / (1 − threshold)` (clamped to 0-1) measures how far the
//!   controller is oversubscribed: it interleaves scan windows around connection
//!   events until then, and loses whole runs of events beyond;
//! - each transmission attempt succeeds with the distance PDR × `(1 − event_miss · x)`;
//! - supervision drops arrive as a Poisson process of rate `drop_rate · x²`.

use mesh_core::{DiscoveryMode, Platform, ScanIntensity};

use crate::config::AppState;

/// Scan cost parameters (defaults: see the module docs).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ScanCost {
    /// Occupancy of an Android scanner at `[LowLatency, Balanced, LowPower]`.
    pub android: [f64; 3],
    /// Occupancy of a foreground iPhone scanner at `[LowLatency, Balanced, LowPower]`
    /// (`AllowDuplicates` only at `LowLatency`).
    pub ios_fg: [f64; 3],
    /// Occupancy of a background iPhone scanner (filtered, coalesced by the OS).
    pub ios_bg: f64,
    /// Extra Android occupancy while `Forming` (250 ms advertising, all-matches
    /// aggressive scan callbacks) and while `Stable` (1 s advertising).
    pub android_adv_forming: f64,
    pub android_adv_stable: f64,
    /// Fraction of connection events lost at full oversubscription (`x = 1`).
    pub event_miss: f64,
    /// Miss weight below which scanning costs the links nothing.
    pub drop_threshold: f64,
    /// Supervision drops per second of a link at miss weight 1.
    pub drop_rate_per_s: f64,
    /// Discovery mean factor of the scanner at `[LowLatency, Balanced, LowPower]`.
    pub android_discovery: [f64; 3],
    pub ios_discovery: [f64; 3],
    /// Discovery mean factor when the advertiser is an Android phone in `Stable`
    /// (1 s advertising instead of 250 ms).
    pub android_stable_adv_discovery: f64,
    /// Experiments: every node scans at this intensity whenever discovery is on,
    /// whatever the engine asked (a driver that ignores the intensity).
    pub force_scan: Option<ScanIntensity>,
}

impl Default for ScanCost {
    fn default() -> Self {
        ScanCost {
            android: [1.0, 0.25, 0.10],
            ios_fg: [0.35, 0.20, 0.10],
            ios_bg: 0.05,
            android_adv_forming: 0.02,
            android_adv_stable: 0.005,
            event_miss: 0.3,
            drop_threshold: 0.3,
            drop_rate_per_s: 1.0 / 120.0,
            android_discovery: [1.0, 2.0, 4.0],
            ios_discovery: [1.0, 1.5, 2.0],
            android_stable_adv_discovery: 1.5,
            force_scan: None,
        }
    }
}

fn idx(s: ScanIntensity) -> usize {
    match s {
        ScanIntensity::LowLatency => 0,
        ScanIntensity::Balanced => 1,
        ScanIntensity::LowPower => 2,
    }
}

impl ScanCost {
    /// The intensity a node actually scans at.
    pub fn effective(&self, asked: ScanIntensity) -> ScanIntensity {
        self.force_scan.unwrap_or(asked)
    }

    /// Radio occupancy of a node.
    pub fn occupancy(
        &self,
        platform: Platform,
        app: AppState,
        mode: DiscoveryMode,
        scan: ScanIntensity,
    ) -> f64 {
        if mode == DiscoveryMode::Off {
            return 0.0;
        }
        let s = idx(self.effective(scan));
        let c = match (platform, app) {
            (Platform::Android, _) => {
                let adv = if mode == DiscoveryMode::Forming {
                    self.android_adv_forming
                } else {
                    self.android_adv_stable
                };
                self.android[s] + adv
            }
            (_, AppState::Foreground) => self.ios_fg[s],
            (_, AppState::Background) => self.ios_bg,
        };
        c.clamp(0.0, 1.0)
    }

    /// Miss weight of a link between nodes of occupancy `ca` and `cb`.
    pub fn miss(ca: f64, cb: f64) -> f64 {
        1.0 - (1.0 - ca) * (1.0 - cb)
    }

    /// How far a miss weight `m` is into oversubscription (0 below the threshold, 1 at
    /// `m = 1`).
    fn excess(&self, m: f64) -> f64 {
        let t = self.drop_threshold.clamp(0.0, 0.999);
        ((m - t) / (1.0 - t)).clamp(0.0, 1.0)
    }

    /// Factor on the per-attempt delivery ratio of a link of miss weight `m`.
    pub fn availability(&self, m: f64) -> f64 {
        (1.0 - self.event_miss * self.excess(m)).clamp(0.0, 1.0)
    }

    /// Supervision drop rate (per second) of a link of miss weight `m`.
    pub fn drop_rate(&self, m: f64) -> f64 {
        let x = self.excess(m);
        self.drop_rate_per_s * x * x
    }

    /// Factor on the discovery mean of a scanner.
    pub fn discovery_factor(&self, platform: Platform, scan: ScanIntensity) -> f64 {
        let s = idx(self.effective(scan));
        match platform {
            Platform::Android => self.android_discovery[s],
            _ => self.ios_discovery[s],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn continuous_android_scanning_drops_links_and_low_power_does_not() {
        let c = ScanCost::default();
        let fg = AppState::Foreground;
        let and = |m, s| c.occupancy(Platform::Android, fg, m, s);
        let ios = |m, s| c.occupancy(Platform::Ios, fg, m, s);
        let ll = and(DiscoveryMode::Forming, ScanIntensity::LowLatency);
        let bal = and(DiscoveryMode::Forming, ScanIntensity::Balanced);
        let lp = and(DiscoveryMode::Stable, ScanIntensity::LowPower);
        let ilp = ios(DiscoveryMode::Stable, ScanIntensity::LowPower);
        // The field case: an Android phone scanning LOW_LATENCY loses a link to an
        // idle iPhone within minutes.
        let field = c.drop_rate(ScanCost::miss(ll, ilp));
        assert!(field > 1.0 / 180.0, "{field}");
        // Stable phones never starve their links.
        assert_eq!(c.drop_rate(ScanCost::miss(lp, lp)), 0.0);
        assert_eq!(c.drop_rate(ScanCost::miss(lp, ilp)), 0.0);
        // Hunting at BALANCED costs far less than LOW_LATENCY (hours per drop).
        let hunt = c.drop_rate(ScanCost::miss(bal, bal));
        assert!(hunt > 0.0 && hunt < 1.0 / 1_800.0, "{hunt}");
        assert!(c.availability(ScanCost::miss(ll, lp)) < c.availability(ScanCost::miss(lp, lp)));
        assert_eq!(
            c.occupancy(
                Platform::Android,
                fg,
                DiscoveryMode::Off,
                ScanIntensity::LowLatency
            ),
            0.0
        );
        // Discovery is slower at lower intensity.
        assert!(
            c.discovery_factor(Platform::Android, ScanIntensity::LowPower)
                > c.discovery_factor(Platform::Android, ScanIntensity::Balanced)
        );
    }

    #[test]
    fn a_forced_intensity_overrides_the_engine() {
        let c = ScanCost {
            force_scan: Some(ScanIntensity::LowLatency),
            ..ScanCost::default()
        };
        let o = c.occupancy(
            Platform::Android,
            AppState::Foreground,
            DiscoveryMode::Stable,
            ScanIntensity::LowPower,
        );
        assert!(o > 0.99);
    }
}
