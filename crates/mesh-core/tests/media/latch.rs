//! The lock-screen Talk latch (RT flag bit3) toggles a PTT burst on and off exactly like
//! a latch tap of the PTT button (§6.5).

use mesh_api::{PttStyle, TalkMode, UiCommand};
use mesh_core::{Platform, rt_flags};
use mesh_wire::mesh::TalkState;

use super::world::{OwnedPayload, Source, World};

fn world() -> World {
    let mut w = World::new(2, Platform::WebSim, 41);
    w.start_all();
    w.connect(0, 1);
    w.run_ms(500);
    w.go_live(0, TalkMode::Ptt);
    w.go_live(1, TalkMode::Ptt);
    w.nodes[0].source = Source::tone(800.0, 5_000.0, 0, 60_000);
    w.run_until_ms(1_000);
    w
}

type Trace = Vec<(u64, u16, OwnedPayload)>;

fn trace(w: &World) -> Trace {
    w.sent_payloads(0)
        .into_iter()
        .map(|(t, h, p)| (t, h.seq, p))
        .collect()
}

#[test]
fn rt_latch_bit_acts_like_a_latch_tap() {
    // Lock-screen latch taps at 1.0 s and 2.5 s: the runtime delivers bit3 as an edge,
    // set for one engine tick and then consumed (mesh_rt.h).
    let mut a = world();
    for at in [1_000, 2_500] {
        a.run_until_ms(at);
        a.set_flags(0, rt_flags::LATCH_TOGGLE);
        a.set_flags(0, 0);
    }
    a.run_until_ms(3_000);
    let ta = trace(&a);

    // In-app latch: two taps of a latch-style PTT button.
    let mut b = world();
    assert!(
        b.command(
            0,
            UiCommand::SetPttStyle {
                style: PttStyle::Latch
            }
        )
        .is_ok()
    );
    for at in [1_000, 2_500] {
        b.run_until_ms(at);
        assert!(b.command(0, UiCommand::Ptt { down: true }).is_ok());
        assert!(b.command(0, UiCommand::Ptt { down: false }).is_ok());
    }
    b.run_until_ms(3_000);
    let tb = trace(&b);

    assert_eq!(ta, tb, "same packets at the same instants");
    let talks: Vec<(u64, TalkState)> = ta
        .iter()
        .filter_map(|(t, _, p)| match p {
            OwnedPayload::Talk(k) => Some((*t, k.state)),
            _ => None,
        })
        .collect();
    assert_eq!(talks.first().map(|x| x.1), Some(TalkState::Start));
    assert!(talks[0].0.abs_diff(1_000_000) <= 1_000);
    let stops: Vec<_> = talks.iter().filter(|x| x.1 == TalkState::Stop).collect();
    assert_eq!(stops.len(), 2, "latch off ends the burst like PTT up");
    assert!(stops[0].0.abs_diff(2_500_000) <= 1_000);
    let voices = ta
        .iter()
        .filter(|(_, _, p)| matches!(p, OwnedPayload::Voice { .. }))
        .count();
    assert!(
        (36..=40).contains(&voices),
        "1.5 s of 40 ms frames: {voices}"
    );
}

#[test]
fn rt_latch_taps_in_consecutive_ticks_are_two_taps() {
    // Two lock-screen taps 5 ms apart (one engine tick): the runtime delivers bit3 in two
    // consecutive ticks with the same flag value; the engine must not merge them.
    let mut a = world();
    for at in [1_000, 1_005] {
        a.run_until_ms(at);
        a.set_flags(0, rt_flags::LATCH_TOGGLE);
    }
    a.run_until_ms(1_010);
    a.set_flags(0, 0);
    a.run_until_ms(1_500);
    let ta = trace(&a);

    // The same two latch taps from the in-app button.
    let mut b = world();
    assert!(
        b.command(
            0,
            UiCommand::SetPttStyle {
                style: PttStyle::Latch
            }
        )
        .is_ok()
    );
    for at in [1_000, 1_005] {
        b.run_until_ms(at);
        assert!(b.command(0, UiCommand::Ptt { down: true }).is_ok());
        assert!(b.command(0, UiCommand::Ptt { down: false }).is_ok());
    }
    b.run_until_ms(1_500);
    let tb = trace(&b);

    assert_eq!(ta, tb, "same packets at the same instants");
    let stops = ta
        .iter()
        .filter(|(_, _, p)| matches!(p, OwnedPayload::Talk(k) if k.state == TalkState::Stop))
        .count();
    assert!(stops >= 1, "the second tap ends the burst");
    let voices = ta
        .iter()
        .filter(|(_, _, p)| matches!(p, OwnedPayload::Voice { .. }))
        .count();
    assert!(voices <= 2, "no 0.5 s burst after the second tap: {voices}");
}
