//! Runtime state of the running group session (links, dedup, reassembly, origin
//! sequence, discovery state, anti-entropy rate limits) next to the persistent
//! [`Group`](super::state::Group).

use std::collections::{BTreeMap, BTreeSet};

use mesh_crypto::{NodeIdRegistry, PublicIdentity};
use mesh_routing::Dedup;
use mesh_wire::hello::Hello;
use mesh_wire::mesh::SegmentAssembler;
use mesh_wire::record::SyncKind;

use crate::secure::HsSlot;
use crate::session::Session;
use crate::{Instant, LinkId, LinkKind, LinkRole};

/// JOIN_REQ re-flood period while not in a verified roster (§8.9 rule 5).
pub const JOIN_RETX_US: u64 = 30_000_000;
/// A non-leader relays a joiner's JOIN_REQ toward the leader at most this often.
pub const JOIN_RELAY_INTERVAL_US: u64 = 10_000_000;
/// Anti-entropy push rate limit per (link, kind) (§8.9 rule 3).
pub const PUSH_INTERVAL_US: u64 = 10_000_000;
/// At most one speculative link per 30 s; 60 s per-peer backoff (§9.3).
pub const SPECULATIVE_INTERVAL_US: u64 = 30_000_000;
pub const SPECULATIVE_BACKOFF_US: u64 = 60_000_000;
/// Seconds per discovery token slot (§8.3).
pub const T10_S: u64 = 600;

/// Per-link state of a group link.
#[derive(Debug)]
pub struct LinkRt {
    pub slot: HsSlot,
    pub role: LinkRole,
    pub kind: LinkKind,
    /// The peer's identity once its IDPROOF verified.
    pub peer: Option<PublicIdentity>,
    /// The link passed admission and is Up.
    pub up: bool,
    /// The peer's CAPS summary `(epoch, roster_version, ctl_version)`.
    pub peer_summary: Option<(u32, u32, u32)>,
    /// SYNC reassembly (≤ 2 concurrent, 5 s).
    pub sync_asm: SegmentAssembler<()>,
    /// Last push per SYNC kind.
    pub pushed: BTreeMap<SyncKind, Instant>,
    /// Opened speculatively toward an epoch-ahead peer (§9.3).
    pub speculative: Option<[u8; 4]>,
    /// The peer's IDPROOF showed an identity whose `node_id` collides with mine (§8.2):
    /// its `REJECT{8}` / `CLOSE{8}` on this link is that direct collision, resolved by
    /// the `id_hash` rule, not a third node's report.
    pub collided: bool,
    /// Datagram links only (§9.6): when my IDPROOF goes out again while the link waits
    /// in IdProof, and how many times it already went out again.
    pub idproof_resend: Option<(Instant, u8)>,
    /// Datagram links only: last time a repeated peer IDPROOF on the Up link was looked
    /// at (rate limit of the answer), and how many answers went out.
    pub idproof_answered: Option<Instant>,
    pub idproof_answers: u8,
    /// I answered a repeated IDPROOF: the peer was still in IdProof when this link went
    /// Up here and dropped what I sent then (JOIN_REQ, PRESENCE). Its CAPS (sent once it
    /// is Up) re-runs my link-up announcements.
    pub reannounce: bool,
    /// Datagram links only: encoded SYNC / MESH records that arrived while this link
    /// waited in IdProof (the peer is Up and its later datagrams overtook its IDPROOF).
    /// Replayed once the link is Up; dropped with the link if admission fails. Bounded
    /// by [`EARLY_RECORDS_MAX`] and [`EARLY_BYTES_MAX`].
    pub early: Vec<Vec<u8>>,
}

/// Most early records kept per link in IdProof.
pub const EARLY_RECORDS_MAX: usize = 8;
/// Most early record bytes kept per link in IdProof.
pub const EARLY_BYTES_MAX: usize = 4_096;

impl LinkRt {
    pub fn new(slot: HsSlot, role: LinkRole, kind: LinkKind) -> Self {
        LinkRt {
            slot,
            role,
            kind,
            peer: None,
            up: false,
            peer_summary: None,
            sync_asm: SegmentAssembler::for_sync(),
            pushed: BTreeMap::new(),
            speculative: None,
            collided: false,
            idproof_resend: None,
            idproof_answered: None,
            idproof_answers: 0,
            reannounce: false,
            early: Vec::new(),
        }
    }

    /// `node_id` of the side that initiated this link.
    pub fn initiator_node(&self, me: u16) -> Option<u16> {
        match self.role {
            LinkRole::Initiator => Some(me),
            LinkRole::Responder => self.peer.map(|p| p.node_id()),
        }
    }
}

/// Runtime of the active group session.
#[derive(Debug)]
pub struct Active {
    pub group_id: [u8; 16],
    pub links: BTreeMap<LinkId, LinkRt>,
    /// `node_id` owners seen (roster, JOIN_REQ, IDPROOF).
    pub registry: NodeIdRegistry,
    /// Identities known to hold the newest epoch (PRESENCE/CAPS or newest handshake).
    pub newest_holders: BTreeSet<[u8; 16]>,
    pub dedup: Dedup,
    /// Mesh segment reassembly by `(origin, sess)`.
    pub asm: SegmentAssembler<(u16, u8)>,
    pub msg_id: u16,
    /// HELLOs read per discovery id (initiator epoch hint).
    pub hellos: BTreeMap<u32, Hello>,
    /// Speculative link bookkeeping.
    pub spec_last: Option<Instant>,
    pub spec_backoff: BTreeMap<[u8; 4], Instant>,
    pub spec_pending: BTreeMap<u32, [u8; 4]>,
    /// Current discovery slot.
    pub t10: u32,
    /// Epoch the radio target was built for.
    pub radio_epoch: u32,
    /// Next JOIN_REQ re-flood.
    pub join_next: Instant,
    /// Last relay of a joiner's JOIN_REQ toward the leader, by id_hash (non-leaders).
    pub relayed_joins: BTreeMap<[u8; 16], Instant>,
    pub session: Session,
    /// The group must be persisted at the end of this call.
    pub dirty_store: bool,
    /// A member refused me with `REJECT{5}` / `CLOSE{5}` (removed while I was away):
    /// the session's radio and audio stopped for the rest of this session.
    pub refused: bool,
    /// What the media module was last told (posted on the bus only on change).
    pub media: MediaSync,
    /// Local mute/block lists and the open-channel per-origin budget.
    pub filter: super::open::LocalFilter,
}

/// The session → media state last posted on the inter-module bus.
#[derive(Debug, Default)]
pub struct MediaSync {
    pub identity: Option<crate::media::MediaIdentity>,
    /// `(roster_version, known identities, my display name)` the names were built for.
    pub names_key: Option<(u32, usize, String)>,
    pub names: Vec<(u16, String)>,
}

impl Active {
    pub fn new(group_id: [u8; 16], session: Session, msg_id: u16, now: Instant) -> Self {
        Active {
            group_id,
            links: BTreeMap::new(),
            registry: NodeIdRegistry::new(),
            newest_holders: BTreeSet::new(),
            dedup: Dedup::new(),
            asm: SegmentAssembler::for_mesh(),
            msg_id,
            hellos: BTreeMap::new(),
            spec_last: None,
            spec_backoff: BTreeMap::new(),
            spec_pending: BTreeMap::new(),
            t10: 0,
            radio_epoch: 0,
            join_next: now,
            relayed_joins: BTreeMap::new(),
            session,
            dirty_store: false,
            refused: false,
            media: MediaSync::default(),
            filter: Default::default(),
        }
    }

    /// Up links (admitted).
    pub fn up_links(&self) -> Vec<LinkId> {
        self.links
            .iter()
            .filter(|(_, l)| l.up)
            .map(|(id, _)| *id)
            .collect()
    }

    /// The Up link whose peer is `node`.
    pub fn link_to(&self, node: u16) -> Option<LinkId> {
        self.links
            .iter()
            .find(|(_, l)| l.up && l.peer.is_some_and(|p| p.node_id() == node))
            .map(|(id, _)| *id)
    }

    pub fn next_msg_id(&mut self) -> u16 {
        let m = self.msg_id;
        self.msg_id = self.msg_id.wrapping_add(1);
        m
    }
}
