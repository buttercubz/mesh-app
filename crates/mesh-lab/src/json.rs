//! A minimal, dependency-free JSON value and writer for `labrun/1` lines.
//!
//! mesh-lab depends only on mesh-wire (DESIGN §3.1), so records are serialized by hand.
//! Objects keep insertion order (a `Vec`, no hashing), numbers that are not finite are
//! written as `null`, and floats are rounded by the caller with [`round`].

use core::fmt::Write as _;

/// A JSON value.
#[derive(Debug, Clone, PartialEq)]
pub enum Json {
    Null,
    Bool(bool),
    Int(i64),
    Num(f64),
    Str(String),
    Arr(Vec<Json>),
    Obj(Vec<(String, Json)>),
}

impl Json {
    /// An empty object.
    pub fn obj() -> Self {
        Json::Obj(Vec::new())
    }

    /// Appends `key: value` to an object (builder style); a no-op on non-objects.
    pub fn with(mut self, key: &str, value: impl Into<Json>) -> Self {
        self.set(key, value);
        self
    }

    /// Appends `key: value` to an object; a no-op on non-objects.
    pub fn set(&mut self, key: &str, value: impl Into<Json>) {
        if let Json::Obj(fields) = self {
            fields.push((key.to_string(), value.into()));
        }
    }

    /// Serializes to compact JSON.
    pub fn to_json_string(&self) -> String {
        let mut s = String::new();
        self.write(&mut s);
        s
    }

    /// Writes compact JSON into `out`.
    pub fn write(&self, out: &mut String) {
        match self {
            Json::Null => out.push_str("null"),
            Json::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
            Json::Int(i) => {
                let _ = write!(out, "{i}");
            }
            Json::Num(v) => write_f64(out, *v),
            Json::Str(s) => write_str(out, s),
            Json::Arr(items) => {
                out.push('[');
                for (i, item) in items.iter().enumerate() {
                    if i > 0 {
                        out.push(',');
                    }
                    item.write(out);
                }
                out.push(']');
            }
            Json::Obj(fields) => {
                out.push('{');
                for (i, (k, v)) in fields.iter().enumerate() {
                    if i > 0 {
                        out.push(',');
                    }
                    write_str(out, k);
                    out.push(':');
                    v.write(out);
                }
                out.push('}');
            }
        }
    }
}

fn write_f64(out: &mut String, v: f64) {
    if !v.is_finite() {
        out.push_str("null");
    } else if v == 0.0 {
        // Never write "-0".
        out.push('0');
    } else {
        // Rust's Display for f64 is the shortest round-trip decimal and never uses an
        // exponent, which is always valid JSON.
        let _ = write!(out, "{v}");
    }
}

fn write_str(out: &mut String, s: &str) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => {
                let _ = write!(out, "\\u{:04x}", c as u32);
            }
            c => out.push(c),
        }
    }
    out.push('"');
}

/// Rounds `v` to `decimals` decimal places (non-finite values pass through).
pub fn round(v: f64, decimals: u32) -> f64 {
    if !v.is_finite() {
        return v;
    }
    let f = 10f64.powi(decimals as i32);
    (v * f).round() / f
}

impl From<bool> for Json {
    fn from(v: bool) -> Self {
        Json::Bool(v)
    }
}
impl From<f64> for Json {
    fn from(v: f64) -> Self {
        Json::Num(v)
    }
}
impl From<&str> for Json {
    fn from(v: &str) -> Self {
        Json::Str(v.to_string())
    }
}
impl From<String> for Json {
    fn from(v: String) -> Self {
        Json::Str(v)
    }
}
impl From<Vec<Json>> for Json {
    fn from(v: Vec<Json>) -> Self {
        Json::Arr(v)
    }
}

macro_rules! from_int {
    ($($t:ty),*) => {$(
        impl From<$t> for Json {
            fn from(v: $t) -> Self {
                Json::Int(i64::from(v))
            }
        }
    )*};
}
from_int!(u8, u16, u32, i8, i16, i32, i64);

impl From<u64> for Json {
    fn from(v: u64) -> Self {
        Json::Int(i64::try_from(v).unwrap_or(i64::MAX))
    }
}

impl From<usize> for Json {
    fn from(v: usize) -> Self {
        Json::Int(i64::try_from(v).unwrap_or(i64::MAX))
    }
}

impl<T: Into<Json>> From<Option<T>> for Json {
    fn from(v: Option<T>) -> Self {
        match v {
            Some(v) => v.into(),
            None => Json::Null,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_compact_json() {
        let j = Json::obj()
            .with("t", "run")
            .with("v", 1u8)
            .with("x", 0.004)
            .with("neg0", -0.0)
            .with("nan", f64::NAN)
            .with("none", Option::<u8>::None)
            .with("arr", vec![Json::Int(1), Json::Bool(false)])
            .with("s", "a\"b\\c\n\u{1}é");
        assert_eq!(
            j.to_json_string(),
            r#"{"t":"run","v":1,"x":0.004,"neg0":0,"nan":null,"none":null,"arr":[1,false],"s":"a\"b\\c\n\u0001é"}"#
        );
        let parsed: serde_json::Value = serde_json::from_str(&j.to_json_string()).unwrap();
        assert_eq!(parsed["s"], "a\"b\\c\n\u{1}é");
    }

    #[test]
    fn rounding_and_large_numbers() {
        assert_eq!(round(1.23456, 3), 1.235);
        assert_eq!(Json::Num(1.0e21).to_json_string(), "1000000000000000000000");
        assert_eq!(Json::Num(1.0e-7).to_json_string(), "0.0000001");
        assert_eq!(Json::from(u64::MAX).to_json_string(), i64::MAX.to_string());
    }
}
