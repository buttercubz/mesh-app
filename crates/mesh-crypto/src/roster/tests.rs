use super::*;
use crate::invite::{Invite, sign_invite};
use crate::join;
use rand_chacha::ChaCha20Rng;
use rand_core::SeedableRng;

const GID: [u8; 16] = [0x5A; 16];

struct Group {
    leader: GroupIdentity,
    members: Vec<GroupIdentity>,
    secret: Secret32,
}

fn group() -> Group {
    let mut rng = ChaCha20Rng::from_seed([31; 32]);
    Group {
        leader: GroupIdentity::generate(&mut rng),
        members: (0..3).map(|_| GroupIdentity::generate(&mut rng)).collect(),
        secret: Secret32::random(&mut rng),
    }
}

fn entries(g: &Group) -> Vec<RosterEntry> {
    let mut v = vec![entry_for_identity(
        g.leader.public(),
        RosterRole::Leader,
        "Lead",
    )];
    for (i, m) in g.members.iter().enumerate() {
        v.push(entry_for_identity(
            m.public(),
            RosterRole::Member,
            &format!("M{i}"),
        ));
    }
    v
}

#[test]
fn sign_verify_and_lookups() {
    let g = group();
    let isig = sign_invite(&g.leader, &GID, 0, InvitePolicy::DEFAULT, "Ride", &g.secret);
    let r = build(&g.leader, GID, 0, 1, isig, entries(&g)).unwrap();
    assert!(verify(&r, &GID, &g.leader.ed_pk()).is_ok());
    assert_eq!(
        verify(&r, &[0; 16], &g.leader.ed_pk()),
        Err(CryptoError::WrongGroup)
    );
    assert!(verify(&r, &GID, &g.members[0].ed_pk()).is_err());
    // Round trip through the wire encoding keeps the signature valid.
    let back = Roster::decode(&r.encode().unwrap()).unwrap();
    assert!(verify(&back, &GID, &g.leader.ed_pk()).is_ok());
    // Any modified byte breaks it.
    let mut m = r.clone();
    m.roster_version = 2;
    assert!(verify(&m, &GID, &g.leader.ed_pk()).is_err());
    let mut m = r.clone();
    m.entries[1].role = RosterRole::Leader;
    assert!(verify(&m, &GID, &g.leader.ed_pk()).is_err());
    assert_eq!(leader_entry(&r).unwrap().ed_pk, g.leader.ed_pk());
    assert_eq!(self_signed_leader(&r), Some(g.leader.ed_pk()));
    let m1 = &g.members[1];
    assert_eq!(ed_pk_of(&r, m1.node_id()), Some(m1.ed_pk()));
    assert!(member(&r, &m1.id_hash()).is_some());
}

#[test]
fn deny_entries_stay_listed() {
    let g = group();
    let mut e = entries(&g);
    assert!(deny(&mut e, &g.members[0].id_hash()));
    assert!(!deny(&mut e, &[0; 16]));
    let r = build(&g.leader, GID, 1, 2, InviteSig([0; 64]), e).unwrap();
    assert!(r.is_denied(&g.members[0].id_hash()));
    assert!(member(&r, &g.members[0].id_hash()).is_none());
    assert_eq!(ed_pk_of(&r, g.members[0].node_id()), None);
    assert_eq!(r.index_of(g.members[1].node_id()), Some(1));
}

#[test]
fn leader_never_assigns_a_colliding_node_id() {
    let g = group();
    let mut e = entries(&g);
    let mut clash = e[2].clone();
    clash.id_hash[5] ^= 1;
    e.push(clash);
    assert_eq!(
        build(&g.leader, GID, 0, 1, InviteSig([0; 64]), e).unwrap_err(),
        CryptoError::NodeIdCollision
    );
}

#[test]
fn invite_sig_verifies_from_a_member_assembled_invite() {
    let g = group();
    let policy = InvitePolicy::new(5, true, true);
    let isig = sign_invite(&g.leader, &GID, 7, policy, "Sierra", &g.secret);
    let r = build(&g.leader, GID, 7, 3, isig, entries(&g)).unwrap();
    // The member holds the group's name, policy, leader key and epoch secret.
    assert!(verify_invite_sig(&r, &g.secret, policy, "Sierra", &g.leader.ed_pk()).is_ok());
    let inv = Invite::assemble(
        GID,
        r.epoch,
        g.secret.clone(),
        g.leader.ed_pk(),
        policy,
        "Sierra",
        InviteSig(r.invite_sig),
        false,
        None,
    );
    assert!(inv.verify_leader_sig().is_ok());
    let parsed = Invite::from_url(&inv.to_url()).unwrap();
    assert!(parsed.verify_leader_sig().is_ok());
    // A member cannot change the name or policy.
    assert!(verify_invite_sig(&r, &g.secret, policy, "Other", &g.leader.ed_pk()).is_err());
    assert!(
        verify_invite_sig(
            &r,
            &g.secret,
            InvitePolicy::DEFAULT,
            "Sierra",
            &g.leader.ed_pk()
        )
        .is_err()
    );
}

#[test]
fn entries_from_join_requests() {
    let g = group();
    let mut book = join::JoinBook::new();
    let j = join::create(&g.members[2], &GID, 1, "Zoë");
    let (id, _) = book.accept(&j.encode(), &GID).unwrap();
    let e = entry_for(book.get(&id.id_hash()).unwrap(), RosterRole::Member);
    assert_eq!(e.id_hash, g.members[2].id_hash());
    assert_eq!(e.node_id, g.members[2].node_id());
    assert_eq!(e.ed_pk, g.members[2].ed_pk());
    assert_eq!(e.name, "Zoë");
}
