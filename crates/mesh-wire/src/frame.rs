//! LinkFrame (DESIGN §4.3): `lf_type u8` · body.
//!
//! - HS: `hs_step u8 (1|2|3)` · `proto_ver u8` · `noise_msg[..]`.
//! - SEC: `kp_ctr u32` (bit 31 key phase, bits 30-0 counter) · ciphertext · `tag[16]`;
//!   nonce `0x00000000 ‖ u64(kp_ctr)`, AAD `lf_type (0x01) ‖ kp_ctr (4 B LE)`.
//! - LAB: `lab_tag u16` · records (plaintext, padded with a PAD record to the SEC size).
//! - REJECT: `reason u8`.
//!
//! This module only frames bytes; sealing and opening is mesh-crypto's job (WP-07).

use sha2::{Digest, Sha256};

use crate::codec::{PutLe, Reader};
use crate::error::{Result, WireError};
use crate::link::MAX_DATAGRAM_FRAME;
use crate::record::{RecordWriter, rtype};

/// SEC tag length (Poly1305).
pub const TAG_LEN: usize = 16;
/// SEC overhead per frame including `lf_type`: 1 + 4 + 16.
pub const SEC_OVERHEAD: usize = 1 + 4 + TAG_LEN;
/// LAB header: `lf_type` + `lab_tag u16`.
pub const LAB_HEADER: usize = 3;
/// PAD body appended to LAB frames so they match the SEC size of the same records:
/// SEC = records + 21, LAB = records + 3 + PAD(1 + 1 + 16).
pub const LAB_PAD_BODY: usize = SEC_OVERHEAD - LAB_HEADER - 2;
/// HS header: `lf_type` + `hs_step` + `proto_ver`.
pub const HS_HEADER: usize = 3;
/// Bit 31 of `kp_ctr`.
pub const KEY_PHASE_BIT: u32 = 0x8000_0000;
/// Largest counter value (bits 30-0).
pub const CTR_MAX: u32 = 0x7FFF_FFFF;
/// Counter value at which a direction updates its key (2³⁰, §4.3).
pub const KEY_UPDATE_CTR: u32 = 1 << 30;
/// Noise prologue prefix (§4.3): `"cvic/v1" ‖ group_id(16) ‖ epoch u32`.
pub const NOISE_PROLOGUE_PREFIX: &[u8] = b"cvic/v1";
/// Noise pattern name (§8.4).
pub const NOISE_PATTERN: &str = "Noise_XXpsk1_25519_ChaChaPoly_SHA256";
/// Noise message sizes with empty payloads: msg1, msg2, msg3.
pub const NOISE_MSG_SIZES: [usize; 3] = [48, 96, 64];
/// Key-update HKDF label (§4.3).
pub const KEY_UPDATE_LABEL: &[u8] = b"cvic/v1/keyupdate";

/// `lf_type` values.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
#[repr(u8)]
pub enum LfType {
    /// Noise handshake.
    Hs = 0x00,
    /// Encrypted records.
    Sec = 0x01,
    /// Plaintext records, Lab links only.
    Lab = 0x02,
    /// Plaintext refusal; the sender then closes.
    Reject = 0x03,
}

impl LfType {
    /// Parses an `lf_type` byte.
    pub fn from_u8(v: u8) -> Result<Self> {
        Ok(match v {
            0x00 => LfType::Hs,
            0x01 => LfType::Sec,
            0x02 => LfType::Lab,
            0x03 => LfType::Reject,
            _ => {
                return Err(WireError::BadValue {
                    what: "lf_type",
                    value: u32::from(v),
                });
            }
        })
    }
}

/// `kp_ctr`: key phase bit + 31-bit counter.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct KpCtr(pub u32);

impl KpCtr {
    /// Builds from a phase and a counter (`ctr` is masked to 31 bits).
    pub const fn new(phase: bool, ctr: u32) -> Self {
        KpCtr(((phase as u32) << 31) | (ctr & CTR_MAX))
    }
    /// The key phase bit.
    pub const fn phase(self) -> bool {
        self.0 & KEY_PHASE_BIT != 0
    }
    /// The 31-bit counter.
    pub const fn ctr(self) -> u32 {
        self.0 & CTR_MAX
    }
    /// AEAD nonce: `0x00000000 ‖ u64(kp_ctr)` little-endian (12 B).
    pub fn nonce(self) -> [u8; 12] {
        let mut n = [0u8; 12];
        n[4..].copy_from_slice(&u64::from(self.0).to_le_bytes());
        n
    }
    /// AEAD associated data: `lf_type (0x01) ‖ kp_ctr (4 B LE)`.
    pub fn aad(self) -> [u8; 5] {
        let k = self.0.to_le_bytes();
        [LfType::Sec as u8, k[0], k[1], k[2], k[3]]
    }
}

/// REJECT reasons (§4.3) and CLOSE reasons (§4.4 adds 9 and 10).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
#[repr(u8)]
pub enum RejectReason {
    Version = 1,
    GroupEpochMismatch = 2,
    AtMaxLinks = 3,
    DuplicateLink = 4,
    MemberRemovedOrDenied = 5,
    LabCodeMismatch = 6,
    HandshakeFailure = 7,
    NodeIdCollision = 8,
    /// CLOSE only.
    Redundant = 9,
    /// CLOSE only.
    Leaving = 10,
}

impl RejectReason {
    /// Parses a reason byte; `close` admits the CLOSE-only values 9 and 10.
    pub fn from_u8(v: u8, close: bool) -> Result<Self> {
        Ok(match v {
            1 => Self::Version,
            2 => Self::GroupEpochMismatch,
            3 => Self::AtMaxLinks,
            4 => Self::DuplicateLink,
            5 => Self::MemberRemovedOrDenied,
            6 => Self::LabCodeMismatch,
            7 => Self::HandshakeFailure,
            8 => Self::NodeIdCollision,
            9 if close => Self::Redundant,
            10 if close => Self::Leaving,
            _ => {
                return Err(WireError::BadValue {
                    what: "reject reason",
                    value: u32::from(v),
                });
            }
        })
    }
}

/// A parsed LinkFrame borrowing its body from the input.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LinkFrame<'a> {
    /// Noise handshake message `step` (1-3) for protocol version `proto_ver`.
    Hs {
        step: u8,
        proto_ver: u8,
        noise_msg: &'a [u8],
    },
    /// Encrypted records: `sealed` is `ciphertext ‖ tag[16]` (at least 16 B).
    Sec { kp_ctr: KpCtr, sealed: &'a [u8] },
    /// Plaintext records on a Lab link.
    Lab { lab_tag: u16, records: &'a [u8] },
    /// Refusal.
    Reject { reason: RejectReason },
}

impl<'a> LinkFrame<'a> {
    /// The frame's `lf_type`.
    pub fn lf_type(&self) -> LfType {
        match self {
            LinkFrame::Hs { .. } => LfType::Hs,
            LinkFrame::Sec { .. } => LfType::Sec,
            LinkFrame::Lab { .. } => LfType::Lab,
            LinkFrame::Reject { .. } => LfType::Reject,
        }
    }

    /// Parses one frame (bounded by the datagram maximum, the largest of all links).
    pub fn decode(frame: &'a [u8]) -> Result<Self> {
        if frame.len() > MAX_DATAGRAM_FRAME {
            return Err(WireError::TooLong {
                what: "link frame",
                len: frame.len(),
                max: MAX_DATAGRAM_FRAME,
            });
        }
        let mut r = Reader::new(frame);
        match LfType::from_u8(r.u8("lf_type")?)? {
            LfType::Hs => {
                let step = r.u8("hs_step")?;
                if !(1..=3).contains(&step) {
                    return Err(WireError::BadValue {
                        what: "hs_step",
                        value: u32::from(step),
                    });
                }
                let proto_ver = r.u8("proto_ver")?;
                Ok(LinkFrame::Hs {
                    step,
                    proto_ver,
                    noise_msg: r.rest(),
                })
            }
            LfType::Sec => {
                let kp_ctr = KpCtr(r.u32("kp_ctr")?);
                let sealed = r.rest();
                if sealed.len() < TAG_LEN {
                    return Err(WireError::Truncated { what: "SEC tag" });
                }
                Ok(LinkFrame::Sec { kp_ctr, sealed })
            }
            LfType::Lab => {
                let lab_tag = r.u16("lab_tag")?;
                Ok(LinkFrame::Lab {
                    lab_tag,
                    records: r.rest(),
                })
            }
            LfType::Reject => {
                let reason = RejectReason::from_u8(r.u8("reject reason")?, false)?;
                r.finish("REJECT")?;
                Ok(LinkFrame::Reject { reason })
            }
        }
    }

    /// Serializes the frame exactly as given (no padding added).
    pub fn encode(&self) -> Result<Vec<u8>> {
        let mut out = Vec::new();
        out.put_u8(self.lf_type() as u8);
        match *self {
            LinkFrame::Hs {
                step,
                proto_ver,
                noise_msg,
            } => {
                if !(1..=3).contains(&step) {
                    return Err(WireError::BadValue {
                        what: "hs_step",
                        value: u32::from(step),
                    });
                }
                out.put_u8(step);
                out.put_u8(proto_ver);
                out.put(noise_msg);
            }
            LinkFrame::Sec { kp_ctr, sealed } => {
                if sealed.len() < TAG_LEN {
                    return Err(WireError::Truncated { what: "SEC tag" });
                }
                out.put_u32(kp_ctr.0);
                out.put(sealed);
            }
            LinkFrame::Lab { lab_tag, records } => {
                out.put_u16(lab_tag);
                out.put(records);
            }
            LinkFrame::Reject { reason } => out.put_u8(reason as u8),
        }
        if out.len() > MAX_DATAGRAM_FRAME {
            return Err(WireError::TooLong {
                what: "link frame",
                len: out.len(),
                max: MAX_DATAGRAM_FRAME,
            });
        }
        Ok(out)
    }
}

/// `lab_tag = SHA-256("cvic/lab" ‖ u16 lab_code)[0..2]` read as a little-endian u16
/// (§4.3, §8.3). `lab_code` is the shared 4-digit code (0-9999).
pub fn lab_tag(lab_code: u16) -> u16 {
    let mut h = Sha256::new();
    h.update(b"cvic/lab");
    h.update(lab_code.to_le_bytes());
    let d = h.finalize();
    u16::from_le_bytes([d[0], d[1]])
}

/// Builds a LAB frame from encoded records and appends the PAD record that makes its
/// on-air size equal to the SEC frame of the same records (§4.3).
pub fn encode_lab_padded(lab_tag: u16, records: &[u8]) -> Result<Vec<u8>> {
    let mut body = records.to_vec();
    let mut pad = RecordWriter::with_capacity_limit(LAB_PAD_BODY + 2);
    pad.push_raw(rtype::PAD, &[0u8; LAB_PAD_BODY])?;
    body.extend_from_slice(pad.as_bytes());
    LinkFrame::Lab {
        lab_tag,
        records: &body,
    }
    .encode()
}

/// Size of the SEC frame that carries `plaintext_len` record bytes.
pub const fn sec_frame_len(plaintext_len: usize) -> usize {
    plaintext_len + SEC_OVERHEAD
}

/// Noise prologue: `"cvic/v1" ‖ group_id(16) ‖ epoch u32 LE` (§4.3).
pub fn noise_prologue(group_id: &[u8; 16], epoch: u32) -> [u8; 27] {
    let mut p = [0u8; 27];
    p[..7].copy_from_slice(NOISE_PROLOGUE_PREFIX);
    p[7..23].copy_from_slice(group_id);
    p[23..].copy_from_slice(&epoch.to_le_bytes());
    p
}
