//! Golden-fixture contract tests (DESIGN §3.7, WP-03 acceptance). The same fixture files
//! are parsed by the TS guards in `packages/engine-api/test/`.

mod common;

use std::collections::BTreeSet;

use common::*;
use mesh_api::*;
use serde_json::Value;

#[test]
fn every_command_fixture_round_trips_and_matches_its_gesture_flag() {
    for entry in entries(COMMANDS) {
        let name = name(&entry);
        let json = &entry["command"];
        let cmd: UiCommand = serde_json::from_value(json.clone())
            .unwrap_or_else(|e| panic!("{name}: does not deserialize: {e}"));
        let back = serde_json::to_value(&cmd).expect("serializes");
        assert_same_json(name, json, &back);
        assert_eq!(
            json["type"].as_str(),
            Some(cmd.command_type().as_str()),
            "{name}: type discriminant"
        );
        let expected = entry["requiresGesture"]
            .as_bool()
            .expect("requiresGesture flag");
        assert_eq!(cmd.requires_gesture(), expected, "{name}: requires_gesture");
        assert_eq!(gesture_by_table(json), expected, "{name}: GESTURE_COMMANDS");
    }
}

#[test]
fn every_command_type_has_a_fixture() {
    let seen: BTreeSet<String> = entries(COMMANDS)
        .iter()
        .map(|e| e["command"]["type"].as_str().expect("type").to_owned())
        .collect();
    for ty in CommandType::ALL {
        assert!(seen.contains(ty.as_str()), "no fixture for command `{ty}`");
    }
}

#[test]
fn every_event_fixture_round_trips_and_every_event_type_has_one() {
    let mut seen = BTreeSet::new();
    for entry in entries(EVENTS) {
        let name = name(&entry);
        let json = &entry["event"];
        let ev: UiEvent = serde_json::from_value(json.clone())
            .unwrap_or_else(|e| panic!("{name}: does not deserialize: {e}"));
        assert_same_json(name, json, &serde_json::to_value(&ev).expect("serializes"));
        assert_eq!(
            json["type"].as_str(),
            Some(ev.event_type().as_str()),
            "{name}"
        );
        seen.insert(ev.event_type());
    }
    for ty in EventType::ALL {
        assert!(seen.contains(ty), "no fixture for event `{ty}`");
    }
}

#[test]
fn every_result_fixture_round_trips_with_the_kind_of_its_command() {
    let mut kinds = BTreeSet::new();
    let mut saw_error = false;
    for entry in entries(RESULTS) {
        let name = name(&entry);
        let json = &entry["result"];
        let res: CommandResult = serde_json::from_value(json.clone())
            .unwrap_or_else(|e| panic!("{name}: does not deserialize: {e}"));
        assert_same_json(name, json, &serde_json::to_value(&res).expect("serializes"));
        let for_type: CommandType = entry["for"]
            .as_str()
            .expect("for")
            .parse()
            .unwrap_or_else(|e| panic!("{name}: {e}"));
        assert_eq!(res.is_ok(), json["ok"] == Value::Bool(true), "{name}: ok");
        match res.kind() {
            Some(kind) => {
                assert_eq!(kind, for_type.result_kind(), "{name}: payload kind");
                kinds.insert(kind);
            }
            None => {
                saw_error = true;
                assert!(res.error_code().is_some(), "{name}: error code");
            }
        }
    }
    assert!(saw_error, "no error result fixture");
    for kind in ResultKind::ALL {
        assert!(kinds.contains(kind), "no fixture for result kind `{kind}`");
    }
}

#[test]
fn invalid_fixtures_are_rejected() {
    let invalid = load(INVALID);
    let section = |key: &str| invalid[key].as_array().expect(key).clone();
    for entry in section("commands") {
        let res = serde_json::from_value::<UiCommand>(entry["json"].clone());
        assert!(res.is_err(), "{}: accepted {:?}", name(&entry), res);
    }
    for entry in section("events") {
        let res = serde_json::from_value::<UiEvent>(entry["json"].clone());
        assert!(res.is_err(), "{}: accepted {:?}", name(&entry), res);
    }
    for entry in section("results") {
        let res = serde_json::from_value::<CommandResult>(entry["json"].clone());
        assert!(res.is_err(), "{}: accepted {:?}", name(&entry), res);
    }
}

#[test]
fn lab_params_without_audio_or_anchor_fail_to_deserialize() {
    let ok = r#"{"audio":"off","anchor":"none","gps":true}"#;
    let params: LabParams = serde_json::from_str(ok).expect("mandatory fields only");
    assert_eq!(params, LabParams::new(LabAudio::Off, LabAnchor::None, true));
    for bad in [
        r#"{"anchor":"audio","gps":false}"#,
        r#"{"audio":"full","gps":false}"#,
        r#"{"audio":"full","anchor":"audio"}"#,
        r#"{"audio":null,"anchor":"audio","gps":false}"#,
        r#"{}"#,
    ] {
        assert!(serde_json::from_str::<LabParams>(bad).is_err(), "{bad}");
        // The same params inside a full labStart command are rejected too.
        let cmd = format!(r#"{{"type":"labStart","bench":"T0","params":{bad}}}"#);
        assert!(serde_json::from_str::<UiCommand>(&cmd).is_err(), "{cmd}");
    }
    let missing = serde_json::from_str::<LabParams>(r#"{"anchor":"audio","gps":false}"#)
        .expect_err("audio is mandatory");
    assert!(missing.to_string().contains("`audio`"), "{missing}");
    let missing = serde_json::from_str::<LabParams>(r#"{"audio":"full","gps":false}"#)
        .expect_err("anchor is mandatory");
    assert!(missing.to_string().contains("`anchor`"), "{missing}");
}

#[test]
fn gesture_rules_agree_with_requires_gesture_on_every_variant_family() {
    let mut cases = vec![
        UiCommand::GoLive {},
        UiCommand::ResumeMic {},
        UiCommand::AudioCheck(AudioCheckOp::Stop {}),
        UiCommand::LabStop {},
        UiCommand::Ptt { down: true },
    ];
    for mode in AudioCheckMode::ALL {
        cases.push(UiCommand::AudioCheck(AudioCheckOp::Start { mode: *mode }));
    }
    for audio in LabAudio::ALL {
        for anchor in LabAnchor::ALL {
            for bench in LabBench::ALL {
                cases.push(UiCommand::LabStart {
                    bench: *bench,
                    params: LabParams::new(*audio, *anchor, false),
                });
            }
        }
    }
    for cmd in cases {
        let json = serde_json::to_value(&cmd).expect("serializes");
        assert_eq!(gesture_by_table(&json), cmd.requires_gesture(), "{json}");
    }
}
