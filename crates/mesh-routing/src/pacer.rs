//! Delay-based pacer (DESIGN §5.6): a token bucket at rate `R` per link (initial L2CAP
//! 60 kbps, GATT 40 kbps, Aware 2 Mbps, Tunnel 64 kbps; bounds [16 kbps, 4 Mbps]).
//!
//! Every 500 ms, with `qd = srtt − min_rtt` (min over 30 s): if `qd > 40 ms` or a
//! `WouldBlock` happened ⇒ `R ← 0.85 R`; if `qd < 15 ms` and the queue was non-empty ⇒
//! `R ← R + 4 kbps` (BLE, Tunnel) or `R × 1.1` (Aware). BLE is reliable at the link layer,
//! so overload shows up as delay. The bucket drains at `min(R, grant)`.
//!
//! **Voice floor (fading is not congestion).** The decrease never takes `R` below the
//! airtime of **one stream of the lowest profile** of the link kind
//! ([`PacerPath::voice_floor_bps`]: BLE12 `cost(1) / 0.9` = 25.1 kbps on BLE). A weak
//! link (edge of range, a ground-reflection null) also shows up as delay and
//! `WouldBlock`: every lost attempt costs the reliable link layer a connection interval.
//! That delay is not ours to remove: at one stream or less our own demand is not what
//! builds the queue, and pacing below one stream only guarantees that no voice gets
//! through (cap 0, `FLOOR_REJECT{CONGESTION}` for a lone talker, field test
//! 2026-10-24). Congestion from too many streams is still fully answered above the floor:
//! at the floor `cap_pacing` is 1, so every extra stream is shed (§6.3).
//!
//! Interpretation of "the queue was non-empty" (reported as a design note), chosen so
//! that the §5.6 steady-state guarantee holds:
//! - **Recovery** (`R` below the initial rate of its path): any traffic in a period
//!   without a congestion signal (`qd ≤ 40 ms`, no `WouldBlock`) is enough, so a single
//!   multiplicative decrease (60 → 51 kbps) is undone within ~1.5 s "when the queue is
//!   not building". Recovery deliberately does not wait for `qd < 15 ms`: on a 30 ms
//!   iOS-central connection interval the RTT phase alone keeps `srtt − min_rtt` (min
//!   over 30 s, a lucky in-phase sample) around 15-30 ms, and a link cut once at the
//!   edge of range then stayed at the voice floor for the rest of the ride (mesh-sim
//!   S15, field test 2026-10-24).
//! - **Probing above the initial rate** needs real backlog (a send had to wait for
//!   tokens in the period) **and** `R ≤ grant(L)`. A link whose demand fits never inflates
//!   `R`, so idle and voice links keep comparable `R` values and the proportional airtime
//!   scaling (§5.6) never squeezes a link that starts carrying voice; probing above the
//!   airtime the node budget grants would be pointless.

use mesh_wire::mesh::EncoderProfile;

use crate::consts::{
    PACER_PERIOD_US, QD_HIGH_US, QD_LOW_US, R_ADD_BPS, R_AWARE_INCREASE_PERMILLE,
    R_DECREASE_PERMILLE, R_INIT_AWARE_BPS, R_INIT_GATT_BPS, R_INIT_L2CAP_BPS, R_INIT_TUNNEL_BPS,
    R_MAX_BPS, R_MIN_BPS,
};
use crate::cost::CostModel;
use crate::types::{Instant, LinkKind};

/// Data path of a link (the initial rate depends on it).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PacerPath {
    L2cap,
    Gatt,
    Aware,
    Tunnel,
}

impl PacerPath {
    pub const fn initial_bps(self) -> u32 {
        match self {
            PacerPath::L2cap => R_INIT_L2CAP_BPS,
            PacerPath::Gatt => R_INIT_GATT_BPS,
            PacerPath::Aware => R_INIT_AWARE_BPS,
            PacerPath::Tunnel => R_INIT_TUNNEL_BPS,
        }
    }

    pub const fn kind(self) -> LinkKind {
        match self {
            PacerPath::L2cap | PacerPath::Gatt => LinkKind::Ble,
            PacerPath::Aware => LinkKind::Aware,
            PacerPath::Tunnel => LinkKind::Tunnel,
        }
    }

    /// The lowest voice profile a link of this path can carry (BLE12 on BLE, WIFI20 on
    /// Aware and Tunnel).
    pub const fn lowest_profile(self) -> EncoderProfile {
        match self.kind() {
            LinkKind::Ble => EncoderProfile::Ble12,
            LinkKind::Aware | LinkKind::Tunnel => EncoderProfile::Wifi20,
        }
    }

    /// The multiplicative decrease stops here: the grant that sustains one stream of the
    /// lowest profile (`cost(1) / 0.9`; BLE 25 112 bps, Aware/Tunnel 41 556 bps).
    pub const fn voice_floor_bps(self) -> u32 {
        CostModel::new(self.lowest_profile()).min_grant_bps(1)
    }
}

/// Outcome of one control step.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PacerSample {
    pub rate_bps: u32,
    pub decreased: bool,
    pub increased: bool,
}

/// One link's pacer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pacer {
    path: PacerPath,
    rate_bps: u32,
    grant_bps: u32,
    /// Available tokens in bit·µs / 1e6 units (i.e. bits, kept as µbits for precision).
    tokens_ubits: u64,
    last_refill: Instant,
    next_control: Instant,
    would_block: bool,
    had_traffic: bool,
    backlogged: bool,
    max_frame_bytes: u32,
}

const BUCKET_US: u64 = 40_000;

impl Pacer {
    pub fn new(now: Instant, path: PacerPath, max_frame_bytes: u32) -> Self {
        let r = path.initial_bps();
        let mut p = Pacer {
            path,
            rate_bps: r,
            grant_bps: r,
            tokens_ubits: 0,
            last_refill: now,
            next_control: now.plus_us(PACER_PERIOD_US),
            would_block: false,
            had_traffic: false,
            backlogged: false,
            max_frame_bytes,
        };
        p.tokens_ubits = p.depth_ubits();
        p
    }

    /// Current `R`.
    pub fn rate_bps(&self) -> u32 {
        self.rate_bps
    }

    /// Current grant (from the airtime budget).
    pub fn grant_bps(&self) -> u32 {
        self.grant_bps
    }

    /// Drain rate `min(R, grant)`.
    pub fn send_rate_bps(&self) -> u32 {
        self.rate_bps.min(self.grant_bps)
    }

    pub fn path(&self) -> PacerPath {
        self.path
    }

    /// Sets the airtime grant (never above `R`, never below 1 bps).
    pub fn set_grant(&mut self, now: Instant, grant_bps: u32) {
        self.refill(now);
        self.grant_bps = grant_bps.max(1);
    }

    fn depth_ubits(&self) -> u64 {
        let by_rate = u64::from(self.send_rate_bps()) * BUCKET_US;
        let one_frame = u64::from(self.max_frame_bytes + 2) * 8 * 1_000_000;
        by_rate.max(one_frame)
    }

    fn refill(&mut self, now: Instant) {
        let dt = now.since(self.last_refill);
        self.last_refill = self.last_refill.max(now);
        let add = u64::from(self.send_rate_bps()) * dt;
        self.tokens_ubits = (self.tokens_ubits + add).min(self.depth_ubits());
    }

    /// `true` if `bytes` may be sent now. A `false` means data waits for tokens: the
    /// period counts as backlogged.
    pub fn can_send(&mut self, now: Instant, bytes: u32) -> bool {
        self.refill(now);
        let ok = self.tokens_ubits >= u64::from(bytes) * 8 * 1_000_000;
        self.had_traffic = true;
        if !ok {
            self.backlogged = true;
        }
        ok
    }

    /// µs until `bytes` may be sent (0 if now).
    pub fn wait_us(&mut self, now: Instant, bytes: u32) -> u64 {
        self.refill(now);
        let need = u64::from(bytes) * 8 * 1_000_000;
        if self.tokens_ubits >= need {
            0
        } else {
            self.backlogged = true;
            (need - self.tokens_ubits).div_ceil(u64::from(self.send_rate_bps()).max(1))
        }
    }

    /// Accounts `bytes` sent now.
    pub fn on_sent(&mut self, now: Instant, bytes: u32) {
        self.refill(now);
        self.tokens_ubits = self
            .tokens_ubits
            .saturating_sub(u64::from(bytes) * 8 * 1_000_000);
        self.had_traffic = true;
    }

    /// The platform returned `WouldBlock`.
    pub fn on_would_block(&mut self) {
        self.would_block = true;
    }

    /// Something was queued on the link (the queue was non-empty in this period).
    pub fn note_queued(&mut self) {
        self.had_traffic = true;
    }

    /// Data waited for tokens in this period (demand exceeded the drain rate).
    pub fn note_backlogged(&mut self) {
        self.had_traffic = true;
        self.backlogged = true;
    }

    /// Next control instant.
    pub fn next_control(&self) -> Instant {
        self.next_control
    }

    /// Runs the control law if a period has elapsed. `qd_us = srtt − min_rtt`.
    pub fn tick(&mut self, now: Instant, qd_us: u64) -> Option<PacerSample> {
        if now < self.next_control {
            return None;
        }
        self.refill(now);
        while self.next_control <= now {
            self.next_control = self.next_control.plus_us(PACER_PERIOD_US);
        }
        let mut s = PacerSample {
            rate_bps: self.rate_bps,
            decreased: false,
            increased: false,
        };
        if qd_us > QD_HIGH_US || self.would_block {
            let cut = (u64::from(self.rate_bps) * u64::from(R_DECREASE_PERMILLE) / 1_000) as u32;
            // Never below one stream of the lowest profile (fading is not congestion);
            // a rate already below the floor is never raised by a decrease.
            let floor = self.path.voice_floor_bps().min(self.rate_bps);
            self.rate_bps = cut.max(floor);
            s.decreased = self.rate_bps < s.rate_bps;
        } else if self.had_traffic
            && ((self.rate_bps < self.path.initial_bps())
                || (qd_us < QD_LOW_US && self.backlogged && self.rate_bps <= self.grant_bps))
        {
            self.rate_bps = match self.path.kind() {
                LinkKind::Aware => {
                    (u64::from(self.rate_bps) * u64::from(R_AWARE_INCREASE_PERMILLE) / 1_000) as u32
                }
                LinkKind::Ble | LinkKind::Tunnel => self.rate_bps.saturating_add(R_ADD_BPS),
            };
            s.increased = true;
        }
        self.rate_bps = self.rate_bps.clamp(R_MIN_BPS, R_MAX_BPS);
        s.rate_bps = self.rate_bps;
        self.would_block = false;
        self.had_traffic = false;
        self.backlogged = false;
        self.tokens_ubits = self.tokens_ubits.min(self.depth_ubits());
        Some(s)
    }
}

#[cfg(test)]
#[path = "pacer_tests.rs"]
mod tests;
