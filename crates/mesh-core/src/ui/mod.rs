//! UI side of the engine (DESIGN §3.3, owned by WP-10A): command dispatch, the
//! snapshot builder (dirty flag, ≤ 10 Hz, only when changed, suppressed while the UI is
//! detached), the platform-event base handling and the device state the snapshot and
//! the Lab samples read.

mod device;
mod dispatch;
mod events;
mod snapshot;

pub use device::DeviceState;
pub(crate) use dispatch::dispatch;
pub(crate) use events::on_platform_event;
pub use snapshot::build_snapshot;

use mesh_api::{EngineState, Snapshot};

use crate::Instant;
use crate::config::Limits;

/// Snapshot emission state.
#[derive(Debug, Clone)]
pub struct UiState {
    attached: bool,
    dirty: bool,
    force: bool,
    last_emit: Option<Instant>,
    last: Option<Snapshot>,
    engine: EngineState,
    emitted: u64,
}

impl Default for UiState {
    fn default() -> Self {
        Self::new()
    }
}

impl UiState {
    pub fn new() -> Self {
        UiState {
            attached: true,
            dirty: true,
            force: false,
            last_emit: None,
            last: None,
            engine: EngineState::Idle,
            emitted: 0,
        }
    }

    /// Something the snapshot shows may have changed.
    pub fn mark_dirty(&mut self) {
        self.dirty = true;
    }

    /// Attach (foreground WebView) or detach the UI. Attaching forces a snapshot.
    pub fn set_attached(&mut self, attached: bool) {
        if attached && !self.attached {
            self.force = true;
        }
        self.attached = attached;
    }

    pub fn attached(&self) -> bool {
        self.attached
    }

    /// `requestSnapshot`: the next emission happens even if nothing changed.
    pub fn request(&mut self) {
        self.force = true;
    }

    /// Engine state shown in the snapshot.
    pub fn set_engine(&mut self, s: EngineState) {
        if self.engine != s {
            self.engine = s;
            self.dirty = true;
        }
    }

    pub fn engine(&self) -> EngineState {
        self.engine
    }

    /// Snapshots emitted so far (diagnostics, tests).
    pub fn emitted(&self) -> u64 {
        self.emitted
    }

    fn earliest(&self, limits: &Limits) -> Instant {
        self.last_emit.map_or(Instant::ZERO, |t| {
            t.plus_us(limits.snapshot_min_interval_us)
        })
    }

    /// `true` when a snapshot should be built now.
    pub fn snapshot_due(&self, now: Instant, limits: &Limits) -> bool {
        self.attached && (self.dirty || self.force) && now >= self.earliest(limits)
    }

    /// When a pending snapshot becomes due (only while attached and dirty).
    pub fn next_deadline(&self, limits: &Limits) -> Option<Instant> {
        (self.attached && (self.dirty || self.force)).then(|| self.earliest(limits))
    }

    /// Offers a freshly built snapshot; returns it when it must be emitted (changed
    /// since the last emission, or forced).
    pub fn offer(&mut self, now: Instant, s: Snapshot) -> Option<Snapshot> {
        self.dirty = false;
        if !self.force && self.last.as_ref() == Some(&s) {
            return None;
        }
        self.force = false;
        self.last_emit = Some(now);
        self.last = Some(s.clone());
        self.emitted += 1;
        Some(s)
    }

    /// The last emitted snapshot.
    pub fn last(&self) -> Option<&Snapshot> {
        self.last.as_ref()
    }
}
