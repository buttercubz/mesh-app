//! Messages into the engine thread (bounded crossbeam MPSC, DESIGN §3.4) and the
//! one-shot replies of the control calls.

use crossbeam_channel::Sender;
use mesh_api::{CommandResult, PromptKey, UiCommand};
use mesh_core::{
    AudioIoState, AudioRouteInfo, Discovery, LinkDownReason, LinkId, LinkInfo, PlatformSample,
    RadioAvail, SessionStart,
};

use crate::api::types::FfiError;

/// Capacity of the inbound channel (DESIGN §3.4).
pub const INBOUND_CAPACITY: usize = 2048;

/// One message to the engine thread.
pub enum Inbound {
    /// A UI command with its reply slot.
    Command {
        cmd: UiCommand,
        reply: Sender<CommandResult>,
    },
    /// Start a group session or Lab mode.
    Start {
        start: SessionStart,
        reply: Sender<Result<(), FfiError>>,
    },
    /// Stop the session (back to Idle).
    Stop {
        reply: Sender<()>,
    },
    /// Stop everything and exit the thread (runtime dropped).
    Shutdown,
    LinkUp {
        link: LinkId,
        info: LinkInfo,
    },
    LinkDown {
        link: LinkId,
        reason: LinkDownReason,
    },
    LinkData {
        link: LinkId,
        data: Vec<u8>,
    },
    Writable {
        link: LinkId,
    },
    Metrics {
        link: LinkId,
        rssi: Option<i16>,
        att_mtu: Option<u16>,
    },
    Discovered(Discovery),
    HelloRead {
        disc_id: u32,
        bytes: Vec<u8>,
    },
    ConnectFailed {
        disc_id: u32,
        reason: LinkDownReason,
    },
    L2capPublished {
        psm: u16,
    },
    AudioRoute(AudioRouteInfo),
    AudioIo(AudioIoState),
    /// 1 Hz sample with the radio state it carries (`ble`, `aware`).
    Platform {
        sample: PlatformSample,
        radio: (RadioAvail, RadioAvail),
    },
    PromptPcm {
        key: PromptKey,
        pcm: Vec<i16>,
    },
    UiAttached(bool),
    /// Test hook: panic on the engine thread (crash-path tests only).
    #[cfg(test)]
    Panic,
}

impl Inbound {
    /// Radio-side events are dropped while no session runs (nothing would consume them,
    /// and stale link data must not reach the next session). `L2capPublished` is state,
    /// not traffic (the local channel can stay published across sessions), so it is
    /// always delivered.
    pub fn is_session_only(&self) -> bool {
        matches!(
            self,
            Inbound::LinkUp { .. }
                | Inbound::LinkDown { .. }
                | Inbound::LinkData { .. }
                | Inbound::Writable { .. }
                | Inbound::Metrics { .. }
                | Inbound::Discovered(_)
                | Inbound::HelloRead { .. }
                | Inbound::ConnectFailed { .. }
        )
    }
}
