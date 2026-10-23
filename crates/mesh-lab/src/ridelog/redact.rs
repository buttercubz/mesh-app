//! Redaction filter (§5.4 of the diagnostics spec): any string field longer than 64
//! characters, or containing a hex run of 24+ characters or a base64-like run of 32+
//! characters, is replaced by `"<redacted>"` and counted. It runs on every line the
//! recorder writes, typed core events included (defence in depth).

use crate::json::Json;

/// Longest string kept verbatim.
pub const MAX_STR: usize = 64;
/// Hex run length that is treated as key material.
pub const HEX_RUN: usize = 24;
/// Base64-like run length that is treated as key material or a token.
pub const B64_RUN: usize = 32;
/// The replacement text.
pub const REDACTED: &str = "<redacted>";

/// Whether `s` must be redacted.
pub fn is_sensitive(s: &str) -> bool {
    if s.chars().count() > MAX_STR {
        return true;
    }
    let (mut hex, mut b64) = (0usize, 0usize);
    for c in s.chars() {
        hex = if c.is_ascii_hexdigit() { hex + 1 } else { 0 };
        b64 = if c.is_ascii_alphanumeric() || matches!(c, '+' | '/' | '=' | '_' | '-') {
            b64 + 1
        } else {
            0
        };
        if hex >= HEX_RUN || b64 >= B64_RUN {
            return true;
        }
    }
    false
}

/// Redacts `v` in place (strings anywhere in the tree, object keys included); returns
/// the number of replaced strings.
pub fn redact(v: &mut Json) -> u32 {
    match v {
        Json::Str(s) => {
            if is_sensitive(s) {
                *s = REDACTED.to_string();
                1
            } else {
                0
            }
        }
        Json::Arr(items) => items.iter_mut().map(redact).sum(),
        Json::Obj(fields) => {
            let mut n = 0;
            for (k, val) in fields.iter_mut() {
                if is_sensitive(k) {
                    *k = REDACTED.to_string();
                    n += 1;
                }
                n += redact(val);
            }
            n
        }
        _ => 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_codes_and_redacts_secrets() {
        for ok in [
            "linkDown",
            "timeout",
            "SM-X520",
            "Android 16 (API 36)",
            "d-20261022-2004-7k2q",
            "9f3a71c2",
        ] {
            assert!(!is_sensitive(ok), "{ok}");
        }
        for bad in [
            "00112233445566778899aabbccdd",
            "q83vEjRWeJq83vEjRWeJq83vEjRWeJq8+/==",
            &"x ".repeat(40),
        ] {
            assert!(is_sensitive(bad), "{bad}");
        }
        let mut j = Json::obj()
            .with("ok", "granted")
            .with("k", "deadbeefdeadbeefdeadbeefdeadbeef")
            .with(
                "a",
                vec![Json::from("AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA")],
            );
        assert_eq!(redact(&mut j), 2);
        assert_eq!(
            j.to_json_string(),
            r#"{"ok":"granted","k":"<redacted>","a":["<redacted>"]}"#
        );
    }
}
