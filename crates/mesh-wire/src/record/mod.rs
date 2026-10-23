//! Records inside SEC/LAB plaintext (DESIGN §4.4): `rtype u8` · `len` LEB128 (1-2 B,
//! ≤ 16383) · `body[len]`; at most 32 records per frame; unknown types are skipped.

mod body;
mod lab;

pub use body::{
    Caps, Close, IdProof, Lstat, Nbr, NbrEntry, NbrFlags, Ping, Pong, Record, Sync, SyncKind,
    Upgrade, UpgradeOp, UpgradeTransport,
};
pub use lab::{LabCtl, LabCtlOp, LabGps, Lecho, Lprobe};

use crate::codec::Reader;
use crate::error::{Result, WireError};

/// Record type bytes.
pub mod rtype {
    pub const MESH: u8 = 0x01;
    pub const PING: u8 = 0x02;
    pub const PONG: u8 = 0x03;
    pub const NBR: u8 = 0x04;
    pub const LSTAT: u8 = 0x05;
    pub const CAPS: u8 = 0x06;
    pub const LPROBE: u8 = 0x07;
    pub const LECHO: u8 = 0x08;
    pub const UPGRADE: u8 = 0x09;
    pub const CLOSE: u8 = 0x0A;
    pub const LAB_CTL: u8 = 0x0B;
    pub const IDPROOF: u8 = 0x0C;
    pub const LAB_GPS: u8 = 0x0D;
    pub const PAD: u8 = 0x0E;
    pub const SYNC: u8 = 0x0F;
}

/// At most 32 records per frame.
pub const MAX_RECORDS_PER_FRAME: usize = 32;
/// Largest record body (two-byte LEB128).
pub const MAX_RECORD_BODY: usize = 16383;

/// Encoded size of a LEB128 length.
pub const fn leb128_len(v: usize) -> usize {
    if v < 0x80 { 1 } else { 2 }
}

/// Encoded size of a record with a `body_len` body: `1 + leb + body`.
pub const fn record_len(body_len: usize) -> usize {
    1 + leb128_len(body_len) + body_len
}

pub(crate) fn put_leb128(out: &mut Vec<u8>, v: usize) -> Result<()> {
    if v > MAX_RECORD_BODY {
        return Err(WireError::TooLong {
            what: "record body",
            len: v,
            max: MAX_RECORD_BODY,
        });
    }
    if v < 0x80 {
        out.push(v as u8);
    } else {
        out.push((v as u8 & 0x7F) | 0x80);
        out.push((v >> 7) as u8);
    }
    Ok(())
}

/// Reads a 1-2 byte LEB128 length; refuses non-shortest forms and a third byte.
pub(crate) fn get_leb128(r: &mut Reader<'_>) -> Result<usize> {
    let b0 = r.u8("record len")?;
    if b0 & 0x80 == 0 {
        return Ok(usize::from(b0));
    }
    let b1 = r.u8("record len")?;
    if b1 & 0x80 != 0 || b1 == 0 {
        return Err(WireError::BadLeb128);
    }
    Ok(usize::from(b0 & 0x7F) | (usize::from(b1) << 7))
}

/// One raw record: type and body, without interpreting the body.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RawRecord<'a> {
    pub rtype: u8,
    pub body: &'a [u8],
}

impl<'a> RawRecord<'a> {
    /// Interprets the body. Unknown types become [`Record::Unknown`].
    pub fn parse(&self) -> Result<Record<'a>> {
        Record::decode(self.rtype, self.body)
    }
}

/// Iterates the records of one plaintext. Yields an error once (then stops) on a
/// truncated record, a bad length, or a 33rd record. Unknown record types are yielded
/// as [`Record::Unknown`] so callers can count and skip them.
#[derive(Debug, Clone)]
pub struct RecordIter<'a> {
    r: Reader<'a>,
    count: usize,
    done: bool,
}

impl<'a> RecordIter<'a> {
    /// Iterates over `plaintext`.
    pub fn new(plaintext: &'a [u8]) -> Self {
        RecordIter {
            r: Reader::new(plaintext),
            count: 0,
            done: false,
        }
    }

    /// Next raw record without body interpretation.
    pub fn next_raw(&mut self) -> Option<Result<RawRecord<'a>>> {
        if self.done || self.r.remaining() == 0 {
            return None;
        }
        let res = self.read_raw();
        if res.is_err() {
            self.done = true;
        }
        Some(res)
    }

    fn read_raw(&mut self) -> Result<RawRecord<'a>> {
        if self.count == MAX_RECORDS_PER_FRAME {
            return Err(WireError::TooManyRecords);
        }
        let rtype = self.r.u8("rtype")?;
        let len = get_leb128(&mut self.r)?;
        let body = self.r.bytes(len, "record body")?;
        self.count += 1;
        Ok(RawRecord { rtype, body })
    }

    /// Collects every record, failing on the first malformed one.
    pub fn collect_all(plaintext: &'a [u8]) -> Result<Vec<Record<'a>>> {
        RecordIter::new(plaintext).collect()
    }
}

impl<'a> Iterator for RecordIter<'a> {
    type Item = Result<Record<'a>>;

    fn next(&mut self) -> Option<Self::Item> {
        let raw = match self.next_raw()? {
            Ok(raw) => raw,
            Err(e) => return Some(Err(e)),
        };
        let rec = raw.parse();
        if rec.is_err() {
            self.done = true;
        }
        Some(rec)
    }
}

/// Coalesces records into one plaintext, refusing to exceed a byte budget or 32 records.
#[derive(Debug, Clone)]
pub struct RecordWriter {
    buf: Vec<u8>,
    limit: usize,
    count: usize,
}

impl RecordWriter {
    /// A writer whose plaintext fits a stream/GATT SEC frame (1024 − 21 B).
    pub fn new() -> Self {
        Self::with_capacity_limit(crate::link::MAX_STREAM_FRAME - crate::frame::SEC_OVERHEAD)
    }

    /// A writer that never grows beyond `limit` bytes.
    pub fn with_capacity_limit(limit: usize) -> Self {
        RecordWriter {
            buf: Vec::new(),
            limit,
            count: 0,
        }
    }

    /// Bytes still free.
    pub fn remaining(&self) -> usize {
        self.limit.saturating_sub(self.buf.len())
    }

    /// Records written so far.
    pub fn count(&self) -> usize {
        self.count
    }

    /// `true` if a record with a `body_len` body still fits.
    pub fn fits(&self, body_len: usize) -> bool {
        self.count < MAX_RECORDS_PER_FRAME
            && body_len <= MAX_RECORD_BODY
            && record_len(body_len) <= self.remaining()
    }

    /// Appends a record from its type and an already encoded body.
    pub fn push_raw(&mut self, rtype: u8, body: &[u8]) -> Result<()> {
        if self.count >= MAX_RECORDS_PER_FRAME {
            return Err(WireError::TooManyRecords);
        }
        if !self.fits(body.len()) {
            return Err(WireError::NoSpace { what: "record" });
        }
        self.buf.push(rtype);
        put_leb128(&mut self.buf, body.len())?;
        self.buf.extend_from_slice(body);
        self.count += 1;
        Ok(())
    }

    /// Appends a typed record.
    pub fn push(&mut self, rec: &Record<'_>) -> Result<()> {
        let body = rec.encode_body()?;
        self.push_raw(rec.rtype(), &body)
    }

    /// The plaintext written so far.
    pub fn as_bytes(&self) -> &[u8] {
        &self.buf
    }

    /// Consumes the writer.
    pub fn into_bytes(self) -> Vec<u8> {
        self.buf
    }
}

impl Default for RecordWriter {
    fn default() -> Self {
        Self::new()
    }
}

/// Encodes a list of records into one plaintext with the default limit.
pub fn encode_records(records: &[Record<'_>]) -> Result<Vec<u8>> {
    let mut w = RecordWriter::new();
    for r in records {
        w.push(r)?;
    }
    Ok(w.into_bytes())
}
