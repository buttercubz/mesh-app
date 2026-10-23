//! Engine outputs (DESIGN §3.2): [`Transmit`] units and [`Effect`]s, plus the bounded
//! effect queue the engine drains through `poll_effect`.

use std::collections::VecDeque;

use mesh_api::{Lang, NoticeKey, Params, PromptKey, UiEvent};
use mesh_wire::frame::RejectReason;

use crate::LinkId;

/// One unit to write on a link: a stream chunk (`len u16` + frame), one ATT value (GATT)
/// or one datagram. Each `Transmit` is written as one platform send.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Transmit {
    pub link: LinkId,
    pub bytes: Vec<u8>,
}

/// Discovery mode (`RadioCommand::SetDiscovery`, §5.7, §9.4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum DiscoveryMode {
    /// Scanning and advertising stopped (radio off, engine stopped).
    Off,
    /// Under target: fast advertising and low-latency scans.
    Forming,
    /// At target: low-power advertising, balanced scans.
    Stable,
}

/// How hard the platform scans (`RadioCommand::SetDiscovery`, §5.7, §9.5). Scanning
/// shares the controller with every connection event, so its duty cycle is what the
/// links pay: the core asks for the cheapest intensity that still finds what it needs.
/// Android maps it to `SCAN_MODE_LOW_LATENCY` / `BALANCED` / `LOW_POWER`; iOS to
/// `AllowDuplicates` (only at `LowLatency`) and the scan filter.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ScanIntensity {
    /// Continuous scanning (~100 % duty): only with no BLE link up, when nothing can
    /// be starved.
    LowLatency,
    /// ~25 % duty: hunting for a member while at least one link is up.
    Balanced,
    /// ~10 % duty: `Stable` (every member reachable), and with discovery `Off`.
    LowPower,
}

/// Advertising payloads built by the core (§9.4): the platform never builds bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdvPayload {
    /// Android scan response service data (9 B after `MESH_SVC`).
    pub service_data: [u8; 9],
    /// iPhone foreground local name (6 base32 chars).
    pub local_name: String,
}

/// How to open the data path after a HELLO check (§9.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OpenVia {
    /// L2CAP CoC on the peer's published PSM.
    L2cap { psm: u16 },
    /// GATT `RX` writes / `TX` notifications.
    Gatt,
}

/// Link priority hint for the platform (Android `requestConnectionPriority`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LinkPriority {
    High,
    Balanced,
}

/// Radio decisions executed by the platform (§3.2).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RadioCommand {
    /// Scan/advertise mode and the advertising payload (`None` while `Off`).
    SetDiscovery {
        mode: DiscoveryMode,
        adv: Option<AdvPayload>,
        /// Scan duty the platform should use (meaningless with `mode: Off`).
        scan: ScanIntensity,
    },
    /// New 16-byte HELLO value for the peripheral role.
    SetHello { bytes: [u8; 16] },
    /// Connect to a discovered peer and read its HELLO.
    Connect { disc_id: u32 },
    /// Open the data path after a HELLO check.
    OpenLink { disc_id: u32, via: OpenVia },
    /// Give up on `disc_id` and ignore it for `ignore_s` seconds.
    Abort { disc_id: u32, ignore_s: u16 },
    /// Close a link. `reason` was also sent in-band (CLOSE/REJECT) when present;
    /// `None` for local decisions without a peer to tell (timeout, radio off).
    Close {
        link: LinkId,
        reason: Option<RejectReason>,
    },
    /// Connection priority hint.
    SetLinkParams {
        link: LinkId,
        priority: LinkPriority,
    },
    /// Keep an outstanding reconnect to a lost peer (T3 variant, §9.5).
    ArmReconnect { peer_ref: u32, until_s: u16 },
    /// Wi-Fi Aware upgrade step (Phase 4).
    AwareUpgrade { peer: u16, op: u8 },
    /// v2 tunnel hook (designed, not built).
    TunnelOpen { peer: u16 },
}

/// Kind of audio session (`AudioCommand::OpenSession`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AudioSessionKind {
    /// Ride session (Go live).
    Ride,
    /// Lab run with `audio = renderOnly` (output only, no mic).
    LabRender,
    /// Lab run with `audio = full` (capture + render, HFP when a headset is present).
    LabFull,
    /// Temporary audio-check session.
    Check,
}

/// Session profile handed to the native audio layer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AudioSessionProfile {
    pub kind: AudioSessionKind,
    /// The session needs the microphone.
    pub capture: bool,
    /// Mixable session variant (T2 Lab toggle).
    pub mixable: bool,
    /// Keep-alive location anchor (CLBackgroundActivitySession / FGS location).
    pub location_anchor: bool,
    /// VPIO `BypassVoiceProcessing` (T5a loopback).
    pub bypass_voice_processing: bool,
}

/// Audio decisions executed by the platform (§3.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AudioCommand {
    OpenSession {
        profile: AudioSessionProfile,
    },
    /// Connect the RT sink and unmute (never activates a session, §3.2).
    StartCapture,
    StopCapture,
    SetRoutePreference(mesh_api::RoutePreference),
    OutputOnlyFallback(bool),
    CloseSession,
}

/// A local notification / Live Activity / FGS notification update.
#[derive(Debug, Clone, PartialEq)]
pub struct Notification {
    pub key: NoticeKey,
    pub params: Params,
}

/// A spoken prompt to render with platform TTS (§7.7).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PromptRequest {
    pub key: PromptKey,
    pub text: String,
    pub lang: Lang,
}

/// Which Lab file a line belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LabFile {
    /// `<runId>.jsonl`: one labrun/1 line.
    Jsonl,
    /// `<runId>-summary.csv`: header + one row.
    SummaryCsv,
}

impl LabFile {
    /// File name for a run.
    pub fn file_name(self, run_id: &str) -> String {
        match self {
            LabFile::Jsonl => format!("{run_id}.jsonl"),
            LabFile::SummaryCsv => format!("{run_id}-summary.csv"),
        }
    }
}

/// Log severity.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum LogLevel {
    Debug,
    Info,
    Warn,
    Error,
}

/// Every engine → platform effect (§3.2).
#[derive(Debug, Clone, PartialEq)]
pub enum Effect {
    Radio(RadioCommand),
    Audio(AudioCommand),
    /// UI event, forwarded verbatim (as JSON) to the plugin.
    Ui(UiEvent),
    /// Secure storage write (`None` deletes the key).
    Persist {
        key: String,
        value: Option<Vec<u8>>,
    },
    Notify(Notification),
    /// Append one line to a Lab file.
    LabWrite {
        run_id: String,
        file: LabFile,
        line: String,
    },
    /// Delete every file of a Lab run.
    LabDelete {
        run_id: String,
    },
    RenderPrompts(Vec<PromptRequest>),
    Log {
        level: LogLevel,
        msg: String,
    },
    /// A `ridelog/1` diagnostics record (typed event or the 1 Hz core sample). Only
    /// emitted while the runtime enabled diagnostics ([`crate::Engine::set_diag`]).
    Diag(mesh_lab::ridelog::DiagRecord),
}

/// Bounded FIFO of effects. When full, `Diag` effects are dropped first (at
/// [`DIAG_DROP_THRESHOLD`]), then `Log` effects; other effects are never dropped (the
/// runtime drains every tick).
#[derive(Debug, Clone, Default)]
pub struct EffectQueue {
    q: VecDeque<Effect>,
    dropped_logs: u32,
    /// Diagnostics records are produced at all (off by default: no cost, no change in
    /// the effect stream for simulators and tests).
    diag_on: bool,
    dropped_diag: u64,
}

/// Queue length above which new `Log` effects are dropped.
pub const LOG_DROP_THRESHOLD: usize = 4096;
/// Queue length above which new `Diag` effects are dropped (below the `Log` threshold,
/// so diagnostics go first).
pub const DIAG_DROP_THRESHOLD: usize = 2048;

impl EffectQueue {
    pub fn new() -> Self {
        Self::default()
    }

    /// Queues an effect.
    pub fn push(&mut self, e: Effect) {
        if matches!(e, Effect::Diag(_)) && self.q.len() >= DIAG_DROP_THRESHOLD {
            self.dropped_diag = self.dropped_diag.saturating_add(1);
            return;
        }
        if matches!(e, Effect::Log { .. }) && self.q.len() >= LOG_DROP_THRESHOLD {
            self.dropped_logs = self.dropped_logs.saturating_add(1);
            return;
        }
        self.q.push_back(e);
    }

    pub fn radio(&mut self, cmd: RadioCommand) {
        self.push(Effect::Radio(cmd));
    }

    pub fn audio(&mut self, cmd: AudioCommand) {
        self.push(Effect::Audio(cmd));
    }

    pub fn ui(&mut self, ev: UiEvent) {
        self.push(Effect::Ui(ev));
    }

    pub fn log(&mut self, level: LogLevel, msg: impl Into<String>) {
        self.push(Effect::Log {
            level,
            msg: msg.into(),
        });
    }

    /// Turns diagnostics records on or off.
    pub fn set_diag(&mut self, on: bool) {
        self.diag_on = on;
    }

    /// `true` while diagnostics records are produced.
    pub fn diag_on(&self) -> bool {
        self.diag_on
    }

    /// Queues a diagnostics event (no-op while diagnostics are off).
    pub fn diag(&mut self, e: mesh_lab::ridelog::DiagEvent) {
        if self.diag_on {
            self.push(Effect::Diag(mesh_lab::ridelog::DiagRecord::Event(e)));
        }
    }

    /// Queues the 1 Hz core sample (no-op while diagnostics are off).
    pub fn diag_sample(&mut self, s: mesh_lab::ridelog::CoreSample) {
        if self.diag_on {
            self.push(Effect::Diag(mesh_lab::ridelog::DiagRecord::Sample(
                Box::new(s),
            )));
        }
    }

    /// `Diag` effects dropped because the queue was full (cumulative).
    pub fn dropped_diag(&self) -> u64 {
        self.dropped_diag
    }

    /// Next effect in FIFO order.
    pub fn pop(&mut self) -> Option<Effect> {
        self.q.pop_front()
    }

    pub fn len(&self) -> usize {
        self.q.len()
    }

    pub fn is_empty(&self) -> bool {
        self.q.is_empty()
    }

    /// Log effects dropped because the queue was full.
    pub fn dropped_logs(&self) -> u32 {
        self.dropped_logs
    }

    /// Iterates queued effects without draining them (tests, diagnostics).
    pub fn iter(&self) -> impl Iterator<Item = &Effect> {
        self.q.iter()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn logs_are_dropped_first_when_full() {
        let mut q = EffectQueue::new();
        for _ in 0..LOG_DROP_THRESHOLD {
            q.audio(AudioCommand::StopCapture);
        }
        q.log(LogLevel::Info, "x");
        assert_eq!(q.dropped_logs(), 1);
        q.audio(AudioCommand::CloseSession);
        assert_eq!(q.len(), LOG_DROP_THRESHOLD + 1);
        assert_eq!(q.pop(), Some(Effect::Audio(AudioCommand::StopCapture)));
        assert_eq!(LabFile::SummaryCsv.file_name("r-1"), "r-1-summary.csv");
    }

    #[test]
    fn diag_is_off_by_default_and_dropped_before_logs() {
        use mesh_lab::ridelog::DiagEvent;
        let mut q = EffectQueue::new();
        q.diag(DiagEvent::Ptt { on: true });
        assert!(q.is_empty(), "off by default");
        q.set_diag(true);
        for _ in 0..DIAG_DROP_THRESHOLD {
            q.audio(AudioCommand::StopCapture);
        }
        q.diag(DiagEvent::Ptt { on: true });
        assert_eq!(q.dropped_diag(), 1);
        q.log(LogLevel::Info, "kept");
        assert_eq!(q.dropped_logs(), 0);
        assert_eq!(q.len(), DIAG_DROP_THRESHOLD + 1);
    }
}
