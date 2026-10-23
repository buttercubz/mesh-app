use super::*;
use rand_chacha::ChaCha20Rng;
use rand_core::SeedableRng;

fn leader_and_invite(expiry: Option<u32>) -> (GroupIdentity, Invite) {
    let mut rng = ChaCha20Rng::from_seed([9; 32]);
    let leader = GroupIdentity::generate(&mut rng);
    let secret = Secret32::random(&mut rng);
    let gid = [0x11; 16];
    let sig = sign_invite(
        &leader,
        &gid,
        3,
        InvitePolicy::DEFAULT,
        "Sunday ride",
        &secret,
    );
    let inv = Invite::assemble(
        gid,
        3,
        secret,
        leader.ed_pk(),
        InvitePolicy::DEFAULT,
        "Sunday ride",
        sig,
        false,
        expiry,
    );
    (leader, inv)
}

#[test]
fn round_trip_and_size() {
    let (_, inv) = leader_and_invite(Some(29_000_000));
    let b = inv.encode();
    assert_eq!(b.len(), 1 + 1 + 16 + 4 + 32 + 32 + 1 + 4 + 1 + 11 + 64);
    assert_eq!(Invite::decode(&b).unwrap(), inv);
    let url = inv.to_url();
    assert!(url.starts_with("convlinkvoice://join?v=1#"));
    assert!(
        url.len() < 270,
        "about 255 base64url characters: {}",
        url.len()
    );
    assert_eq!(Invite::from_url(&url).unwrap(), inv);
    assert!(inv.verify_leader_sig().is_ok());
    let (_, no_exp) = leader_and_invite(None);
    assert_eq!(no_exp.encode().len(), b.len() - 4);
    assert_eq!(Invite::decode(&no_exp.encode()).unwrap(), no_exp);
}

#[test]
fn policy_bits() {
    let p = InvitePolicy::DEFAULT;
    assert_eq!(p.voice_ttl(), 4);
    assert!(!p.ble_frame_20ms());
    assert!(p.leader_priority());
    assert_eq!(p.0, 0b0110_1000);
    let q = InvitePolicy::new(9, true, false);
    assert_eq!(q.voice_ttl(), 6);
    assert!(q.ble_frame_20ms());
    assert_eq!(InvitePolicy::new(1, false, false).voice_ttl(), 3);
}

#[test]
fn signature_covers_signed_fields_only() {
    let (_, inv) = leader_and_invite(Some(100));
    // Unsigned soft fields can change.
    let mut soft = inv.clone();
    soft.flags |= FLAG_AWARE_PAIRING;
    soft.expires_at = Some(200);
    assert!(soft.verify_leader_sig().is_ok());
    // Every signed field is covered.
    let mut m = inv.clone();
    m.name = "Other".into();
    assert!(m.verify_leader_sig().is_err());
    let mut m = inv.clone();
    m.policy = InvitePolicy::new(5, false, true);
    assert!(m.verify_leader_sig().is_err());
    let mut m = inv.clone();
    m.epoch += 1;
    assert!(m.verify_leader_sig().is_err());
    let mut m = inv.clone();
    m.group_id[0] ^= 1;
    assert!(m.verify_leader_sig().is_err());
    let mut m = inv.clone();
    m.epoch_secret = Secret32::new([0; 32]);
    assert!(m.verify_leader_sig().is_err());
    // A member substituting its own key cannot reuse the leader's signature.
    let mut rng = ChaCha20Rng::from_seed([10; 32]);
    let mallory = GroupIdentity::generate(&mut rng);
    let mut m = inv.clone();
    m.leader_ed_pk = mallory.ed_pk();
    assert!(m.verify_leader_sig().is_err());
}

#[test]
fn member_assembles_byte_identical_invite() {
    let (leader, inv) = leader_and_invite(None);
    // A member stores group_id, epoch secret, leader key, policy and name, and takes
    // invite_sig from the ROSTER.
    let member_copy = Invite::assemble(
        inv.group_id,
        inv.epoch,
        inv.epoch_secret.clone(),
        leader.ed_pk(),
        inv.policy,
        &inv.name,
        inv.invite_sig,
        false,
        None,
    );
    assert_eq!(member_copy.encode(), inv.encode());
    assert!(member_copy.verify_leader_sig().is_ok());
}

#[test]
fn expiry_is_soft_with_tolerance() {
    let (_, inv) = leader_and_invite(Some(1000));
    let at = 1000 * 60;
    assert!(!inv.is_expired(at));
    assert!(!inv.is_expired(at + 600));
    assert!(inv.is_expired(at + 601));
    let (_, never) = leader_and_invite(None);
    assert!(!never.is_expired(u64::MAX));
}

#[test]
fn names_truncate_at_grapheme_boundary() {
    let mut rng = ChaCha20Rng::from_seed([4; 32]);
    let leader = GroupIdentity::generate(&mut rng);
    let secret = Secret32::random(&mut rng);
    let long = "\u{1F3CD}\u{FE0F} Rodada dominical por la sierra";
    let sig = sign_invite(&leader, &[1; 16], 0, InvitePolicy::DEFAULT, long, &secret);
    let inv = Invite::assemble(
        [1; 16],
        0,
        secret,
        leader.ed_pk(),
        InvitePolicy::DEFAULT,
        long,
        sig,
        true,
        None,
    );
    assert!(inv.name.len() <= GROUP_NAME_MAX);
    assert!(long.starts_with(&inv.name));
    let back = Invite::decode(&inv.encode()).unwrap();
    assert!(back.verify_leader_sig().is_ok());
    assert!(back.aware_pairing());
}

#[test]
fn rejects_malformed() {
    let (_, inv) = leader_and_invite(Some(5));
    let b = inv.encode();
    for cut in 0..b.len() {
        assert!(Invite::decode(&b[..cut]).is_err(), "truncated at {cut}");
    }
    let mut extra = b.clone();
    extra.push(0);
    assert!(Invite::decode(&extra).is_err());
    let mut ver = b.clone();
    ver[0] = 2;
    assert!(Invite::decode(&ver).is_err());
    assert!(Invite::from_url("https://example.com/#abc").is_err());
    assert!(Invite::from_url("convlinkvoice://join?v=2#AAAA").is_err());
    assert!(Invite::from_url("convlinkvoice://join?v=1").is_err());
    let mut bad_name = b.clone();
    // name_len sits right after policy + expires_at.
    let pos = 1 + 1 + 16 + 4 + 32 + 32 + 1 + 4;
    bad_name[pos] = 33;
    assert!(Invite::decode(&bad_name).is_err());
}
