//! Bounded little-endian reader and writer used by every codec (DESIGN §4: all
//! multi-byte integers are little-endian).

use crate::error::{Result, WireError};

/// A cursor over a byte slice. Every read checks the remaining length first.
#[derive(Debug, Clone)]
pub(crate) struct Reader<'a> {
    buf: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    pub(crate) fn new(buf: &'a [u8]) -> Self {
        Reader { buf, pos: 0 }
    }

    pub(crate) fn remaining(&self) -> usize {
        self.buf.len().saturating_sub(self.pos)
    }

    pub(crate) fn bytes(&mut self, n: usize, what: &'static str) -> Result<&'a [u8]> {
        if self.remaining() < n {
            return Err(WireError::Truncated { what });
        }
        let start = self.pos;
        self.pos += n;
        self.buf
            .get(start..start + n)
            .ok_or(WireError::Truncated { what })
    }

    pub(crate) fn array<const N: usize>(&mut self, what: &'static str) -> Result<[u8; N]> {
        let s = self.bytes(N, what)?;
        let mut out = [0u8; N];
        out.copy_from_slice(s);
        Ok(out)
    }

    pub(crate) fn u8(&mut self, what: &'static str) -> Result<u8> {
        Ok(self.array::<1>(what)?[0])
    }

    pub(crate) fn u16(&mut self, what: &'static str) -> Result<u16> {
        Ok(u16::from_le_bytes(self.array(what)?))
    }

    pub(crate) fn u32(&mut self, what: &'static str) -> Result<u32> {
        Ok(u32::from_le_bytes(self.array(what)?))
    }

    pub(crate) fn i32(&mut self, what: &'static str) -> Result<i32> {
        Ok(i32::from_le_bytes(self.array(what)?))
    }

    pub(crate) fn u64(&mut self, what: &'static str) -> Result<u64> {
        Ok(u64::from_le_bytes(self.array(what)?))
    }

    /// Everything not read yet.
    pub(crate) fn rest(&mut self) -> &'a [u8] {
        let start = self.pos.min(self.buf.len());
        self.pos = self.buf.len();
        self.buf.get(start..).unwrap_or(&[])
    }

    /// Fails unless the whole input was consumed.
    pub(crate) fn finish(&self, what: &'static str) -> Result<()> {
        if self.remaining() == 0 {
            Ok(())
        } else {
            Err(WireError::TrailingBytes { what })
        }
    }
}

/// Little-endian append helpers on `Vec<u8>`.
pub(crate) trait PutLe {
    fn put_u8(&mut self, v: u8);
    fn put_u16(&mut self, v: u16);
    fn put_u32(&mut self, v: u32);
    fn put_i32(&mut self, v: i32);
    fn put_u64(&mut self, v: u64);
    fn put(&mut self, v: &[u8]);
}

impl PutLe for Vec<u8> {
    fn put_u8(&mut self, v: u8) {
        self.push(v);
    }
    fn put_u16(&mut self, v: u16) {
        self.extend_from_slice(&v.to_le_bytes());
    }
    fn put_u32(&mut self, v: u32) {
        self.extend_from_slice(&v.to_le_bytes());
    }
    fn put_i32(&mut self, v: i32) {
        self.extend_from_slice(&v.to_le_bytes());
    }
    fn put_u64(&mut self, v: u64) {
        self.extend_from_slice(&v.to_le_bytes());
    }
    fn put(&mut self, v: &[u8]) {
        self.extend_from_slice(v);
    }
}

/// Checks `len <= max`.
pub(crate) fn check_max(what: &'static str, len: usize, max: usize) -> Result<()> {
    if len > max {
        Err(WireError::TooLong { what, len, max })
    } else {
        Ok(())
    }
}

/// Lowercase hex of `bytes` (vectors, docs, debug output).
pub fn to_hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push(char::from(DIGITS[usize::from(b >> 4)]));
        s.push(char::from(DIGITS[usize::from(b & 0x0F)]));
    }
    s
}

/// Parses lowercase or uppercase hex; `None` on odd length or a non-hex digit.
pub fn from_hex(s: &str) -> Option<Vec<u8>> {
    fn nibble(c: u8) -> Option<u8> {
        match c {
            b'0'..=b'9' => Some(c - b'0'),
            b'a'..=b'f' => Some(c - b'a' + 10),
            b'A'..=b'F' => Some(c - b'A' + 10),
            _ => None,
        }
    }
    let b = s.as_bytes();
    let (pairs, rest) = b.as_chunks::<2>();
    if !rest.is_empty() {
        return None;
    }
    pairs
        .iter()
        .map(|[hi, lo]| Some((nibble(*hi)? << 4) | nibble(*lo)?))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reader_is_bounded() {
        let mut r = Reader::new(&[1, 2, 3]);
        assert_eq!(r.u16("a"), Ok(0x0201));
        assert_eq!(r.u16("b"), Err(WireError::Truncated { what: "b" }));
        assert_eq!(r.u8("c"), Ok(3));
        assert_eq!(r.finish("x"), Ok(()));
        assert!(r.rest().is_empty());
    }

    #[test]
    fn hex_round_trip() {
        let v = vec![0x00, 0xAB, 0x7F, 0xFF];
        assert_eq!(to_hex(&v), "00ab7fff");
        assert_eq!(from_hex("00AB7fff"), Some(v));
        assert_eq!(from_hex("0"), None);
        assert_eq!(from_hex("zz"), None);
    }
}
