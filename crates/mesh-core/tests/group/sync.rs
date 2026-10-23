//! Control-state anti-entropy (§8.9): a node that missed ROSTER, REKEY and GROUP_CTL
//! converges after link-up (even across a leader handover), replayed older bodies are
//! ignored, and a JOIN_REQ sent while the leader was unreachable is retried until it
//! lands in a roster.

use mesh_api::UiCommand;
use mesh_core::{LinkDownReason, Platform};
use mesh_wire::record::SyncKind;

use crate::support::*;
use crate::{inject, members, summary};

#[test]
fn a_node_that_missed_roster_rekey_and_group_ctl_converges_after_link_up() {
    let (mut w, g) = members(3, Platform::Ios, 71);
    w.connect(1, 0);
    w.connect(2, 0);
    w.run_ms(3_000);
    assert_eq!(roster(&w, 2, &g).len(), 3);
    let ben = node_id(&w, 1, &g);
    // Cid misses a lock, a handover to Ben and Ben's own lock.
    stop(&mut w, 2);
    w.run_ms(500);
    assert!(w.command(0, UiCommand::LockGroup {}).is_ok());
    w.run_ms(500);
    assert!(
        w.command(0, UiCommand::TransferLeader { node_id: ben })
            .is_ok()
    );
    w.run_ms(1_000);
    assert!(
        w.command(1, UiCommand::LockGroup {}).is_ok(),
        "Ben leads now"
    );
    w.run_ms(1_000);
    let latest = summary(&w, 1, &g);
    assert_eq!(summary(&w, 0, &g), latest);
    assert_eq!(latest.0, 2);
    assert!(latest.2 >= 1);
    let behind = summary(&w, 2, &g);
    assert!(behind.0 == 0 && behind.1 < latest.1 && behind.2 < latest.2);
    // Cid is back and meets Ana (no longer the leader): her CAPS summary is ahead, so
    // she pushes GROUP_CTL (the handover signed by the key Cid trusts), then Ben's REKEY
    // and Ben's roster.
    start(&mut w, 2, &g);
    w.connect(2, 0);
    w.run_ms(3_000);
    assert_eq!(summary(&w, 2, &g), latest);
    let cid = group(&w, 2, &g);
    assert_eq!(cid.control.leader_pk(), &group(&w, 1, &g).identity.ed_pk());
    assert_eq!(cid.epoch(), 2, "Ben's REKEY was sealed to Cid");
    assert!(cid.in_roster());
    // The link Cid opened under the old epoch stays up.
    assert_eq!(up_peers(&w, 2), vec![node_id(&w, 0, &g)]);
}

#[test]
fn replayed_older_roster_and_rekey_are_ignored() {
    let (mut w, g) = pair(72);
    let queue = inject::attach(&mut w, 0);
    let old_roster = group(&w, 1, &g)
        .control
        .body(SyncKind::Roster)
        .expect("roster")
        .to_vec();
    // A rename makes the leader publish the next roster version; a lock adds a REKEY.
    assert!(
        w.command(
            1,
            UiCommand::SetDisplayName {
                name: "Benny".into()
            }
        )
        .is_ok()
    );
    w.run_ms(1_000);
    assert!(w.command(0, UiCommand::LockGroup {}).is_ok());
    w.run_ms(500);
    let rekey1 = group(&w, 0, &g)
        .control
        .body(SyncKind::Rekey)
        .unwrap()
        .to_vec();
    assert!(w.command(0, UiCommand::LockGroup {}).is_ok());
    w.run_ms(500);
    let now = summary(&w, 1, &g);
    assert_eq!(now.0, 2);
    assert_eq!(now, summary(&w, 0, &g));
    let ben = node_id(&w, 1, &g);
    assert_eq!(group(&w, 1, &g).name_of(ben).as_deref(), Some("Benny"));
    // Stale but validly signed bodies replayed over the authenticated link.
    queue.borrow_mut().push((SyncKind::Roster, old_roster));
    queue.borrow_mut().push((SyncKind::Rekey, rekey1));
    w.run_ms(2_000);
    assert!(queue.borrow().is_empty(), "injected");
    assert_eq!(summary(&w, 1, &g), now, "monotonic: nothing rolls back");
    assert_eq!(group(&w, 1, &g).name_of(ben).as_deref(), Some("Benny"));
}

#[test]
fn join_req_sent_while_the_leader_was_unreachable_is_retried_into_the_roster() {
    let (mut w, g) = pair(73);
    let url = invite_url(&w, 0, &g);
    let c = crate::epochs::add_node(&mut w, Platform::Ios, 74);
    join(&mut w, c, &url, "Cid");
    // The leader rides out of range of Ben.
    w.drop_link(0, LinkDownReason::Timeout);
    w.run_ms(500);
    start(&mut w, c, &g);
    w.connect(c, 1);
    w.run_ms(10_000);
    let cid = node_id(&w, c, &g);
    assert_eq!(up_peers(&w, c), vec![node_id(&w, 1, &g)]);
    assert!(!roster(&w, 0, &g).contains(&cid));
    assert!(
        !group(&w, c, &g).in_roster(),
        "Ben's roster (pushed on link-up) lacks Cid"
    );
    // The leader is back in range of Ben; Cid's next 30 s retransmission travels on.
    w.connect(1, 0);
    w.run_ms(45_000);
    assert!(
        roster(&w, 0, &g).contains(&cid),
        "landed in the leader's next roster"
    );
    assert!(
        group(&w, c, &g).in_roster(),
        "the new roster reached Cid by anti-entropy"
    );
    assert_eq!(summary(&w, c, &g), summary(&w, 0, &g));
}
