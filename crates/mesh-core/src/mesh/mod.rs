//! The `mesh` module of mesh-core (DESIGN §3.3, owned by WP-10D): the glue between the
//! engine and mesh-routing for multi-hop group sessions.
//!
//! ```text
//! authenticated MESH record (SEC tag verified, Up group link)
//!   ─► own-echo filter ─► per-neighbor ingress caps ─► Router (dedup: one window per
//!      (origin, sess), first-hop origin check, TTL, voice age cap, floor admission per
//!      outgoing link, FLOOR_REJECT, near-talker rejection, ctrl jitter + counter
//!      suppression, ctrl/bulk two-hop pruning; voice is never pruned)
//!   ─► link bundle choice (Aware/BLE per packet, 1 s overlap; Tunnel cost hook)
//!   ─► staging LinkQueue per link (strict priority, ≤ 2 voice frames per origin,
//!      60 ms voice backlog, stream-level shedding, age rewritten at dequeue)
//!   ─► just-in-time hand-off to the link layer queue, paced by the link token bucket
//!      at min(R, grant) (delay-based pacer + voice-aware node airtime budget)
//!
//! my own packet (Ctx::originate: group PRESENCE/ROSTER/…, media TALK/VOICE/…)
//!   ─► relay floor table, my own per-link voice admission (an own burst holds a slot)
//!   ─► every neighbor (no pruning at the origin) ─► the same bundle choice, staging,
//!      pacing and airtime budget as relayed packets
//! ```
//!
//! Around it: NBR send/ingest (§4.4, §5.1), link quality from the link layer's PING
//! RTT, PING loss and RSSI, per-link caps with hysteresis published to the router and
//! advertised as `vcap`, PATH_HINT{BLE12} when a grant stays below `cost(1)`, the link
//! selection policy (redundant links over max with the articulation check,
//! `LimitReached` backoff, `ArmReconnect`, and the §5.7 radio hooks: unreachable
//! members' tokens connect first, discovery stays `Forming` while one is missing), and
//! the Phase 4 UPGRADE hooks.
//!
//! The content of what this node originates stays with the modules that own it; every
//! originated MESH packet takes its `seq` from the engine's shared per-origin counter
//! (`Ctx::next_mesh_seq`, packets the router builds itself included). The module is
//! active only while a group session runs and the group identity is known (see
//! [`identity`]); otherwise the engine queues originated packets on every Up link.

pub mod bundle;
pub mod identity;
mod ingress;
mod links;
mod module;
mod origin;
mod pace;
pub mod policy;
mod sched;
pub mod seg;
mod select;
mod state;
pub mod upgrade;

pub use identity::MeshIdentity;
pub use module::MeshModule;
pub use policy::{MeshPolicy, ReconnectPolicy};
pub use state::MeshStats;

/// How often the staging queues retry a hand-off to a busy link layer queue.
pub const PUMP_RETRY_US: u64 = 5_000;
/// Period of the loss observation from PING/PONG counters (EWMA input, §5.1).
pub const LOSS_PERIOD_US: u64 = 1_000_000;
/// A member is reachable when its PRESENCE was heard within this window (§5.8).
pub const REACHABLE_US: u64 = mesh_routing::consts::MEMBER_LOST_US;

/// Engine link id → mesh-routing link id.
pub(crate) const fn rl(l: crate::LinkId) -> mesh_routing::LinkId {
    mesh_routing::LinkId(l.0)
}

/// mesh-routing link id → engine link id.
pub(crate) const fn cl(l: mesh_routing::LinkId) -> crate::LinkId {
    crate::LinkId(l.0)
}
