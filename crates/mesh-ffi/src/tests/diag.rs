//! The ridelog/1 recorder through the runtime: platform notes before a session, one
//! session per start with its ride key, 1 Hz samples with RT counters, batching to
//! `diag_write`, the RAM ring, marks, refusals, the enable switch and the crash path.

use super::*;
use crate::api::types::FfiSessionStart;
use crate::rt::RtShared;
use mesh_lab::ridelog::{DiagEvent, DiagRecord};

fn parsed(lines: &[String]) -> Vec<serde_json::Value> {
    lines
        .iter()
        .map(|l| serde_json::from_str(l).expect("JSON line"))
        .collect()
}

fn has_ev(h: &FakeHooks, ev: &str) -> bool {
    parsed(&h.diag_lines())
        .iter()
        .any(|v| v["t"] == "e" && v["ev"] == ev)
}

#[test]
fn platform_notes_open_an_idle_session_and_are_validated() {
    let r = Rig::new(60);
    let e = &r.engine;
    assert!(
        e.diag_note(
            "plat".into(),
            "perm".into(),
            r#"{"name":"microphone","state":"granted","via":"check","api":"checkSelfPermission"}"#
                .into(),
        )
    );
    // Refusals: core is not a note source, ids are camelCase, no nested objects.
    assert!(!e.diag_note("core".into(), "x".into(), "{}".into()));
    assert!(!e.diag_note("plat".into(), "Bad".into(), "{}".into()));
    assert!(!e.diag_note("ui".into(), "cmd".into(), r#"{"o":{"a":1}}"#.into()));
    assert!(!e.diag_note("ui".into(), "cmd".into(), "[1]".into()));
    assert!(e.diag_mark("crackle".into()));
    assert!(!e.diag_mark("free text".into()));
    e.diag_flush();
    let lines = parsed(&r.hooks.diag_lines());
    assert_eq!(lines[0]["t"], "hdr");
    assert_eq!(lines[0]["schema"], "ridelog/1");
    assert_eq!(lines[0]["device"]["model"], "iPhone15,2");
    assert_eq!(lines[0]["device"]["platform"], "ios");
    let perm = lines.iter().find(|v| v["ev"] == "perm").unwrap();
    assert_eq!(perm["src"], "plat");
    assert_eq!(perm["api"], "checkSelfPermission");
    assert!(
        lines
            .iter()
            .any(|v| v["t"] == "mark" && v["label"] == "crackle")
    );
    let st = e.diag_status();
    assert!(st.enabled);
    assert!(st.session_id.unwrap().starts_with("d-"));
    // The RAM ring is a self-contained file.
    let ring = e.diag_snapshot_ring();
    assert!(ring[0].starts_with("{\"t\":\"hdr\""));
    assert_eq!(ring.len(), lines.len());
}

#[test]
fn a_session_gets_its_ride_key_samples_and_end_line() {
    let r = Rig::new(61);
    r.engine.set_ui_attached(true);
    r.engine
        .start(FfiSessionStart::Lab { lab_code: 4321 })
        .unwrap();
    let key = format!("lab-{:04x}", mesh_wire::frame::lab_tag(4321));
    wait_until("engineStart written", || has_ev(&r.hooks, "engineStart"));
    assert_eq!(
        r.engine.diag_status().ride_key.as_deref(),
        Some(key.as_str())
    );
    // RT callbacks of two sizes inside one second: the sample reports the window.
    let mut out = [0i16; 480];
    r.rt.render_rt(&mut out[..192], 1);
    r.rt.render_rt(&mut out, 2);
    wait_until("two samples", || {
        parsed(&r.hooks.diag_lines())
            .iter()
            .filter(|v| v["t"] == "s")
            .count()
            >= 2
    });
    let lines = parsed(&r.hooks.diag_lines());
    let s = lines
        .iter()
        .filter(|v| v["t"] == "s")
        .find(|v| !v["audio"]["cbRenFrames"].is_null());
    let s = s.expect("a sample with render callbacks");
    assert_eq!(s["audio"]["cbRenFrames"], serde_json::json!([192, 480]));
    assert_eq!(s["audio"]["running"], true);
    assert!(s["cpuMs"]["engine"].as_f64().unwrap() >= 0.0);
    assert!(
        lines
            .iter()
            .any(|v| v["ev"] == "glitch" && v["kind"] == "cbSizeChange")
    );
    // A core record (what mesh-core's Effect::Diag will carry) lands in the same file.
    r.engine
        .runtime()
        .shared()
        .diag
        .record_now(DiagRecord::Event(DiagEvent::MicState {
            state: "paused",
            cause: Some("builtInMicOff"),
        }));
    r.engine.stop();
    let lines = parsed(&r.hooks.diag_lines());
    let session: Vec<_> = lines
        .iter()
        .skip_while(|v| !(v["t"] == "hdr" && v["rideKey"] == key.as_str()))
        .collect();
    assert!(
        !session.is_empty(),
        "the Lab session has its own hdr with the ride key"
    );
    assert!(
        session
            .iter()
            .any(|v| v["ev"] == "micState" && v["src"] == "core")
    );
    assert!(session.iter().any(|v| v["ev"] == "engineStop"));
    assert_eq!(session.last().unwrap()["t"], "end");
    assert_eq!(session.last().unwrap()["reason"], "sessionEnd");
    // Every write is one segment of one session, batched.
    let writes = r.hooks.diag.lock().unwrap().clone();
    assert!(
        writes
            .iter()
            .all(|(id, seg, l)| id.starts_with("d-") && *seg >= 1 && !l.is_empty())
    );
}

#[test]
fn disabled_recorder_keeps_the_ring_only() {
    let r = Rig::new(62);
    r.engine.diag_configure(false, false);
    assert!(
        r.engine
            .diag_note("plat".into(), "bt".into(), r#"{"state":"on"}"#.into())
    );
    r.engine.diag_flush();
    assert!(r.hooks.diag_lines().is_empty());
    let ring = r.engine.diag_snapshot_ring();
    assert!(ring.iter().any(|l| l.contains("\"ev\":\"bt\"")));
    assert!(ring[0].contains("\"build\":\"release\""));
    assert!(!r.engine.diag_status().enabled);
}

#[test]
fn a_crash_is_recorded() {
    let r = Rig::new(63);
    r.engine
        .start(FfiSessionStart::Lab { lab_code: 12 })
        .unwrap();
    r.engine.runtime().inject_panic();
    wait_until("engineCrash written", || has_ev(&r.hooks, "engineCrash"));
    let lines = parsed(&r.hooks.diag_lines());
    let crash = lines.iter().find(|v| v["ev"] == "engineCrash").unwrap();
    assert_eq!(crash["src"], "rt");
    assert_eq!(crash["at"], "engine-thread");
}

#[test]
fn callback_window_resets_after_each_read() {
    let rt = RtShared::new();
    assert_eq!(rt.take_cb_window(), (None, None));
    let mut buf = [0i16; 480];
    rt.capture_rt(&buf[..96], 1);
    rt.capture_rt(&buf[..192], 2);
    rt.render_rt(&mut buf[..240], 3);
    assert_eq!(rt.take_cb_window(), (Some((96, 192)), Some((240, 240))));
    assert_eq!(rt.take_cb_window(), (None, None));
}
