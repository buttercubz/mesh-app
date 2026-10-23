//! [`SimConvoy`]: the browser simulator's world (DESIGN §11.2).
//!
//! Node 0 is the local phone: its engine receives the page's UI commands and its UI
//! events and labrun/1 lines go back to the page. Nodes 1.. are bots: at creation they
//! form a group (node 1 creates it, the others join from its invite, every session
//! starts and node 1 starts the ride), they go live in PTT and talk on a seeded random
//! schedule with the simulator's speech-like microphone (SimCodec frames on the wire),
//! so the real floor and busy logic run. When the page creates its own group the bots
//! join it from its invite instead.
//!
//! Everything here is deterministic for a given scenario and command sequence; the
//! page's clock only decides how much simulated time each `step` advances.

use std::collections::{BTreeMap, VecDeque};

use mesh_api::UiEvent;
use mesh_core::{Effect, EngineMode, Instant};
use mesh_sim::ride::RideGroup;
use mesh_sim::rng::SimRng;
use mesh_sim::{PresetOptions, World};

use crate::config::ConvoyConfig;

/// The local phone's world index.
pub const LOCAL: usize = 0;
/// Pending UI events kept when the page does not poll (oldest are dropped first).
pub const MAX_PENDING_EVENTS: usize = 4_096;
/// Simulated time between two housekeeping passes (bot schedule, memory trimming).
pub const HOUSEKEEP_US: u64 = 100_000;
/// Simulated time between two trims of the simulator's diagnostic records.
pub const TRIM_US: u64 = 10_000_000;
/// Longest `step` the page may ask for at once (a backgrounded tab resuming must not
/// freeze the page for minutes): 2 s of simulated time.
pub const MAX_STEP_US: u64 = 2_000_000;
/// Bot burst length range (ms).
pub const BOT_BURST_MS: (u64, u64) = (1_500, 4_000);

/// Per-bot state.
#[derive(Debug, Clone)]
pub(crate) struct Bot {
    pub(crate) node: usize,
    /// Live (audio on, PTT mode) in its current session.
    pub(crate) live: bool,
    /// Next go-live attempt (µs) while not live.
    pub(crate) next_live_try: u64,
    /// Next scheduled burst start (µs).
    pub(crate) next_talk: u64,
    /// End of the burst it is scripted to speak (µs), if any.
    pub(crate) talking_until: Option<u64>,
    /// Out of radio range of every other phone (`sim.botAway` or roaming).
    pub(crate) away: bool,
    /// While roaming: when it next drops out (in range) or comes back (away), µs.
    pub(crate) next_roam: u64,
}

/// Where the bots currently ride.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum BotHome {
    /// A group session in this group.
    Group([u8; 16]),
    /// Lab mode with the page's code (they follow the local controller).
    Lab(u16),
}

/// The browser simulator world.
pub struct SimConvoy {
    pub(crate) world: World,
    pub(crate) cfg: ConvoyConfig,
    pub(crate) bots: Vec<Bot>,
    /// The group the bots formed at creation (the sim invite of `scanInviteQr`).
    pub(crate) bot_group: RideGroup,
    pub(crate) home: BotHome,
    pub(crate) rng: SimRng,
    pub(crate) events: VecDeque<String>,
    pub(crate) dropped_events: u64,
    pub(crate) lab_cursor: BTreeMap<String, usize>,
    /// The page holds the PTT button.
    pub(crate) local_ptt: bool,
    /// The page simulates speech for VOX (`sim.speak`).
    pub(crate) local_speak: bool,
    /// The local microphone is inside an open speech span.
    pub(crate) local_mic_open: bool,
    pub(crate) next_housekeep: u64,
    pub(crate) next_trim: u64,
}

impl SimConvoy {
    /// Builds the world of `cfg` and forms the bots' ride.
    pub fn new(cfg: ConvoyConfig) -> SimConvoy {
        let mut wc = cfg.preset.config(
            cfg.seed,
            PresetOptions {
                nodes: Some(cfg.nodes),
                loss: cfg.loss,
            },
        );
        wc.wall_ms = cfg.wall_ms;
        // Effects are kept so node 0's UI events can be read back; `housekeep` empties
        // every node's log on each pass.
        wc.record_effects = true;
        // Group mode closes links over the BLE max at its next selection round.
        wc.link_cap_grace_us = Some(3_500_000);
        let mut world = World::new(wc);
        for &(a, b) in &cfg.shadow {
            world.block(a, b, true);
        }
        // Every microphone is silent outside scripted speech spans.
        for i in 0..world.nodes.len() {
            world.mute_mic(i);
        }
        let leader = 1;
        let others: Vec<usize> = (2..cfg.nodes).collect();
        let bot_group = world.form_group(leader, &others);
        let mut c = SimConvoy {
            world,
            bots: (1..cfg.nodes)
                .map(|node| Bot {
                    node,
                    live: false,
                    next_live_try: 0,
                    next_talk: 0,
                    talking_until: None,
                    away: false,
                    next_roam: 0,
                })
                .collect(),
            home: BotHome::Group(bot_group.id),
            bot_group,
            rng: SimRng::new(cfg.seed, 0x3A5_B075),
            events: VecDeque::new(),
            dropped_events: 0,
            lab_cursor: BTreeMap::new(),
            local_ptt: false,
            local_speak: false,
            local_mic_open: false,
            next_housekeep: 0,
            next_trim: TRIM_US,
            cfg,
        };
        let id = c.bot_group.id;
        c.bots_enter_group(id);
        for b in 0..c.bots.len() {
            c.bots[b].next_talk = c.draw_pause();
            if c.cfg.roam {
                c.bots[b].next_roam = c.draw_roam_stay();
            }
        }
        c.housekeep();
        c
    }

    /// The scenario this world was built from.
    pub fn config(&self) -> &ConvoyConfig {
        &self.cfg
    }

    /// The underlying simulator world (tests and diagnostics).
    pub fn world(&self) -> &World {
        &self.world
    }

    /// Simulated time (µs since world creation).
    pub fn now_us(&self) -> u64 {
        self.world.now.0
    }

    /// The bots' world indices.
    pub fn bot_nodes(&self) -> Vec<usize> {
        self.bots.iter().map(|b| b.node).collect()
    }

    /// Advances simulated time by `dt_us` (capped at [`MAX_STEP_US`]) in housekeeping
    /// slices, so bots act on their schedule inside long steps.
    pub fn step(&mut self, dt_us: u64) {
        let end = self.world.now.0.saturating_add(dt_us.min(MAX_STEP_US));
        while self.world.now.0 < end {
            let until = end.min(self.next_housekeep.max(self.world.now.0 + 1));
            self.world.run_until(Instant(until));
            if self.world.now.0 >= self.next_housekeep {
                self.housekeep();
            }
        }
        self.collect();
    }

    /// Takes the UI events of the local phone gathered since the last call, as JSON.
    pub fn poll_events(&mut self) -> Vec<String> {
        self.collect();
        self.events.drain(..).collect()
    }

    /// UI events dropped because the page did not poll.
    pub fn dropped_events(&self) -> u64 {
        self.dropped_events
    }

    /// labrun/1 lines the local phone wrote since the last call, in order. A
    /// `{"t":"run",…}` line opens the next run's file.
    pub fn poll_lab(&mut self) -> Vec<String> {
        let node = &self.world.nodes[LOCAL];
        let runs = node.lab_runs();
        self.lab_cursor.retain(|r, _| runs.contains(r));
        let mut out = Vec::new();
        for run in runs {
            let lines = node.lab_lines(&run);
            let cur = self.lab_cursor.entry(run).or_insert(0);
            if *cur < lines.len() {
                out.extend(lines[*cur..].iter().cloned());
                *cur = lines.len();
            }
        }
        out
    }

    /// The local engine's mode.
    pub fn local_mode(&self) -> EngineMode {
        self.world.nodes[LOCAL].engine.mode()
    }

    /// Moves node 0's UI events into the pending queue and drops every other recorded
    /// effect (the world keeps per-node logs only for scenarios).
    pub(crate) fn collect(&mut self) {
        let fx = core::mem::take(&mut self.world.nodes[LOCAL].effects);
        for (_, e) in fx {
            if let Effect::Ui(ev) = e {
                self.push_event(&ev);
            }
        }
        for n in self.world.nodes.iter_mut().skip(1) {
            n.effects.clear();
        }
        for n in &mut self.world.nodes {
            n.commands.clear();
        }
    }

    fn push_event(&mut self, ev: &UiEvent) {
        let Ok(json) = serde_json::to_string(ev) else {
            return;
        };
        if self.events.len() >= MAX_PENDING_EVENTS {
            self.events.pop_front();
            self.dropped_events += 1;
        }
        self.events.push_back(json);
    }

    /// One random pause before a bot's next burst (µs from now).
    pub(crate) fn draw_pause(&mut self) -> u64 {
        let mean_us = self.cfg.bot_talk_mean_s * 1e6;
        let p = self.rng.exp(mean_us).clamp(1_000_000.0, 10.0 * mean_us);
        self.world.now.0 + p as u64
    }
}
