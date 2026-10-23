//! UI command/event/snapshot JSON contract (DESIGN §3.7, contract C1).
//!
//! serde types only: `#[serde(tag = "type", rename_all = "camelCase")]` for the
//! discriminated unions, i18n keys plus params instead of text, and the ★ gesture list.
//! With the `ts-export` feature every type also derives `ts_rs::TS`, and
//! `cargo xtask gen-ts` (`cargo test -p mesh-api --features ts-export export_bindings`)
//! writes `packages/engine-api/src/generated/`.
//!
//! Golden JSON fixtures shared with the TS client live in `fixtures/`; `tests/contract`
//! round-trips every one of them.

mod command;
mod common;
mod event;
mod gesture;
mod keys;
mod lab;
mod macros;
mod result;
mod snapshot;

#[cfg(all(test, feature = "ts-export"))]
mod ts_export;

pub use command::{
    AudioCheckMode, AudioCheckOp, CommandType, PttStyle, RideOp, RoutePreference, UiCommand,
    VoxSensitivity,
};
pub use common::{
    AudioRouteKind, Lang, MemberRole, NodeRef, ParamValue, Params, PeerPlatform, TalkMode, Vehicle,
};
pub use event::{AudioCheckPhase, EventType, GateState, TalkDeniedReason, UiEvent};
pub use gesture::{GESTURE_COMMANDS, GestureCondition, GestureOp, GestureRule};
pub use keys::{ErrorCode, NoticeKey, PromptKey};
pub use lab::{
    LabAnchor, LabAudio, LabBench, LabLinkRole, LabLiveSample, LabParams, LabPattern,
    LabPercentiles, LabRate, LabRunInfo, LabSummary, LabTransport, LabVerdict,
};
pub use result::{
    CommandError, CommandResult, GroupSummary, OkAccessLoss, OkDone, OkFalse, OkGroupId, OkGroups,
    OkInvite, OkInviteInfo, OkLabRun, OkLabRuns, OkTrue, ResultKind,
};
pub use snapshot::{
    AudioInfo, BluetoothState, DeviceInfo, EngineState, LadderLevel, LatencyInfo, LinkKind, Member,
    MemberFlag, MemberReady, MemberStatus, MeshInfo, MeshProfile, RidePhase, SessionInfo, Snapshot,
    TalkState, Talker, ThermalState, VoiceReach,
};

/// UI API version `major.minor` (Appendix A). 1.1 is frozen: the Lab subset at Phase 0
/// exit, the full contract in week 1 of Phase 1; any later change bumps the minor
/// version (and adds `frozen/api-<version>.digest`, checked by `cargo xtask gen-ts --check`).
pub const API_VERSION: &str = "1.1";
/// Major part of [`API_VERSION`]; the plugin refuses an engine with another major.
pub const API_VERSION_MAJOR: u8 = 1;
/// Minor part of [`API_VERSION`].
pub const API_VERSION_MINOR: u8 = 1;

/// [`API_VERSION`] packed as `major << 8 | minor` (UniFFI `api_version() -> u16`, §3.5).
pub const fn api_version_u16() -> u16 {
    ((API_VERSION_MAJOR as u16) << 8) | API_VERSION_MINOR as u16
}

/// Splits a packed [`api_version_u16`] value into `(major, minor)`.
pub const fn split_api_version(packed: u16) -> (u8, u8) {
    ((packed >> 8) as u8, (packed & 0xff) as u8)
}

/// Auto-hide delay of the invite QR in ms (Appendix A, §1.3).
pub const QR_HIDE_AFTER_MS: u32 = 60_000;

/// Maximum display-name length in UTF-8 bytes (§1.3 F0).
pub const DISPLAY_NAME_MAX_BYTES: usize = 24;

/// A string that is not one of the values of a string enum.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnknownValue {
    /// Rust name of the enum.
    pub type_name: &'static str,
    /// The rejected value.
    pub value: String,
}

impl std::fmt::Display for UnknownValue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "`{}` is not a valid {}", self.value, self.type_name)
    }
}

impl std::error::Error for UnknownValue {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn api_version_parts_agree() {
        assert_eq!(
            API_VERSION,
            format!("{API_VERSION_MAJOR}.{API_VERSION_MINOR}")
        );
        assert_eq!(api_version_u16(), 0x0101);
        assert_eq!(split_api_version(api_version_u16()), (1, 1));
    }

    #[test]
    fn string_enums_round_trip_through_from_str() {
        for code in ErrorCode::ALL {
            assert_eq!(code.as_str().parse::<ErrorCode>(), Ok(*code));
        }
        for key in PromptKey::ALL {
            assert_eq!(key.to_string().parse::<PromptKey>(), Ok(*key));
        }
        let err = "nope".parse::<NoticeKey>().expect_err("unknown key");
        assert_eq!(err.to_string(), "`nope` is not a valid NoticeKey");
    }

    #[test]
    fn required_keys_of_the_design_exist() {
        for code in ["useGestureCommand", "notForeground"] {
            assert!(code.parse::<ErrorCode>().is_ok(), "{code}");
        }
        for key in [
            "memberJoined",
            "memberLost",
            "memberBack",
            "alone",
            "partition",
            "captureLost",
            "captureRestored",
            "headsetLost",
            "onCall",
            "removed",
            "groupLocked",
            "rideStarted",
            "bluetoothOff",
            "bluetoothDenied",
            "linkLimitReached",
            "leaderChanged",
            "inviteLeaderMismatch",
            "liveActivityRenewed",
        ] {
            assert!(key.parse::<NoticeKey>().is_ok(), "{key}");
        }
        let prompts: Vec<_> = PromptKey::ALL.iter().map(|k| k.as_str()).collect();
        assert_eq!(prompts.len(), 14 + 8, "14 prompts + demo.line1..8 (§7.7)");
        assert!(prompts.contains(&"demo.line8"));
    }

    #[test]
    fn gesture_list_matches_the_design() {
        let types: Vec<_> = GESTURE_COMMANDS
            .iter()
            .map(|r| r.command.as_str())
            .collect();
        assert_eq!(types, ["goLive", "resumeMic", "labStart", "audioCheck"]);
        for rule in GESTURE_COMMANDS {
            assert!(rule.command.may_require_gesture());
        }
        let maybe = CommandType::ALL
            .iter()
            .filter(|t| t.may_require_gesture())
            .count();
        assert_eq!(maybe, GESTURE_COMMANDS.len());
    }
}
