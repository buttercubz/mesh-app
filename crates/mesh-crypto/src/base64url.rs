//! Unpadded base64url (RFC 4648 §5) for the invite fragment. Small and bounded, so the
//! crate needs no extra dependency.

use crate::error::{CryptoError, Result};

const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";

/// Encodes without padding.
pub fn encode(data: &[u8]) -> String {
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    let sym = |v: u32| char::from(ALPHABET[(v & 63) as usize]);
    for chunk in data.chunks(3) {
        let b0 = u32::from(chunk[0]);
        let b1 = chunk.get(1).map_or(0, |&b| u32::from(b));
        let b2 = chunk.get(2).map_or(0, |&b| u32::from(b));
        let n = (b0 << 16) | (b1 << 8) | b2;
        out.push(sym(n >> 18));
        out.push(sym(n >> 12));
        if chunk.len() > 1 {
            out.push(sym(n >> 6));
        }
        if chunk.len() > 2 {
            out.push(sym(n));
        }
    }
    out
}

fn value(c: u8) -> Option<u32> {
    Some(u32::from(match c {
        b'A'..=b'Z' => c - b'A',
        b'a'..=b'z' => c - b'a' + 26,
        b'0'..=b'9' => c - b'0' + 52,
        b'-' => 62,
        b'_' => 63,
        _ => return None,
    }))
}

/// Decodes unpadded base64url. Trailing `=` padding is tolerated; any other character,
/// an impossible length (4n + 1) or non-zero trailing bits are refused, so every byte
/// string has exactly one accepted encoding.
pub fn decode(s: &str, max_len: usize) -> Result<Vec<u8>> {
    let s = s.trim_end_matches('=').as_bytes();
    if s.len() % 4 == 1 {
        return Err(CryptoError::BadInvite("base64url length"));
    }
    let out_len = s.len() / 4 * 3 + (s.len() % 4).saturating_sub(1);
    if out_len > max_len {
        return Err(CryptoError::BadInvite("invite too long"));
    }
    let mut out = Vec::with_capacity(out_len);
    for chunk in s.chunks(4) {
        let mut n = 0u32;
        for (i, &c) in chunk.iter().enumerate() {
            let v = value(c).ok_or(CryptoError::BadInvite("base64url character"))?;
            n |= v << (18 - 6 * i);
        }
        let bytes = [(n >> 16) as u8, (n >> 8) as u8, n as u8];
        let take = chunk.len() - 1;
        // Canonical form: the unused low bits of the last symbol must be zero.
        if bytes[take..].iter().any(|&b| b != 0) && take < 3 {
            return Err(CryptoError::BadInvite("base64url trailing bits"));
        }
        out.extend_from_slice(&bytes[..take]);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand_chacha::ChaCha20Rng;
    use rand_core::{RngCore, SeedableRng};

    #[test]
    fn rfc4648_vectors() {
        // RFC 4648 §10 (base64 test vectors; same symbols in this range, no padding).
        for (plain, enc) in [
            ("", ""),
            ("f", "Zg"),
            ("fo", "Zm8"),
            ("foo", "Zm9v"),
            ("foob", "Zm9vYg"),
            ("fooba", "Zm9vYmE"),
            ("foobar", "Zm9vYmFy"),
        ] {
            assert_eq!(encode(plain.as_bytes()), enc);
            assert_eq!(decode(enc, 64).unwrap(), plain.as_bytes());
        }
        assert_eq!(encode(&[0xFB, 0xFF]), "-_8");
        assert_eq!(decode("Zg==", 8).unwrap(), b"f");
    }

    #[test]
    fn rejects_malformed() {
        assert!(decode("Z", 8).is_err());
        assert!(decode("Zh", 8).is_err(), "non-zero trailing bits");
        assert!(
            decode("Zm+v", 8).is_err(),
            "standard alphabet is not base64url"
        );
        assert!(decode("Zm9vYmFy", 5).is_err(), "bounded");
    }

    #[test]
    fn round_trips_random_bytes() {
        let mut rng = ChaCha20Rng::from_seed([3; 32]);
        for len in 0..300 {
            let mut b = vec![0u8; len];
            rng.fill_bytes(&mut b);
            let e = encode(&b);
            assert!(!e.contains('='));
            assert_eq!(decode(&e, 300).unwrap(), b);
        }
    }
}
