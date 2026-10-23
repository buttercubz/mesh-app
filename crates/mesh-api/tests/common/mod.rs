//! Helpers shared by the contract tests: fixture loading, JS-number-aware JSON
//! comparison and an independent evaluator of the exported `GESTURE_COMMANDS` rules.

#![allow(dead_code)]

use mesh_api::{GESTURE_COMMANDS, GestureOp};
use serde_json::Value;

/// `fixtures/commands.json`.
pub const COMMANDS: &str = include_str!("../../fixtures/commands.json");
/// `fixtures/events.json`.
pub const EVENTS: &str = include_str!("../../fixtures/events.json");
/// `fixtures/results.json`.
pub const RESULTS: &str = include_str!("../../fixtures/results.json");
/// `fixtures/invalid.json`.
pub const INVALID: &str = include_str!("../../fixtures/invalid.json");

/// Parses a fixture file.
pub fn load(text: &str) -> Value {
    serde_json::from_str(text).expect("fixture file is valid JSON")
}

/// The array of fixture entries in `text` (top-level array).
pub fn entries(text: &str) -> Vec<Value> {
    match load(text) {
        Value::Array(items) => items,
        other => panic!("fixture is not an array: {other}"),
    }
}

/// The `name` of a fixture entry.
pub fn name(entry: &Value) -> &str {
    entry["name"].as_str().expect("fixture entry has a name")
}

/// Rewrites every JSON number as an `f64`, so `12` and `12.0` compare equal exactly as
/// they do in JavaScript (Rust `f64` fields print integral values with `.0`).
pub fn js_numbers(v: &Value) -> Value {
    match v {
        Value::Number(n) => {
            let f = n.as_f64().expect("finite JSON number");
            serde_json::Number::from_f64(f).map_or(Value::Null, Value::Number)
        }
        Value::Array(items) => Value::Array(items.iter().map(js_numbers).collect()),
        Value::Object(map) => Value::Object(
            map.iter()
                .map(|(k, v)| (k.clone(), js_numbers(v)))
                .collect(),
        ),
        other => other.clone(),
    }
}

/// Asserts `actual` equals `expected` modulo JS number representation.
pub fn assert_same_json(what: &str, expected: &Value, actual: &Value) {
    assert_eq!(
        js_numbers(expected),
        js_numbers(actual),
        "{what}: JSON changed in the round trip"
    );
}

/// Follows `path` from the command root.
fn field<'a>(root: &'a Value, path: &[&str]) -> Option<&'a Value> {
    path.iter().try_fold(root, |v, key| v.get(*key))
}

/// Evaluates the exported `GESTURE_COMMANDS` table on raw command JSON, the same way the
/// TS client does (independent of `UiCommand::requires_gesture`).
pub fn gesture_by_table(command: &Value) -> bool {
    let Some(ty) = command["type"].as_str() else {
        return false;
    };
    GESTURE_COMMANDS
        .iter()
        .filter(|rule| rule.command.as_str() == ty)
        .any(|rule| match rule.when {
            None => true,
            Some(cond) => match (field(command, cond.path), cond.op) {
                (Some(Value::String(s)), GestureOp::Equals) => s == cond.value,
                (Some(Value::String(s)), GestureOp::NotEquals) => s != cond.value,
                _ => false,
            },
        })
}
