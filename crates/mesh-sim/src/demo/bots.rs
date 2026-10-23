//! The bot riders of the Demo world: real engines driven by a small event loop (5 ms
//! engine ticks, 10 ms audio blocks, 1 Hz platform samples, link delivery) and the talk
//! schedule.

use std::collections::{BTreeMap, VecDeque};

use mesh_api::{CommandResult, RideOp, TalkMode, UiCommand, Vehicle};
use mesh_core::effects::{AudioCommand, Effect, RadioCommand};
use mesh_core::group::store;
use mesh_core::{
    AudioIoState, AudioRouteInfo, AudioRouteKind, BLOCK_SAMPLES, Engine, EngineConfig, Instant,
    Platform, PlatformEvent, PlatformSample, SessionStart, StoredGroup, rt_flags,
};

use super::voice::Block;
use super::{BOT_NAMES, DemoError, DemoWorld, End};
use crate::rng::derive_seed;

const TICK_US: u64 = 5_000;
const BLOCK_US: u64 = 10_000;
const SAMPLE_US: u64 = 1_000_000;

/// One bot rider.
pub(crate) struct Bot {
    pub engine: Engine,
    pub name: &'static str,
    pub group_id: Option<[u8; 16]>,
    pub store: BTreeMap<String, Vec<u8>>,
    pub audio_open: bool,
    pub capture: bool,
    pub speech: VecDeque<Block>,
    /// PTT release instant of the current utterance.
    pub release_at: Option<Instant>,
    pub next_tick: Instant,
    pub next_block: Instant,
    pub next_sample: Instant,
}

impl Bot {
    pub fn new(i: usize, platform: Platform, seed: u64, now: Instant, wall_ms: u64) -> Bot {
        let mut cfg = EngineConfig::new(platform, derive_seed(seed, 0xB07_0000 + i as u64));
        cfg.wall_ms = wall_ms;
        cfg.device.model = "Demo bot".into();
        let phase = (i as u64 * 1_250) % TICK_US;
        Bot {
            engine: Engine::new(cfg, now),
            name: BOT_NAMES[i % BOT_NAMES.len()],
            group_id: None,
            store: BTreeMap::new(),
            audio_open: false,
            capture: false,
            speech: VecDeque::new(),
            release_at: None,
            next_tick: now.plus_us(TICK_US + phase),
            next_block: now.plus_us(BLOCK_US + phase),
            next_sample: now,
        }
    }

    fn cmd(&mut self, now: Instant, c: UiCommand) -> CommandResult {
        let r = self.engine.handle_command(now, c);
        self.drain_store();
        r
    }

    /// Keeps the bot's secure storage (persisted groups) from its effects queue; other
    /// effects are handled by the world loop.
    fn drain_store(&mut self) {
        // Effects are drained by `DemoWorld::drain_bot`; group creation and joining
        // happen before the loop runs, so they are drained here.
        while let Some(e) = self.engine.poll_effect() {
            if let Effect::Persist { key, value } = &e {
                match value {
                    Some(v) => {
                        self.store.insert(key.clone(), v.clone());
                    }
                    None => {
                        self.store.remove(key);
                    }
                }
            }
        }
    }

    /// The leader bot creates the demo group; returns its invite URL.
    pub fn create_group(&mut self, now: Instant) -> Result<String, DemoError> {
        let name = self.name.to_string();
        if !self.cmd(now, UiCommand::SetDisplayName { name }).is_ok() {
            return Err(DemoError::Bot("setDisplayName".into()));
        }
        let r = self.cmd(
            now,
            UiCommand::CreateGroup {
                name: "Demo ride".into(),
                vehicle: Vehicle::Moto,
            },
        );
        let CommandResult::GroupId(g) = r else {
            return Err(DemoError::Bot(format!("createGroup: {r:?}")));
        };
        let id = store::parse_hex(&g.group_id).ok_or_else(|| DemoError::Bot("group id".into()))?;
        self.group_id = Some(id);
        let blob = self
            .store
            .get(&store::group_key(&id))
            .ok_or_else(|| DemoError::Bot("group not persisted".into()))?;
        mesh_core::group::invite_url_of_blob(blob).ok_or_else(|| DemoError::Bot("invite".into()))
    }

    /// A bot joins the demo group from the invite URL.
    pub fn join(&mut self, now: Instant, url: &str) -> Result<(), DemoError> {
        let r = self.cmd(
            now,
            UiCommand::JoinGroup {
                url: url.into(),
                display_name: self.name.into(),
            },
        );
        match r {
            CommandResult::GroupId(g) => {
                self.group_id = store::parse_hex(&g.group_id);
                Ok(())
            }
            other => Err(DemoError::Bot(format!("joinGroup: {other:?}"))),
        }
    }

    /// Starts the bot's group session from its stored group.
    pub fn start(&mut self, now: Instant, group_id: [u8; 16]) {
        // The engine was built before the group existed: restart it from storage.
        let mut cfg = self.engine.ctx().cfg.clone();
        cfg.stored_groups = self
            .store
            .iter()
            .filter_map(|(k, v)| {
                let id = store::parse_hex(k.strip_prefix("group/")?)?;
                Some(StoredGroup {
                    group_id: id,
                    blob: v.clone(),
                })
            })
            .collect();
        self.engine = Engine::new(cfg, now);
        let _ = self.engine.start(now, SessionStart::Group { group_id });
        self.engine
            .handle_event(now, PlatformEvent::L2capPublished { psm: 0x0080 });
    }
}

impl DemoWorld {
    /// Runs the bots up to `now` and delivers their packets to the user's engine.
    pub fn drive(&mut self, local: &mut Engine, now: Instant) {
        while let Some(t) = self.next_internal().filter(|t| *t <= now) {
            self.now = t.max(self.now);
            self.step(local, now);
        }
        self.now = now.max(self.now);
    }

    /// The next instant the world needs to run (deliveries, bot timers, audio blocks).
    pub fn next_deadline(&self) -> Option<Instant> {
        self.next_internal()
    }

    fn next_internal(&self) -> Option<Instant> {
        let mut t = self.in_flight.keys().next().map(|k| Instant(k.0));
        for b in &self.bots {
            for c in [
                Some(b.next_tick),
                Some(b.next_sample),
                b.engine.poll_timeout(),
            ] {
                t = match (t, c) {
                    (Some(a), Some(b)) => Some(a.min(b)),
                    (a, b) => a.or(b),
                };
            }
            if b.audio_open {
                t = Some(t.map_or(b.next_block, |a| a.min(b.next_block)));
            }
        }
        if let Some(n) = self.next_talk {
            t = Some(t.map_or(n, |a| a.min(n)));
        }
        t
    }

    fn step(&mut self, local: &mut Engine, local_now: Instant) {
        let now = self.now;
        // Deliveries due.
        while let Some((&(at, s), _)) = self.in_flight.iter().next() {
            if at > now.0 {
                break;
            }
            let (to, link, bytes) = self.in_flight.remove(&(at, s)).expect("in flight");
            let up = self.find(to, link).is_some_and(|(li, _)| self.links[li].up);
            if !up {
                continue;
            }
            match to {
                End::Local => local.handle_packet(local_now, link, &bytes),
                End::Bot(i) => self.bots[i].engine.handle_packet(now, link, &bytes),
            }
        }
        for i in 0..self.bots.len() {
            let b = &mut self.bots[i];
            if b.next_sample <= now {
                b.next_sample = now.plus_us(SAMPLE_US);
                let wall = self.wall_ms + now.0 / 1_000;
                let s = PlatformSample {
                    wall_ms: Some(wall),
                    battery: Some(0.8),
                    charging: Some(true),
                    ..PlatformSample::default()
                };
                b.engine.handle_event(now, PlatformEvent::Platform(s));
            }
            if b.audio_open && b.next_block <= now {
                b.next_block = b.next_block.plus_us(BLOCK_US).max(now.plus_us(1));
                let block = if b.capture {
                    b.speech.pop_front().unwrap_or([0i16; BLOCK_SAMPLES])
                } else {
                    [0i16; BLOCK_SAMPLES]
                };
                if b.capture {
                    b.engine.push_capture(now, &block);
                }
                let mut out = [0i16; BLOCK_SAMPLES];
                b.engine.pull_playout(now, &mut out);
            }
            if b.release_at.is_some_and(|r| r <= now) {
                b.release_at = None;
                b.engine.set_rt_flags(0);
            }
            let due = b.engine.poll_timeout().is_some_and(|t| t <= now);
            if due || b.next_tick <= now {
                b.engine.handle_timeout(now);
                while b.next_tick <= now {
                    b.next_tick = b.next_tick.plus_us(TICK_US);
                }
            }
            self.drain_bot(local, i, now);
        }
        self.schedule(now);
    }

    fn drain_bot(&mut self, local: &mut Engine, i: usize, now: Instant) {
        loop {
            let mut progressed = false;
            while let Some(t) = self.bots[i].engine.poll_transmit(now) {
                progressed = true;
                if let Some((li, d)) = self.find(End::Bot(i), t.link)
                    && self.links[li].up
                {
                    let (to, link) = self.links[li].ends[1 - d];
                    self.send(now, to, link, t.bytes);
                }
            }
            while let Some(e) = self.bots[i].engine.poll_effect() {
                progressed = true;
                self.bot_effect(local, i, now, e);
            }
            if !progressed {
                break;
            }
        }
    }

    fn bot_effect(&mut self, local: &mut Engine, i: usize, now: Instant, e: Effect) {
        let b = &mut self.bots[i];
        match e {
            Effect::Persist { key, value } => match value {
                Some(v) => {
                    b.store.insert(key, v);
                }
                None => {
                    b.store.remove(&key);
                }
            },
            Effect::Audio(AudioCommand::OpenSession { .. }) => {
                b.audio_open = true;
                b.next_block = now.plus_us(BLOCK_US);
                let route = AudioRouteInfo {
                    kind: AudioRouteKind::BuiltinSpeaker,
                    input_rate: 16_000,
                    output: AudioRouteKind::BuiltinSpeaker,
                    headset_present: false,
                };
                b.engine.handle_event(now, PlatformEvent::AudioRoute(route));
                b.engine
                    .handle_event(now, PlatformEvent::AudioIo(AudioIoState::Running));
            }
            Effect::Audio(AudioCommand::StartCapture) => b.capture = true,
            Effect::Audio(AudioCommand::StopCapture) => b.capture = false,
            Effect::Audio(AudioCommand::CloseSession) => {
                b.audio_open = false;
                b.capture = false;
            }
            Effect::Radio(RadioCommand::Close { link, .. }) => {
                if let Some((li, d)) = self.find(End::Bot(i), link)
                    && self.links[li].up
                {
                    self.links[li].up = false;
                    let (to, l) = self.links[li].ends[1 - d];
                    let ev = PlatformEvent::Down {
                        link: l,
                        reason: mesh_core::LinkDownReason::PeerClosed,
                    };
                    match to {
                        End::Local => local.handle_event(now, ev),
                        End::Bot(j) => self.bots[j].engine.handle_event(now, ev),
                    }
                }
            }
            _ => {}
        }
    }

    /// Ride start once the user is in the leader's roster, go live, and the talk
    /// schedule.
    fn schedule(&mut self, now: Instant) {
        if !self.ride_started {
            let complete = self.bots[0]
                .group_id
                .and_then(|id| self.bots[0].store.get(&store::group_key(&id)))
                .and_then(|blob| match store::decode(blob).ok()? {
                    store::Stored::Live(g) => Some(g.member_count()),
                    store::Stored::Closed(_) => None,
                })
                .is_some_and(|n| usize::from(n) > self.bots.len());
            if complete {
                self.ride_started = true;
                for b in &mut self.bots {
                    let _ = b.engine.handle_command(now, UiCommand::GoLive {});
                    let _ = b.engine.handle_command(
                        now,
                        UiCommand::SetTalkMode {
                            mode: TalkMode::Ptt,
                        },
                    );
                }
                let _ = self.bots[0]
                    .engine
                    .handle_command(now, UiCommand::RideControl { op: RideOp::Start });
                self.next_talk = Some(now.plus_us(self.opts.first_talk_us));
            }
            return;
        }
        let Some(t) = self.next_talk.filter(|t| *t <= now) else {
            return;
        };
        let n = self.utterances;
        // Members first, the leader every fourth utterance.
        let who = [1, 2, 3, 0][n % 4] % self.bots.len();
        let ms = self.rng.range_u64(2_500, 4_500) as u32;
        let speech = self.voice.utterance(n, ms);
        let dur = speech.len() as u64 * BLOCK_US;
        let b = &mut self.bots[who];
        b.speech = speech;
        b.engine.set_rt_flags(rt_flags::PTT_DOWN);
        b.release_at = Some(t.plus_us(dur + 100_000));
        self.utterances += 1;
        let (g0, g1) = self.opts.gap_us;
        let gap = self.rng.range_u64(g0, g1);
        self.next_talk = Some(t.plus_us(dur + gap));
    }
}
