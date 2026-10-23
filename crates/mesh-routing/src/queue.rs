//! Per-link strict-priority queues (DESIGN §5.6).
//!
//! - P0 link control (HS, PONG, IDPROOF, CLOSE, NBR on change) and TALK/FLOOR_REJECT;
//! - P1 voice: ≤ 2 frames per origin per link (drop oldest), whole-link voice backlog
//!   ≤ 60 ms of pace-rate drain, deadline `VOICE_MAX_AGE` (checked at dequeue);
//! - P2 presence/position/roster/LSTAT, limited to 10 % of the pace rate while the link
//!   carries voice (voice dequeued within the last 2 s), so a burst of roster segments
//!   never delays a voice stream;
//! - P3 lab/bulk.
//!
//! Records are coalesced up to the frame budget the caller gives. Mesh packets carry
//! their age at enqueue and get `age` rewritten at dequeue (local residence added,
//! §4.5). When the voice backlog overflows, the link sheds the **lowest-ranked burst
//! entirely** ([`ShedEvent`]); the caller stops admitting it on that link, marks the
//! link congested and the next NBR advertises the lower vcap.

use std::collections::VecDeque;

use mesh_wire::mesh::{MeshHeader, TrafficClass, add_age};
use mesh_wire::record::{MAX_RECORDS_PER_FRAME, record_len, rtype};

use crate::consts::{
    P2_SHARE_PERMILLE, VOICE_BACKLOG_US, VOICE_FRAMES_PER_ORIGIN, VOICE_LINK_WINDOW_US,
};
use crate::floor::{BurstKey, FloorTable};
use crate::types::Instant;

/// Queue priority.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Priority {
    P0,
    P1,
    P2,
    P3,
}

impl Priority {
    /// Priority of a mesh packet by type and class.
    pub fn of_mesh(h: &MeshHeader) -> Priority {
        use mesh_wire::mesh::PacketType as T;
        match (h.ptype, h.class) {
            (T::Talk | T::FloorReject, _) => Priority::P0,
            (_, TrafficClass::Voice) => Priority::P1,
            (_, TrafficClass::Lab) => Priority::P3,
            _ => Priority::P2,
        }
    }
}

/// A mesh packet waiting on a link, with its relay rewrite already applied except the
/// residence time.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueuedPacket {
    pub header: MeshHeader,
    pub payload: Vec<u8>,
    pub enqueued_at: Instant,
    /// Age at enqueue (origin age + incoming one-way delay), µs.
    pub base_age_us: u64,
    /// The voice burst (VOICE only).
    pub burst: Option<BurstKey>,
}

impl QueuedPacket {
    /// Age at `now` in µs.
    pub fn age_at(&self, now: Instant) -> u64 {
        self.base_age_us + now.since(self.enqueued_at)
    }

    /// Encoded MESH record body with `age` rewritten for `now`.
    pub fn encode_at(&self, now: Instant) -> Vec<u8> {
        let mut h = self.header;
        h.age = add_age(0, self.age_at(now));
        let mut out = Vec::with_capacity(MeshHeader::LEN + self.payload.len());
        // The header was validated when it was built (ttl/hops ≤ 15, sess ≠ 0).
        if let Ok(hb) = h.encode() {
            out.extend_from_slice(&hb);
        }
        out.extend_from_slice(&self.payload);
        out
    }

    fn body_len(&self) -> usize {
        MeshHeader::LEN + self.payload.len()
    }
}

/// Anything a link sends.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QueueItem {
    Mesh(QueuedPacket),
    /// A link-scoped record (PONG, NBR, LSTAT, SYNC, …) built by the core.
    Record {
        rtype: u8,
        body: Vec<u8>,
    },
}

impl QueueItem {
    fn body_len(&self) -> usize {
        match self {
            QueueItem::Mesh(p) => p.body_len(),
            QueueItem::Record { body, .. } => body.len(),
        }
    }
}

/// A record ready for the SEC/LAB frame.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutRecord {
    pub rtype: u8,
    pub body: Vec<u8>,
    /// The mesh header (MESH records), for stats and voice-link accounting.
    pub mesh: Option<MeshHeader>,
}

/// Voice the link dropped.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShedEvent {
    /// A burst was shed entirely (backlog bound); stop admitting it on this link.
    Burst(BurstKey),
    /// A single voice frame was dropped (per-origin limit or deadline).
    Frame(Option<BurstKey>),
}

/// The queues of one link.
#[derive(Debug, Clone, Default)]
pub struct LinkQueue {
    p0: VecDeque<QueueItem>,
    voice: VecDeque<QueuedPacket>,
    p2: VecDeque<QueueItem>,
    p3: VecDeque<QueueItem>,
    p2_tokens_ubits: u64,
    p2_last: Instant,
    last_voice_pop: Option<Instant>,
    pub voice_max_age_us: u64,
}

impl LinkQueue {
    pub fn new(voice_max_age_us: u64) -> Self {
        LinkQueue {
            voice_max_age_us,
            ..Self::default()
        }
    }

    /// Voice bytes queued.
    pub fn voice_bytes(&self) -> usize {
        self.voice.iter().map(|p| record_len(p.body_len())).sum()
    }

    pub fn is_empty(&self) -> bool {
        self.p0.is_empty() && self.voice.is_empty() && self.p2.is_empty() && self.p3.is_empty()
    }

    pub fn len(&self) -> usize {
        self.p0.len() + self.voice.len() + self.p2.len() + self.p3.len()
    }

    /// Enqueues `item`. `rate_bps` is the link's drain rate; `table` ranks bursts for
    /// stream-level shedding.
    pub fn push(
        &mut self,
        prio: Priority,
        item: QueueItem,
        rate_bps: u32,
        table: &FloorTable,
    ) -> Vec<ShedEvent> {
        match (prio, item) {
            (Priority::P1, QueueItem::Mesh(p)) => self.push_voice(p, rate_bps, table),
            (Priority::P1 | Priority::P0, it) => {
                self.p0.push_back(it);
                Vec::new()
            }
            (Priority::P2, it) => {
                self.p2.push_back(it);
                Vec::new()
            }
            (Priority::P3, it) => {
                self.p3.push_back(it);
                Vec::new()
            }
        }
    }

    fn push_voice(&mut self, p: QueuedPacket, rate_bps: u32, table: &FloorTable) -> Vec<ShedEvent> {
        let mut ev = Vec::new();
        let origin = p.header.origin;
        let same: Vec<usize> = (0..self.voice.len())
            .filter(|&i| self.voice[i].header.origin == origin)
            .collect();
        if same.len() >= VOICE_FRAMES_PER_ORIGIN
            && let Some(old) = self.voice.remove(same[0])
        {
            ev.push(ShedEvent::Frame(old.burst));
        }
        self.voice.push_back(p);
        let limit = (u64::from(rate_bps) * VOICE_BACKLOG_US / 8_000_000) as usize;
        while self.voice_bytes() > limit && !self.voice.is_empty() {
            // Shed the lowest-ranked burst present in the queue, entirely.
            let victim = self.voice.iter().filter_map(|q| q.burst).max_by(|a, b| {
                match (table.get(*a), table.get(*b)) {
                    (Some(x), Some(y)) => x.rank.cmp(&y.rank),
                    (None, Some(_)) => core::cmp::Ordering::Greater,
                    (Some(_), None) => core::cmp::Ordering::Less,
                    (None, None) => a.cmp(b),
                }
            });
            match victim {
                Some(k) => {
                    self.voice.retain(|q| q.burst != Some(k));
                    ev.push(ShedEvent::Burst(k));
                }
                None => {
                    let old = self.voice.pop_front();
                    ev.push(ShedEvent::Frame(old.and_then(|o| o.burst)));
                }
            }
        }
        ev
    }

    /// Pops the records of one frame: at most `budget` bytes of records (each counted as
    /// `record_len`) and 32 records, strict priority. Expired voice is dropped and
    /// reported in `dropped`.
    pub fn pop_frame(
        &mut self,
        now: Instant,
        budget: usize,
        rate_bps: u32,
        dropped: &mut Vec<ShedEvent>,
    ) -> Vec<OutRecord> {
        self.refill_p2(now, rate_bps);
        let mut out = Vec::new();
        let mut used = 0usize;
        let fits = |used: usize, len: usize, n: usize| {
            used + record_len(len) <= budget && n < MAX_RECORDS_PER_FRAME
        };
        while let Some(it) = self.p0.front() {
            if !fits(used, it.body_len(), out.len()) {
                return out;
            }
            used += record_len(it.body_len());
            if let Some(it) = self.p0.pop_front() {
                out.push(Self::emit(now, it));
            }
        }
        while let Some(p) = self.voice.front() {
            if p.age_at(now) > self.voice_max_age_us {
                let old = self.voice.pop_front();
                dropped.push(ShedEvent::Frame(old.and_then(|o| o.burst)));
                continue;
            }
            if !fits(used, p.body_len(), out.len()) {
                return out;
            }
            used += record_len(p.body_len());
            if let Some(p) = self.voice.pop_front() {
                self.last_voice_pop = Some(now);
                out.push(Self::emit(now, QueueItem::Mesh(p)));
            }
        }
        let voice_active = self
            .last_voice_pop
            .is_some_and(|t| now.since(t) < VOICE_LINK_WINDOW_US);
        while let Some(it) = self.p2.front() {
            let len = record_len(it.body_len());
            if !fits(used, it.body_len(), out.len()) {
                return out;
            }
            let bits = (len as u64) * 8 * 1_000_000;
            if voice_active && self.p2_tokens_ubits < bits {
                return out;
            }
            self.p2_tokens_ubits = self.p2_tokens_ubits.saturating_sub(bits);
            used += len;
            if let Some(it) = self.p2.pop_front() {
                out.push(Self::emit(now, it));
            }
        }
        while let Some(it) = self.p3.front() {
            if !fits(used, it.body_len(), out.len()) {
                return out;
            }
            used += record_len(it.body_len());
            if let Some(it) = self.p3.pop_front() {
                out.push(Self::emit(now, it));
            }
        }
        out
    }

    /// Size in record bytes of the next frame-worth of queued items (for pacer checks).
    pub fn head_len(&self) -> Option<usize> {
        self.p0
            .front()
            .map(QueueItem::body_len)
            .or_else(|| self.voice.front().map(QueuedPacket::body_len))
            .or_else(|| self.p2.front().map(QueueItem::body_len))
            .or_else(|| self.p3.front().map(QueueItem::body_len))
            .map(record_len)
    }

    fn refill_p2(&mut self, now: Instant, rate_bps: u32) {
        let share = u64::from(rate_bps) * u64::from(P2_SHARE_PERMILLE) / 1_000;
        let dt = now.since(self.p2_last);
        self.p2_last = self.p2_last.max(now);
        // Bucket depth: 100 ms at the P2 share, at least one 1 KB record.
        let depth = (share * 100_000).max(1_100 * 8 * 1_000_000);
        self.p2_tokens_ubits = (self.p2_tokens_ubits + share * dt).min(depth);
    }

    fn emit(now: Instant, it: QueueItem) -> OutRecord {
        match it {
            QueueItem::Mesh(p) => OutRecord {
                rtype: rtype::MESH,
                body: p.encode_at(now),
                mesh: Some(p.header),
            },
            QueueItem::Record { rtype, body } => OutRecord {
                rtype,
                body,
                mesh: None,
            },
        }
    }

    /// Drops every queued frame of `burst` (shed on this link).
    pub fn drop_burst(&mut self, burst: BurstKey) {
        self.voice.retain(|q| q.burst != Some(burst));
    }
}

#[cfg(test)]
#[path = "queue_tests.rs"]
mod tests;
