//! Spoken prompts (§7.7): fixed prompts and `intercomLive{n}` are rendered at Go live;
//! rendered PCM is played through the mixer after the cue's earcon; a prompt that was
//! not rendered falls back to its earcon; prompts off means no render request.

use std::collections::BTreeMap;

use mesh_api::{AudioRouteKind, Lang, PromptKey, TalkMode, UiCommand};
use mesh_core::effects::Effect;
use mesh_core::{AudioRouteInfo, Platform, PlatformEvent, RadioAvail};

use super::dsp::{first_tone, tone_amp};
use super::world::World;

fn templates() -> BTreeMap<PromptKey, String> {
    PromptKey::ALL
        .iter()
        .map(|k| (*k, format!("{} {{{{n}}}} {{{{name}}}}", k.as_str())))
        .collect()
}

fn render_requests(w: &World) -> Vec<mesh_core::PromptRequest> {
    w.nodes[0]
        .effects
        .iter()
        .filter_map(|(_, e)| match e {
            Effect::RenderPrompts(v) => Some(v.clone()),
            _ => None,
        })
        .flatten()
        .collect()
}

fn set_prompts(w: &mut World, enabled: bool) {
    let r = w.command(
        0,
        UiCommand::SetPrompts {
            lang: Lang::En,
            enabled,
            templates: templates(),
        },
    );
    assert!(r.is_ok());
}

fn tone_pcm(hz: f64, ms: usize) -> Vec<i16> {
    (0..ms * 16)
        .map(|i| (6_000.0 * (2.0 * std::f64::consts::PI * hz * i as f64 / 16_000.0).sin()) as i16)
        .collect()
}

#[test]
fn go_live_renders_fixed_prompts_and_cues_play_earcon_then_prompt() {
    let mut w = World::new(2, Platform::WebSim, 51);
    w.start_all();
    w.connect(0, 1);
    w.run_ms(300);
    set_prompts(&mut w, true);
    w.go_live(0, TalkMode::Vox);
    let reqs = render_requests(&w);
    let keys: Vec<PromptKey> = reqs.iter().map(|r| r.key).collect();
    for k in [
        PromptKey::Alone,
        PromptKey::MicPaused,
        PromptKey::BluetoothOff,
        PromptKey::RideStarted,
        PromptKey::HeadsetLost,
    ] {
        assert!(keys.contains(&k), "{k:?} rendered at go-live");
    }
    assert!(!keys.iter().any(|k| k.as_str().starts_with("demo.")));
    let live = reqs
        .iter()
        .find(|r| r.key == PromptKey::IntercomLive)
        .expect("intercomLive{n}");
    assert_eq!(
        live.text, "intercomLive 2 2",
        "n = riders reachable incl. me"
    );
    assert!(reqs.iter().all(|r| r.lang == Lang::En));

    // The platform renders Bluetooth-off; Bluetooth goes off: earcon, then the prompt.
    let pcm = tone_pcm(1_200.0, 500);
    w.event(
        0,
        PlatformEvent::PromptPcm {
            key: PromptKey::BluetoothOff,
            pcm: &pcm,
        },
    );
    w.run_ms(100);
    let t0 = w.now;
    w.event(
        0,
        PlatformEvent::RadioState {
            ble: RadioAvail::Off,
            aware: RadioAvail::Unsupported,
        },
    );
    w.run_ms(1_500);
    let ear = first_tone(&w.nodes[0].out, t0, 600.0, 1_500.0).expect("Bluetooth-off earcon");
    let spoken = first_tone(&w.nodes[0].out, t0, 1_200.0, 3_000.0).expect("spoken prompt");
    assert!(
        ear < spoken,
        "earcon first ({ear}), then the prompt ({spoken})"
    );
    assert!(spoken - ear < 600_000);
    let spoken_blocks = w.nodes[0]
        .out
        .iter()
        .filter(|(t, b)| *t >= t0 && tone_amp(b, 1_200.0) > 3_000.0)
        .count();
    assert!(
        (45..=51).contains(&spoken_blocks),
        "500 ms of prompt: {spoken_blocks}"
    );
}

#[test]
fn unrendered_prompt_falls_back_to_its_earcon_and_prompts_off_render_nothing() {
    let mut w = World::new(1, Platform::WebSim, 52);
    w.start_all();
    set_prompts(&mut w, false);
    w.go_live(0, TalkMode::Vox);
    assert!(
        render_requests(&w).is_empty(),
        "prompts off: nothing rendered"
    );
    let route = |headset| AudioRouteInfo {
        kind: if headset {
            AudioRouteKind::HfpWideband
        } else {
            AudioRouteKind::BuiltinSpeaker
        },
        input_rate: 16_000,
        output: AudioRouteKind::BuiltinSpeaker,
        headset_present: headset,
    };
    w.event(0, PlatformEvent::AudioRoute(route(true)));
    w.run_ms(200);
    let t0 = w.now;
    w.event(0, PlatformEvent::AudioRoute(route(false)));
    w.run_ms(800);
    let hit = first_tone(&w.nodes[0].out, t0, 400.0, 1_500.0).expect("headset-lost earcon");
    assert!(hit - t0 <= 20_000);
    // Prompts on but never rendered: the earcon still plays, and a render is requested.
    let mut w = World::new(1, Platform::WebSim, 53);
    w.start_all();
    set_prompts(&mut w, true);
    w.go_live(0, TalkMode::Vox);
    w.run_ms(100);
    let t0 = w.now;
    w.event(
        0,
        PlatformEvent::RadioState {
            ble: RadioAvail::Off,
            aware: RadioAvail::Unsupported,
        },
    );
    w.run_ms(800);
    assert!(first_tone(&w.nodes[0].out, t0, 600.0, 1_500.0).is_some());
    assert!(
        w.nodes[0]
            .out
            .iter()
            .filter(|(t, _)| *t >= t0)
            .all(|(_, b)| tone_amp(b, 1_200.0) < 500.0)
    );
}
