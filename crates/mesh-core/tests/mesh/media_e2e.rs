//! The real media module through the glue: the group identity reaches the media module
//! (session → media seam), PTT voice crosses a relay and plays two hops away, and a
//! talking relay counts its own burst against each link's cap.

use mesh_api::{TalkMode, UiCommand};
use mesh_core::rt_flags;

use crate::talk::{Talker, run_talking};
use crate::world::World;
use crate::{max_concurrent_bursts, voice_on};

fn go_live_ptt(w: &mut World, i: usize) {
    let r = w.with(i, |e, now| e.handle_command(now, UiCommand::GoLive {}));
    assert!(r.is_ok(), "goLive: {r:?}");
    let r = w.with(i, |e, now| {
        e.handle_command(
            now,
            UiCommand::SetTalkMode {
                mode: TalkMode::Ptt,
            },
        )
    });
    assert!(r.is_ok(), "setTalkMode: {r:?}");
}

fn press(w: &mut World, i: usize, down: bool) {
    w.with(i, |e, now| {
        e.handle_timeout(now);
        e.set_rt_flags(if down { rt_flags::PTT_DOWN } else { 0 });
    });
}

#[test]
fn real_ptt_voice_crosses_a_relay_with_the_group_identity() {
    let mut w = World::new(3, 51);
    w.start_all();
    w.connect(0, 1);
    let l12 = w.connect(1, 2);
    w.run_ms(2_000);
    for i in 0..3 {
        go_live_ptt(&mut w, i);
    }
    w.run_ms(200);
    let start = w.now;
    w.nodes[0].tone = Some((start, start + 2_000_000));
    press(&mut w, 0, true);
    w.run_ms(2_000);
    press(&mut w, 0, false);
    w.run_ms(500);
    // The media module originated with the group node_id (no provisional id): the
    // first-hop origin check passed and node 1 relayed the frames.
    let origin = w.nodes[0].node_id;
    let relayed = voice_on(&w, 1, w.local(l12, 1), origin);
    assert!(relayed.len() >= 40, "relayed {} frames", relayed.len());
    assert!(relayed.iter().all(|(_, h)| h.hops == 1));
    // Two hops away the tone is played out.
    let loud = w.nodes[2]
        .heard
        .iter()
        .filter(|(t, p)| *t > start && *p > 1_000)
        .count();
    assert!(loud > 50, "node 2 played {loud} loud blocks");
}

#[test]
fn a_talking_relay_counts_its_own_burst_against_the_cap() {
    // Relay 1 links tails 0 and 2 and listener 3.
    let mut w = World::new(4, 52);
    w.start_all();
    w.connect(0, 1);
    w.connect(2, 1);
    let l13 = w.connect(1, 3);
    w.run_ms(2_000);
    go_live_ptt(&mut w, 1);
    w.run_ms(200);
    let start = w.now;
    w.nodes[1].tone = Some((start, start + 4_000_000));
    press(&mut w, 1, true);
    w.run_ms(300);
    // Two scripted talkers start after the relay: link 1 → 3 has three candidates.
    let mut t = vec![Talker::new(0, 1), Talker::new(2, 1)];
    run_talking(&mut w, &mut t, 3_000);
    press(&mut w, 1, false);
    let link = w.local(l13, 1);
    let me = w.nodes[1].node_id;
    assert!(
        !voice_on(&w, 1, link, me).is_empty(),
        "the relay's own voice went out"
    );
    let n = max_concurrent_bursts(&w, 1, link);
    assert!(n <= 2, "link 1 → 3 carried {n} bursts at once");
    // One relayed burst shares the link with the relay's own; the other got at most the
    // frame in flight when it was evicted.
    let mut relayed: Vec<usize> = [0usize, 2]
        .iter()
        .map(|k| voice_on(&w, 1, link, w.nodes[*k].node_id).len())
        .collect();
    relayed.sort_unstable();
    assert!(
        relayed[0] <= 1 && relayed[1] >= 60,
        "relayed frames per talker: {relayed:?}"
    );
}
