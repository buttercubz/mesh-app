//! The "no link exceeds its cap" global assertion (DESIGN §13.2, §6.3): from the burst
//! spans the receive taps recorded per incoming link, the most bursts one link direction
//! carried at once (a burst is active from its first to its last VOICE frame on that
//! link, end-exclusive: a burst evicted within `T_STICKY` hands over at a frame
//! boundary), against the link kind's `S_MAX`.

use std::collections::BTreeMap;

use crate::audit::{Violation, ViolationKind};

use super::World;

/// An over-cap overlap shorter than this is the in-flight tail of an optimistic start
/// (§6: a talker starts on its local view; the frames already handed to the radio when
/// an eviction or FLOOR_REJECT lands still leave). Two 40 ms frame periods.
pub const STREAM_CAP_GRACE_US: u64 = 80_000;

/// Streams seen on one link direction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LinkStreams {
    /// Receiving node and its local link id.
    pub node: usize,
    pub link: u32,
    pub ble: bool,
    /// Most bursts at once and when.
    pub max: usize,
    pub at_us: u64,
    pub cap: usize,
    /// Longest continuous period above the cap (µs).
    pub over_cap_us: u64,
}

impl World {
    /// Per link direction: the most simultaneous voice bursts it carried.
    pub fn link_streams(&self) -> Vec<LinkStreams> {
        let mut kinds: BTreeMap<(usize, u32), mesh_core::LinkKind> = BTreeMap::new();
        for l in &self.links {
            for e in l.ends {
                kinds.insert((e.node, e.link.0), l.kind);
            }
        }
        let mut out = Vec::new();
        for (i, n) in self.nodes.iter().enumerate() {
            let tap = n.tap.borrow();
            let mut per_link: BTreeMap<u32, Vec<(u64, i32)>> = BTreeMap::new();
            for ((link, _), segs) in &tap.spans {
                let ev = per_link.entry(*link).or_default();
                for (a, b) in segs {
                    ev.push((*a, 1));
                    ev.push(((*b).max(*a + 1), -1));
                }
            }
            for (link, mut ev) in per_link {
                ev.sort_unstable();
                let kind = kinds
                    .get(&(i, link))
                    .copied()
                    .unwrap_or(mesh_core::LinkKind::BleL2cap);
                let lim = self.cfg.limits_for(i).routing;
                let cap = i32::from(if kind.is_ble() {
                    lim.s_max_ble
                } else {
                    lim.s_max_aware
                });
                let (mut cur, mut max, mut at) = (0i32, 0i32, 0u64);
                let (mut over_since, mut over_max) = (None::<u64>, 0u64);
                for (t, d) in ev {
                    cur += d;
                    if cur > max {
                        max = cur;
                        at = t;
                    }
                    match (cur > cap, over_since) {
                        (true, None) => over_since = Some(t),
                        (false, Some(s)) => {
                            over_max = over_max.max(t - s);
                            over_since = None;
                        }
                        _ => {}
                    }
                }
                out.push(LinkStreams {
                    node: i,
                    link,
                    ble: kind.is_ble(),
                    max: max.max(0) as usize,
                    at_us: at,
                    cap: cap.max(0) as usize,
                    over_cap_us: over_max,
                });
            }
        }
        out
    }

    /// Violations of the per-link stream cap.
    pub fn stream_cap_violations(&self) -> Vec<Violation> {
        self.link_streams()
            .into_iter()
            .filter(|s| s.over_cap_us > STREAM_CAP_GRACE_US)
            .map(|s| Violation {
                t: mesh_core::Instant(s.at_us),
                kind: ViolationKind::StreamCap,
                detail: format!(
                    "node {} link {}: {} voice bursts at once > cap {} for {} ms",
                    s.node,
                    s.link,
                    s.max,
                    s.cap,
                    s.over_cap_us / 1_000
                ),
            })
            .collect()
    }
}
