//! Host tests of the runtime and the UniFFI surface, with fake platform hooks, an
//! instrumented engine module and private RT planes (so tests can run in parallel).

mod api;
mod audio;
mod crash;
mod demo;
mod diag;
mod lab;

use std::sync::atomic::{AtomicBool, AtomicU32, Ordering::SeqCst};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant as StdInstant};

use mesh_api::Snapshot;
use mesh_core::{BLOCK_SAMPLES, CoreModule, Ctx, Engine, EngineConfig, Instant, PlatformEvent};

use crate::api::MeshEngine;
use crate::api::hooks::PlatformHooks;
use crate::api::outputs::*;
use crate::api::types::{FfiEngineConfig, FfiPlatform};
use crate::rt::RtShared;
use crate::runtime::Runtime;
use crate::runtime::config::BaseConfig;
use crate::runtime::worker::EngineFactory;

/// One recorded hook call.
#[derive(Debug, Clone, PartialEq)]
pub enum Call {
    Send(u32, Vec<u8>),
    Radio(FfiRadioCommand),
    Audio(FfiAudioCommand),
    Ui(String),
    Persist(String, Option<Vec<u8>>),
    Notify(FfiNotification),
    LabWrite(String, String),
    LabSummary(String, String),
    LabDelete(String),
    Prompts(Vec<FfiPromptRequest>),
    Log(FfiLogLevel, String),
}

type SendFn = dyn Fn(u32, &[u8]) -> FfiSendOutcome + Send + Sync;

/// Records every call; `link_send` answers through a replaceable closure.
pub struct FakeHooks {
    pub calls: Mutex<Vec<Call>>,
    pub on_send: Mutex<Arc<SendFn>>,
    /// `diag_write` batches (kept apart from `calls`: diagnostics run in every test).
    pub diag: Mutex<Vec<(String, u32, Vec<String>)>>,
}

impl FakeHooks {
    pub fn new() -> Arc<Self> {
        Arc::new(FakeHooks {
            calls: Mutex::new(Vec::new()),
            on_send: Mutex::new(Arc::new(|_, _| FfiSendOutcome::Accepted)),
            diag: Mutex::new(Vec::new()),
        })
    }

    pub fn set_send(&self, f: impl Fn(u32, &[u8]) -> FfiSendOutcome + Send + Sync + 'static) {
        *self.on_send.lock().unwrap() = Arc::new(f);
    }

    fn push(&self, c: Call) {
        self.calls.lock().unwrap().push(c);
    }

    pub fn calls(&self) -> Vec<Call> {
        self.calls.lock().unwrap().clone()
    }

    pub fn count(&self, f: impl Fn(&Call) -> bool) -> usize {
        self.calls.lock().unwrap().iter().filter(|c| f(c)).count()
    }

    /// UI events as JSON values.
    pub fn ui_events(&self) -> Vec<serde_json::Value> {
        self.calls()
            .into_iter()
            .filter_map(|c| match c {
                Call::Ui(j) => serde_json::from_str(&j).ok(),
                _ => None,
            })
            .collect()
    }
}

impl PlatformHooks for FakeHooks {
    fn link_send(&self, link: u32, bytes: Vec<u8>) -> FfiSendOutcome {
        let f = self.on_send.lock().unwrap().clone();
        let out = f(link, &bytes);
        self.push(Call::Send(link, bytes));
        out
    }
    fn radio(&self, cmd: FfiRadioCommand) {
        self.push(Call::Radio(cmd));
    }
    fn audio(&self, cmd: FfiAudioCommand) {
        self.push(Call::Audio(cmd));
    }
    fn ui_event(&self, json: String) {
        self.push(Call::Ui(json));
    }
    fn persist(&self, key: String, value: Option<Vec<u8>>) {
        self.push(Call::Persist(key, value));
    }
    fn notify(&self, n: FfiNotification) {
        self.push(Call::Notify(n));
    }
    fn lab_write(&self, run_id: String, line: String) {
        self.push(Call::LabWrite(run_id, line));
    }
    fn lab_write_summary(&self, run_id: String, line: String) {
        self.push(Call::LabSummary(run_id, line));
    }
    fn lab_delete(&self, run_id: String) {
        self.push(Call::LabDelete(run_id));
    }
    fn render_prompts(&self, reqs: Vec<FfiPromptRequest>) {
        self.push(Call::Prompts(reqs));
    }
    fn log(&self, level: FfiLogLevel, msg: String) {
        self.push(Call::Log(level, msg));
    }
    fn diag_write(&self, session_id: String, seg: u32, lines: Vec<String>) {
        self.diag.lock().unwrap().push((session_id, seg, lines));
    }
}

impl FakeHooks {
    /// Every diagnostics line written so far, in order.
    pub fn diag_lines(&self) -> Vec<String> {
        self.diag
            .lock()
            .unwrap()
            .iter()
            .flat_map(|(_, _, l)| l.iter().cloned())
            .collect()
    }
}

/// Observations of the instrumented module.
#[derive(Default)]
pub struct Probe {
    pub capture_blocks: AtomicU32,
    pub first_sample: AtomicU32,
    pub flags: Mutex<Vec<u32>>,
    pub events: Mutex<Vec<String>>,
    /// The next snapshot build panics (consumed by that panic).
    pub panic_armed: AtomicBool,
}

/// Test module, registered after the production modules (which answer every command,
/// `audioCheck` included): counts capture blocks, plays a constant value, records
/// platform events and RT flags, and panics while building one snapshot after
/// [`Probe::panic_armed`] is set (a panic inside `handle_command`).
pub struct TestModule {
    pub probe: Arc<Probe>,
}

/// Value the test module plays.
pub const PLAYOUT_VALUE: i16 = 1234;

impl CoreModule for TestModule {
    fn name(&self) -> &'static str {
        "test"
    }

    fn on_event(&mut self, _ctx: &mut Ctx, ev: &PlatformEvent<'_>) {
        let s = match ev {
            PlatformEvent::RadioState { ble, .. } => format!("radio:{ble:?}"),
            PlatformEvent::Platform(p) => format!("sample:{}", p.audio.xruns),
            PlatformEvent::PromptPcm { key, pcm } => format!("prompt:{key}:{}", pcm.len()),
            PlatformEvent::Writable { link } => format!("writable:{}", link.0),
            _ => return,
        };
        self.probe.events.lock().unwrap().push(s);
    }

    fn on_capture(&mut self, _ctx: &mut Ctx, pcm: &[i16; BLOCK_SAMPLES]) {
        if self.probe.capture_blocks.fetch_add(1, SeqCst) == 0 {
            self.probe.first_sample.store(pcm[0] as u16 as u32, SeqCst);
        }
    }

    fn on_playout(&mut self, _ctx: &mut Ctx, out: &mut [i16; BLOCK_SAMPLES]) {
        out.fill(PLAYOUT_VALUE);
    }

    fn on_rt_flags(&mut self, _ctx: &mut Ctx, flags: u32) {
        self.probe.flags.lock().unwrap().push(flags);
    }

    fn fill_snapshot(&self, _ctx: &Ctx, _s: &mut Snapshot) {
        if self.probe.panic_armed.swap(false, SeqCst) {
            panic!("instrumented module panic while building a snapshot");
        }
    }
}

pub fn ffi_config(seed: u8) -> FfiEngineConfig {
    FfiEngineConfig {
        platform: FfiPlatform::Ios,
        app_version: 7,
        rng_seed: vec![seed; 32],
        device_model: "iPhone15,2".into(),
        device_os: "iOS 27.0".into(),
        app_label: "0.1.0 (7)".into(),
        persisted: vec![],
    }
}

/// A test rig: engine handle, hooks, probe and private RT plane.
pub struct Rig {
    pub engine: Arc<MeshEngine>,
    pub hooks: Arc<FakeHooks>,
    pub probe: Arc<Probe>,
    pub rt: &'static RtShared,
}

impl Rig {
    pub fn new(seed: u8) -> Rig {
        let hooks = FakeHooks::new();
        let probe = Arc::new(Probe::default());
        let rt: &'static RtShared = Box::leak(Box::new(RtShared::new()));
        let p = probe.clone();
        let factory: Arc<EngineFactory> = Arc::new(move |cfg: EngineConfig, now: Instant| {
            let mut e = Engine::new(cfg, now);
            e.register_module(Box::new(TestModule { probe: p.clone() }));
            e
        });
        let (base, store) = BaseConfig::from_ffi(ffi_config(seed)).unwrap();
        let h: Arc<dyn PlatformHooks> = hooks.clone();
        let engine = MeshEngine::from_runtime(Runtime::new(base, store, h, rt, factory));
        Rig {
            engine,
            hooks,
            probe,
            rt,
        }
    }

    pub fn cmd(&self, json: &str) -> serde_json::Value {
        serde_json::from_str(&self.engine.ui_command(json.into())).unwrap()
    }
}

/// Polls `cond` every 2 ms for up to 10 s (generous for loaded CI machines).
pub fn wait_until(what: &str, mut cond: impl FnMut() -> bool) {
    let t0 = StdInstant::now();
    while !cond() {
        assert!(t0.elapsed() < Duration::from_secs(10), "timed out: {what}");
        std::thread::sleep(Duration::from_millis(2));
    }
}
