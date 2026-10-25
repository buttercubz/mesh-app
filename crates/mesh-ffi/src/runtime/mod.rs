//! The mesh-ffi runtime (DESIGN §3.4): owns the engine thread, the bounded inbound
//! channel and the RT plane claim. `MeshEngine` (UniFFI) is a thin shell over it.
//!
//! Threads: the UI thread calls `ui_command` (one-shot reply through the channel), radio
//! threads post link events (dropped while no session runs), the engine thread owns the
//! `Engine` (see `worker`), and the OS audio threads only touch the RT rings.

pub mod config;
pub mod diag;
pub mod dispatch;
pub mod inbound;
pub mod os;
pub mod worker;

use std::sync::atomic::{AtomicBool, AtomicU32, Ordering::SeqCst};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

use crossbeam_channel::{Receiver, RecvTimeoutError, Sender, bounded};
use mesh_api::{CommandResult, ErrorCode, UiCommand};
use mesh_core::{Engine, SessionStart};

use self::config::{BaseConfig, PersistStore};
use self::diag::DiagHub;
use self::inbound::{INBOUND_CAPACITY, Inbound};
use self::os::Clock;
use self::worker::EngineFactory;
use crate::api::hooks::PlatformHooks;
use crate::api::outputs::FfiLogLevel;
use crate::api::types::{FfiError, FfiRuntimeStatus};
use crate::rt::RtShared;

/// How long a control call waits for the engine thread before giving up.
pub const REPLY_TIMEOUT: Duration = Duration::from_secs(3);
/// Poll slice while waiting for a reply (checks that the engine thread is alive).
const REPLY_SLICE: Duration = Duration::from_millis(25);
/// How long a radio thread may block on a full inbound channel.
const POST_TIMEOUT: Duration = Duration::from_millis(100);

/// State shared by the runtime handle and its engine thread.
pub struct Shared {
    pub hooks: Arc<dyn PlatformHooks>,
    pub store: Arc<Mutex<PersistStore>>,
    pub base: BaseConfig,
    pub rt: &'static RtShared,
    pub clock: Clock,
    /// A group session, Lab mode or the Demo ride runs (link events are accepted).
    pub active: AtomicBool,
    /// The core is still closing links after `stop` (ordered shutdown): link events
    /// are accepted until every link is closed or the drain bound expires.
    pub draining: AtomicBool,
    /// An audio check without a session runs.
    pub checking: AtomicBool,
    /// The last engine thread panicked and no session started since.
    pub crashed: AtomicBool,
    pub ui_attached: AtomicBool,
    /// Engines built so far (seeds later engines differently).
    pub generation: AtomicU32,
    /// Inbound messages dropped because the channel stayed full.
    pub dropped: AtomicU32,
    /// The ridelog/1 diagnostics recorder.
    pub diag: Arc<DiagHub>,
    /// Pending Wi-Fi Aware PMKs per peer (`take_aware_pmk`).
    pub aware_pmks: dispatch::AwarePmks,
}

/// The runtime handle.
pub struct Runtime {
    shared: Arc<Shared>,
    tx: Sender<Inbound>,
    rx: Receiver<Inbound>,
    worker: Mutex<Option<JoinHandle<()>>>,
    factory: Arc<EngineFactory>,
}

impl Runtime {
    /// A runtime with no thread yet (cheap). `rt` is the RT plane it claims while a
    /// session runs (the global `RT` in production).
    pub fn new(
        base: BaseConfig,
        store: PersistStore,
        hooks: Arc<dyn PlatformHooks>,
        rt: &'static RtShared,
        factory: Arc<EngineFactory>,
    ) -> Self {
        let (tx, rx) = bounded(INBOUND_CAPACITY);
        let clock = Clock::new();
        let diag = Arc::new(DiagHub::new(hooks.clone(), base.diag_device(), clock));
        Runtime {
            shared: Arc::new(Shared {
                hooks,
                store: Arc::new(Mutex::new(store)),
                base,
                rt,
                clock,
                active: AtomicBool::new(false),
                draining: AtomicBool::new(false),
                checking: AtomicBool::new(false),
                crashed: AtomicBool::new(false),
                ui_attached: AtomicBool::new(false),
                generation: AtomicU32::new(0),
                dropped: AtomicU32::new(0),
                diag,
                aware_pmks: Default::default(),
            }),
            tx,
            rx,
            worker: Mutex::new(None),
            factory,
        }
    }

    /// The production engine factory (`Engine::new` with the full module list).
    pub fn default_factory() -> Arc<EngineFactory> {
        Arc::new(Engine::new)
    }

    /// Shared state (tests and diagnostics).
    pub fn shared(&self) -> &Shared {
        &self.shared
    }

    /// Spawns the engine thread if it is not running (first use, or after a crash).
    fn ensure_worker(&self) -> Result<(), FfiError> {
        let mut slot = self.worker.lock().map_err(|_| FfiError::Crashed)?;
        if slot.as_ref().is_some_and(|h| h.is_finished())
            && let Some(h) = slot.take()
        {
            let _ = h.join();
        }
        if slot.is_none() {
            let (shared, rx, factory) =
                (self.shared.clone(), self.rx.clone(), self.factory.clone());
            let h = std::thread::Builder::new()
                .name("mesh-engine".into())
                .spawn(move || worker::run(shared, rx, factory))
                .map_err(|e| FfiError::ThreadSpawn {
                    detail: e.to_string(),
                })?;
            *slot = Some(h);
        }
        Ok(())
    }

    fn worker_finished(&self) -> bool {
        self.worker
            .lock()
            .map(|s| s.as_ref().is_none_or(|h| h.is_finished()))
            .unwrap_or(true)
    }

    /// Sends a control request and waits for its reply. `None` when the engine thread
    /// died with the request (crash) or did not answer in time.
    fn request<T>(&self, make: impl FnOnce(Sender<T>) -> Inbound) -> Result<T, ReplyError> {
        self.ensure_worker().map_err(|_| ReplyError::Crashed)?;
        let (rtx, rrx) = bounded(1);
        self.tx
            .send_timeout(make(rtx), REPLY_TIMEOUT)
            .map_err(|_| ReplyError::Timeout)?;
        let mut waited = Duration::ZERO;
        loop {
            match rrx.recv_timeout(REPLY_SLICE) {
                Ok(v) => return Ok(v),
                Err(RecvTimeoutError::Disconnected) => return Err(ReplyError::Crashed),
                Err(RecvTimeoutError::Timeout) => {
                    waited += REPLY_SLICE;
                    if self.worker_finished() {
                        // The thread exited before taking the request: respawn so the
                        // queued request is served by the next engine.
                        self.ensure_worker().map_err(|_| ReplyError::Crashed)?;
                    }
                    if waited >= REPLY_TIMEOUT {
                        return Err(ReplyError::Timeout);
                    }
                }
            }
        }
    }

    /// `ui_command(json)`: parses, runs on the engine thread, answers JSON.
    pub fn ui_command(&self, json: &str) -> String {
        let r = match serde_json::from_str::<UiCommand>(json) {
            Err(_) => CommandResult::error(ErrorCode::InvalidCommand),
            Ok(cmd) => match self.request(|reply| Inbound::Command { cmd, reply }) {
                Ok(r) => r,
                Err(ReplyError::Crashed) => CommandResult::error(ErrorCode::EngineCrashed),
                Err(ReplyError::Timeout) => CommandResult::error(ErrorCode::Internal),
            },
        };
        serde_json::to_string(&r).unwrap_or_else(|_| r#"{"ok":false,"code":"internal"}"#.to_owned())
    }

    /// Starts a group session, Lab mode or the Demo ride (the runtime attaches the
    /// mesh-sim demo world, DESIGN §3.4) on the engine thread.
    pub fn start(&self, start: SessionStart) -> Result<(), FfiError> {
        match self.request(|reply| Inbound::Start { start, reply }) {
            Ok(r) => r,
            Err(ReplyError::Crashed) => Err(FfiError::Crashed),
            Err(ReplyError::Timeout) => Err(FfiError::ThreadSpawn {
                detail: "engine thread did not answer".into(),
            }),
        }
    }

    /// Stops the session (radios, audio) and returns to Idle. No-op without a thread.
    pub fn stop(&self) {
        if self.worker_finished() {
            self.shared.active.store(false, SeqCst);
            self.shared.draining.store(false, SeqCst);
            return;
        }
        let _ = self.request(|reply| Inbound::Stop { reply });
    }

    /// Posts a platform event. Radio events are dropped while no session runs and no
    /// link is still closing after `stop`.
    pub fn post(&self, m: Inbound) {
        if m.is_session_only()
            && !self.shared.active.load(SeqCst)
            && !self.shared.draining.load(SeqCst)
        {
            return;
        }
        if self.ensure_worker().is_err() {
            return;
        }
        if self.tx.send_timeout(m, POST_TIMEOUT).is_err() {
            let n = self.shared.dropped.fetch_add(1, SeqCst);
            if n.is_multiple_of(256) {
                self.shared.hooks.log(
                    FfiLogLevel::Warn,
                    format!("mesh-ffi: inbound channel full, {} events dropped", n + 1),
                );
            }
        }
    }

    /// Gates snapshot emission.
    pub fn set_ui_attached(&self, attached: bool) {
        self.shared.ui_attached.store(attached, SeqCst);
        if !self.worker_finished() {
            self.post(Inbound::UiAttached(attached));
        }
    }

    /// Current runtime state.
    pub fn status(&self) -> FfiRuntimeStatus {
        let s = &self.shared;
        if s.active.load(SeqCst) {
            FfiRuntimeStatus::Running
        } else if s.checking.load(SeqCst) {
            FfiRuntimeStatus::AudioCheck
        } else if s.crashed.load(SeqCst) {
            FfiRuntimeStatus::Crashed
        } else {
            FfiRuntimeStatus::Idle
        }
    }

    /// Test hook: makes the engine thread panic.
    #[cfg(test)]
    pub fn inject_panic(&self) {
        let _ = self.ensure_worker();
        let _ = self.tx.send(Inbound::Panic);
    }
}

enum ReplyError {
    Crashed,
    Timeout,
}

impl Drop for Runtime {
    fn drop(&mut self) {
        let handle = self.worker.lock().ok().and_then(|mut s| s.take());
        let Some(h) = handle else {
            return;
        };
        let _ = self.tx.send_timeout(Inbound::Shutdown, REPLY_TIMEOUT);
        // Never join from the engine thread itself (a hook dropping the last handle).
        if h.thread().id() != std::thread::current().id() {
            let _ = h.join();
        }
    }
}
