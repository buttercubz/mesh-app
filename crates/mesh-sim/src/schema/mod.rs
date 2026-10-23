//! labrun/1 schema checking (contract C7) without a heavyweight validator: a JSON
//! Schema (draft 2020-12) interpreter for exactly the keywords the shipped schema
//! (`mesh_lab::SCHEMA_JSON`) uses. Unknown keywords make [`SchemaValidator::new`]
//! fail, so a schema change that needs more is noticed instead of silently ignored.
//! The tests cross-check this interpreter against the reference validator (`boon`).
//!
//! Kept in-crate because a general validator pulls `getrandom` (through `ahash`) into
//! the mesh-ffi and mesh-wasm graphs, which DESIGN §2.3 forbids.

pub mod pattern;

use std::collections::BTreeMap;

use serde_json::Value;

use pattern::Pattern;

/// Keywords that only annotate.
const ANNOTATIONS: &[&str] = &[
    "$schema",
    "$id",
    "$comment",
    "title",
    "description",
    "examples",
    "default",
    "$defs",
];

/// Keywords this interpreter implements.
const SUPPORTED: &[&str] = &[
    "type",
    "enum",
    "const",
    "$ref",
    "oneOf",
    "anyOf",
    "allOf",
    "not",
    "properties",
    "required",
    "additionalProperties",
    "items",
    "prefixItems",
    "minItems",
    "maxItems",
    "minimum",
    "maximum",
    "exclusiveMinimum",
    "exclusiveMaximum",
    "minLength",
    "maxLength",
    "pattern",
];

/// A compiled schema.
#[derive(Debug, Clone)]
pub struct SchemaValidator {
    root: Value,
    patterns: BTreeMap<String, Pattern>,
}

fn json_eq(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Number(x), Value::Number(y)) => x.as_f64() == y.as_f64(),
        (Value::Array(x), Value::Array(y)) => {
            x.len() == y.len() && x.iter().zip(y).all(|(p, q)| json_eq(p, q))
        }
        (Value::Object(x), Value::Object(y)) => {
            x.len() == y.len()
                && x.iter()
                    .all(|(k, v)| y.get(k).is_some_and(|w| json_eq(v, w)))
        }
        _ => a == b,
    }
}

fn is_type(v: &Value, t: &str) -> bool {
    match t {
        "null" => v.is_null(),
        "boolean" => v.is_boolean(),
        "object" => v.is_object(),
        "array" => v.is_array(),
        "string" => v.is_string(),
        "number" => v.is_number(),
        "integer" => v.as_f64().is_some_and(|f| f.fract() == 0.0) && v.is_number(),
        _ => false,
    }
}

impl SchemaValidator {
    /// Compiles a schema document.
    pub fn new(schema_json: &str) -> Result<Self, String> {
        let root: Value = serde_json::from_str(schema_json).map_err(|e| e.to_string())?;
        let mut patterns = BTreeMap::new();
        Self::precheck(&root, "#", &mut patterns)?;
        Ok(SchemaValidator { root, patterns })
    }

    /// The labrun/1 schema shipped by mesh-lab.
    pub fn labrun() -> Self {
        Self::new(mesh_lab::SCHEMA_JSON).expect("the labrun/1 schema uses supported keywords")
    }

    fn precheck(s: &Value, path: &str, pats: &mut BTreeMap<String, Pattern>) -> Result<(), String> {
        let obj = match s {
            Value::Bool(_) => return Ok(()),
            Value::Object(o) => o,
            _ => return Err(format!("{path}: a schema must be an object or a boolean")),
        };
        for (k, v) in obj {
            if !SUPPORTED.contains(&k.as_str()) && !ANNOTATIONS.contains(&k.as_str()) {
                return Err(format!("{path}: unsupported keyword {k}"));
            }
            let sub = format!("{path}/{k}");
            match k.as_str() {
                "$defs" | "properties" => {
                    for (name, s) in v.as_object().ok_or(format!("{sub}: not an object"))? {
                        Self::precheck(s, &format!("{sub}/{name}"), pats)?;
                    }
                }
                "oneOf" | "anyOf" | "allOf" | "prefixItems" => {
                    for (i, s) in v
                        .as_array()
                        .ok_or(format!("{sub}: not an array"))?
                        .iter()
                        .enumerate()
                    {
                        Self::precheck(s, &format!("{sub}/{i}"), pats)?;
                    }
                }
                "items" | "not" | "additionalProperties" => Self::precheck(v, &sub, pats)?,
                "pattern" => {
                    let src = v.as_str().ok_or(format!("{sub}: not a string"))?;
                    let p = Pattern::new(src).map_err(|e| format!("{sub}: {e}"))?;
                    pats.insert(src.to_string(), p);
                }
                "$ref" => {
                    let r = v.as_str().ok_or(format!("{sub}: not a string"))?;
                    if !r.starts_with("#/") {
                        return Err(format!("{sub}: only local refs are supported ({r})"));
                    }
                }
                _ => {}
            }
        }
        Ok(())
    }

    fn resolve(&self, r: &str) -> Result<&Value, String> {
        self.root
            .pointer(&r[1..])
            .ok_or_else(|| format!("unresolvable $ref {r}"))
    }

    /// Validates one JSON value.
    pub fn validate(&self, v: &Value) -> Result<(), String> {
        self.check(&self.root, v, "")
    }

    /// Parses and validates one JSON line.
    pub fn validate_line(&self, line: &str) -> Result<Value, String> {
        let v: Value = serde_json::from_str(line).map_err(|e| format!("not JSON: {e}"))?;
        self.validate(&v)?;
        Ok(v)
    }

    fn check(&self, s: &Value, v: &Value, at: &str) -> Result<(), String> {
        let o = match s {
            Value::Bool(true) => return Ok(()),
            Value::Bool(false) => return Err(format!("{at}: not allowed")),
            Value::Object(o) => o,
            _ => return Err(format!("{at}: bad schema")),
        };
        if let Some(r) = o.get("$ref").and_then(Value::as_str) {
            self.check(self.resolve(r)?, v, at)?;
        }
        if let Some(t) = o.get("type") {
            let ok = match t {
                Value::String(t) => is_type(v, t),
                Value::Array(ts) => ts.iter().filter_map(Value::as_str).any(|t| is_type(v, t)),
                _ => false,
            };
            if !ok {
                return Err(format!("{at}: {v} is not of type {t}"));
            }
        }
        if let Some(e) = o.get("enum").and_then(Value::as_array)
            && !e.iter().any(|x| json_eq(x, v))
        {
            return Err(format!(
                "{at}: {v} is not one of {}",
                Value::Array(e.clone())
            ));
        }
        if let Some(c) = o.get("const")
            && !json_eq(c, v)
        {
            return Err(format!("{at}: {v} is not {c}"));
        }
        self.check_combinators(o, v, at)?;
        self.check_number(o, v, at)?;
        self.check_string(o, v, at)?;
        self.check_array(o, v, at)?;
        self.check_object(o, v, at)
    }

    fn check_combinators(
        &self,
        o: &serde_json::Map<String, Value>,
        v: &Value,
        at: &str,
    ) -> Result<(), String> {
        if let Some(all) = o.get("allOf").and_then(Value::as_array) {
            for s in all {
                self.check(s, v, at)?;
            }
        }
        if let Some(any) = o.get("anyOf").and_then(Value::as_array)
            && !any.iter().any(|s| self.check(s, v, at).is_ok())
        {
            return Err(format!("{at}: matches none of anyOf"));
        }
        if let Some(one) = o.get("oneOf").and_then(Value::as_array) {
            let results: Vec<Result<(), String>> =
                one.iter().map(|s| self.check(s, v, at)).collect();
            let ok = results.iter().filter(|r| r.is_ok()).count();
            if ok != 1 {
                let why: Vec<String> = results.into_iter().filter_map(Result::err).collect();
                return Err(format!(
                    "{at}: matches {ok} of oneOf (need 1): {}",
                    why.join(" | ")
                ));
            }
        }
        if let Some(not) = o.get("not")
            && self.check(not, v, at).is_ok()
        {
            return Err(format!("{at}: matches a `not` schema"));
        }
        Ok(())
    }

    fn check_number(
        &self,
        o: &serde_json::Map<String, Value>,
        v: &Value,
        at: &str,
    ) -> Result<(), String> {
        let Some(x) = v.as_f64().filter(|_| v.is_number()) else {
            return Ok(());
        };
        let bound = |k: &str| o.get(k).and_then(Value::as_f64);
        if bound("minimum").is_some_and(|m| x < m)
            || bound("maximum").is_some_and(|m| x > m)
            || bound("exclusiveMinimum").is_some_and(|m| x <= m)
            || bound("exclusiveMaximum").is_some_and(|m| x >= m)
        {
            return Err(format!("{at}: {x} is out of range"));
        }
        Ok(())
    }

    fn check_string(
        &self,
        o: &serde_json::Map<String, Value>,
        v: &Value,
        at: &str,
    ) -> Result<(), String> {
        let Some(s) = v.as_str() else {
            return Ok(());
        };
        let n = s.chars().count() as u64;
        if o.get("minLength")
            .and_then(Value::as_u64)
            .is_some_and(|m| n < m)
            || o.get("maxLength")
                .and_then(Value::as_u64)
                .is_some_and(|m| n > m)
        {
            return Err(format!("{at}: length {n} out of range"));
        }
        if let Some(p) = o.get("pattern").and_then(Value::as_str) {
            let ok = self.patterns.get(p).is_some_and(|re| re.is_match(s));
            if !ok {
                return Err(format!("{at}: {s:?} does not match {p}"));
            }
        }
        Ok(())
    }

    fn check_array(
        &self,
        o: &serde_json::Map<String, Value>,
        v: &Value,
        at: &str,
    ) -> Result<(), String> {
        let Some(a) = v.as_array() else {
            return Ok(());
        };
        let n = a.len() as u64;
        if o.get("minItems")
            .and_then(Value::as_u64)
            .is_some_and(|m| n < m)
            || o.get("maxItems")
                .and_then(Value::as_u64)
                .is_some_and(|m| n > m)
        {
            return Err(format!("{at}: {n} items out of range"));
        }
        let prefix = o.get("prefixItems").and_then(Value::as_array);
        let np = prefix.map_or(0, Vec::len);
        if let Some(p) = prefix {
            for (i, (s, x)) in p.iter().zip(a).enumerate() {
                self.check(s, x, &format!("{at}/{i}"))?;
            }
        }
        if let Some(items) = o.get("items") {
            for (i, x) in a.iter().enumerate().skip(np) {
                self.check(items, x, &format!("{at}/{i}"))?;
            }
        }
        Ok(())
    }

    fn check_object(
        &self,
        o: &serde_json::Map<String, Value>,
        v: &Value,
        at: &str,
    ) -> Result<(), String> {
        let Some(m) = v.as_object() else {
            return Ok(());
        };
        if let Some(req) = o.get("required").and_then(Value::as_array) {
            for k in req.iter().filter_map(Value::as_str) {
                if !m.contains_key(k) {
                    return Err(format!("{at}: missing required {k}"));
                }
            }
        }
        let props = o.get("properties").and_then(Value::as_object);
        for (k, x) in m {
            match props.and_then(|p| p.get(k)) {
                Some(s) => self.check(s, x, &format!("{at}/{k}"))?,
                None => {
                    if let Some(ap) = o.get("additionalProperties") {
                        self.check(ap, x, &format!("{at}/{k}"))?;
                    }
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_shipped_schema_compiles_and_checks_the_design_example() {
        let v = SchemaValidator::labrun();
        let run = r#"{"t":"run","v":1,"runId":"r-20261023-0912-ab12","bench":"T0","variant":{"transport":"l2cap"},"audio":"full","anchor":"audio","startedAt":"2026-10-23T09:12:03.120Z","device":{"model":"iPhone15,2","os":"iOS 27.0","app":"0.1.0 (12)","platform":"ios"},"labNode":4711,"role":"controller","peers":[{"labNode":918,"model":"iPhone13,1"}],"params":{"durationS":60,"streams":4,"payloadB":118,"intervalMs":40}}"#;
        v.validate_line(run).expect("design example validates");
        let bad = run.replace("\"audio\":\"full\"", "\"audio\":\"loud\"");
        assert!(v.validate_line(&bad).is_err());
        let missing = run.replace("\"anchor\":\"audio\",", "");
        assert!(v.validate_line(&missing).is_err());
    }

    #[test]
    fn unsupported_keywords_are_refused() {
        assert!(SchemaValidator::new(r#"{"if": true}"#).is_err());
        assert!(SchemaValidator::new(r#"{"$ref": "other.json"}"#).is_err());
        let s = SchemaValidator::new(r#"{"type":"integer","minimum":1}"#).unwrap();
        assert!(s.validate(&serde_json::json!(2.0)).is_ok());
        assert!(s.validate(&serde_json::json!(2.5)).is_err());
        assert!(s.validate(&serde_json::json!(0)).is_err());
    }
}
