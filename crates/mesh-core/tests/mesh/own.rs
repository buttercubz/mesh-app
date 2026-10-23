//! My own packets go through the glue like relayed ones (DESIGN §5.2, §6.3): an own
//! burst needs a slot on each link that would carry it; when one of my links already
//! carries `cap` bursts that outrank it, my frames go nowhere and the media module is
//! told (its talk FSM goes BLOCKED), while the relayed bursts keep their slots.

use mesh_core::media::MediaRequest;
use mesh_wire::mesh::{FloorRejectReason, PacketType};

use crate::talk::{Talker, run_talking};
use crate::voice_on;
use crate::world::World;

#[test]
fn an_own_burst_a_full_link_refuses_goes_nowhere_and_the_media_module_is_told() {
    // Relay 1 links talkers 0 and 2 and listener 3.
    let mut w = World::new(4, 64);
    w.start_all();
    w.connect(0, 1);
    w.connect(2, 1);
    let l13 = w.connect(1, 3);
    w.run_ms(2_500);
    let mut t = vec![Talker::new(0, 1), Talker::new(2, 1)];
    // Both relayed bursts hold link 1 → 3 (cap 2) past T_STICKY.
    run_talking(&mut w, &mut t, 1_000);
    let start = w.now;
    t.push(Talker::new(1, 9));
    run_talking(&mut w, &mut t, 1_000);
    let me = w.nodes[1].node_id;
    let refused: Vec<MediaRequest> = w
        .bus(1)
        .into_iter()
        .filter(|r| matches!(r, MediaRequest::FloorRejected { .. }))
        .collect();
    assert_eq!(
        refused,
        [MediaRequest::FloorRejected {
            burst: 9,
            reason: FloorRejectReason::Cap
        }],
        "reported once"
    );
    let links: Vec<_> = w.nodes[1].engine.ctx().links.iter().map(|l| l.id).collect();
    for l in links {
        assert!(
            voice_on(&w, 1, l, me).is_empty(),
            "my refused burst went out on {l:?}"
        );
    }
    // TALK is control (never admitted): it still went out.
    assert!(
        w.own_on(1, w.local(l13, 1))
            .iter()
            .any(|(p, _)| *p == PacketType::Talk)
    );
    // The relayed bursts kept their slots on link 1 → 3.
    let to3 = w.local(l13, 1);
    for k in [0, 2] {
        let got = voice_on(&w, 1, to3, w.nodes[k].node_id)
            .into_iter()
            .filter(|(at, _)| *at >= start)
            .count();
        assert!(
            got >= 23,
            "talker {k}: {got} frames relayed while I was refused"
        );
    }
}
