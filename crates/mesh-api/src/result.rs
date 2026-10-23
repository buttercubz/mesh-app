//! `CommandResult`: `{ ok: true, ...payload } | { ok: false, code: ErrorCode }`
//! (DESIGN §3.7). The payload shape depends on the command ([`ResultKind`]); every
//! payload struct denies unknown fields so a JSON object matches exactly one variant.

use serde::de::{Deserializer, Error as _};
use serde::{Deserialize, Serialize, Serializer};

use crate::common::{MemberRole, NodeRef, Vehicle};
use crate::keys::ErrorCode;
use crate::lab::LabRunInfo;
use crate::macros::string_enum;

/// The literal `true` of `ok` in a successful result.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct OkTrue;

/// The literal `false` of `ok` in a failed result.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct OkFalse;

macro_rules! bool_literal {
    ($ty:ident, $value:literal) => {
        impl Serialize for $ty {
            fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
                s.serialize_bool($value)
            }
        }

        impl<'de> Deserialize<'de> for $ty {
            fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
                if bool::deserialize(d)? == $value {
                    Ok($ty)
                } else {
                    Err(D::Error::custom(concat!("expected `ok: ", $value, "`")))
                }
            }
        }
    };
}

bool_literal!(OkTrue, true);
bool_literal!(OkFalse, false);

string_enum! {
    /// Payload kind of a successful result, per command type.
    pub enum ResultKind {
        /// `{ ok: true }`.
        Done = "done",
        /// `{ ok: true, groupId }`.
        GroupId = "groupId",
        /// `parseInvite` answer.
        InviteInfo = "inviteInfo",
        /// `{ ok: true, groups }`.
        Groups = "groups",
        /// `{ ok: true, qrPng, hideAfterMs }`.
        Invite = "invite",
        /// `{ ok: true, unconfirmed }`.
        AccessLoss = "accessLoss",
        /// `{ ok: true, runId }`.
        LabRun = "labRun",
        /// `{ ok: true, runs }`.
        LabRuns = "labRuns",
    }
}

/// `{ ok: true }`: the command was applied.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "ts-export", derive(ts_rs::TS), ts(export))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OkDone {
    /// Always `true`.
    #[cfg_attr(feature = "ts-export", ts(type = "true"))]
    pub ok: OkTrue,
}

/// `createGroup` / `joinGroup` answer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts-export", derive(ts_rs::TS), ts(export))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OkGroupId {
    /// Always `true`.
    #[cfg_attr(feature = "ts-export", ts(type = "true"))]
    pub ok: OkTrue,
    /// Group id (32 hex chars).
    pub group_id: String,
}

/// `parseInvite` answer. Group fields are `null` when the invite does not parse.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts-export", derive(ts_rs::TS), ts(export))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OkInviteInfo {
    /// Always `true`.
    #[cfg_attr(feature = "ts-export", ts(type = "true"))]
    pub ok: OkTrue,
    /// The invite parses and its version is supported.
    pub valid: bool,
    /// Group name (user content).
    #[serde(deserialize_with = "crate::common::nullable")]
    pub group_name: Option<String>,
    /// Group id.
    #[serde(deserialize_with = "crate::common::nullable")]
    pub group_id: Option<String>,
    /// Key epoch carried by the invite.
    #[serde(deserialize_with = "crate::common::nullable")]
    pub epoch: Option<u32>,
    /// The invite is past its expiry.
    pub expired: bool,
    /// This device already belongs to the group.
    pub already_member: bool,
    /// The invite blob signature verifies against its leader key.
    pub leader_sig_ok: bool,
}

/// One stored group in `listGroups` (metadata only, never secrets).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts-export", derive(ts_rs::TS), ts(export))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GroupSummary {
    /// Group id.
    pub group_id: String,
    /// Group name (user content).
    pub name: String,
    /// This device's role.
    pub role: MemberRole,
    /// Vehicle profile used with this group.
    pub vehicle: Vehicle,
    /// Roster size known to this device.
    pub members: u8,
    /// Current key epoch.
    pub epoch: u32,
    /// The leader closed the group.
    pub closed: bool,
}

/// `listGroups` answer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts-export", derive(ts_rs::TS), ts(export))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OkGroups {
    /// Always `true`.
    #[cfg_attr(feature = "ts-export", ts(type = "true"))]
    pub ok: OkTrue,
    /// Stored groups.
    pub groups: Vec<GroupSummary>,
}

/// `getInvite` answer: the QR is rendered natively; the invite text never reaches JS.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts-export", derive(ts_rs::TS), ts(export))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OkInvite {
    /// Always `true`.
    #[cfg_attr(feature = "ts-export", ts(type = "true"))]
    pub ok: OkTrue,
    /// `data:image/png;base64,…`.
    pub qr_png: String,
    /// Auto-hide delay in ms (60 000, Appendix A).
    pub hide_after_ms: u32,
}

/// `previewAccessLoss` answer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts-export", derive(ts_rs::TS), ts(export))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OkAccessLoss {
    /// Always `true`.
    #[cfg_attr(feature = "ts-export", ts(type = "true"))]
    pub ok: OkTrue,
    /// Unconfirmed members who would lose access.
    pub unconfirmed: Vec<NodeRef>,
}

/// `labStart` answer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts-export", derive(ts_rs::TS), ts(export))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OkLabRun {
    /// Always `true`.
    #[cfg_attr(feature = "ts-export", ts(type = "true"))]
    pub ok: OkTrue,
    /// Id of the started run.
    pub run_id: String,
}

/// `labListRuns` answer.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts-export", derive(ts_rs::TS), ts(export))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OkLabRuns {
    /// Always `true`.
    #[cfg_attr(feature = "ts-export", ts(type = "true"))]
    pub ok: OkTrue,
    /// Stored runs, newest first.
    pub runs: Vec<LabRunInfo>,
}

/// `{ ok: false, code }`: the command was refused.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts-export", derive(ts_rs::TS), ts(export))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CommandError {
    /// Always `false`.
    #[cfg_attr(feature = "ts-export", ts(type = "false"))]
    pub ok: OkFalse,
    /// Why (an `errors.<code>` i18n key).
    pub code: ErrorCode,
}

/// Answer to a [`crate::UiCommand`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts-export", derive(ts_rs::TS), ts(export))]
#[serde(untagged)]
pub enum CommandResult {
    /// Refused.
    Error(CommandError),
    /// `createGroup` / `joinGroup`.
    GroupId(OkGroupId),
    /// `parseInvite`.
    InviteInfo(OkInviteInfo),
    /// `listGroups`.
    Groups(OkGroups),
    /// `getInvite`.
    Invite(OkInvite),
    /// `previewAccessLoss`.
    AccessLoss(OkAccessLoss),
    /// `labStart`.
    LabRun(OkLabRun),
    /// `labListRuns`.
    LabRuns(OkLabRuns),
    /// Every other command.
    Done(OkDone),
}

impl CommandResult {
    /// `{ ok: true }`.
    pub const fn done() -> Self {
        CommandResult::Done(OkDone { ok: OkTrue })
    }

    /// `{ ok: false, code }`.
    pub const fn error(code: ErrorCode) -> Self {
        CommandResult::Error(CommandError { ok: OkFalse, code })
    }

    /// `ok` of the JSON object.
    pub const fn is_ok(&self) -> bool {
        !matches!(self, CommandResult::Error(_))
    }

    /// The error code of a refused command.
    pub const fn error_code(&self) -> Option<ErrorCode> {
        match self {
            CommandResult::Error(e) => Some(e.code),
            _ => None,
        }
    }

    /// The payload kind of a successful result (`None` for an error).
    pub const fn kind(&self) -> Option<ResultKind> {
        Some(match self {
            CommandResult::Error(_) => return None,
            CommandResult::GroupId(_) => ResultKind::GroupId,
            CommandResult::InviteInfo(_) => ResultKind::InviteInfo,
            CommandResult::Groups(_) => ResultKind::Groups,
            CommandResult::Invite(_) => ResultKind::Invite,
            CommandResult::AccessLoss(_) => ResultKind::AccessLoss,
            CommandResult::LabRun(_) => ResultKind::LabRun,
            CommandResult::LabRuns(_) => ResultKind::LabRuns,
            CommandResult::Done(_) => ResultKind::Done,
        })
    }
}
