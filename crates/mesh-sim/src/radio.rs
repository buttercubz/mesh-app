//! Radio model (DESIGN §13.2): BLE packet delivery ratio as a logistic function of
//! distance with `R50` = 50 m (pocket) / 80 m (mount), log-normal shadowing
//! (σ = 6 dB, redrawn per pair every 5 s), link drop after 3 s below PDR 0.5,
//! per-hop delay = connection-interval phase (30 ms iOS-central, 15 ms Android↔Android)
//! plus serialization at 120 kbps per link plus retransmissions (geometric in PDR, one
//! CI each), and a per-node airtime limit equal to `Limits::node_ble_budget_bps`.
//! Wi-Fi Aware: R50 120 m, 5 ± 2 ms, 20 Mbps, unreliable datagrams.

use mesh_core::{LinkKind, Platform};

use crate::rng::{mix, normal_from};

/// Where the phone is carried (sets `R50`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Mount {
    /// In a pocket or a jacket (body shadowing): R50 = 50 m.
    Pocket,
    /// On the handlebar / dashboard mount: R50 = 80 m.
    Mount,
}

/// Tunable radio model parameters (defaults = DESIGN §13.2).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RadioParams {
    /// BLE distance at which PDR = 0.5, phone in a pocket.
    pub r50_pocket_m: f64,
    /// BLE distance at which PDR = 0.5, phone on a mount.
    pub r50_mount_m: f64,
    /// Width of the logistic transition as a fraction of R50 (PDR 0.73 at
    /// `R50 − width`, 0.27 at `R50 + width`). 3.5 % puts the 95 % → 5 % transition
    /// within ±10 % of R50 (≈ ±1 dB of path loss, a BLE PER-vs-SNR cliff), so a phone
    /// at 0.9·R50 links well (S2's 45 m chain) and one at 1.8·R50 does not.
    pub pdr_width_frac: f64,
    /// Log-normal shadowing standard deviation (dB) of the received signal (RSSI in
    /// discovery and link metrics, which the engine's link selection reads).
    pub shadow_sigma_db: f64,
    /// Fraction of the shadowing that also moves the PDR curve. The §13.2 PDR(d) is
    /// the logistic of distance; with the full 6 dB on the PDR cliff at R50 any link
    /// beyond ~0.5·R50 would flap (S2's 45 m chain could not exist), so the default
    /// couples none of it. Experiments raise it (1.0 = full coupling).
    pub pdr_shadow_frac: f64,
    /// Shadowing is redrawn per pair every this many µs.
    pub shadow_window_us: u64,
    /// Path-loss exponent converting shadowing dB into an effective distance factor.
    pub path_loss_exp: f64,
    /// A link whose PDR stays below this for `drop_after_us` is dropped (supervision).
    pub drop_below_pdr: f64,
    pub drop_after_us: u64,
    /// Connection interval of links with an iPhone central.
    pub ci_ios_us: u64,
    /// Connection interval of Android↔Android links.
    pub ci_android_us: u64,
    /// Serialization rate per BLE link (bit/s).
    pub ble_link_bps: u64,
    /// Extra airtime reserved on a node while a headset is active (bit/s). The design
    /// budget already includes the eSCO reservation (T0c measures `NODE_BLE_BUDGET`
    /// with HFP active), so the default is 0; experiments may raise it.
    pub esco_reserve_bps: u64,
    /// Below this PDR a BLE unit cannot get through at all (the direction stalls
    /// until the PDR recovers or the link drops).
    pub stall_pdr: f64,
    /// Wi-Fi Aware R50, delay (mean ± jitter) and rate.
    pub aware_r50_m: f64,
    pub aware_delay_us: u64,
    pub aware_jitter_us: u64,
    pub aware_bps: u64,
    /// Time the platform takes to build an Aware data path once both ends accepted
    /// the upgrade (publish/subscribe + NDP set-up).
    pub aware_setup_us: u64,
    /// OS connection limit per device (`LimitReached` beyond it); `None` = unlimited.
    pub os_link_limit: Option<usize>,
    /// OS send buffer per stream link direction (bytes) before WouldBlock.
    pub stream_buffer_bytes: usize,
    /// OS send queue per GATT link direction (ATT values) before WouldBlock.
    pub gatt_buffer_units: usize,
    /// Delay from `OpenLink` to the data path being up.
    pub open_delay_us: u64,
    /// Extra per-transmission loss on top of the distance model (the web `loss`
    /// scenario parameter): it lowers the PDR of every attempt, so reliable BLE
    /// links pay it in retransmissions and Aware datagrams in losses. Discovery and
    /// supervision are not affected.
    pub extra_loss: f64,
    /// What scanning and advertising cost the links ([`crate::scan`]).
    pub scan: crate::scan::ScanCost,
}

impl Default for RadioParams {
    fn default() -> Self {
        RadioParams {
            r50_pocket_m: 50.0,
            r50_mount_m: 80.0,
            pdr_width_frac: 0.035,
            shadow_sigma_db: 6.0,
            pdr_shadow_frac: 0.0,
            shadow_window_us: 5_000_000,
            path_loss_exp: 2.5,
            drop_below_pdr: 0.5,
            drop_after_us: 3_000_000,
            ci_ios_us: 30_000,
            ci_android_us: 15_000,
            ble_link_bps: 120_000,
            esco_reserve_bps: 0,
            stall_pdr: 0.02,
            aware_r50_m: 120.0,
            aware_delay_us: 5_000,
            aware_jitter_us: 2_000,
            aware_bps: 20_000_000,
            aware_setup_us: 1_500_000,
            os_link_limit: None,
            stream_buffer_bytes: 2_048,
            gatt_buffer_units: 4,
            open_delay_us: 120_000,
            extra_loss: 0.0,
            scan: crate::scan::ScanCost::default(),
        }
    }
}

/// The radio model of one world (seeded, stateless lookups).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RadioModel {
    pub params: RadioParams,
    seed: u64,
}

/// Logistic PDR at effective distance `d` for a given `r50` and width.
pub fn logistic_pdr(d: f64, r50: f64, width: f64) -> f64 {
    let w = width.max(1e-6);
    1.0 / (1.0 + ((d - r50) / w).exp())
}

impl RadioModel {
    pub fn new(params: RadioParams, seed: u64) -> Self {
        RadioModel { params, seed }
    }

    /// `R50` of a node.
    pub fn r50(&self, m: Mount) -> f64 {
        match m {
            Mount::Pocket => self.params.r50_pocket_m,
            Mount::Mount => self.params.r50_mount_m,
        }
    }

    /// `R50` of a pair: the mean of both ends.
    pub fn pair_r50(&self, a: Mount, b: Mount) -> f64 {
        (self.r50(a) + self.r50(b)) / 2.0
    }

    /// Shadowing (dB) of the unordered pair `(a, b)` at time `t_us`: constant over each
    /// window, independent across windows and pairs.
    pub fn shadow_db(&self, a: usize, b: usize, t_us: u64) -> f64 {
        let (lo, hi) = if a <= b { (a, b) } else { (b, a) };
        let win = t_us / self.params.shadow_window_us.max(1);
        let h1 = mix(&[self.seed, 0x5AD0, lo as u64, hi as u64, win]);
        let h2 = mix(&[self.seed, 0x5AD1, lo as u64, hi as u64, win]);
        self.params.shadow_sigma_db * normal_from(h1, h2)
    }

    /// The part of the shadowing that moves the PDR curve.
    pub fn pdr_shadow_db(&self, a: usize, b: usize, t_us: u64) -> f64 {
        let k = self.params.pdr_shadow_frac;
        if k == 0.0 {
            0.0
        } else {
            k * self.shadow_db(a, b, t_us)
        }
    }

    /// Effective distance after shadowing (positive dB = better than the median).
    pub fn effective_distance(&self, d: f64, shadow_db: f64) -> f64 {
        d * 10f64.powf(-shadow_db / (10.0 * self.params.path_loss_exp))
    }

    /// BLE PDR of the pair at distance `d` (metres) and time `t_us`.
    pub fn ble_pdr(&self, a: usize, b: usize, ma: Mount, mb: Mount, d: f64, t_us: u64) -> f64 {
        let r50 = self.pair_r50(ma, mb);
        let de = self.effective_distance(d, self.pdr_shadow_db(a, b, t_us));
        logistic_pdr(de, r50, r50 * self.params.pdr_width_frac)
    }

    /// Wi-Fi Aware PDR of the pair.
    pub fn aware_pdr(&self, a: usize, b: usize, d: f64, t_us: u64) -> f64 {
        let r50 = self.params.aware_r50_m;
        let de = self.effective_distance(d, self.pdr_shadow_db(a, b, t_us));
        logistic_pdr(de, r50, r50 * self.params.pdr_width_frac)
    }

    /// PDR of a link of `kind`.
    #[allow(clippy::too_many_arguments)]
    pub fn pdr(
        &self,
        kind: LinkKind,
        a: usize,
        b: usize,
        ma: Mount,
        mb: Mount,
        d: f64,
        t_us: u64,
    ) -> f64 {
        match kind {
            LinkKind::WifiAware => self.aware_pdr(a, b, d, t_us),
            // The v2 tunnel is not built; model it as a perfect link.
            LinkKind::Tunnel => 1.0,
            LinkKind::BleL2cap | LinkKind::BleGatt => self.ble_pdr(a, b, ma, mb, d, t_us),
        }
    }

    /// Received signal strength (dBm) reported in `Metrics`.
    pub fn rssi(&self, a: usize, b: usize, d: f64, t_us: u64) -> i16 {
        let pl = 10.0 * self.params.path_loss_exp * d.max(1.0).log10();
        let v = -45.0 - pl + self.shadow_db(a, b, t_us);
        v.clamp(-127.0, -20.0).round() as i16
    }

    /// Connection interval of a BLE link between two platforms (§13.2): 15 ms only
    /// when both ends are Android, otherwise an iPhone is the central (30 ms).
    pub fn ble_ci_us(&self, a: Platform, b: Platform) -> u64 {
        if a == Platform::Android && b == Platform::Android {
            self.params.ci_android_us
        } else {
            self.params.ci_ios_us
        }
    }

    /// Serialization time of `bytes` on a link of `kind` (µs, at least 1).
    pub fn serialization_us(&self, kind: LinkKind, bytes: usize) -> u64 {
        let bps = match kind {
            LinkKind::WifiAware | LinkKind::Tunnel => self.params.aware_bps,
            LinkKind::BleL2cap | LinkKind::BleGatt => self.params.ble_link_bps,
        };
        ((bytes as u64 * 8 * 1_000_000) / bps.max(1)).max(1)
    }

    /// Airtime (µs) one transmission of `bytes` consumes from a node budget of
    /// `budget_bps` (minus the eSCO reservation while a headset is active).
    pub fn airtime_us(&self, bytes: usize, budget_bps: u32, headset: bool) -> u64 {
        let mut bps = u64::from(budget_bps);
        if headset {
            bps = bps.saturating_sub(self.params.esco_reserve_bps);
        }
        ((bytes as u64 * 8 * 1_000_000) / bps.max(1)).max(1)
    }

    /// Retransmission cap: as many connection events as fit in the supervision window.
    pub fn max_attempts(&self, ci_us: u64) -> u32 {
        (self.params.drop_after_us / ci_us.max(1)).clamp(1, 10_000) as u32
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn logistic_is_half_at_r50_and_monotone() {
        assert!((logistic_pdr(50.0, 50.0, 5.0) - 0.5).abs() < 1e-12);
        assert!(logistic_pdr(10.0, 50.0, 5.0) > 0.999);
        assert!(logistic_pdr(90.0, 50.0, 5.0) < 0.001);
        let m = RadioModel::new(RadioParams::default(), 1);
        assert_eq!(m.pair_r50(Mount::Pocket, Mount::Mount), 65.0);
    }

    #[test]
    fn shadowing_has_the_configured_sigma_and_holds_per_window() {
        let m = RadioModel::new(RadioParams::default(), 9);
        assert_eq!(m.shadow_db(1, 2, 100), m.shadow_db(2, 1, 4_999_999));
        let xs: Vec<f64> = (0..4_000u64)
            .map(|w| m.shadow_db(0, 1, w * 5_000_000))
            .collect();
        let mean = xs.iter().sum::<f64>() / xs.len() as f64;
        let sd = (xs.iter().map(|x| (x - mean).powi(2)).sum::<f64>() / xs.len() as f64).sqrt();
        assert!(mean.abs() < 0.4 && (sd - 6.0).abs() < 0.4, "{mean} {sd}");
    }

    #[test]
    fn delays_follow_the_design_numbers() {
        let m = RadioModel::new(RadioParams::default(), 1);
        assert_eq!(m.ble_ci_us(Platform::Ios, Platform::Android), 30_000);
        assert_eq!(m.ble_ci_us(Platform::Android, Platform::Android), 15_000);
        // 150 B at 120 kbps = 10 ms; at the 180 kbps node budget = 6.67 ms.
        assert_eq!(m.serialization_us(LinkKind::BleL2cap, 150), 10_000);
        assert_eq!(m.airtime_us(150, 180_000, false), 6_666);
        assert_eq!(m.max_attempts(30_000), 100);
        let near = m.ble_pdr(0, 1, Mount::Pocket, Mount::Pocket, 3.0, 0);
        assert!(near > 0.99, "{near}");
        // Same shadowing at both distances: the difference is 25·log10(20) ≈ 32.5 dB.
        let diff = m.rssi(0, 1, 3.0, 0) - m.rssi(0, 1, 60.0, 0);
        assert!((32..=33).contains(&diff), "{diff}");
    }
}
