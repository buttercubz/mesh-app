//! Voice is never pruned; ctrl/bulk pruning needs a fresh, good, uncongested NBR
//! (DESIGN §5.4, §5.1).

use mesh_wire::mesh::PacketType;

use crate::talk::{Talker, run_talking};
use crate::world::World;
use crate::{nbrs, of_type, voice_on};

/// N = 0, X = 1, M1 = 2, M2 = 3: N is linked to everyone, X to M1 and M2.
fn cluster(salt: u8) -> (World, [usize; 5]) {
    let mut w = World::new(4, salt);
    w.start_all();
    let links = [
        w.connect(0, 1),
        w.connect(0, 2),
        w.connect(0, 3),
        w.connect(1, 2),
        w.connect(1, 3),
    ];
    w.run_ms(3_000);
    (w, links)
}

/// Injects one POSITION (bulk class) at node 0 as its origin.
fn position(w: &mut World) {
    w.position(0);
}

/// POSITIONs X relayed to M1 and M2 since `from`.
fn relayed_positions(w: &World, links: [usize; 5], from: u64) -> (usize, usize) {
    let to_m1 = w.local(links[3], 1);
    let to_m2 = w.local(links[4], 1);
    let n = |l| {
        of_type(w, 1, l, PacketType::Position)
            .into_iter()
            .filter(|(t, _)| *t >= from)
            .count()
    };
    (n(to_m1), n(to_m2))
}

#[test]
fn fresh_nbr_prunes_ctrl_but_never_voice() {
    let (mut w, links) = cluster(21);
    // X holds a fresh NBR from N listing M1 and M2 good and uncongested.
    let n_reports = nbrs(&w, 0);
    assert!(n_reports.iter().any(|(_, _, r)| r.entries().len() == 3));
    let from = w.now;
    for _ in 0..5 {
        position(&mut w);
        w.run_ms(200);
    }
    assert_eq!(
        relayed_positions(&w, links, from),
        (0, 0),
        "N served M1 and M2"
    );
    // Voice from N: X forwards it to both M1 and M2 anyway.
    let origin = w.nodes[0].node_id;
    let mut t = vec![Talker::new(0, 2)];
    run_talking(&mut w, &mut t, 1_000);
    for l in [links[3], links[4]] {
        let got = voice_on(&w, 1, w.local(l, 1), origin).len() as u32;
        assert!(
            got + 1 >= t[0].frames,
            "voice relayed {got} of {}",
            t[0].frames
        );
    }
}

#[test]
fn missing_or_stale_nbr_disables_pruning_but_voice_flows() {
    let (mut w, links) = cluster(22);
    // N's NBRs stop arriving: after 1.5 s X's copy is stale.
    w.strip_nbr = vec![0];
    w.run_ms(1_600);
    let from = w.now;
    position(&mut w);
    w.run_ms(300);
    assert_eq!(
        relayed_positions(&w, links, from),
        (1, 1),
        "stale NBR: no pruning"
    );
    let origin = w.nodes[0].node_id;
    let mut t = vec![Talker::new(0, 4)];
    run_talking(&mut w, &mut t, 1_000);
    for l in [links[3], links[4]] {
        let got = voice_on(&w, 1, w.local(l, 1), origin).len() as u32;
        assert!(
            got + 1 >= t[0].frames,
            "voice with a stale NBR: {got} of {}",
            t[0].frames
        );
    }
    // A world where N's NBR never arrived at all.
    let mut w = World::new(4, 23);
    w.strip_nbr = vec![0];
    w.start_all();
    let links = [
        w.connect(0, 1),
        w.connect(0, 2),
        w.connect(0, 3),
        w.connect(1, 2),
        w.connect(1, 3),
    ];
    w.run_ms(2_000);
    let from = w.now;
    position(&mut w);
    w.run_ms(300);
    assert_eq!(
        relayed_positions(&w, links, from),
        (1, 1),
        "missing NBR: no pruning"
    );
    let mut t = vec![Talker::new(0, 1)];
    run_talking(&mut w, &mut t, 1_000);
    let origin = w.nodes[0].node_id;
    for l in [links[3], links[4]] {
        assert!(voice_on(&w, 1, w.local(l, 1), origin).len() as u32 + 1 >= t[0].frames);
    }
}

#[test]
fn ctrl_pruning_never_skips_a_congested_link() {
    let (mut w, links) = cluster(24);
    // N → M1 is slow: N's voice overflows the platform buffer (WouldBlock) and N
    // reports M1 congested in its NBR.
    w.links[links[1]].rate_bps = 12_000;
    let mut t = vec![Talker::new(0, 3)];
    run_talking(&mut w, &mut t, 1_500);
    let m1 = w.nodes[2].node_id;
    let congested = nbrs(&w, 0).iter().rev().take(3).any(|(_, _, r)| {
        r.entries()
            .iter()
            .any(|e| e.node_id == m1 && e.flags.congested())
    });
    assert!(congested, "N advertises N→M1 congested");
    let from = w.now;
    for _ in 0..3 {
        position(&mut w);
        run_talking(&mut w, &mut t, 100);
    }
    let (to_m1, to_m2) = relayed_positions(&w, links, from);
    assert_eq!(to_m1, 3, "never pruned toward a congested link");
    assert_eq!(to_m2, 0, "still pruned toward a good, uncongested one");
}
