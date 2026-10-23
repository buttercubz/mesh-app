//! Shared scaffolding of the group-ride scenarios: forming the ride over the real
//! discovery and secure-link path, readiness checks, and the per-node voice metrics
//! (loss, duplicates, modeled mouth-to-ear per hop count) computed from the receive
//! taps and the onset detectors.

use std::collections::BTreeMap;

use mesh_api::TalkMode;
use mesh_core::Instant;

use crate::onset::{match_onsets, percentile};
use crate::ride::RideGroup;
use crate::talk::TalkSpan;
use crate::tap::unpack_frame;
use crate::world::World;

/// Mouth onsets this close to a span's end are not matched (see [`voice_stats`]).
pub const TAIL_US: u64 = 150_000;

/// Platform audio stages of the §7.9 budget that the simulated platform does not run
/// but every modeled mouth-to-ear sample includes: VPIO/APM processing (5 ms) and the OS
/// render IO (10 ms). The 10 ms OS capture IO and the 20 ms render ring are modeled by
/// the onset time conventions ([`crate::onset`]); headset legs are excluded (§13.2).
pub const PLATFORM_AUDIO_US: u64 = 15_000;

/// Group mode closes a link over the BLE max at its next link-selection round (every
/// 2 s, §5.7) and the close takes a moment: an episode over the max may last this long.
pub const GROUP_LINK_CAP_GRACE_US: u64 = 3_500_000;

/// Prepares a world config for a group ride: counters instead of a full effect log,
/// and the group-mode link-max grace.
pub fn group_config(mut cfg: crate::config::WorldConfig) -> crate::config::WorldConfig {
    cfg.record_effects = false;
    cfg.link_cap_grace_us = Some(GROUP_LINK_CAP_GRACE_US);
    cfg
}

/// How long a ride may take to form before the scenario gives up (simulated ms).
pub const FORM_LIMIT_MS: u64 = 60_000;

/// `true` when every node of `nodes` sees all of them reachable in its snapshot.
pub fn all_reachable(w: &World, nodes: &[usize]) -> bool {
    nodes.iter().all(|&i| {
        w.nodes[i].snapshot.as_ref().is_some_and(|s| {
            usize::from(s.mesh.reachable) >= nodes.len() && s.mesh.total as usize >= nodes.len()
        })
    })
}

/// Forms a ride of every node: `leader` creates, the others join, every session
/// starts, then the world runs until each node reaches every other one (roster
/// complete) or `FORM_LIMIT_MS`. Returns the group and the formation time.
pub fn form_ride(w: &mut World, leader: usize) -> (RideGroup, Option<Instant>) {
    let n = w.nodes.len();
    let members: Vec<usize> = (0..n).filter(|i| *i != leader).collect();
    let g = w.form_group(leader, &members);
    let all: Vec<usize> = (0..n).collect();
    w.start_group_all(&g.id, &all);
    let t = w.run_until_pred(FORM_LIMIT_MS, 250, |w| all_reachable(w, &all));
    (g, t)
}

/// The leader starts the ride and every node goes live in `mode` with a silent
/// microphone (talk spans script the speech) and onset tracking when `onsets`.
pub fn go_live_all(w: &mut World, g: &RideGroup, mode: TalkMode, onsets: bool) -> bool {
    w.start_ride(g.leader);
    w.run_ms(300);
    let mut ok = true;
    for i in 0..w.nodes.len() {
        w.mute_mic(i);
        if onsets {
            w.track_onsets(i);
        }
        ok &= w.go_live(i, mode);
    }
    w.run_ms(300);
    ok
}

/// Voice metrics of a set of talk spans.
#[derive(Debug, Clone, Default)]
pub struct VoiceStats {
    /// Modeled mouth-to-ear samples (µs) per hop count (link traversals).
    pub m2e: BTreeMap<u8, Vec<u64>>,
    /// Frames expected / received per hop count, at listeners in scope.
    pub expected: BTreeMap<u8, u64>,
    pub received: BTreeMap<u8, u64>,
    /// Frames received more than once at a listener (copies beyond the first).
    pub duplicate_copies: u64,
    /// Syllables spoken / matched at the ear (talk-span listeners only).
    pub syllables: u64,
    pub matched: u64,
}

impl VoiceStats {
    /// Overall loss over hop counts `≤ max_hops` (0 when nothing was expected).
    pub fn loss(&self, max_hops: u8) -> f64 {
        let (mut e, mut r) = (0u64, 0u64);
        for (h, x) in &self.expected {
            if *h <= max_hops {
                e += x;
                r += self.received.get(h).copied().unwrap_or(0);
            }
        }
        if e == 0 {
            0.0
        } else {
            1.0 - r as f64 / e as f64
        }
    }

    /// p-th percentile of mouth-to-ear (ms) at exactly `hops` traversals.
    pub fn m2e_ms(&self, hops: u8, p: f64) -> Option<f64> {
        percentile(self.m2e.get(&hops)?, p).map(|v| v as f64 / 1_000.0)
    }

    /// p-th percentile (ms) over hop counts in `lo..=hi`.
    pub fn m2e_range_ms(&self, lo: u8, hi: u8, p: f64) -> Option<f64> {
        let v: Vec<u64> = self
            .m2e
            .range(lo..=hi)
            .flat_map(|(_, v)| v.iter().copied())
            .collect();
        percentile(&v, p).map(|v| v as f64 / 1_000.0)
    }

    /// Samples at `hops`.
    pub fn samples(&self, hops: u8) -> usize {
        self.m2e.get(&hops).map_or(0, Vec::len)
    }
}

/// Which listeners a talk span is measured at, and whether a frame counts.
pub struct Scope<'a> {
    /// Listener filter `(span, listener) → bool`.
    pub listener: &'a dyn Fn(&TalkSpan, usize) -> bool,
    /// Latency is measured only for spans that overlap no other span.
    pub solo_only: bool,
}

/// Collects voice metrics for `spans` (scripted talk) at every listener the scope
/// admits. Frames are attributed to a span by origin and arrival time
/// (`[from, until + 1 s)`); the expected frames of a burst are `ts = 0 ..= max ts`
/// observed at any node, in steps of the frame duration.
pub fn voice_stats(w: &World, g: &RideGroup, spans: &[TalkSpan], scope: &Scope<'_>) -> VoiceStats {
    let mut st = VoiceStats::default();
    let n = w.nodes.len();
    // Highest ts and frame step seen per burst, anywhere.
    let mut bursts: BTreeMap<(u16, u8, u8), (u16, u8)> = BTreeMap::new();
    for node in &w.nodes {
        for (k, f) in &node.tap.borrow().frames {
            let (o, s, b, ts) = unpack_frame(*k);
            let e = bursts.entry((o, s, b)).or_insert((0, f.ticks.max(1)));
            e.0 = e.0.max(ts);
        }
    }
    for (si, sp) in spans.iter().enumerate() {
        let Some(origin) = w.node_id(sp.node, &g.id) else {
            continue;
        };
        let solo = !spans.iter().enumerate().any(|(sj, o)| {
            sj != si && o.from_us < sp.until_us + 300_000 && sp.from_us < o.until_us + 300_000
        });
        let lo = sp.from_us;
        // Frames up to 1 s after the span, but never into the talker's next span.
        let next = spans
            .iter()
            .filter(|o| o.node == sp.node && o.from_us >= sp.until_us)
            .map(|o| o.from_us)
            .min()
            .unwrap_or(u64::MAX);
        let hi = (sp.until_us + 1_000_000).min(next);
        for l in (0..n).filter(|l| *l != sp.node) {
            if !(scope.listener)(sp, l) {
                continue;
            }
            let tap = l_tap(w, l);
            // Frames of this span at the listener: by burst.
            let mut by_burst: BTreeMap<(u8, u8), (u64, Vec<u8>)> = BTreeMap::new();
            for (k, f) in &tap.frames {
                let (o, s, b, _) = unpack_frame(*k);
                if o != origin || f.t < lo || f.t >= hi {
                    continue;
                }
                let e = by_burst.entry((s, b)).or_insert((0, Vec::new()));
                e.0 += 1;
                e.1.push(f.traversals);
                st.duplicate_copies += u64::from(f.copies.saturating_sub(1));
            }
            let mut span_hops: Vec<u8> = Vec::new();
            for ((s, b), (got, hops)) in &by_burst {
                let (max_ts, step) = bursts.get(&(origin, *s, *b)).copied().unwrap_or((0, 4));
                let exp = u64::from(max_ts / u16::from(step.max(1))) + 1;
                let h = median_u8(hops);
                *st.expected.entry(h).or_default() += exp;
                *st.received.entry(h).or_default() += (*got).min(exp);
                span_hops.extend(hops.iter().copied());
            }
            if by_burst.is_empty() {
                continue;
            }
            if scope.solo_only && !solo {
                continue;
            }
            let (Some(md), Some(ed)) = (
                w.nodes[sp.node].audio.mouth.as_ref(),
                w.nodes[l].audio.ear.as_ref(),
            ) else {
                continue;
            };
            let mouth: Vec<u64> = md
                .onsets
                .iter()
                .copied()
                // A syllable that starts in the span's last 150 ms is cut by the PTT
                // release / VOX hangover before its peak leaves; it is not a test of
                // the path.
                .filter(|m| *m >= sp.from_us && *m + TAIL_US < sp.until_us)
                .collect();
            let ear: Vec<u64> = ed
                .onsets
                .iter()
                .copied()
                .filter(|e| *e >= sp.from_us && *e < hi)
                .collect();
            let d = match_onsets(&mouth, &ear, 20_000, 700_000, 60_000);
            st.syllables += mouth.len() as u64;
            st.matched += d.len() as u64;
            let h = median_u8(&span_hops);
            st.m2e
                .entry(h)
                .or_default()
                .extend(d.into_iter().map(|x| x + PLATFORM_AUDIO_US));
        }
    }
    st
}

fn l_tap(w: &World, l: usize) -> std::cell::Ref<'_, crate::tap::TapState> {
    w.nodes[l].tap.borrow()
}

/// Median of small hop counts (0 for none).
pub fn median_u8(v: &[u8]) -> u8 {
    if v.is_empty() {
        return 0;
    }
    let mut s = v.to_vec();
    s.sort_unstable();
    s[s.len() / 2]
}

/// Every listener of every span.
pub fn everyone(_: &TalkSpan, _: usize) -> bool {
    true
}

/// Hop distance from `from` to every node over the up links of the world
/// (`u8::MAX` = not connected).
pub fn hops_from(w: &World, from: usize) -> Vec<u8> {
    let n = w.nodes.len();
    let mut adj: Vec<Vec<usize>> = vec![Vec::new(); n];
    for l in w.links.iter().filter(|l| l.up) {
        adj[l.ends[0].node].push(l.ends[1].node);
        adj[l.ends[1].node].push(l.ends[0].node);
    }
    let mut d = vec![u8::MAX; n];
    d[from] = 0;
    let mut q = std::collections::VecDeque::from([from]);
    while let Some(u) = q.pop_front() {
        for &v in &adj[u] {
            if d[v] == u8::MAX {
                d[v] = d[u].saturating_add(1);
                q.push_back(v);
            }
        }
    }
    d
}

/// Component index of every node over the up links of the world.
pub fn component_of(w: &World) -> Vec<usize> {
    let mut c = vec![0; w.nodes.len()];
    for (k, comp) in w.components().iter().enumerate() {
        for &i in comp {
            c[i] = k;
        }
    }
    c
}

/// `true` when node `i`'s latest snapshot shows member `node` as reachable (direct,
/// relayed or one-way heard).
pub fn sees_reachable(w: &World, i: usize, node: u16) -> bool {
    use mesh_api::MemberStatus as M;
    matches!(
        w.member_status(i, node).map(|s| s.0),
        Some(M::Direct | M::Relayed | M::OneWay)
    )
}

/// When node `i`'s engine last heard member `node` (µs), from its latest snapshot's
/// `lastHeardMs`; `None` when the snapshot does not list the member.
pub fn last_heard_at(w: &World, i: usize, node: u16) -> Option<u64> {
    let n = &w.nodes[i];
    let m = n
        .snapshot
        .as_ref()?
        .members
        .iter()
        .find(|m| m.node_id == node)?;
    Some(
        n.snapshot_at
            .saturating_sub(u64::from(m.last_heard_ms) * 1_000),
    )
}
