//! "Events carry i18n keys plus params only, never English text" (DESIGN §3.7).
//!
//! Every string an event can carry is either a closed enum value (an i18n key or a
//! discriminant), an id/path, or user content (names). This test walks every event
//! fixture and fails on any string leaf that is none of those, so a new free-text field
//! (e.g. `message`) cannot slip in with a fixture.

mod common;

use std::collections::BTreeSet;

use common::*;
use mesh_api::*;
use serde_json::Value;

/// Keys whose string values are ids, file paths or user content (never engine prose).
const OPAQUE_STRING_KEYS: &[&str] = &[
    "groupId",
    "groupName",
    "name",
    "runId",
    "files", // ids, user content, paths
];

/// Param keys the engine may use in `notice`/`error` params (values are names,
/// counts or enum values; the UI formats them through the catalog).
const PARAM_KEYS: &[&str] = &[
    "name", "names", "count", "group", "islands", "percent", "labNode", "route", "minutes",
    "hours", "seconds", "limit",
];

/// Union of every closed string set of the contract.
fn enum_values() -> BTreeSet<&'static str> {
    let mut all = BTreeSet::new();
    macro_rules! add {
        ($($ty:ty),+) => { $( all.extend(<$ty>::ALL.iter().map(|v| v.as_str())); )+ };
    }
    add!(
        ErrorCode,
        NoticeKey,
        PromptKey,
        EventType,
        AudioRouteKind,
        TalkDeniedReason,
        GateState,
        AudioCheckPhase,
        EngineState,
        TalkState,
        RidePhase,
        LadderLevel,
        MemberStatus,
        LinkKind,
        VoiceReach,
        MemberFlag,
        MeshProfile,
        ThermalState,
        BluetoothState,
        MemberRole,
        PeerPlatform,
        TalkMode,
        LabVerdict
    );
    all
}

/// `is_params`: `v` is a `params` object; `param_value`: `v` is (inside) one param value.
fn walk(
    path: &str,
    key: Option<&str>,
    v: &Value,
    enums: &BTreeSet<&str>,
    is_params: bool,
    param_value: bool,
) {
    match v {
        Value::String(s) => {
            let key = key.unwrap_or("");
            let opaque =
                OPAQUE_STRING_KEYS.contains(&key) || (param_value && PARAM_KEYS.contains(&key));
            assert!(
                opaque || enums.contains(s.as_str()),
                "{path}: string `{s}` is neither an i18n key/enum value nor an id/name"
            );
        }
        Value::Array(items) => {
            for (i, item) in items.iter().enumerate() {
                walk(
                    &format!("{path}[{i}]"),
                    key,
                    item,
                    enums,
                    false,
                    param_value,
                );
            }
        }
        Value::Object(map) => {
            for (k, child) in map {
                if is_params {
                    assert!(
                        PARAM_KEYS.contains(&k.as_str()),
                        "{path}.{k}: unknown param key"
                    );
                }
                let child_path = format!("{path}.{k}");
                walk(
                    &child_path,
                    Some(k),
                    child,
                    enums,
                    k == "params",
                    is_params || param_value,
                );
            }
        }
        _ => {}
    }
}

#[test]
fn event_fixtures_contain_no_free_text() {
    let enums = enum_values();
    for entry in entries(EVENTS) {
        walk(name(&entry), None, &entry["event"], &enums, false, false);
    }
}

#[test]
fn the_walker_rejects_prose() {
    let enums = enum_values();
    let prose = serde_json::json!({"type": "notice", "key": "alone", "message": "You are alone"});
    let caught = std::panic::catch_unwind(|| walk("prose", None, &prose, &enums, false, false));
    assert!(
        caught.is_err(),
        "a `message` field with English text must fail"
    );
    let bad_param = serde_json::json!({"type": "notice", "key": "alone", "params": {"text": "hi"}});
    let caught = std::panic::catch_unwind(|| walk("param", None, &bad_param, &enums, false, false));
    assert!(caught.is_err(), "an unknown param key must fail");
}

#[test]
fn event_types_have_no_text_fields() {
    // Serialize one of every event and check the field names: none may be a prose field.
    for entry in entries(EVENTS) {
        let ev: UiEvent = serde_json::from_value(entry["event"].clone()).expect("event");
        let json = serde_json::to_string(&ev).expect("serializes");
        for forbidden in [
            "\"message\"",
            "\"text\"",
            "\"title\"",
            "\"description\"",
            "\"body\"",
        ] {
            assert!(
                !json.contains(forbidden),
                "{}: has {forbidden}",
                name(&entry)
            );
        }
    }
}
