//! Lock and remove with the retained keyring (§8.6): a stranger with an old QR is
//! refused, a roster member several epochs behind is readmitted (as BLE central over a
//! speculative link) and brought to the newest epoch by the latest REKEY, a removed
//! member is closed and later refused, and `previewAccessLoss` lists the unconfirmed.

use mesh_api::{CommandResult, ErrorCode, NoticeKey, UiCommand};
use mesh_core::media::MediaRequest;
use mesh_core::{LinkRole, Platform};
use mesh_crypto::invite::Invite;
use mesh_wire::frame::RejectReason;

use crate::harness::World;
use crate::support::*;
use crate::{close_reasons, members, summary};

/// Adds a fresh node (no groups) and returns its index.
pub fn add_node(w: &mut World, platform: Platform, salt: u8) -> usize {
    w.nodes.push(World::new(1, platform, salt).nodes.remove(0));
    w.nodes.len() - 1
}

fn preview(w: &mut World, i: usize) -> Vec<u16> {
    match w.command(i, UiCommand::PreviewAccessLoss {}) {
        CommandResult::AccessLoss(x) => x.unconfirmed.into_iter().map(|n| n.node_id).collect(),
        other => panic!("previewAccessLoss: {other:?}"),
    }
}

#[test]
fn lock_refuses_a_stranger_with_an_old_qr() {
    let (mut w, g) = pair(61);
    let old = invite_url(&w, 0, &g);
    assert!(preview(&mut w, 0).is_empty());
    assert_eq!(
        w.command(1, UiCommand::PreviewAccessLoss {}).error_code(),
        Some(ErrorCode::NotLeader)
    );
    assert_eq!(
        w.command(1, UiCommand::LockGroup {}).error_code(),
        Some(ErrorCode::NotLeader)
    );
    assert!(w.command(0, UiCommand::LockGroup {}).is_ok());
    assert!(notices(&w, 0).contains(&NoticeKey::GroupLocked));
    w.run_ms(1_000);
    assert_eq!(summary(&w, 0, &g), summary(&w, 1, &g));
    assert_eq!(summary(&w, 1, &g).0, 1, "the member opened the REKEY");
    // The new epoch reaches both media modules (floor ranking input, §6.2).
    for i in 0..2 {
        let last = media_requests(&w, i)
            .into_iter()
            .rev()
            .find_map(|r| match r {
                MediaRequest::Identity(id) => Some(id),
                _ => None,
            });
        assert_eq!(last.map(|id| (id.epoch, id.leader)), Some((1, i == 0)));
    }
    assert_eq!(up_peers(&w, 0).len(), 1, "existing links stay up");
    // New invites carry the new epoch (re-shared byte-identically by the member).
    let new = invite_url(&w, 1, &g);
    assert_eq!(new, invite_url(&w, 0, &g));
    assert_eq!(Invite::from_url(&new).unwrap().epoch, 1);
    // A stranger holding the old QR: the Noise handshake under the retained epoch-0 PSK
    // completes, but IDPROOF admission refuses an old epoch from a non-roster identity.
    for target in [0usize, 1] {
        let s = add_node(&mut w, Platform::Ios, 62 + target as u8);
        join(&mut w, s, &old, "Eve");
        start(&mut w, s, &g);
        w.connect(s, target);
        w.run_ms(2_000);
        assert!(
            up_peers(&w, s).is_empty(),
            "stranger refused by node {target}"
        );
        assert!(close_reasons(&w, target).contains(&RejectReason::GroupEpochMismatch));
    }
    // With the new QR a newcomer is admitted and listed.
    let n = add_node(&mut w, Platform::Ios, 64);
    join(&mut w, n, &new, "Neo");
    start(&mut w, n, &g);
    w.connect(n, 1);
    w.run_ms(3_000);
    assert_eq!(up_peers(&w, n), vec![node_id(&w, 1, &g)]);
    assert!(
        roster(&w, 0, &g).contains(&node_id(&w, n, &g)),
        "JOIN_REQ relayed to the leader"
    );
}

#[test]
fn member_three_epochs_behind_rejoins_as_central_via_a_speculative_link() {
    let mut w = World::new(2, Platform::Ios, 65);
    // The leader is an Android peripheral: an iPhone always initiates toward it (§9.2).
    reboot(&mut w, 0, Platform::Android, 65);
    let g = create(&mut w, 0, "Ana");
    let url = invite_url(&w, 0, &g);
    join(&mut w, 1, &url, "Ben");
    start(&mut w, 0, &g);
    start(&mut w, 1, &g);
    w.connect(1, 0);
    w.run_ms(3_000);
    let ben = node_id(&w, 1, &g);
    assert!(roster(&w, 0, &g).contains(&ben));
    // Ben skips three rides: the leader locks three times meanwhile.
    stop(&mut w, 1);
    w.run_ms(500);
    for _ in 0..3 {
        assert!(w.command(0, UiCommand::LockGroup {}).is_ok());
        w.run_ms(200);
    }
    assert_eq!(summary(&w, 0, &g).0, 3);
    assert_eq!(summary(&w, 1, &g).0, 0);
    // Ben comes back. The leader's advertised tag is epoch 3's, which Ben cannot match;
    // its HELLO shows an epoch 3 ahead, so Ben opens a speculative link with his newest
    // (epoch-0) PSK, which the leader still retains.
    start(&mut w, 1, &g);
    w.in_range = vec![(0, 1)];
    for _ in 0..40 {
        w.advertise();
        w.run_ms(500);
        if !up_peers(&w, 1).is_empty() {
            break;
        }
    }
    assert_eq!(
        up_peers(&w, 1),
        vec![node_id(&w, 0, &g)],
        "readmitted under epoch 0"
    );
    let l = w.nodes[1]
        .engine
        .ctx()
        .links
        .iter()
        .next()
        .map(|l| l.info.role);
    assert_eq!(l, Some(LinkRole::Initiator), "Ben is the BLE central");
    w.run_ms(2_000);
    // Anti-entropy: CAPS showed epoch 0, the leader pushed the latest REKEY (sealed to
    // every roster member) and the roster; the authenticated link stays up.
    assert_eq!(summary(&w, 1, &g), summary(&w, 0, &g));
    assert_eq!(group(&w, 1, &g).epoch(), 3);
    assert_eq!(up_peers(&w, 1).len(), 1);
}

/// What the removal flow left behind on every node (compared across timer cadences).
#[derive(Debug, PartialEq, Eq)]
struct Removal {
    summaries: Vec<(u32, u32, u32)>,
    rosters: Vec<Vec<u16>>,
    up: Vec<Vec<u16>>,
    notices: Vec<Vec<NoticeKey>>,
}

/// Remove member, end to end (§8.6, §1.3 F9), with an optional periodic wakeup of every
/// engine on top of their own deadlines (`tick_us`; the runtime ticks every ≤ 5 ms) and
/// the link capacity from the removal on (`rate_bps`).
fn removal_flow(tick_us: Option<u64>, rate_bps: u64) -> Removal {
    let (mut w, g) = members(4, Platform::Ios, 66);
    w.tick_us = tick_us;
    for i in 1..4 {
        w.connect(i, 0);
    }
    w.run_ms(3_000);
    let (c, d) = (node_id(&w, 2, &g), node_id(&w, 3, &g));
    assert_eq!(roster(&w, 0, &g).len(), 4);
    w.connect(3, 1);
    w.run_ms(2_000);
    // Dee is offline when she is removed.
    stop(&mut w, 3);
    w.run_ms(500);
    assert_eq!(
        w.command(0, UiCommand::RemoveMember { node_id: 0xFFFF })
            .error_code(),
        Some(ErrorCode::UnknownMember)
    );
    w.rate_bps = rate_bps;
    assert!(w.command(0, UiCommand::RemoveMember { node_id: d }).is_ok());
    // Cid is online when he is removed: the REKEY of Dee's removal and the roster
    // denying him reach him ahead of the leader's CLOSE{5}; he floods a signed
    // LEAVE{removed} and his media session ends.
    assert!(w.command(0, UiCommand::RemoveMember { node_id: c }).is_ok());
    w.run_ms(4_000);
    assert!(notices(&w, 2).contains(&NoticeKey::Removed));
    assert!(media_requests(&w, 2).contains(&MediaRequest::EndSession));
    assert!(close_reasons(&w, 0).contains(&RejectReason::MemberRemovedOrDenied));
    assert!(up_peers(&w, 2).is_empty());
    assert!(!up_peers(&w, 0).contains(&c));
    assert_eq!(summary(&w, 1, &g), summary(&w, 0, &g));
    assert_eq!(summary(&w, 0, &g).0, 2, "one rekey per removal");
    assert_eq!(
        summary(&w, 2, &g).0,
        1,
        "Cid opened the REKEY of Dee's removal, not the one of his own"
    );
    let left = roster(&w, 1, &g);
    assert!(!left.contains(&c) && !left.contains(&d));
    // The members' media identity follows the new epoch.
    let epoch = media_requests(&w, 1)
        .into_iter()
        .rev()
        .find_map(|r| match r {
            MediaRequest::Identity(id) => Some(id.epoch),
            _ => None,
        });
    assert_eq!(epoch, Some(2));
    // Dee returns with her stored epoch-0 secret and dials a member: the handshake under
    // the retained PSK completes, IDPROOF finds her denied, REJECT{5}. She never saw
    // the roster that denies her: the reason tells her she was removed.
    start(&mut w, 3, &g);
    w.connect(3, 1);
    w.run_ms(2_000);
    assert!(up_peers(&w, 3).is_empty());
    assert!(close_reasons(&w, 1).contains(&RejectReason::MemberRemovedOrDenied));
    assert!(notices(&w, 3).contains(&NoticeKey::Removed));
    assert!(media_requests(&w, 3).contains(&MediaRequest::EndSession));
    assert_eq!(
        w.command(3, UiCommand::GoLive {}).error_code(),
        Some(ErrorCode::NoSession),
        "a refused session does not go live again"
    );
    Removal {
        summaries: (0..4).map(|i| summary(&w, i, &g)).collect(),
        rosters: (0..4).map(|i| roster(&w, i, &g)).collect(),
        up: (0..4).map(|i| up_peers(&w, i)).collect(),
        notices: (0..4)
            .map(|i| {
                let mut n = notices(&w, i);
                n.sort();
                n.dedup();
                n
            })
            .collect(),
    }
}

#[test]
fn removed_member_is_closed_and_a_denied_member_is_refused_later() {
    removal_flow(None, 1_000_000);
}

/// Regression: removal and rekey are sequenced by what each link carries, in order (the
/// REKEY and the ROSTER that denies Cid, then CLOSE{5}), never by a wall-clock window
/// guessing when they "had time to leave". Extra engine wakeups at the runtime's 5 ms
/// tick or at 1 Hz change nothing, and neither does a link as slow as the 8 kbps
/// idle-link grant (§5.6), where the old 300 ms close window cut the roster off.
#[test]
fn removal_and_rekey_do_not_depend_on_the_timer_cadence() {
    let base = removal_flow(None, 1_000_000);
    for tick in [5_000, 1_000_000] {
        assert_eq!(removal_flow(Some(tick), 1_000_000), base, "tick {tick} µs");
    }
    for tick in [None, Some(5_000)] {
        assert_eq!(removal_flow(tick, 8_000), base, "8 kbps, tick {tick:?}");
    }
}

#[test]
fn preview_access_loss_lists_unconfirmed_neighbors() {
    let (mut w, g) = pair(67);
    let url = invite_url(&w, 0, &g);
    let u = add_node(&mut w, Platform::Ios, 68);
    join(&mut w, u, &url, "Uma");
    start(&mut w, u, &g);
    w.connect(u, 0);
    // Step until the leader admitted the link; the JOIN_REQ is still in flight.
    let me = node_id(&w, u, &g);
    for _ in 0..200 {
        w.run_ms(1);
        if up_peers(&w, 0).contains(&me) {
            break;
        }
    }
    assert!(!roster(&w, 0, &g).contains(&me));
    assert_eq!(preview(&mut w, 0), vec![me], "would lose access on lock");
    w.run_ms(1_000);
    assert!(roster(&w, 0, &g).contains(&me));
    assert!(preview(&mut w, 0).is_empty());
}
