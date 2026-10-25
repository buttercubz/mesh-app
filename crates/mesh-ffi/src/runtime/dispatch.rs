//! Engine outputs → `PlatformHooks` (runs on the engine thread only).

use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex};

use mesh_api::{EngineState, ErrorCode, Params, PromptKey, Snapshot, UiEvent};
use mesh_core::media::demo::DEMO_LINES;
use mesh_core::{
    AudioCommand, DiscoveryMode, Effect, Engine, LabFile, LinkId, PlatformEvent, RadioCommand,
    ScanIntensity, Transmit,
};
use mesh_sim::demo::DemoWorld;

use super::config::PersistStore;
use super::diag::DiagHub;
use super::os::Clock;
use crate::api::hooks::PlatformHooks;
use crate::api::outputs::{
    FfiAudioCommand, FfiLogLevel, FfiNotification, FfiNotificationCategory, FfiRadioCommand,
    FfiSendOutcome,
};

/// Safety valve: at most this many transmits + effects per flush (a core bug must not
/// spin the engine thread forever).
const MAX_OUTPUTS_PER_FLUSH: usize = 8192;

/// Forwards transmits and effects to the platform and remembers what the crash path
/// needs (open links, last snapshot) and what the runtime watches (`CloseSession`).
pub struct Dispatcher {
    hooks: Arc<dyn PlatformHooks>,
    store: Arc<Mutex<PersistStore>>,
    /// Snapshots are rewritten to `engine: crashed` until the next successful start.
    pub crashed_view: bool,
    last_snapshot: Option<Snapshot>,
    close_session_seen: bool,
    open_links: BTreeSet<u32>,
    /// `demo.*` lines asked from platform TTS while the Demo world runs, not rendered yet.
    demo_pending: BTreeSet<PromptKey>,
    /// The diagnostics recorder (log counts; `Effect::Diag` records).
    diag: Option<Arc<DiagHub>>,
    /// Wi-Fi Aware data-path PMKs per peer `node_id`, handed out once through
    /// `MeshEngine::take_aware_pmk` (never through `radio`, never logged).
    aware_pmks: Option<AwarePmks>,
}

/// Shared slot of pending Aware PMKs (engine thread writes, platform takes).
pub type AwarePmks = Arc<Mutex<BTreeMap<u16, [u8; 32]>>>;

impl Dispatcher {
    pub fn new(hooks: Arc<dyn PlatformHooks>, store: Arc<Mutex<PersistStore>>) -> Self {
        Dispatcher {
            hooks,
            store,
            crashed_view: false,
            last_snapshot: None,
            close_session_seen: false,
            open_links: BTreeSet::new(),
            demo_pending: BTreeSet::new(),
            diag: None,
            aware_pmks: None,
        }
    }

    /// Attaches the Aware PMK slot (`take_aware_pmk`).
    pub fn set_aware_pmks(&mut self, slot: AwarePmks) {
        self.aware_pmks = Some(slot);
    }

    /// Stores the PMK of op 2 before the command reaches the platform; drops it again
    /// when the attempt is abandoned (3) or retired (5).
    fn stash_aware_pmk(&self, cmd: &RadioCommand) {
        let (RadioCommand::AwareUpgrade { peer, op, pmk }, Some(slot)) = (cmd, &self.aware_pmks)
        else {
            return;
        };
        let Ok(mut m) = slot.lock() else { return };
        match (op, pmk) {
            (2, Some(p)) => {
                m.insert(*peer, p.0);
            }
            (3 | 5, _) => {
                m.remove(peer);
            }
            _ => {}
        }
    }

    /// Attaches the diagnostics recorder.
    pub fn set_diag(&mut self, diag: Arc<DiagHub>) {
        self.diag = Some(diag);
    }

    /// Forwards a core diagnostics record to the recorder. mesh-core's
    /// `Effect::Diag(r)` arm calls this (stage 2 of the recorder).
    pub fn diag_record(&self, r: mesh_lab::ridelog::DiagRecord) {
        if let Some(d) = &self.diag {
            d.record_now(r);
        }
    }

    /// The platform hooks.
    pub fn hooks(&self) -> &dyn PlatformHooks {
        self.hooks.as_ref()
    }

    /// A link came up (tracked for the crash path).
    pub fn link_up(&mut self, link: LinkId) {
        self.open_links.insert(link.0);
    }

    /// A link went down.
    pub fn link_down(&mut self, link: LinkId) {
        self.open_links.remove(&link.0);
    }

    /// Links the platform reported up and that are not closed yet.
    pub fn open_links(&self) -> Vec<u32> {
        self.open_links.iter().copied().collect()
    }

    /// Whether an `AudioCommand::CloseSession` went out since the last call.
    pub fn take_close_session(&mut self) -> bool {
        std::mem::take(&mut self.close_session_seen)
    }

    /// A rendered `demo.*` line arrived: `true` once every requested line is in.
    pub fn demo_line_rendered(&mut self, key: PromptKey) -> bool {
        self.demo_pending.remove(&key);
        self.demo_pending.is_empty()
    }

    /// Forgets the pending `demo.*` renders (the Demo ended).
    pub fn clear_demo(&mut self) {
        self.demo_pending.clear();
    }

    /// Drains every transmit and effect of `engine` to the platform. While the Demo
    /// world runs, units on its virtual links go to the bots and radio commands are
    /// absorbed by the world (no radio in the Demo, DESIGN §3.4).
    pub fn flush(&mut self, engine: &mut Engine, clock: &Clock, mut demo: Option<&mut DemoWorld>) {
        let mut budget = MAX_OUTPUTS_PER_FLUSH;
        loop {
            let mut progressed = false;
            while budget > 0 {
                let Some(t) = engine.poll_transmit(clock.now()) else {
                    break;
                };
                budget -= 1;
                progressed = true;
                if let Some(d) = demo.as_deref_mut()
                    && d.on_local_transmit(clock.now(), &t)
                {
                    continue;
                }
                self.transmit(engine, clock, t);
            }
            while budget > 0 {
                let Some(e) = engine.poll_effect() else {
                    break;
                };
                budget -= 1;
                progressed = true;
                match (demo.as_deref_mut(), e) {
                    (Some(d), Effect::Radio(cmd)) => {
                        d.on_local_radio(clock.now(), &cmd);
                    }
                    (Some(_), Effect::RenderPrompts(reqs)) => {
                        self.demo_pending.extend(
                            reqs.iter()
                                .map(|r| r.key)
                                .filter(|k| DEMO_LINES.contains(k)),
                        );
                        self.effect(Effect::RenderPrompts(reqs));
                    }
                    (_, e) => self.effect(e),
                }
            }
            if !progressed {
                return;
            }
            if budget == 0 {
                self.hooks.log(
                    FfiLogLevel::Error,
                    "mesh-ffi: output budget exhausted in one flush".into(),
                );
                return;
            }
        }
    }

    fn transmit(&mut self, engine: &mut Engine, clock: &Clock, t: Transmit) {
        let Transmit { link, bytes } = t;
        match self.hooks.link_send(link.0, bytes.clone()) {
            FfiSendOutcome::Accepted => {}
            FfiSendOutcome::WouldBlock => engine.handle_event(
                clock.now(),
                PlatformEvent::WouldBlock { link, unit: &bytes },
            ),
            FfiSendOutcome::NoLink => self.hooks.log(
                FfiLogLevel::Debug,
                format!("mesh-ffi: link_send on unknown link {}", link.0),
            ),
        }
    }

    /// Forwards one effect.
    pub fn effect(&mut self, e: Effect) {
        let h = self.hooks.as_ref();
        match e {
            Effect::Radio(cmd) => {
                if let RadioCommand::Close { link, .. } = &cmd {
                    self.open_links.remove(&link.0);
                }
                self.stash_aware_pmk(&cmd);
                h.radio(cmd.into());
            }
            Effect::Audio(cmd) => {
                if cmd == AudioCommand::CloseSession {
                    self.close_session_seen = true;
                }
                h.audio(cmd.into());
            }
            Effect::Ui(ev) => self.ui(ev),
            Effect::Persist { key, value } => {
                if let Ok(mut s) = self.store.lock() {
                    s.apply(&key, value.clone());
                }
                h.persist(key, value);
            }
            Effect::Notify(n) => h.notify(n.into()),
            Effect::LabWrite { run_id, file, line } => match file {
                LabFile::Jsonl => h.lab_write(run_id, line),
                LabFile::SummaryCsv => h.lab_write_summary(run_id, line),
            },
            Effect::LabDelete { run_id } => h.lab_delete(run_id),
            Effect::RenderPrompts(reqs) => {
                h.render_prompts(reqs.into_iter().map(Into::into).collect());
            }
            Effect::Log { level, msg } => {
                let level = level.into();
                if let Some(d) = &self.diag {
                    d.count_log(level);
                }
                h.log(level, msg);
            }
            Effect::Diag(r) => self.diag_record(r),
        }
    }

    /// Serialises a UI event for the plugin (snapshots patched while crashed).
    pub fn ui(&mut self, ev: UiEvent) {
        let ev = match ev {
            UiEvent::Snapshot { mut s } => {
                if self.crashed_view {
                    s.engine = EngineState::Crashed;
                }
                self.last_snapshot = Some(s.clone());
                UiEvent::Snapshot { s }
            }
            other => other,
        };
        match serde_json::to_string(&ev) {
            Ok(json) => self.hooks.ui_event(json),
            Err(e) => self.hooks.log(
                FfiLogLevel::Error,
                format!("mesh-ffi: UI event not serialisable: {e}"),
            ),
        }
    }

    /// The engine thread panicked: tell the UI and the user, and stop radios and audio
    /// directly (the engine can no longer be trusted to emit its own stop effects).
    pub fn report_crash(&mut self, what: &str) {
        self.crashed_view = true;
        self.hooks.log(
            FfiLogLevel::Error,
            format!("mesh-ffi: engine crashed: {what}"),
        );
        for link in std::mem::take(&mut self.open_links) {
            self.hooks
                .radio(FfiRadioCommand::Close { link, reason: None });
        }
        self.hooks.radio(
            RadioCommand::SetDiscovery {
                mode: DiscoveryMode::Off,
                adv: None,
                scan: ScanIntensity::LowPower,
            }
            .into(),
        );
        self.hooks.audio(FfiAudioCommand::CloseSession);
        self.ui(UiEvent::Error {
            code: ErrorCode::EngineCrashed,
            params: Params::new(),
        });
        if let Some(mut s) = self.last_snapshot.take() {
            s.talkers.clear();
            s.session = None;
            self.ui(UiEvent::Snapshot { s });
        }
        self.hooks.notify(FfiNotification {
            category: FfiNotificationCategory::Error,
            key: ErrorCode::EngineCrashed.as_str().to_owned(),
            params_json: "{}".into(),
        });
    }
}
