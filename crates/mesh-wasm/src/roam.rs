//! Members dropping out and coming back: a bot rides out of radio range of every other
//! phone (`sim.botAway`, or on its own random schedule with the `roam` scenario
//! option) or its app is killed and relaunched (`sim.botRestart`). The real link
//! supervision, rejoin and roster logic of the engines then runs (§5, §9.5).

use mesh_core::EngineMode;

use crate::convoy::{BotHome, SimConvoy};

/// Shortest and longest time a roaming bot stays out of range (µs).
pub const ROAM_AWAY_US: (u64, u64) = (8_000_000, 30_000_000);

impl SimConvoy {
    /// The index in `bots` of world node `node`.
    pub(crate) fn bot_index(&self, node: usize) -> Option<usize> {
        self.bots.iter().position(|b| b.node == node)
    }

    /// Bot `k` leaves (`true`) or comes back into (`false`) radio range of every other
    /// phone. Pairs shadowed by the scenario stay shadowed when it comes back.
    pub(crate) fn set_away(&mut self, k: usize, away: bool) {
        let node = self.bots[k].node;
        if self.bots[k].away == away {
            return;
        }
        for other in 0..self.world.nodes.len() {
            if other == node {
                continue;
            }
            let shadowed = self
                .cfg
                .shadow
                .iter()
                .any(|&(a, b)| (a, b) == (node, other) || (b, a) == (node, other));
            let other_away = self.bot_index(other).is_some_and(|j| self.bots[j].away);
            // A pair stays blocked while either side is away or the scenario shadows it.
            self.world
                .block(node, other, away || shadowed || other_away);
        }
        self.bots[k].away = away;
    }

    /// Bot `k`'s app is killed and relaunched: a new engine (links torn down, peers see
    /// a supervision timeout), then it starts its session again and goes live.
    pub(crate) fn restart_bot(&mut self, k: usize) {
        let node = self.bots[k].node;
        if let Some(spans) = self.world.nodes[node].audio.talk.as_mut() {
            spans.clear();
        }
        if self.bots[k].talking_until.take().is_some() {
            self.world.ptt(node, false);
        }
        let was_running = !matches!(self.world.nodes[node].engine.mode(), EngineMode::Idle);
        self.world.restart_node(node);
        self.world.mute_mic(node);
        self.bots[k].live = false;
        self.bots[k].next_live_try = self.world.now.0;
        if was_running {
            match self.home {
                BotHome::Group(id) => {
                    self.start_group_node(node, id);
                }
                BotHome::Lab(code) => {
                    self.start_lab_node(node, code);
                }
            }
        }
    }

    /// How long a roaming bot stays in range before it drops out (absolute µs).
    pub(crate) fn draw_roam_stay(&mut self) -> u64 {
        let mean_us = self.cfg.roam_mean_s * 1e6;
        let p = self.rng.exp(mean_us).clamp(5_000_000.0, 10.0 * mean_us);
        self.world.now.0 + p as u64
    }

    /// One roaming step of bot `k`: drop out or come back when its time is up. At
    /// most one bot is away at a time, so the ride never falls apart completely.
    pub(crate) fn roam_tick(&mut self, k: usize, now: u64) {
        if now < self.bots[k].next_roam {
            return;
        }
        if self.bots[k].away {
            self.set_away(k, false);
            self.bots[k].next_roam = self.draw_roam_stay();
            return;
        }
        if self.bots.iter().any(|b| b.away) {
            self.bots[k].next_roam = self.draw_roam_stay();
            return;
        }
        self.set_away(k, true);
        let d = self.rng.range_u64(ROAM_AWAY_US.0, ROAM_AWAY_US.1);
        self.bots[k].next_roam = now + d;
    }
}
