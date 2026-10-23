use super::*;
use rand_chacha::ChaCha20Rng;
use rand_core::{RngCore, SeedableRng};

fn pair() -> (SecChannel, SecChannel) {
    let a2b = Secret32::new([1; 32]);
    let b2a = Secret32::new([2; 32]);
    (
        SecChannel::new(a2b.clone(), b2a.clone(), 0),
        SecChannel::new(b2a, a2b, 0),
    )
}

#[test]
fn round_trip_both_directions() {
    let (mut a, mut b) = pair();
    let f = a.seal(b"records", 10).unwrap();
    assert_eq!(f.len(), 7 + 21);
    assert_eq!(f[0], 0x01);
    assert_eq!(b.open(&f, 11).unwrap(), b"records");
    let g = b.seal(b"back", 12).unwrap();
    assert_eq!(a.open(&g, 13).unwrap(), b"back");
    // Replay is refused.
    assert_eq!(b.open(&f, 14), Err(CryptoError::Replay));
    // The other direction's key cannot open it.
    let h = a.seal(b"x", 15).unwrap();
    assert!(a.open(&h, 16).is_err());
}

#[test]
fn flipped_lf_type_or_kp_ctr_byte_fails_the_tag() {
    let (mut a, mut b) = pair();
    let f = a.seal(b"hello world", 0).unwrap();
    for i in 1..5 {
        for bit in 0..8 {
            let mut bad = f.clone();
            bad[i] ^= 1 << bit;
            let before = b.clone();
            assert!(b.open(&bad, 0).is_err(), "kp_ctr byte {i} bit {bit}");
            assert_eq!(before, b, "no state change on a forged frame");
        }
    }
    // lf_type is authenticated through the AAD: re-labelled frames do not open as SEC,
    // and the AAD itself binds 0x01.
    let mut relabel = f.clone();
    relabel[0] = 0x02;
    assert!(b.open(&relabel, 0).is_err());
    let kp = KpCtr(u32::from_le_bytes([f[1], f[2], f[3], f[4]]));
    let mut buf = f[5..f.len() - 16].to_vec();
    let mut tag = [0u8; 16];
    tag.copy_from_slice(&f[f.len() - 16..]);
    let mut aad = kp.aad();
    aad[0] = 0x02;
    assert!(prim::aead_open(&[1; 32], &kp.nonce(), &aad, &mut buf, &tag).is_err());
    // A flipped ciphertext or tag byte fails too, and the genuine frame still opens.
    let mut bad = f.clone();
    let last = bad.len() - 1;
    bad[last] ^= 0x80;
    assert_eq!(b.open(&bad, 0), Err(CryptoError::Decrypt));
    assert_eq!(b.open(&f, 0).unwrap(), b"hello world");
}

#[test]
fn tag_failure_does_not_touch_the_replay_window() {
    let (mut a, mut b) = pair();
    let f0 = a.seal(b"0", 0).unwrap();
    let mut forged = f0.clone();
    forged[5] ^= 1;
    assert_eq!(b.open(&forged, 0), Err(CryptoError::Decrypt));
    // ctr 0 was not marked: the genuine frame is still accepted.
    assert_eq!(b.open(&f0, 0).unwrap(), b"0");
    // A forged far-future counter does not slide the window either.
    let f1 = a.seal(b"1", 0).unwrap();
    let mut far = f1.clone();
    far[1..5].copy_from_slice(&0x0010_0000u32.to_le_bytes());
    assert!(b.open(&far, 0).is_err());
    assert_eq!(b.open(&f1, 0).unwrap(), b"1");
}

#[test]
fn key_update_after_12h_both_phases_decrypt() {
    let (mut a, mut b) = pair();
    let old: Vec<Vec<u8>> = (0..3).map(|i| a.seal(&[i], 1).unwrap()).collect();
    assert_eq!(b.open(&old[0], 2).unwrap(), [0]);
    let t = KEY_UPDATE_AFTER_US + 5;
    let new0 = a.seal(b"new", t).unwrap();
    assert_eq!(a.tx_state(), (true, 1));
    assert_eq!(new0[4] & 0x80, 0x80, "phase bit set");
    assert_eq!(b.open(&new0, t).unwrap(), b"new");
    assert_eq!(b.rx_state(), (true, true));
    // Reordered old-phase frames: accepted within 10 s ...
    assert_eq!(b.open(&old[1], t + OLD_KEY_GRACE_US - 1).unwrap(), [1]);
    assert_eq!(b.open(&old[1], t + 1), Err(CryptoError::Replay));
    // ... rejected after.
    assert!(b.open(&old[2], t + OLD_KEY_GRACE_US).is_err());
    assert_eq!(b.rx_state(), (true, false));
    // Each phase has its own window: counter 0 of the new phase was accepted once.
    assert_eq!(
        b.open(&new0, t + OLD_KEY_GRACE_US),
        Err(CryptoError::Replay)
    );
    let new1 = a.seal(b"more", t + 20).unwrap();
    assert_eq!(b.open(&new1, t + OLD_KEY_GRACE_US + 1).unwrap(), b"more");
}

#[test]
fn key_update_at_2_pow_30_frames() {
    let (mut a, mut b) = pair();
    a.tx.ctr = KEY_UPDATE_CTR - 1;
    let last = a.seal(b"last", 0).unwrap();
    assert_eq!(a.tx_state(), (false, KEY_UPDATE_CTR));
    let first = a.seal(b"first", 0).unwrap();
    assert_eq!(a.tx_state(), (true, 1));
    assert_eq!(&first[1..5], &KpCtr::new(true, 0).0.to_le_bytes());
    assert_eq!(b.open(&last, 0).unwrap(), b"last");
    assert_eq!(b.open(&first, 0).unwrap(), b"first");
    // Two successive updates chain through HKDF.
    let k1 = next_key(&Secret32::new([1; 32]));
    assert_eq!(a.tx.key, k1);
    assert_ne!(next_key(&k1), k1);
}

#[test]
fn forged_next_phase_frame_does_not_commit() {
    let (mut a, mut b) = pair();
    let ok = a.seal(b"x", 0).unwrap();
    let mut forged = ok.clone();
    forged[4] |= 0x80;
    let before = b.clone();
    assert_eq!(b.open(&forged, 0), Err(CryptoError::Decrypt));
    assert_eq!(before, b);
    assert_eq!(b.open(&ok, 0).unwrap(), b"x");
}

#[test]
fn rejects_oversize_and_wrong_types() {
    let (mut a, mut b) = pair();
    assert!(a.seal(&[0u8; 1180], 0).is_err());
    assert!(a.seal(&[0u8; 1179], 0).is_ok());
    assert_eq!(b.open(&[0x03, 0x07], 0), Err(CryptoError::WrongFrameType));
    assert!(b.open(&[0x01, 0, 0, 0, 0, 1, 2], 0).is_err());
}

/// Reordering and duplication under random delivery (seeded property test): every
/// genuine frame is delivered exactly once while it is inside the window.
#[test]
fn property_random_reordering_and_duplicates() {
    let mut rng = ChaCha20Rng::from_seed([5; 32]);
    for _ in 0..32 {
        let (mut a, mut b) = pair();
        let frames: Vec<(u32, Vec<u8>)> = (0..200u32)
            .map(|i| (i, a.seal(&i.to_le_bytes(), 0).unwrap()))
            .collect();
        let mut delivered = std::collections::BTreeSet::new();
        for _ in 0..600 {
            let k = (rng.next_u32() % 200) as usize;
            let (i, f) = &frames[k];
            match b.open(f, 0) {
                Ok(pt) => {
                    assert_eq!(pt, i.to_le_bytes());
                    assert!(delivered.insert(*i), "frame {i} delivered twice");
                }
                Err(e) => {
                    assert_eq!(e, CryptoError::Replay);
                    assert!(delivered.contains(i));
                }
            }
        }
    }
}
