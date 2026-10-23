//! The session → media seam (`MediaRequest`): the group identity, member names rendered
//! at go-live, cues with a name (earcon first, then the spoken prompt once rendered), and
//! the Demo-ride lines rendered at Demo start but never banked (§3.4, §7.7).

use std::collections::BTreeMap;

use mesh_api::{Lang, PromptKey, UiCommand};
use mesh_core::media::{Cue, MediaIdentity, MediaRequest};
use mesh_core::{Platform, PlatformEvent, RadioAvail};
use mesh_voice::Earcon;

use super::dsp::first_tone;
use super::rig::Rig;

fn templates() -> BTreeMap<PromptKey, String> {
    PromptKey::ALL
        .iter()
        .map(|k| {
            let t = match k {
                PromptKey::MemberJoined | PromptKey::MemberLeft | PromptKey::LeaderChanged => {
                    format!("{} {{{{name}}}}", k.as_str())
                }
                PromptKey::IntercomLive | PromptKey::RidersOutOfRange => {
                    format!("{} {{{{n}}}}", k.as_str())
                }
                _ => k.as_str().to_string(),
            };
            (*k, t)
        })
        .collect()
}

fn tone_pcm(hz: f64, ms: usize) -> Vec<i16> {
    (0..ms * 16)
        .map(|i| (6_000.0 * (2.0 * std::f64::consts::PI * hz * i as f64 / 16_000.0).sin()) as i16)
        .collect()
}

fn rig() -> Rig {
    let mut r = Rig::new(Platform::WebSim, 81);
    let ok = r.command(UiCommand::SetPrompts {
        lang: Lang::Es,
        enabled: true,
        templates: templates(),
    });
    assert!(ok.is_some_and(|x| x.is_ok()));
    r.request(MediaRequest::Identity(MediaIdentity {
        node: 0x0102,
        leader: true,
        epoch: 5,
        leader_priority: true,
    }));
    r.request(MediaRequest::MemberNames(vec![
        (0x0203, "Ana".to_string()),
        (0x0304, "Bob".to_string()),
    ]));
    r
}

#[test]
fn names_known_at_go_live_are_rendered_and_a_named_cue_speaks() {
    let mut r = rig();
    r.go_live(3);
    let reqs = r.render_requests();
    let texts: Vec<(PromptKey, String)> = reqs.iter().map(|q| (q.key, q.text.clone())).collect();
    for want in [
        (PromptKey::MemberJoined, "memberJoined Ana"),
        (PromptKey::MemberJoined, "memberJoined Bob"),
        (PromptKey::MemberLeft, "memberLeft Ana"),
        (PromptKey::IntercomLive, "intercomLive 3"),
        (PromptKey::Alone, "alone"),
    ] {
        assert!(
            texts.contains(&(want.0, want.1.to_string())),
            "{want:?} in {texts:?}"
        );
    }
    assert!(reqs.iter().all(|q| q.lang == Lang::Es));
    // The platform renders the first memberJoined request (Ana's).
    let pcm = tone_pcm(1_200.0, 400);
    r.event(PlatformEvent::PromptPcm {
        key: PromptKey::MemberJoined,
        pcm: &pcm,
    });
    assert_eq!(r.media.prompt_bank_bytes(), pcm.len() * 2);
    r.run_ms(100);
    let t0 = r.now();
    r.request(MediaRequest::Cue(
        Cue::with_prompt(Some(Earcon::MemberJoined), PromptKey::MemberJoined).param("Ana"),
    ));
    r.run_ms(1_000);
    let ear = first_tone(&r.out, t0, 660.0, 1_500.0).expect("member-joined earcon");
    let spoken = first_tone(&r.out, t0, 1_200.0, 3_000.0).expect("Ana joined, spoken");
    assert!(
        ear < spoken && spoken - ear < 400_000,
        "{ear} then {spoken}"
    );
    // A name learned mid-ride is rendered on demand.
    let before = r.render_requests().len();
    r.request(MediaRequest::MemberNames(vec![
        (0x0203, "Ana".to_string()),
        (0x0304, "Bob".to_string()),
        (0x0405, "Eva".to_string()),
    ]));
    let new: Vec<String> = r.render_requests()[before..]
        .iter()
        .map(|q| q.text.clone())
        .collect();
    assert_eq!(new, ["memberJoined Eva", "memberLeft Eva"]);
}

#[test]
fn session_cues_replace_the_standalone_platform_cues() {
    // With the session module's identity, Bluetooth-off is the session's cue: the media
    // module does not derive it from the platform event as well (no double cue).
    let mut r = rig();
    r.go_live(2);
    r.run_ms(100);
    let t0 = r.now();
    r.event(PlatformEvent::RadioState {
        ble: RadioAvail::Off,
        aware: RadioAvail::Unsupported,
    });
    r.run_ms(600);
    assert!(first_tone(&r.out, t0, 600.0, 1_500.0).is_none());
    // The session's cue plays it (earcon; the prompt is not rendered yet).
    let t1 = r.now();
    r.request(MediaRequest::Cue(Cue::with_prompt(
        Some(Earcon::BluetoothOff),
        PromptKey::BluetoothOff,
    )));
    r.run_ms(600);
    assert!(first_tone(&r.out, t1, 600.0, 1_500.0).is_some());
}

#[test]
fn demo_lines_are_rendered_at_demo_start_and_never_banked() {
    let mut r = rig();
    r.request(MediaRequest::DemoStart);
    let keys: Vec<PromptKey> = r.render_requests().iter().map(|q| q.key).collect();
    assert_eq!(keys, mesh_core::media::demo::DEMO_LINES.to_vec());
    let pcm = tone_pcm(300.0, 5_000);
    r.event(PlatformEvent::PromptPcm {
        key: PromptKey::DemoLine1,
        pcm: &pcm,
    });
    assert_eq!(r.media.prompt_bank_bytes(), 0, "demo PCM is not banked");
    // The bots encode each line once at the session profile and drop the PCM.
    let lines = mesh_core::media::demo::DemoLines::encode(
        &mesh_voice::SimCodec::new(),
        mesh_voice::Profile::Ble16,
        vec![(PromptKey::DemoLine1, pcm)],
    )
    .unwrap();
    assert_eq!(
        lines.line(PromptKey::DemoLine1).unwrap().duration_ms(),
        5_000
    );
}
