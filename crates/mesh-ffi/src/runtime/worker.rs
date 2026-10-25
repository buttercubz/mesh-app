//! The engine thread (`mesh-engine`, DESIGN §3.4).
//!
//! The `Engine` is created on this thread and never leaves it (mesh-core modules are not
//! required to be `Send`). In Idle the thread blocks on the inbound channel: no tick, no
//! radios, no audio; group-management commands are answered as they arrive. While a
//! session, Lab mode or an audio check runs, it loops: drain inbound → drain the capture
//! ring in 10 ms blocks → refill the render ring to its target (≥ 20 ms, above the largest
//! render callback) → apply RT flags →
//! `handle_timeout` → flush transmits/effects → wait `min(next deadline, 5 ms)` (on the
//! channel, so inbound messages wake it immediately).
//!
//! Every step runs under `catch_unwind`: a panic is reported as `engine: crashed`, radios
//! and audio are stopped through the hooks, and the thread exits cleanly; the runtime
//! spawns a fresh engine on the next call.

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::Arc;
use std::sync::atomic::Ordering::SeqCst;
use std::time::Duration;

use crossbeam_channel::{Receiver, RecvTimeoutError, TryRecvError};
use mesh_api::{AudioCheckOp, CommandResult, ErrorCode, UiCommand};
use mesh_core::media::demo::DEMO_LINES;
use mesh_core::{
    AudioCounters, BLOCK_SAMPLES, Engine, Instant, PlatformEvent, RadioAvail, SessionStart,
    StartError,
};
use mesh_sim::demo::{DemoError, DemoWorld, Pcm};

use super::Shared;
use super::dispatch::Dispatcher;
use super::inbound::Inbound;
use crate::api::outputs::FfiLogLevel;
use crate::api::types::FfiError;
use crate::rt::{CAPTURE_RING_SAMPLES, RtClaim, RtStats};

/// Engine tick (Appendix A).
pub const TICK: Duration = Duration::from_millis(5);
/// An audio check without a session is stopped after this long (safety valve).
pub const CHECK_MAX_US: u64 = 120_000_000;
/// Shortest wait between two loops (spin guard).
const MIN_WAIT: Duration = Duration::from_micros(500);
/// Inbound messages handled per loop before the audio pump runs again.
const DRAIN_BATCH: usize = 512;
/// After `stop`, link events are accepted until the core closed every link or this
/// long passed (the core's `CLOSE_DRAIN_US` bound plus a margin).
pub const STOP_DRAIN_US: u64 = mesh_core::links::CLOSE_DRAIN_US + 500_000;

/// Builds the engine on the engine thread (tests inject instrumented engines).
pub type EngineFactory = dyn Fn(mesh_core::EngineConfig, Instant) -> Engine + Send + Sync;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    Idle,
    Session,
    Check { since: Instant },
}

enum Flow {
    Continue,
    Exit,
}

/// Engine-thread state.
pub struct Worker {
    shared: Arc<Shared>,
    rx: Receiver<Inbound>,
    engine: Engine,
    fx: Dispatcher,
    mode: Mode,
    claim: Option<RtClaim<'static>>,
    radio: Option<(RadioAvail, RadioAvail)>,
    rt_base: RtStats,
    rt_last: Option<(RtStats, Instant)>,
    reports_seen: u32,
    /// The Demo ride world (virtual links and bots) while the Demo runs.
    demo: Option<DemoWorld>,
    /// Rendered `demo.*` lines, in `DEMO_LINES` order.
    demo_pcm: Vec<Pcm>,
    /// Ordered shutdown in progress since this instant (links still closing).
    draining_since: Option<Instant>,
}

/// Thread entry point.
pub fn run(shared: Arc<Shared>, rx: Receiver<Inbound>, factory: Arc<EngineFactory>) {
    if !super::os::raise_engine_priority() {
        shared.hooks.log(
            FfiLogLevel::Debug,
            "mesh-ffi: engine thread priority unchanged".into(),
        );
    }
    let built = catch_unwind(AssertUnwindSafe(|| {
        let generation = shared.generation.fetch_add(1, SeqCst);
        let store = shared.store.lock().map(|s| s.clone()).unwrap_or_default();
        let cfg = shared
            .base
            .engine_config(&store, generation, super::os::wall_ms());
        let mut engine = factory(cfg, shared.clock.now());
        engine.set_ui_attached(shared.ui_attached.load(SeqCst));
        engine
    }));
    let mut fx = Dispatcher::new(shared.hooks.clone(), shared.store.clone());
    fx.set_diag(shared.diag.clone());
    fx.set_aware_pmks(shared.aware_pmks.clone());
    fx.crashed_view = shared.crashed.load(SeqCst);
    let engine = match built {
        Ok(e) => e,
        Err(p) => {
            crash(&shared, &rx, &mut fx, &panic_text(p.as_ref()));
            return;
        }
    };
    let mut w = Worker {
        shared,
        rx,
        engine,
        fx,
        mode: Mode::Idle,
        claim: None,
        radio: None,
        rt_base: RtStats::default(),
        rt_last: None,
        reports_seen: 0,
        demo: None,
        demo_pcm: Vec::new(),
        draining_since: None,
    };
    w.flush();
    loop {
        match catch_unwind(AssertUnwindSafe(|| w.step())) {
            Ok(Flow::Continue) => {}
            Ok(Flow::Exit) => break,
            Err(p) => {
                w.claim = None;
                crash(&w.shared, &w.rx, &mut w.fx, &panic_text(p.as_ref()));
                return;
            }
        }
    }
    w.claim = None;
    w.shared.active.store(false, SeqCst);
    w.shared.draining.store(false, SeqCst);
    w.shared.checking.store(false, SeqCst);
}

/// Diagnostics mode and ride key of a session start: group sessions hash the group id
/// with the UTC date (every member computes the same key), Lab sessions use the public
/// Lab tag.
fn diag_identity(start: SessionStart, wall_ms: u64) -> (&'static str, Option<String>) {
    use mesh_lab::ridelog::{DEMO_RIDE_KEY, hex4, lab_ride_key, ride_key, utc_date};
    match start {
        SessionStart::Group { group_id } => {
            ("group", Some(hex4(&ride_key(&group_id, utc_date(wall_ms)))))
        }
        SessionStart::Lab { lab_code } => (
            "lab",
            Some(lab_ride_key(mesh_wire::frame::lab_tag(lab_code))),
        ),
        SessionStart::Demo => ("demo", Some(DEMO_RIDE_KEY.to_owned())),
    }
}

fn panic_text(p: &(dyn std::any::Any + Send)) -> String {
    p.downcast_ref::<&str>()
        .map(|s| (*s).to_owned())
        .or_else(|| p.downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "panic".into())
}

/// Crash path: report, answer queued requests, mark the runtime idle.
fn crash(shared: &Shared, rx: &Receiver<Inbound>, fx: &mut Dispatcher, what: &str) {
    shared.crashed.store(true, SeqCst);
    shared.diag.end(shared.clock.now(), Some("engine-thread"));
    shared.active.store(false, SeqCst);
    shared.draining.store(false, SeqCst);
    shared.checking.store(false, SeqCst);
    // The hooks are foreign code: a failure while reporting must not abort the process.
    let _ = catch_unwind(AssertUnwindSafe(|| fx.report_crash(what)));
    while let Ok(m) = rx.try_recv() {
        match m {
            Inbound::Command { reply, .. } => {
                let _ = reply.send(CommandResult::error(ErrorCode::EngineCrashed));
            }
            Inbound::Start { reply, .. } => {
                let _ = reply.send(Err(FfiError::Crashed));
            }
            Inbound::Stop { reply } => {
                let _ = reply.send(());
            }
            _ => {}
        }
    }
}

impl Worker {
    fn now(&self) -> Instant {
        self.shared.clock.now()
    }

    fn flush(&mut self) {
        self.fx
            .flush(&mut self.engine, &self.shared.clock, self.demo.as_mut());
    }

    fn step(&mut self) -> Flow {
        if self.mode == Mode::Idle {
            let flow = self.idle_step();
            self.update_draining();
            return flow;
        }
        for _ in 0..DRAIN_BATCH {
            match self.rx.try_recv() {
                Ok(m) => {
                    if let Flow::Exit = self.handle(m) {
                        return Flow::Exit;
                    }
                    if self.mode == Mode::Idle {
                        return Flow::Continue;
                    }
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => return Flow::Exit,
            }
        }
        self.tick();
        if self.mode == Mode::Idle {
            return Flow::Continue;
        }
        match self.rx.recv_timeout(self.wait()) {
            Ok(m) => self.handle(m),
            Err(RecvTimeoutError::Timeout) => Flow::Continue,
            Err(RecvTimeoutError::Disconnected) => Flow::Exit,
        }
    }

    /// Idle: no tick, no audio. Blocks on the channel, waking only for an engine
    /// deadline (a rate-limited snapshot, an idle module timer).
    fn idle_step(&mut self) -> Flow {
        let next = match self.engine.poll_timeout() {
            None => {
                return match self.rx.recv() {
                    Ok(m) => self.handle(m),
                    Err(_) => Flow::Exit,
                };
            }
            Some(t) => t,
        };
        let wait = Duration::from_micros(next.since(self.now())).max(MIN_WAIT);
        match self.rx.recv_timeout(wait) {
            Ok(m) => self.handle(m),
            Err(RecvTimeoutError::Timeout) => {
                let now = self.now();
                if self.engine.poll_timeout().is_some_and(|t| t <= now) {
                    self.engine.handle_timeout(now);
                    self.flush();
                }
                Flow::Continue
            }
            Err(RecvTimeoutError::Disconnected) => Flow::Exit,
        }
    }

    /// Ends the ordered shutdown once the core closed every platform link or the
    /// drain bound expired (link events are dropped again from then on).
    fn update_draining(&mut self) {
        let Some(since) = self.draining_since else {
            return;
        };
        if self.fx.open_links().is_empty() || self.now().since(since) > STOP_DRAIN_US {
            self.draining_since = None;
            self.shared.draining.store(false, SeqCst);
        }
    }

    /// `min(next engine/demo deadline, 5 ms)`, floored at [`MIN_WAIT`] so a deadline
    /// that a module fails to advance cannot spin the thread.
    fn wait(&self) -> Duration {
        let now = self.now();
        let demo = self.demo.as_ref().and_then(|d| d.next_deadline());
        let next = match (self.engine.poll_timeout(), demo) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        };
        match next {
            Some(t) => TICK.min(Duration::from_micros(t.since(now))).max(MIN_WAIT),
            None => TICK,
        }
    }

    /// One engine tick (see [`Self::tick_inner`]) plus the diagnostics sample: the
    /// recorder gets the tick's busy time and, once per second, the RT counters.
    fn tick(&mut self) {
        let t0 = std::time::Instant::now();
        self.tick_inner();
        let busy = u64::try_from(t0.elapsed().as_micros()).unwrap_or(u64::MAX);
        let rt = self.claim.as_ref().map(|c| c.shared());
        self.shared.diag.tick(self.now(), rt, busy);
    }

    /// Audio pump, flags, timers, flush, audio-check bookkeeping.
    fn tick_inner(&mut self) {
        if let Some(c) = self.claim.as_mut() {
            let mut block = [0i16; BLOCK_SAMPLES];
            for _ in 0..CAPTURE_RING_SAMPLES / BLOCK_SAMPLES {
                if c.capture_available() < BLOCK_SAMPLES || !c.pop_capture_exact(&mut block) {
                    break;
                }
                self.engine.push_capture(self.shared.clock.now(), &block);
            }
            // Adaptive: ≥ 20 ms, and above the device's largest render callback.
            let target = c.render_target();
            while c.render_fill() < target {
                self.engine
                    .pull_playout(self.shared.clock.now(), &mut block);
                if c.push_render(&block) < BLOCK_SAMPLES {
                    break;
                }
            }
            let flags = c.take_flags();
            self.engine.set_rt_flags(flags);
            let (count, code) = c.shared().reports();
            if count != self.reports_seen {
                self.reports_seen = count;
                self.fx.hooks().log(
                    FfiLogLevel::Info,
                    format!("mesh-ffi: RT report #{count} code {code}"),
                );
            }
        }
        let now = self.now();
        if self.engine.poll_timeout().is_some_and(|t| t <= now) {
            self.engine.handle_timeout(now);
        }
        self.flush();
        if let Some(d) = self.demo.as_mut() {
            d.drive(&mut self.engine, now);
            self.flush();
        }
        if let Mode::Check { since } = self.mode {
            if self.fx.take_close_session() {
                self.leave_check();
            } else if now.since(since) > CHECK_MAX_US {
                let cmd = UiCommand::AudioCheck(AudioCheckOp::Stop {});
                let _ = self.engine.handle_command(now, cmd);
                self.flush();
                self.leave_check();
            }
        }
    }

    fn audio_on(&mut self) {
        if self.claim.is_none() {
            self.claim = self.shared.rt.claim();
            if self.claim.is_none() {
                self.fx.hooks().log(
                    FfiLogLevel::Warn,
                    "mesh-ffi: RT plane owned by another runtime; running without audio".into(),
                );
            }
        }
        if let Some(c) = self.claim.as_mut() {
            c.reset_and_activate();
            self.rt_base = c.shared().stats();
            self.reports_seen = c.shared().reports().0;
        }
        self.rt_last = None;
    }

    fn audio_off(&mut self) {
        self.claim = None;
        self.rt_last = None;
    }

    fn leave_check(&mut self) {
        self.mode = Mode::Idle;
        self.shared.checking.store(false, SeqCst);
        self.audio_off();
    }

    fn handle(&mut self, m: Inbound) -> Flow {
        let now = self.now();
        match m {
            Inbound::Command { cmd, reply } => {
                let check = match &cmd {
                    UiCommand::AudioCheck(AudioCheckOp::Start { .. }) => Some(true),
                    UiCommand::AudioCheck(AudioCheckOp::Stop {}) => Some(false),
                    _ => None,
                };
                let r = self.engine.handle_command(now, cmd);
                if check == Some(true) && r.is_ok() && self.mode == Mode::Idle {
                    self.fx.take_close_session();
                    self.mode = Mode::Check { since: now };
                    self.shared.checking.store(true, SeqCst);
                    self.audio_on();
                }
                self.flush();
                let ended = self.fx.take_close_session();
                if matches!(self.mode, Mode::Check { .. })
                    && (ended || (check == Some(false) && r.is_ok()))
                {
                    self.leave_check();
                }
                let _ = reply.send(r);
            }
            Inbound::Start { start, reply } => {
                let r = self.start(now, start);
                let _ = reply.send(r);
            }
            Inbound::Stop { reply } => {
                self.stop(now);
                let _ = reply.send(());
            }
            Inbound::Shutdown => {
                self.stop(now);
                return Flow::Exit;
            }
            other => self.event(now, other),
        }
        Flow::Continue
    }

    fn start(&mut self, now: Instant, start: SessionStart) -> Result<(), FfiError> {
        self.shared.active.store(true, SeqCst);
        if start == SessionStart::Demo {
            return self.start_demo(now);
        }
        match self.engine.start(now, start) {
            Ok(()) => {
                let (mode, key) = diag_identity(start, super::os::wall_ms());
                self.shared.diag.begin(now, super::os::wall_ms(), mode, key);
                self.mode = Mode::Session;
                self.shared.checking.store(false, SeqCst);
                self.shared.crashed.store(false, SeqCst);
                self.fx.crashed_view = false;
                self.audio_on();
                self.flush();
                Ok(())
            }
            Err(e) => {
                self.shared.active.store(self.mode == Mode::Session, SeqCst);
                self.flush();
                Err(match e {
                    StartError::DemoIsRuntime => FfiError::DemoUnavailable,
                    StartError::BadLabCode => FfiError::InvalidStart {
                        detail: "labCode must be 0-9999".into(),
                    },
                    StartError::UnknownGroup => FfiError::UnknownGroup,
                })
            }
        }
    }

    /// The Demo ride (DESIGN §3.4): the mesh-sim world joins the idle engine to the
    /// bots' group over virtual links and starts its session; the core asks platform
    /// TTS for the `demo.*` lines (`MediaRequest::DemoStart`, issued by the world).
    /// No radio runs; audio starts with `goLive` exactly as in a group ride.
    fn start_demo(&mut self, now: Instant) -> Result<(), FfiError> {
        if self.mode == Mode::Session {
            self.shared.active.store(true, SeqCst);
            return Err(FfiError::InvalidStart {
                detail: "a session is already running".into(),
            });
        }
        if matches!(self.mode, Mode::Check { .. }) {
            let cmd = UiCommand::AudioCheck(AudioCheckOp::Stop {});
            let _ = self.engine.handle_command(now, cmd);
            self.flush();
            self.leave_check();
        }
        match DemoWorld::attach(&mut self.engine, now, None) {
            Ok(world) => {
                self.fx.clear_demo();
                let key = Some(mesh_lab::ridelog::DEMO_RIDE_KEY.to_owned());
                self.shared
                    .diag
                    .begin(now, super::os::wall_ms(), "demo", key);
                self.demo = Some(world);
                self.demo_pcm = vec![Vec::new(); DEMO_LINES.len()];
                self.mode = Mode::Session;
                self.shared.checking.store(false, SeqCst);
                self.shared.crashed.store(false, SeqCst);
                self.fx.crashed_view = false;
                self.audio_on();
                self.flush();
                Ok(())
            }
            Err(e) => {
                // A partial attach may have started the session: return to Idle.
                self.engine.stop(now);
                self.flush();
                self.shared.active.store(false, SeqCst);
                Err(match e {
                    DemoError::Start(StartError::UnknownGroup) => FfiError::UnknownGroup,
                    other => FfiError::InvalidStart {
                        detail: format!("demo: {other:?}"),
                    },
                })
            }
        }
    }

    fn stop(&mut self, now: Instant) {
        // Link events keep flowing while the core closes its links (ordered shutdown):
        // `draining` is raised before `active` drops so no event slips through the gap.
        self.shared.draining.store(true, SeqCst);
        self.shared.active.store(false, SeqCst);
        if self.mode == Mode::Idle {
            self.update_draining_after_stop(now);
            return;
        }
        if matches!(self.mode, Mode::Check { .. }) {
            let cmd = UiCommand::AudioCheck(AudioCheckOp::Stop {});
            let _ = self.engine.handle_command(now, cmd);
        }
        self.engine.stop(now);
        self.flush();
        if let Some(world) = self.demo.take() {
            // Virtual links only: take them down now (nothing to drain on a radio).
            let group_id = world.group_id();
            world.detach(&mut self.engine, now);
            // The demo group is not the user's: forget it so it never lingers in the
            // group list and the next Demo can join it again.
            let hex: String = group_id.iter().map(|b| format!("{b:02x}")).collect();
            let _ = self
                .engine
                .handle_command(now, UiCommand::ForgetGroup { group_id: hex });
            self.fx.clear_demo();
            self.demo_pcm.clear();
            self.flush();
        }
        self.update_draining_after_stop(now);
        self.mode = Mode::Idle;
        self.shared.checking.store(false, SeqCst);
        self.audio_off();
        self.shared.diag.end(now, None);
    }

    /// Starts (or keeps) the drain window after a stop; ends it at once when no
    /// platform link is open.
    fn update_draining_after_stop(&mut self, now: Instant) {
        if self.draining_since.is_none() {
            self.draining_since = Some(now);
        }
        self.update_draining();
    }

    /// A rendered `demo.*` line: kept until every requested line is in, then the
    /// world encodes them once (the PCM is dropped) and the bots speak them.
    fn demo_line(&mut self, key: mesh_api::PromptKey, pcm: &[i16]) {
        let Some(k) = DEMO_LINES.iter().position(|d| *d == key) else {
            return;
        };
        let Some(world) = self.demo.as_mut() else {
            return;
        };
        if let Some(slot) = self.demo_pcm.get_mut(k) {
            *slot = pcm.to_vec();
        }
        if self.fx.demo_line_rendered(key) {
            world.set_lines(std::mem::take(&mut self.demo_pcm));
            self.demo_pcm = vec![Vec::new(); DEMO_LINES.len()];
        }
    }

    /// Platform events (everything except the control requests).
    fn event(&mut self, now: Instant, m: Inbound) {
        let e = &mut self.engine;
        match m {
            Inbound::LinkUp { link, info } => {
                self.fx.link_up(link);
                e.handle_event(now, PlatformEvent::Up { link, info });
            }
            Inbound::LinkDown { link, reason } => {
                self.fx.link_down(link);
                e.handle_event(now, PlatformEvent::Down { link, reason });
            }
            Inbound::LinkData { link, data } => e.handle_packet(now, link, &data),
            Inbound::Writable { link } => e.handle_event(now, PlatformEvent::Writable { link }),
            Inbound::Metrics {
                link,
                rssi,
                att_mtu,
            } => e.handle_event(
                now,
                PlatformEvent::Metrics {
                    link,
                    rssi,
                    att_mtu,
                },
            ),
            Inbound::Discovered(d) => e.handle_event(now, PlatformEvent::Discovered(d)),
            Inbound::HelloRead { disc_id, bytes } => e.handle_event(
                now,
                PlatformEvent::HelloRead {
                    disc_id,
                    bytes: &bytes,
                },
            ),
            Inbound::ConnectFailed { disc_id, reason } => {
                e.handle_event(now, PlatformEvent::ConnectFailed { disc_id, reason });
            }
            Inbound::L2capPublished { psm } => {
                e.handle_event(now, PlatformEvent::L2capPublished { psm });
            }
            Inbound::AudioRoute(r) => e.handle_event(now, PlatformEvent::AudioRoute(r)),
            Inbound::AudioIo(s) => e.handle_event(now, PlatformEvent::AudioIo(s)),
            Inbound::Platform { mut sample, radio } => {
                if self.radio != Some(radio) {
                    self.radio = Some(radio);
                    e.handle_event(
                        now,
                        PlatformEvent::RadioState {
                            ble: radio.0,
                            aware: radio.1,
                        },
                    );
                }
                sample.audio = self.audio_counters(now);
                self.engine
                    .handle_event(now, PlatformEvent::Platform(sample));
            }
            Inbound::PromptPcm { key, pcm } => {
                // Media ignores `demo.*` keys (they never enter the prompt bank).
                e.handle_event(now, PlatformEvent::PromptPcm { key, pcm: &pcm });
                self.demo_line(key, &pcm);
            }
            Inbound::UiAttached(a) => e.set_ui_attached(a),
            #[cfg(test)]
            Inbound::Panic => panic!("mesh-ffi test panic on the engine thread"),
            Inbound::Command { .. }
            | Inbound::Start { .. }
            | Inbound::Stop { .. }
            | Inbound::Shutdown => {}
        }
        self.flush();
    }

    /// Audio counters for the 1 Hz platform sample, from the RT stats deltas.
    fn audio_counters(&mut self, now: Instant) -> AudioCounters {
        let Some(c) = self.claim.as_ref() else {
            return AudioCounters::default();
        };
        let st = c.shared().stats();
        let mut out = AudioCounters {
            capture_fps: None,
            render_fps: None,
            xruns: st.xruns().saturating_sub(self.rt_base.xruns()),
        };
        if let Some((prev, t)) = self.rt_last {
            let dt = now.since(t) as f64 / 1e6;
            if dt > 0.0 {
                out.capture_fps =
                    Some(st.captured_frames.saturating_sub(prev.captured_frames) as f64 / dt);
                out.render_fps =
                    Some(st.rendered_frames.saturating_sub(prev.rendered_frames) as f64 / dt);
            }
        }
        self.rt_last = Some((st, now));
        out
    }
}
