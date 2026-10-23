//! i18n key sets the engine emits instead of text (DESIGN §3.7, §7.7, §11.5).
//!
//! The UI owns every user-facing string: `errors.<ErrorCode>`, `notices.<NoticeKey>` and
//! `prompts.<PromptKey>` in its catalogs (WP-18), with an ES/EN parity test over the
//! `ALL` lists exported to `constants.ts`.

use crate::macros::string_enum;

string_enum! {
    /// Why a command failed (`CommandResult { ok: false, code }`) or an `error` event.
    pub enum ErrorCode {
        /// The command JSON is malformed or names an unknown `type` (`params.detail` is a
        /// key, never prose).
        InvalidCommand = "invalidCommand",
        /// A field has an out-of-range value (e.g. display name over 24 bytes).
        InvalidParams = "invalidParams",
        /// A capture-starting ★ command arrived through `send` (DESIGN §3.2).
        UseGestureCommand = "useGestureCommand",
        /// `gestureCommand` was called while the app is not foreground-active (§11.1).
        NotForeground = "notForeground",
        /// The plugin and the native engine disagree on the API major version (§11.1).
        ApiVersionMismatch = "apiVersionMismatch",
        /// The command needs a running engine (`startEngine` first).
        EngineNotRunning = "engineNotRunning",
        /// The engine thread crashed and stopped cleanly (§3.4).
        EngineCrashed = "engineCrashed",
        /// The command needs an open group session.
        NoSession = "noSession",
        /// No stored group has this id.
        UnknownGroup = "unknownGroup",
        /// The group was closed by its leader.
        GroupClosed = "groupClosed",
        /// Leader-only command sent by a member.
        NotLeader = "notLeader",
        /// The target node is not a member of the group.
        UnknownMember = "unknownMember",
        /// The invite text does not parse or its signature does not verify.
        InviteInvalid = "inviteInvalid",
        /// The invite expired (`setInviteExpiry`).
        InviteExpired = "inviteExpired",
        /// This device already belongs to the group.
        AlreadyMember = "alreadyMember",
        /// `goLive` while already live.
        AlreadyLive = "alreadyLive",
        /// The command needs the rider to be live (Go live first).
        NotLive = "notLive",
        /// The command is refused in the current mode (e.g. `audioCheck` while live or
        /// during a Lab run, §3.2).
        WrongMode = "wrongMode",
        /// Microphone permission missing.
        MicPermissionDenied = "micPermissionDenied",
        /// Bluetooth permission missing.
        BluetoothPermissionDenied = "bluetoothPermissionDenied",
        /// Bluetooth is off or in airplane mode.
        BluetoothOff = "bluetoothOff",
        /// Location permission missing (Share my distance, `audioLocation` anchor, Lab GPS).
        LocationPermissionDenied = "locationPermissionDenied",
        /// The audio session or capture could not start.
        AudioUnavailable = "audioUnavailable",
        /// A Lab run is already active.
        LabRunActive = "labRunActive",
        /// No Lab run is active.
        NoLabRun = "noLabRun",
        /// No stored Lab run has this id.
        UnknownLabRun = "unknownLabRun",
        /// The command belongs to a later phase and is not available in this build.
        NotSupported = "notSupported",
        /// Secure storage (Keychain/Keystore) or the file system failed.
        StorageFailed = "storageFailed",
        /// Unexpected internal failure (logged with details natively).
        Internal = "internal",
    }
}

string_enum! {
    /// Notice shown as a banner/toast and possibly a local notification (UI text lives
    /// in the `notices` catalog).
    pub enum NoticeKey {
        /// A member joined (`params.name`).
        MemberJoined = "memberJoined",
        /// A member left the ride on purpose (`params.name`).
        MemberLeft = "memberLeft",
        /// Members went out of range, aggregated over 5 s (`params.count`, `params.names`).
        MemberLost = "memberLost",
        /// A lost member is reachable again (`params.name`).
        MemberBack = "memberBack",
        /// Nobody is reachable (10 s).
        Alone = "alone",
        /// The group split into islands (`params.islands`).
        Partition = "partition",
        /// Microphone capture stopped (ladder L2/L3).
        CaptureLost = "captureLost",
        /// Microphone capture is back (L0).
        CaptureRestored = "captureRestored",
        /// The headset route disappeared; TX muted.
        HeadsetLost = "headsetLost",
        /// The headset route is back.
        HeadsetBack = "headsetBack",
        /// A phone call interrupted the intercom.
        OnCall = "onCall",
        /// The phone call ended and the intercom resumed.
        CallEnded = "callEnded",
        /// This device was removed from the group (`params.group`).
        Removed = "removed",
        /// The leader locked the group (new epoch).
        GroupLocked = "groupLocked",
        /// The leader closed the group.
        GroupClosed = "groupClosed",
        /// The ride started (`params.count` riders).
        RideStarted = "rideStarted",
        /// The leader ended the ride.
        RideEnded = "rideEnded",
        /// Bluetooth is off or the phone is in airplane mode.
        BluetoothOff = "bluetoothOff",
        /// Bluetooth is back on.
        BluetoothOn = "bluetoothOn",
        /// Bluetooth permission denied (offer "open Settings").
        BluetoothDenied = "bluetoothDenied",
        /// The OS connection limit was hit; link max lowered for 10 min.
        LinkLimitReached = "linkLimitReached",
        /// Leadership moved (`params.name` is the new leader).
        LeaderChanged = "leaderChanged",
        /// The scanned invite was signed by a different leader key than the stored group.
        InviteLeaderMismatch = "inviteLeaderMismatch",
        /// The Live Activity was renewed before the 8 h system limit (§10.1.3).
        LiveActivityRenewed = "liveActivityRenewed",
        /// Battery at or under 15 % (`params.percent`).
        BatteryLow = "batteryLow",
        /// Thermal state serious or critical; audio profile reduced.
        ThermalThrottled = "thermalThrottled",
        /// The talk burst reached its maximum length (§6.1).
        TalkLimit = "talkLimit",
        /// VOX sensitivity was lowered automatically (wind guard, §7.3).
        VoxLessSensitive = "voxLessSensitive",
        /// TX muted because only the built-in mic is available (motorcycle, §1.4).
        TxMutedNoHeadset = "txMutedNoHeadset",
        /// A Lab peer joined the Lab code (`params.labNode`).
        LabPeerJoined = "labPeerJoined",
        /// A Lab peer left (`params.labNode`).
        LabPeerLost = "labPeerLost",
        /// The keep-alive anchor of the running Lab run dropped (§12.1).
        LabAnchorLost = "labAnchorLost",
    }
}

string_enum! {
    /// Spoken-prompt template keys (DESIGN §7.7); templates come from the `prompts`
    /// catalog through `setPrompts`.
    pub enum PromptKey {
        /// "Intercom live, {n} riders".
        IntercomLive = "intercomLive",
        /// "{name} joined".
        MemberJoined = "memberJoined",
        /// "{name} left".
        MemberLeft = "memberLeft",
        /// "{n} riders out of range".
        RidersOutOfRange = "ridersOutOfRange",
        /// Nobody reachable.
        Alone = "alone",
        /// Mic paused, tap the phone to resume.
        MicPaused = "micPaused",
        /// Mic back.
        MicBack = "micBack",
        /// Headset lost.
        HeadsetLost = "headsetLost",
        /// Bluetooth off.
        BluetoothOff = "bluetoothOff",
        /// Battery low.
        BatteryLow = "batteryLow",
        /// Talk limit reached.
        TalkLimit = "talkLimit",
        /// VOX less sensitive.
        VoxLessSensitive = "voxLessSensitive",
        /// Ride started.
        RideStarted = "rideStarted",
        /// "{name} is the leader now".
        LeaderChanged = "leaderChanged",
        /// Demo-ride bot line 1 (§3.4).
        DemoLine1 = "demo.line1",
        /// Demo-ride bot line 2.
        DemoLine2 = "demo.line2",
        /// Demo-ride bot line 3.
        DemoLine3 = "demo.line3",
        /// Demo-ride bot line 4.
        DemoLine4 = "demo.line4",
        /// Demo-ride bot line 5.
        DemoLine5 = "demo.line5",
        /// Demo-ride bot line 6.
        DemoLine6 = "demo.line6",
        /// Demo-ride bot line 7.
        DemoLine7 = "demo.line7",
        /// Demo-ride bot line 8.
        DemoLine8 = "demo.line8",
    }
}
