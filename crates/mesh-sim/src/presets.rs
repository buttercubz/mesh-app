//! Scenario presets for the browser simulator and quick experiments (DESIGN §11.2
//! `mw_world_new({preset, nodes, loss, seed})`, `?sim=chain5&loss=0.1`): the
//! geometries of S1 (`pair`), S2 (`chain5`), S3 (`cluster15`) and S4 (`convoy15`).
//! A preset only builds a [`WorldConfig`]; the caller starts the engines (Lab mode,
//! a group session, the Demo world) on top of it.

use crate::config::{NodeSpec, WorldConfig};
use crate::mobility::{Convoy, ConvoyParams, Mobility};
use crate::rng::SimRng;

/// The named geometries.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Preset {
    /// Two phones 10 m apart (S1 pair-ble).
    Pair,
    /// A line with 45 m spacing (S2 chain5-ble).
    Chain5,
    /// A highway convoy at 90-110 km/h with log-normal gaps (S4 convoy15-highway).
    Convoy15,
    /// Phones scattered inside a 30 m circle (S3 cluster15).
    Cluster15,
}

/// Options a caller may override; `None` keeps the preset's default.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct PresetOptions {
    /// Number of phones (at least 2).
    pub nodes: Option<usize>,
    /// Extra per-transmission loss in `[0, 1]` (see `RadioParams::extra_loss`).
    pub loss: Option<f64>,
}

/// Spacing of `chain5` (§13.2 S2).
pub const CHAIN_SPACING_M: f64 = 45.0;
/// Distance of `pair` (§13.2 S1).
pub const PAIR_DISTANCE_M: f64 = 10.0;
/// Diameter of `cluster15` (§13.2 S3).
pub const CLUSTER_DIAMETER_M: f64 = 30.0;

impl Preset {
    /// Every preset, in the order the web UI lists them.
    pub const ALL: [Preset; 4] = [
        Preset::Pair,
        Preset::Chain5,
        Preset::Convoy15,
        Preset::Cluster15,
    ];

    /// The wire / query-parameter name.
    pub const fn name(self) -> &'static str {
        match self {
            Preset::Pair => "pair",
            Preset::Chain5 => "chain5",
            Preset::Convoy15 => "convoy15",
            Preset::Cluster15 => "cluster15",
        }
    }

    /// Parses a name (case-insensitive).
    pub fn parse(s: &str) -> Option<Preset> {
        Preset::ALL
            .into_iter()
            .find(|p| p.name().eq_ignore_ascii_case(s.trim()))
    }

    /// Default number of phones.
    pub const fn default_nodes(self) -> usize {
        match self {
            Preset::Pair => 2,
            Preset::Chain5 => 5,
            Preset::Convoy15 | Preset::Cluster15 => 15,
        }
    }

    /// The world of this preset for `seed` (every node a foreground iPhone in a
    /// pocket, the current field fleet).
    pub fn config(self, seed: u64, opts: PresetOptions) -> WorldConfig {
        let n = opts.nodes.unwrap_or(self.default_nodes()).max(2);
        let mobility = match self {
            Preset::Pair => Mobility::line(n, PAIR_DISTANCE_M),
            Preset::Chain5 => Mobility::line(n, CHAIN_SPACING_M),
            Preset::Cluster15 => cluster(seed, n, CLUSTER_DIAMETER_M / 2.0),
            Preset::Convoy15 => Mobility::Convoy(Box::new(Convoy::new(
                n,
                ConvoyParams::default(),
                SimRng::new(seed, 0x0C0_4F00),
            ))),
        };
        let nodes = (0..n)
            .map(|i| {
                let mut s = NodeSpec::iphone();
                s.model = format!("iPhone{},{}", 13 + i % 4, 1 + i % 3);
                s
            })
            .collect();
        let mut cfg = WorldConfig::new(seed, nodes, mobility);
        if let Some(loss) = opts.loss {
            cfg.radio.extra_loss = loss.clamp(0.0, 1.0);
        }
        cfg
    }
}

/// `n` positions drawn uniformly inside a disc of `radius_m` (seeded).
pub fn cluster(seed: u64, n: usize, radius_m: f64) -> Mobility {
    let mut rng = SimRng::new(seed, 0xC1_0575);
    let pts = (0..n)
        .map(|_| {
            // sqrt for a uniform density over the area.
            let r = radius_m * rng.f64().sqrt();
            let a = rng.range_f64(0.0, core::f64::consts::TAU);
            (r * a.cos(), r * a.sin())
        })
        .collect();
    Mobility::Static(pts)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_round_trip_and_geometries_match_the_scenarios() {
        for p in Preset::ALL {
            assert_eq!(Preset::parse(p.name()), Some(p));
            assert_eq!(Preset::parse(&p.name().to_uppercase()), Some(p));
            let c = p.config(7, PresetOptions::default());
            assert_eq!(c.nodes.len(), p.default_nodes());
        }
        assert_eq!(Preset::parse("ring9"), None);
        let chain = Preset::Chain5.config(1, PresetOptions::default());
        assert!((chain.mobility.distance(0, 4) - 180.0).abs() < 1e-9);
        let cl = Preset::Cluster15.config(3, PresetOptions::default());
        for a in 0..15 {
            for b in 0..15 {
                assert!(cl.mobility.distance(a, b) <= CLUSTER_DIAMETER_M + 1e-9);
            }
        }
        assert!(
            Preset::Convoy15
                .config(1, PresetOptions::default())
                .mobility
                .is_dynamic()
        );
        let opts = PresetOptions {
            nodes: Some(3),
            loss: Some(0.1),
        };
        let pair = Preset::Pair.config(1, opts);
        assert_eq!(pair.nodes.len(), 3);
        assert_eq!(pair.radio.extra_loss, 0.1);
        // Same seed, same cluster; different seed, different cluster.
        let a = Preset::Cluster15.config(3, PresetOptions::default());
        let b = Preset::Cluster15.config(4, PresetOptions::default());
        assert_eq!(a.mobility.position(5), cl.mobility.position(5));
        assert_ne!(a.mobility.position(5), b.mobility.position(5));
    }
}
