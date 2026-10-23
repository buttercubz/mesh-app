//! Receive path under realistic BLE arrival traces (first device test, 2026-10-22:
//! "petardeo" between two phones).
//!
//! BLE delivers notifications only at connection events (every 30-50 ms on Android's
//! BALANCED priority, 15-30 ms on iOS), a packet missed by its event waits a whole
//! interval, and relays add one such hop each. 40 ms voice frames therefore arrive
//! bunched with 0-100 ms of jitter. These tests drive the jitter buffer with such traces
//! (deterministic LCG, no OS randomness) and check that frames that did arrive are
//! played, not discarded as late and replaced by concealment.

mod common;

use common::TagDecoder;
use mesh_voice::{
    AudioBandwidth, BLOCK_SAMPLES, BLOCK_US, FrameDuration, JitterBuffer, PullStatus, VoiceCfg,
    VoiceFrame,
};

const FRAME_US: u64 = 40_000;

/// Deterministic 64-bit LCG (Knuth MMIX constants).
struct Lcg(u64);

impl Lcg {
    fn next(&mut self) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        self.0 >> 33
    }

    /// Uniform in `[0, 1)`.
    fn unit(&mut self) -> f64 {
        (self.next() % 1_000_000) as f64 / 1_000_000.0
    }
}

/// One BLE hop: connection events every `ci_us` (phase `phase_us`); a packet goes out
/// at the first event at or after it is ready, and each retry (probability `retry`)
/// costs one more interval.
#[derive(Clone, Copy)]
struct Hop {
    ci_us: u64,
    phase_us: u64,
    retry: f64,
}

impl Hop {
    fn deliver(&self, ready_us: u64, rng: &mut Lcg) -> u64 {
        let k = ready_us.saturating_sub(self.phase_us).div_ceil(self.ci_us);
        let mut t = self.phase_us + k * self.ci_us;
        while rng.unit() < self.retry {
            t += self.ci_us;
        }
        t
    }
}

/// Arrival time (µs) of every frame of a burst of `n` frames, `None` when lost (dropped
/// by a mesh queue). The BLE link layer is reliable and in order: a retried packet holds
/// back the ones queued behind it on the same hop.
fn trace(n: usize, hops: &[Hop], loss: f64, seed: u64) -> Vec<Option<u64>> {
    let mut rng = Lcg(seed);
    let mut last = vec![0u64; hops.len()];
    (0..n)
        .map(|i| {
            // Encoded once the 40 ms frame is complete, plus 2 ms of engine latency.
            let mut t = (i as u64 + 1) * FRAME_US + 2_000;
            if rng.unit() < loss {
                return None;
            }
            for (h, prev) in hops.iter().zip(last.iter_mut()) {
                t = h.deliver(t, &mut rng).max(*prev);
                *prev = t;
            }
            Some(t)
        })
        .collect()
}

fn frame(burst: u8, n: u16) -> VoiceFrame {
    let dur = FrameDuration::Ms40;
    let cfg = VoiceCfg::new(dur, AudioBandwidth::Wb, false, false, n == 0, false);
    VoiceFrame::from_parts(burst, n * dur.ticks(), cfg, &(n + 1).to_le_bytes()).unwrap()
}

/// Statistics of one simulated burst.
#[derive(Debug, Default)]
struct Outcome {
    lost: usize,
    late: u32,
    concealed_frames: u32,
    decoded: u32,
    /// Concealed 10 ms blocks played between two decoded blocks.
    plc_gaps: usize,
    expanded: u32,
}

fn play(arrivals: &[Option<u64>]) -> Outcome {
    let mut jb = JitterBuffer::new();
    let mut dec = TagDecoder::default();
    let mut block = [0i16; BLOCK_SAMPLES];
    let mut order: Vec<(u64, usize)> = arrivals
        .iter()
        .enumerate()
        .filter_map(|(i, a)| a.map(|t| (t, i)))
        .collect();
    order.sort_unstable();
    let end = order.last().map_or(0, |o| o.0) + 400_000;
    let mut next = 0;
    let mut statuses = Vec::new();
    let mut now = 0;
    let mut stopped = false;
    while now <= end {
        while next < order.len() && order[next].0 <= now {
            jb.push(order[next].0, &frame(1, order[next].1 as u16));
            next += 1;
        }
        if !stopped && next == order.len() {
            jb.end_burst(1);
            stopped = true;
        }
        statuses.push(jb.pull(now, &mut dec, &mut block));
        now += BLOCK_US;
    }
    let st = jb.stats();
    let first = statuses.iter().position(|s| *s == PullStatus::Normal);
    let last = statuses.iter().rposition(|s| *s == PullStatus::Normal);
    let plc_gaps = match (first, last) {
        (Some(a), Some(b)) => statuses[a..=b]
            .iter()
            .filter(|s| **s == PullStatus::Plc)
            .count(),
        _ => 0,
    };
    Outcome {
        lost: arrivals.iter().filter(|a| a.is_none()).count(),
        late: st.late,
        concealed_frames: st.concealed,
        decoded: st.decoded,
        plc_gaps,
        expanded: st.expanded,
    }
}

const ANDROID_BALANCED: Hop = Hop {
    ci_us: 45_000,
    phase_us: 7_000,
    retry: 0.08,
};
const IOS_CENTRAL: Hop = Hop {
    ci_us: 30_000,
    phase_us: 19_000,
    retry: 0.05,
};

#[test]
fn two_phones_bunched_ble_arrivals_play_every_received_frame() {
    for seed in 1..=8u64 {
        let arrivals = trace(250, &[ANDROID_BALANCED], 0.02, seed);
        let o = play(&arrivals);
        assert_eq!(o.late, 0, "seed {seed}: no received frame discarded: {o:?}");
        assert_eq!(
            o.decoded as usize + o.lost,
            250,
            "seed {seed}: every received frame decoded: {o:?}"
        );
        assert!(
            o.concealed_frames as usize <= o.lost,
            "seed {seed}: concealment only for lost frames: {o:?}"
        );
        // Starvation is absorbed by stretching (not by skipping audio), and it settles:
        // a 10 s burst stretches at most a few hundred ms in total.
        assert!(
            u64::from(o.expanded) * BLOCK_US <= 300_000,
            "seed {seed}: stretch converges: {o:?}"
        );
    }
}

#[test]
fn relayed_three_phone_chain_keeps_every_received_frame() {
    for seed in 11..=16u64 {
        let arrivals = trace(250, &[ANDROID_BALANCED, IOS_CENTRAL], 0.03, seed);
        let o = play(&arrivals);
        assert_eq!(o.late, 0, "seed {seed}: {o:?}");
        assert_eq!(o.decoded as usize + o.lost, 250, "seed {seed}: {o:?}");
        assert!(o.concealed_frames as usize <= o.lost, "seed {seed}: {o:?}");
    }
}

#[test]
fn lossless_jitter_free_stream_never_conceals_or_stretches() {
    let arrivals: Vec<_> = (0..100u64).map(|i| Some((i + 1) * FRAME_US)).collect();
    let o = play(&arrivals);
    assert_eq!(o.plc_gaps, 0, "{o:?}");
    assert_eq!(o.expanded, 0, "{o:?}");
    assert_eq!(o.decoded, 100);
}

#[test]
fn receive_path_totals_survive_slot_reuse() {
    use mesh_voice::{ReceivePath, SimCodec, StreamKey};
    let codec = SimCodec::new();
    let mut rx = ReceivePath::new(&codec).unwrap();
    let mut enc = Vec::new();
    {
        use mesh_voice::{EncoderConfig, Packetizer, PreRoll, Profile};
        let mut p = Packetizer::new(mesh_voice::Codec::new_encoder(&codec).unwrap());
        let cfg = EncoderConfig::for_profile(Profile::Ble16, 16_000, 0.0);
        p.start_burst(1, &cfg, &PreRoll::new(), &mut |f| enc.push(f.clone()))
            .unwrap();
        for b in common::blocks(&common::sine(400.0, -12.0, 4_000)) {
            p.push_block(&b, &mut |f| enc.push(f.clone())).unwrap();
        }
    }
    let mut now = 0;
    // Seven origins, one after another, through the pool of six slots.
    for origin in 0..7u16 {
        let key = StreamKey { origin, sess: 1 };
        for (n, f) in enc.iter().enumerate() {
            rx.push(now + n as u64 * FRAME_US, key, f);
        }
        rx.talk_stop(key, 1);
        for _ in 0..60 {
            rx.pull(now);
            now += BLOCK_US;
        }
    }
    let t = rx.totals();
    assert_eq!(t.bursts, 7, "{t:?}");
    assert_eq!(t.received as usize, 7 * enc.len(), "{t:?}");
}
