//! The built-in latency estimator (§12.6) against the simulated ground truth: the time
//! from a tone onset at the talker's microphone (capture-block clock) to the same onset
//! at the listener's ear (render ring included), excluding headset legs.

use mesh_api::TalkMode;
use mesh_core::{Platform, rt_flags};

use super::world::{Source, World};

fn ground_truth_ms(w: &World, onset_us: u64) -> f64 {
    let ear = w.ear(1, onset_us, onset_us + 1_000_000);
    let (t, _) = ear
        .iter()
        .find(|(_, s)| s.unsigned_abs() > 1_000)
        .expect("onset reached the ear");
    (t - onset_us as f64) / 1_000.0
}

fn run(delay_us: u64, salt: u8) -> (f64, mesh_api::LatencyInfo) {
    let mut w = World::new(2, Platform::WebSim, salt);
    w.delay_us = delay_us;
    w.start_all();
    w.connect(0, 1);
    w.go_live(0, TalkMode::Ptt);
    w.go_live(1, TalkMode::Ptt);
    // Let PING/PONG establish the RTT (idle pings every 2 s).
    w.run_until_ms(4_000);
    let onset = 4_100_000 + 437; // not block-aligned on purpose
    w.nodes[0].source = Source::Tone {
        hz: 1_000.0,
        amp: 8_000.0,
        from_us: onset,
        until_us: 60_000_000,
    };
    w.set_flags(0, rt_flags::PTT_DOWN);
    w.run_until_ms(5_500);
    let truth = ground_truth_ms(&w, onset);
    let info = w
        .snapshot(1)
        .and_then(|s| s.latency)
        .expect("latency in the listener's snapshot");
    // The 1 Hz export for the ride recorder (§12.6) carries the same estimator.
    let lat = w.nodes[1]
        .engine
        .ctx()
        .media_latency
        .clone()
        .expect("1 Hz latency export in a session");
    assert!(lat["frames"].as_u64().is_some_and(|n| n > 0), "{lat}");
    assert!(lat["overall"]["p50Ms"].as_f64().is_some(), "{lat}");
    // Leaving the session clears it.
    let now = mesh_core::Instant(w.now);
    w.nodes[1].engine.stop(now);
    assert!(w.nodes[1].engine.ctx().media_latency.is_none());
    (truth, info)
}

#[test]
fn latency_estimate_matches_ground_truth_within_5ms() {
    for (delay, salt) in [(15_000, 21), (4_000, 22), (37_500, 23)] {
        let (truth, info) = run(delay, salt);
        assert!(
            (info.p50_ms - truth).abs() <= 5.0,
            "delay {delay} µs: estimate p50 {} ms vs truth {truth} ms",
            info.p50_ms
        );
        assert!(info.p95_ms >= info.p50_ms);
        assert_eq!(info.by_hops.len(), 1, "every frame crossed one link");
        assert!((info.by_hops[0] - info.p95_ms).abs() < 1e-9);
        // Sanity: frame 40 + look-ahead + tick + link + jitter buffer + render ring.
        let floor = 40.0 + 2.5 + delay as f64 / 1_000.0 + 20.0;
        assert!(truth > floor && truth < floor + 90.0, "{truth}");
    }
}
