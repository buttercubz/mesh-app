//! End-to-end runs of `BenchRunner` pairs over a simulated Lab link.

mod common;

use mesh_lab::record::SummaryLine;
use mesh_lab::{
    Anchor, Audio, Bench, BenchRunner, Instant, LabParams, LabRecord, Pattern, RunRole, Verdict,
};

fn summary(lines: &[LabRecord]) -> &SummaryLine {
    match lines.last() {
        Some(LabRecord::Summary(s)) => s,
        other => panic!("last line is not a summary: {other:?}"),
    }
}

fn t0_params(streams: u8) -> LabParams {
    let mut p = LabParams::new(Audio::Full, Anchor::Audio, true);
    p.pattern = Some(Pattern::PingUnderLoad);
    p.streams = Some(streams);
    p.duration_s = Some(20);
    p
}

#[test]
fn t0_voice_like_with_pings_measures_rtt_owd_and_rate() {
    // B's clock is 7.5 s ahead of A's; 20 ms one way, up to 4 ms jitter.
    let mut pair = common::pair(Bench::T0, &t0_params(2), 7_500_000);
    pair.jitter_us = 4_000;
    pair.run(21.0);
    let (a, b) = (summary(&pair.lines_a), summary(&pair.lines_b));
    assert!((a.duration_s - 20.0).abs() < 1e-6, "{}", a.duration_s);
    // 2 voice-like streams = 47.2 kbps uncoalesced, plus pings and answers.
    assert!(
        a.tx_kbps_mean > 47.0 && a.tx_kbps_mean < 60.0,
        "tx {}",
        a.tx_kbps_mean
    );
    assert!(b.loss < 0.001, "loss {}", b.loss);
    let rtt = a.rtt_ms.expect("rtt");
    assert!(rtt[0] >= 40.0 && rtt[0] <= 48.5, "rtt p50 {}", rtt[0]);
    // One-way on B is ~20-24 ms despite the 7.5 s clock offset (NTP estimator).
    let owd = b.owd_ms.expect("owd");
    assert!(owd[0] > 17.0 && owd[1] < 27.0, "owd {owd:?}");
    assert_eq!(a.verdict, Verdict::Go);
    // Every second produced a sample line on both devices.
    let samples = pair
        .lines_a
        .iter()
        .filter(|l| matches!(l, LabRecord::Sample(_)))
        .count();
    assert_eq!(samples, 20);
    // Raw lines carry full four-timestamp pings.
    let raw_full = pair.lines_a.iter().any(|l| match l {
        LabRecord::Raw(r) => r.pings.iter().any(|p| p.t3_remote_s.is_some()),
        _ => false,
    });
    assert!(raw_full);
    // Live numbers reflect the last second.
    let live = pair.a.live();
    assert_eq!(live.links, 1);
    assert!(live.anchor_ok);
}

#[test]
fn t0_loss_is_measured_and_fails_the_threshold() {
    let mut p = t0_params(2);
    p.ping_hz = Some(1);
    let mut pair = common::pair(Bench::T0, &p, 0);
    pair.drop_every = 10;
    pair.run(21.0);
    let b = summary(&pair.lines_b);
    assert!((b.loss - 0.1).abs() < 0.02, "loss {}", b.loss);
    assert_eq!(b.verdict, Verdict::NoGo);
}

#[test]
fn t1_excludes_windows_whose_anchor_dropped() {
    let mut p = LabParams::defaults_for(Bench::T1, true);
    p.duration_s = Some(180);
    let mut pair = common::pair(Bench::T1, &p, 0);
    pair.run(60.0);
    // The phone suspends: no traffic and no confirmed anchor for 60 s.
    pair.anchor_ok = false;
    pair.blackout = true;
    pair.run(60.0);
    pair.anchor_ok = true;
    pair.blackout = false;
    pair.run(61.0);
    let b = summary(&pair.lines_b);
    assert!(b.anchor_drops >= 59, "anchor drops {}", b.anchor_drops);
    assert_eq!(
        b.verdict,
        Verdict::Go,
        "dropped-anchor windows are excluded, not counted as loss"
    );
    assert!(
        pair.lines_b
            .iter()
            .any(|l| matches!(l, LabRecord::Event { ev, .. } if ev == "anchorLost"))
    );
    assert!(
        pair.lines_b
            .iter()
            .any(|l| matches!(l, LabRecord::Event { ev, .. } if ev == "anchorRestored"))
    );
    // Peer GPS and distance are recorded.
    let dist = pair.lines_b.iter().find_map(|l| match l {
        LabRecord::Sample(s) => s.dist_m,
        _ => None,
    });
    assert!(dist.is_some());

    // The same blackout with the anchor holding is radio loss and fails T1.
    let mut pair = common::pair(Bench::T1, &p, 0);
    pair.run(60.0);
    pair.blackout = true;
    pair.run(60.0);
    pair.blackout = false;
    pair.run(61.0);
    assert_eq!(summary(&pair.lines_b).verdict, Verdict::NoGo);
}

#[test]
fn t3_rejoin_times() {
    let mut p = LabParams::defaults_for(Bench::T3, false);
    p.duration_s = Some(60);
    let mut pair = common::pair(Bench::T3, &p, 0);
    pair.run(10.0);
    pair.link_down();
    pair.run(4.0);
    pair.link_up();
    pair.run(47.0);
    let a = summary(&pair.lines_a);
    assert_eq!(a.verdict, Verdict::Go);
    let reconnect = pair.lines_a.iter().find_map(|l| match l {
        LabRecord::Event { ev, fields, .. } if ev == "linkUp" => {
            fields.iter().find(|(k, _)| k == "reconnectS").cloned()
        }
        _ => None,
    });
    assert!(reconnect.is_some());
}

#[test]
fn t5a_loopback_round_trip() {
    for (delay_samples, want) in [(640u64, Verdict::Go), (1_600, Verdict::NoGo)] {
        let p = LabParams::defaults_for(Bench::T5a, false);
        let cfg = common::config(Bench::T5a, &p, RunRole::Controller, 4711);
        let mut r = BenchRunner::new(cfg, Instant(0));
        let mut render: Vec<i16> = Vec::new();
        let mut cap_index = 0u64;
        let mut t = 0u64;
        // 10 ms blocks of 160 samples: capture = render delayed by `delay_samples`.
        while !r.is_finished() && t < 200_000_000 {
            let now = Instant(t);
            if let Some(train) = r.poll_click(now, cap_index) {
                render.resize(cap_index as usize, 0);
                render.extend_from_slice(&train);
            }
            let block: Vec<i16> = (cap_index..cap_index + 160)
                .map(|i| {
                    i.checked_sub(delay_samples)
                        .and_then(|j| render.get(j as usize))
                        .copied()
                        .unwrap_or(0)
                        / 2
                })
                .collect();
            r.on_capture(now, cap_index, &block);
            cap_index += 160;
            if t.is_multiple_of(1_000_000) {
                r.on_platform(now, common::platform(t as f64 / 1e6, true));
            }
            r.tick(now);
            t += 10_000;
        }
        let lines = r.drain();
        let s = summary(&lines);
        let leg = s.per_leg_ms.expect("per leg");
        assert!((leg - delay_samples as f64 / 32.0).abs() < 0.1, "leg {leg}");
        assert_eq!(s.verdict, want);
        assert!(r.clicks_done() >= 50);
    }
}

#[test]
fn t0b_hop_latency_from_mesh_probes() {
    let p = LabParams::defaults_for(Bench::T0b, false);
    let mut origin = BenchRunner::new(
        common::config(Bench::T0b, &p, RunRole::Controller, 4711),
        Instant(0),
    );
    let mut sink = BenchRunner::new(
        common::config(Bench::T0b, &p, RunRole::Follower, 918),
        Instant(0),
    );
    origin.on_link_up(Instant(0), 1, mesh_lab::LinkKind::BleL2cap, None, Some(918));
    sink.on_link_up(
        Instant(0),
        1,
        mesh_lab::LinkKind::BleL2cap,
        None,
        Some(4711),
    );
    let mut t = 0;
    while t <= 60_000_000 {
        let now = Instant(t);
        if let Some(mut probe) = origin.poll_mesh_probe(now) {
            probe.append(7).unwrap();
            // Two hops, 30 ms each.
            sink.on_mesh_probe(Instant(t + 60_000), 4711, &probe, 2, 60_000);
        }
        // Voice-like traffic from origin to sink, delivered instantly.
        while let Some(tx) = origin.poll_tx(now, 1, true) {
            sink.on_rx(now, 1, tx.on_air, &tx.records).unwrap();
        }
        while let Some(tx) = sink.poll_tx(now, 1, true) {
            origin.on_rx(now, 1, tx.on_air, &tx.records).unwrap();
        }
        origin.tick(now);
        sink.tick(now);
        t += 1_000;
    }
    let lines = sink.drain();
    let s = summary(&lines);
    assert_eq!(s.verdict, Verdict::Go);
    let traced = lines.iter().any(|l| match l {
        LabRecord::Raw(r) => r.mesh_probes.iter().any(|m| m.trace == [7] && m.hops == 2),
        _ => false,
    });
    assert!(traced);
}

#[test]
fn t5b_click_trains_only_in_the_echo_return_check() {
    let p = LabParams::defaults_for(Bench::T5b, false);
    for (check, plays) in [("echoReturn", true), ("ducking", false)] {
        let mut cfg = common::config(Bench::T5b, &p, RunRole::Controller, 1);
        cfg.variant = vec![("check".into(), mesh_lab::json::Json::from(check))];
        let mut r = BenchRunner::new(cfg, Instant(0));
        assert_eq!(
            r.poll_click(Instant(2_000_000), 0).is_some(),
            plays,
            "{check}"
        );
    }
}
