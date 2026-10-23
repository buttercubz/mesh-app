//! Jitter buffer (§7.5): proptest over arrival permutations, delay adaptation, LBRR, PLC
//! fade, burst lifecycle; receive path (pool of 6, ≤ 4 mixed by rank).

mod common;

use common::TagDecoder;
use mesh_voice::jitter::{BURST_IDLE_US, MAX_DELAY_US, PushResult};
use mesh_voice::receive::{POOL_SIZE, ReceiveResult};
use mesh_voice::{
    AudioBandwidth, BLOCK_SAMPLES, BLOCK_US, FrameDuration, JitterBuffer, Mixer, PullStatus,
    ReceivePath, SimCodec, StreamKey, VoiceCfg, VoiceFrame,
};
use proptest::prelude::*;

fn frame(burst: u8, n: u16, dur: FrameDuration, lbrr: bool) -> VoiceFrame {
    let ts = n * dur.ticks();
    let cfg = VoiceCfg::new(dur, AudioBandwidth::Wb, lbrr, false, n == 0, false);
    VoiceFrame::from_parts(burst, ts, cfg, &(n + 1).to_le_bytes()).unwrap()
}

/// Runs a burst: `arrivals[n]` = tick (10 ms) at which frame n arrives (None = lost).
/// Returns the first sample of every pulled block plus its status, and the buffer.
fn run(
    arrivals: &[Option<u64>],
    dur: FrameDuration,
    lbrr: bool,
    ticks: u64,
    stop_at: Option<u64>,
) -> (Vec<(i16, PullStatus)>, JitterBuffer, TagDecoder) {
    let mut jb = JitterBuffer::new();
    let mut dec = TagDecoder::default();
    let mut out = Vec::new();
    let mut block = [0i16; BLOCK_SAMPLES];
    for t in 0..ticks {
        let now = t * BLOCK_US;
        for (n, a) in arrivals.iter().enumerate() {
            if *a == Some(t) {
                jb.push(now, &frame(1, n as u16, dur, lbrr));
            }
        }
        if stop_at == Some(t) {
            jb.end_burst(1);
        }
        let st = jb.pull(now, &mut dec, &mut block);
        out.push((block[0], st));
    }
    (out, jb, dec)
}

fn played_tags(out: &[(i16, PullStatus)]) -> Vec<i16> {
    out.iter()
        .filter(|(_, s)| *s == PullStatus::Normal)
        .map(|(v, _)| *v)
        .collect()
}

proptest! {
    /// Any arrival order: frames come out in `ts` order, each at most once (≤ its 10 ms
    /// sub-blocks), every frame is either played or counted late, and with jitter within
    /// the 20 ms slack of the minimum delay nothing is late.
    #[test]
    fn jb_orders_any_arrival_permutation(
        jitter in proptest::collection::vec(0u64..16, 3..50),
        twenty in any::<bool>(),
    ) {
        let dur = if twenty { FrameDuration::Ms20 } else { FrameDuration::Ms40 };
        let k = u64::from(dur.ticks());
        let n = jitter.len();
        let arrivals: Vec<Option<u64>> = jitter.iter().enumerate().map(|(i, j)| Some(i as u64 * k + j)).collect();
        let (out, jb, _) = run(&arrivals, dur, false, n as u64 * k + 200, Some(n as u64 * k + 20));
        let tags = played_tags(&out);
        prop_assert!(tags.windows(2).all(|w| w[0] <= w[1]), "in order: {:?}", tags);
        let mut distinct = tags.clone();
        distinct.dedup();
        for t in &distinct {
            prop_assert!(tags.iter().filter(|x| *x == t).count() <= k as usize);
        }
        let st = jb.stats();
        prop_assert_eq!(st.decoded as usize, distinct.len());
        prop_assert_eq!(st.decoded + st.late, n as u32, "every frame played or late");
        if jitter.iter().max().unwrap() * 10 <= 20 && jitter[0] == *jitter.iter().min().unwrap() {
            prop_assert_eq!(st.late, 0);
            prop_assert_eq!(distinct.len(), n);
        }
        prop_assert!(jb.is_idle(), "burst ended after TALK STOP drained");
    }

    /// Everything delivered before playout in a random order plays fully and in order.
    #[test]
    fn jb_full_permutation_before_playout(order in Just((0u16..30).collect::<Vec<_>>()).prop_shuffle()) {
        let mut jb = JitterBuffer::new();
        let mut dec = TagDecoder::default();
        for &n in &order {
            jb.push(0, &frame(3, n, FrameDuration::Ms40, false));
        }
        jb.end_burst(3);
        let mut block = [0i16; BLOCK_SAMPLES];
        let mut tags = Vec::new();
        for t in 0..300u64 {
            if jb.pull(t * BLOCK_US, &mut dec, &mut block) == PullStatus::Normal {
                tags.push(block[0]);
            }
        }
        let mut d = tags.clone();
        d.dedup();
        prop_assert_eq!(d, (1..=30).collect::<Vec<i16>>());
        prop_assert_eq!(jb.stats().concealed, 0);
    }
}

#[test]
fn initial_delay_and_buffering() {
    // 40 ms frames: D starts at 60 ms → playout of frame 0 starts 20 ms after its arrival.
    let arrivals: Vec<_> = (0..10).map(|i| Some(i * 4)).collect();
    let (out, jb, _) = run(&arrivals, FrameDuration::Ms40, false, 60, None);
    assert_eq!(out[0].1, PullStatus::Buffering);
    assert_eq!(out[1].1, PullStatus::Buffering);
    assert_eq!(out[2], (1, PullStatus::Normal));
    assert_eq!(jb.target_delay_us(), 60_000);
    // 20 ms frames: D = 40 ms.
    let arrivals: Vec<_> = (0..10).map(|i| Some(i * 2)).collect();
    let (out, jb, _) = run(&arrivals, FrameDuration::Ms20, false, 30, None);
    assert_eq!(out[1].1, PullStatus::Buffering);
    assert_eq!(out[2], (1, PullStatus::Normal));
    assert_eq!(jb.target_delay_us(), 40_000);
    // With LBRR the floor is frame + 20 ms + one frame.
    let arrivals: Vec<_> = (0..10).map(|i| Some(i * 2)).collect();
    let (_, jb, _) = run(&arrivals, FrameDuration::Ms20, true, 30, None);
    assert_eq!(jb.target_delay_us(), 60_000);
}

#[test]
fn delay_tracks_p95_and_is_clamped() {
    let mut jb = JitterBuffer::new();
    // Every 10th packet is 100 ms late: p95 = 100 ms → D = 40 + 100 ms.
    for n in 0..60u16 {
        let late = if n % 10 == 5 { 100_000 } else { 0 };
        jb.push(
            u64::from(n) * 40_000 + late,
            &frame(1, n, FrameDuration::Ms40, false),
        );
    }
    assert_eq!(jb.target_delay_us(), 140_000);
    let mut jb = JitterBuffer::new();
    for n in 0..60u16 {
        let late = if n % 3 == 1 { 400_000 } else { 0 };
        jb.push(
            u64::from(n) * 40_000 + late,
            &frame(1, n, FrameDuration::Ms40, false),
        );
    }
    assert_eq!(jb.target_delay_us(), MAX_DELAY_US);
}

#[test]
fn lbrr_recovers_a_lost_frame() {
    let mut arrivals: Vec<_> = (0..12).map(|i| Some(i * 2)).collect();
    arrivals[5] = None;
    let (out, jb, _) = run(&arrivals, FrameDuration::Ms20, true, 40, Some(24));
    let fec: Vec<_> = out.iter().filter(|(_, s)| *s == PullStatus::Fec).collect();
    assert_eq!(
        fec.len(),
        2,
        "two 10 ms blocks of the recovered 20 ms frame"
    );
    assert_eq!(
        fec[0].0,
        1000 + 7 - 1,
        "recovered from frame 6's LBRR (tag 7)"
    );
    assert_eq!(jb.stats().fec_recovered, 1);
    assert_eq!(jb.stats().concealed, 0);
    // Without LBRR the same loss is concealed.
    let (out, jb, _) = run(&arrivals, FrameDuration::Ms20, false, 40, Some(24));
    assert_eq!(out.iter().filter(|(_, s)| *s == PullStatus::Plc).count(), 2);
    assert_eq!(jb.stats().concealed, 1);
}

#[test]
fn long_loss_fades_out_then_burst_ends() {
    let arrivals: Vec<_> = (0..5).map(|i| Some(i * 4)).collect();
    let (out, jb, dec) = run(&arrivals, FrameDuration::Ms40, false, 120, None);
    let first_plc = out.iter().position(|(_, s)| *s == PullStatus::Plc).unwrap();
    // 100 ms of concealment (10 blocks) at full level, then a 20 ms fade, then silence.
    for (v, _) in &out[first_plc..first_plc + 10] {
        assert_eq!(*v, -1);
    }
    assert_eq!(out[first_plc + 10].1, PullStatus::Plc, "fading");
    assert_eq!(out[first_plc + 12].1, PullStatus::Faded);
    let ended = out
        .iter()
        .position(|(_, s)| *s == PullStatus::Ended)
        .unwrap() as u64;
    // Last frame arrived at tick 16; the burst ends 600 ms later.
    assert_eq!(ended * BLOCK_US, 16 * BLOCK_US + BURST_IDLE_US);
    assert!(jb.is_idle());
    assert!(dec.resets >= 2, "decoder reset at burst start and end");
    assert_eq!(out[ended as usize + 1].1, PullStatus::Idle);
}

#[test]
fn new_burst_resets_timeline_and_stale_frames_are_ignored() {
    let mut jb = JitterBuffer::new();
    let mut dec = TagDecoder::default();
    let mut b = [0i16; BLOCK_SAMPLES];
    assert_eq!(
        jb.push(0, &frame(1, 0, FrameDuration::Ms40, false)),
        PushResult::NewBurst
    );
    assert_eq!(
        jb.push(0, &frame(1, 0, FrameDuration::Ms40, false)),
        PushResult::Duplicate
    );
    assert_eq!(
        jb.push(10_000, &frame(1, 1, FrameDuration::Ms40, false)),
        PushResult::Accepted
    );
    for t in 0..12 {
        jb.pull(t * BLOCK_US, &mut dec, &mut b);
    }
    // Frame 1 of burst 1 arriving after it was played is late.
    assert_eq!(
        jb.push(120_000, &frame(1, 1, FrameDuration::Ms40, false)),
        PushResult::Late
    );
    assert_eq!(
        jb.push(120_000, &frame(1, 0, FrameDuration::Ms40, false)),
        PushResult::Late
    );
    // A new burst (ts restarts at 0) resets the timeline.
    assert_eq!(
        jb.push(130_000, &frame(2, 0, FrameDuration::Ms40, false)),
        PushResult::NewBurst
    );
    assert_eq!(jb.current_burst(), Some(2));
    assert_eq!(
        jb.push(140_000, &frame(1, 5, FrameDuration::Ms40, false)),
        PushResult::Stale
    );
    // A frame of a different duration inside a burst is rejected.
    assert_eq!(
        jb.push(150_000, &frame(2, 1, FrameDuration::Ms20, false)),
        PushResult::Stale
    );
    let mut tags = Vec::new();
    for t in 13..30 {
        if jb.pull(t * BLOCK_US, &mut dec, &mut b) == PullStatus::Normal {
            tags.push(b[0]);
        }
    }
    assert_eq!(tags.first(), Some(&1), "burst 2 frame 0 plays");
}

#[test]
fn backlog_drops_quiet_subblocks_at_most_every_100ms() {
    // 400 ms of quiet audio arrives at once (pre-roll + a relink backlog).
    let arrivals: Vec<_> = (0..10).map(|_| Some(0)).collect();
    let (out, jb, _) = run(&arrivals, FrameDuration::Ms40, false, 60, Some(0));
    let dropped = jb.stats().dropped_subblocks;
    assert!(dropped >= 1, "backlog above D + 30 ms is trimmed");
    let normal = out.iter().filter(|(_, s)| *s == PullStatus::Normal).count();
    assert_eq!(normal as u32 + dropped, 40);
    // At most one drop per 100 ms of playout: 40 sub-blocks played in ≤ 40 ticks.
    assert!(dropped <= 4);
}

#[test]
fn receive_path_pool_and_rank_selection() {
    let codec = SimCodec::new();
    let mut rx = ReceivePath::new(&codec).unwrap();
    let mut enc_frames = Vec::new();
    {
        use mesh_voice::{EncoderConfig, Packetizer, PreRoll, Profile};
        let mut p = Packetizer::new(mesh_voice::Codec::new_encoder(&codec).unwrap());
        let cfg = EncoderConfig::for_profile(Profile::Ble16, 16_000, 0.0);
        p.start_burst(1, &cfg, &PreRoll::new(), &mut |f| {
            enc_frames.push(f.clone())
        })
        .unwrap();
        let tone = common::sine(500.0, -12.0, 16_000);
        for b in common::blocks(&tone) {
            p.push_block(&b, &mut |f| enc_frames.push(f.clone()))
                .unwrap();
        }
    }
    let keys: Vec<StreamKey> = (0..7)
        .map(|o| StreamKey {
            origin: 100 + o,
            sess: 1,
        })
        .collect();
    for (i, k) in keys.iter().enumerate().take(POOL_SIZE) {
        assert!(matches!(
            rx.push(0, *k, &enc_frames[0]),
            ReceiveResult::Buffered(PushResult::NewBurst)
        ));
        rx.set_rank(*k, (POOL_SIZE - i) as u32);
    }
    assert_eq!(
        rx.push(0, keys[6], &enc_frames[0]),
        ReceiveResult::NoSlot,
        "pool of 6"
    );
    let mut mixer = Mixer::new();
    let mut out = [0i16; BLOCK_SAMPLES];
    let mut t = 0;
    for (n, f) in enc_frames.iter().enumerate().skip(1).take(10) {
        for k in keys.iter().take(POOL_SIZE) {
            rx.push(n as u64 * 40_000, *k, f);
        }
        for _ in 0..4 {
            rx.pull(t);
            rx.mix_into(&mut mixer, &[], &mut out);
            t += BLOCK_US;
        }
    }
    let sel: Vec<u16> = rx.selected().map(|(k, _)| k.origin).collect();
    assert_eq!(sel, vec![105, 104, 103, 102], "four best ranks, best first");
    assert!(mesh_voice::rms_dbfs(&out) > -30.0, "mixed audio audible");
    assert_eq!(rx.active_streams(), 6);
    // Streams end; a seventh origin then gets a slot.
    for k in keys.iter().take(POOL_SIZE) {
        rx.talk_stop(*k, 1);
    }
    for _ in 0..100 {
        rx.pull(t);
        t += BLOCK_US;
    }
    assert_eq!(rx.active_streams(), 0);
    assert!(matches!(
        rx.push(t, keys[6], &enc_frames[0]),
        ReceiveResult::Buffered(PushResult::NewBurst)
    ));
}
