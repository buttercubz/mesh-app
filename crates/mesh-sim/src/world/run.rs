//! The event loop: queue events and engine deadlines in time order, then every touched
//! engine is drained (transmits go to the air model, effects to the platform model).

use mesh_core::{EngineMode, Instant, PlatformEvent};

use super::{Ev, World};

/// The engine thread's wake-up period while the engine runs (§3.4).
pub const ENGINE_TICK_US: u64 = 5_000;

/// Iterations at one instant before the loop is declared stuck (a livelock is a
/// failure, not a hang).
const SPIN_LIMIT: u32 = 200_000;

impl World {
    /// Advances simulated time to `until`.
    pub fn run_until(&mut self, until: Instant) {
        let mut spins = 0u32;
        let mut last = self.now;
        loop {
            self.settle();
            let next_q = self.queue.keys().next().map(|k| k.0);
            let next_t = self
                .nodes
                .iter()
                .filter_map(|n| n.next_timeout)
                .min()
                .map(|t| t.0);
            let next = match (next_q, next_t) {
                (Some(a), Some(b)) => a.min(b),
                (a, b) => match a.or(b) {
                    Some(t) => t,
                    None => break,
                },
            }
            .max(self.now.0);
            if next > until.0 {
                break;
            }
            self.now = Instant(next);
            if self.now == last {
                spins += 1;
                assert!(
                    spins < SPIN_LIMIT,
                    "event loop stuck at {:?}: queue head {:?}, timeouts {:?}",
                    self.now,
                    self.queue.keys().next(),
                    self.nodes
                        .iter()
                        .map(|n| n.next_timeout)
                        .collect::<Vec<_>>()
                );
            } else {
                spins = 0;
                last = self.now;
            }
            if next_q == Some(next) {
                let key = *self.queue.keys().next().expect("queue head");
                let ev = self.queue.remove(&key).expect("queue entry");
                self.stats.events += 1;
                self.dispatch(ev);
            } else {
                for i in 0..self.nodes.len() {
                    if self.nodes[i].next_timeout.is_some_and(|t| t.0 <= next) {
                        let now = self.now;
                        self.stats.timeouts += 1;
                        self.nodes[i].engine.handle_timeout(now);
                        self.dirty.insert(i);
                        self.drain(i);
                    }
                }
            }
        }
        self.now = until.max(self.now);
        self.settle();
    }

    /// Advances by `ms` milliseconds.
    pub fn run_ms(&mut self, ms: u64) {
        let t = self.now.plus_ms(ms);
        self.run_until(t);
    }

    /// Advances by `s` seconds.
    pub fn run_s(&mut self, s: u64) {
        self.run_ms(s * 1_000);
    }

    /// Drains every engine touched since the last settle.
    pub(crate) fn settle(&mut self) {
        while let Some(i) = self.dirty.pop_first() {
            self.drain(i);
        }
    }

    /// Drains node `i`: its transmits go on air, its effects to the platform model.
    pub(crate) fn drain(&mut self, i: usize) {
        self.dirty.remove(&i);
        loop {
            let mut progressed = false;
            let now = self.now;
            while let Some(t) = self.nodes[i].engine.poll_transmit(now) {
                progressed = true;
                self.send(i, t);
            }
            while let Some(e) = self.nodes[i].engine.poll_effect() {
                progressed = true;
                self.apply(i, &e);
                if let mesh_core::Effect::LabWrite { line, .. } = &e {
                    self.fold_digest(&[now.0, i as u64], line.as_bytes());
                }
                let keep = self.cfg.record_effects;
                self.nodes[i].record(now, e, keep);
            }
            if !progressed {
                break;
            }
        }
        // The engine thread wakes at its next deadline and at least every 5 ms while
        // running (`park_timeout(min(next_deadline, 5 ms))`, §3.4); each phone's loop
        // runs on its own phase.
        let deadline = self.nodes[i].engine.poll_timeout();
        let tick = (self.nodes[i].engine.mode() != EngineMode::Idle).then(|| {
            let phase = self.nodes[i].tick_phase_us;
            let now = self.now.0;
            let k = now.saturating_sub(phase) / ENGINE_TICK_US + 1;
            Instant((phase + k * ENGINE_TICK_US).max(now + 1))
        });
        self.nodes[i].next_timeout = match (deadline, tick) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        };
    }

    fn dispatch(&mut self, ev: Ev) {
        match ev {
            Ev::Deliver { link, to, bytes } => self.deliver(link, to, &bytes),
            Ev::Pump { link, dir } => self.pump(link, dir),
            Ev::Writable { node, link } => {
                if self.link_index.contains_key(&(node, link.0)) {
                    let now = self.now;
                    self.nodes[node]
                        .engine
                        .handle_event(now, PlatformEvent::Writable { link });
                    self.dirty.insert(node);
                }
            }
            Ev::Teardown { link } => self.teardown(link),
            Ev::Discover { scanner, adv } => self.discover(scanner, adv),
            Ev::HelloRead {
                node,
                disc,
                generation,
            } => self.hello_read(node, disc, generation),
            Ev::Open {
                node,
                disc,
                generation,
                gatt,
            } => self.open(node, disc, generation, gatt),
            Ev::AudioTick { node } => self.audio_tick(node),
            Ev::Ptt { node, down } => self.ptt(node, down),
            Ev::Replay { link, to, bytes } => self.replay_deliver(link, to, &bytes),
            Ev::AwareOpen { a, b } => self.aware_open(a, b),
            Ev::SampleTick => self.sample_tick(),
            Ev::RadioTick => self.radio_tick(),
            Ev::MobilityTick => {
                let dt = self.cfg.mobility_tick_us as f64 / 1e6;
                self.mobility.step(dt);
                let t = self.now.0 + self.cfg.mobility_tick_us;
                self.schedule(t, Ev::MobilityTick);
            }
        }
    }
}
