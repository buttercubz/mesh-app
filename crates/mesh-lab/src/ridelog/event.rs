//! Typed diagnostic events of the `ridelog/1` flight recorder (§2.4 of the diagnostics
//! spec). The core emits [`DiagEvent`]s with static string codes only (no free text,
//! no keys, no names); platform and UI code inject [`NoteEvent`]s through the runtime's
//! `diag_note`, which are checked by the redaction filter before they are written.

use crate::json::{Json, round};
use crate::kinds::{LinkKind, LinkRole, lab_enum};

lab_enum! {
    /// Source of an `e` line.
    pub enum DiagSrc {
        /// mesh-core (sans-IO engine).
        Core = "core",
        /// The native runtime (mesh-ffi / mesh-wasm): RT counters, engine meta.
        Rt = "rt",
        /// Swift / Kotlin platform code.
        Plat = "plat",
        /// Plugin or web app.
        Ui = "ui",
    }
}

lab_enum! {
    /// Fixed `mark` labels (chips on the Diagnostics screen; never free text).
    pub enum MarkLabel {
        Crackle = "crackle",
        CantHear = "cantHear",
        NotHeard = "notHeard",
        Lag = "lag",
        Dropped = "dropped",
        Other = "other",
    }
}

lab_enum! {
    /// Frame types counted per link and direction in the `s` line (`byType`).
    pub enum FrameType {
        Voice = "voice",
        Nbr = "nbr",
        Lstat = "lstat",
        Ping = "ping",
        Pong = "pong",
        Presence = "presence",
        Ctrl = "ctrl",
        Rekey = "rekey",
        Sync = "sync",
        Bulk = "bulk",
        /// Handshake frames (HELLO / Noise).
        Hs = "hs",
        Lab = "lab",
        Other = "other",
    }
}

impl FrameType {
    /// Number of frame types (length of the per-type counter arrays).
    pub const COUNT: usize = FrameType::ALL.len();

    /// Index into the per-type counter arrays.
    pub const fn index(self) -> usize {
        self as usize
    }
}

lab_enum! {
    /// Floor event kind.
    pub enum FloorKind {
        Grant = "floorGrant",
        Reject = "floorReject",
        Release = "floorRelease",
    }
}

lab_enum! {
    /// RT glitch kind (`glitch` event, src `rt`).
    pub enum GlitchKind {
        Underrun = "underrun",
        Overrun = "overrun",
        CbSizeChange = "cbSizeChange",
        ClockDrift = "clockDrift",
        TimestampJump = "timestampJump",
    }
}

/// A typed event. Codes are `&'static str` so the core never formats text; the
/// recorder still passes every string through the redaction filter.
#[derive(Debug, Clone, PartialEq)]
pub enum DiagEvent {
    // --- session identity (core) ---
    /// The session's public identity, once the core knows it (fills `hdr` fields of
    /// later segments). `ride_key` from [`super::ride_key`].
    Session {
        ride_key: Option<[u8; 4]>,
        node: Option<u16>,
        role: Option<&'static str>,
    },
    // --- radio and links (core) ---
    RadioState {
        ble: &'static str,
        aware: &'static str,
    },
    DiscoveryMode {
        mode: &'static str,
        scan: Option<&'static str>,
        adv: Option<&'static str>,
        duty: Option<f32>,
    },
    PeerSeen {
        disc: u32,
        rssi: Option<i16>,
        group_match: bool,
        epoch_behind: bool,
    },
    ConnectTry {
        disc: u32,
        via: &'static str,
        attempt: u16,
    },
    ConnectFail {
        disc: u32,
        reason: &'static str,
        attempt: u16,
        backoff_s: u32,
    },
    Abort {
        disc: u32,
        ignore_s: u32,
        why: &'static str,
    },
    LinkUp {
        link: u32,
        kind: LinkKind,
        role: Option<LinkRole>,
        mtu: Option<u16>,
        psm: Option<u16>,
    },
    Handshake {
        link: u32,
        peer: Option<u16>,
        ok: bool,
        ms: u32,
        err: Option<&'static str>,
    },
    LinkDown {
        link: u32,
        peer: Option<u16>,
        reason: &'static str,
        up_s: u32,
        tx_b: u64,
        rx_b: u64,
    },
    LinkParams {
        link: u32,
        prio: &'static str,
        grant_kbps: f32,
    },
    WouldBlockBurst {
        link: u32,
        count: u32,
        ms: u32,
    },
    ReconnectArmed {
        peer: u16,
        until_s: u32,
    },
    ClkOffset {
        link: u32,
        peer: u16,
        offset_us: i64,
        rtt_us: u32,
    },
    // --- mesh and floor (core) ---
    NbrChange {
        peer: u16,
        q: f32,
        good: bool,
        congested: bool,
        kind: LinkKind,
    },
    Reach {
        nodes: u16,
        added: Vec<u16>,
        removed: Vec<u16>,
    },
    Partition {
        islands: u16,
        mine: u16,
        others: Vec<u16>,
    },
    Heard {
        peer: u16,
        last_heard_s: u32,
        lost: bool,
    },
    Floor {
        kind: FloorKind,
        origin: u16,
        reason: Option<&'static str>,
        streams: u8,
    },
    Rekey {
        epoch: u32,
    },
    MemberRemoved {
        peer: u16,
        epoch: u32,
    },
    LeaderChange {
        peer: u16,
        epoch: u32,
    },
    BudgetCap {
        cap_kbps: f32,
        level: u8,
    },
    // --- audio decisions (core) ---
    CaptureStart {
        why: &'static str,
    },
    CaptureStop {
        why: &'static str,
    },
    CaptureLost {
        reason: &'static str,
        detail: Option<&'static str>,
    },
    /// Exactly what the ride banner shows.
    MicState {
        state: &'static str,
        cause: Option<&'static str>,
    },
    Route {
        route: &'static str,
        prev: Option<&'static str>,
        reason: Option<&'static str>,
        hfp_wb: Option<bool>,
    },
    JbRetarget {
        origin: u16,
        from_ms: u16,
        to_ms: u16,
        why: &'static str,
    },
    Vad {
        on: bool,
        p: f32,
    },
    Ptt {
        on: bool,
    },
    PromptPlay {
        key: &'static str,
    },
    // --- runtime (rt) ---
    Glitch {
        kind: GlitchKind,
        n: u32,
        ms: u32,
    },
    EngineStart {
        mode: &'static str,
    },
    EngineStop,
    /// Panic location only (`file:line`), never the payload.
    EngineCrash {
        at: &'static str,
    },
    EffectOverflow {
        dropped: u32,
    },
    Logs {
        debug: u32,
        info: u32,
        warn: u32,
        error: u32,
    },
    /// Wall-clock anchor, re-emitted every 60 s (detects wall-clock jumps).
    ClkAnchor {
        wall_ms: u64,
        mono_us: u64,
    },
}

impl DiagEvent {
    /// The source this event is written with.
    pub const fn src(&self) -> DiagSrc {
        match self {
            DiagEvent::Glitch { .. }
            | DiagEvent::EngineStart { .. }
            | DiagEvent::EngineStop
            | DiagEvent::EngineCrash { .. }
            | DiagEvent::EffectOverflow { .. }
            | DiagEvent::Logs { .. }
            | DiagEvent::ClkAnchor { .. } => DiagSrc::Rt,
            _ => DiagSrc::Core,
        }
    }

    /// The `ev` id.
    pub const fn name(&self) -> &'static str {
        match self {
            DiagEvent::Session { .. } => "session",
            DiagEvent::RadioState { .. } => "radioState",
            DiagEvent::DiscoveryMode { .. } => "discoveryMode",
            DiagEvent::PeerSeen { .. } => "peerSeen",
            DiagEvent::ConnectTry { .. } => "connectTry",
            DiagEvent::ConnectFail { .. } => "connectFail",
            DiagEvent::Abort { .. } => "abort",
            DiagEvent::LinkUp { .. } => "linkUp",
            DiagEvent::Handshake { .. } => "handshake",
            DiagEvent::LinkDown { .. } => "linkDown",
            DiagEvent::LinkParams { .. } => "linkParams",
            DiagEvent::WouldBlockBurst { .. } => "wouldBlockBurst",
            DiagEvent::ReconnectArmed { .. } => "reconnectArmed",
            DiagEvent::ClkOffset { .. } => "clkOffset",
            DiagEvent::NbrChange { .. } => "nbrChange",
            DiagEvent::Reach { .. } => "reach",
            DiagEvent::Partition { .. } => "partition",
            DiagEvent::Heard { .. } => "heard",
            DiagEvent::Floor { kind, .. } => kind.as_str(),
            DiagEvent::Rekey { .. } => "rekey",
            DiagEvent::MemberRemoved { .. } => "memberRemoved",
            DiagEvent::LeaderChange { .. } => "leaderChange",
            DiagEvent::BudgetCap { .. } => "budgetCap",
            DiagEvent::CaptureStart { .. } => "captureStart",
            DiagEvent::CaptureStop { .. } => "captureStop",
            DiagEvent::CaptureLost { .. } => "captureLost",
            DiagEvent::MicState { .. } => "micState",
            DiagEvent::Route { .. } => "route",
            DiagEvent::JbRetarget { .. } => "jbRetarget",
            DiagEvent::Vad { .. } => "vad",
            DiagEvent::Ptt { .. } => "ptt",
            DiagEvent::PromptPlay { .. } => "promptPlay",
            DiagEvent::Glitch { .. } => "glitch",
            DiagEvent::EngineStart { .. } => "engineStart",
            DiagEvent::EngineStop => "engineStop",
            DiagEvent::EngineCrash { .. } => "engineCrash",
            DiagEvent::EffectOverflow { .. } => "effectOverflow",
            DiagEvent::Logs { .. } => "logs",
            DiagEvent::ClkAnchor { .. } => "clkAnchor",
        }
    }

    /// The event's fields (everything after `t, ts, src, ev`).
    pub fn fields(&self) -> Vec<(&'static str, Json)> {
        super::event_fields::fields(self)
    }
}

/// An event injected by platform or UI code (`diag_note`): `ev` plus scalar fields.
/// Built from untrusted input, so it is validated ([`NoteEvent::parse`]) and redacted.
#[derive(Debug, Clone, PartialEq)]
pub struct NoteEvent {
    pub src: DiagSrc,
    pub ev: String,
    pub fields: Vec<(String, Json)>,
}

/// Why a note was refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NoteError {
    BadSrc,
    BadEv,
    NotAnObject,
    BadField(String),
}

/// Reserved keys of an `e` line (a note cannot override them).
pub const RESERVED_KEYS: [&str; 4] = ["t", "ts", "src", "ev"];
/// Most fields in one note.
pub const MAX_NOTE_FIELDS: usize = 24;
/// Longest array in a note field.
pub const MAX_NOTE_ARRAY: usize = 64;

/// `^[a-z][A-Za-z0-9]*$`, at most 40 characters (the `ev` and field-key rule).
pub fn is_ident(s: &str) -> bool {
    let mut it = s.chars();
    matches!(it.next(), Some(c) if c.is_ascii_lowercase())
        && it.all(|c| c.is_ascii_alphanumeric())
        && s.len() <= 40
}

impl NoteEvent {
    /// Builds a note from already-parsed parts, checking ids and value shapes
    /// (scalars, or arrays of numbers).
    pub fn new(src: DiagSrc, ev: &str, fields: Vec<(String, Json)>) -> Result<Self, NoteError> {
        if src == DiagSrc::Core {
            return Err(NoteError::BadSrc);
        }
        if !is_ident(ev) {
            return Err(NoteError::BadEv);
        }
        if fields.len() > MAX_NOTE_FIELDS {
            return Err(NoteError::BadField("too many fields".into()));
        }
        for (k, v) in &fields {
            if !is_ident(k) || RESERVED_KEYS.contains(&k.as_str()) {
                return Err(NoteError::BadField(k.clone()));
            }
            let ok = match v {
                Json::Arr(items) => {
                    items.len() <= MAX_NOTE_ARRAY
                        && items
                            .iter()
                            .all(|i| matches!(i, Json::Int(_) | Json::Num(_)))
                }
                Json::Obj(_) => false,
                _ => true,
            };
            if !ok {
                return Err(NoteError::BadField(k.clone()));
            }
        }
        Ok(NoteEvent {
            src,
            ev: ev.to_string(),
            fields,
        })
    }
}

/// Rounds an `f32` gauge for records.
pub(crate) fn r32(v: f32, d: u32) -> Json {
    Json::Num(round(f64::from(v), d))
}
