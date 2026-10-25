//! UniFFI surface: free functions, config validation, Idle commands, start errors.

use super::*;
use crate::api::types::{FfiError, FfiRuntimeStatus, FfiSessionStart};
use crate::api::{api_version, ble_constants, protocol_version, rt_abi_version};

#[test]
fn free_functions_export_the_contract_constants() {
    let c = ble_constants();
    assert_eq!(c.service_uuid, "31B8CD85-9D3A-4622-A465-3CDCA2C32DC5");
    assert_eq!(c.hello_uuid, "86841842-A7C3-4844-84D2-5B281A3F616C");
    assert_eq!(c.hello_len, 16);
    assert_eq!(c.max_stream_frame, 1024);
    assert_eq!(c.max_datagram_frame, 1200);
    assert_eq!((c.ble_links_target, c.ble_links_max), (2, 3));
    assert_eq!(protocol_version(), 1);
    assert_eq!(api_version(), 0x0102);
    assert_eq!(rt_abi_version(), 1);
}

#[test]
fn constructor_validates_the_seed_and_spawns_nothing() {
    let hooks: Arc<dyn PlatformHooks> = FakeHooks::new();
    let mut cfg = ffi_config(1);
    cfg.rng_seed = vec![1; 16];
    assert!(matches!(
        MeshEngine::new(cfg, hooks.clone()),
        Err(FfiError::InvalidConfig { .. })
    ));
    let e = MeshEngine::new(ffi_config(1), hooks).unwrap();
    assert_eq!(e.status(), FfiRuntimeStatus::Idle);
    assert!(
        e.runtime().shared().generation.load(SeqCst) == 0,
        "no engine built yet"
    );
}

#[test]
fn idle_commands_are_answered_with_command_results() {
    let r = Rig::new(2);
    let bad = r.cmd("{not json");
    assert_eq!(bad["ok"], false);
    assert_eq!(bad["code"], "invalidCommand");
    let unknown = r.cmd(r#"{"type":"noSuchCommand"}"#);
    assert_eq!(unknown["code"], "invalidCommand");
    // Lab run listing works without a session (empty index).
    let runs = r.cmd(r#"{"type":"labListRuns"}"#);
    assert_eq!(runs["ok"], true);
    assert_eq!(runs["runs"], serde_json::json!([]));
    // A command that needs a session is refused in Idle.
    let ptt = r.cmd(r#"{"type":"ptt","down":true}"#);
    assert_eq!(ptt["ok"], false);
    assert_eq!(r.engine.status(), FfiRuntimeStatus::Idle);
}

#[test]
fn snapshots_flow_only_while_the_ui_is_attached() {
    let r = Rig::new(3);
    r.engine.set_ui_attached(true);
    assert_eq!(r.cmd(r#"{"type":"requestSnapshot"}"#)["ok"], true);
    wait_until("snapshot event", || {
        r.hooks
            .ui_events()
            .iter()
            .any(|e| e["type"] == "snapshot" && e["s"]["engine"] == "idle")
    });
    r.engine.set_ui_attached(false);
    let before = r.hooks.ui_events().len();
    r.cmd(r#"{"type":"requestSnapshot"}"#);
    r.cmd(r#"{"type":"labListRuns"}"#);
    assert_eq!(r.hooks.ui_events().len(), before);
}

#[test]
fn start_validates_parameters() {
    let r = Rig::new(4);
    assert!(matches!(
        r.engine.start(FfiSessionStart::Group {
            group_id: vec![1; 3]
        }),
        Err(FfiError::InvalidStart { .. })
    ));
    assert_eq!(
        r.engine.start(FfiSessionStart::Group {
            group_id: vec![1; 16]
        }),
        Err(FfiError::UnknownGroup)
    );
    assert!(matches!(
        r.engine.start(FfiSessionStart::Lab { lab_code: 10_000 }),
        Err(FfiError::InvalidStart { .. })
    ));
    assert_eq!(r.engine.status(), FfiRuntimeStatus::Idle);
}

#[test]
fn radio_events_are_dropped_without_a_session() {
    let r = Rig::new(5);
    r.engine.on_link_data(9, vec![1, 2, 3]);
    r.engine.on_l2cap_published(0x81);
    // Nothing was spawned for them.
    assert_eq!(r.engine.runtime().shared().generation.load(SeqCst), 0);
}

#[test]
fn unknown_prompt_keys_are_logged_and_dropped() {
    let r = Rig::new(6);
    r.engine.on_prompt_pcm("noSuchPrompt".into(), vec![0; 10]);
    assert_eq!(
        r.hooks
            .count(|c| matches!(c, Call::Log(FfiLogLevel::Warn, m) if m.contains("noSuchPrompt"))),
        1
    );
    r.engine.on_prompt_pcm("alone".into(), vec![0; 10]);
    wait_until("prompt event", || {
        r.probe
            .events
            .lock()
            .unwrap()
            .iter()
            .any(|e| e == "prompt:alone:10")
    });
}

#[test]
fn persisted_groups_reach_the_engine() {
    let hooks = FakeHooks::new();
    let mut cfg = ffi_config(8);
    cfg.persisted.push(crate::api::types::FfiPersistedEntry {
        key: format!("group/{}", "ab".repeat(16)),
        value: vec![1],
    });
    let e = MeshEngine::new(cfg, hooks).unwrap();
    // Known group: the core accepts it (the group module of WP-10B owns the blob).
    let r = e.start(FfiSessionStart::Group {
        group_id: vec![0xab; 16],
    });
    assert_ne!(r, Err(FfiError::UnknownGroup));
    e.stop();
    assert_eq!(e.status(), FfiRuntimeStatus::Idle);
}
