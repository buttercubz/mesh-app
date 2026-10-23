//! UI dispatch and snapshots (WP-10A): error codes per mode, the snapshot dirty flag
//! with ≤ 10 Hz emission, change-only emission, detached suppression, requestSnapshot,
//! module registration and dispatch through `CoreModule`, the inter-module media bus and
//! RT flag delivery.

#[path = "../links/harness.rs"]
mod harness;

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::rc::Rc;

use harness::World;
use mesh_api::{
    AudioCheckMode, AudioCheckOp, CommandResult, ErrorCode, LabAnchor, LabAudio, LabBench,
    LabParams, Lang, MemberRole, PromptKey, PttStyle, RoutePreference, Snapshot, UiCommand,
    UiEvent, Vehicle, VoxSensitivity,
};
use mesh_core::effects::{AudioCommand, Effect, LogLevel};
use mesh_core::engine::MAX_BUS_PASSES;
use mesh_core::media::MediaRequest;
use mesh_core::{
    AudioIoState, AudioRouteInfo, AudioRouteKind, CoreModule, Ctx, Engine, EngineConfig,
    EngineMode, Instant, LinkId, Platform, PlatformEvent, RadioAvail, SessionStart, StoredGroup,
    rt_flags,
};

fn err(r: &CommandResult) -> Option<ErrorCode> {
    r.error_code()
}

fn snapshots(w: &World, i: usize) -> Vec<(usize, Snapshot)> {
    w.nodes[i]
        .effects
        .iter()
        .enumerate()
        .filter_map(|(k, e)| match e {
            Effect::Ui(UiEvent::Snapshot { s }) => Some((k, s.clone())),
            _ => None,
        })
        .collect()
}

#[test]
fn commands_are_refused_with_the_right_code_per_mode() {
    let mut w = World::new(1, Platform::Ios, 50);
    let lab = UiCommand::LabStart {
        bench: LabBench::T0,
        params: LabParams::new(LabAudio::Off, LabAnchor::Audio, false),
    };
    assert_eq!(
        err(&w.command(0, lab.clone())),
        Some(ErrorCode::EngineNotRunning)
    );
    assert_eq!(
        err(&w.command(0, UiCommand::GoLive {})),
        Some(ErrorCode::NoSession)
    );
    // Group management works in Idle (§3.2): the group module (WP-10B) answers it, the
    // base fallback (`notSupported`) is never reached.
    let created = w.command(
        0,
        UiCommand::CreateGroup {
            name: "Ride".into(),
            vehicle: Vehicle::Moto,
        },
    );
    assert_ne!(err(&created), Some(ErrorCode::NotSupported));
    assert!(created.is_ok(), "createGroup in Idle: {created:?}");
    assert_eq!(
        err(&w.command(0, UiCommand::LabStop {})),
        Some(ErrorCode::NoLabRun)
    );
    assert_eq!(
        err(&w.command(
            0,
            UiCommand::LabDeleteRun {
                run_id: "r-x".into()
            }
        )),
        Some(ErrorCode::UnknownLabRun)
    );
    assert!(w.command(0, UiCommand::RequestSnapshot {}).is_ok());
    let r = w.command(
        0,
        UiCommand::SetRoutePreference {
            pref: RoutePreference::Speaker,
        },
    );
    assert!(r.is_ok());
    assert!(w.nodes[0].effects.iter().any(|e| matches!(
        e,
        Effect::Audio(AudioCommand::SetRoutePreference(RoutePreference::Speaker))
    )));
    w.start_lab(1234);
    assert_eq!(
        err(&w.command(0, UiCommand::GoLive {})),
        Some(ErrorCode::WrongMode)
    );
    let mut bad = LabParams::new(LabAudio::Off, LabAnchor::Audio, false);
    bad.streams = Some(9);
    assert_eq!(
        err(&w.command(
            0,
            UiCommand::LabStart {
                bench: LabBench::T0,
                params: bad
            }
        )),
        Some(ErrorCode::InvalidParams)
    );
    assert!(w.command(0, lab.clone()).is_ok());
    assert_eq!(err(&w.command(0, lab)), Some(ErrorCode::LabRunActive));
    assert_eq!(
        err(&w.command(
            0,
            UiCommand::AudioCheck(AudioCheckOp::Start {
                mode: AudioCheckMode::Tone
            })
        )),
        Some(ErrorCode::LabRunActive)
    );
    assert!(
        w.command(0, UiCommand::LabMark { label: "x".into() })
            .is_ok()
    );
    assert!(w.command(0, UiCommand::LabStop {}).is_ok());
    let CommandResult::LabRuns(runs) = w.command(0, UiCommand::LabListRuns {}) else {
        panic!("labListRuns");
    };
    assert_eq!(runs.runs.len(), 1);
    let id = runs.runs[0].run_id.clone();
    assert!(
        w.command(0, UiCommand::LabDeleteRun { run_id: id.clone() })
            .is_ok()
    );
    assert!(w.nodes[0].effects.iter().any(|e| matches!(
        e,
        Effect::LabDelete { run_id } if *run_id == id
    )));
}

#[test]
fn snapshots_are_rate_limited_change_only_and_suppressed_while_detached() {
    let mut w = World::new(2, Platform::Ios, 51);
    w.start_lab(77);
    w.connect(0, 1);
    // Many state changes in quick succession (route flapping every 5 ms).
    for k in 0..200u64 {
        let kind = if k % 2 == 0 {
            AudioRouteKind::BuiltinSpeaker
        } else {
            AudioRouteKind::HfpWideband
        };
        w.event(
            0,
            PlatformEvent::AudioRoute(AudioRouteInfo {
                kind,
                ..AudioRouteInfo::NONE
            }),
        );
        w.run_ms(5);
    }
    let snaps = snapshots(&w, 0);
    assert!(snaps.len() >= 5);
    // ≤ 10 Hz: 1 s of flapping produces at most ~11 snapshots.
    assert!(snaps.len() <= 12, "{} snapshots", snaps.len());
    // Quiet engine: no further snapshot without a change.
    w.run_ms(3_000);
    let quiet = snapshots(&w, 0).len();
    w.run_ms(3_000);
    assert_eq!(
        snapshots(&w, 0).len(),
        quiet,
        "unchanged state is not re-sent"
    );
    // requestSnapshot forces one even without a change.
    w.command(0, UiCommand::RequestSnapshot {});
    w.run_ms(200);
    assert_eq!(snapshots(&w, 0).len(), quiet + 1);
    // Detached: nothing, even with changes.
    let now = w.now;
    w.nodes[0].engine.set_ui_attached(false);
    w.collect(0);
    w.event(0, PlatformEvent::AudioIo(AudioIoState::Running));
    w.run_ms(1_000);
    assert_eq!(snapshots(&w, 0).len(), quiet + 1);
    // Re-attached: the current state at once.
    w.nodes[0].engine.set_ui_attached(true);
    w.collect(0);
    let after = snapshots(&w, 0);
    assert_eq!(after.len(), quiet + 2);
    let s = &after.last().unwrap().1;
    assert!(s.audio.capture_ok);
    assert_eq!(s.mesh.links, 1);
    assert_eq!(s.mesh.neighbors, 1);
    assert_eq!(s.engine, mesh_api::EngineState::Running);
    let _ = now;
}

#[test]
fn snapshot_tracks_bluetooth_and_engine_state() {
    let mut w = World::new(1, Platform::Android, 52);
    w.start_lab(5);
    w.run_ms(200);
    w.event(
        0,
        PlatformEvent::RadioState {
            ble: RadioAvail::Unauthorized,
            aware: RadioAvail::Unsupported,
        },
    );
    w.run_ms(200);
    let s = snapshots(&w, 0).last().unwrap().1.clone();
    assert_eq!(s.device.bluetooth, mesh_api::BluetoothState::Unauthorized);
    assert!(w.ui_events(0).iter().any(|e| matches!(
        e,
        UiEvent::Notice {
            key: mesh_api::NoticeKey::BluetoothDenied,
            ..
        }
    )));
    let now = w.now;
    w.nodes[0].engine.stop(now);
    w.collect(0);
    w.run_ms(200);
    let s = snapshots(&w, 0).last().unwrap().1.clone();
    assert_eq!(s.engine, mesh_api::EngineState::Idle);
    assert_eq!(w.nodes[0].engine.mode(), EngineMode::Idle);
}

/// A test module standing in for a WP-10B/C/D module. Registered after every built-in
/// module, it only sees the commands none of them answered.
struct Probe {
    log: Rc<RefCell<Vec<String>>>,
}

impl CoreModule for Probe {
    fn name(&self) -> &'static str {
        "probe"
    }

    fn on_mode(&mut self, _ctx: &mut Ctx, mode: EngineMode) {
        self.log.borrow_mut().push(format!("mode {mode:?}"));
    }

    fn on_command(&mut self, _ctx: &mut Ctx, cmd: &UiCommand) -> Option<CommandResult> {
        self.log
            .borrow_mut()
            .push(format!("cmd {}", cmd.command_type().as_str()));
        matches!(cmd, UiCommand::GoLive {}).then(CommandResult::done)
    }

    fn on_rt_flags(&mut self, _ctx: &mut Ctx, flags: u32) {
        self.log.borrow_mut().push(format!("rt {flags}"));
    }

    fn security_for(
        &mut self,
        _ctx: &mut Ctx,
        link: LinkId,
        _info: &mesh_core::LinkInfo,
    ) -> Option<Box<dyn mesh_core::LinkSecurity>> {
        self.log.borrow_mut().push(format!("security {}", link.0));
        None
    }

    fn fill_snapshot(&self, ctx: &Ctx, s: &mut Snapshot) {
        if let EngineMode::Session { .. } = ctx.mode {
            s.session = Some(mesh_api::SessionInfo {
                group_id: "00".repeat(16),
                group_name: "Ride".into(),
                role: MemberRole::Leader,
                my_node_id: 7,
                live: false,
                mode: mesh_api::TalkMode::Vox,
                talk: mesh_api::TalkState::Idle,
                ride: mesh_api::RidePhase::Lobby,
                ladder: mesh_api::LadderLevel::L0,
                share_distance: false,
            });
        }
    }
}

fn probe_commands(log: &Rc<RefCell<Vec<String>>>) -> Vec<String> {
    log.borrow()
        .iter()
        .filter(|s| s.starts_with("cmd "))
        .cloned()
        .collect()
}

#[test]
fn modules_receive_commands_events_and_fill_the_snapshot() {
    let mut cfg = EngineConfig::new(Platform::Ios, [3; 32]);
    cfg.stored_groups.push(StoredGroup {
        group_id: [0; 16],
        blob: Vec::new(),
    });
    let mut e = Engine::new(cfg, Instant::ZERO);
    // Built-in modules (group, media, ... as the sub-WPs land) come first; a module
    // registered later is called after all of them.
    let builtin = e.module_names();
    assert!(!builtin.contains(&"probe"));
    let log = Rc::new(RefCell::new(Vec::new()));
    e.register_module(Box::new(Probe { log: log.clone() }));
    let names = e.module_names();
    assert_eq!(names.last(), Some(&"probe"));
    assert_eq!(names[..names.len() - 1], builtin[..]);
    // In Idle no built-in module takes goLive: it reaches the probe.
    assert!(e.handle_command(Instant(1), UiCommand::GoLive {}).is_ok());
    assert_eq!(probe_commands(&log), ["cmd goLive"]);
    assert_eq!(
        e.start(Instant(2), SessionStart::Group { group_id: [1; 16] }),
        Err(mesh_core::StartError::UnknownGroup)
    );
    assert_eq!(
        e.start(Instant(2), SessionStart::Demo),
        Err(mesh_core::StartError::DemoIsRuntime)
    );
    e.start(Instant(3), SessionStart::Group { group_id: [0; 16] })
        .unwrap();
    // A link no module secures (the stored group blob is empty) is refused at once.
    e.handle_event(
        Instant(4),
        PlatformEvent::Up {
            link: LinkId(9),
            info: mesh_core::LinkInfo::new(
                mesh_core::LinkKind::BleL2cap,
                mesh_core::LinkRole::Responder,
            ),
        },
    );
    // The snapshot is rate-limited (≤ 10 Hz): it leaves at the next deadline.
    let due = e.poll_timeout().expect("a snapshot is pending");
    assert!(due <= Instant(100_001));
    e.handle_timeout(due);
    let mut effects = Vec::new();
    while let Some(x) = e.poll_effect() {
        effects.push(x);
    }
    assert!(effects.iter().any(|x| matches!(
        x,
        Effect::Radio(mesh_core::RadioCommand::Close {
            link: LinkId(9),
            ..
        })
    )));
    let snap = effects
        .iter()
        .rev()
        .find_map(|x| match x {
            Effect::Ui(UiEvent::Snapshot { s }) => Some(s.clone()),
            _ => None,
        })
        .unwrap();
    // The last module to fill the snapshot wins.
    assert_eq!(snap.session.unwrap().my_node_id, 7);
    {
        let l = log.borrow();
        assert!(l.iter().any(|s| s.starts_with("mode Session")));
        assert!(l.iter().any(|s| s == "security 9"));
    }
    // In a session the media module (WP-10C) answers its commands, and those the session
    // module passes on to it, before a later-registered module: the probe sees none.
    let media_cmds = [
        UiCommand::GoLive {},
        UiCommand::ResumeMic {},
        UiCommand::SetTalkMode {
            mode: mesh_api::TalkMode::Ptt,
        },
        UiCommand::Ptt { down: true },
        UiCommand::Ptt { down: false },
        UiCommand::SetPrompts {
            lang: Lang::En,
            enabled: false,
            templates: BTreeMap::new(),
        },
        UiCommand::SetVoxSensitivity {
            level: VoxSensitivity::High,
        },
        UiCommand::SetPttStyle {
            style: PttStyle::Latch,
        },
        UiCommand::SetBuiltinMicTx { allowed: true },
    ];
    for (k, cmd) in media_cmds.into_iter().enumerate() {
        let t = cmd.command_type().as_str();
        e.handle_command(Instant(200_000 + k as u64), cmd);
        assert_eq!(
            probe_commands(&log),
            ["cmd goLive"],
            "{t} reached the probe"
        );
    }
}

/// Posts a media request when it answers `goLive` (which no built-in module takes in
/// Idle) and logs what it sees on the bus; with `spin`, it re-posts on every pass.
struct Poster {
    log: Rc<RefCell<Vec<String>>>,
    spin: bool,
}

impl CoreModule for Poster {
    fn name(&self) -> &'static str {
        "poster"
    }

    fn on_command(&mut self, ctx: &mut Ctx, cmd: &UiCommand) -> Option<CommandResult> {
        matches!(cmd, UiCommand::GoLive {}).then(|| {
            ctx.post_media(MediaRequest::SetTalkMode(mesh_api::TalkMode::Ptt));
            CommandResult::done()
        })
    }

    fn on_bus(&mut self, ctx: &mut Ctx) {
        self.log
            .borrow_mut()
            .push(format!("bus {}", ctx.media_bus.len()));
        if self.spin {
            ctx.post_media(MediaRequest::AudioCheckStop);
        }
    }
}

fn drain(e: &mut Engine) -> Vec<Effect> {
    std::iter::from_fn(|| e.poll_effect()).collect()
}

#[test]
fn media_requests_on_the_bus_run_within_the_same_engine_call() {
    let mut e = Engine::new(EngineConfig::new(Platform::WebSim, [4; 32]), Instant::ZERO);
    let log = Rc::new(RefCell::new(Vec::new()));
    e.register_module(Box::new(Poster {
        log: log.clone(),
        spin: false,
    }));
    // Posted while a module handles a command: the media module (registered earlier)
    // drains the bus before the call returns, so later modules see it empty.
    assert!(e.handle_command(Instant(1), UiCommand::GoLive {}).is_ok());
    assert!(e.ctx().media_bus.is_empty());
    assert_eq!(*log.borrow(), ["bus 0"]);
    // A runtime request (Demo start, §3.4) is handled within `media_request` itself.
    let mut templates = BTreeMap::new();
    templates.insert(PromptKey::DemoLine1, "Hola".to_string());
    let r = e.handle_command(
        Instant(2),
        UiCommand::SetPrompts {
            lang: Lang::Es,
            enabled: true,
            templates,
        },
    );
    assert!(r.is_ok());
    drain(&mut e);
    e.media_request(Instant(3), MediaRequest::DemoStart);
    assert!(e.ctx().media_bus.is_empty());
    assert!(drain(&mut e).iter().any(|x| matches!(
        x,
        Effect::RenderPrompts(v) if v.iter().any(|q| q.key == PromptKey::DemoLine1)
    )));
    // A module that keeps re-posting is cut off after MAX_BUS_PASSES passes (logged).
    let mut e = Engine::new(EngineConfig::new(Platform::WebSim, [5; 32]), Instant::ZERO);
    let log = Rc::new(RefCell::new(Vec::new()));
    e.register_module(Box::new(Poster {
        log: log.clone(),
        spin: true,
    }));
    e.media_request(Instant(1), MediaRequest::AudioCheckStop);
    assert!(e.ctx().media_bus.is_empty());
    assert_eq!(log.borrow().len(), MAX_BUS_PASSES);
    assert!(drain(&mut e).iter().any(|x| matches!(
        x,
        Effect::Log { level: LogLevel::Warn, msg } if msg.contains("dropped")
    )));
}

#[test]
fn latch_toggle_is_delivered_on_every_call_that_carries_it() {
    let mut e = Engine::new(EngineConfig::new(Platform::Ios, [6; 32]), Instant::ZERO);
    let log = Rc::new(RefCell::new(Vec::new()));
    e.register_module(Box::new(Probe { log: log.clone() }));
    let delivered =
        |l: &Rc<RefCell<Vec<String>>>| l.borrow().iter().filter(|s| s.starts_with("rt ")).count();
    // Level bits: an unchanged value is not re-delivered.
    e.set_rt_flags(rt_flags::HARD_MUTE);
    e.set_rt_flags(rt_flags::HARD_MUTE);
    assert_eq!(delivered(&log), 1);
    // Bit3 is an edge: two lock-screen taps in consecutive ticks are two deliveries.
    e.set_rt_flags(rt_flags::HARD_MUTE | rt_flags::LATCH_TOGGLE);
    e.set_rt_flags(rt_flags::HARD_MUTE | rt_flags::LATCH_TOGGLE);
    e.set_rt_flags(rt_flags::HARD_MUTE);
    e.set_rt_flags(rt_flags::HARD_MUTE);
    assert_eq!(delivered(&log), 4);
}
