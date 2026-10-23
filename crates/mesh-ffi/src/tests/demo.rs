//! The Demo ride through the runtime (DESIGN §3.4): `start(Demo)` attaches the mesh-sim
//! world, no radio command or link send reaches the platform, the `demo.*` lines go
//! through `render_prompts` / `on_prompt_pcm`, and the bots become reachable. Also the
//! ordered-shutdown window: link events are still accepted while links drain.

use super::*;
use crate::api::types::{FfiRuntimeStatus, FfiSessionStart};
use crate::runtime::inbound::Inbound;

const SET_PROMPTS: &str = r#"{"type":"setPrompts","lang":"en","enabled":true,"templates":{
    "demo.line1":"One","demo.line2":"Two","demo.line3":"Three","demo.line4":"Four",
    "demo.line5":"Five","demo.line6":"Six","demo.line7":"Seven","demo.line8":"Eight"}}"#;

fn demo_keys(calls: &[Call]) -> Vec<String> {
    calls
        .iter()
        .filter_map(|c| match c {
            Call::Prompts(reqs) => Some(reqs.iter().map(|r| r.key.clone()).collect::<Vec<_>>()),
            _ => None,
        })
        .flatten()
        .filter(|k| k.starts_with("demo."))
        .collect()
}

fn reachable(r: &Rig) -> u64 {
    r.hooks
        .ui_events()
        .iter()
        .filter(|e| e["type"] == "snapshot")
        .filter_map(|e| e["s"]["mesh"]["reachable"].as_u64())
        .max()
        .unwrap_or(0)
}

#[test]
fn demo_ride_runs_on_virtual_links_without_radio() {
    let r = Rig::new(40);
    r.engine.set_ui_attached(true);
    let ok = r.cmd(SET_PROMPTS);
    assert_eq!(ok["ok"], true, "{ok}");

    r.engine.start(FfiSessionStart::Demo).expect("demo starts");
    assert_eq!(r.engine.status(), FfiRuntimeStatus::Running);

    // The core asked platform TTS for the eight bot lines.
    wait_until("demo render requests", || {
        demo_keys(&r.hooks.calls()).len() == 8
    });
    for key in demo_keys(&r.hooks.calls()) {
        let tone: Vec<i16> = (0..16_000)
            .map(|i| ((i as f32 * 0.17).sin() * 8_000.0) as i16)
            .collect();
        r.engine.on_prompt_pcm(key, tone);
    }

    // The bots join over virtual links: the user's mesh sees them.
    wait_until("bots reachable", || {
        let _ = r.cmd(r#"{"type":"requestSnapshot"}"#);
        reachable(&r) >= 1
    });

    r.engine.stop();
    assert_eq!(r.engine.status(), FfiRuntimeStatus::Idle);
    let calls = r.hooks.calls();
    assert_eq!(
        calls.iter().filter(|c| matches!(c, Call::Radio(_))).count(),
        0,
        "no radio command reaches the platform in the Demo"
    );
    assert_eq!(
        calls.iter().filter(|c| matches!(c, Call::Send(..))).count(),
        0,
        "every unit stays on the virtual links"
    );

    // A second Demo after the first one ended works the same way.
    r.engine
        .start(FfiSessionStart::Demo)
        .expect("demo restarts");
    r.engine.stop();
    assert_eq!(r.engine.status(), FfiRuntimeStatus::Idle);
}

#[test]
fn a_demo_cannot_start_over_a_running_session() {
    let r = Rig::new(41);
    r.engine
        .start(FfiSessionStart::Lab { lab_code: 7 })
        .expect("lab starts");
    assert!(r.engine.start(FfiSessionStart::Demo).is_err());
    assert_eq!(r.engine.status(), FfiRuntimeStatus::Running);
    r.engine.stop();
}

#[test]
fn link_events_are_accepted_while_links_drain_after_stop() {
    let r = Rig::new(42);
    r.engine
        .start(FfiSessionStart::Lab { lab_code: 8 })
        .expect("lab starts");
    r.engine.stop();
    // No platform link was open: the drain window closed at once.
    assert!(!r.engine.runtime().shared().draining.load(SeqCst));
    r.engine.on_link_writable(3);
    std::thread::sleep(Duration::from_millis(50));
    assert!(
        !r.probe
            .events
            .lock()
            .unwrap()
            .iter()
            .any(|e| e == "writable:3"),
        "dropped once nothing drains"
    );

    // While links drain (window open), the same event reaches the core.
    r.engine.runtime().shared().draining.store(true, SeqCst);
    r.engine.runtime().post(Inbound::Writable {
        link: mesh_core::LinkId(4),
    });
    wait_until("writable delivered while draining", || {
        r.probe
            .events
            .lock()
            .unwrap()
            .iter()
            .any(|e| e == "writable:4")
    });
}
