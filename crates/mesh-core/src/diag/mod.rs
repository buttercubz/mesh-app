//! `ridelog/1` emission (diagnostics spec §2.4, stage 2): the core's typed events and
//! its 1 Hz [`CoreSample`](mesh_lab::ridelog::CoreSample), handed to the runtime as
//! `Effect::Diag`.
//!
//! Everything here is additive and inert while diagnostics are off (the default):
//! the runtime turns them on with [`crate::Engine::set_diag`]. Emission never changes
//! a decision of the engine. Event codes are `&'static str` only.
//!
//! Where events come from:
//! - the link layer (`LinkTable`): link up/down, handshake results, WouldBlock bursts,
//!   per-frame counters ([`link::LinkDiag`]);
//! - the radio policy: discovery mode and scan intensity, peers seen, connect tries,
//!   failures with their backoff, transport choice (L2CAP, GATT, GATT fallback),
//!   aborts;
//! - the modules (`CoreModule::fill_diag` on the 1 Hz tick): mesh counters and
//!   neighbor changes, floor, audio and jitter-buffer state, and edge events (floor
//!   grants and rejects, PTT/VAD, capture start/stop) at the decision points;
//! - [`sample`]: the 1 Hz tick itself and the changes visible in the snapshot
//!   (heard/lost, reachability, partitions, mic state, route, leader).

pub mod classify;
pub mod link;
pub mod sample;

use mesh_lab::ridelog::DiagEvent;
use mesh_wire::frame::RejectReason;

use crate::links::{LinkDownReason, LinkRole};

pub use sample::DiagTracker;

/// Sample period of the core's `s` contribution (µs).
pub const SAMPLE_US: u64 = 1_000_000;

/// Static code of a reject / close reason.
pub const fn reject_str(r: RejectReason) -> &'static str {
    match r {
        RejectReason::Version => "version",
        RejectReason::GroupEpochMismatch => "epochMismatch",
        RejectReason::AtMaxLinks => "atMaxLinks",
        RejectReason::DuplicateLink => "duplicateLink",
        RejectReason::MemberRemovedOrDenied => "removed",
        RejectReason::LabCodeMismatch => "labCodeMismatch",
        RejectReason::HandshakeFailure => "handshakeFailure",
        RejectReason::NodeIdCollision => "nodeIdCollision",
        RejectReason::Redundant => "redundant",
        RejectReason::Leaving => "leaving",
    }
}

/// Static code of a link-down reason (the peer's reject reason when it sent one).
pub fn down_str(r: LinkDownReason) -> &'static str {
    match r {
        LinkDownReason::PeerRejected(x) => reject_str(x),
        other => other.as_str(),
    }
}

/// ridelog link role.
pub const fn role(r: LinkRole) -> mesh_lab::LinkRole {
    match r {
        LinkRole::Initiator => mesh_lab::LinkRole::Central,
        LinkRole::Responder => mesh_lab::LinkRole::Peripheral,
    }
}

/// A buffer of events produced by code without access to the effect queue (link
/// table, radio policy); the engine moves them into `Effect::Diag` after each call.
#[derive(Debug, Clone, Default)]
pub struct DiagBuf {
    on: bool,
    ev: Vec<DiagEvent>,
}

impl DiagBuf {
    /// Turns recording on or off (off drops what is buffered).
    pub fn set(&mut self, on: bool) {
        self.on = on;
        if !on {
            self.ev.clear();
        }
    }

    /// `true` while recording.
    pub fn on(&self) -> bool {
        self.on
    }

    /// Buffers one event (no-op while off).
    pub fn push(&mut self, e: DiagEvent) {
        if self.on {
            self.ev.push(e);
        }
    }

    /// Takes the buffered events.
    pub fn take(&mut self) -> Vec<DiagEvent> {
        std::mem::take(&mut self.ev)
    }
}
