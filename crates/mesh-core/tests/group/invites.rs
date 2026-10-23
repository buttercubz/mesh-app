//! Invite create/parse/join/list/forget, the native QR, members re-sharing the
//! leader-signed invite, the first-ROSTER leader-key cross-check, invite expiry.

use mesh_api::{CommandResult, ErrorCode, MemberRole, NoticeKey, UiCommand};
use mesh_core::Platform;
use mesh_core::effects::Effect;
use mesh_crypto::GroupIdentity;
use mesh_crypto::invite::{Invite, sign_invite};
use rand_core::SeedableRng;

use crate::harness::World;
use crate::support::*;

fn parse(w: &mut World, i: usize, url: &str) -> mesh_api::OkInviteInfo {
    match w.command(i, UiCommand::ParseInvite { url: url.into() }) {
        CommandResult::InviteInfo(x) => x,
        other => panic!("parseInvite: {other:?}"),
    }
}

fn groups(w: &mut World, i: usize) -> Vec<mesh_api::GroupSummary> {
    match w.command(i, UiCommand::ListGroups {}) {
        CommandResult::Groups(x) => x.groups,
        other => panic!("listGroups: {other:?}"),
    }
}

fn persisted(w: &World, i: usize, key: &str) -> Option<Option<Vec<u8>>> {
    w.nodes[i].effects.iter().rev().find_map(|e| match e {
        Effect::Persist { key: k, value } if k == key => Some(value.clone()),
        _ => None,
    })
}

#[test]
fn create_parse_join_list_and_forget() {
    let mut w = World::new(2, Platform::Ios, 51);
    let g = create(&mut w, 0, "Ana");
    // F1: persisted per group plus the index, stored natively as opaque bytes.
    assert!(matches!(
        persisted(&w, 0, &format!("group/{g}")),
        Some(Some(_))
    ));
    assert!(matches!(persisted(&w, 0, "groups/index"), Some(Some(_))));
    let l = groups(&mut w, 0);
    assert_eq!(l.len(), 1);
    assert_eq!(
        (l[0].role, l[0].members, l[0].epoch, l[0].closed),
        (MemberRole::Leader, 1, 0, false)
    );
    assert_eq!(l[0].name, "Sunday ride");
    // The invite leaves the core only as a natively rendered QR image.
    match w.command(
        0,
        UiCommand::GetInvite {
            group_id: g.clone(),
        },
    ) {
        CommandResult::Invite(x) => {
            assert!(x.qr_png.starts_with("data:image/png;base64,"));
            assert!(x.qr_png.len() > 400);
            assert_eq!(x.hide_after_ms, 60_000);
        }
        other => panic!("getInvite: {other:?}"),
    }
    let url = invite_url(&w, 0, &g);
    assert!(url.starts_with("convlinkvoice://join?v=1#"));
    let info = parse(&mut w, 1, &url);
    assert!(info.valid && info.leader_sig_ok && !info.expired && !info.already_member);
    assert_eq!(info.group_name.as_deref(), Some("Sunday ride"));
    assert_eq!(info.group_id.as_deref(), Some(g.as_str()));
    assert_eq!(info.epoch, Some(0));
    assert!(parse(&mut w, 0, &url).already_member);
    assert!(!parse(&mut w, 1, "convlinkvoice://join?v=1#AAAA").valid);
    // A tampered invite (name changed, signature kept) fails the leader signature.
    let mut fake = Invite::from_url(&url).expect("invite");
    fake.name = "Other ride".into();
    assert!(!parse(&mut w, 1, &fake.to_url()).leader_sig_ok);
    // F2: join, then a second join of the same group is refused.
    assert_eq!(join(&mut w, 1, &url, "Ben"), g);
    assert_eq!(
        w.command(
            1,
            UiCommand::JoinGroup {
                url: url.clone(),
                display_name: "Ben".into()
            }
        )
        .error_code(),
        Some(ErrorCode::AlreadyMember)
    );
    let l = groups(&mut w, 1);
    assert_eq!((l.len(), l[0].role), (1, MemberRole::Member));
    // Each group gets its own identity: a second group never reuses it.
    let g2 = gid(&w.command(
        1,
        UiCommand::CreateGroup {
            name: "Weekday".into(),
            vehicle: mesh_api::Vehicle::Car,
        },
    ));
    assert_ne!(
        group(&w, 1, &g).my_id_hash(),
        group(&w, 1, &g2).my_id_hash()
    );
    // Before a verified roster a member cannot re-share an invite.
    assert_eq!(
        w.command(
            1,
            UiCommand::GetInvite {
                group_id: g.clone()
            }
        )
        .error_code(),
        Some(ErrorCode::InviteInvalid)
    );
    // Forget deletes the stored group.
    assert!(
        w.command(
            1,
            UiCommand::ForgetGroup {
                group_id: g.clone()
            }
        )
        .is_ok()
    );
    assert_eq!(persisted(&w, 1, &format!("group/{g}")), Some(None));
    assert_eq!(groups(&mut w, 1).len(), 1);
    assert_eq!(
        w.command(
            1,
            UiCommand::ForgetGroup {
                group_id: g.clone()
            }
        )
        .error_code(),
        Some(ErrorCode::UnknownGroup)
    );
    assert_eq!(
        w.command(1, UiCommand::GetInvite { group_id: g })
            .error_code(),
        Some(ErrorCode::UnknownGroup)
    );
}

#[test]
fn members_reshare_the_byte_identical_leader_signed_invite() {
    let (mut w, g) = pair(52);
    let leader = invite_url(&w, 0, &g);
    let member = invite_url(&w, 1, &g);
    assert_eq!(
        member, leader,
        "assembled from the stored secret + ROSTER invite_sig"
    );
    assert!(matches!(
        w.command(
            1,
            UiCommand::GetInvite {
                group_id: g.clone()
            }
        ),
        CommandResult::Invite(_)
    ));
    // A forgotten group cannot be forgotten while its session runs.
    assert_eq!(
        w.command(1, UiCommand::ForgetGroup { group_id: g })
            .error_code(),
        Some(ErrorCode::WrongMode)
    );
}

#[test]
fn invite_expiry_is_leader_local_and_unsigned() {
    let (mut w, g) = pair(53);
    assert_eq!(
        w.command(1, UiCommand::SetInviteExpiry { hours: Some(2) })
            .error_code(),
        Some(ErrorCode::NotLeader)
    );
    assert!(
        w.command(0, UiCommand::SetInviteExpiry { hours: Some(2) })
            .is_ok()
    );
    w.run_ms(100);
    let inv = Invite::from_url(&invite_url(&w, 0, &g)).expect("invite");
    let at = inv.expires_at.expect("has expiry");
    let now_min = (1_792_746_723_120 / 1_000 + w.now.0 / 1_000_000) / 60;
    assert!((now_min + 119..=now_min + 121).contains(&u64::from(at)));
    assert!(inv.verify_leader_sig().is_ok(), "expiry is not signed");
    assert!(!parse(&mut w, 1, &inv.to_url()).expired);
    // An invite that expired long ago is reported and refused at join.
    let mut old = inv.clone();
    old.expires_at = Some(at - 24 * 60);
    old.flags |= 0b10;
    assert!(parse(&mut w, 1, &old.to_url()).expired);
    let mut w2 = World::new(1, Platform::Ios, 54);
    assert_eq!(
        w2.command(
            0,
            UiCommand::JoinGroup {
                url: old.to_url(),
                display_name: "Eve".into()
            }
        )
        .error_code(),
        Some(ErrorCode::InviteExpired)
    );
    assert!(
        w.command(0, UiCommand::SetInviteExpiry { hours: None })
            .is_ok()
    );
    w.run_ms(100);
    assert_eq!(
        Invite::from_url(&invite_url(&w, 0, &g)).unwrap().expires_at,
        None
    );
}

#[test]
fn a_fake_leader_invite_is_flagged_by_the_first_roster() {
    let (mut w, g) = crate::members(3, Platform::Ios, 55);
    // A member hands out an invite for the real group secret but signed by its own key.
    let real = Invite::from_url(&invite_url(&w, 0, &g)).unwrap();
    let mut rng = rand_chacha::ChaCha20Rng::from_seed([9; 32]);
    let fake_leader = GroupIdentity::generate(&mut rng);
    let sig = sign_invite(
        &fake_leader,
        &real.group_id,
        real.epoch,
        real.policy,
        &real.name,
        &real.epoch_secret,
    );
    let fake = Invite::assemble(
        real.group_id,
        real.epoch,
        real.epoch_secret.clone(),
        fake_leader.ed_pk(),
        real.policy,
        &real.name,
        sig,
        false,
        None,
    );
    w.nodes
        .push(World::new(1, Platform::Ios, 56).nodes.remove(0));
    let info = parse(&mut w, 3, &fake.to_url());
    assert!(
        info.leader_sig_ok,
        "self-consistent, so it verifies on its own"
    );
    join(&mut w, 3, &fake.to_url(), "Eve");
    start(&mut w, 3, &g);
    w.connect(3, 0);
    w.connect(1, 0);
    w.run_ms(3_000);
    assert!(notices(&w, 3).contains(&NoticeKey::InviteLeaderMismatch));
    let eve = group(&w, 3, &g);
    assert!(eve.unverified_leader);
    assert!(
        eve.roster().is_none(),
        "nothing signed by the real leader is trusted"
    );
    // Real members never raise it.
    assert!(!notices(&w, 1).contains(&NoticeKey::InviteLeaderMismatch));
    assert!(group(&w, 1, &g).roster().is_some());
}
