//! Cost model (DESIGN §4.8, normative): `cost(k, profile) = k × r_stream(profile) +
//! C_CTRL`, with uncoalesced per-stream rates (BLE16 23.6, BLE12 19.6, BLE-DRED 27.6,
//! WIFI20 34.4 kbps) and `C_CTRL = 3 kbps`. The byte chain lives in mesh-wire
//! (`mesh_wire::budget`); this type binds it to one encoder profile.

use mesh_wire::budget;
use mesh_wire::mesh::EncoderProfile;

/// The cost model for one encoder profile.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CostModel {
    pub profile: EncoderProfile,
}

impl CostModel {
    pub const fn new(profile: EncoderProfile) -> Self {
        CostModel { profile }
    }

    /// Uncoalesced per-stream rate.
    pub const fn r_stream_bps(&self) -> u32 {
        budget::r_stream_bps(self.profile)
    }

    /// `cost(k) = k × r_stream + C_CTRL`.
    pub const fn cost_bps(&self, k: u32) -> u32 {
        budget::cost_bps(k, self.profile)
    }

    /// `cap_pacing = max{k ∈ 1..s_max : cost(k) ≤ 0.9 × grant}` (0 when one stream does
    /// not fit).
    pub const fn cap_pacing(&self, grant_bps: u32, s_max: u8) -> u8 {
        budget::cap_pacing(grant_bps, self.profile, s_max as u32) as u8
    }

    /// The smallest grant that sustains `k` streams (`cost(k) / 0.9`, rounded up).
    pub const fn min_grant_bps(&self, k: u32) -> u32 {
        let c = self.cost_bps(k) as u64 * 1_000;
        c.div_ceil(budget::CAP_UTILISATION_PERMILLE as u64) as u32
    }
}

impl Default for CostModel {
    fn default() -> Self {
        CostModel::new(EncoderProfile::Ble16)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn appendix_a_values() {
        let ble16 = CostModel::new(EncoderProfile::Ble16);
        assert_eq!(ble16.r_stream_bps(), 23_600);
        assert_eq!(ble16.cost_bps(2), 50_200);
        assert_eq!(ble16.cost_bps(1), 26_600);
        // §5.6: cap 2 whenever grant ≥ 55.8 kbps, cap 1 down to 29.6 kbps.
        assert_eq!(ble16.min_grant_bps(2), 55_778);
        assert_eq!(ble16.min_grant_bps(1), 29_556);
        assert_eq!(ble16.cap_pacing(55_778, 2), 2);
        assert_eq!(ble16.cap_pacing(55_777, 2), 1);
        assert_eq!(ble16.cap_pacing(29_556, 2), 1);
        assert_eq!(ble16.cap_pacing(29_555, 2), 0);
        assert_eq!(ble16.cap_pacing(60_000, 2), 2);
        assert_eq!(ble16.cap_pacing(51_000, 2), 1);
        let dred = CostModel::new(EncoderProfile::BleDred);
        // BLE-DRED needs grant ≥ 64.7 kbps for cap 2.
        assert_eq!(dred.min_grant_bps(2), 64_667);
        let ble12 = CostModel::new(EncoderProfile::Ble12);
        assert_eq!(ble12.r_stream_bps(), 19_600);
        let wifi = CostModel::new(EncoderProfile::Wifi20);
        assert_eq!(wifi.r_stream_bps(), 34_400);
        assert_eq!(wifi.cap_pacing(2_000_000, 4), 4);
    }
}
