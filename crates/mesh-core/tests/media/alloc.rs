//! `push_capture` and `pull_playout` do not allocate in steady state (§3.1), including
//! VOX burst starts and stops on the capture path, the receive/mix path with a remote
//! talker, earcons and the latency estimator.

use mesh_api::TalkMode;
use mesh_core::Platform;

use super::world::{Source, World};

#[test]
fn capture_and_playout_paths_do_not_allocate() {
    let mut w = World::new(2, Platform::WebSim, 61);
    w.start_all();
    w.connect(0, 1);
    w.run_ms(300);
    w.go_live(0, TalkMode::Vox);
    w.go_live(1, TalkMode::Vox);
    // Node 0 talks in 400 ms bursts every 1.2 s (VOX onsets and releases on the
    // capture path); node 1 listens.
    let bursts: Vec<Source> = (0..8)
        .map(|k| Source::tone(900.0, 5_000.0, 500 + k * 1_200, 900 + k * 1_200))
        .collect();
    w.nodes[0].source = Source::Sum(bursts);
    // Warm-up: the first burst creates the receive stream and the decoder state.
    w.run_until_ms(2_000);
    w.count_audio = true;
    w.run_until_ms(8_000);
    w.count_audio = false;
    assert!(w.audio_calls > 1_000);
    assert_eq!(
        w.audio_allocs, 0,
        "allocations in push_capture/pull_playout over {} calls",
        w.audio_calls
    );
    // Voice really flowed during the measured window.
    let voice = w
        .sent_packets(0)
        .iter()
        .filter(|(t, _, h, _)| *t > 2_000_000 && h.ptype == mesh_wire::mesh::PacketType::Voice)
        .count();
    assert!(voice > 40, "voice frames sent: {voice}");
}
