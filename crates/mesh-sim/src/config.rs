//! World configuration: node specs (platform, mount, app state, headset), the
//! discovery model of DESIGN §13.2, radio parameters, mobility and engine limits.

use mesh_core::{Limits, Platform};

use crate::mobility::Mobility;
use crate::radio::{Mount, RadioParams};

/// Foreground / background state of the app on a node (iPhone advertising and
/// scanning depend on it, §9.4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum AppState {
    Foreground,
    Background,
}

/// One simulated phone.
#[derive(Debug, Clone, PartialEq)]
pub struct NodeSpec {
    pub platform: Platform,
    pub mount: Mount,
    pub app: AppState,
    /// A Bluetooth headset (HFP) is connected: the audio route reports it.
    pub headset: bool,
    /// Hardware model written in Lab files.
    pub model: String,
}

impl NodeSpec {
    /// A foreground iPhone in a pocket without headset.
    pub fn iphone() -> Self {
        NodeSpec {
            platform: Platform::Ios,
            mount: Mount::Pocket,
            app: AppState::Foreground,
            headset: false,
            model: "iPhone15,2".into(),
        }
    }

    /// A foreground Android phone in a pocket without headset.
    pub fn android() -> Self {
        NodeSpec {
            platform: Platform::Android,
            mount: Mount::Pocket,
            app: AppState::Foreground,
            headset: false,
            model: "Pixel 9".into(),
        }
    }

    pub fn with_mount(mut self, m: Mount) -> Self {
        self.mount = m;
        self
    }

    pub fn with_app(mut self, a: AppState) -> Self {
        self.app = a;
        self
    }

    pub fn with_headset(mut self, h: bool) -> Self {
        self.headset = h;
        self
    }
}

/// Discovery and connection timing (DESIGN §13.2): exponential time to discover,
/// connect 1-3 s; the handshake (2 RTT) happens in the engines over the radio model.
/// The scanner's intensity and the advertiser's rate scale the mean
/// ([`crate::scan::ScanCost`]).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DiscoveryParams {
    /// Mean time to discover a peer while both apps are in the foreground (µs).
    pub fg_mean_us: u64,
    /// Factor applied when exactly one side of an iPhone pair (or an iPhone scanner)
    /// is in the background.
    pub bg_factor: f64,
    /// Mean for iPhone (background) ↔ iPhone (background); `None` = never.
    pub ios_bg_both_mean_us: Option<u64>,
    /// Connect + HELLO read delay range (µs).
    pub connect_min_us: u64,
    pub connect_max_us: u64,
    /// A peer is discoverable while the pair's PDR is at least this.
    pub min_pdr: f64,
    /// A connect or open succeeds while the pair's PDR is at least this.
    pub connect_min_pdr: f64,
}

impl Default for DiscoveryParams {
    fn default() -> Self {
        DiscoveryParams {
            fg_mean_us: 1_500_000,
            bg_factor: 3.0,
            ios_bg_both_mean_us: None,
            connect_min_us: 1_000_000,
            connect_max_us: 3_000_000,
            min_pdr: 0.5,
            connect_min_pdr: 0.3,
        }
    }
}

/// Everything that defines a world; with the same config (seed included) a run is
/// bit-for-bit reproducible.
#[derive(Debug, Clone)]
pub struct WorldConfig {
    /// Scenario seed: every random stream of the world and every engine RNG derive
    /// from it.
    pub seed: u64,
    pub nodes: Vec<NodeSpec>,
    pub mobility: Mobility,
    pub radio: RadioParams,
    pub discovery: DiscoveryParams,
    /// Engine limits for every node (`None` = each platform's defaults). The node
    /// airtime budget of the radio model is read from here (`NODE_BLE_BUDGET`).
    pub limits: Option<Limits>,
    /// Wall clock (Unix ms) at simulated time zero.
    pub wall_ms: u64,
    /// Radio supervision / discovery / audit tick (µs).
    pub radio_tick_us: u64,
    /// Mobility step (µs).
    pub mobility_tick_us: u64,
    /// Platform sample and link metrics period (µs).
    pub sample_period_us: u64,
    /// Keep every effect of every node (tests, S10); long scenarios may turn it off
    /// and keep only Lab files and counters.
    pub record_effects: bool,
    /// A Lab or node id collision may last this long before it is a violation.
    pub id_collision_grace_us: u64,
    /// A node may stay over its BLE link max this long, while §5.7 has a redundant
    /// link it could close, before it is a violation (`None` = report-only, see
    /// `Audit::check_link_cap`). Over-max episodes come from two initiators reading an
    /// `accepting` HELLO at once; Lab engines have no over-max policy, group engines
    /// close a redundant link at their next 2 s selection round.
    pub link_cap_grace_us: Option<u64>,
}

impl WorldConfig {
    /// A world of `nodes` at fixed positions.
    pub fn new(seed: u64, nodes: Vec<NodeSpec>, mobility: Mobility) -> Self {
        WorldConfig {
            seed,
            nodes,
            mobility,
            radio: RadioParams::default(),
            discovery: DiscoveryParams::default(),
            limits: None,
            // 2026-10-23T09:12:03.120Z, as in the labrun/1 examples.
            wall_ms: 1_792_746_723_120,
            radio_tick_us: 100_000,
            mobility_tick_us: 100_000,
            sample_period_us: 1_000_000,
            record_effects: true,
            id_collision_grace_us: 5_000_000,
            link_cap_grace_us: Some(1_500_000),
        }
    }

    /// `n` foreground iPhones on a line, `spacing_m` apart.
    pub fn iphones_on_line(seed: u64, n: usize, spacing_m: f64) -> Self {
        Self::new(
            seed,
            (0..n)
                .map(|i| {
                    let mut s = NodeSpec::iphone();
                    s.model = format!("iPhone{},{}", 13 + i % 4, 1 + i % 3);
                    s
                })
                .collect(),
            Mobility::line(n, spacing_m),
        )
    }

    /// The limits node `i` runs with.
    pub fn limits_for(&self, i: usize) -> Limits {
        self.limits
            .unwrap_or_else(|| Limits::for_platform(self.nodes[i].platform))
    }
}
