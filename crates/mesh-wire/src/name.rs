//! Name encoders (DESIGN §4.6.1): names are UTF-8 bytes, `name_len` counts bytes, and
//! encoders truncate at the last grapheme-cluster boundary that fits (display name
//! ≤ 24 B, group name ≤ 32 B), so a truncated name never splits an emoji, a flag or a
//! letter with its combining marks.

use unicode_segmentation::UnicodeSegmentation;

use crate::codec::Reader;
use crate::error::{Result, WireError};

/// Largest display (rider) name in bytes.
pub const DISPLAY_NAME_MAX: usize = 24;
/// Largest group name in bytes.
pub const GROUP_NAME_MAX: usize = 32;

/// The longest prefix of `name` that is at most `max_bytes` long and ends on an
/// extended grapheme-cluster boundary.
pub fn truncate_graphemes(name: &str, max_bytes: usize) -> &str {
    if name.len() <= max_bytes {
        return name;
    }
    let mut end = 0usize;
    for (start, g) in name.grapheme_indices(true) {
        let next = start + g.len();
        if next > max_bytes {
            break;
        }
        end = next;
    }
    name.get(..end).unwrap_or("")
}

/// A display name as wire bytes (≤ 24 B, grapheme-safe truncation).
pub fn display_name_bytes(name: &str) -> Vec<u8> {
    truncate_graphemes(name, DISPLAY_NAME_MAX)
        .as_bytes()
        .to_vec()
}

/// A group name as wire bytes (≤ 32 B, grapheme-safe truncation).
pub fn group_name_bytes(name: &str) -> Vec<u8> {
    truncate_graphemes(name, GROUP_NAME_MAX).as_bytes().to_vec()
}

/// Appends `name_len u8 · name[..]`, truncating `name` to `max_bytes` at a grapheme
/// boundary.
pub fn put_name(out: &mut Vec<u8>, name: &str, max_bytes: usize) {
    let t = truncate_graphemes(name, max_bytes.min(usize::from(u8::MAX)));
    out.push(t.len() as u8);
    out.extend_from_slice(t.as_bytes());
}

/// Reads `name_len u8 · name[..]`, refusing more than `max_bytes` and invalid UTF-8.
pub(crate) fn get_name<'a>(r: &mut Reader<'a>, max_bytes: usize) -> Result<&'a str> {
    let len = usize::from(r.u8("name_len")?);
    if len > max_bytes {
        return Err(WireError::TooLong {
            what: "name",
            len,
            max: max_bytes,
        });
    }
    decode_name(r.bytes(len, "name")?)
}

/// Validates wire name bytes as UTF-8.
pub fn decode_name(bytes: &[u8]) -> Result<&str> {
    core::str::from_utf8(bytes).map_err(|_| WireError::BadUtf8)
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    #[test]
    fn ascii_and_short_names_are_untouched() {
        assert_eq!(truncate_graphemes("Ana", 24), "Ana");
        assert_eq!(display_name_bytes("Ana"), b"Ana".to_vec());
        let long = "abcdefghijklmnopqrstuvwxyz";
        assert_eq!(display_name_bytes(long).len(), 24);
    }

    #[test]
    fn never_splits_a_grapheme() {
        // "é" as e + combining acute (3 B), 8 of them = 24 B: fits exactly.
        let e = "e\u{301}";
        let name = e.repeat(9);
        assert_eq!(truncate_graphemes(&name, 24), e.repeat(8));
        assert_eq!(truncate_graphemes(&name, 25), e.repeat(8));
        // A family emoji is one 25-byte grapheme: nothing of it fits in 24 B.
        let family = "\u{1F468}\u{200D}\u{1F469}\u{200D}\u{1F467}\u{200D}\u{1F466}";
        assert_eq!(family.len(), 25);
        assert_eq!(truncate_graphemes(&format!("Jo{family}"), 24), "Jo");
        // Flags are two regional indicators (8 B) and stay whole.
        let flags = "\u{1F1F2}\u{1F1FD}".repeat(4);
        assert_eq!(
            truncate_graphemes(&flags, 30),
            "\u{1F1F2}\u{1F1FD}".repeat(3)
        );
        assert_eq!(group_name_bytes(&flags).len(), 32);
    }

    #[test]
    fn length_prefixed_names() {
        let mut out = Vec::new();
        put_name(&mut out, "Mónica", DISPLAY_NAME_MAX);
        assert_eq!(out[0], 7);
        let mut r = Reader::new(&out);
        assert_eq!(get_name(&mut r, DISPLAY_NAME_MAX).unwrap(), "Mónica");
        let bad = [2u8, 0xC3, 0x28];
        assert_eq!(
            get_name(&mut Reader::new(&bad), 24),
            Err(WireError::BadUtf8)
        );
        let long = [25u8; 26];
        assert!(get_name(&mut Reader::new(&long), 24).is_err());
    }

    proptest! {
        #[test]
        fn truncation_is_a_valid_bounded_prefix(s in "\\PC{0,40}", max in 0usize..40) {
            let t = truncate_graphemes(&s, max);
            prop_assert!(t.len() <= max);
            prop_assert!(s.starts_with(t));
            // The cut is on a grapheme boundary: the prefix's graphemes are a prefix of
            // the original's graphemes.
            let a: Vec<&str> = t.graphemes(true).collect();
            let b: Vec<&str> = s.graphemes(true).take(a.len()).collect();
            prop_assert_eq!(a, b);
        }
    }
}
