//! `UiEvent`: everything the engine pushes to the UI (DESIGN §3.7). Events carry i18n
//! keys plus params only, never English text.

use serde::{Deserialize, Serialize};

use crate::common::{AudioRouteKind, Params};
use crate::keys::{ErrorCode, NoticeKey};
use crate::lab::{LabLiveSample, LabSummary};
use crate::macros::string_enum;
use crate::snapshot::Snapshot;

string_enum! {
    /// Why a talk request was denied (busy tone).
    pub enum TalkDeniedReason {
        /// The simultaneous-talker cap is reached.
        Cap = "cap",
        /// A higher-ranked talker took the slot.
        Preempted = "preempted",
        /// The path is congested.
        Congestion = "congestion",
    }
}

string_enum! {
    /// VAD gate state.
    pub enum GateState {
        /// Voice detected.
        Open = "open",
        /// Silence.
        Closed = "closed",
    }
}

string_enum! {
    /// Phase of a running audio check.
    pub enum AudioCheckPhase {
        /// Playing the test tone.
        Tone = "tone",
        /// Recording.
        Record = "record",
        /// Playing the recording back.
        Echo = "echo",
        /// Live VOX meter.
        Meter = "meter",
    }
}

/// An engine → UI event (JSON discriminated by `type`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts-export", derive(ts_rs::TS), ts(export))]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum UiEvent {
    /// Full state (≤ 10 Hz, only when changed and the UI is attached).
    Snapshot {
        /// The snapshot.
        s: Snapshot,
    },
    /// The local talk request was denied.
    TalkDenied {
        /// Why.
        reason: TalkDeniedReason,
    },
    /// A notice (`notices.<key>` i18n entry).
    Notice {
        /// Notice key.
        key: NoticeKey,
        /// Interpolation params.
        params: Params,
    },
    /// The audio route changed.
    RouteChanged {
        /// New route.
        route: AudioRouteKind,
    },
    /// 20 Hz while an audio check runs.
    AudioCheckSample {
        /// Input level (0..1).
        level: f64,
        /// VAD speech probability (0..1).
        vad_p: f64,
        /// VAD gate.
        gate: GateState,
        /// Current route.
        route: AudioRouteKind,
        /// Hardware input sample rate, Hz (8k/16k/32k/48k).
        input_rate: u32,
        /// Check phase.
        phase: AudioCheckPhase,
    },
    /// 2 Hz while a Lab run is active.
    LabProgress {
        /// Run id.
        run_id: String,
        /// Seconds since the run started.
        elapsed_s: f64,
        /// Live numbers.
        sample: LabLiveSample,
    },
    /// A Lab run finished (stopped, timed out or aborted).
    LabFinished {
        /// Run id.
        run_id: String,
        /// On-device summary.
        summary: LabSummary,
        /// Written files (labrun/1 JSONL and summary CSV).
        files: Vec<String>,
    },
    /// An asynchronous error (`errors.<code>` i18n entry).
    Error {
        /// Error code.
        code: ErrorCode,
        /// Interpolation params.
        params: Params,
    },
}

string_enum! {
    /// The `type` discriminant of every [`UiEvent`].
    pub enum EventType {
        /// `snapshot`.
        Snapshot = "snapshot",
        /// `talkDenied`.
        TalkDenied = "talkDenied",
        /// `notice`.
        Notice = "notice",
        /// `routeChanged`.
        RouteChanged = "routeChanged",
        /// `audioCheckSample`.
        AudioCheckSample = "audioCheckSample",
        /// `labProgress`.
        LabProgress = "labProgress",
        /// `labFinished`.
        LabFinished = "labFinished",
        /// `error`.
        Error = "error",
    }
}

impl UiEvent {
    /// The `type` discriminant of this event.
    pub const fn event_type(&self) -> EventType {
        match self {
            UiEvent::Snapshot { .. } => EventType::Snapshot,
            UiEvent::TalkDenied { .. } => EventType::TalkDenied,
            UiEvent::Notice { .. } => EventType::Notice,
            UiEvent::RouteChanged { .. } => EventType::RouteChanged,
            UiEvent::AudioCheckSample { .. } => EventType::AudioCheckSample,
            UiEvent::LabProgress { .. } => EventType::LabProgress,
            UiEvent::LabFinished { .. } => EventType::LabFinished,
            UiEvent::Error { .. } => EventType::Error,
        }
    }
}

impl EventType {
    /// Part of the Phase 0 Lab subset of API 1.1 (frozen at Phase 0 exit).
    pub const fn in_lab_subset(self) -> bool {
        matches!(
            self,
            EventType::Snapshot
                | EventType::Notice
                | EventType::LabProgress
                | EventType::LabFinished
                | EventType::Error
        )
    }
}
