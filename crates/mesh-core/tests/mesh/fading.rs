//! A weak link is not a congested one (field test 2026-10-24, DESIGN §5.6, §6.3): at the
//! edge of BLE range or in a ground-reflection null, link-layer retransmissions inflate
//! the RTT for as long as the fade lasts. The delay-based pacer reads that as
//! congestion; it must still keep one stream of the lowest profile on the link, so a
//! lone talker is never refused with `FLOOR_REJECT{CONGESTION}` (the "Network busy"
//! banner) and its burst is never shed while the link still delivers.

use mesh_core::media::MediaRequest;
use mesh_wire::mesh::{FloorRejectReason, PacketType};

use crate::talk::{Talker, run_talking};
use crate::voice_on;
use crate::world::World;

/// One BLE12 stream: `cost(1) / 0.9` (the pacer's voice floor).
const ONE_STREAM_BPS: u32 = 25_112;

#[test]
fn a_lone_talker_keeps_talking_over_a_fading_link() {
    let mut w = World::new(2, 71);
    w.start_all();
    let l = w.connect(0, 1);
    w.run_ms(2_500);
    // The fade: every unit now needs a few connection intervals of retransmissions,
    // ~120 ms one way instead of 10 ms (min_rtt keeps the clean 20 ms), for good.
    w.links[l].delay_us = 120_000;
    let me = w.nodes[0].node_id;
    let to1 = w.local(l, 0);
    let start = w.now;
    // Two bursts: the first spans the whole pacer decrease (60 → floor in ~3 s plus the
    // 2 s cap hold), the second starts on a link already at the floor.
    let mut first = vec![Talker::new(0, 1)];
    run_talking(&mut w, &mut first, 8_000);
    first[0].stop(&mut w);
    w.run_ms(1_000);
    let second_from = w.now;
    let mut second = vec![Talker::new(0, 2)];
    run_talking(&mut w, &mut second, 4_000);
    second[0].stop(&mut w);
    w.run_ms(500);

    let refused: Vec<MediaRequest> = w
        .bus(0)
        .into_iter()
        .filter(|r| matches!(r, MediaRequest::FloorRejected { .. }))
        .collect();
    assert!(refused.is_empty(), "a lone talker was refused: {refused:?}");
    assert!(
        !w.sent(0).iter().any(|(t, _, h, _)| *t >= start
            && h.ptype == PacketType::FloorReject
            && h.origin == me),
        "no FLOOR_REJECT for my own burst"
    );
    // Both bursts kept going out on the weak link (the first one was never shed).
    let sent = voice_on(&w, 0, to1, me);
    let late_first = sent
        .iter()
        .filter(|(t, _)| *t >= start + 6_000_000 && *t < second_from)
        .count();
    assert!(
        late_first >= 40,
        "first burst after the fade settled: {late_first} frames"
    );
    let second_n = sent.iter().filter(|(t, _)| *t >= second_from).count();
    assert!(
        second_n * 100 >= second[0].frames as usize * 90,
        "second burst: {second_n} of {}",
        second[0].frames
    );
    // The pacer sits at one stream, never below.
    let rate = w.nodes[0]
        .engine
        .ctx()
        .links
        .iter()
        .find(|x| x.id == to1)
        .and_then(|x| x.pace.rate_bps)
        .expect("paced");
    assert!(rate >= ONE_STREAM_BPS, "paced at {rate} bps");
}

#[test]
fn over_a_fading_link_the_second_stream_is_refused_never_the_first() {
    // The weak link still carries one stream: a second burst competing for it is a
    // full channel (cap 1 published from pacing), never a refusal of the first one.
    let mut w = World::new(3, 72);
    w.start_all();
    w.connect(0, 1);
    let l12 = w.connect(1, 2);
    w.run_ms(2_500);
    w.links[l12].delay_us = 120_000;
    let mut t = vec![Talker::new(0, 1)];
    run_talking(&mut w, &mut t, 7_000);
    // The relay forwards talker 0 on the weak link 1 → 2 the whole time.
    let me0 = w.nodes[0].node_id;
    let to2 = w.local(l12, 1);
    let since = w.now - 2_000_000;
    let fwd = voice_on(&w, 1, to2, me0)
        .into_iter()
        .filter(|(at, _)| *at >= since)
        .count();
    assert!(fwd >= 45, "relayed on the weak link in the last 2 s: {fwd}");
    // Now the relay itself talks: its link to 2 has cap 1 and carries talker 0.
    t.push(Talker::new(1, 9));
    run_talking(&mut w, &mut t, 1_000);
    let refused: Vec<MediaRequest> = w
        .bus(1)
        .into_iter()
        .filter(|r| matches!(r, MediaRequest::FloorRejected { .. }))
        .collect();
    assert_eq!(
        refused,
        [MediaRequest::FloorRejected {
            burst: 9,
            reason: FloorRejectReason::Congestion
        }],
        "the second stream is the one refused"
    );
    assert!(!t[0].stopped);
}
