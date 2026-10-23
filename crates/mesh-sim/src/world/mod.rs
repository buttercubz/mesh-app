//! The discrete-event world (DESIGN §13.2): N real [`Engine`]s, a µs clock, one
//! seeded event queue, the radio / discovery / role / mobility / audio models and the
//! global assertions. Everything a run does derives from `WorldConfig::seed`.
//!
//! Split: `run` (event loop, engine draining), `air` (link transmit timeline and link
//! lifecycle), `platform` (radio and audio commands, discovery, platform samples).

mod air;
mod aware;
mod faults;
mod platform;
mod run;
mod streams;

pub use aware::AwareState;
pub use faults::FaultStats;
pub use streams::LinkStreams;

use std::collections::{BTreeMap, BTreeSet};

use mesh_api::{CommandResult, UiCommand};
use mesh_core::{Engine, EngineConfig, Instant, LinkId, PlatformEvent, SessionStart, StoredGroup};

use crate::audit::Audit;
use crate::config::{AppState, WorldConfig};
use crate::link::SimLink;
use crate::mobility::Mobility;
use crate::node::{CommandRecord, SimNode};
use crate::radio::RadioModel;
use crate::rng::{SimRng, derive_seed};
use crate::tap::{Tap, TapHandle, new_tap};

/// `disc_id` under which every scanner sees node `i` advertise.
pub const DISC_BASE: u32 = 1_000;

/// The disc id of node `i`.
pub const fn disc_of(i: usize) -> u32 {
    DISC_BASE + i as u32
}

/// L2CAP PSM node `i` publishes.
pub const fn psm_of(i: usize) -> u16 {
    0x0080 + i as u16
}

/// A platform event owned by the queue.
#[derive(Debug, Clone)]
pub(crate) enum Ev {
    Deliver {
        link: usize,
        to: usize,
        bytes: Vec<u8>,
    },
    Pump {
        link: usize,
        dir: usize,
    },
    Writable {
        node: usize,
        link: LinkId,
    },
    Teardown {
        link: usize,
    },
    Discover {
        scanner: usize,
        adv: usize,
    },
    HelloRead {
        node: usize,
        disc: u32,
        generation: u64,
    },
    Open {
        node: usize,
        disc: u32,
        generation: u64,
        gatt: bool,
    },
    AudioTick {
        node: usize,
    },
    /// A replayed unit (fault injection).
    Replay {
        link: usize,
        to: usize,
        bytes: Vec<u8>,
    },
    /// Scripted PTT press / release.
    Ptt {
        node: usize,
        down: bool,
    },
    /// A Wi-Fi Aware data path is ready (pair `a < b`).
    AwareOpen {
        a: usize,
        b: usize,
    },
    SampleTick,
    RadioTick,
    MobilityTick,
}

/// Counters of a world run.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct WorldStats {
    pub events: u64,
    pub timeouts: u64,
    pub deliveries: u64,
    pub links_created: u64,
    pub links_dropped: u64,
    pub discoveries: u64,
    pub connects: u64,
    /// BLE links dropped because scanning starved their connection events.
    pub starved_drops: u64,
}

/// The simulated world.
pub struct World {
    pub cfg: WorldConfig,
    pub now: Instant,
    pub nodes: Vec<SimNode>,
    pub links: Vec<SimLink>,
    pub mobility: Mobility,
    pub radio: RadioModel,
    pub audit: Audit,
    pub stats: WorldStats,
    pub(crate) queue: BTreeMap<(u64, u64), Ev>,
    pub(crate) seq: u64,
    pub(crate) next_link_id: u32,
    pub(crate) link_index: BTreeMap<(usize, u32), usize>,
    pub(crate) rng: SimRng,
    pub(crate) blocked: BTreeSet<(usize, usize)>,
    pub(crate) discover_scheduled: BTreeSet<(usize, usize)>,
    pub(crate) connect_generation: u64,
    pub(crate) dirty: BTreeSet<usize>,
    pub(crate) digest: u64,
    pub(crate) talk_log: Vec<crate::talk::TalkSpan>,
    pub faults: FaultStats,
    /// Native Wi-Fi Aware state (simulated data paths).
    pub aware: AwareState,
    pub(crate) corrupt: BTreeMap<(usize, usize), u32>,
    pub(crate) capture: BTreeSet<usize>,
    pub(crate) captured: Vec<(usize, usize, u64, Vec<u8>)>,
}

impl World {
    /// Builds the world: one engine per node (in Idle), the periodic ticks scheduled.
    pub fn new(cfg: WorldConfig) -> World {
        let radio = RadioModel::new(cfg.radio, cfg.seed);
        let mobility = cfg.mobility.clone();
        let nodes = (0..cfg.nodes.len())
            .map(|i| {
                let (engine, tap) = Self::build_engine(&cfg, i, 0, Instant::ZERO, Vec::new());
                SimNode::new(i, cfg.nodes[i].clone(), engine, tap, cfg.seed)
            })
            .collect();
        let mut w = World {
            now: Instant::ZERO,
            nodes,
            links: Vec::new(),
            mobility,
            radio,
            audit: Audit::new(),
            stats: WorldStats::default(),
            queue: BTreeMap::new(),
            seq: 0,
            next_link_id: 1,
            link_index: BTreeMap::new(),
            rng: SimRng::new(cfg.seed, 0x3011_D000),
            blocked: BTreeSet::new(),
            discover_scheduled: BTreeSet::new(),
            connect_generation: 0,
            dirty: BTreeSet::new(),
            digest: 0xCBF2_9CE4_8422_2325,
            talk_log: Vec::new(),
            faults: FaultStats::default(),
            aware: AwareState::default(),
            corrupt: BTreeMap::new(),
            capture: BTreeSet::new(),
            captured: Vec::new(),
            cfg,
        };
        w.schedule(w.cfg.radio_tick_us, Ev::RadioTick);
        w.schedule(w.cfg.sample_period_us, Ev::SampleTick);
        if w.mobility.is_dynamic() {
            w.schedule(w.cfg.mobility_tick_us, Ev::MobilityTick);
        }
        for i in 0..w.nodes.len() {
            w.dirty.insert(i);
        }
        w.settle();
        w
    }

    /// Builds node `i`'s engine (generation `generation`, i.e. after that many app
    /// restarts) with the groups of its secure storage, and registers the receive tap
    /// ahead of every module.
    fn build_engine(
        cfg: &WorldConfig,
        i: usize,
        generation: u32,
        now: Instant,
        stored_groups: Vec<StoredGroup>,
    ) -> (Engine, TapHandle) {
        let spec = &cfg.nodes[i];
        let tag = 0xE000_0000u64 + ((generation as u64) << 16) + i as u64;
        let mut ec = EngineConfig::new(spec.platform, derive_seed(cfg.seed, tag));
        ec.limits = cfg.limits_for(i);
        ec.wall_ms = cfg.wall_ms + now.0 / 1_000;
        ec.device.model = spec.model.clone();
        ec.device.os = match spec.platform {
            mesh_core::Platform::Ios => "iOS 27.0".into(),
            mesh_core::Platform::Android => "Android 16".into(),
            mesh_core::Platform::WebSim => "sim".into(),
        };
        ec.device.app = "0.1.0 (sim)".into();
        ec.stored_groups = stored_groups;
        let mut engine = Engine::new(ec, now);
        let tap = new_tap();
        engine.register_module_first(Box::new(Tap { state: tap.clone() }));
        (engine, tap)
    }

    /// Queues `ev` at absolute time `t_us`.
    pub(crate) fn schedule(&mut self, t_us: u64, ev: Ev) {
        self.seq += 1;
        self.queue.insert((t_us.max(self.now.0), self.seq), ev);
    }

    /// Starts node `i` in Lab mode with `code`; the platform publishes its L2CAP PSM.
    pub fn start_lab(&mut self, i: usize, code: u16) {
        self.start_lab_psm(i, code, psm_of(i));
    }

    /// Starts node `i` in Lab mode with `code` and the given PSM (0 = GATT only, as a
    /// phone without L2CAP publishing).
    pub fn start_lab_psm(&mut self, i: usize, code: u16, psm: u16) {
        let now = self.now;
        let n = &mut self.nodes[i];
        n.engine
            .start(now, SessionStart::Lab { lab_code: code })
            .expect("lab start");
        n.psm = psm;
        n.engine
            .handle_event(now, PlatformEvent::L2capPublished { psm: n.psm });
        self.dirty.insert(i);
        self.settle();
    }

    /// Starts every node in Lab mode with `code`.
    pub fn start_lab_all(&mut self, code: u16) {
        for i in 0..self.nodes.len() {
            self.start_lab(i, code);
        }
    }

    /// Runs a UI command on node `i` (recorded with the effects it produced).
    pub fn command(&mut self, i: usize, cmd: UiCommand) -> CommandResult {
        let now = self.now;
        // Effects of earlier work are drained first so the range is exact.
        self.settle();
        let start = self.nodes[i].effects.len();
        let r = self.nodes[i].engine.handle_command(now, cmd.clone());
        self.dirty.insert(i);
        self.drain(i);
        let end = self.nodes[i].effects.len();
        self.nodes[i].commands.push(CommandRecord {
            at: now,
            cmd,
            result: r.clone(),
            start,
            end,
        });
        self.settle();
        r
    }

    /// Delivers a platform event to node `i` now.
    pub fn event(&mut self, i: usize, ev: PlatformEvent<'_>) {
        let now = self.now;
        self.nodes[i].engine.handle_event(now, ev);
        self.dirty.insert(i);
        self.settle();
    }

    /// Runs `f` on node `i`'s engine (scenario instrumentation, e.g. forcing a Lab id)
    /// and drains whatever it produced.
    pub fn with_engine<R>(&mut self, i: usize, f: impl FnOnce(&mut Engine, Instant) -> R) -> R {
        let now = self.now;
        let r = f(&mut self.nodes[i].engine, now);
        self.dirty.insert(i);
        self.settle();
        r
    }

    /// Simulated app restart (S8): node `i` gets a new engine (new RNG stream, new
    /// `sess`) and the OS tears its links down; peers see a supervision timeout.
    pub fn restart_node(&mut self, i: usize) {
        let now = self.now;
        let lis: Vec<usize> = (0..self.links.len())
            .filter(|l| self.links[*l].up && self.links[*l].peer(i).is_some())
            .collect();
        for li in lis {
            self.drop_link_quiet(li, i);
        }
        let generation = self.nodes[i].generation + 1;
        let stored = self.stored_groups(i);
        let (engine, tap) = Self::build_engine(&self.cfg, i, generation, now, stored);
        let n = &mut self.nodes[i];
        n.engine = engine;
        n.tap = tap;
        n.snapshot = None;
        n.generation = generation;
        n.hello = None;
        n.discovery = (mesh_core::effects::DiscoveryMode::Off, None);
        n.pending.clear();
        n.audio.session = None;
        n.audio.capture = false;
        n.next_timeout = None;
        self.dirty.insert(i);
        self.settle();
    }

    /// The groups in node `i`'s secure storage (what the runtime loads at app start).
    pub fn stored_groups(&self, i: usize) -> Vec<StoredGroup> {
        self.nodes[i]
            .store
            .iter()
            .filter_map(|(k, v)| {
                let id = mesh_core::group::store::parse_hex(k.strip_prefix("group/")?)?;
                Some(StoredGroup {
                    group_id: id,
                    blob: v.clone(),
                })
            })
            .collect()
    }

    /// Blocks or unblocks the radio path between `a` and `b` (shielding, the T0b/T3
    /// "forced out of range" variants): PDR 0 while blocked.
    pub fn block(&mut self, a: usize, b: usize, blocked: bool) {
        let k = (a.min(b), a.max(b));
        if blocked {
            self.blocked.insert(k);
        } else {
            self.blocked.remove(&k);
        }
    }

    /// `true` when the pair is blocked.
    pub fn is_blocked(&self, a: usize, b: usize) -> bool {
        self.blocked.contains(&(a.min(b), a.max(b)))
    }

    /// Sends node `i` to the foreground or background.
    pub fn set_app_state(&mut self, i: usize, s: AppState) {
        self.nodes[i].spec.app = s;
    }

    /// Distance between two nodes (m).
    pub fn distance(&self, a: usize, b: usize) -> f64 {
        self.mobility.distance(a, b)
    }

    /// Current BLE PDR between two nodes (0 when blocked).
    pub fn ble_pdr(&self, a: usize, b: usize) -> f64 {
        if self.is_blocked(a, b) {
            return 0.0;
        }
        let (ma, mb) = (self.nodes[a].spec.mount, self.nodes[b].spec.mount);
        self.radio
            .ble_pdr(a, b, ma, mb, self.distance(a, b), self.now.0)
    }

    /// Indices of the links currently up between `a` and `b`.
    pub fn links_between(&self, a: usize, b: usize) -> Vec<usize> {
        (0..self.links.len())
            .filter(|l| self.links[*l].up && self.links[*l].joins(a, b))
            .collect()
    }

    /// Indices of every link currently up.
    pub fn up_links(&self) -> Vec<usize> {
        (0..self.links.len())
            .filter(|l| self.links[*l].up)
            .collect()
    }

    /// Connected components of the up-link graph (sorted node lists).
    pub fn components(&self) -> Vec<Vec<usize>> {
        let n = self.nodes.len();
        let mut comp: Vec<usize> = (0..n).collect();
        fn find(c: &mut [usize], x: usize) -> usize {
            let mut r = x;
            while c[r] != r {
                r = c[r];
            }
            c[x] = r;
            r
        }
        for l in self.links.iter().filter(|l| l.up) {
            let a = find(&mut comp, l.ends[0].node);
            let b = find(&mut comp, l.ends[1].node);
            if a != b {
                comp[a.max(b)] = a.min(b);
            }
        }
        let mut groups: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
        for i in 0..n {
            let r = find(&mut comp, i);
            groups.entry(r).or_default().push(i);
        }
        groups.into_values().collect()
    }

    /// Order-sensitive digest of everything delivered and written so far: two runs
    /// with the same seed produce the same digest.
    pub fn digest(&self) -> u64 {
        self.digest
    }

    pub(crate) fn fold_digest(&mut self, words: &[u64], bytes: &[u8]) {
        let mut h = self.digest;
        for w in words {
            for b in w.to_le_bytes() {
                h = (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01B3);
            }
        }
        for b in bytes {
            h = (h ^ u64::from(*b)).wrapping_mul(0x0100_0000_01B3);
        }
        self.digest = h;
    }
}
