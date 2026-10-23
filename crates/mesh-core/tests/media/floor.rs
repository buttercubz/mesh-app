//! Floor cues at the engine level: a blocked VOX talker hears the busy tone only after
//! 600 ms of sustained speech, a blocked PTT press hears it at once (§6.4-§6.5), and
//! repeated VOX denials trigger the wind-hog protection (§6.5).

use mesh_api::{NoticeKey, TalkDeniedReason, TalkMode, UiEvent};
use mesh_core::{Platform, rt_flags};

use super::dsp::{first_tone, max_windowed_amp};
use super::world::{Source, World};

const BUSY_HZ: f64 = 480.0;
const BUSY_MIN: f64 = 2_000.0;

/// Four nodes: 0 and 1 hold the floor (PTT, admitted 400 ms apart), 2 is VOX, 3 is PTT.
fn busy_world(salt: u8) -> World {
    let mut w = World::new(4, Platform::WebSim, salt);
    w.start_all();
    w.connect_all();
    w.run_ms(500);
    w.go_live(0, TalkMode::Ptt);
    w.go_live(1, TalkMode::Ptt);
    w.go_live(2, TalkMode::Vox);
    w.go_live(3, TalkMode::Ptt);
    w.nodes[0].source = Source::tone(1_500.0, 4_000.0, 0, 60_000);
    w.nodes[1].source = Source::tone(2_200.0, 4_000.0, 0, 60_000);
    w.run_until_ms(1_000);
    w.set_flags(0, rt_flags::PTT_DOWN);
    w.run_until_ms(1_400);
    w.set_flags(1, rt_flags::PTT_DOWN);
    w.run_until_ms(1_900);
    w
}

#[test]
fn vox_busy_after_600ms_of_speech_ptt_busy_at_once() {
    let mut w = busy_world(11);
    // VOX speech at 2.0 s on the blocked node.
    w.nodes[2].source = Source::tone(400.0, 5_000.0, 2_000, 3_500);
    w.run_until_ms(3_000);
    let early = max_windowed_amp(&w.nodes[2].out, 1_900_000, 2_600_000, BUSY_HZ, 5);
    assert!(
        early < 1_000.0,
        "no busy tone before 600 ms of speech: {early}"
    );
    let busy = first_tone(&w.nodes[2].out, 2_000_000, BUSY_HZ, BUSY_MIN).expect("VOX busy tone");
    assert!(
        (2_600_000..2_700_000).contains(&busy),
        "VOX busy 600-700 ms after speech onset: {busy}"
    );
    let denied: Vec<_> = w
        .ui_events(2)
        .into_iter()
        .filter_map(|(t, e)| match e {
            UiEvent::TalkDenied { reason } => Some((t, reason)),
            _ => None,
        })
        .collect();
    assert_eq!(denied.len(), 1);
    assert!(denied[0].0 < 2_100_000, "denial raised at the onset");
    assert_eq!(denied[0].1, TalkDeniedReason::Cap);
    assert!(
        w.sent_payloads(2).is_empty(),
        "the blocked VOX node never transmits"
    );
    // PTT on a full floor: busy within the next playout block.
    let t_p = w.now;
    w.set_flags(3, rt_flags::PTT_DOWN);
    w.run_ms(300);
    let early = max_windowed_amp(&w.nodes[3].out, 1_900_000, t_p, BUSY_HZ, 5);
    assert!(early < 1_000.0, "no busy tone before the press: {early}");
    let busy = first_tone(&w.nodes[3].out, t_p, BUSY_HZ, BUSY_MIN).expect("PTT busy tone");
    assert!(busy - t_p <= 10_500, "PTT busy at once: {} µs", busy - t_p);
}

#[test]
fn repeated_vox_denials_raise_the_vox_threshold() {
    let mut w = busy_world(12);
    w.nodes[2].source = Source::Sum(vec![
        Source::tone(400.0, 5_000.0, 2_000, 2_300),
        Source::tone(400.0, 5_000.0, 5_000, 5_300),
        Source::tone(400.0, 5_000.0, 8_000, 8_300),
    ]);
    w.run_until_ms(9_000);
    let denials = w
        .ui_events(2)
        .iter()
        .filter(|(_, e)| matches!(e, UiEvent::TalkDenied { .. }))
        .count();
    assert_eq!(denials, 3);
    let raised: Vec<u64> = w
        .ui_events(2)
        .into_iter()
        .filter_map(|(t, e)| match e {
            UiEvent::Notice {
                key: NoticeKey::VoxLessSensitive,
                ..
            } => Some(t),
            _ => None,
        })
        .collect();
    assert_eq!(
        raised.len(),
        1,
        "voxLessSensitive once, at the third denial"
    );
    assert!((8_000_000..8_100_000).contains(&raised[0]), "{raised:?}");
}
