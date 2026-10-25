//! The simulated platform layer: radio commands (HELLO, discovery, connect, open,
//! abort, close), the discovery model with the §9.2/§9.4 visibility rules, audio
//! sessions and the 10 ms RT clock, 1 Hz platform samples and link metrics, and the
//! radio tick (supervision drops, discovery scheduling, global assertions).

use std::collections::BTreeSet;

use mesh_api::AudioRouteKind;
use mesh_core::effects::{AudioCommand, DiscoveryMode, Effect, OpenVia, RadioCommand};
use mesh_core::{
    AdvData, AnchorState, AudioCounters, AudioIoState, AudioRouteInfo, BLOCK_SAMPLES, Discovery,
    EngineMode, GpsFix, Instant, LinkDownReason, LinkKind, Platform, PlatformEvent, PlatformSample,
    Thermal, Transport,
};

use super::{DISC_BASE, Ev, World, disc_of};
use crate::audit::ViolationKind;
use crate::config::AppState;
use crate::mobility::to_lat_lon;
use crate::node::PendingConnect;
use crate::onset::CAPTURE_IO_US;
use crate::scan::ScanCost;
use mesh_core::media::RENDER_BUFFER_US;

/// 10 ms RT block period.
const BLOCK_US: u64 = 10_000;
/// How often an outstanding connect to an out-of-reach target checks whether the
/// target is back (the controller's initiator scan catching its next advertisement).
const CONNECT_POLL_US: u64 = 250_000;

impl World {
    /// Applies one effect of node `i` to the platform model.
    pub(crate) fn apply(&mut self, i: usize, e: &Effect) {
        match e {
            Effect::Radio(cmd) => self.apply_radio(i, cmd),
            Effect::Audio(cmd) => self.apply_audio(i, *cmd),
            _ => {}
        }
    }

    fn apply_radio(&mut self, i: usize, cmd: &RadioCommand) {
        let now = self.now.0;
        match cmd {
            RadioCommand::SetHello { bytes } => self.nodes[i].hello = Some(*bytes),
            RadioCommand::SetDiscovery { mode, adv, scan } => {
                self.nodes[i].discovery = (*mode, adv.clone());
                self.nodes[i].scan = *scan;
            }
            RadioCommand::Connect { disc_id } => {
                let target = disc_id.wrapping_sub(DISC_BASE) as usize;
                if target >= self.nodes.len() || target == i {
                    return;
                }
                self.connect_generation += 1;
                let generation = self.connect_generation;
                self.nodes[i].pending.insert(
                    *disc_id,
                    PendingConnect {
                        target,
                        generation,
                        hello_read: false,
                        waiting: false,
                    },
                );
                self.stats.connects += 1;
                let d = self.cfg.discovery;
                let delay = self.rng.range_u64(d.connect_min_us, d.connect_max_us);
                self.schedule(
                    now + delay,
                    Ev::HelloRead {
                        node: i,
                        disc: *disc_id,
                        generation,
                    },
                );
            }
            RadioCommand::OpenLink { disc_id, via } => {
                let Some(p) = self.nodes[i].pending.get(disc_id).copied() else {
                    return;
                };
                let gatt = matches!(via, OpenVia::Gatt);
                let t = now + self.cfg.radio.open_delay_us;
                self.schedule(
                    t,
                    Ev::Open {
                        node: i,
                        disc: *disc_id,
                        generation: p.generation,
                        gatt,
                    },
                );
            }
            RadioCommand::Abort { disc_id, .. } => {
                self.nodes[i].pending.remove(disc_id);
            }
            RadioCommand::Close { link, .. } => self.close_from(i, *link),
            RadioCommand::AwareUpgrade { peer, op, .. } => self.aware_upgrade(i, *peer, *op),
            // Connection priority, reconnect arming (the T3 variants; §13.2 models
            // the foreground discovery path) and the v2 tunnel are recorded with the
            // effects; the radio model does not act on them.
            RadioCommand::SetLinkParams { .. }
            | RadioCommand::ArmReconnect { .. }
            | RadioCommand::TunnelOpen { .. } => {}
        }
    }

    fn route_of(&self, i: usize) -> AudioRouteInfo {
        if self.nodes[i].spec.headset {
            AudioRouteInfo {
                kind: AudioRouteKind::HfpWideband,
                input_rate: 16_000,
                output: AudioRouteKind::HfpWideband,
                headset_present: true,
            }
        } else {
            AudioRouteInfo {
                kind: AudioRouteKind::BuiltinSpeaker,
                input_rate: 48_000,
                output: AudioRouteKind::BuiltinSpeaker,
                headset_present: false,
            }
        }
    }

    fn apply_audio(&mut self, i: usize, cmd: AudioCommand) {
        let now = self.now;
        match cmd {
            AudioCommand::OpenSession { profile } => {
                let route = self.route_of(i);
                let a = &mut self.nodes[i].audio;
                a.session = Some(profile);
                a.capture_blocks = 0;
                a.render_blocks = 0;
                a.last_sample = (0, 0, now);
                a.playout_peak = 0;
                let start_tick = !a.ticking;
                a.ticking = true;
                let e = &mut self.nodes[i].engine;
                e.handle_event(now, PlatformEvent::AudioRoute(route));
                e.handle_event(now, PlatformEvent::AudioIo(AudioIoState::Running));
                self.dirty.insert(i);
                if start_tick {
                    self.schedule(now.0 + BLOCK_US, Ev::AudioTick { node: i });
                }
            }
            AudioCommand::StartCapture => {
                if self.nodes[i].audio.session.is_none() {
                    self.audit.report(
                        now,
                        ViolationKind::CaptureWithoutSession,
                        format!("node {i}: StartCapture without an open audio session"),
                    );
                } else {
                    self.nodes[i].audio.capture = true;
                }
            }
            AudioCommand::StopCapture => self.nodes[i].audio.capture = false,
            AudioCommand::CloseSession => {
                let a = &mut self.nodes[i].audio;
                a.session = None;
                a.capture = false;
                self.nodes[i]
                    .engine
                    .handle_event(now, PlatformEvent::AudioIo(AudioIoState::Stopped));
                self.dirty.insert(i);
            }
            AudioCommand::SetRoutePreference(_) | AudioCommand::OutputOnlyFallback(_) => {}
        }
    }

    /// One 10 ms RT block: capture (when the sink is connected) and playout.
    pub(crate) fn audio_tick(&mut self, i: usize) {
        let now = self.now;
        if self.nodes[i].audio.session.is_none() {
            self.nodes[i].audio.ticking = false;
            return;
        }
        let n = &mut self.nodes[i];
        if n.audio.capture {
            let mut block = [0i16; BLOCK_SAMPLES];
            n.audio.fill_mic(now.0, &mut block);
            if let Some(d) = n.audio.mouth.as_mut() {
                d.push_block(now.0 as f64 - CAPTURE_IO_US, &block);
            }
            n.engine.push_capture(now, &block);
            n.audio.capture_blocks += 1;
        }
        let mut out = [0i16; BLOCK_SAMPLES];
        n.engine.pull_playout(now, &mut out);
        if let Some(d) = n.audio.ear.as_mut() {
            d.push_block((now.0 + RENDER_BUFFER_US) as f64, &out);
        }
        n.audio.render_blocks += 1;
        let peak = out.iter().map(|s| s.saturating_abs()).max().unwrap_or(0);
        n.audio.playout_peak = n.audio.playout_peak.max(peak);
        self.dirty.insert(i);
        self.schedule(now.0 + BLOCK_US, Ev::AudioTick { node: i });
    }

    /// The advertising data scanner `x` sees from advertiser `y`, if it can see it
    /// (§9.4: a backgrounded iPhone advertises only in the overflow area, which only
    /// iPhones see; Android's overflow-bitmask detection is off by default, §9.2).
    pub(crate) fn adv_seen(&self, x: usize, y: usize) -> Option<AdvData> {
        let (sx, sy) = (&self.nodes[x], &self.nodes[y]);
        if sx.discovery.0 == DiscoveryMode::Off || sy.discovery.0 == DiscoveryMode::Off {
            return None;
        }
        let adv = sy.discovery.1.as_ref()?;
        match (sy.spec.platform, sy.spec.app) {
            (Platform::Android, _) => Some(AdvData::ServiceData(adv.service_data.to_vec())),
            (_, AppState::Foreground) => Some(AdvData::LocalName(adv.local_name.clone())),
            (_, AppState::Background) if sx.spec.platform == Platform::Ios => {
                Some(AdvData::UuidOnly)
            }
            _ => None,
        }
    }

    /// Mean time (µs) for `x` to discover `y`, `None` when it never does.
    pub(crate) fn discovery_mean_us(&self, x: usize, y: usize) -> Option<f64> {
        let d = self.cfg.discovery;
        let (sx, sy) = (&self.nodes[x].spec, &self.nodes[y].spec);
        let ios_bg = |s: &crate::config::NodeSpec| {
            s.platform == Platform::Ios && s.app == AppState::Background
        };
        let mut mean = if ios_bg(sx) && ios_bg(sy) {
            d.ios_bg_both_mean_us? as f64
        } else if ios_bg(sx) || ios_bg(sy) {
            d.fg_mean_us as f64 * d.bg_factor
        } else {
            d.fg_mean_us as f64
        };
        // The scanner's duty and the advertiser's rate (§5.7 duty cycle, [`ScanCost`]).
        let cost = &self.cfg.radio.scan;
        mean *= cost.discovery_factor(sx.platform, self.nodes[x].scan);
        if sy.platform == Platform::Android && self.nodes[y].discovery.0 == DiscoveryMode::Stable {
            mean *= cost.android_stable_adv_discovery;
        }
        Some(mean)
    }

    /// Radio occupancy of node `i` by scanning and advertising ([`ScanCost`]).
    pub fn scan_occupancy(&self, i: usize) -> f64 {
        let n = &self.nodes[i];
        self.cfg
            .radio
            .scan
            .occupancy(n.spec.platform, n.spec.app, n.discovery.0, n.scan)
    }

    /// Connection-event miss weight of a BLE link between `a` and `b`.
    pub fn link_miss(&self, a: usize, b: usize) -> f64 {
        ScanCost::miss(self.scan_occupancy(a), self.scan_occupancy(b))
    }

    fn discoverable(&self, x: usize, y: usize) -> bool {
        self.adv_seen(x, y).is_some() && self.ble_pdr(x, y) >= self.cfg.discovery.min_pdr
    }

    /// Schedules discoveries for every visible pair in range that has none pending.
    fn schedule_discoveries(&mut self) {
        let n = self.nodes.len();
        for x in 0..n {
            for y in 0..n {
                if x == y || self.discover_scheduled.contains(&(x, y)) || !self.discoverable(x, y) {
                    continue;
                }
                let Some(mean) = self.discovery_mean_us(x, y) else {
                    continue;
                };
                let dt = self.rng.exp(mean).max(1.0) as u64;
                self.discover_scheduled.insert((x, y));
                let t = self.now.0 + dt;
                self.schedule(t, Ev::Discover { scanner: x, adv: y });
            }
        }
    }

    pub(crate) fn discover(&mut self, x: usize, y: usize) {
        self.discover_scheduled.remove(&(x, y));
        if !self.discoverable(x, y) {
            return;
        }
        let Some(adv) = self.adv_seen(x, y) else {
            return;
        };
        let rssi = self.radio.rssi(x, y, self.distance(x, y), self.now.0);
        let d = Discovery {
            disc_id: disc_of(y),
            transport: Transport::Ble,
            rssi: Some(rssi),
            adv,
            platform_hint: None,
        };
        self.stats.discoveries += 1;
        let now = self.now;
        self.nodes[x]
            .engine
            .handle_event(now, PlatformEvent::Discovered(d));
        self.dirty.insert(x);
    }

    fn sim_ble_links(&self, i: usize) -> usize {
        self.links
            .iter()
            .filter(|l| l.up && l.kind.is_ble() && l.peer(i).is_some())
            .count()
    }

    /// The connect reached the peer: its HELLO is read (or the OS limit refuses).
    pub(crate) fn hello_read(&mut self, i: usize, disc: u32, generation: u64) {
        let Some(p) = self.nodes[i].pending.get(&disc).copied() else {
            return;
        };
        if p.generation != generation {
            return;
        }
        let t = p.target;
        let now = self.now;
        if let Some(limit) = self.cfg.radio.os_link_limit
            && (self.sim_ble_links(i) >= limit || self.sim_ble_links(t) >= limit)
        {
            self.nodes[i].pending.remove(&disc);
            self.nodes[i].engine.handle_event(
                now,
                PlatformEvent::ConnectFailed {
                    disc_id: disc,
                    reason: LinkDownReason::LimitReached,
                },
            );
            self.dirty.insert(i);
            return;
        }
        let reachable = self.ble_pdr(i, t) >= self.cfg.discovery.connect_min_pdr
            && self.nodes[t].discovery.0 != DiscoveryMode::Off;
        // Out of reach: the connect stays outstanding (iOS `connect` never times out by
        // itself; the engine aborts it) and completes if the target comes back in reach
        // first, after the fastest connect + HELLO read (the link-layer connection,
        // service discovery and the read start over).
        let hello = self.nodes[t].hello.filter(|_| reachable);
        let Some(pp) = self.nodes[i].pending.get_mut(&disc) else {
            return;
        };
        let (next, waiting) = match hello {
            None => (Some(now.0 + CONNECT_POLL_US), true),
            Some(_) if pp.waiting => (Some(now.0 + self.cfg.discovery.connect_min_us), false),
            Some(_) => (None, false),
        };
        pp.waiting = waiting;
        if let Some(at) = next {
            self.schedule(
                at,
                Ev::HelloRead {
                    node: i,
                    disc,
                    generation,
                },
            );
            return;
        }
        let Some(hello) = hello else {
            return;
        };
        if let Some(pp) = self.nodes[i].pending.get_mut(&disc) {
            pp.hello_read = true;
        }
        self.nodes[i].engine.handle_event(
            now,
            PlatformEvent::HelloRead {
                disc_id: disc,
                bytes: &hello,
            },
        );
        self.dirty.insert(i);
    }

    /// The data path opens (L2CAP channel or GATT subscription).
    pub(crate) fn open(&mut self, i: usize, disc: u32, generation: u64, gatt: bool) {
        let Some(p) = self.nodes[i].pending.get(&disc).copied() else {
            return;
        };
        if p.generation != generation || !p.hello_read {
            return;
        }
        self.nodes[i].pending.remove(&disc);
        let t = p.target;
        let now = self.now;
        if self.ble_pdr(i, t) < self.cfg.discovery.connect_min_pdr {
            self.nodes[i].engine.handle_event(
                now,
                PlatformEvent::ConnectFailed {
                    disc_id: disc,
                    reason: LinkDownReason::Timeout,
                },
            );
            self.dirty.insert(i);
            return;
        }
        let kind = if gatt {
            LinkKind::BleGatt
        } else {
            LinkKind::BleL2cap
        };
        self.create_link(i, t, kind, Some(disc));
    }
}

impl World {
    fn sample_for(&mut self, i: usize) -> PlatformSample {
        let now = self.now;
        let pos = self.mobility.position(i);
        let (lat, lon) = to_lat_lon(pos);
        let speed = self.mobility.speed(i);
        let n = &mut self.nodes[i];
        let a = &mut n.audio;
        let (cap0, ren0, t0) = a.last_sample;
        let dt = now.since(t0) as f64 / 1e6;
        let fps = |blocks: u64, prev: u64| (dt > 0.0).then(|| (blocks - prev) as f64 / dt);
        let open = a.session.is_some();
        let audio = AudioCounters {
            capture_fps: if open {
                fps(a.capture_blocks, cap0)
            } else {
                None
            },
            render_fps: if open {
                fps(a.render_blocks, ren0)
            } else {
                None
            },
            xruns: 0,
        };
        a.last_sample = (a.capture_blocks, a.render_blocks, now);
        let background = n.spec.app == AppState::Background;
        PlatformSample {
            wall_ms: Some(self.cfg.wall_ms + now.0 / 1_000),
            gps: Some(GpsFix {
                lat,
                lon,
                speed_mps: Some(speed),
                acc_m: Some(4.8),
                heading_deg: None,
            }),
            battery: Some((0.9 - now.0 as f64 / 3.6e9 * 0.05).max(0.05)),
            charging: Some(false),
            thermal: Some(Thermal::Nominal),
            background,
            locked: Some(background),
            on_call: false,
            audio,
            anchor: AnchorState {
                session: Some(open),
                location: Some(a.session.is_some_and(|p| p.location_anchor)),
                live_activity: None,
            },
        }
    }

    /// 1 Hz: platform samples and link metrics for every running engine.
    pub(crate) fn sample_tick(&mut self) {
        let now = self.now;
        for i in 0..self.nodes.len() {
            if self.nodes[i].engine.mode() == EngineMode::Idle {
                continue;
            }
            let s = self.sample_for(i);
            self.nodes[i]
                .engine
                .handle_event(now, PlatformEvent::Platform(s));
            let links: Vec<(usize, usize)> = (0..self.links.len())
                .filter_map(|li| {
                    let l = &self.links[li];
                    (l.up && l.closing_by.is_none())
                        .then(|| l.ends.iter().position(|e| e.node == i).map(|d| (li, d)))
                        .flatten()
                })
                .collect();
            for (li, d) in links {
                let l = &self.links[li];
                let peer = l.ends[1 - d].node;
                let rssi = self.radio.rssi(i, peer, self.distance(i, peer), now.0);
                let ev = PlatformEvent::Metrics {
                    link: l.ends[d].link,
                    rssi: Some(rssi),
                    att_mtu: l.att_mtu,
                };
                self.nodes[i].engine.handle_event(now, ev);
            }
            self.dirty.insert(i);
        }
        let t = now.0 + self.cfg.sample_period_us;
        self.schedule(t, Ev::SampleTick);
    }

    /// Radio tick: supervision drops, discovery scheduling and global assertions.
    pub(crate) fn radio_tick(&mut self) {
        let now = self.now.0;
        let p = self.cfg.radio;
        for li in 0..self.links.len() {
            if !self.links[li].up {
                continue;
            }
            let (a, b) = (self.links[li].ends[0].node, self.links[li].ends[1].node);
            let pdr = if self.links[li].kind.is_ble() {
                self.ble_pdr(a, b)
            } else {
                1.0
            };
            if pdr < p.drop_below_pdr {
                let since = *self.links[li].below_since.get_or_insert(now);
                if now - since >= p.drop_after_us {
                    self.drop_link(li, [LinkDownReason::Timeout; 2]);
                    continue;
                }
            } else {
                self.links[li].below_since = None;
            }
            // Connection events starved by scanning: a supervision timeout.
            if self.links[li].kind.is_ble() {
                let rate = p.scan.drop_rate(self.link_miss(a, b));
                let dt = self.cfg.radio_tick_us as f64 / 1e6;
                if rate > 0.0 && self.rng.chance(rate * dt) {
                    self.stats.starved_drops += 1;
                    self.drop_link(li, [LinkDownReason::Timeout; 2]);
                }
            }
        }
        self.schedule_discoveries();
        self.check_invariants();
        self.schedule(now + self.cfg.radio_tick_us, Ev::RadioTick);
    }

    /// Lab id uniqueness per connected component and the BLE link max per node.
    pub(crate) fn check_invariants(&mut self) {
        let now: Instant = self.now;
        let mut pairs = Vec::new();
        for comp in self.components() {
            for (k, &a) in comp.iter().enumerate() {
                for &b in &comp[k + 1..] {
                    let (ia, ib) = (self.nodes[a].lab_node_id(), self.nodes[b].lab_node_id());
                    if let (Some(x), Some(y)) = (ia, ib)
                        && x == y
                    {
                        pairs.push((x, a, b));
                    }
                }
            }
        }
        let grace = self.cfg.id_collision_grace_us;
        self.audit.check_ids(now, &pairs, grace);
        let adj = self.ble_adjacency();
        for i in 0..self.nodes.len() {
            let ctx = self.nodes[i].engine.ctx();
            let links = ctx.links.active_count(true);
            let max = ctx.radio.ble_max(&ctx.cfg.limits, now);
            let grace = self.cfg.link_cap_grace_us;
            let closable = links > max && closable_link(&adj, i);
            self.audit
                .check_link_cap(now, i, links, max, closable, grace);
        }
    }

    /// Neighbor sets of the up BLE link graph.
    pub(crate) fn ble_adjacency(&self) -> Vec<BTreeSet<usize>> {
        let mut adj = vec![BTreeSet::new(); self.nodes.len()];
        for l in self.links.iter().filter(|l| l.up && l.kind.is_ble()) {
            let (a, b) = (l.ends[0].node, l.ends[1].node);
            adj[a].insert(b);
            adj[b].insert(a);
        }
        adj
    }
}

/// `true` when node `me` has a link §5.7 may close over the max: to a neighbor that
/// another neighbor is linked to (it appears in that neighbor's NBR), whose removal
/// keeps every node of `me`'s 2-hop graph reachable (the articulation check).
pub(crate) fn closable_link(adj: &[BTreeSet<usize>], me: usize) -> bool {
    let reach = |without: Option<usize>| -> BTreeSet<usize> {
        let mut seen = BTreeSet::from([me]);
        let mut stack = vec![me];
        while let Some(u) = stack.pop() {
            // The 2-hop graph: my links and my neighbors' links.
            let next: Vec<usize> = if u == me {
                adj[me]
                    .iter()
                    .copied()
                    .filter(|v| Some(*v) != without)
                    .collect()
            } else if adj[me].contains(&u) {
                adj[u]
                    .iter()
                    .copied()
                    .filter(|v| !(Some(u) == without && *v == me))
                    .collect()
            } else {
                adj[u]
                    .iter()
                    .copied()
                    .filter(|v| adj[me].contains(v))
                    .collect()
            };
            for v in next {
                if seen.insert(v) {
                    stack.push(v);
                }
            }
        }
        seen
    };
    let base = reach(None);
    adj[me].iter().any(|&n| {
        let redundant = adj[me].iter().any(|&o| o != n && adj[o].contains(&n));
        redundant && reach(Some(n)).is_superset(&base)
    })
}
