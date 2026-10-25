//! Routing, floor control, pacing and presence (contract C8, DESIGN §5, §6, §4.8).
//!
//! Sans-IO and deterministic: time is a caller-supplied [`Instant`] (µs, monotonic),
//! randomness is an injected `ChaCha20Rng`, maps are `BTreeMap`s. Every input that
//! changes routing state is an **authenticated** mesh record (the core calls in only
//! after the SEC frame's AEAD tag verified, §4.1, §5.3); the type
//! [`router::Authenticated`] makes that contract explicit in the API.
//!
//! Module map:
//! - [`types`], [`consts`], [`limits`]: shared vocabulary and Appendix A constants;
//! - [`quality`], [`neighbor`], [`nbr`]: link quality, neighbor table, NBR build/ingest
//!   (§5.1);
//! - [`dedup`]: per-(origin, sess) windows (§5.3);
//! - [`prune`]: two-hop pruning, ctrl/bulk only (§5.4);
//! - [`cost`], [`cap`], [`pacer`], [`airtime`], [`pacing`], [`queue`]: cost model, cap
//!   estimator with hysteresis, delay-based pacer, voice-aware node airtime budget, the
//!   node-level pacing loop, per-link priority queues with stream-level shedding (§4.8,
//!   §5.6);
//! - [`floor`]: floor table, total rank order, per-link admission, near-talker rejection,
//!   origin `local_admit` (§6);
//! - [`router`]: forwarding (§5.2), TTL/age (§5.5), penalties, control jitter;
//! - [`reach`]: presence, reachability, who-hears-me, one-way, islands, voice reach
//!   (§5.8);
//! - [`select`]: link selection policy (§5.7, §9.3, §9.5);
//! - [`position`]: distance/bearing math (§5.8).
//!
//! Owned by WP-09 (stub created by WP-01, DESIGN §16.2). The determinism lints below are
//! denied here (clippy.toml holds the list; the workspace allows them elsewhere).
#![deny(clippy::disallowed_methods, clippy::disallowed_types)]
#![forbid(unsafe_code)]

pub mod airtime;
pub mod cap;
pub mod consts;
pub mod cost;
pub mod dedup;
pub mod floor;
pub mod limits;
pub mod nbr;
pub mod neighbor;
pub mod pacer;
pub mod pacing;
pub mod position;
pub mod prune;
pub mod quality;
pub mod queue;
pub mod reach;
pub mod router;
pub mod select;
pub mod types;

pub use airtime::{AirtimeBudget, AirtimeInput, Grant};
pub use cap::{CapEstimator, CapModels, CapUpdate};
pub use cost::CostModel;
pub use dedup::{Dedup, DedupVerdict};
pub use floor::{
    Admission, AdmitDecision, BurstEntry, BurstKey, EndReason, FloorEvent, FloorTable, Rank,
    floor_hash,
};
pub use limits::Limits;
pub use nbr::{NbrBuilder, NbrReport, NbrReportEntry, OwnLinkEntry};
pub use neighbor::{LinkBundle, LinkState, LinkUpError, Neighbor, NeighborTable};
pub use pacer::{Pacer, PacerPath, PacerSample};
pub use pacing::{LinkPacing, NodePacing, PacingUpdate};
pub use position::{Fix, RelativePosition};
pub use prune::{SegTimeouts, pruned};
pub use quality::{LinkQuality, LossEstimator, RttEstimator};
pub use queue::{LinkQueue, OutRecord, Priority, QueueItem, QueuedPacket, ShedEvent};
pub use reach::{IslandView, MemberReach, ReachEvent, Reachability, VoiceReach};
pub use router::{
    Authenticated, Outgoing, RouteOutcome, Router, RouterAction, RouterConfig, RouterStats,
};
pub use select::{
    Candidate, CandidateClass, DiscoveryMode, LinkSelector, SelectAction, SelectInput,
    resolve_duplicate,
};
pub use types::{Instant, LinkId, LinkKind, LinkRole, NodeId};
