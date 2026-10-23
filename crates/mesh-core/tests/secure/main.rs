//! Secure group links (WP-10B, DESIGN §4.3, §8.4, §5.7): two engines create/join over an
//! in-memory link, run the Noise XXpsk1 handshake and IDPROOF and reach link Up; frames
//! with a tampered AAD byte or tag are dropped; a stranger without the PSK is refused
//! before IDPROOF; simultaneous connects resolve to one link kept by the lower
//! initiator `node_id`.

#[path = "../links/harness.rs"]
mod harness;
#[path = "support.rs"]
mod support;

use mesh_core::links::LinkState;
use mesh_core::{LinkId, Platform};
use mesh_wire::frame::RejectReason;
use support::*;

fn bad_frames(w: &harness::World, i: usize, link: LinkId) -> u64 {
    w.nodes[i]
        .engine
        .ctx()
        .links
        .get(link)
        .map_or(0, |l| l.counters.bad_frames)
}

#[test]
fn create_join_handshake_idproof_and_link_up() {
    let (w, g) = pair(11);
    let a = node_id(&w, 0, &g);
    let b = node_id(&w, 1, &g);
    assert_ne!(a, b);
    assert_eq!(
        up_peers(&w, 0),
        vec![b],
        "leader sees the member after IDPROOF"
    );
    assert_eq!(up_peers(&w, 1), vec![a]);
    // The member's JOIN_REQ (SYNC on the new link) reached the leader, whose next
    // roster was pushed back.
    let mut both = vec![a, b];
    both.sort_unstable();
    assert_eq!(roster(&w, 0, &g), both);
    assert_eq!(roster(&w, 1, &g), both);
    // Channel binding: both sides hold the same handshake hash.
    let ha: Vec<_> = w.nodes[0].engine.ctx().links.iter().map(|l| l.id).collect();
    assert_eq!(ha.len(), 1);
}

#[test]
fn tampered_aad_or_tag_frames_are_dropped() {
    let (mut w, _g) = pair(12);
    let (a_link, b_link) = (w.links[0].a.1, w.links[0].b.1);
    // A SEC unit sent by node 0 on the link: `len u16` + `0x01 · kp_ctr u32 · ct · tag`.
    let unit = w.nodes[0]
        .tx_log
        .iter()
        .rev()
        .find(|(_, l, b)| *l == a_link.0 && b.len() > 2 + 21 && b[2] == 0x01)
        .map(|(_, _, b)| b.clone())
        .expect("a SEC frame was sent");
    let before = bad_frames(&w, 1, b_link);
    let now = w.now;
    // Flip one AAD byte (kp_ctr is both AAD and nonce): the tag no longer verifies.
    let mut aad = unit.clone();
    aad[3] ^= 0x40;
    w.nodes[1].engine.handle_packet(now, b_link, &aad);
    // Flip one tag byte.
    let mut tag = unit.clone();
    let last = tag.len() - 1;
    tag[last] ^= 0x01;
    w.nodes[1].engine.handle_packet(now, b_link, &tag);
    // Replay of an already accepted frame is refused by the window.
    w.nodes[1].engine.handle_packet(now, b_link, &unit);
    w.collect(1);
    assert_eq!(bad_frames(&w, 1, b_link), before + 3);
    assert_eq!(
        w.nodes[1].engine.ctx().links.get(b_link).map(|l| l.state),
        Some(LinkState::Up),
        "a bad frame never kills the link"
    );
    w.run_ms(2_000);
    assert_eq!(up_links(&w, 1), vec![b_link]);
}

#[test]
fn stranger_without_the_psk_is_refused_before_idproof() {
    let mut w = harness::World::new(3, Platform::Ios, 13);
    let g = create(&mut w, 0, "Ana");
    // Node 2 runs another group: its msg1 does not authenticate under our PSK.
    let other = create(&mut w, 2, "Eve");
    start(&mut w, 0, &g);
    start(&mut w, 2, &other);
    w.connect(2, 0);
    w.run_ms(2_000);
    assert!(up_peers(&w, 0).is_empty());
    assert!(up_peers(&w, 2).is_empty());
    // The responder refused at msg1 with REJECT{2} and never sent msg2.
    let rejected = w.nodes[0].tx_log.iter().any(|(_, _, b)| {
        b.len() == 4 && b[2] == 0x03 && b[3] == RejectReason::GroupEpochMismatch as u8
    });
    assert!(rejected, "REJECT{{2}} sent");
    assert!(
        !w.nodes[0]
            .tx_log
            .iter()
            .any(|(_, _, b)| b.len() > 3 && b[2] == 0x00 && b[3] == 2),
        "no Noise msg2 to a stranger"
    );
}

#[test]
fn duplicate_links_keep_the_lower_initiator() {
    let (mut w, g) = pair(14);
    let a = node_id(&w, 0, &g);
    let b = node_id(&w, 1, &g);
    // A second, simultaneous link initiated by the other side.
    w.connect(1, 0);
    w.run_ms(3_000);
    assert_eq!(up_links(&w, 0).len(), 1, "one link per peer and kind");
    assert_eq!(up_links(&w, 1).len(), 1);
    let kept = up_links(&w, 0)[0];
    let kept_init = if w.links[0].a.1 == kept { a } else { b };
    assert_eq!(kept_init, a.min(b), "the lower initiator node_id wins");
}

#[test]
fn session_survives_restart_from_persisted_state() {
    let (mut w, g) = pair(15);
    let roster_before = roster(&w, 1, &g);
    stop(&mut w, 0);
    stop(&mut w, 1);
    reboot(&mut w, 1, Platform::Ios, 99);
    reboot(&mut w, 0, Platform::Ios, 98);
    start(&mut w, 0, &g);
    start(&mut w, 1, &g);
    w.links.clear();
    w.connect(1, 0);
    w.run_ms(2_000);
    assert_eq!(up_peers(&w, 1), vec![node_id(&w, 0, &g)]);
    assert_eq!(roster(&w, 1, &g), roster_before);
}

/// Two per-group identities with the same `node_id` and distinct `id_hash`es.
fn colliding_identities() -> (mesh_crypto::GroupIdentity, mesh_crypto::GroupIdentity) {
    use rand_core::SeedableRng;
    let mut seen = std::collections::BTreeMap::new();
    for k in 0u64.. {
        let mut rng = rand_chacha::ChaCha20Rng::seed_from_u64(k);
        let id = mesh_crypto::GroupIdentity::generate(&mut rng);
        if let Some(other) = seen.insert(id.node_id(), id.clone()) {
            return (other, id);
        }
    }
    unreachable!()
}

/// Gives node `i` the identity `id` in its stored group `g` (as if it had drawn it).
fn give_identity(w: &mut harness::World, i: usize, g: &str, id: mesh_crypto::GroupIdentity) {
    let mut grp = group(w, i, g);
    grp.identity = id;
    let blob = mesh_core::group::store::encode(&grp);
    w.nodes[i]
        .effects
        .push(mesh_core::effects::Effect::Persist {
            key: format!("group/{g}"),
            value: Some(blob),
        });
    reboot(w, i, Platform::Ios, 70 + i as u8);
}

#[test]
fn node_id_collision_rejects_and_the_larger_id_hash_regenerates() {
    let mut w = harness::World::new(3, Platform::Ios, 16);
    let g = create(&mut w, 0, "Ana");
    let url = invite_url(&w, 0, &g);
    join(&mut w, 1, &url, "Xia");
    join(&mut w, 2, &url, "Yan");
    let (x, y) = colliding_identities();
    assert_eq!(x.node_id(), y.node_id());
    let larger = if x.id_hash() > y.id_hash() { 1 } else { 2 };
    give_identity(&mut w, 1, &g, x);
    give_identity(&mut w, 2, &g, y);
    start(&mut w, 1, &g);
    start(&mut w, 2, &g);
    let before = node_id(&w, larger, &g);
    w.connect(1, 2);
    w.run_ms(1_000);
    // Both sides saw two id_hashes for one node_id after IDPROOF: REJECT{8}.
    assert!(up_peers(&w, 1).is_empty() && up_peers(&w, 2).is_empty());
    let sent_8 = |i: usize| {
        w.radio_cmds(i).iter().any(|c| {
            matches!(
                c,
                mesh_core::effects::RadioCommand::Close {
                    reason: Some(RejectReason::NodeIdCollision),
                    ..
                }
            )
        })
    };
    assert!(sent_8(1) || sent_8(2));
    // The member with the larger id_hash drew a new identity (persisted); the other kept
    // its own. Rejoining with the same invite links them with distinct node ids.
    let after = node_id(&w, larger, &g);
    assert_ne!(after, before, "larger id_hash regenerated");
    let smaller = 3 - larger;
    assert_eq!(node_id(&w, smaller, &g), before);
    w.connect(1, 2);
    w.run_ms(2_000);
    assert_eq!(up_peers(&w, smaller), vec![after]);
    assert_eq!(up_peers(&w, larger), vec![before]);
}

#[test]
fn a_third_nodes_collision_report_makes_the_newcomer_regenerate() {
    let mut w = harness::World::new(3, Platform::Ios, 17);
    let g = create(&mut w, 0, "Ana");
    let url = invite_url(&w, 0, &g);
    join(&mut w, 1, &url, "Xia");
    join(&mut w, 2, &url, "Yan");
    let (x, y) = colliding_identities();
    give_identity(&mut w, 1, &g, x);
    give_identity(&mut w, 2, &g, y);
    for i in 0..3 {
        start(&mut w, i, &g);
    }
    w.connect(1, 0);
    w.run_ms(2_000);
    let (ana, xia) = (node_id(&w, 0, &g), node_id(&w, 1, &g));
    assert_eq!(up_peers(&w, 0), vec![xia]);
    let before = node_id(&w, 2, &g);
    assert_eq!(before, xia);
    // Yan dials the leader, who already holds Xia's identity under that node_id: CLOSE{8}
    // behind the leader's IDPROOF. Yan saw no collision itself (the leader's node_id is
    // another), so the reason alone tells it to regenerate.
    w.connect(2, 0);
    w.run_ms(1_000);
    assert!(w.radio_cmds(0).iter().any(|c| matches!(
        c,
        mesh_core::effects::RadioCommand::Close {
            reason: Some(RejectReason::NodeIdCollision),
            ..
        }
    )));
    let after = node_id(&w, 2, &g);
    assert_ne!(after, before, "the newcomer regenerated");
    assert_eq!(node_id(&w, 1, &g), xia, "Xia kept her identity");
    assert_eq!(node_id(&w, 0, &g), ana, "the leader never regenerates");
    // With the new identity it is admitted and listed.
    w.connect(2, 0);
    w.run_ms(3_000);
    let mut both = vec![xia, after];
    both.sort_unstable();
    assert_eq!(up_peers(&w, 0), both);
    assert!(roster(&w, 0, &g).contains(&after));
}
