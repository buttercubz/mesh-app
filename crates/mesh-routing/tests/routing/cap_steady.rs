//! Cap steady state (DESIGN §5.6, WP-09 acceptance): with BLE16, default `R`, no loss and
//! no queueing delay, a chain interior node (2 voice links), a 3-link hub (3 voice links)
//! and a 3-link node (2 voice + 1 idle) keep cap 2 on every link for 10 simulated
//! minutes; one injected `WouldBlock` lowers the cap for at most 2 s + recovery.

use std::collections::{BTreeMap, VecDeque};

use mesh_routing::{
    Instant, Limits, LinkId, LinkKind, LinkRole, NeighborTable, NodePacing, PacerPath,
};

const RTT_US: u64 = 30_000;
/// One BLE16 VOICE frame on air (§4.8) and the control reserve per 40 ms (3 kbps).
const VOICE_FRAME: u32 = 118;
const CTRL_PER_40MS: u32 = 15;

struct Sim {
    table: NeighborTable,
    pacing: NodePacing,
    queues: BTreeMap<LinkId, VecDeque<u32>>,
    voice: BTreeMap<LinkId, bool>,
    now: Instant,
    /// Published cap per link after every tick.
    trace: Vec<(Instant, LinkId, u8)>,
}

impl Sim {
    fn new(voice_links: u32, idle_links: u32) -> Self {
        let now = Instant::ZERO;
        let mut table = NeighborTable::new();
        let mut pacing = NodePacing::new(Limits::V1);
        let mut queues = BTreeMap::new();
        let mut voice = BTreeMap::new();
        for i in 0..(voice_links + idle_links) {
            let l = LinkId(i + 1);
            table
                .link_up(
                    now,
                    (i + 2) as u16,
                    l,
                    LinkKind::Ble,
                    LinkRole::Initiator,
                    1024,
                )
                .expect("up");
            table.on_rtt(now, l, RTT_US);
            pacing.add_link(now, l, PacerPath::L2cap, 1024);
            queues.insert(l, VecDeque::new());
            voice.insert(l, i < voice_links);
        }
        Sim {
            table,
            pacing,
            queues,
            voice,
            now,
            trace: Vec::new(),
        }
    }

    /// Runs 1 ms steps until `until`; `would_block(now, link)` injects `WouldBlock`s.
    fn run(&mut self, until: Instant, would_block: &dyn Fn(Instant, LinkId) -> bool) {
        while self.now < until {
            let ms = self.now.us() / 1_000;
            let links: Vec<LinkId> = self.queues.keys().copied().collect();
            for &l in &links {
                if ms.is_multiple_of(40) {
                    let q = self.queues.get_mut(&l).expect("q");
                    if self.voice[&l] {
                        // Two talkers' frames, not phase aligned (uncoalesced, §4.8).
                        q.push_back(VOICE_FRAME);
                        self.table.mark_voice_tx(self.now, l);
                    }
                    q.push_back(CTRL_PER_40MS);
                }
                if self.voice[&l] && ms % 40 == 20 {
                    self.queues.get_mut(&l).expect("q").push_back(VOICE_FRAME);
                }
                if would_block(self.now, l) {
                    self.pacing.on_would_block(l);
                }
                let pacer = &mut self.pacing.link_mut(l).expect("p").pacer;
                let q = self.queues.get_mut(&l).expect("q");
                if !q.is_empty() {
                    pacer.note_queued();
                }
                while let Some(&b) = q.front() {
                    if !pacer.can_send(self.now, b) {
                        break;
                    }
                    pacer.on_sent(self.now, b);
                    q.pop_front();
                }
                if ms.is_multiple_of(500) {
                    self.table.on_rtt(self.now, l, RTT_US);
                }
            }
            if ms.is_multiple_of(10) {
                self.pacing.tick(self.now, &self.table);
                for &l in &links {
                    let cap = self.pacing.link(l).expect("l").published_cap;
                    self.trace.push((self.now, l, cap));
                }
            }
            self.now = self.now.plus_ms(1);
        }
    }

    /// Longest contiguous time any link published a cap below 2, in µs.
    fn longest_low(&self, from: Instant) -> u64 {
        let mut since: BTreeMap<LinkId, Instant> = BTreeMap::new();
        let mut longest = 0;
        for &(t, l, cap) in self.trace.iter().filter(|x| x.0 >= from) {
            if cap < 2 {
                let s = *since.entry(l).or_insert(t);
                longest = longest.max(t.since(s));
            } else {
                since.remove(&l);
            }
        }
        longest
    }

    fn all_caps_two_after(&self, from: Instant) -> bool {
        self.trace.iter().filter(|x| x.0 >= from).all(|x| x.2 == 2)
    }
}

const TEN_MIN: Instant = Instant::from_secs(600);

fn steady(voice: u32, idle: u32) {
    let mut s = Sim::new(voice, idle);
    s.run(TEN_MIN, &|_, _| false);
    assert!(
        s.all_caps_two_after(Instant::ZERO),
        "{voice} voice + {idle} idle: cap left 2"
    );
    // R never needed to move and no link was starved.
    for l in s.queues.keys() {
        let p = &s.pacing.link(*l).expect("l").pacer;
        assert!(p.rate_bps() >= 60_000, "R {}", p.rate_bps());
        assert!(s.queues[l].len() < 4, "queue built up on {l:?}");
    }
}

#[test]
fn chain_interior_two_voice_links() {
    steady(2, 0);
}

#[test]
fn hub_three_voice_links() {
    steady(3, 0);
}

#[test]
fn two_voice_plus_one_idle() {
    steady(2, 1);
}

#[test]
fn one_would_block_recovers() {
    for (voice, idle) in [(2, 0), (3, 0), (2, 1)] {
        let mut s = Sim::new(voice, idle);
        let hit = Instant::from_secs(300);
        s.run(TEN_MIN, &|now, l| now == hit && l == LinkId(1));
        let low = s.longest_low(Instant::ZERO);
        assert!(
            low <= 2_000_000 + 2_000_000,
            "{voice}+{idle}: cap low for {low} µs"
        );
        assert!(s.all_caps_two_after(hit.plus_ms(10_000)));
    }
}

#[test]
fn sustained_would_block_drops_then_recovers() {
    let mut s = Sim::new(3, 0);
    let from = Instant::from_secs(60);
    let to = Instant::from_secs(64);
    // WouldBlock on every step for 4 s: R collapses, the published cap drops after the
    // 2 s hold, then recovers once R is back (5 s up-hold).
    s.run(Instant::from_secs(120), &|now, l| {
        l == LinkId(1) && now >= from && now < to
    });
    let dropped = s
        .trace
        .iter()
        .any(|&(t, l, c)| l == LinkId(1) && c < 2 && t < to.plus_ms(1_000));
    assert!(dropped, "the cap must drop under sustained WouldBlock");
    let first_low = s
        .trace
        .iter()
        .find(|&&(_, l, c)| l == LinkId(1) && c < 2)
        .map(|x| x.0)
        .expect("low");
    assert!(
        first_low.since(from) >= 2_000_000,
        "dropped before the 2 s hold"
    );
    assert!(s.all_caps_two_after(Instant::from_secs(100)));
}
