//! S15 distance-sweep (field test 2026-10-24 follow-up): the S14 pair (iPhone central +
//! Android tablet, PTT turns) at 5, 10, ... 60 m under a harsher radio model than the
//! §13.2 default: half of the 6 dB log-normal shadowing moves the PDR cliff, redrawn
//! every 2 s (body and traffic fading), plus the ground-reflection null at 10.5 m.
//!
//! A distance is *usable* when the pair forms, no lone talker is ever refused with
//! `talkDenied{congestion}` and the listener gets ≥ 80 % of the scripted speech. The
//! scenario reports the largest distance up to which every point is usable and asserts
//! (a) zero congestion refusals anywhere and (b) that distance ≥ [`MIN_USABLE_M`].

use super::Outcome;
use super::s14::{Case, FIELD_NULL, run_case_with};
use crate::config::WorldConfig;

/// Sweep points (m).
pub const DISTANCES_M: [f64; 12] = [
    5.0, 10.0, 15.0, 20.0, 25.0, 30.0, 35.0, 40.0, 45.0, 50.0, 55.0, 60.0,
];
/// Talk time per point (µs).
const TALK_US: u64 = 20_000_000;
/// Minimum share of the speech a usable point delivers.
pub const MIN_DELIVERED: f64 = 0.80;
/// The sweep must stay usable at least this far (m). Regression guard: under this
/// model both CI seeds reach 30-35 m; beyond, 2 s shadow windows push the PDR to
/// 0-0.3 and the reliable link layer cannot carry 25 frames/s through them.
pub const MIN_USABLE_M: f64 = 30.0;

/// The S15 radio model on top of the §13.2 defaults.
pub fn harsh_radio(cfg: &mut WorldConfig) {
    cfg.radio.pdr_shadow_frac = 0.5;
    cfg.radio.shadow_window_us = 2_000_000;
}

/// One sweep point.
#[derive(Debug, Clone, Copy, Default)]
pub struct Point {
    pub distance_m: f64,
    pub formed: bool,
    pub denied_congestion: usize,
    pub delivered: f64,
}

impl Point {
    pub fn usable(&self) -> bool {
        self.formed && self.denied_congestion == 0 && self.delivered >= MIN_DELIVERED
    }
}

/// Runs the sweep; `o` collects engine failures.
pub fn sweep(seed: u64, o: &mut Outcome) -> Vec<Point> {
    DISTANCES_M
        .iter()
        .map(|&d| {
            let case = Case {
                name: "sweep",
                distance_m: d,
                null: Some(FIELD_NULL),
                min_delivered: MIN_DELIVERED,
                max_loss: None,
            };
            // A point that does not form is a sweep result, not a scenario failure.
            let mut scratch = Outcome::new("S15", seed);
            let r = run_case_with(seed, &case, TALK_US, &harsh_radio, &mut scratch);
            if r.formed {
                o.failures.extend(scratch.failures);
            }
            o.violations.extend(scratch.violations);
            o.digest = crate::rng::mix(&[o.digest, scratch.digest]);
            o.sim_s += scratch.sim_s;
            Point {
                distance_m: d,
                formed: r.formed,
                denied_congestion: r.denied_congestion,
                delivered: r.delivered,
            }
        })
        .collect()
}

/// The largest distance up to which every point is usable (0 when the first is not).
pub fn max_usable_m(points: &[Point]) -> f64 {
    points
        .iter()
        .take_while(|p| p.usable())
        .last()
        .map_or(0.0, |p| p.distance_m)
}

pub fn run(seed: u64) -> Outcome {
    let mut o = Outcome::new("S15", seed);
    let points = sweep(seed, &mut o);
    let mut congestion = 0;
    for p in &points {
        let d = p.distance_m as u32;
        o.report(&format!("d{d:02}.delivered"), p.delivered);
        congestion += p.denied_congestion;
    }
    o.metric(
        "denied.congestion",
        congestion as f64,
        congestion == 0,
        "= 0 at every distance",
    );
    let max = max_usable_m(&points);
    o.metric(
        "max.usable.m",
        max,
        max >= MIN_USABLE_M,
        &format!("≥ {MIN_USABLE_M}"),
    );
    o
}
