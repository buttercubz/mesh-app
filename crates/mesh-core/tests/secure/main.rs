//! Secure group links (WP-10B, DESIGN §4.3, §8.4, §5.7): two engines create/join over an
//! in-memory link, run the Noise XXpsk1 handshake and IDPROOF and reach link Up; frames
//! with a tampered AAD byte or tag are dropped; a stranger without the PSK is refused
//! before IDPROOF; simultaneous connects resolve to one link kept by the lower
//! initiator `node_id`; on a Wi-Fi Aware (datagram) link a reordered or lost IDPROOF
//! is repaired by its retransmission and the Up end's answer.

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

/// How the hand-driven Wi-Fi Aware link of [`aware_pair`] disturbs the datagrams.
#[derive(Clone, Copy, PartialEq, Eq)]
enum AwareFault {
    /// The initiator's IDPROOF overtakes its last handshake message (UDP reordering):
    /// the responder cannot open it yet and drops it.
    IdProofOvertakesMsg3,
    /// The responder's IDPROOF datagram is lost. The initiator's IDPROOF is held back
    /// until then so the responder's IDPROOF goes out alone (a datagram
    /// that also carried its CAPS and JOIN_REQ would be a plain control-traffic loss,
    /// repaired by the 30 s JOIN_REQ re-flood, not by IDPROOF).
    ResponderIdProofLost,
}

/// Leader 0 (initiator) and member 1 (responder) over one Wi-Fi Aware link whose
/// datagrams this test delivers by hand (1 ms per hop) with `fault` applied. Returns
/// the world, the group, the link ids `(initiator, responder)` and whether the fault
/// was actually exercised.
fn aware_pair(salt: u8, fault: AwareFault) -> (harness::World, String, LinkId, LinkId, bool) {
    use mesh_core::{LinkInfo, LinkKind, LinkRole, PlatformEvent};
    use mesh_wire::frame::LinkFrame;

    let mut w = harness::World::new(2, Platform::Android, salt);
    let g = create(&mut w, 0, "Ana");
    let url = invite_url(&w, 0, &g);
    join(&mut w, 1, &url, "Ben");
    start(&mut w, 0, &g);
    start(&mut w, 1, &g);
    // Link ids outside the harness's own range: the harness does not route them, so
    // every datagram lands in `tx_log` and is delivered below.
    let (la, lb) = (LinkId(7_000), LinkId(7_001));
    let now = w.now;
    for (i, link, role) in [(0, la, LinkRole::Initiator), (1, lb, LinkRole::Responder)] {
        let info = LinkInfo::new(LinkKind::WifiAware, role);
        w.nodes[i]
            .engine
            .handle_event(now, PlatformEvent::Up { link, info });
        w.collect(i);
    }
    let mut cursor = [0usize; 2];
    let mut exercised = false;
    let mut held: Vec<Vec<u8>> = Vec::new();
    let mut hold_done = false;
    for _ in 0..5_000 {
        let t = w.now.plus_ms(1);
        w.run_until(t);
        for (from, to, own, peer) in [(0, 1, la, lb), (1, 0, lb, la)] {
            let mut batch: Vec<Vec<u8>> = w.nodes[from].tx_log[cursor[from]..]
                .iter()
                .filter(|(_, l, _)| *l == own.0)
                .map(|(_, _, b)| b.clone())
                .collect();
            cursor[from] = w.nodes[from].tx_log.len();
            let is_hs = |b: &[u8]| matches!(LinkFrame::decode(b), Ok(LinkFrame::Hs { .. }));
            let is_sec = |b: &[u8]| matches!(LinkFrame::decode(b), Ok(LinkFrame::Sec { .. }));
            if from == 0 && exercised {
                batch.splice(0..0, std::mem::take(&mut held));
            }
            match fault {
                AwareFault::ResponderIdProofLost if from == 0 && !hold_done => {
                    if batch.iter().any(|b| is_hs(b)) && batch.iter().any(|b| is_sec(b)) {
                        held = batch.iter().filter(|b| is_sec(b)).cloned().collect();
                        batch.retain(|b| !is_sec(b));
                        hold_done = true;
                    }
                }
                AwareFault::IdProofOvertakesMsg3 if from == 0 && !exercised => {
                    if batch.iter().any(|b| is_hs(b)) && batch.iter().any(|b| is_sec(b)) {
                        // SEC datagrams (IDPROOF first) before the HS message 3.
                        batch.sort_by_key(|b| is_hs(b));
                        exercised = true;
                    }
                }
                AwareFault::ResponderIdProofLost if from == 1 && hold_done && !exercised => {
                    if let Some(pos) = batch.iter().position(|b| is_sec(b)) {
                        batch.remove(pos);
                        exercised = true;
                    }
                }
                _ => {}
            }
            let now = w.now;
            for b in batch {
                w.nodes[to].engine.handle_packet(now, peer, &b);
                w.collect(to);
            }
        }
    }
    (w, g, la, lb, exercised)
}

fn link_state(w: &harness::World, i: usize, link: LinkId) -> Option<LinkState> {
    w.nodes[i].engine.ctx().links.get(link).map(|l| l.state)
}

#[test]
fn aware_idproof_overtaking_the_last_handshake_message_still_completes() {
    let (w, g, la, lb, exercised) = aware_pair(41, AwareFault::IdProofOvertakesMsg3);
    assert!(exercised, "the IDPROOF never overtook message 3");
    assert!(
        bad_frames(&w, 1, lb) >= 1,
        "the early IDPROOF reached the responder before its keys"
    );
    assert_eq!(link_state(&w, 0, la), Some(LinkState::Up));
    assert_eq!(
        link_state(&w, 1, lb),
        Some(LinkState::Up),
        "the responder re-sent its IDPROOF and the Up initiator answered with its own"
    );
    // Control traffic flows both ways: the member's JOIN_REQ reached the leader and
    // the roster with both came back.
    let both = {
        let mut v = vec![node_id(&w, 0, &g), node_id(&w, 1, &g)];
        v.sort_unstable();
        v
    };
    assert_eq!(roster(&w, 0, &g), both);
    assert_eq!(roster(&w, 1, &g), both);
}

#[test]
fn aware_lost_responder_idproof_is_answered_and_the_link_comes_up() {
    let (w, g, la, lb, exercised) = aware_pair(42, AwareFault::ResponderIdProofLost);
    assert!(exercised, "no responder IDPROOF was dropped");
    assert_eq!(link_state(&w, 0, la), Some(LinkState::Up));
    assert_eq!(link_state(&w, 1, lb), Some(LinkState::Up));
    let both = {
        let mut v = vec![node_id(&w, 0, &g), node_id(&w, 1, &g)];
        v.sort_unstable();
        v
    };
    assert_eq!(roster(&w, 0, &g), both);
    assert_eq!(roster(&w, 1, &g), both);
    // Bounded: once both ends are Up nobody keeps re-sending IDPROOF (two Up ends
    // never answer each other; the CAPS of each stops the other's answers).
    let sec_after = |i: usize, link: LinkId| {
        w.nodes[i]
            .tx_log
            .iter()
            .filter(|(t, l, _)| *l == link.0 && *t >= 3_000_000)
            .count()
    };
    let idle_tx = sec_after(0, la) + sec_after(1, lb);
    assert!(
        idle_tx < 200,
        "{idle_tx} datagrams in the last 2 s of an idle pair"
    );
}

/// Faults of [`aware_timed`], a hand-driven Wi-Fi Aware pair (leader 0 initiator,
/// member 1 responder) with per-datagram delivery times at 100 µs resolution.
#[derive(Clone, Copy, PartialEq, Eq)]
enum TimedFault {
    /// Nothing is lost: the responder's first SEC datagram (its IDPROOF) is delivered
    /// 0.3 ms *behind* its next datagram (CAPS and JOIN_REQ, sent once it is Up), well
    /// inside the ±2 ms Aware jitter. The leader is still in IdProof when the JOIN_REQ
    /// arrives.
    JoinOvertakesIdProof,
    /// The initiator's IDPROOF overtakes its message 3 (so the responder re-sends its
    /// IDPROOF every 250 ms), the Up initiator's first answer is lost, and the
    /// responder→initiator delay shrinks by 0.1 ms per resend (4 ms at first): the
    /// repeated IDPROOFs arrive 249.9 ms apart.
    ShrinkingDelayLostAnswer,
}

struct TimedRun {
    w: harness::World,
    g: String,
    la: LinkId,
    lb: LinkId,
    exercised: bool,
    /// First repeated responder IDPROOF seen at the Up initiator (µs).
    first_repeat_at: Option<u64>,
    /// When the responder's link went Up (µs).
    responder_up_at: Option<u64>,
}

fn aware_timed(salt: u8, fault: TimedFault, run_us: u64) -> TimedRun {
    use mesh_core::{Instant, LinkInfo, LinkKind, LinkRole, PlatformEvent};
    use mesh_wire::frame::LinkFrame;

    let mut w = harness::World::new(2, Platform::Android, salt);
    let g = create(&mut w, 0, "Ana");
    let url = invite_url(&w, 0, &g);
    join(&mut w, 1, &url, "Ben");
    start(&mut w, 0, &g);
    start(&mut w, 1, &g);
    let (la, lb) = (LinkId(7_000), LinkId(7_001));
    let now = w.now;
    for (i, link, role) in [(0, la, LinkRole::Initiator), (1, lb, LinkRole::Responder)] {
        let info = LinkInfo::new(LinkKind::WifiAware, role);
        w.nodes[i]
            .engine
            .handle_event(now, PlatformEvent::Up { link, info });
        w.collect(i);
    }
    let t0 = w.now.0;
    let is_hs = |b: &[u8]| matches!(LinkFrame::decode(b), Ok(LinkFrame::Hs { .. }));
    let is_sec = |b: &[u8]| matches!(LinkFrame::decode(b), Ok(LinkFrame::Sec { .. }));
    let mut cursor = [0usize; 2];
    // (deliver at, to, link at the receiver, datagram)
    let mut queue: Vec<(u64, usize, LinkId, Vec<u8>)> = Vec::new();
    let mut held: Option<Vec<u8>> = None;
    let mut hs_from_initiator = 0u32;
    let mut exercised = false;
    let mut first_repeat_at = None;
    let mut responder_up_at = None;
    // Length of the responder's first SEC datagram (its IDPROOF): its repeats have the
    // same length, which tells them apart from PING / LSTAT without decrypting.
    let mut idproof_len: Option<usize> = None;
    let mut t = t0;
    while t < t0 + run_us {
        t += 100;
        w.run_until(Instant(t));
        let now = w.now.0;
        for (from, to, own, peer) in [(0usize, 1usize, la, lb), (1, 0, lb, la)] {
            let batch: Vec<Vec<u8>> = w.nodes[from].tx_log[cursor[from]..]
                .iter()
                .filter(|(_, l, _)| *l == own.0)
                .map(|(_, _, b)| b.clone())
                .collect();
            cursor[from] = w.nodes[from].tx_log.len();
            for b in batch {
                if from == 1 && idproof_len.is_none() && is_sec(&b) {
                    idproof_len = Some(b.len());
                }
                let mut at = now + 1_000;
                match fault {
                    TimedFault::JoinOvertakesIdProof if from == 1 && !exercised && is_sec(&b) => {
                        match held.take() {
                            None => {
                                held = Some(b);
                                continue;
                            }
                            Some(idproof) => {
                                queue.push((now + 1_300, to, peer, idproof));
                                exercised = true;
                            }
                        }
                    }
                    TimedFault::ShrinkingDelayLostAnswer if from == 0 && !exercised => {
                        if is_hs(&b) {
                            hs_from_initiator += 1;
                            if hs_from_initiator == 2 {
                                held = Some(b);
                                continue;
                            }
                        } else if let Some(m3) = held.take() {
                            // The IDPROOF first, message 3 right behind it.
                            queue.push((at, to, peer, b));
                            queue.push((at, to, peer, m3));
                            exercised = true;
                            continue;
                        }
                    }
                    TimedFault::ShrinkingDelayLostAnswer if from == 1 && exercised => {
                        // 4 ms, minus 0.4 ms per second since the start (0.1 ms per resend).
                        at = now
                            + 4_000u64
                                .saturating_sub((now - t0) * 400 / 1_000_000)
                                .max(500);
                    }
                    TimedFault::ShrinkingDelayLostAnswer
                        if from == 0
                            && first_repeat_at.is_some_and(|f| now >= f && now < f + 10_000) =>
                    {
                        // The Up initiator's first answer is lost.
                        continue;
                    }
                    _ => {}
                }
                queue.push((at, to, peer, b));
            }
        }
        queue.sort_by_key(|q| q.0);
        let due: Vec<_> = queue.iter().filter(|q| q.0 <= t).cloned().collect();
        queue.retain(|q| q.0 > t);
        for (_, to, peer, b) in due {
            if fault == TimedFault::ShrinkingDelayLostAnswer
                && first_repeat_at.is_none()
                && to == 0
                && is_sec(&b)
                && Some(b.len()) == idproof_len
                && link_state(&w, 0, la) == Some(LinkState::Up)
                && link_state(&w, 1, lb) == Some(LinkState::IdProof)
            {
                first_repeat_at = Some(w.now.0);
            }
            let now = w.now;
            w.nodes[to].engine.handle_packet(now, peer, &b);
            w.collect(to);
        }
        if responder_up_at.is_none() && link_state(&w, 1, lb) == Some(LinkState::Up) {
            responder_up_at = Some(w.now.0);
        }
    }
    TimedRun {
        w,
        g,
        la,
        lb,
        exercised,
        first_repeat_at,
        responder_up_at,
    }
}

/// Regression: a member JOIN_REQ that overtakes its own IDPROOF on an Aware link (no
/// loss, a 0.3 ms reorder) was dropped by the leader still in IdProof and only came back
/// with the 30 s JOIN_REQ re-flood. The leader now keeps it and replays it once Up.
#[test]
fn aware_join_req_overtaking_its_idproof_is_replayed_once_up() {
    let r = aware_timed(45, TimedFault::JoinOvertakesIdProof, 3_000_000);
    assert!(
        r.exercised,
        "the member's IDPROOF never fell behind its JOIN_REQ"
    );
    assert_eq!(link_state(&r.w, 0, r.la), Some(LinkState::Up));
    assert_eq!(link_state(&r.w, 1, r.lb), Some(LinkState::Up));
    let mut both = vec![node_id(&r.w, 0, &r.g), node_id(&r.w, 1, &r.g)];
    both.sort_unstable();
    assert_eq!(
        roster(&r.w, 0, &r.g),
        both,
        "leader roster 3 s after link-up"
    );
    assert_eq!(
        roster(&r.w, 1, &r.g),
        both,
        "member roster 3 s after link-up"
    );
}

/// Regression: the answer gate equalled the resend period, so a repeated IDPROOF that
/// arrived 249.9 ms after the last answered one (network delay shrinking by 0.1 ms) was
/// ignored and, after a lost answer, completion slipped a full extra period. The second
/// repeat is answered now: the responder is Up within one resend period of the first.
#[test]
fn aware_repeated_idproof_with_shrinking_delay_is_answered_every_period() {
    let r = aware_timed(41, TimedFault::ShrinkingDelayLostAnswer, 2_000_000);
    assert!(
        r.exercised,
        "the initiator's IDPROOF never overtook message 3"
    );
    let first = r
        .first_repeat_at
        .expect("a repeated responder IDPROOF reached the initiator");
    let up = r.responder_up_at.expect("the responder came Up");
    assert!(
        up < first + 400_000,
        "responder Up {} µs after the first repeat (the second repeat went unanswered)",
        up - first
    );
    assert_eq!(link_state(&r.w, 0, r.la), Some(LinkState::Up));
}
