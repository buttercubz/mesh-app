//! Segmentation (DESIGN §4.6 `SEG` flag, §4.4 SYNC): payload `msg_id u16` ·
//! `seg u8 = idx<<4 | (count−1)` · data (≤ 900 B), at most 16 segments.
//!
//! [`SegmentAssembler`] is the bounded, time-injected reassembler used both for flooded
//! segments (keyed by `(origin, sess)` + `msg_id`, ≤ 4 concurrent per origin) and for
//! link-scoped SYNC chunks (keyed by kind + `msg_id`, ≤ 2 concurrent per link), with a
//! 5 s timeout.

use std::collections::BTreeMap;

use crate::codec::{PutLe, Reader, check_max};
use crate::error::{Result, WireError};

/// Largest data per segment or SYNC chunk.
pub const MAX_SEGMENT_DATA: usize = 900;
/// Largest segment count.
pub const MAX_SEGMENTS: usize = 16;
/// Largest logical body.
pub const MAX_LOGICAL_BODY: usize = MAX_SEGMENT_DATA * MAX_SEGMENTS;
/// Reassembly timeout (µs).
pub const REASSEMBLY_TIMEOUT_US: u64 = 5_000_000;
/// Concurrent assemblies per origin (flooded segments).
pub const MAX_ASSEMBLIES_PER_ORIGIN: usize = 4;
/// Concurrent SYNC assemblies per link.
pub const MAX_SYNC_ASSEMBLIES_PER_LINK: usize = 2;
/// Segment header: `msg_id u16` + `seg u8`.
pub const SEGMENT_HEADER: usize = 3;

/// Packs `idx<<4 | (count−1)`; requires `1 ≤ count ≤ 16` and `idx < count`.
pub fn seg_byte(idx: u8, count: u8) -> Result<u8> {
    if count == 0 || usize::from(count) > MAX_SEGMENTS {
        return Err(WireError::BadValue {
            what: "segment count",
            value: u32::from(count),
        });
    }
    if idx >= count {
        return Err(WireError::BadValue {
            what: "segment idx",
            value: u32::from(idx),
        });
    }
    Ok((idx << 4) | (count - 1))
}

/// Unpacks a `seg` byte into `(idx, count)`, refusing `idx ≥ count`.
pub fn split_seg_byte(b: u8) -> Result<(u8, u8)> {
    let idx = b >> 4;
    let count = (b & 0x0F) + 1;
    if idx >= count {
        return Err(WireError::BadValue {
            what: "segment idx",
            value: u32::from(idx),
        });
    }
    Ok((idx, count))
}

/// One segment of a logical body (payload of a MeshPacket with the SEG flag).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Segment<'a> {
    pub msg_id: u16,
    pub idx: u8,
    pub count: u8,
    pub data: &'a [u8],
}

impl<'a> Segment<'a> {
    pub fn decode(p: &'a [u8]) -> Result<Self> {
        let mut r = Reader::new(p);
        let msg_id = r.u16("segment msg_id")?;
        let (idx, count) = split_seg_byte(r.u8("segment seg")?)?;
        let data = r.rest();
        check_max("segment data", data.len(), MAX_SEGMENT_DATA)?;
        Ok(Segment {
            msg_id,
            idx,
            count,
            data,
        })
    }

    pub fn encode(&self) -> Result<Vec<u8>> {
        check_max("segment data", self.data.len(), MAX_SEGMENT_DATA)?;
        let mut o = Vec::with_capacity(SEGMENT_HEADER + self.data.len());
        o.put_u16(self.msg_id);
        o.put_u8(seg_byte(self.idx, self.count)?);
        o.put(self.data);
        Ok(o)
    }
}

/// Splits a logical body into ≤ 16 segments of ≤ 900 B (all full except the last).
pub fn segment_body(msg_id: u16, body: &[u8]) -> Result<Vec<Segment<'_>>> {
    if body.is_empty() {
        return Err(WireError::TooShort {
            what: "logical body",
            len: 0,
            min: 1,
        });
    }
    check_max("logical body", body.len(), MAX_LOGICAL_BODY)?;
    let count = body.len().div_ceil(MAX_SEGMENT_DATA) as u8;
    Ok(body
        .chunks(MAX_SEGMENT_DATA)
        .enumerate()
        .map(|(i, data)| Segment {
            msg_id,
            idx: i as u8,
            count,
            data,
        })
        .collect())
}

/// Splits a logical body into SYNC chunks (same rules as segments).
pub fn sync_chunks(
    kind: crate::record::SyncKind,
    msg_id: u16,
    body: &[u8],
) -> Result<Vec<crate::record::Sync<'_>>> {
    Ok(segment_body(msg_id, body)?
        .into_iter()
        .map(|s| crate::record::Sync {
            kind,
            msg_id,
            idx: s.idx,
            count: s.count,
            chunk: s.data,
        })
        .collect())
}

/// What one segment did to the assembler.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AssemblyEvent {
    /// The logical body is complete.
    Complete(Vec<u8>),
    /// Stored; more segments are needed.
    Pending,
    /// Already held (duplicate index); ignored.
    Duplicate,
    /// `count` differs from the assembly in progress: the assembly was discarded.
    Inconsistent,
}

#[derive(Debug, Clone)]
struct Assembly {
    count: u8,
    started_us: u64,
    received: u8,
    parts: Vec<Option<Vec<u8>>>,
}

/// Bounded reassembler. `G` groups assemblies for the concurrency limit (an origin
/// `(origin, sess)` for flooded segments, a SYNC kind on one link). Time is injected.
#[derive(Debug, Clone)]
pub struct SegmentAssembler<G: Ord + Copy> {
    max_per_group: usize,
    timeout_us: u64,
    slots: BTreeMap<(G, u16), Assembly>,
}

impl<G: Ord + Copy> SegmentAssembler<G> {
    /// An assembler allowing `max_per_group` concurrent assemblies per group.
    pub fn new(max_per_group: usize, timeout_us: u64) -> Self {
        SegmentAssembler {
            max_per_group: max_per_group.max(1),
            timeout_us,
            slots: BTreeMap::new(),
        }
    }

    /// The flooded-segment assembler (≤ 4 per origin, 5 s).
    pub fn for_mesh() -> Self {
        Self::new(MAX_ASSEMBLIES_PER_ORIGIN, REASSEMBLY_TIMEOUT_US)
    }

    /// The per-link SYNC assembler (≤ 2 per link, 5 s); use one per link.
    pub fn for_sync() -> Self {
        Self::new(MAX_SYNC_ASSEMBLIES_PER_LINK, REASSEMBLY_TIMEOUT_US)
    }

    /// Assemblies in progress.
    pub fn in_progress(&self) -> usize {
        self.slots.len()
    }

    /// Drops assemblies older than the timeout.
    pub fn expire(&mut self, now_us: u64) {
        let timeout = self.timeout_us;
        self.slots
            .retain(|_, a| now_us.saturating_sub(a.started_us) < timeout);
    }

    /// Absorbs one segment of group `group`.
    pub fn push(&mut self, now_us: u64, group: G, seg: &Segment<'_>) -> Result<AssemblyEvent> {
        check_max("segment data", seg.data.len(), MAX_SEGMENT_DATA)?;
        seg_byte(seg.idx, seg.count)?;
        self.expire(now_us);
        if seg.count == 1 {
            return Ok(AssemblyEvent::Complete(seg.data.to_vec()));
        }
        let key = (group, seg.msg_id);
        if let Some(a) = self.slots.get(&key)
            && a.count != seg.count
        {
            self.slots.remove(&key);
            return Ok(AssemblyEvent::Inconsistent);
        }
        if !self.slots.contains_key(&key) {
            self.make_room(group);
            self.slots.insert(
                key,
                Assembly {
                    count: seg.count,
                    started_us: now_us,
                    received: 0,
                    parts: vec![None; usize::from(seg.count)],
                },
            );
        }
        let Some(a) = self.slots.get_mut(&key) else {
            return Ok(AssemblyEvent::Pending);
        };
        let Some(slot) = a.parts.get_mut(usize::from(seg.idx)) else {
            return Ok(AssemblyEvent::Inconsistent);
        };
        if slot.is_some() {
            return Ok(AssemblyEvent::Duplicate);
        }
        *slot = Some(seg.data.to_vec());
        a.received += 1;
        if a.received < a.count {
            return Ok(AssemblyEvent::Pending);
        }
        let done = self.slots.remove(&key).map(|a| a.parts).unwrap_or_default();
        Ok(AssemblyEvent::Complete(
            done.into_iter().flatten().flatten().collect(),
        ))
    }

    /// Evicts the oldest assembly of `group` while the group is at its limit.
    fn make_room(&mut self, group: G) {
        loop {
            let in_group: Vec<((G, u16), u64)> = self
                .slots
                .iter()
                .filter(|((g, _), _)| *g == group)
                .map(|(k, a)| (*k, a.started_us))
                .collect();
            if in_group.len() < self.max_per_group {
                return;
            }
            let Some((oldest, _)) = in_group.into_iter().min_by_key(|(k, t)| (*t, k.1)) else {
                return;
            };
            self.slots.remove(&oldest);
        }
    }
}
