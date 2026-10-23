use super::*;
use crate::group_ctl as gc;
use crate::invite::InviteSig;
use crate::rekey::{self as rk, RekeyRecipient};
use crate::roster::{self as ro, RosterEntry, RosterRole, entry_for_identity};
use rand_chacha::ChaCha20Rng;
use rand_core::SeedableRng;

const GID: [u8; 16] = [0x2C; 16];

struct G {
    rng: ChaCha20Rng,
    leader: GroupIdentity,
    m: Vec<GroupIdentity>,
}

fn g() -> G {
    let mut rng = ChaCha20Rng::from_seed([71; 32]);
    let leader = GroupIdentity::generate(&mut rng);
    let m = (0..4).map(|_| GroupIdentity::generate(&mut rng)).collect();
    G { rng, leader, m }
}

fn entries(g: &G, leader: &GroupIdentity, denied: &[usize]) -> Vec<RosterEntry> {
    let mut v = vec![entry_for_identity(leader.public(), RosterRole::Leader, "L")];
    for (i, m) in g.m.iter().enumerate() {
        if m.ed_pk() == leader.ed_pk() {
            continue;
        }
        let role = if denied.contains(&i) {
            RosterRole::Denied
        } else {
            RosterRole::Member
        };
        v.push(entry_for_identity(m.public(), role, "M"));
    }
    v
}

fn roster_body(
    g: &G,
    signer: &GroupIdentity,
    leader: &GroupIdentity,
    version: u32,
    epoch: u32,
    denied: &[usize],
) -> Vec<u8> {
    ro::build(
        signer,
        GID,
        epoch,
        version,
        InviteSig([0; 64]),
        entries(g, leader, denied),
    )
    .unwrap()
    .encode()
    .unwrap()
}

fn recipients(ids: &[&GroupIdentity]) -> Vec<RekeyRecipient> {
    ids.iter()
        .map(|m| RekeyRecipient {
            node_id: m.node_id(),
            x_pk: m.x_pk(),
        })
        .collect()
}

#[test]
fn older_or_equal_roster_is_rejected() {
    let g = g();
    let mut st = ControlState::new(GID, g.leader.ed_pk(), 0);
    let v2 = roster_body(&g, &g.leader, &g.leader, 2, 0, &[1]);
    let v1 = roster_body(&g, &g.leader, &g.leader, 1, 0, &[]);
    let fx = st.on_roster(&v2).unwrap();
    assert_eq!(
        fx,
        vec![ControlEffect::RosterAccepted {
            roster_version: 2,
            newly_denied: vec![g.m[1].id_hash()]
        }]
    );
    // A replayed older roster without the deny entry is ignored.
    assert!(matches!(st.on_roster(&v1), Err(CryptoError::Stale(_))));
    assert!(matches!(st.on_roster(&v2), Err(CryptoError::Stale(_))));
    assert!(st.deny().is_denied(&g.m[1].id_hash()));
    // Not signed by the leader.
    let forged = roster_body(&g, &g.m[0], &g.leader, 3, 0, &[]);
    assert_eq!(st.on_roster(&forged), Err(CryptoError::BadSignature));
    assert_eq!(st.roster_version(), 2);
    assert_eq!(
        roster(&Roster::decode(&v1).unwrap(), &GID, &g.leader.ed_pk(), None),
        Ok(())
    );
}

#[test]
fn rekey_monotonic_and_three_epochs_behind() {
    let mut g = g();
    let leader = g.leader.clone();
    let (m0, m1, m2, removed) = (
        g.m[0].clone(),
        g.m[1].clone(),
        g.m[2].clone(),
        g.m[3].clone(),
    );
    // m2 stayed at epoch 0 while the group went through epochs 1, 2, 3.
    let mut behind = ControlState::new(GID, leader.ed_pk(), 0);
    let mut current = ControlState::new(GID, leader.ed_pk(), 0);
    let mut last = Vec::new();
    for e in 1..=4u32 {
        let secret = Secret32::random(&mut g.rng);
        let to: Vec<&GroupIdentity> = if e == 4 {
            vec![&m0, &m1, &m2]
        } else {
            vec![&m0, &m1, &m2, &removed]
        };
        let body = rk::seal(&leader, &GID, e, e, &secret, &recipients(&to), &mut g.rng)
            .unwrap()
            .encode()
            .unwrap();
        let fx = current.on_rekey(&body, &m0).unwrap();
        assert_eq!(
            fx,
            vec![ControlEffect::EpochAdvanced {
                epoch: e,
                secret: secret.clone()
            }]
        );
        last = body;
    }
    assert_eq!(current.epoch(), 4);
    // The member 3 epochs behind needs only the latest REKEY.
    let fx = behind.on_rekey(&last, &m2).unwrap();
    assert!(matches!(
        &fx[0],
        ControlEffect::EpochAdvanced { epoch: 4, .. }
    ));
    // The removed member cannot open it.
    let mut gone = ControlState::new(GID, leader.ed_pk(), 3);
    assert_eq!(gone.on_rekey(&last, &removed), Err(CryptoError::NoEntry));
    assert_eq!(gone.epoch(), 3);
    // Older or equal epochs are ignored.
    assert!(matches!(
        behind.on_rekey(&last, &m2),
        Err(CryptoError::Stale(_))
    ));
    let rk4 = Rekey::decode(&last).unwrap();
    assert!(rekey(&rk4, &GID, &leader.ed_pk(), 4, &m1).is_err());
    assert!(rekey(&rk4, &GID, &leader.ed_pk(), 3, &m1).is_ok());
    // Anti-entropy offers the REKEY to a peer whose CAPS shows an older epoch.
    assert_eq!(current.to_push((1, 0, 0)), vec![SyncKind::Rekey]);
    assert_eq!(current.body(SyncKind::Rekey), Some(last.as_slice()));
    assert!(current.to_push((4, 0, 0)).is_empty());
    assert_eq!(current.to_push_presence(2, 0), vec![SyncKind::Rekey]);
}

#[test]
fn group_ctl_monotonic_and_close() {
    let g = g();
    let mut st = ControlState::new(GID, g.leader.ed_pk(), 2);
    let old_epoch = gc::close(&g.leader, &GID, 5, 1).unwrap();
    assert!(matches!(
        st.on_group_ctl(&old_epoch.encode().unwrap()),
        Err(CryptoError::Stale(_))
    ));
    let t = gc::transfer_leader(&g.leader, &GID, 5, 2, g.m[0].ed_pk(), g.m[0].node_id()).unwrap();
    st.on_group_ctl(&t.encode().unwrap()).unwrap();
    let older = gc::close(&g.leader, &GID, 4, 3).unwrap();
    assert!(matches!(
        st.on_group_ctl(&older.encode().unwrap()),
        Err(CryptoError::Stale(_))
    ));
    let same = gc::close(&g.leader, &GID, 5, 3).unwrap();
    assert!(st.on_group_ctl(&same.encode().unwrap()).is_err());
    assert_eq!(st.ctl_version(), 5);
    let close = gc::close(&g.leader, &GID, 6, 2).unwrap();
    assert_eq!(
        st.on_group_ctl(&close.encode().unwrap()).unwrap(),
        vec![ControlEffect::GroupClosed]
    );
    assert!(st.is_closed());
    let later = roster_body(&g, &g.leader, &g.leader, 9, 2, &[]);
    assert!(
        st.on_roster(&later).is_err(),
        "closed groups accept nothing"
    );
    assert_eq!(st.to_push((2, 0, 5)), vec![SyncKind::GroupCtl]);
    let c = GroupCtl::decode(st.body(SyncKind::GroupCtl).unwrap()).unwrap();
    assert!(group_ctl(&c, &GID, &g.leader.ed_pk(), Some(5), 2).is_ok());
    assert!(group_ctl(&c, &GID, &g.leader.ed_pk(), Some(6), 2).is_err());
}

#[test]
fn transfer_leader_needs_both_messages_in_either_order() {
    let g = g();
    let new = g.m[0].clone();
    let t = gc::transfer_leader(&g.leader, &GID, 1, 0, new.ed_pk(), new.node_id())
        .unwrap()
        .encode()
        .unwrap();
    // Roles swapped, signed by the OLD key.
    let swapped = roster_body(&g, &g.leader, &new, 2, 0, &[]);
    let after = roster_body(&g, &new, &new, 3, 0, &[]);

    // Order 1: GROUP_CTL then ROSTER (flood order).
    let mut a = ControlState::new(GID, g.leader.ed_pk(), 0);
    let fx = a.on_group_ctl(&t).unwrap();
    assert!(matches!(fx[0], ControlEffect::TransferPending { .. }));
    assert_eq!(a.leader_ed_pk(), &g.leader.ed_pk());
    assert!(a.on_roster(&after).is_err(), "new key not trusted yet");
    let fx = a.on_roster(&swapped).unwrap();
    assert_eq!(
        fx.last(),
        Some(&ControlEffect::LeaderChanged {
            new_leader_ed_pk: new.ed_pk()
        })
    );
    assert!(a.on_roster(&after).is_ok());

    // Order 2: ROSTER then GROUP_CTL (anti-entropy order REKEY → ROSTER → GROUP_CTL).
    let mut b = ControlState::new(GID, g.leader.ed_pk(), 0);
    b.on_roster(&swapped).unwrap();
    assert_eq!(b.leader_ed_pk(), &g.leader.ed_pk());
    let fx = b.on_group_ctl(&t).unwrap();
    assert_eq!(fx.len(), 2);
    assert_eq!(b.leader_ed_pk(), &new.ed_pk());
    assert!(b.pending_transfer().is_none());
    // The old leader can no longer sign rosters.
    let stale_leader = roster_body(&g, &g.leader, &g.leader, 4, 0, &[]);
    assert!(b.on_roster(&stale_leader).is_err());
    assert_eq!(b.summary(), (0, 2, 1));
}

#[test]
fn leader_records_its_own_rekey() {
    let mut g = g();
    let leader = g.leader.clone();
    let m0 = g.m[0].clone();
    let mut st = ControlState::new(GID, leader.ed_pk(), 0);
    let s = Secret32::random(&mut g.rng);
    let body = rk::seal(&leader, &GID, 1, 1, &s, &recipients(&[&m0]), &mut g.rng)
        .unwrap()
        .encode()
        .unwrap();
    assert_eq!(st.record_own_rekey(&body), Ok(1));
    assert!(st.record_own_rekey(&body).is_err());
    assert!(st.on_body(SyncKind::JoinReq, &body, &m0).is_err());
}
