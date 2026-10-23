use mesh_wire::mesh::Presence;

use super::*;
use crate::consts::{MEMBER_LOST_US, VOICE_MAX_AGE_US};

fn presence(heard: u32) -> Presence {
    Presence {
        heard,
        ..Presence::default()
    }
}

/// Me = node 1 (index 0); members 2, 3, 4 at indices 1, 2, 3.
fn reach() -> Reachability {
    let mut r = Reachability::new(1, Instant::ZERO, 4, VOICE_MAX_AGE_US);
    r.set_roster(&[(1, 0), (2, 1), (3, 2), (4, 3)]);
    r
}

#[test]
fn voice_reach_classes() {
    let c = |t, a, aw| VoiceReach::classify(t, a, aw, 4, VOICE_MAX_AGE_US);
    assert_eq!(c(1, 10_000, false), VoiceReach::Kpi);
    assert_eq!(c(2, 60_000, false), VoiceReach::Kpi);
    assert_eq!(c(3, 60_000, false), VoiceReach::BestEffort);
    assert_eq!(c(4, 120_000, false), VoiceReach::BestEffort);
    assert_eq!(c(5, 60_000, false), VoiceReach::No);
    assert_eq!(c(3, 200_000, false), VoiceReach::No);
    assert_eq!(c(4, 60_000, true), VoiceReach::Kpi);
    assert_eq!(VoiceReach::BestEffort.key(), "bestEffort");
}

#[test]
fn who_hears_me_one_way_and_bitmap() {
    let mut r = reach();
    let t = Instant::from_secs(1);
    // 2 hears me (bit 0); 3 does not.
    r.on_presence(t, 2, 9, 0, 4_000, &presence(0b0101), false);
    r.on_presence(t, 3, 9, 1, 30_000, &presence(0b0010), false);
    assert_eq!(r.heard_bitmap(t), 0b0110);
    assert_eq!(r.who_hears_me(t), vec![2]);
    assert_eq!(r.one_way(t), vec![3]);
    assert_eq!(r.voice_reach(t, 2), VoiceReach::Kpi);
    assert_eq!(r.voice_reach(t, 3), VoiceReach::Kpi);
    assert_eq!(r.voice_reach(t, 4), VoiceReach::No);
    assert!(r.reachable(t.plus_us(MEMBER_LOST_US - 1), 2));
    assert!(!r.reachable(t.plus_us(MEMBER_LOST_US), 2));
    // Unconfirmed members are tracked by node_id only (no bitmap bit).
    r.on_presence(t, 77, 1, 0, 0, &presence(0), false);
    assert_eq!(r.heard_bitmap(t), 0b0110);
    assert!(r.reachable(t, 77));
}

#[test]
fn islands_from_hears_graph() {
    let mut r = reach();
    let t = Instant::from_secs(1);
    // 1-2 hear each other; 3-4 hear each other, nobody bridges.
    r.on_presence(t, 2, 1, 0, 0, &presence(0b0001), false);
    let v = r.islands(t);
    assert_eq!(v.islands, vec![vec![1, 2], vec![3], vec![4]]);
    // 2 hears 3 and 3 hears 4: one island.
    r.on_presence(t, 2, 1, 0, 0, &presence(0b0101), false);
    r.on_presence(t, 3, 1, 1, 0, &presence(0b1010), false);
    assert_eq!(r.islands(t).count(), 1);
}

#[test]
fn lost_back_aggregation_alone_and_partition() {
    let mut r = reach();
    let t = Instant::from_secs(1);
    r.on_presence(t, 2, 1, 0, 0, &presence(0b1101), false);
    r.on_presence(t, 3, 1, 0, 0, &presence(0b1011), false);
    r.on_presence(t, 4, 1, 0, 0, &presence(0b0111), false);
    assert!(r.tick(t).is_empty());
    // Keep 2 alive; 3 and 4 fall silent.
    let mut events = Vec::new();
    let mut now = t;
    for _ in 0..30 {
        now = now.plus_ms(500);
        r.on_presence(now, 2, 1, 0, 0, &presence(0b0001), false);
        for e in r.tick(now) {
            events.push((now.since(t) / 1_000, e));
        }
    }
    // Lost at +6 s, reported once for both after 5 s of aggregation; the island count
    // changed (3 and 4 became separate) and the partition is reported 6 s later.
    assert!(
        events.contains(&(11_000, ReachEvent::MemberLost(vec![3, 4]))),
        "{events:?}"
    );
    assert!(
        events
            .iter()
            .any(|(_, e)| matches!(e, ReachEvent::Partition(3))),
        "{events:?}"
    );
    // 3 comes back: memberBack after aggregation.
    r.on_presence(now, 3, 1, 0, 0, &presence(0b0001), false);
    let mut back = Vec::new();
    for _ in 0..12 {
        now = now.plus_ms(500);
        back.extend(r.tick(now));
    }
    assert!(back.contains(&ReachEvent::MemberBack(vec![3])), "{back:?}");
    // Everyone silent: alone once, 10 s after the last member was reachable.
    let mut alone = Vec::new();
    for _ in 0..60 {
        now = now.plus_ms(500);
        alone.extend(r.tick(now).into_iter().filter(|e| *e == ReachEvent::Alone));
    }
    assert_eq!(alone, vec![ReachEvent::Alone]);
}

#[test]
fn lost_then_back_within_window_cancels() {
    let mut r = reach();
    let t = Instant::from_secs(1);
    r.on_presence(t, 2, 1, 0, 0, &presence(0b1), false);
    let lost_at = t.plus_us(MEMBER_LOST_US);
    assert!(r.tick(lost_at).is_empty());
    r.on_presence(lost_at.plus_ms(500), 2, 1, 0, 0, &presence(0b1), false);
    let ev = r.tick(lost_at.plus_ms(5_000));
    assert!(
        ev.iter()
            .all(|e| !matches!(e, ReachEvent::MemberLost(_) | ReachEvent::MemberBack(_)))
    );
}

#[test]
fn peer_behind_triggers_anti_entropy() {
    let mut r = reach();
    r.set_my_versions(10, 3);
    let t = Instant::from_secs(1);
    let mut p = presence(0).with_versions(9, 3);
    assert_eq!(
        r.on_presence(t, 2, 1, 0, 0, &p, false),
        vec![ReachEvent::PeerBehind(2)]
    );
    // Not from a direct neighbor: no push.
    assert!(r.on_presence(t, 3, 1, 1, 0, &p, false).is_empty());
    p = presence(0).with_versions(10, 3);
    assert!(r.on_presence(t, 2, 1, 0, 0, &p, false).is_empty());
    p = presence(0).with_versions(10, 2);
    assert_eq!(
        r.on_presence(t, 2, 1, 0, 0, &p, false),
        vec![ReachEvent::PeerBehind(2)]
    );
}
