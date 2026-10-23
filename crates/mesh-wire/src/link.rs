//! Link framing per transport (DESIGN §4.2).
//!
//! - Stream links (BLE L2CAP CoC, v2 tunnel): `len u16` + `frame[len]`, `len ∈ [1, 1024]`;
//!   an invalid length closes the link (no resync). [`StreamDeframer`] accepts any chunk
//!   boundary; [`encode_stream`] writes one frame.
//! - GATT data path: per ATT value `hdr u8` + `chunk`, `hdr` bit7 START, bit6 END,
//!   bits5-0 frame counter mod 64; chunk ≤ `ATT_MTU − 3 − 1`. [`GattChunker`] splits,
//!   [`GattReassembler`] reassembles (bounded at 1024 B).
//! - Datagram links (Wi-Fi Aware): one datagram is one frame, at most 1200 B.

use crate::error::{Result, WireError};

/// Maximum LinkFrame on stream and GATT links.
pub const MAX_STREAM_FRAME: usize = 1024;
/// Maximum LinkFrame on datagram links (Wi-Fi Aware UDP).
pub const MAX_DATAGRAM_FRAME: usize = 1200;
/// Bytes of the stream length prefix.
pub const STREAM_PREFIX: usize = 2;
/// GATT chunk header bits.
pub const GATT_START: u8 = 0x80;
/// GATT END bit.
pub const GATT_END: u8 = 0x40;
/// GATT frame counter mask (mod 64).
pub const GATT_COUNTER_MASK: u8 = 0x3F;
/// Smallest ATT MTU (BLE 4.0 default).
pub const ATT_MTU_MIN: u16 = 23;
/// Largest ATT MTU.
pub const ATT_MTU_MAX: u16 = 517;

/// Prepends the `len u16` prefix to one LinkFrame.
pub fn encode_stream(frame: &[u8]) -> Result<Vec<u8>> {
    check_frame_len(frame.len(), MAX_STREAM_FRAME)?;
    let mut out = Vec::with_capacity(frame.len() + STREAM_PREFIX);
    out.extend_from_slice(&(frame.len() as u16).to_le_bytes());
    out.extend_from_slice(frame);
    Ok(out)
}

/// Checks a datagram (one frame) against the datagram limits.
pub fn check_datagram(datagram: &[u8]) -> Result<()> {
    check_frame_len(datagram.len(), MAX_DATAGRAM_FRAME)
}

fn check_frame_len(len: usize, max: usize) -> Result<()> {
    if len == 0 {
        return Err(WireError::TooShort {
            what: "link frame",
            len,
            min: 1,
        });
    }
    if len > max {
        return Err(WireError::TooLong {
            what: "link frame",
            len,
            max,
        });
    }
    Ok(())
}

/// Incremental deframer for stream links. Feed arbitrary chunks with [`push`]; pull
/// complete frames with [`next_frame`]. After an invalid length the deframer is
/// poisoned: the link must be closed (no resync, DESIGN §4.2).
///
/// [`push`]: StreamDeframer::push
/// [`next_frame`]: StreamDeframer::next_frame
#[derive(Debug, Default, Clone)]
pub struct StreamDeframer {
    buf: Vec<u8>,
    start: usize,
    poisoned: Option<WireError>,
}

impl StreamDeframer {
    /// A fresh deframer.
    pub fn new() -> Self {
        Self::default()
    }

    /// Appends received bytes. The internal buffer never holds more than one partial
    /// frame plus its prefix beyond what the caller has not pulled yet.
    pub fn push(&mut self, chunk: &[u8]) {
        if self.poisoned.is_some() {
            return;
        }
        if self.start > 0 && self.start == self.buf.len() {
            self.buf.clear();
            self.start = 0;
        }
        self.buf.extend_from_slice(chunk);
    }

    /// The next complete frame, `Ok(None)` if more bytes are needed, or the error that
    /// poisoned the stream.
    pub fn next_frame(&mut self) -> Result<Option<Vec<u8>>> {
        if let Some(e) = self.poisoned {
            return Err(e);
        }
        let avail = self.buf.get(self.start..).unwrap_or(&[]);
        let Some(prefix) = avail.get(..STREAM_PREFIX) else {
            return Ok(None);
        };
        let len = usize::from(u16::from_le_bytes([prefix[0], prefix[1]]));
        if let Err(e) = check_frame_len(len, MAX_STREAM_FRAME) {
            self.poisoned = Some(e);
            self.buf = Vec::new();
            self.start = 0;
            return Err(e);
        }
        let Some(frame) = avail.get(STREAM_PREFIX..STREAM_PREFIX + len) else {
            return Ok(None);
        };
        let frame = frame.to_vec();
        self.start += STREAM_PREFIX + len;
        if self.start == self.buf.len() {
            self.buf.clear();
            self.start = 0;
        } else if self.start > MAX_STREAM_FRAME {
            self.buf.drain(..self.start);
            self.start = 0;
        }
        Ok(Some(frame))
    }

    /// Pushes `chunk` and drains every complete frame (the usual call per inbound
    /// stream chunk). On error the link must be closed.
    pub fn feed(&mut self, chunk: &[u8]) -> Result<Vec<Vec<u8>>> {
        self.push(chunk);
        let mut frames = Vec::new();
        while let Some(f) = self.next_frame()? {
            frames.push(f);
        }
        Ok(frames)
    }

    /// Bytes buffered but not yet returned as a frame.
    pub fn buffered(&self) -> usize {
        self.buf.len().saturating_sub(self.start)
    }

    /// `true` once an invalid length was seen.
    pub fn is_poisoned(&self) -> bool {
        self.poisoned.is_some()
    }
}

/// Splits frames into GATT values for one direction of one link.
#[derive(Debug, Clone)]
pub struct GattChunker {
    att_mtu: u16,
    counter: u8,
}

impl GattChunker {
    /// A chunker for the negotiated ATT MTU (clamped to `23..=517`).
    pub fn new(att_mtu: u16) -> Self {
        GattChunker {
            att_mtu: att_mtu.clamp(ATT_MTU_MIN, ATT_MTU_MAX),
            counter: 0,
        }
    }

    /// Updates the MTU after an exchange; affects the next frame.
    pub fn set_att_mtu(&mut self, att_mtu: u16) {
        self.att_mtu = att_mtu.clamp(ATT_MTU_MIN, ATT_MTU_MAX);
    }

    /// Payload bytes per GATT value: `ATT_MTU − 3 − 1`.
    pub fn chunk_payload(&self) -> usize {
        usize::from(self.att_mtu) - 3 - 1
    }

    /// The counter the next frame will carry.
    pub fn next_counter(&self) -> u8 {
        self.counter
    }

    /// Splits one frame into ATT values (each ≤ `ATT_MTU − 3`).
    pub fn chunk(&mut self, frame: &[u8]) -> Result<Vec<Vec<u8>>> {
        check_frame_len(frame.len(), MAX_STREAM_FRAME)?;
        let per = self.chunk_payload();
        let n = frame.len().div_ceil(per);
        let ctr = self.counter & GATT_COUNTER_MASK;
        self.counter = (self.counter + 1) & GATT_COUNTER_MASK;
        let mut out = Vec::with_capacity(n);
        for (i, part) in frame.chunks(per).enumerate() {
            let mut hdr = ctr;
            if i == 0 {
                hdr |= GATT_START;
            }
            if i + 1 == n {
                hdr |= GATT_END;
            }
            let mut v = Vec::with_capacity(part.len() + 1);
            v.push(hdr);
            v.extend_from_slice(part);
            out.push(v);
        }
        Ok(out)
    }
}

/// What one GATT value did to the reassembler.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GattEvent {
    /// A frame completed.
    Frame(Vec<u8>),
    /// The value was absorbed; the frame is not complete yet.
    Pending,
    /// The value was dropped (`reason`); any partial frame was discarded.
    Dropped(GattDrop),
}

/// Why a GATT value was dropped.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GattDrop {
    /// Empty ATT value (no header).
    Empty,
    /// A continuation without a START (or after a discard).
    NoStart,
    /// Counter differs from the partial frame's counter.
    CounterMismatch,
    /// Reassembled size would exceed 1024 B.
    Overflow,
    /// The completed frame is empty.
    EmptyFrame,
}

/// Reassembles GATT values into frames (DESIGN §4.2): a START discards any partial
/// frame; a counter mismatch discards it; the buffer is bounded at 1024 B; END completes.
#[derive(Debug, Default, Clone)]
pub struct GattReassembler {
    partial: Vec<u8>,
    counter: Option<u8>,
}

impl GattReassembler {
    /// A fresh reassembler.
    pub fn new() -> Self {
        Self::default()
    }

    /// Bytes of the partial frame held now.
    pub fn partial_len(&self) -> usize {
        self.partial.len()
    }

    /// Absorbs one ATT value.
    pub fn push(&mut self, value: &[u8]) -> GattEvent {
        let Some((&hdr, chunk)) = value.split_first() else {
            return GattEvent::Dropped(GattDrop::Empty);
        };
        let ctr = hdr & GATT_COUNTER_MASK;
        if hdr & GATT_START != 0 {
            self.partial.clear();
            self.counter = Some(ctr);
        } else {
            match self.counter {
                None => return GattEvent::Dropped(GattDrop::NoStart),
                Some(c) if c != ctr => {
                    self.reset();
                    return GattEvent::Dropped(GattDrop::CounterMismatch);
                }
                Some(_) => {}
            }
        }
        if self.partial.len() + chunk.len() > MAX_STREAM_FRAME {
            self.reset();
            return GattEvent::Dropped(GattDrop::Overflow);
        }
        self.partial.extend_from_slice(chunk);
        if hdr & GATT_END == 0 {
            return GattEvent::Pending;
        }
        let frame = core::mem::take(&mut self.partial);
        self.counter = None;
        if frame.is_empty() {
            GattEvent::Dropped(GattDrop::EmptyFrame)
        } else {
            GattEvent::Frame(frame)
        }
    }

    fn reset(&mut self) {
        self.partial.clear();
        self.counter = None;
    }
}
