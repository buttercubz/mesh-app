//! Mobility model (DESIGN §13.2): fixed positions for benches and clusters, and a
//! 1-D road with curves for convoys (speed profiles 0-33 m/s, stops, log-normal gap
//! distributions). Positions are metres in a local plane; [`to_lat_lon`] maps them
//! to GPS fixes for platform samples.

use crate::rng::SimRng;

/// A road along `x` with sinusoidal curves: `y = amplitude · sin(2π x / wavelength)`.
/// The road coordinate `s` is the `x` projection, so the Euclidean distance between
/// two riders is the `x` gap plus the lateral offset the curve adds.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Road {
    pub amplitude_m: f64,
    pub wavelength_m: f64,
}

impl Default for Road {
    fn default() -> Self {
        Road {
            amplitude_m: 60.0,
            wavelength_m: 900.0,
        }
    }
}

impl Road {
    /// The point at road coordinate `s` (metres; `x = s`).
    pub fn point(&self, s: f64) -> (f64, f64) {
        let y = if self.wavelength_m > 0.0 {
            self.amplitude_m * (core::f64::consts::TAU * s / self.wavelength_m).sin()
        } else {
            0.0
        };
        (s, y)
    }
}

/// Convoy parameters (S4 defaults: 90-110 km/h, gaps median 40 m within 20-500 m).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ConvoyParams {
    pub road: Road,
    /// Cruise speed range of the leader (m/s, ≤ 33).
    pub speed_min_mps: f64,
    pub speed_max_mps: f64,
    /// Log-normal gap distribution: median, log-space sigma, clamp range.
    pub gap_median_m: f64,
    pub gap_sigma: f64,
    pub gap_min_m: f64,
    pub gap_max_m: f64,
    /// Probability that a new leader segment is a stop.
    pub stop_prob: f64,
    /// Segment duration range (s) and stop duration range (s).
    pub segment_s: (f64, f64),
    pub stop_s: (f64, f64),
    /// Mean time between gap-target redraws per follower (s).
    pub gap_redraw_mean_s: f64,
    /// Acceleration / braking limits (m/s²).
    pub accel_mps2: f64,
    pub brake_mps2: f64,
}

impl Default for ConvoyParams {
    fn default() -> Self {
        ConvoyParams {
            road: Road::default(),
            speed_min_mps: 25.0,
            speed_max_mps: 30.5,
            gap_median_m: 40.0,
            gap_sigma: 0.8,
            gap_min_m: 20.0,
            gap_max_m: 500.0,
            stop_prob: 0.0,
            segment_s: (30.0, 120.0),
            stop_s: (10.0, 60.0),
            gap_redraw_mean_s: 120.0,
            accel_mps2: 2.0,
            brake_mps2: 4.0,
        }
    }
}

/// A convoy on the road: vehicle 0 leads, vehicle `i` follows `i − 1`.
#[derive(Debug, Clone)]
pub struct Convoy {
    pub params: ConvoyParams,
    /// Road coordinate of each vehicle.
    pub s: Vec<f64>,
    /// Speed of each vehicle (m/s).
    pub v: Vec<f64>,
    gap_target: Vec<f64>,
    gap_redraw_at: Vec<f64>,
    leader_target: f64,
    segment_end: f64,
    t: f64,
    rng: SimRng,
}

impl Convoy {
    /// `n` vehicles with gaps drawn from the distribution, all at cruise speed.
    pub fn new(n: usize, params: ConvoyParams, rng: SimRng) -> Self {
        let mut c = Convoy {
            params,
            s: vec![0.0; n],
            v: vec![0.0; n],
            gap_target: vec![params.gap_median_m; n],
            gap_redraw_at: vec![0.0; n],
            leader_target: params.speed_min_mps,
            segment_end: 0.0,
            t: 0.0,
            rng,
        };
        let v0 = c.rng.range_f64(params.speed_min_mps, params.speed_max_mps);
        c.leader_target = v0;
        let mut s = 0.0;
        for i in 0..n {
            if i > 0 {
                let g = c.draw_gap();
                c.gap_target[i] = g;
                s -= g;
            }
            c.s[i] = s;
            c.v[i] = v0;
            c.gap_redraw_at[i] = c.rng.exp(params.gap_redraw_mean_s);
        }
        c.segment_end = c.rng.range_f64(params.segment_s.0, params.segment_s.1);
        c
    }

    /// Parks the convoy `gap_m` apart, everyone stopped, until [`Convoy::depart`]: a
    /// ride formed in a parking lot, then on the road.
    pub fn park(&mut self, gap_m: f64) {
        for i in 0..self.s.len() {
            self.s[i] = -(i as f64) * gap_m;
            self.v[i] = 0.0;
        }
        self.leader_target = 0.0;
        self.segment_end = f64::INFINITY;
    }

    /// The leader pulls away at a cruise speed; followers stretch out to their gaps.
    pub fn depart(&mut self) {
        self.segment_end = self.t;
    }

    fn draw_gap(&mut self) -> f64 {
        let p = self.params;
        self.rng
            .lognormal(p.gap_median_m, p.gap_sigma)
            .clamp(p.gap_min_m, p.gap_max_m)
    }

    /// Current along-road gap between `i` and its predecessor.
    pub fn gap(&self, i: usize) -> Option<f64> {
        (i > 0).then(|| self.s[i - 1] - self.s[i])
    }

    /// Overrides the gap target of follower `i` (scenario scripts: splits, rejoins).
    pub fn set_gap_target(&mut self, i: usize, gap_m: f64, hold_s: f64) {
        if i > 0 && i < self.s.len() {
            self.gap_target[i] = gap_m;
            self.gap_redraw_at[i] = self.t + hold_s;
        }
    }

    fn approach(v: f64, target: f64, accel: f64, brake: f64, dt: f64) -> f64 {
        if target > v {
            (v + accel * dt).min(target)
        } else {
            (v - brake * dt).max(target)
        }
    }

    /// Advances the convoy by `dt` seconds.
    pub fn step(&mut self, dt: f64) {
        let p = self.params;
        self.t += dt;
        if self.t >= self.segment_end {
            if self.rng.chance(p.stop_prob) {
                self.leader_target = 0.0;
                self.segment_end = self.t + self.rng.range_f64(p.stop_s.0, p.stop_s.1);
            } else {
                self.leader_target = self.rng.range_f64(p.speed_min_mps, p.speed_max_mps);
                self.segment_end = self.t + self.rng.range_f64(p.segment_s.0, p.segment_s.1);
            }
        }
        let n = self.s.len();
        if n == 0 {
            return;
        }
        self.v[0] = Self::approach(
            self.v[0],
            self.leader_target,
            p.accel_mps2,
            p.brake_mps2,
            dt,
        );
        for i in 1..n {
            if self.t >= self.gap_redraw_at[i] {
                self.gap_target[i] = self.draw_gap();
                self.gap_redraw_at[i] = self.t + self.rng.exp(p.gap_redraw_mean_s);
            }
            let gap = self.s[i - 1] - self.s[i];
            let want = (self.v[i - 1] + 0.2 * (gap - self.gap_target[i]))
                .clamp(0.0, 33.0_f64.max(p.speed_max_mps * 1.15));
            self.v[i] = Self::approach(self.v[i], want, p.accel_mps2 * 1.5, p.brake_mps2, dt);
        }
        for i in 0..n {
            self.s[i] += self.v[i] * dt;
            if i > 0 && self.s[i] > self.s[i - 1] - 5.0 {
                // Never overtake: keep a 5 m minimum gap.
                self.s[i] = self.s[i - 1] - 5.0;
                self.v[i] = self.v[i].min(self.v[i - 1]);
            }
        }
    }
}

/// Positions of every node.
#[derive(Debug, Clone)]
pub enum Mobility {
    /// Fixed positions (metres); scenario scripts may move nodes.
    Static(Vec<(f64, f64)>),
    /// A convoy on a curved road.
    Convoy(Box<Convoy>),
}

impl Mobility {
    /// Nodes on a line with `spacing_m` between neighbours.
    pub fn line(n: usize, spacing_m: f64) -> Mobility {
        Mobility::Static((0..n).map(|i| (i as f64 * spacing_m, 0.0)).collect())
    }

    /// Position of node `i`.
    pub fn position(&self, i: usize) -> (f64, f64) {
        match self {
            Mobility::Static(p) => p.get(i).copied().unwrap_or((0.0, 0.0)),
            Mobility::Convoy(c) => c.params.road.point(c.s.get(i).copied().unwrap_or(0.0)),
        }
    }

    /// Speed of node `i` (m/s).
    pub fn speed(&self, i: usize) -> f64 {
        match self {
            Mobility::Static(_) => 0.0,
            Mobility::Convoy(c) => c.v.get(i).copied().unwrap_or(0.0),
        }
    }

    /// Euclidean distance between two nodes.
    pub fn distance(&self, a: usize, b: usize) -> f64 {
        let (xa, ya) = self.position(a);
        let (xb, yb) = self.position(b);
        ((xa - xb).powi(2) + (ya - yb).powi(2)).sqrt()
    }

    /// Moves node `i` (static mobility only).
    pub fn set_position(&mut self, i: usize, p: (f64, f64)) {
        if let Mobility::Static(v) = self {
            if v.len() <= i {
                v.resize(i + 1, (0.0, 0.0));
            }
            v[i] = p;
        }
    }

    /// `true` when positions change over time.
    pub fn is_dynamic(&self) -> bool {
        matches!(self, Mobility::Convoy(_))
    }

    /// Advances dynamic models by `dt` seconds.
    pub fn step(&mut self, dt: f64) {
        if let Mobility::Convoy(c) = self {
            c.step(dt);
        }
    }
}

/// Reference point of the local plane (Santo Domingo, as in the labrun/1 examples).
pub const REF_LAT: f64 = 18.4861;
pub const REF_LON: f64 = -69.9312;

/// Maps local metres (x east, y north) to latitude/longitude.
pub fn to_lat_lon(p: (f64, f64)) -> (f64, f64) {
    let lat = REF_LAT + p.1 / 111_195.0;
    let lon = REF_LON + p.0 / (111_195.0 * REF_LAT.to_radians().cos());
    (lat, lon)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn static_line_distances() {
        let m = Mobility::line(3, 35.0);
        assert_eq!(m.distance(0, 2), 70.0);
        assert!(!m.is_dynamic());
        let (lat, lon) = to_lat_lon((1_000.0, 1_000.0));
        assert!((lat - REF_LAT - 0.009).abs() < 1e-3 && lon > REF_LON);
    }

    #[test]
    fn convoy_keeps_order_speed_and_gap_bounds() {
        let p = ConvoyParams {
            stop_prob: 0.2,
            ..ConvoyParams::default()
        };
        let mut c = Convoy::new(15, p, SimRng::new(3, 3));
        let mut max_gap: f64 = 0.0;
        for _ in 0..(30 * 60 * 10) {
            c.step(0.1);
            for i in 1..15 {
                let g = c.gap(i).unwrap();
                assert!(g >= 4.999, "overtake at {i}: {g}");
                max_gap = max_gap.max(g);
            }
            assert!(c.v.iter().all(|v| (0.0..=36.0).contains(v)));
        }
        assert!(c.s[0] > 10_000.0, "the convoy moved: {}", c.s[0]);
        assert!(max_gap > 60.0, "gaps vary: {max_gap}");
        let g = c.gap(1).unwrap();
        let m = Mobility::Convoy(Box::new(c));
        let d = m.distance(0, 1);
        assert!(d >= g - 1e-9 && d <= g * 1.5 + 1.0, "{d} vs gap {g}");
    }

    #[test]
    fn a_parked_convoy_waits_then_stretches_out() {
        let mut c = Convoy::new(5, ConvoyParams::default(), SimRng::new(9, 1));
        c.park(8.0);
        for _ in 0..600 {
            c.step(0.1);
        }
        assert_eq!(c.s[0], 0.0, "parked");
        assert_eq!(c.gap(4), Some(8.0));
        c.depart();
        for _ in 0..1_200 {
            c.step(0.1);
        }
        assert!(
            c.v[0] >= 25.0 && c.s[0] > 1_000.0,
            "the leader left: {}",
            c.s[0]
        );
        assert!((1..5).all(|i| c.gap(i).unwrap() > 8.0), "{:?}", c.s);
    }

    #[test]
    fn road_curves_add_lateral_offset() {
        let r = Road::default();
        let (x0, y0) = r.point(0.0);
        let (x1, y1) = r.point(225.0);
        assert_eq!(y0, 0.0);
        assert!(
            (y1 - 60.0).abs() < 1e-9,
            "quarter wavelength is the crest: {y1}"
        );
        let d = ((x1 - x0).powi(2) + (y1 - y0).powi(2)).sqrt();
        assert!(d > 225.0);
    }
}
