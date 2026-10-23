//! NBR records (DESIGN §4.4, §5.1): ingesting a neighbor's report and building ours.
//!
//! An NBR is sent on change (rate limit 4 Hz) whenever the set of good links, any
//! congested flag or any `vcap` changes, at once when a link goes down, and at least
//! every 1 s. A report is fresh for 1.5 s. `vcap` is informational (diagnostics and the
//! origin's `cap_local`), never a voice-pruning input.

use mesh_wire::record::{Nbr, NbrEntry, NbrFlags};

use crate::consts::{NBR_FRESH_US, NBR_HEARTBEAT_US, NBR_MIN_INTERVAL_US};
use crate::types::{Instant, LinkKind, NodeId};

/// One entry of a received NBR.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NbrReportEntry {
    pub node_id: NodeId,
    pub q: u8,
    pub good: bool,
    pub congested: bool,
    /// `None` for the reserved kind value 3.
    pub kind: Option<LinkKind>,
    pub vcap: u8,
}

/// The latest NBR received from one neighbor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NbrReport {
    pub nseq: u8,
    pub at: Instant,
    pub entries: Vec<NbrReportEntry>,
}

impl NbrReport {
    /// From a decoded NBR record received at `now`.
    pub fn from_record(now: Instant, nbr: &Nbr) -> Self {
        NbrReport {
            nseq: nbr.nseq,
            at: now,
            entries: nbr
                .entries()
                .iter()
                .map(|e| NbrReportEntry {
                    node_id: e.node_id,
                    q: e.q,
                    good: e.flags.good(),
                    congested: e.flags.congested(),
                    kind: LinkKind::from_nbr_kind(e.flags.kind()),
                    vcap: e.vcap,
                })
                .collect(),
        }
    }

    /// Fresh: received less than 1.5 s ago.
    pub fn fresh(&self, now: Instant) -> bool {
        now.since(self.at) < NBR_FRESH_US
    }

    /// `true` when the report lists `node` with `good = 1` and `congested = 0` on any
    /// link kind (the ctrl/bulk pruning condition, §5.4).
    pub fn lists_good_uncongested(&self, node: NodeId) -> bool {
        self.entries
            .iter()
            .any(|e| e.node_id == node && e.good && !e.congested)
    }

    /// `true` when the report lists `node` as good (congested or not).
    pub fn lists_good(&self, node: NodeId) -> bool {
        self.entries.iter().any(|e| e.node_id == node && e.good)
    }

    /// Neighbors listed in the report.
    pub fn nodes(&self) -> impl Iterator<Item = NodeId> + '_ {
        self.entries.iter().map(|e| e.node_id)
    }
}

/// What our own NBR says about one of our links.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct OwnLinkEntry {
    pub node_id: NodeId,
    pub kind: LinkKind,
    pub q: u8,
    pub good: bool,
    pub congested: bool,
    pub vcap: u8,
}

impl OwnLinkEntry {
    fn to_wire(self) -> NbrEntry {
        NbrEntry {
            node_id: self.node_id,
            q: self.q,
            flags: NbrFlags::new(self.good, self.congested, self.kind.nbr_kind()),
            vcap: self.vcap,
        }
    }

    /// The part whose change triggers an immediate NBR (good set, congested, vcap).
    fn signature(&self) -> (NodeId, LinkKind, bool, bool, u8) {
        (
            self.node_id,
            self.kind,
            self.good,
            self.congested,
            self.vcap,
        )
    }
}

/// Decides when to send our NBR and builds it.
#[derive(Debug, Clone, Default)]
pub struct NbrBuilder {
    nseq: u8,
    last_sent: Option<Instant>,
    last_signature: Vec<(NodeId, LinkKind, bool, bool, u8)>,
    link_down_pending: bool,
}

impl NbrBuilder {
    pub fn new() -> Self {
        Self::default()
    }

    /// Call when a link goes down: the next [`NbrBuilder::poll`] sends at once.
    pub fn on_link_down(&mut self) {
        self.link_down_pending = true;
    }

    /// Returns the NBR to send now, if one is due. `links` is the current state of our
    /// links (any order; at most 8 are reported, best `q` first).
    pub fn poll(&mut self, now: Instant, links: &[OwnLinkEntry]) -> Option<Nbr> {
        let mut sorted: Vec<OwnLinkEntry> = links.to_vec();
        sorted.sort_by(|a, b| b.q.cmp(&a.q).then(a.cmp(b)));
        sorted.truncate(Nbr::MAX_ENTRIES);
        let mut sig: Vec<_> = sorted.iter().map(OwnLinkEntry::signature).collect();
        sig.sort();
        let changed = sig != self.last_signature;
        let since = self.last_sent.map(|t| now.since(t));
        let due = match since {
            None => true,
            Some(_) if self.link_down_pending => true,
            Some(s) if s >= NBR_HEARTBEAT_US => true,
            Some(s) => changed && s >= NBR_MIN_INTERVAL_US,
        };
        if !due {
            return None;
        }
        let entries: Vec<NbrEntry> = sorted.iter().map(|e| e.to_wire()).collect();
        let nbr = Nbr::new(self.nseq, &entries).ok()?;
        self.nseq = self.nseq.wrapping_add(1);
        self.last_sent = Some(now);
        self.last_signature = sig;
        self.link_down_pending = false;
        Some(nbr)
    }

    /// Next instant at which [`NbrBuilder::poll`] might send (heartbeat or rate limit).
    pub fn next_deadline(&self) -> Option<Instant> {
        self.last_sent.map(|t| t.plus_us(NBR_MIN_INTERVAL_US))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn e(node: NodeId, q: u8, congested: bool, vcap: u8) -> OwnLinkEntry {
        OwnLinkEntry {
            node_id: node,
            kind: LinkKind::Ble,
            q,
            good: q >= 60,
            congested,
            vcap,
        }
    }

    #[test]
    fn send_on_change_heartbeat_and_rate_limit() {
        let mut b = NbrBuilder::new();
        let links = [e(2, 90, false, 2), e(3, 70, false, 2)];
        let t0 = Instant::from_secs(1);
        let first = b.poll(t0, &links).expect("first NBR is sent at once");
        assert_eq!(first.entries()[0].node_id, 2);
        assert_eq!(first.nseq, 0);
        // Unchanged: nothing until the 1 s heartbeat.
        assert!(b.poll(t0.plus_ms(500), &links).is_none());
        let hb = b.poll(t0.plus_ms(1_000), &links).expect("heartbeat");
        assert_eq!(hb.nseq, 1);
        // A vcap change is rate limited to 4 Hz.
        let changed = [e(2, 90, false, 1), e(3, 70, false, 2)];
        assert!(b.poll(t0.plus_ms(1_100), &changed).is_none());
        let c = b.poll(t0.plus_ms(1_250), &changed).expect("on change");
        assert_eq!(c.entries()[0].vcap, 1);
        // A q change alone (good set unchanged) is not a trigger.
        let qonly = [e(2, 85, false, 1), e(3, 70, false, 2)];
        assert!(b.poll(t0.plus_ms(1_600), &qonly).is_none());
        // Link down: at once, regardless of the rate limit.
        b.on_link_down();
        let d = b
            .poll(t0.plus_ms(1_610), &[e(2, 85, false, 1)])
            .expect("down");
        assert_eq!(d.entries().len(), 1);
    }

    #[test]
    fn at_most_eight_entries_best_first() {
        let mut b = NbrBuilder::new();
        let links: Vec<_> = (0..10u16).map(|i| e(i, (i * 10) as u8, false, 2)).collect();
        let n = b.poll(Instant::ZERO, &links).expect("nbr");
        assert_eq!(n.entries().len(), 8);
        assert_eq!(n.entries()[0].node_id, 9);
    }

    #[test]
    fn report_ingest_and_freshness() {
        let wire = Nbr::new(
            7,
            &[
                NbrEntry {
                    node_id: 5,
                    q: 90,
                    flags: NbrFlags::new(true, false, NbrFlags::KIND_BLE),
                    vcap: 2,
                },
                NbrEntry {
                    node_id: 6,
                    q: 90,
                    flags: NbrFlags::new(true, true, NbrFlags::KIND_AWARE),
                    vcap: 1,
                },
            ],
        )
        .expect("nbr");
        let t = Instant::from_secs(3);
        let r = NbrReport::from_record(t, &wire);
        assert!(r.fresh(t.plus_ms(1_499)));
        assert!(!r.fresh(t.plus_ms(1_500)));
        assert!(r.lists_good_uncongested(5));
        assert!(!r.lists_good_uncongested(6));
        assert!(r.lists_good(6));
        assert_eq!(r.entries[1].kind, Some(LinkKind::Aware));
        assert_eq!(r.nodes().collect::<Vec<_>>(), vec![5, 6]);
    }
}
