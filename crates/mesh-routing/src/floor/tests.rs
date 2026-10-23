use mesh_wire::mesh::{EncoderProfile, FloorRejectReason, Talk, TalkMode, TalkPrio, TalkState};

use super::*;
use crate::types::{Instant, LinkId};

const TIE: u64 = 150_000;

fn talk(state: TalkState, burst: u8, prio: TalkPrio, elapsed: u16) -> Talk {
    Talk {
        burst,
        state,
        prio,
        mode: TalkMode::Vox,
        profile: EncoderProfile::Ble16,
        elapsed_10ms: elapsed,
    }
}

fn key(origin: u16) -> BurstKey {
    BurstKey::new(origin, 1, 1)
}

#[test]
fn entry_lifecycle() {
    let mut t = FloorTable::new(1, TIE, true);
    let now = Instant::from_secs(1);
    let ev = t.on_talk(
        now,
        key(5),
        &talk(TalkState::Start, 1, TalkPrio::Member, 0),
        0,
    );
    assert_eq!(ev, vec![FloorEvent::Started(key(5))]);
    assert!(t.on_voice(now.plus_ms(40), key(5), false, 4, 0).is_empty());
    assert_eq!(
        t.get(key(5)).map(|e| e.profile),
        Some(Some(EncoderProfile::Ble16))
    );
    // A new burst of the same origin replaces the old one.
    let k2 = BurstKey::new(5, 1, 2);
    let ev = t.on_voice(now.plus_ms(100), k2, false, 0, 0);
    assert_eq!(
        ev,
        vec![
            FloorEvent::Ended(key(5), EndReason::Replaced),
            FloorEvent::Started(k2)
        ]
    );
    // Idle release after 600 ms.
    assert!(t.expire(now.plus_ms(699)).is_empty());
    assert_eq!(
        t.expire(now.plus_ms(700)),
        vec![FloorEvent::Ended(k2, EndReason::Idle)]
    );
    // STOP removes; reject tombstones.
    t.on_voice(now, key(6), false, 0, 0);
    assert_eq!(
        t.on_talk(
            now,
            key(6),
            &talk(TalkState::Stop, 1, TalkPrio::Member, 3),
            0
        ),
        vec![FloorEvent::Ended(key(6), EndReason::Stop)]
    );
    t.on_voice(now, key(7), false, 0, 0);
    assert_eq!(
        t.on_reject(key(7)),
        Some(FloorEvent::Ended(key(7), EndReason::Rejected))
    );
    assert_eq!(t.on_reject(key(7)), None);
    assert!(t.top(4).is_empty());
    // A late frame of a rejected burst does not resurrect it.
    assert!(t.on_voice(now.plus_ms(10), key(7), false, 1, 0).is_empty());
    assert!(t.top(4).is_empty());
}

#[test]
fn rank_from_age_and_burst_relative_time() {
    let mut t = FloorTable::new(1, TIE, true);
    let now = Instant::from_secs(100);
    // A 20 s-old burst seen first mid-burst ranks as old as a START seen 20 s ago.
    t.on_voice(now, key(9), false, 2_000, 30_000);
    let e = t.get(key(9)).copied().expect("entry");
    assert_eq!(e.t_origin_us, now.us() as i64 - 30_000 - 20_000_000);
    t.on_talk(
        now,
        key(3),
        &talk(TalkState::Start, 1, TalkPrio::Member, 0),
        0,
    );
    assert_eq!(t.top(1), vec![key(9)]);
    // Leader priority wins over age; off by policy it does not.
    t.on_talk(
        now,
        key(4),
        &talk(TalkState::Start, 1, TalkPrio::Leader, 0),
        0,
    );
    assert_eq!(t.top(1), vec![key(4)]);
    let mut np = FloorTable::new(1, TIE, false);
    np.on_voice(now, key(9), false, 2_000, 30_000);
    np.on_talk(
        now,
        key(4),
        &talk(TalkState::Start, 1, TalkPrio::Leader, 0),
        0,
    );
    assert_eq!(np.top(1), vec![key(9)]);
    // KEEP creates the entry too, with elapsed_10ms.
    let mut j = FloorTable::new(1, TIE, true);
    j.on_talk(
        now,
        key(9),
        &talk(TalkState::Keep, 1, TalkPrio::Member, 2_000),
        30_000,
    );
    assert_eq!(j.get(key(9)).map(|e| e.t_origin_us), Some(e.t_origin_us));
}

#[test]
fn table_capacity_eight() {
    let mut t = FloorTable::new(1, TIE, true);
    let now = Instant::from_secs(10);
    for o in 1..=8u16 {
        t.on_voice(now.plus_ms(u64::from(o) * 1_000), key(o), false, 0, 0);
    }
    assert_eq!(t.entries().len(), 8);
    // Ranks below everyone: ignored.
    assert!(
        t.on_voice(now.plus_ms(20_000), key(20), false, 0, 0)
            .is_empty()
    );
    // A leader evicts the lowest-ranked (the latest start).
    let ev = t.on_voice(now.plus_ms(20_000), key(21), true, 0, 0);
    assert_eq!(
        ev,
        vec![
            FloorEvent::Ended(key(8), EndReason::Evicted),
            FloorEvent::Started(key(21))
        ]
    );
}

#[test]
fn admission_rules() {
    let mut t = FloorTable::new(1, TIE, true);
    let mut a = Admission::new(300_000);
    let l = LinkId(1);
    let now = Instant::from_secs(10);
    t.on_voice(now, key(1), false, 0, 0);
    t.on_voice(now.plus_ms(1_000), key(2), false, 0, 0);
    t.on_voice(now.plus_ms(2_000), key(3), false, 0, 0);
    let at = now.plus_ms(2_000);
    assert_eq!(
        a.admits(at, l, key(1), &mut t, 2, false),
        AdmitDecision::Admitted
    );
    assert_eq!(
        a.admits(at, l, key(2), &mut t, 2, false),
        AdmitDecision::Admitted
    );
    // Over the cap, lower rank, outside stickiness: refused once with CAP, then silently.
    let later = at.plus_ms(400);
    assert_eq!(
        a.admits(later, l, key(3), &mut t, 2, false),
        AdmitDecision::Refused {
            reject: Some(FloorRejectReason::Cap)
        }
    );
    assert_eq!(
        a.admits(later, l, key(3), &mut t, 2, true),
        AdmitDecision::Refused { reject: None }
    );
    // A leader preempts the lowest-ranked admitted burst.
    t.on_voice(later, key(4), true, 0, 0);
    assert_eq!(
        a.admits(later, l, key(4), &mut t, 2, false),
        AdmitDecision::AdmittedEvicting {
            victim: key(2),
            reason: FloorRejectReason::Preempted
        }
    );
    assert_eq!(a.admitted(l), vec![key(1), key(4)]);
}

#[test]
fn sticky_window_converges_on_rank() {
    let mut t = FloorTable::new(1, 0, true);
    let mut a = Admission::new(300_000);
    let l = LinkId(1);
    let now = Instant::from_secs(10);
    // key(2) started earlier but its first packet arrives after key(3) was admitted.
    t.on_voice(now, key(1), false, 0, 0);
    t.on_voice(now, key(3), false, 0, 0);
    a.admits(now, l, key(1), &mut t, 2, false);
    a.admits(now, l, key(3), &mut t, 2, false);
    t.on_voice(now.plus_ms(50), key(2), false, 0, 60_000);
    assert_eq!(
        a.admits(now.plus_ms(50), l, key(2), &mut t, 2, false),
        AdmitDecision::AdmittedEvicting {
            victim: key(3),
            reason: FloorRejectReason::Outranked
        }
    );
    // After T_STICKY equal priority never evicts.
    let mut t2 = t.clone();
    let mut a2 = Admission::new(300_000);
    a2.admits(now, l, key(1), &mut t2, 2, false);
    a2.admits(now, l, key(3), &mut t2, 2, false);
    let d = a2.admits(now.plus_ms(300), l, key(2), &mut t2, 2, false);
    assert!(!d.admitted());
}

#[test]
fn shedding_and_cap_drop() {
    let mut t = FloorTable::new(1, TIE, true);
    let mut a = Admission::new(300_000);
    let l = LinkId(1);
    let now = Instant::from_secs(10);
    t.on_voice(now, key(1), false, 0, 0);
    t.on_voice(now.plus_ms(500), key(2), false, 0, 0);
    a.admits(now.plus_ms(500), l, key(1), &mut t, 2, false);
    a.admits(now.plus_ms(500), l, key(2), &mut t, 2, false);
    assert_eq!(a.shed_lowest(l, &t), Some(key(2)));
    // Not re-admitted on that link until it ends, even with room.
    assert!(
        !a.admits(now.plus_ms(600), l, key(2), &mut t, 2, false)
            .admitted()
    );
    a.on_burst_ended(key(2));
    t.on_talk(
        now.plus_ms(600),
        key(2),
        &talk(TalkState::Stop, 1, TalkPrio::Member, 1),
        0,
    );
    // A cap drop to 1 sheds the lowest-ranked.
    t.on_voice(now.plus_ms(700), key(3), false, 0, 0);
    a.admits(now.plus_ms(700), l, key(3), &mut t, 2, false);
    assert!(
        a.admits(now.plus_ms(800), l, key(1), &mut t, 1, true)
            .admitted()
    );
    assert_eq!(a.admitted(l), vec![key(1)]);
    assert!(
        !a.admits(now.plus_ms(800), l, key(3), &mut t, 1, true)
            .admitted()
    );
}

#[test]
fn local_admit_and_near_talker() {
    let mut t = FloorTable::new(1, TIE, true);
    let now = Instant::from_secs(10);
    t.on_voice(now, key(1), false, 0, 0);
    t.on_voice(now, key(2), false, 0, 0);
    let me = BurstKey::new(9, 1, 1);
    let later = now.plus_ms(1_000);
    assert!(!t.local_admit(later, me, 1, 2));
    assert!(t.local_admit(later, me, 1, 3));
    assert!(t.local_admit(later, me, 2, 2));
    // My own older burst never blocks me.
    t.on_voice(now, BurstKey::new(9, 1, 0), false, 0, 0);
    assert!(t.local_admit(later, me, 1, 3));
    // Near-talker: hops ≤ 1 and outside the top 2 ⇒ once.
    t.on_voice(later, key(5), false, 0, 0);
    assert!(t.near_talker_reject(key(5), 1, 2));
    assert!(!t.near_talker_reject(key(5), 1, 2));
    t.on_voice(later, key(6), false, 0, 0);
    assert!(!t.near_talker_reject(key(6), 2, 2));
    assert!(!t.near_talker_reject(key(1), 0, 2));
}
