//! Built-in latency estimator (DESIGN §12.6): no clock synchronisation. For every voice
//! frame played, the receiver adds up
//!
//! ```text
//! capture→ear (excl. headset legs) = frame_dur + codec look-ahead + tick alignment
//!                                    + age_at_arrival + jitter-buffer wait + render buffering
//! ```
//!
//! where `age_at_arrival` is the MeshPacket `age` (origin queueing, per-hop residence and
//! one-way estimates) plus the incoming link's RTT/2, and the jitter-buffer wait is the
//! time from the frame's arrival to the playout of its first 10 ms block. p50/p95 are
//! kept overall, per origin and per hop count, shown in the snapshot and exported for
//! ride-recorder files. Fixed-size rings: recording a sample never allocates.

use mesh_api::LatencyInfo;
use serde_json::{Value, json};

/// Samples kept per ring.
pub const WINDOW: usize = 256;
/// Origins tracked individually (the least recently used one is replaced).
pub const MAX_ORIGINS: usize = 8;
/// Hop classes: 1, 2, 3 and ≥ 4 link traversals.
pub const HOP_CLASSES: usize = 4;
/// Samples needed before the snapshot shows an estimate.
pub const MIN_SAMPLES: usize = 10;

/// One per-frame estimate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LatencySample {
    pub origin: u16,
    /// Link traversals from the origin (≥ 1).
    pub hops: u8,
    /// Estimated capture→ear excluding headset legs, µs.
    pub m2e_us: u64,
}

/// The estimate for one frame from its components (µs).
pub const fn estimate_us(
    frame_us: u64,
    lookahead_us: u64,
    age_at_arrival_us: u64,
    jb_wait_us: u64,
) -> u64 {
    frame_us
        + lookahead_us
        + super::TICK_ALIGN_US
        + age_at_arrival_us
        + jb_wait_us
        + super::RENDER_BUFFER_US
}

#[derive(Debug, Clone)]
struct Ring {
    buf: [u32; WINDOW],
    len: usize,
    next: usize,
}

impl Ring {
    const fn new() -> Self {
        Ring {
            buf: [0; WINDOW],
            len: 0,
            next: 0,
        }
    }

    fn push(&mut self, v: u32) {
        self.buf[self.next] = v;
        self.next = (self.next + 1) % WINDOW;
        self.len = (self.len + 1).min(WINDOW);
    }

    /// `(p50, p95)` in µs.
    fn percentiles(&self) -> Option<(u32, u32)> {
        if self.len == 0 {
            return None;
        }
        let mut s = [0u32; WINDOW];
        s[..self.len].copy_from_slice(&self.buf[..self.len]);
        let s = &mut s[..self.len];
        s.sort_unstable();
        let at = |p: usize| {
            s[((self.len * p).div_ceil(100))
                .saturating_sub(1)
                .min(self.len - 1)]
        };
        Some((at(50), at(95)))
    }
}

#[derive(Debug, Clone)]
struct OriginSlot {
    origin: Option<u16>,
    last_seq: u64,
    ring: Ring,
}

/// The estimator of one session.
#[derive(Debug, Clone)]
pub struct LatencyEstimator {
    all: Ring,
    hops: [Ring; HOP_CLASSES],
    origins: [OriginSlot; MAX_ORIGINS],
    count: u64,
}

impl Default for LatencyEstimator {
    fn default() -> Self {
        Self::new()
    }
}

fn ms(us: u32) -> f64 {
    (f64::from(us) / 100.0).round() / 10.0
}

impl LatencyEstimator {
    pub fn new() -> Self {
        LatencyEstimator {
            all: Ring::new(),
            hops: core::array::from_fn(|_| Ring::new()),
            origins: core::array::from_fn(|_| OriginSlot {
                origin: None,
                last_seq: 0,
                ring: Ring::new(),
            }),
            count: 0,
        }
    }

    /// Forgets every sample (new session).
    pub fn clear(&mut self) {
        *self = Self::new();
    }

    /// Frames recorded since the session started.
    pub fn count(&self) -> u64 {
        self.count
    }

    /// Records one frame (no allocation).
    pub fn record(&mut self, s: LatencySample) {
        let v = s.m2e_us.min(u64::from(u32::MAX)) as u32;
        self.count += 1;
        self.all.push(v);
        let h = usize::from(s.hops.max(1)) - 1;
        self.hops[h.min(HOP_CLASSES - 1)].push(v);
        let i = match self.origins.iter().position(|o| o.origin == Some(s.origin)) {
            Some(i) => i,
            None => {
                let i = self
                    .origins
                    .iter()
                    .enumerate()
                    .min_by_key(|(_, o)| (o.origin.is_some(), o.last_seq))
                    .map_or(0, |(i, _)| i);
                self.origins[i] = OriginSlot {
                    origin: Some(s.origin),
                    last_seq: 0,
                    ring: Ring::new(),
                };
                i
            }
        };
        self.origins[i].last_seq = self.count;
        self.origins[i].ring.push(v);
    }

    /// `(p50, p95)` over every origin, µs.
    pub fn overall_us(&self) -> Option<(u32, u32)> {
        self.all.percentiles()
    }

    /// `(p50, p95)` of one origin, µs.
    pub fn origin_us(&self, origin: u16) -> Option<(u32, u32)> {
        self.origins
            .iter()
            .find(|o| o.origin == Some(origin))
            .and_then(|o| o.ring.percentiles())
    }

    /// `(p50, p95)` of frames that crossed `hops` links (4 = four or more), µs.
    pub fn hops_us(&self, hops: u8) -> Option<(u32, u32)> {
        let h = usize::from(hops.max(1)) - 1;
        self.hops[h.min(HOP_CLASSES - 1)].percentiles()
    }

    /// Snapshot view (`null` before [`MIN_SAMPLES`] frames).
    pub fn info(&self) -> Option<LatencyInfo> {
        if self.all.len < MIN_SAMPLES {
            return None;
        }
        let (p50, p95) = self.all.percentiles()?;
        let last = (1..=HOP_CLASSES as u8)
            .rev()
            .find(|&h| self.hops_us(h).is_some())
            .unwrap_or(1);
        let by_hops = (1..=last)
            .map(|h| self.hops_us(h).map_or(0.0, |(_, p)| ms(p)))
            .collect();
        Some(LatencyInfo {
            p50_ms: ms(p50),
            p95_ms: ms(p95),
            by_hops,
        })
    }

    /// Ride-recorder / diagnostics export (JSON object, ms values).
    pub fn export(&self) -> Value {
        let pair = |p: Option<(u32, u32)>| match p {
            Some((a, b)) => json!({ "p50Ms": ms(a), "p95Ms": ms(b) }),
            None => Value::Null,
        };
        let hops: Vec<Value> = (1..=HOP_CLASSES as u8)
            .filter_map(|h| {
                let n = self.hops[usize::from(h) - 1].len;
                (n > 0).then(|| json!({ "hops": h, "n": n, "latency": pair(self.hops_us(h)) }))
            })
            .collect();
        let origins: Vec<Value> = self
            .origins
            .iter()
            .filter_map(|o| {
                let id = o.origin?;
                Some(
                    json!({ "origin": id, "n": o.ring.len, "latency": pair(o.ring.percentiles()) }),
                )
            })
            .collect();
        json!({
            "frames": self.count,
            "overall": pair(self.overall_us()),
            "byHops": hops,
            "byOrigin": origins,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn percentiles_per_class_and_origin() {
        let mut e = LatencyEstimator::new();
        assert!(e.info().is_none());
        for i in 0..100u64 {
            e.record(LatencySample {
                origin: 7,
                hops: 1,
                m2e_us: 100_000 + i * 1_000,
            });
            e.record(LatencySample {
                origin: 9,
                hops: 2,
                m2e_us: 200_000,
            });
        }
        assert_eq!(e.hops_us(1), Some((149_000, 194_000)));
        assert_eq!(e.hops_us(2), Some((200_000, 200_000)));
        assert_eq!(e.origin_us(9), Some((200_000, 200_000)));
        assert!(e.hops_us(3).is_none());
        let info = e.info().unwrap();
        assert_eq!(info.by_hops, vec![194.0, 200.0]);
        assert!(info.p95_ms >= 199.0);
        let x = e.export();
        assert_eq!(x["frames"], 200);
        assert_eq!(x["byOrigin"].as_array().unwrap().len(), 2);
        assert_eq!(
            estimate_us(40_000, 6_500, 10_000, 50_000),
            40_000 + 6_500 + 2_500 + 10_000 + 50_000 + 20_000
        );
    }

    #[test]
    fn origin_slots_are_recycled_lru() {
        let mut e = LatencyEstimator::new();
        for o in 0..(MAX_ORIGINS as u16 + 3) {
            e.record(LatencySample {
                origin: o,
                hops: 9,
                m2e_us: 1,
            });
        }
        assert!(e.origin_us(0).is_none());
        assert!(e.origin_us(MAX_ORIGINS as u16 + 2).is_some());
        assert!(e.hops_us(4).is_some());
    }
}
