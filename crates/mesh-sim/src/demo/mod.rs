//! The Demo ride world (DESIGN §3.4, §1.3): four bot riders around the user's own
//! engine, so the real audio, floor and background path runs on one phone, in the iOS
//! Simulator and in the browser. The runtime (mesh-ffi, mesh-wasm) owns the user's
//! engine; [`DemoWorld::attach`] joins it to a demo group led by a bot, starts its
//! session and connects it to the bots over virtual links:
//!
//! ```text
//! runtime loop (every tick, `now` from the runtime clock):
//!   for t in local.poll_transmit(now): if !demo.on_local_transmit(now, &t) { platform.send(t) }
//!   for e in local.poll_effect():   Radio(c) → demo.on_local_radio(local, now, &c) (no radio in demo)
//!                                   RenderPrompts(demo.*) → TTS → demo.set_lines(pcm)
//!   demo.drive(local, now)          // bots run up to `now`; their packets reach `local`
//!   park until min(local.poll_timeout(), demo.next_deadline(), 5 ms)
//! ```
//!
//! Bots are real `mesh_core::Engine`s (the same platform and codec as the user's engine,
//! so libopus on devices and SimCodec in WASM) with their own secure links, group,
//! session, media and mesh modules. The leader bot starts the ride once the user is in
//! its roster; bots go live in PTT and take turns speaking the `demo.*` lines (or the
//! synthesized fallback) on a talk schedule.

mod bots;
pub mod voice;

use std::collections::BTreeMap;

use mesh_api::{CommandResult, UiCommand};
use mesh_core::effects::{RadioCommand, Transmit};
use mesh_core::media::MediaRequest;
use mesh_core::{Engine, EngineMode, Instant, LinkId, SessionStart, StartError};

pub use voice::{DemoVoice, Pcm};

use bots::Bot;

/// Display names of the bots (user content shown in the member list).
pub const BOT_NAMES: [&str; 4] = ["Alex", "Sam", "Kim", "Lou"];

/// Local link ids of the demo links (far from the platform's own ids).
pub const DEMO_LINK_BASE: u32 = 0xDE00_0000;

/// Demo world options.
#[derive(Debug, Clone, PartialEq)]
pub struct DemoOptions {
    /// The user's display name in the demo group.
    pub display_name: String,
    /// Seed of the bots' RNGs and talk schedule.
    pub seed: u64,
    /// One-way delay of a virtual link (µs).
    pub link_delay_us: u64,
    /// Time from the ride start to the first bot utterance (µs).
    pub first_talk_us: u64,
    /// Pause between two utterances (µs, uniform in the range).
    pub gap_us: (u64, u64),
}

impl Default for DemoOptions {
    fn default() -> Self {
        DemoOptions {
            display_name: "You".into(),
            seed: 0xDE30_5EED,
            link_delay_us: 15_000,
            first_talk_us: 3_000_000,
            gap_us: (4_000_000, 9_000_000),
        }
    }
}

/// Why the demo could not start.
#[derive(Debug, Clone, PartialEq)]
pub enum DemoError {
    /// The user's engine is not idle (a session or a Lab run is active).
    NotIdle,
    /// A bot could not create or join the demo group.
    Bot(String),
    /// The user's engine refused to join the demo group.
    Join(CommandResult),
    /// The user's engine refused to start the demo session.
    Start(StartError),
}

/// One end of a virtual link.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum End {
    Local,
    Bot(usize),
}

/// A virtual link.
#[derive(Debug, Clone, Copy)]
pub(crate) struct VLink {
    pub ends: [(End, LinkId); 2],
    pub up: bool,
}

/// The demo world.
pub struct DemoWorld {
    pub(crate) now: Instant,
    pub(crate) opts: DemoOptions,
    pub(crate) bots: Vec<Bot>,
    pub(crate) links: Vec<VLink>,
    /// `(deliver at, seq) → (to, link, bytes)`.
    pub(crate) in_flight: BTreeMap<(u64, u64), (End, LinkId, Vec<u8>)>,
    pub(crate) seq: u64,
    pub(crate) voice: DemoVoice,
    pub(crate) group_id: [u8; 16],
    pub(crate) rng: crate::rng::SimRng,
    /// The leader bot started the ride.
    pub ride_started: bool,
    pub(crate) next_talk: Option<Instant>,
    pub(crate) utterances: usize,
    pub(crate) wall_ms: u64,
}

impl DemoWorld {
    /// Attaches a demo world to the user's idle engine with default options; `lines`
    /// is the TTS rendering of `demo.line1…8` (or `None`: the synthesized fallback).
    pub fn attach(
        local: &mut Engine,
        now: Instant,
        lines: Option<Vec<Pcm>>,
    ) -> Result<DemoWorld, DemoError> {
        Self::attach_with(local, now, lines, DemoOptions::default())
    }

    /// Attaches with explicit options: the bots form the demo group (bot 0 leads), the
    /// user's engine joins it from the invite and starts its session, the virtual links
    /// come up, and the user's engine is asked to render the `demo.*` lines (its
    /// `RenderPrompts` effect; pass the PCM to [`DemoWorld::set_lines`]).
    pub fn attach_with(
        local: &mut Engine,
        now: Instant,
        lines: Option<Vec<Pcm>>,
        opts: DemoOptions,
    ) -> Result<DemoWorld, DemoError> {
        if local.mode() != EngineMode::Idle {
            return Err(DemoError::NotIdle);
        }
        let platform = local.ctx().cfg.platform;
        let wall_ms = local.ctx().device.wall_ms(now);
        let mut bots: Vec<Bot> = (0..BOT_NAMES.len())
            .map(|i| Bot::new(i, platform, opts.seed, now, wall_ms))
            .collect();
        let url = bots[0].create_group(now)?;
        for b in bots.iter_mut().skip(1) {
            b.join(now, &url)?;
        }
        let group_id = bots[0]
            .group_id
            .ok_or_else(|| DemoError::Bot("no group".into()))?;
        let r = local.handle_command(
            now,
            UiCommand::JoinGroup {
                url,
                display_name: opts.display_name.clone(),
            },
        );
        if !matches!(r, CommandResult::GroupId(_)) {
            return Err(DemoError::Join(r));
        }
        local
            .start(now, SessionStart::Group { group_id })
            .map_err(DemoError::Start)?;
        for b in &mut bots {
            b.start(now, group_id);
        }
        let voice = DemoVoice::new(platform, lines, opts.seed);
        let mut w = DemoWorld {
            now,
            rng: crate::rng::SimRng::new(opts.seed, 0xDE30_0001),
            opts,
            bots,
            links: Vec::new(),
            in_flight: BTreeMap::new(),
            seq: 0,
            voice,
            group_id,
            ride_started: false,
            next_talk: None,
            utterances: 0,
            wall_ms,
        };
        // A small mesh: the user hears the leader directly and the far bots relayed.
        for (a, b) in [
            (End::Local, End::Bot(0)),
            (End::Local, End::Bot(1)),
            (End::Bot(0), End::Bot(2)),
            (End::Bot(1), End::Bot(3)),
            (End::Bot(2), End::Bot(3)),
        ] {
            w.connect(local, a, b);
        }
        local.media_request(now, MediaRequest::DemoStart);
        Ok(w)
    }

    /// The demo group id.
    pub fn group_id(&self) -> [u8; 16] {
        self.group_id
    }

    /// Replaces the bots' voice with rendered `demo.*` lines (encoded once, PCM
    /// dropped). Empty or missing renders keep the synthesized fallback.
    pub fn set_lines(&mut self, lines: Vec<Pcm>) {
        let platform = self.bots[0].engine.ctx().cfg.platform;
        let v = DemoVoice::new(platform, Some(lines), self.opts.seed);
        if v.has_lines() {
            self.voice = v;
        }
    }

    /// `true` when the bots speak rendered lines.
    pub fn has_lines(&self) -> bool {
        self.voice.has_lines()
    }

    /// Bot engines (diagnostics, tests).
    pub fn bot_engine(&self, i: usize) -> &Engine {
        &self.bots[i].engine
    }

    /// Utterances the bots started so far.
    pub fn utterances(&self) -> usize {
        self.utterances
    }

    fn connect(&mut self, local: &mut Engine, a: End, b: End) {
        let k = self.links.len() as u32;
        let ids = [
            LinkId(DEMO_LINK_BASE + 2 * k),
            LinkId(DEMO_LINK_BASE + 2 * k + 1),
        ];
        self.links.push(VLink {
            ends: [(a, ids[0]), (b, ids[1])],
            up: true,
        });
        let now = self.now;
        for (role, (end, id)) in [
            (mesh_core::LinkRole::Initiator, (a, ids[0])),
            (mesh_core::LinkRole::Responder, (b, ids[1])),
        ] {
            let mut info = mesh_core::LinkInfo::new(mesh_core::LinkKind::BleL2cap, role);
            if role == mesh_core::LinkRole::Initiator {
                info.disc_id = Some(0xDE00 + k);
            }
            let ev = mesh_core::PlatformEvent::Up { link: id, info };
            match end {
                End::Local => local.handle_event(now, ev),
                End::Bot(i) => {
                    self.bots[i].engine.handle_event(now, ev);
                }
            }
        }
    }

    /// Routes one unit of the user's engine: `true` when it belonged to a demo link
    /// (it is delivered to the bot after the link delay), `false` for the platform.
    pub fn on_local_transmit(&mut self, now: Instant, t: &Transmit) -> bool {
        let Some((li, d)) = self.find(End::Local, t.link) else {
            return false;
        };
        if self.links[li].up {
            let (to, link) = self.links[li].ends[1 - d];
            self.send(now, to, link, t.bytes.clone());
        }
        true
    }

    /// A radio command of the user's engine while the demo runs: demo-link closes take
    /// the link down on the bot side; every other command is absorbed (no radio in
    /// Demo). Always `true` (never forward a radio command to the platform in Demo).
    pub fn on_local_radio(&mut self, now: Instant, cmd: &RadioCommand) -> bool {
        if let RadioCommand::Close { link, .. } = cmd
            && let Some((li, d)) = self.find(End::Local, *link)
            && self.links[li].up
        {
            self.links[li].up = false;
            let (to, l) = self.links[li].ends[1 - d];
            if let End::Bot(i) = to {
                self.bots[i].engine.handle_event(
                    now,
                    mesh_core::PlatformEvent::Down {
                        link: l,
                        reason: mesh_core::LinkDownReason::PeerClosed,
                    },
                );
            }
        }
        true
    }

    /// Takes every demo link down on the user's engine (leaving the Demo).
    pub fn detach(mut self, local: &mut Engine, now: Instant) {
        for li in 0..self.links.len() {
            if !self.links[li].up {
                continue;
            }
            self.links[li].up = false;
            for (end, id) in self.links[li].ends {
                if end == End::Local {
                    local.handle_event(
                        now,
                        mesh_core::PlatformEvent::Down {
                            link: id,
                            reason: mesh_core::LinkDownReason::LocalClose,
                        },
                    );
                }
            }
        }
    }

    pub(crate) fn find(&self, end: End, link: LinkId) -> Option<(usize, usize)> {
        self.links.iter().enumerate().find_map(|(li, l)| {
            l.ends
                .iter()
                .position(|e| *e == (end, link))
                .map(|d| (li, d))
        })
    }

    pub(crate) fn send(&mut self, now: Instant, to: End, link: LinkId, bytes: Vec<u8>) {
        self.seq += 1;
        let at = now.0 + self.opts.link_delay_us;
        self.in_flight.insert((at, self.seq), (to, link, bytes));
    }
}
