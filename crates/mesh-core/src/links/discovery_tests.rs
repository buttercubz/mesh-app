//! Discovery duty cycle (§5.7, §9.5, first field test) and the per-peer L2CAP → GATT
//! fallback (§9.2).

use super::*;
use crate::effects::Effect;
use mesh_wire::adv::AdvFlags;

const TAG: [u8; 4] = [1, 2, 3, 4];
/// Lowest token: every Android peer below is one I initiate to (§9.2).
const ME: [u8; 4] = [0, 0, 0, 1];
const A: [u8; 4] = [5, 0, 0, 0];
const S: u64 = 1_000_000;

fn policy(target: RadioTarget) -> (RadioPolicy, EffectQueue, Limits) {
    let mut p = RadioPolicy::new(Platform::Android);
    let mut fx = EffectQueue::new();
    p.start(target, ME, &mut fx);
    (p, fx, Limits::for_platform(Platform::Android))
}

fn group() -> RadioTarget {
    RadioTarget::Group {
        group_tag: TAG,
        epoch: 0,
    }
}

fn android(disc_id: u32, token: [u8; 4]) -> Discovery {
    let sd = ServiceData {
        flags: AdvFlags::new(true, false),
        group_tag: TAG,
        node_token: token,
    };
    Discovery {
        disc_id,
        transport: Transport::Ble,
        rssi: Some(-50),
        adv: AdvData::ServiceData(sd.encode().to_vec()),
        platform_hint: None,
    }
}

fn radio(fx: &mut EffectQueue) -> Vec<RadioCommand> {
    core::iter::from_fn(|| fx.pop())
        .filter_map(|e| match e {
            Effect::Radio(c) => Some(c),
            _ => None,
        })
        .collect()
}

/// The last `SetDiscovery` emitted, if any.
fn mode(fx: &mut EffectQueue) -> Option<(DiscoveryMode, ScanIntensity)> {
    radio(fx).into_iter().rev().find_map(|c| match c {
        RadioCommand::SetDiscovery { mode, scan, .. } => Some((mode, scan)),
        _ => None,
    })
}

const STABLE: (DiscoveryMode, ScanIntensity) = (DiscoveryMode::Stable, ScanIntensity::LowPower);
const HUNT: (DiscoveryMode, ScanIntensity) = (DiscoveryMode::Forming, ScanIntensity::Balanced);
const ALONE: (DiscoveryMode, ScanIntensity) = (DiscoveryMode::Forming, ScanIntensity::LowLatency);

/// Runs the policy second by second with `links` up; returns the mode at each second.
fn seconds(
    p: &mut RadioPolicy,
    fx: &mut EffectQueue,
    l: &Limits,
    from: u64,
    n: u64,
    links: usize,
) -> Vec<(DiscoveryMode, ScanIntensity)> {
    let mut cur = p.sent_mode.as_ref().map(|m| (m.0, m.2)).expect("a mode");
    (from..from + n)
        .map(|s| {
            p.on_timer(Instant(s * S), links, l, fx);
            if let Some(m) = mode(fx) {
                cur = m;
            }
            cur
        })
        .collect()
}

#[test]
fn every_member_reachable_is_stable_below_the_link_target() {
    let (mut p, mut fx, l) = policy(group());
    // One link, nobody missing, nothing else heard: Stable (a pair, a line end).
    p.refresh(1, &l, Instant(0), &mut fx);
    assert_eq!(mode(&mut fx), Some(STABLE));
    // Under target, an unlinked peer of ours advertises: hunt (duty-cycled) ...
    p.on_discovered(Instant(S), &android(10, A), 1, &l, &mut fx);
    assert_eq!(mode(&mut fx), Some(HUNT));
    // ... until it has not been heard for WANTED_FRESH_US.
    let m = seconds(&mut p, &mut fx, &l, 2, 40, 1);
    assert_eq!(m[(WANTED_FRESH_US / S) as usize - 2], HUNT);
    assert_eq!(*m.last().unwrap(), STABLE);
    // At the link target an unlinked peer is no reason to hunt.
    p.on_discovered(Instant(50 * S), &android(10, A), 2, &l, &mut fx);
    p.refresh(2, &l, Instant(50 * S), &mut fx);
    assert_eq!(mode(&mut fx), None, "still Stable");
}

#[test]
fn a_missing_member_is_hunted_in_bounded_windows_never_continuously() {
    let (mut p, mut fx, l) = policy(group());
    p.refresh(1, &l, Instant(0), &mut fx);
    radio(&mut fx);
    assert!(p.set_force_forming(true));
    p.refresh(1, &l, Instant(0), &mut fx);
    assert_eq!(mode(&mut fx), Some(HUNT));
    let m = seconds(&mut p, &mut fx, &l, 1, 600, 1);
    // The first window is HUNT_FIRST_US, then a quiet period.
    let first = (HUNT_FIRST_US / S) as usize;
    assert!(m[..first - 1].iter().all(|x| *x == HUNT));
    assert_eq!(m[first], STABLE);
    // LOW_LATENCY never while a link is up; bounded Forming overall (field: 57/60 s).
    assert!(m.iter().all(|x| *x != ALONE));
    let hunting = m.iter().filter(|x| **x == HUNT).count();
    assert!(hunting <= 150, "{hunting} s of 600 hunting");
    // Later windows keep coming (a lost member is still looked for).
    assert!(m[500..].contains(&HUNT) && m[500..].contains(&STABLE));
    // With no link up nothing can be starved: continuous LOW_LATENCY.
    let m0 = seconds(&mut p, &mut fx, &l, 601, 60, 0);
    assert!(m0.iter().all(|x| *x == ALONE), "{m0:?}");
    // Everybody reachable again: Stable at once; a new loss starts a fresh hunt.
    assert!(p.set_force_forming(false));
    p.refresh(1, &l, Instant(700 * S), &mut fx);
    assert_eq!(mode(&mut fx), Some(STABLE));
    p.set_force_forming(true);
    p.refresh(1, &l, Instant(701 * S), &mut fx);
    assert_eq!(mode(&mut fx), Some(HUNT));
    let m = seconds(&mut p, &mut fx, &l, 702, 25, 1);
    assert!(m.iter().all(|x| *x == HUNT));
    // Bluetooth off wins over everything.
    p.on_radio_state(
        RadioAvail::Off,
        RadioAvail::Off,
        0,
        &l,
        Instant(730 * S),
        &mut fx,
    );
    assert_eq!(
        mode(&mut fx).map(|m| m.0),
        Some(DiscoveryMode::Off),
        "radio off"
    );
}

#[test]
fn quiet_periods_differ_between_nodes() {
    // Two nodes of one side lose the other side at the same instant: their quiet
    // periods are stretched by per-node factors so they do not hunt in lockstep.
    let base = QUIET_US[2];
    let a = jitter(&[1, 2, 3, 4], 2, base);
    let b = jitter(&[9, 8, 7, 6], 2, base);
    assert_ne!(a, b);
    for q in [a, b] {
        assert!((base * 3 / 4..base * 5 / 4).contains(&q), "{q}");
    }
}

#[test]
fn a_lost_member_heard_in_a_quiet_period_opens_a_hunt_window() {
    let (mut p, mut fx, l) = policy(group());
    p.set_force_forming(true);
    p.set_lost_tokens([A].into());
    p.refresh(1, &l, Instant(0), &mut fx);
    let m = seconds(&mut p, &mut fx, &l, 1, 31, 1);
    assert_eq!(*m.last().unwrap(), STABLE, "quiet");
    // A is in range again (it advertises): hunt now, so it finds us fast too.
    p.on_discovered(Instant(32 * S), &android(10, A), 1, &l, &mut fx);
    assert_eq!(mode(&mut fx), Some(HUNT));
    let m = seconds(&mut p, &mut fx, &l, 33, 12, 1);
    assert_eq!(*m.last().unwrap(), STABLE, "the evidence window is HUNT_US");
    // Within EVIDENCE_GAP_US another advertisement does not reopen it.
    p.on_discovered(Instant(46 * S), &android(10, A), 1, &l, &mut fx);
    assert_ne!(mode(&mut fx), Some(HUNT));
}

#[test]
fn lab_hunts_under_the_link_target_like_a_group() {
    let (mut p, mut fx, l) = policy(RadioTarget::Lab { lab_tag: 7 });
    p.refresh(0, &l, Instant(0), &mut fx);
    assert_eq!(mode(&mut fx), Some(ALONE));
    p.refresh(1, &l, Instant(S), &mut fx);
    assert_eq!(mode(&mut fx), Some(HUNT));
    let m = seconds(&mut p, &mut fx, &l, 2, 120, 1);
    assert!(m.contains(&STABLE), "duty-cycled");
    p.refresh(2, &l, Instant(200 * S), &mut fx);
    assert_eq!(mode(&mut fx), Some(STABLE));
}

#[test]
fn a_failed_l2cap_open_makes_the_next_link_to_that_peer_gatt() {
    let (mut p, mut fx, l) = policy(group());
    radio(&mut fx);
    let flags = HelloFlags::new(true, false, true, false, false);
    let hello = Hello::group(flags, 0x80, TAG, A, 0).encode().unwrap();
    let open = |p: &mut RadioPolicy, fx: &mut EffectQueue, t: u64| {
        p.on_discovered(Instant(t), &android(10, A), 0, &l, fx);
        assert!(
            radio(fx).contains(&RadioCommand::Connect { disc_id: 10 }),
            "connect at {t}"
        );
        p.on_hello_read(Instant(t + 1), 10, &hello, &l, fx);
        radio(fx).into_iter().find_map(|c| match c {
            RadioCommand::OpenLink { via, .. } => Some(via),
            _ => None,
        })
    };
    assert_eq!(open(&mut p, &mut fx, S), Some(OpenVia::L2cap { psm: 0x80 }));
    // The L2CAP channel never opens (a dead PSM on the peer).
    p.on_connect_failed(Instant(2 * S), 10, LinkDownReason::Timeout, &l);
    // After the per-peer backoff the same peer is linked over GATT.
    assert_eq!(open(&mut p, &mut fx, 10 * S), Some(OpenVia::Gatt));
    // A failure before the HELLO check says nothing about L2CAP.
    let (mut p, mut fx, l) = policy(group());
    radio(&mut fx);
    p.on_discovered(Instant(S), &android(10, A), 0, &l, &mut fx);
    p.on_connect_failed(Instant(2 * S), 10, LinkDownReason::Timeout, &l);
    assert!(p.gatt_peers.is_empty());
}
