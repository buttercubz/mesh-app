//! §5.7 hooks of the radio policy: candidate classes.

use super::*;
use crate::effects::Effect;
use mesh_wire::adv::AdvFlags;

const TAG: [u8; 4] = [1, 2, 3, 4];
/// My token is the lowest, so every Android peer below is one I initiate to (§9.2).
const ME: [u8; 4] = [0, 0, 0, 1];
const UNREACHABLE: [u8; 4] = [5, 0, 0, 0];
const REACHABLE: [u8; 4] = [6, 0, 0, 0];

fn policy() -> (RadioPolicy, EffectQueue, Limits) {
    let mut p = RadioPolicy::new(Platform::Android);
    let mut fx = EffectQueue::new();
    let target = RadioTarget::Group {
        group_tag: TAG,
        epoch: 0,
    };
    p.start(target, ME, &mut fx);
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

fn radio(fx: &mut EffectQueue) -> Vec<RadioCommand> {
    core::iter::from_fn(|| fx.pop())
        .filter_map(|e| match e {
            Effect::Radio(c) => Some(c),
            _ => None,
        })
        .collect()
}

fn connects(fx: &mut EffectQueue) -> Vec<u32> {
    radio(fx)
        .into_iter()
        .filter_map(|c| match c {
            RadioCommand::Connect { disc_id } => Some(disc_id),
            _ => None,
        })
        .collect()
}

/// Discovers every peer while at the BLE max (no connect yet), then lets the policy
/// connect one candidate at a time, failing each attempt: the connect order.
fn order(p: &mut RadioPolicy, fx: &mut EffectQueue, l: &Limits, peers: &[Discovery]) -> Vec<u32> {
    let max = usize::from(l.ble_max());
    for d in peers {
        p.on_discovered(Instant(1_000), d, max, l, fx);
    }
    assert!(connects(fx).is_empty(), "no connect at the link max");
    let mut seen = Vec::new();
    for k in 0..peers.len() as u64 {
        let now = Instant(2_000 + k);
        p.on_timer(now, 0, l, fx);
        let c = connects(fx);
        assert_eq!(c.len(), 1, "one outstanding connect: {c:?}");
        seen.push(c[0]);
        p.on_connect_failed(now, c[0], LinkDownReason::Timeout, l);
    }
    seen
}

#[test]
fn candidates_connect_unreachable_members_first_then_unknown_then_rssi() {
    let peers = [
        android(10, REACHABLE, -40),
        android(11, [7, 0, 0, 0], -55),
        android(12, UNREACHABLE, -90),
        android(13, [8, 0, 0, 0], -50),
    ];
    // Without the mesh view the order is RSSI only.
    let (mut p, mut fx, l) = policy();
    assert_eq!(order(&mut p, &mut fx, &l, &peers), [10, 13, 11, 12]);
    // With it: the unreachable member's token first (weakest signal), then unknown
    // tokens by RSSI, then the reachable member (strongest signal) last.
    let (mut p, mut fx, l) = policy();
    p.set_member_tokens([UNREACHABLE].into(), [REACHABLE].into());
    assert_eq!(order(&mut p, &mut fx, &l, &peers), [12, 13, 11, 10]);
    // Classes of the other advertising forms.
    assert_eq!(
        p.class_of(PeerKey::Prefix(token_prefix14(&UNREACHABLE))),
        CandidateClass::UnreachableMember
    );
    assert_eq!(
        p.class_of(PeerKey::Prefix(token_prefix14(&REACHABLE))),
        CandidateClass::ReachableMember
    );
    assert_eq!(p.class_of(PeerKey::Disc(4)), CandidateClass::Unknown);
    // Engine stop forgets the view.
    p.stop(&mut fx);
    assert_eq!(
        p.class_of(PeerKey::Token(UNREACHABLE)),
        CandidateClass::Unknown
    );
}
