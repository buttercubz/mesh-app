//! Rejoin behaviour of the radio policy (§5.7, §9.5; mesh-sim S4 convoy15-highway):
//! stale candidates and stale outstanding connects, full peers, backoff reset on
//! reappearance, links that die in their handshake, neighbors under a rotated token,
//! lost members (both sides initiate, spare slot at the max).

use super::*;
use crate::effects::Effect;
use crate::links::types::{LinkKind, LinkRole};
use mesh_wire::adv::AdvFlags;

const TAG: [u8; 4] = [1, 2, 3, 4];
/// Lowest token: every Android peer below is one I initiate to (§9.2).
const ME: [u8; 4] = [0, 0, 0, 1];
const A: [u8; 4] = [5, 0, 0, 0];
const B: [u8; 4] = [6, 0, 0, 0];
const S: u64 = 1_000_000;

fn policy(me: [u8; 4]) -> (RadioPolicy, EffectQueue, Limits) {
    policy_on(Platform::Android, me)
}

fn policy_on(platform: Platform, me: [u8; 4]) -> (RadioPolicy, EffectQueue, Limits) {
    let mut p = RadioPolicy::new(platform);
    let mut fx = EffectQueue::new();
    let target = RadioTarget::Group {
        group_tag: TAG,
        epoch: 0,
    };
    p.start(target, me, &mut fx);
    radio(&mut fx);
    (p, fx, Limits::for_platform(Platform::Android))
}

fn android(disc_id: u32, token: [u8; 4], rssi: i16) -> Discovery {
    let sd = ServiceData {
        flags: AdvFlags::new(true, false),
        group_tag: TAG,
        node_token: token,
    };
    Discovery {
        disc_id,
        transport: Transport::Ble,
        rssi: Some(rssi),
        adv: AdvData::ServiceData(sd.encode().to_vec()),
        platform_hint: None,
    }
}

fn hello(token: [u8; 4], accepting: bool) -> [u8; 16] {
    let flags = HelloFlags::new(true, false, accepting, false, false);
    Hello::group(flags, 0x80, TAG, token, 0).encode().unwrap()
}

fn radio(fx: &mut EffectQueue) -> Vec<RadioCommand> {
    core::iter::from_fn(|| fx.pop())
        .filter_map(|e| match e {
            Effect::Radio(c) => Some(c),
            _ => None,
        })
        .collect()
}

fn connects(cmds: &[RadioCommand]) -> Vec<u32> {
    cmds.iter()
        .filter_map(|c| match c {
            RadioCommand::Connect { disc_id } => Some(*disc_id),
            _ => None,
        })
        .collect()
}

/// HELLO `accepting` of the last `SetHello` in `cmds`, if any.
fn accepting(cmds: &[RadioCommand]) -> Option<bool> {
    cmds.iter().rev().find_map(|c| match c {
        RadioCommand::SetHello { bytes } => Hello::decode(bytes).ok().map(|h| h.flags.accepting()),
        _ => None,
    })
}

#[test]
fn an_unanswered_connect_to_a_stale_peer_yields_to_a_fresh_candidate() {
    let (mut p, mut fx, l) = policy(ME);
    p.on_discovered(Instant(0), &android(10, A, -50), 0, &l, &mut fx);
    assert_eq!(connects(&radio(&mut fx)), [10]);
    // A has not advertised since: it left. B shows up while the connect is out.
    p.on_discovered(Instant(2 * S), &android(11, B, -80), 0, &l, &mut fx);
    assert!(
        connects(&radio(&mut fx)).is_empty(),
        "A is still fresh at 2 s"
    );
    p.on_discovered(Instant(3_500_000), &android(11, B, -80), 0, &l, &mut fx);
    let cmds = radio(&mut fx);
    assert!(
        cmds.contains(&RadioCommand::Abort {
            disc_id: 10,
            ignore_s: 0
        }),
        "{cmds:?}"
    );
    assert_eq!(connects(&cmds), [11], "the slot goes to the fresh peer");
    // A connect whose peer keeps advertising is never preempted.
    let (mut p, mut fx, l) = policy(ME);
    p.on_discovered(Instant(0), &android(10, A, -50), 0, &l, &mut fx);
    radio(&mut fx);
    for k in 1..=7 {
        p.on_discovered(Instant(k * S), &android(10, A, -50), 0, &l, &mut fx);
        p.on_discovered(Instant(k * S), &android(11, B, -40), 0, &l, &mut fx);
        assert!(connects(&radio(&mut fx)).is_empty(), "at {k} s");
    }
}

#[test]
fn fresh_candidates_connect_before_stale_ones_whatever_their_class() {
    let (mut p, mut fx, l) = policy(ME);
    p.set_member_tokens([A].into(), [B].into());
    let max = usize::from(l.ble_max());
    // A (unreachable member: first class) was last seen 4 s ago, B (reachable) now.
    p.on_discovered(Instant(0), &android(10, A, -40), max, &l, &mut fx);
    p.on_discovered(Instant(4 * S), &android(11, B, -90), max, &l, &mut fx);
    assert!(connects(&radio(&mut fx)).is_empty(), "no room at the max");
    p.on_timer(Instant(4 * S + 1), 0, &l, &mut fx);
    assert_eq!(connects(&radio(&mut fx)), [11]);
    // Both fresh: the §5.7 class order holds.
    let (mut p, mut fx, l) = policy(ME);
    p.set_member_tokens([A].into(), [B].into());
    p.on_discovered(Instant(0), &android(10, A, -40), max, &l, &mut fx);
    p.on_discovered(Instant(S), &android(11, B, -90), max, &l, &mut fx);
    p.on_timer(Instant(S + 1), 0, &l, &mut fx);
    assert_eq!(connects(&radio(&mut fx)), [10]);
}

#[test]
fn a_full_peer_is_backed_off_not_ignored_for_a_minute() {
    let (mut p, mut fx, l) = policy(ME);
    p.on_discovered(Instant(0), &android(10, A, -50), 0, &l, &mut fx);
    radio(&mut fx);
    p.on_hello_read(Instant(S), 10, &hello(A, false), &l, &mut fx);
    assert!(
        radio(&mut fx).contains(&RadioCommand::Abort {
            disc_id: 10,
            ignore_s: 0
        }),
        "native keeps reporting the peer"
    );
    p.on_discovered(Instant(3 * S), &android(10, A, -50), 0, &l, &mut fx);
    assert!(connects(&radio(&mut fx)).is_empty(), "held FULL_RETRY_US");
    p.on_discovered(Instant(4_100_000), &android(10, A, -50), 0, &l, &mut fx);
    assert_eq!(
        connects(&radio(&mut fx)),
        [10],
        "retried once a slot may be free"
    );
    // Still full: the same short hold, not a doubled backoff.
    p.on_hello_read(Instant(5 * S), 10, &hello(A, false), &l, &mut fx);
    radio(&mut fx);
    p.on_discovered(Instant(8_100_000), &android(10, A, -50), 0, &l, &mut fx);
    assert_eq!(connects(&radio(&mut fx)), [10], "no growing backoff");
    // A HELLO of another ride is still ignored for `abort_ignore_s`.
    p.on_hello_read(Instant(9 * S), 10, &[0u8; 16], &l, &mut fx);
    assert!(radio(&mut fx).contains(&RadioCommand::Abort {
        disc_id: 10,
        ignore_s: l.abort_ignore_s
    }));
}

#[test]
fn the_backoff_resets_when_a_peer_reappears() {
    let (mut p, mut fx, l) = policy(ME);
    let mut at = Vec::new();
    // In range but failing: the backoff grows 5 s → 10 s → 20 s.
    for k in 0..=16 {
        let now = Instant(k * S);
        p.on_discovered(now, &android(10, A, -50), 0, &l, &mut fx);
        for d in connects(&radio(&mut fx)) {
            at.push(k);
            p.on_connect_failed(now, d, LinkDownReason::Timeout, &l);
        }
    }
    assert_eq!(at, [0, 5, 15]);
    // Gone for 7 s, then back: connected at once, not after the 20 s backoff.
    p.on_discovered(Instant(23 * S), &android(10, A, -50), 0, &l, &mut fx);
    assert_eq!(connects(&radio(&mut fx)), [10]);
}

#[test]
fn a_link_that_dies_in_its_handshake_frees_the_slot_and_backs_off() {
    let (mut p, mut fx, l) = policy(ME);
    p.on_discovered(Instant(0), &android(10, A, -50), 0, &l, &mut fx);
    p.on_hello_read(Instant(S), 10, &hello(A, true), &l, &mut fx);
    radio(&mut fx);
    let mut info = LinkInfo::new(LinkKind::BleL2cap, LinkRole::Initiator);
    info.disc_id = Some(10);
    p.on_link_opened(LinkId(7), &info);
    assert!(
        !p.connecting(),
        "the connect is over once the platform link exists"
    );
    // A rediscovery while the handshake runs is not a second connect to A.
    p.on_discovered(Instant(1_100_000), &android(10, A, -50), 0, &l, &mut fx);
    assert!(connects(&radio(&mut fx)).is_empty());
    p.on_discovered(Instant(1_200_000), &android(11, B, -50), 0, &l, &mut fx);
    assert_eq!(connects(&radio(&mut fx)), [11], "the slot moved on");
    p.on_connect_failed(Instant(1_300_000), 11, LinkDownReason::Timeout, &l);
    // Closed as a duplicate before IDPROOF completed: a failed attempt.
    p.on_link_down(Instant(2 * S), LinkId(7), LinkDownReason::LocalClose, &l);
    p.on_discovered(Instant(3 * S), &android(10, A, -50), 0, &l, &mut fx);
    assert!(connects(&radio(&mut fx)).is_empty(), "A is backed off");
}

#[test]
fn neighbors_are_never_candidates_under_a_rotated_token() {
    let (mut p, mut fx, l) = policy(ME);
    p.set_neighbor_tokens([A].into());
    p.on_discovered(Instant(0), &android(10, A, -50), 0, &l, &mut fx);
    assert!(connects(&radio(&mut fx)).is_empty());
    p.on_hello_read(Instant(0), 10, &hello(A, true), &l, &mut fx);
    assert!(
        !radio(&mut fx)
            .iter()
            .any(|c| matches!(c, RadioCommand::OpenLink { .. }))
    );
}

#[test]
fn both_sides_initiate_toward_a_lost_member() {
    // My token is the highest: by §9.2 token order the peer initiates.
    let me = [9, 0, 0, 0];
    let (mut p, mut fx, l) = policy(me);
    p.on_discovered(Instant(0), &android(10, A, -50), 0, &l, &mut fx);
    assert!(connects(&radio(&mut fx)).is_empty(), "the peer's turn");
    let (mut p, mut fx, l) = policy(me);
    p.set_lost_tokens([A].into());
    p.on_discovered(Instant(0), &android(10, A, -50), 0, &l, &mut fx);
    assert_eq!(connects(&radio(&mut fx)), [10], "a rejoin: either side");
}

#[test]
fn a_spare_slot_at_the_max_while_a_lost_member_advertises() {
    let (mut p, mut fx, l) = policy_on(Platform::Ios, [9, 0, 0, 0]);
    let max = usize::from(l.ble_max());
    p.set_force_forming(true);
    p.refresh(max, &l, Instant(0), &mut fx);
    assert_eq!(accepting(&radio(&mut fx)), Some(false), "full");
    p.set_lost_tokens([A].into());
    p.on_discovered(Instant(S), &android(10, A, -50), max, &l, &mut fx);
    let cmds = radio(&mut fx);
    assert_eq!(connects(&cmds), [10], "a connect toward the lost member");
    assert_eq!(accepting(&cmds), Some(false), "the spare is taken by it");
    p.on_connect_failed(Instant(S), 10, LinkDownReason::Timeout, &l);
    p.on_timer(Instant(1_500_000), max, &l, &mut fx);
    assert_eq!(
        accepting(&radio(&mut fx)),
        Some(true),
        "room for its connect"
    );
    // A reachable member's advert opens nothing; the spare closes 3 s after A.
    p.on_discovered(Instant(2 * S), &android(11, B, -50), max, &l, &mut fx);
    assert!(connects(&radio(&mut fx)).is_empty());
    p.on_timer(Instant(4_100_000), max, &l, &mut fx);
    assert_eq!(accepting(&radio(&mut fx)), Some(false));
    // Over the max there is no spare.
    p.on_discovered(Instant(5 * S), &android(10, A, -50), max + 1, &l, &mut fx);
    assert!(connects(&radio(&mut fx)).is_empty());
}

#[test]
fn an_android_phone_keeps_a_spare_slot_while_a_member_is_lost() {
    // A backgrounded iPhone advertises only in the overflow area, which Android never
    // sees, and it can rejoin only through an Android peripheral (mesh-sim S6).
    let (mut p, mut fx, l) = policy([9, 0, 0, 0]);
    let max = usize::from(l.ble_max());
    p.set_force_forming(true);
    p.refresh(max, &l, Instant(0), &mut fx);
    assert_eq!(accepting(&radio(&mut fx)), Some(false), "nobody lost yet");
    p.set_lost_tokens([A].into());
    p.on_timer(Instant(S), max, &l, &mut fx);
    assert_eq!(accepting(&radio(&mut fx)), Some(true));
    // Still open long after, without ever hearing A; the deadline is never in the past.
    p.on_timer(Instant(60 * S), max, &l, &mut fx);
    assert!(
        p.next_deadline(&l, Instant(60 * S))
            .is_none_or(|t| t > Instant(60 * S))
    );
    // Not while voice flows through it (an over-max close mid-burst could cut an
    // admitted talker's inbound path).
    assert!(p.set_voice_active(true));
    p.on_timer(Instant(62 * S), max, &l, &mut fx);
    assert_eq!(accepting(&radio(&mut fx)), Some(false), "mid-burst");
    assert!(p.set_voice_active(false));
    p.on_timer(Instant(63 * S), max, &l, &mut fx);
    assert_eq!(accepting(&radio(&mut fx)), Some(true), "after the burst");
    // Everybody reachable again: full.
    p.set_force_forming(false);
    p.set_lost_tokens(BTreeSet::new());
    p.on_timer(Instant(64 * S), max, &l, &mut fx);
    assert_eq!(accepting(&radio(&mut fx)), Some(false));
}

#[test]
fn a_walk_away_drop_settles_briefly_without_backoff_growth() {
    let (mut p, mut fx, l) = policy(ME);
    p.on_discovered(Instant(0), &android(10, A, -50), 0, &l, &mut fx);
    p.on_hello_read(Instant(S), 10, &hello(A, true), &l, &mut fx);
    radio(&mut fx);
    let mut info = LinkInfo::new(LinkKind::BleL2cap, LinkRole::Initiator);
    info.disc_id = Some(10);
    p.on_link_opened(LinkId(7), &info);
    p.on_link_up(LinkId(7), &info);
    // Supervision timeout at the edge of range, peer still advertising.
    let down = 2 * S;
    p.on_link_down(Instant(down), LinkId(7), LinkDownReason::Timeout, &l);
    let mut first = None;
    for k in 1..=20 {
        let now = Instant(down + k * 250_000);
        p.on_discovered(now, &android(10, A, -80), 0, &l, &mut fx);
        if !connects(&radio(&mut fx)).is_empty() {
            first = Some(now.0 - down);
            break;
        }
    }
    // A failed connect would have held it 5 s; the settle hold is 3 s.
    assert_eq!(first, Some(FULL_RETRY_US));
}
