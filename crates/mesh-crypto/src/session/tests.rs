use super::*;
use crate::keyring::EpochKeyring;
use mesh_wire::frame::RejectReason;
use rand_core::SeedableRng;

const GID: [u8; 16] = [0x42; 16];

fn sched(epoch: u32) -> KeySchedule {
    KeySchedule::new(GID, epoch, Secret32::new([epoch as u8 + 1; 32]))
}

struct Peers {
    a: GroupIdentity,
    b: GroupIdentity,
    rng: ChaCha20Rng,
}

fn peers() -> Peers {
    let mut rng = ChaCha20Rng::from_seed([11; 32]);
    Peers {
        a: GroupIdentity::generate(&mut rng),
        b: GroupIdentity::generate(&mut rng),
        rng,
    }
}

/// Runs a full handshake; returns (initiator, responder, msg sizes).
fn handshake(
    p: &mut Peers,
    init_epoch: &KeySchedule,
    ring: &EpochKeyring,
) -> (LinkSession, LinkSession, [usize; 3]) {
    let (mut i, m1) = LinkSession::initiator(&p.a, init_epoch, 1, 0, &mut p.rng).unwrap();
    let mut r = LinkSession::responder(&p.b, ring, None, 1, 0, &mut p.rng);
    let s1 = r.on_handshake(&m1, 1).unwrap();
    let m2 = s1.send.unwrap();
    assert!(!s1.established);
    let s2 = i.on_handshake(&m2, 2).unwrap();
    assert!(s2.established);
    let m3 = s2.send.unwrap();
    let s3 = r.on_handshake(&m3, 3).unwrap();
    assert!(s3.established && s3.send.is_none());
    (i, r, [m1.len(), m2.len(), m3.len()])
}

#[test]
fn two_in_memory_peers_complete_xxpsk1() {
    let mut p = peers();
    let ring = EpochKeyring::new(sched(0));
    let (mut i, mut r, sizes) = handshake(&mut p, &sched(0), &ring);
    // HS header (3 B) + Noise messages 48 / 96 / 64 B.
    assert_eq!(sizes, [51, 99, 67]);
    let (ii, ri) = (*i.info().unwrap(), *r.info().unwrap());
    assert_eq!(ii.handshake_hash, ri.handshake_hash);
    assert_eq!(ii.remote_static, p.b.x_pk());
    assert_eq!(ri.remote_static, p.a.x_pk());
    assert_eq!((ii.epoch, ri.epoch), (0, 0));
    // SEC both ways.
    let f = i.seal(b"hi", 10).unwrap();
    assert_eq!(r.open(&f, 11).unwrap(), b"hi");
    let g = r.seal(b"yo", 12).unwrap();
    assert_eq!(i.open(&g, 13).unwrap(), b"yo");
    // IDPROOF channel binding both ways.
    let pi = i.create_idproof(&p.a).unwrap();
    let pr = r.create_idproof(&p.b).unwrap();
    assert_eq!(r.verify_peer_idproof(&pi).unwrap().id_hash(), p.a.id_hash());
    assert_eq!(i.verify_peer_idproof(&pr).unwrap().id_hash(), p.b.id_hash());
    // A proof made for one role does not verify as the other.
    assert!(i.verify_peer_idproof(&pi).is_err());
    assert!(i.create_idproof(&p.b).is_err());
}

#[test]
fn wrong_psk_or_epoch_rejected_at_msg1_without_static_key() {
    let mut p = peers();
    // Responder knows epoch 1 only; the initiator uses epoch 0.
    let ring = EpochKeyring::new(sched(1));
    let (_, m1) = LinkSession::initiator(&p.a, &sched(0), 1, 0, &mut p.rng).unwrap();
    let mut r = LinkSession::responder(&p.b, &ring, None, 1, 0, &mut p.rng);
    let err = r.on_handshake(&m1, 1).unwrap_err();
    assert_eq!(err, CryptoError::Handshake);
    assert_eq!(err.reject_reason(), Some(RejectReason::HandshakeFailure));
    assert!(r.is_failed());
    // msg1 itself carries no static key: only e (32 B) and an empty payload tag.
    assert_eq!(m1.len(), 3 + 48);
    assert!(!m1.windows(32).any(|w| w == p.a.x_pk()));
    // A stranger in another group with the same secret fails too (salt = group_id).
    let other = KeySchedule::new([0x43; 16], 1, Secret32::new([2; 32]));
    let (_, m1) = LinkSession::initiator(&p.a, &other, 1, 0, &mut p.rng).unwrap();
    let mut r = LinkSession::responder(&p.b, &ring, None, 1, 0, &mut p.rng);
    assert!(r.on_handshake(&m1, 1).is_err());
    // Same secret (hence the same PSK, which does not depend on the epoch number) but
    // a prologue for another epoch: refused (prologue binding).
    let relabelled = KeySchedule::new(GID, 2, Secret32::new([2; 32]));
    assert_eq!(relabelled.psk(), sched(1).psk());
    let (_, m1) = LinkSession::initiator(&p.a, &relabelled, 1, 0, &mut p.rng).unwrap();
    let mut r = LinkSession::responder(&p.b, &ring, None, 1, 0, &mut p.rng);
    assert_eq!(r.on_handshake(&m1, 1), Err(CryptoError::Handshake));
}

#[test]
fn responder_accepts_any_retained_epoch() {
    let mut p = peers();
    let mut ring = EpochKeyring::new(sched(0));
    for e in 1..=3 {
        ring.insert(sched(e));
    }
    let (i, r, _) = handshake(&mut p, &sched(1), &ring);
    assert_eq!(i.info().unwrap().epoch, 1);
    assert_eq!(r.info().unwrap().epoch, 1);
    // Dropped from the ring after 4 newer epochs: refused.
    ring.insert(sched(4));
    ring.insert(sched(5));
    let (_, m1) = LinkSession::initiator(&p.a, &sched(1), 1, 0, &mut p.rng).unwrap();
    let mut r = LinkSession::responder(&p.b, &ring, Some(1), 1, 0, &mut p.rng);
    assert!(r.on_handshake(&m1, 1).is_err());
}

#[test]
fn tampered_messages_and_order_errors_fail_the_session() {
    let mut p = peers();
    let ring = EpochKeyring::new(sched(0));
    let (mut i, m1) = LinkSession::initiator(&p.a, &sched(0), 1, 0, &mut p.rng).unwrap();
    let mut r = LinkSession::responder(&p.b, &ring, None, 1, 0, &mut p.rng);
    // msg3 before msg1.
    assert_eq!(
        r.on_handshake(&[0x00, 3, 1, 0, 0], 0),
        Err(CryptoError::HandshakeOrder)
    );
    assert!(r.is_failed());
    let mut r = LinkSession::responder(&p.b, &ring, None, 1, 0, &mut p.rng);
    let mut m2 = r.on_handshake(&m1, 1).unwrap().send.unwrap();
    m2[50] ^= 1;
    assert_eq!(i.on_handshake(&m2, 2), Err(CryptoError::Handshake));
    assert!(i.is_failed());
    assert_eq!(i.seal(b"x", 0), Err(CryptoError::NotEstablished));
    // Unsupported proto_ver.
    let (_, m1) = LinkSession::initiator(&p.a, &sched(0), 2, 0, &mut p.rng).unwrap();
    let mut r = LinkSession::responder(&p.b, &ring, None, 1, 0, &mut p.rng);
    assert_eq!(r.on_handshake(&m1, 1), Err(CryptoError::Version));
    // Non-HS frames.
    let mut r = LinkSession::responder(&p.b, &ring, None, 1, 0, &mut p.rng);
    assert_eq!(
        r.on_handshake(&[0x03, 1], 0),
        Err(CryptoError::WrongFrameType)
    );
}

#[test]
fn handshake_timeout_and_determinism() {
    let mut p = peers();
    let ring = EpochKeyring::new(sched(0));
    let r = LinkSession::responder(&p.b, &ring, None, 1, 100, &mut p.rng);
    assert!(!r.handshake_timed_out(100 + HANDSHAKE_TIMEOUT_US - 1));
    assert!(r.handshake_timed_out(100 + HANDSHAKE_TIMEOUT_US));
    // Same injected RNG seed → byte-identical handshakes.
    let run = || {
        let mut q = peers();
        let (_, m1) = LinkSession::initiator(&q.a, &sched(0), 1, 0, &mut q.rng).unwrap();
        m1
    };
    assert_eq!(run(), run());
    // Fresh ephemerals per link from the same RNG.
    let (_, x) = LinkSession::initiator(&p.a, &sched(0), 1, 0, &mut p.rng).unwrap();
    let (_, y) = LinkSession::initiator(&p.a, &sched(0), 1, 0, &mut p.rng).unwrap();
    assert_ne!(x, y);
}

#[test]
fn fixed_ephemerals_are_reproducible() {
    let mut p = peers();
    let ring = EpochKeyring::new(sched(0));
    let (mut i, m1) =
        LinkSession::initiator_fixed(&p.a, &sched(0), 1, 0, &mut p.rng, [1; 32]).unwrap();
    let mut r = LinkSession::responder_fixed(&p.b, &ring, 1, 0, &mut p.rng, [2; 32]);
    assert_eq!(&m1[3..35], &crate::prim::x25519_public(&[1; 32]));
    let m2 = r.on_handshake(&m1, 0).unwrap().send.unwrap();
    assert_eq!(&m2[3..35], &crate::prim::x25519_public(&[2; 32]));
    let m3 = i.on_handshake(&m2, 0).unwrap().send.unwrap();
    r.on_handshake(&m3, 0).unwrap();
    let (ki, kr) = (
        i.sec().unwrap().tx_key().clone(),
        r.sec().unwrap().tx_key().clone(),
    );
    assert_ne!(ki, kr, "one key per direction");
    let f = i.seal(b"v", 0).unwrap();
    assert_eq!(r.open(&f, 0).unwrap(), b"v");
}
