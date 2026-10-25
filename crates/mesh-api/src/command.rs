//! `UiCommand`: every command the UI sends to the engine (DESIGN §3.7, §1.3).

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::common::{Lang, TalkMode, Vehicle};
use crate::keys::PromptKey;
use crate::lab::{LabAudio, LabBench, LabParams};
use crate::macros::string_enum;
use crate::result::ResultKind;

string_enum! {
    /// Audio-check mode (onboarding headset check, Echo test, Settings VOX meter).
    pub enum AudioCheckMode {
        /// Play a tone, record 2 s, show level and route.
        Tone = "tone",
        /// Record and play back (Echo test).
        Echo = "echo",
        /// Live VAD probability and gate state.
        VoxMeter = "voxMeter",
    }
}

string_enum! {
    /// PTT button behaviour.
    pub enum PttStyle {
        /// Transmit while held.
        Hold = "hold",
        /// Tap to latch, tap to release (gloves).
        Latch = "latch",
    }
}

string_enum! {
    /// VOX sensitivity (DESIGN §7.3).
    pub enum VoxSensitivity {
        /// Least sensitive.
        Low = "low",
        /// Default.
        Medium = "medium",
        /// Most sensitive.
        High = "high",
    }
}

string_enum! {
    /// Ride control operation (leader floods RIDE_CTL).
    pub enum RideOp {
        /// Start the ride.
        Start = "start",
        /// End the ride.
        End = "end",
    }
}

string_enum! {
    /// Preferred output route.
    pub enum RoutePreference {
        /// Engine policy per vehicle profile (§1.4).
        Auto = "auto",
        /// Prefer a Bluetooth/wired headset.
        Headset = "headset",
        /// Prefer the loudspeaker.
        Speaker = "speaker",
    }
}

/// `audioCheck` operation; only `start` is a ★ gesture command.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts-export", derive(ts_rs::TS), ts(export))]
#[serde(
    tag = "op",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum AudioCheckOp {
    /// Start a check in `mode` (★ gesture command).
    Start {
        /// Check mode.
        mode: AudioCheckMode,
    },
    /// Stop the check and close the temporary session.
    Stop {},
}

/// A UI command (JSON discriminated by `type`). Each command answers with a
/// [`crate::CommandResult`] whose payload kind is [`UiCommand::result_kind`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts-export", derive(ts_rs::TS), ts(export))]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum UiCommand {
    /// Create a group; this device becomes its leader. → `{ groupId }`.
    CreateGroup {
        /// Group name (user content).
        name: String,
        /// Vehicle profile of the creator.
        vehicle: Vehicle,
    },
    /// Validate an invite URL without joining. → invite info.
    ParseInvite {
        /// `convlinkvoice://join?v=1#<invite>`.
        url: String,
    },
    /// Join the group of an invite. → `{ groupId }`.
    JoinGroup {
        /// Invite URL.
        url: String,
        /// Display name to announce (≤ 24 UTF-8 bytes).
        display_name: String,
    },
    /// List stored groups. → `{ groups }`.
    ListGroups {},
    /// Delete a stored group and its secrets.
    ForgetGroup {
        /// Group id (32 hex chars).
        group_id: String,
    },
    /// Render the invite QR natively. → `{ qrPng, hideAfterMs }`.
    GetInvite {
        /// Group id.
        group_id: String,
    },
    /// ★ Start capture (muted until the ride starts).
    GoLive {},
    /// ★ Restart capture after a loss (ladder L2/L3 → L0).
    ResumeMic {},
    /// Headset/echo/VOX check; `op: "start"` is ★.
    AudioCheck(AudioCheckOp),
    /// Change the talk mode.
    SetTalkMode {
        /// New mode.
        mode: TalkMode,
    },
    /// PTT press/release (also mirrored to `mesh_rt_set_flags`).
    Ptt {
        /// Button down.
        down: bool,
    },
    /// PTT style.
    SetPttStyle {
        /// New style.
        style: PttStyle,
    },
    /// VOX sensitivity.
    SetVoxSensitivity {
        /// New level.
        level: VoxSensitivity,
    },
    /// Start or end the ride.
    RideControl {
        /// Operation.
        op: RideOp,
    },
    /// Change the display name.
    SetDisplayName {
        /// New name (≤ 24 UTF-8 bytes).
        name: String,
    },
    /// Change the vehicle profile.
    SetVehicle {
        /// New profile.
        vehicle: Vehicle,
    },
    /// Output route preference.
    SetRoutePreference {
        /// New preference.
        pref: RoutePreference,
    },
    /// Share my distance (location anchor + POSITION).
    SetShareDistance {
        /// On/off.
        enabled: bool,
    },
    /// Spoken-prompt templates from the `prompts` catalog (sent on every `startEngine`).
    SetPrompts {
        /// Template language.
        lang: Lang,
        /// Prompts on/off (earcons always play).
        enabled: bool,
        /// Template per key (i18next interpolation syntax).
        templates: BTreeMap<PromptKey, String>,
    },
    /// Allow transmitting with only the built-in mic.
    SetBuiltinMicTx {
        /// Allowed.
        allowed: bool,
    },
    /// Leader: remove a member (signed ROSTER deny + REKEY, Phase 2).
    RemoveMember {
        /// Member node id.
        node_id: u16,
    },
    /// Leader: rekey without removing anyone (Phase 2).
    LockGroup {},
    /// Leader: close the group (signed CLOSE).
    CloseGroup {},
    /// Leader: who would lose access on lock/remove. → `{ unconfirmed }`.
    PreviewAccessLoss {},
    /// Leader: hand leadership to a present member (signed GROUP_CTL).
    TransferLeader {
        /// New leader node id.
        node_id: u16,
    },
    /// Leader: invite expiry; `null` = never.
    SetInviteExpiry {
        /// Hours, or `null`.
        #[serde(deserialize_with = "crate::common::nullable")]
        hours: Option<u32>,
    },
    /// Ask for a full snapshot event (foreground return).
    RequestSnapshot {},
    /// Start a Lab run; ★ when `params.audio` is not `off`. → `{ runId }`.
    LabStart {
        /// Bench.
        bench: LabBench,
        /// Parameters (audio, anchor and gps mandatory).
        params: LabParams,
    },
    /// Stop the running Lab run.
    LabStop {},
    /// Add a mark line to the running run (passengers or stops only).
    LabMark {
        /// Mark label (user content).
        label: String,
    },
    /// List stored runs. → `{ runs }`.
    LabListRuns {},
    /// Delete a stored run and its files.
    LabDeleteRun {
        /// Run id.
        run_id: String,
    },
    /// Prepare open channel 1-9 (no invite, no leader, nothing persisted); the UI then
    /// calls `startEngine({ groupId })` with the returned id. → `{ groupId }`.
    JoinOpenChannel {
        /// Channel number 1-9.
        channel: u8,
        /// Display name to announce (optional, may be empty; ≤ 24 UTF-8 bytes).
        display_name: String,
    },
    /// Locally mute (or unmute) a member's voice. Never sent on air.
    SetMemberMuted {
        /// Member node id.
        node_id: u16,
        /// Muted.
        muted: bool,
    },
    /// Locally block (or unblock) a member: muted and refused as a direct neighbor.
    /// Never sent on air.
    SetMemberBlocked {
        /// Member node id.
        node_id: u16,
        /// Blocked.
        blocked: bool,
    },
}

string_enum! {
    /// The `type` discriminant of every [`UiCommand`].
    pub enum CommandType {
        /// `createGroup`.
        CreateGroup = "createGroup",
        /// `parseInvite`.
        ParseInvite = "parseInvite",
        /// `joinGroup`.
        JoinGroup = "joinGroup",
        /// `listGroups`.
        ListGroups = "listGroups",
        /// `forgetGroup`.
        ForgetGroup = "forgetGroup",
        /// `getInvite`.
        GetInvite = "getInvite",
        /// `goLive` ★.
        GoLive = "goLive",
        /// `resumeMic` ★.
        ResumeMic = "resumeMic",
        /// `audioCheck` (★ on start).
        AudioCheck = "audioCheck",
        /// `setTalkMode`.
        SetTalkMode = "setTalkMode",
        /// `ptt`.
        Ptt = "ptt",
        /// `setPttStyle`.
        SetPttStyle = "setPttStyle",
        /// `setVoxSensitivity`.
        SetVoxSensitivity = "setVoxSensitivity",
        /// `rideControl`.
        RideControl = "rideControl",
        /// `setDisplayName`.
        SetDisplayName = "setDisplayName",
        /// `setVehicle`.
        SetVehicle = "setVehicle",
        /// `setRoutePreference`.
        SetRoutePreference = "setRoutePreference",
        /// `setShareDistance`.
        SetShareDistance = "setShareDistance",
        /// `setPrompts`.
        SetPrompts = "setPrompts",
        /// `setBuiltinMicTx`.
        SetBuiltinMicTx = "setBuiltinMicTx",
        /// `removeMember`.
        RemoveMember = "removeMember",
        /// `lockGroup`.
        LockGroup = "lockGroup",
        /// `closeGroup`.
        CloseGroup = "closeGroup",
        /// `previewAccessLoss`.
        PreviewAccessLoss = "previewAccessLoss",
        /// `transferLeader`.
        TransferLeader = "transferLeader",
        /// `setInviteExpiry`.
        SetInviteExpiry = "setInviteExpiry",
        /// `requestSnapshot`.
        RequestSnapshot = "requestSnapshot",
        /// `labStart` (★ when audio is not off).
        LabStart = "labStart",
        /// `labStop`.
        LabStop = "labStop",
        /// `labMark`.
        LabMark = "labMark",
        /// `labListRuns`.
        LabListRuns = "labListRuns",
        /// `labDeleteRun`.
        LabDeleteRun = "labDeleteRun",
        /// `joinOpenChannel`.
        JoinOpenChannel = "joinOpenChannel",
        /// `setMemberMuted`.
        SetMemberMuted = "setMemberMuted",
        /// `setMemberBlocked`.
        SetMemberBlocked = "setMemberBlocked",
    }
}

impl UiCommand {
    /// The `type` discriminant of this command.
    pub const fn command_type(&self) -> CommandType {
        match self {
            UiCommand::CreateGroup { .. } => CommandType::CreateGroup,
            UiCommand::ParseInvite { .. } => CommandType::ParseInvite,
            UiCommand::JoinGroup { .. } => CommandType::JoinGroup,
            UiCommand::ListGroups {} => CommandType::ListGroups,
            UiCommand::ForgetGroup { .. } => CommandType::ForgetGroup,
            UiCommand::GetInvite { .. } => CommandType::GetInvite,
            UiCommand::GoLive {} => CommandType::GoLive,
            UiCommand::ResumeMic {} => CommandType::ResumeMic,
            UiCommand::AudioCheck(_) => CommandType::AudioCheck,
            UiCommand::SetTalkMode { .. } => CommandType::SetTalkMode,
            UiCommand::Ptt { .. } => CommandType::Ptt,
            UiCommand::SetPttStyle { .. } => CommandType::SetPttStyle,
            UiCommand::SetVoxSensitivity { .. } => CommandType::SetVoxSensitivity,
            UiCommand::RideControl { .. } => CommandType::RideControl,
            UiCommand::SetDisplayName { .. } => CommandType::SetDisplayName,
            UiCommand::SetVehicle { .. } => CommandType::SetVehicle,
            UiCommand::SetRoutePreference { .. } => CommandType::SetRoutePreference,
            UiCommand::SetShareDistance { .. } => CommandType::SetShareDistance,
            UiCommand::SetPrompts { .. } => CommandType::SetPrompts,
            UiCommand::SetBuiltinMicTx { .. } => CommandType::SetBuiltinMicTx,
            UiCommand::RemoveMember { .. } => CommandType::RemoveMember,
            UiCommand::LockGroup {} => CommandType::LockGroup,
            UiCommand::CloseGroup {} => CommandType::CloseGroup,
            UiCommand::PreviewAccessLoss {} => CommandType::PreviewAccessLoss,
            UiCommand::TransferLeader { .. } => CommandType::TransferLeader,
            UiCommand::SetInviteExpiry { .. } => CommandType::SetInviteExpiry,
            UiCommand::RequestSnapshot {} => CommandType::RequestSnapshot,
            UiCommand::LabStart { .. } => CommandType::LabStart,
            UiCommand::LabStop {} => CommandType::LabStop,
            UiCommand::LabMark { .. } => CommandType::LabMark,
            UiCommand::LabListRuns {} => CommandType::LabListRuns,
            UiCommand::LabDeleteRun { .. } => CommandType::LabDeleteRun,
            UiCommand::JoinOpenChannel { .. } => CommandType::JoinOpenChannel,
            UiCommand::SetMemberMuted { .. } => CommandType::SetMemberMuted,
            UiCommand::SetMemberBlocked { .. } => CommandType::SetMemberBlocked,
        }
    }

    /// `true` for the ★ capture-gated commands (DESIGN §3.2): `goLive`, `resumeMic`,
    /// `labStart` with `audio` ≠ `off` and `audioCheck{op:"start"}`. They are accepted
    /// only through the plugin's `gestureCommand`; `send` rejects them with
    /// [`crate::ErrorCode::UseGestureCommand`]. Mirrors [`crate::GESTURE_COMMANDS`].
    pub fn requires_gesture(&self) -> bool {
        match self {
            UiCommand::GoLive {} | UiCommand::ResumeMic {} => true,
            UiCommand::AudioCheck(op) => matches!(op, AudioCheckOp::Start { .. }),
            UiCommand::LabStart { params, .. } => params.audio != LabAudio::Off,
            _ => false,
        }
    }

    /// The payload kind of a successful result of this command.
    pub const fn result_kind(&self) -> ResultKind {
        self.command_type().result_kind()
    }
}

impl CommandType {
    /// The payload kind of a successful result of a command of this type.
    pub const fn result_kind(self) -> ResultKind {
        match self {
            CommandType::CreateGroup | CommandType::JoinGroup | CommandType::JoinOpenChannel => {
                ResultKind::GroupId
            }
            CommandType::ParseInvite => ResultKind::InviteInfo,
            CommandType::ListGroups => ResultKind::Groups,
            CommandType::GetInvite => ResultKind::Invite,
            CommandType::PreviewAccessLoss => ResultKind::AccessLoss,
            CommandType::LabStart => ResultKind::LabRun,
            CommandType::LabListRuns => ResultKind::LabRuns,
            CommandType::ForgetGroup
            | CommandType::GoLive
            | CommandType::ResumeMic
            | CommandType::AudioCheck
            | CommandType::SetTalkMode
            | CommandType::Ptt
            | CommandType::SetPttStyle
            | CommandType::SetVoxSensitivity
            | CommandType::RideControl
            | CommandType::SetDisplayName
            | CommandType::SetVehicle
            | CommandType::SetRoutePreference
            | CommandType::SetShareDistance
            | CommandType::SetPrompts
            | CommandType::SetBuiltinMicTx
            | CommandType::RemoveMember
            | CommandType::LockGroup
            | CommandType::CloseGroup
            | CommandType::TransferLeader
            | CommandType::SetInviteExpiry
            | CommandType::RequestSnapshot
            | CommandType::LabStop
            | CommandType::LabMark
            | CommandType::LabDeleteRun
            | CommandType::SetMemberMuted
            | CommandType::SetMemberBlocked => ResultKind::Done,
        }
    }

    /// Part of the Phase 0 Lab subset of API 1.1 (frozen at Phase 0 exit, DESIGN §14).
    pub const fn in_lab_subset(self) -> bool {
        matches!(
            self,
            CommandType::LabStart
                | CommandType::LabStop
                | CommandType::LabMark
                | CommandType::LabListRuns
                | CommandType::LabDeleteRun
                | CommandType::RequestSnapshot
        )
    }

    /// Commands of this type may be ★ gesture commands (always for `goLive` and
    /// `resumeMic`, conditionally for `audioCheck` and `labStart`).
    pub const fn may_require_gesture(self) -> bool {
        matches!(
            self,
            CommandType::GoLive
                | CommandType::ResumeMic
                | CommandType::AudioCheck
                | CommandType::LabStart
        )
    }
}
