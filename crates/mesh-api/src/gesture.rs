//! The ★ capture-gated commands as data (DESIGN §3.2, §3.7, §11.1).
//!
//! `GESTURE_COMMANDS` is exported to `constants.ts` so the plugin (WP-17 web gating,
//! WP-19 native bridges), the typed client and the app lint rule all read one list. Rust
//! code uses [`crate::UiCommand::requires_gesture`]; a test proves both agree on every
//! shared fixture.

use serde::Serialize;

use crate::command::CommandType;

/// Comparison applied to one field of the command JSON.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum GestureOp {
    /// The field equals `value`.
    Equals,
    /// The field is present and differs from `value`.
    NotEquals,
}

/// Condition on the command JSON; the rule applies only when it holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GestureCondition {
    /// JSON path from the command root (e.g. `["params", "audio"]`).
    pub path: &'static [&'static str],
    /// Comparison.
    pub op: GestureOp,
    /// String value compared with the field.
    pub value: &'static str,
}

/// One ★ command type and the condition under which it is gesture-gated.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GestureRule {
    /// Command `type`.
    #[serde(rename = "type")]
    pub command: CommandType,
    /// `None`: always gated.
    pub when: Option<GestureCondition>,
}

/// goLive, resumeMic, labStart with audio ≠ off, audioCheck start (DESIGN §3.2).
pub const GESTURE_COMMANDS: &[GestureRule] = &[
    GestureRule {
        command: CommandType::GoLive,
        when: None,
    },
    GestureRule {
        command: CommandType::ResumeMic,
        when: None,
    },
    GestureRule {
        command: CommandType::LabStart,
        when: Some(GestureCondition {
            path: &["params", "audio"],
            op: GestureOp::NotEquals,
            value: "off",
        }),
    },
    GestureRule {
        command: CommandType::AudioCheck,
        when: Some(GestureCondition {
            path: &["op"],
            op: GestureOp::Equals,
            value: "start",
        }),
    },
];
