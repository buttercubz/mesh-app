//! A panicking engine thread reports `crashed` and stops cleanly instead of aborting.

use super::*;
use crate::api::types::{FfiRuntimeStatus, FfiSessionStart};

fn crash_reported(h: &FakeHooks) -> bool {
    h.ui_events()
        .iter()
        .any(|e| e["type"] == "error" && e["code"] == "engineCrashed")
        && h.count(|c| {
            matches!(c, Call::Notify(n)
                if n.category == FfiNotificationCategory::Error && n.key == "engineCrashed")
        }) == 1
}

#[test]
fn engine_thread_panic_is_reported_and_the_runtime_recovers() {
    let r = Rig::new(30);
    r.engine.set_ui_attached(true);
    r.engine
        .start(FfiSessionStart::Lab { lab_code: 77 })
        .unwrap();
    r.engine.on_link_up(
        5,
        crate::api::types::FfiLinkInfo {
            kind: crate::api::types::FfiLinkKind::BleL2cap,
            role: crate::api::types::FfiLinkRole::Initiator,
            max_frame: 1024,
            unit: crate::api::types::FfiLinkUnit::Stream,
            peer_platform_hint: None,
            att_mtu: None,
            disc_id: Some(1),
        },
    );
    wait_until("link up sent something", || {
        r.hooks.count(|c| matches!(c, Call::Send(5, _))) > 0
    });
    assert!(r.rt.is_active());

    r.engine.runtime().inject_panic();
    wait_until("crash reported", || crash_reported(&r.hooks));
    wait_until("status crashed", || {
        r.engine.status() == FfiRuntimeStatus::Crashed
    });
    // Radios and audio were stopped by the runtime itself.
    let calls = r.hooks.calls();
    assert!(calls.contains(&Call::Radio(FfiRadioCommand::Close {
        link: 5,
        reason: None
    })));
    assert!(calls.contains(&Call::Radio(FfiRadioCommand::SetDiscovery {
        mode: FfiDiscoveryMode::Off,
        adv: None,
        scan: FfiScanIntensity::LowPower,
    })));
    assert!(calls.contains(&Call::Audio(FfiAudioCommand::CloseSession)));
    wait_until("RT released", || !r.rt.is_claimed());
    // Link events are no longer accepted.
    let sends = r.hooks.count(|c| matches!(c, Call::Send(..)));
    r.engine.on_link_data(5, vec![0, 1]);

    // A fresh engine answers commands; snapshots say `crashed` until the next start.
    assert_eq!(r.cmd(r#"{"type":"labListRuns"}"#)["ok"], true);
    r.cmd(r#"{"type":"requestSnapshot"}"#);
    wait_until("crashed snapshot", || {
        r.hooks
            .ui_events()
            .iter()
            .rev()
            .find(|e| e["type"] == "snapshot")
            .is_some_and(|e| e["s"]["engine"] == "crashed")
    });
    assert_eq!(r.engine.runtime().shared().generation.load(SeqCst), 2);
    r.engine
        .start(FfiSessionStart::Lab { lab_code: 77 })
        .unwrap();
    assert_eq!(r.engine.status(), FfiRuntimeStatus::Running);
    r.engine.stop();
    assert_eq!(r.engine.status(), FfiRuntimeStatus::Idle);
    assert_eq!(
        r.hooks.count(|c| matches!(c, Call::Send(5, _))),
        sends,
        "no traffic on the dead link"
    );
}

#[test]
fn a_command_that_panics_answers_engine_crashed() {
    let r = Rig::new(31);
    r.engine.set_ui_attached(true);
    // Build the engine (its first snapshot is emitted while it is constructed), then
    // let the ≤ 10 Hz window pass so the next snapshot is built inside
    // `handle_command`, where the armed module panics.
    assert_eq!(r.cmd(r#"{"type":"labListRuns"}"#)["ok"], true);
    wait_until("first snapshot", || {
        r.hooks.ui_events().iter().any(|e| e["type"] == "snapshot")
    });
    std::thread::sleep(Duration::from_millis(150));
    r.probe.panic_armed.store(true, SeqCst);
    let out = r.cmd(r#"{"type":"requestSnapshot"}"#);
    assert_eq!(out["ok"], false);
    assert_eq!(out["code"], "engineCrashed");
    assert!(!r.probe.panic_armed.load(SeqCst));
    wait_until("crash reported", || crash_reported(&r.hooks));
    wait_until("status crashed", || {
        r.engine.status() == FfiRuntimeStatus::Crashed
    });
    // The next command is served by a new engine, whose snapshots say `crashed`.
    assert_eq!(r.cmd(r#"{"type":"labListRuns"}"#)["ok"], true);
    assert_eq!(r.cmd(r#"{"type":"requestSnapshot"}"#)["ok"], true);
    wait_until("crashed snapshot", || {
        r.hooks
            .ui_events()
            .iter()
            .rev()
            .find(|e| e["type"] == "snapshot")
            .is_some_and(|e| e["s"]["engine"] == "crashed")
    });
    assert_eq!(r.engine.runtime().shared().generation.load(SeqCst), 2);
}

#[test]
fn idle_snapshots_deferred_by_the_rate_limit_are_still_emitted() {
    let r = Rig::new(32);
    r.engine.set_ui_attached(true);
    let snapshots = || {
        r.hooks
            .ui_events()
            .iter()
            .filter(|e| e["type"] == "snapshot")
            .count()
    };
    assert_eq!(r.cmd(r#"{"type":"requestSnapshot"}"#)["ok"], true);
    wait_until("first snapshot", || snapshots() >= 1);
    let before = snapshots();
    // Inside the ≤ 10 Hz window: the engine defers it; the idle thread must wake for
    // the deadline even though nothing else arrives on the channel.
    assert_eq!(r.cmd(r#"{"type":"requestSnapshot"}"#)["ok"], true);
    wait_until("deferred snapshot", || snapshots() > before);
    assert_eq!(r.engine.status(), FfiRuntimeStatus::Idle);
}
