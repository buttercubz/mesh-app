//! Leader operations (§8.6 steps 6-7): transfer of leadership moves every member's trust
//! anchor once the signed GROUP_CTL and ROSTER verify, after which only the new leader's
//! rosters are accepted; Close group marks the group closed and deletes its secrets.

use mesh_api::{CommandResult, ErrorCode, MemberRole, NoticeKey, UiCommand};
use mesh_core::group::store::{self, Stored};
use mesh_core::media::{Cue, MediaRequest};
use mesh_core::{Platform, PromptKey};
use mesh_crypto::invite::{Invite, sign_invite};
use mesh_wire::mesh::RosterRole;
use mesh_wire::record::SyncKind;

use crate::harness::World;
use crate::support::*;
use crate::{inject, members, summary};

fn role(w: &mut World, i: usize, g: &str) -> MemberRole {
    match w.command(i, UiCommand::ListGroups {}) {
        CommandResult::Groups(x) => x.groups.iter().find(|s| s.group_id == g).unwrap().role,
        other => panic!("listGroups: {other:?}"),
    }
}

#[test]
fn transfer_leader_moves_the_anchor_and_only_the_new_leaders_roster_counts() {
    let (mut w, g) = members(3, Platform::Ios, 81);
    w.connect(1, 0);
    w.connect(2, 0);
    w.run_ms(3_000);
    let queue = inject::attach(&mut w, 0);
    let ben = node_id(&w, 1, &g);
    assert_eq!(
        w.command(
            1,
            UiCommand::TransferLeader {
                node_id: node_id(&w, 2, &g)
            }
        )
        .error_code(),
        Some(ErrorCode::NotLeader)
    );
    assert_eq!(
        w.command(0, UiCommand::TransferLeader { node_id: 0xFFFF })
            .error_code(),
        Some(ErrorCode::UnknownMember)
    );
    assert!(
        w.command(0, UiCommand::TransferLeader { node_id: ben })
            .is_ok()
    );
    w.run_ms(2_000);
    let ben_pk = group(&w, 1, &g).identity.ed_pk();
    for i in 0..3 {
        assert_eq!(
            group(&w, i, &g).control.leader_pk(),
            &ben_pk,
            "node {i} anchor"
        );
    }
    assert_eq!(role(&mut w, 1, &g), MemberRole::Leader);
    assert_eq!(role(&mut w, 0, &g), MemberRole::Member);
    assert!(notices(&w, 2).contains(&NoticeKey::LeaderChanged));
    // Cid hears "R1 is the leader now" (the prompt names the new leader).
    let spoken = Cue::with_prompt(None, PromptKey::LeaderChanged).param("R1");
    assert!(cues(&w, 2).contains(&spoken));
    // The handover reaches the media modules (TALK priority, LEADER flag).
    for (i, leader) in [(0, false), (1, true), (2, false)] {
        let last = media_requests(&w, i)
            .into_iter()
            .rev()
            .find_map(|r| match r {
                MediaRequest::Identity(id) => Some(id.leader),
                _ => None,
            });
        assert_eq!(last, Some(leader), "node {i}");
    }
    // The new leader re-published the roster with its own invite_sig: members re-share
    // an invite that names it.
    let url = invite_url(&w, 2, &g);
    assert_eq!(url, invite_url(&w, 1, &g));
    assert_eq!(Invite::from_url(&url).unwrap().leader_ed_pk, ben_pk);
    // The old leader can no longer lead...
    assert_eq!(
        w.command(0, UiCommand::LockGroup {}).error_code(),
        Some(ErrorCode::NotLeader)
    );
    // ...and a roster it still signs (next version, itself as leader) is refused.
    let before = summary(&w, 2, &g);
    let ana = group(&w, 0, &g);
    let r = ana.roster().unwrap();
    let mut entries = r.entries.clone();
    for e in &mut entries {
        e.role = if e.ed_pk == ana.identity.ed_pk() {
            RosterRole::Leader
        } else if e.role == RosterRole::Leader {
            RosterRole::Member
        } else {
            e.role
        };
    }
    let sched = ana.keyring.newest();
    let sig = sign_invite(
        &ana.identity,
        &ana.id,
        sched.epoch(),
        ana.policy,
        &ana.name,
        sched.epoch_secret(),
    );
    let forged = mesh_crypto::roster::build(
        &ana.identity,
        ana.id,
        r.epoch,
        r.roster_version + 5,
        sig,
        entries,
    )
    .unwrap()
    .encode()
    .unwrap();
    queue.borrow_mut().push((SyncKind::Roster, forged));
    w.run_ms(2_000);
    assert!(queue.borrow().is_empty());
    assert_eq!(summary(&w, 2, &g), before, "old leader's roster ignored");
    assert_eq!(group(&w, 2, &g).control.leader_pk(), &ben_pk);
    // The new leader's operations are accepted (Cid, two hops away, picks the REKEY up
    // from Ana by anti-entropy once his PRESENCE shows the old epoch).
    assert!(w.command(1, UiCommand::LockGroup {}).is_ok());
    w.run_ms(3_000);
    assert_eq!(summary(&w, 2, &g).0, 1);
    assert_eq!(summary(&w, 2, &g), summary(&w, 1, &g));
}

#[test]
fn close_group_marks_it_closed_and_deletes_the_secrets() {
    let (mut w, g) = pair(82);
    assert_eq!(
        w.command(1, UiCommand::CloseGroup {}).error_code(),
        Some(ErrorCode::NotLeader)
    );
    assert!(w.command(0, UiCommand::CloseGroup {}).is_ok());
    w.run_ms(2_000);
    assert!(notices(&w, 1).contains(&NoticeKey::GroupClosed));
    for i in 0..2 {
        assert!(
            media_requests(&w, i).contains(&MediaRequest::EndSession),
            "node {i}: the media session ends with the group"
        );
        let b = blob(&w, i, &g).expect("tombstone persisted");
        assert!(
            matches!(store::decode(&b), Ok(Stored::Closed(_))),
            "node {i}: secrets deleted"
        );
        match w.command(i, UiCommand::ListGroups {}) {
            CommandResult::Groups(x) => assert!(x.groups[0].closed),
            other => panic!("{other:?}"),
        }
        assert_eq!(
            w.command(
                i,
                UiCommand::GetInvite {
                    group_id: g.clone()
                }
            )
            .error_code(),
            Some(ErrorCode::GroupClosed)
        );
        assert_eq!(
            w.command(i, UiCommand::GoLive {}).error_code(),
            Some(ErrorCode::GroupClosed)
        );
        assert!(
            up_peers(&w, i).is_empty(),
            "links closed after the GROUP_CTL left"
        );
    }
    // A closed group can be forgotten once its session stopped.
    stop(&mut w, 1);
    assert!(w.command(1, UiCommand::ForgetGroup { group_id: g }).is_ok());
}
