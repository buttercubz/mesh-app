//! The `LinkSecurity` trait (DESIGN §3.3) and its `Plain` implementation for Lab links
//! (LAB frames: plaintext records gated by the 4-digit Lab code, padded to the SEC
//! size, §4.3, §8.8). The secure implementation (Noise XXpsk1 + SEC) is WP-10B's.

use mesh_wire::WireError;
use mesh_wire::frame::{LAB_HEADER, LAB_PAD_BODY, LinkFrame, RejectReason, encode_lab_padded};
use mesh_wire::record::record_len;

use crate::Instant;

/// Progress of the link security setup.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SecurityStep {
    /// Handshake in progress.
    Pending,
    /// Records may flow.
    Established,
    /// The link must be refused with this reason (REJECT, then close).
    Failed(RejectReason),
}

/// Why a frame could not be sealed or opened.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SecurityError {
    /// No keys yet (handshake not finished).
    NotEstablished,
    /// The frame type is not valid on this link (e.g. SEC on a Lab link).
    WrongFrame,
    /// Authentication failed (AEAD tag, replay); the frame is dropped.
    Auth,
    /// The peer must be refused with this reason.
    Reject(RejectReason),
    /// Encoding failed.
    Wire(WireError),
}

impl From<WireError> for SecurityError {
    fn from(e: WireError) -> Self {
        SecurityError::Wire(e)
    }
}

/// Per-link security (DESIGN §3.3): handshake, sealing of outgoing record plaintexts
/// into one LinkFrame and opening of incoming SEC/LAB frames. `open` returns plaintext
/// records only for authenticated frames, so dedup and delivery happen after
/// authentication (§5.3).
pub trait LinkSecurity {
    /// The link came up; an initiator may queue its first handshake frames in `out`.
    fn on_start(&mut self, now: Instant, out: &mut Vec<Vec<u8>>) -> SecurityStep;

    /// A handshake frame arrived; reply frames go to `out`.
    fn on_hs(
        &mut self,
        now: Instant,
        step: u8,
        proto_ver: u8,
        noise_msg: &[u8],
        out: &mut Vec<Vec<u8>>,
    ) -> SecurityStep;

    /// Seals one record plaintext into a complete LinkFrame.
    fn seal(&mut self, now: Instant, records: &[u8]) -> Result<Vec<u8>, SecurityError>;

    /// Opens one SEC or LAB frame into its record plaintext.
    fn open(&mut self, now: Instant, frame: &LinkFrame<'_>) -> Result<Vec<u8>, SecurityError>;

    /// Bytes `seal` adds to a plaintext of records (21 for SEC and padded LAB frames).
    fn overhead(&self) -> usize;

    /// `true` for Lab links (LAB frames; never group traffic).
    fn is_lab(&self) -> bool {
        false
    }

    /// Channel binding for IDPROOF (the Noise handshake hash), once established.
    fn channel_binding(&self) -> Option<[u8; 32]> {
        None
    }
}

/// Lab link security: LAB frames with `lab_tag`, padded with a PAD record so the on-air
/// size equals the SEC frame of the same records (§4.3). No handshake.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Plain {
    lab_tag: u16,
}

impl Plain {
    /// Plain security for the Lab code's tag (`mesh_wire::frame::lab_tag`).
    pub const fn new(lab_tag: u16) -> Self {
        Plain { lab_tag }
    }

    pub const fn lab_tag(&self) -> u16 {
        self.lab_tag
    }
}

/// LAB frame overhead: header (3) + PAD record (1 + 1 + 16) = SEC overhead (21).
pub const PLAIN_OVERHEAD: usize = LAB_HEADER + record_len(LAB_PAD_BODY);

impl LinkSecurity for Plain {
    fn on_start(&mut self, _now: Instant, _out: &mut Vec<Vec<u8>>) -> SecurityStep {
        SecurityStep::Established
    }

    fn on_hs(
        &mut self,
        _now: Instant,
        _step: u8,
        _proto_ver: u8,
        _noise_msg: &[u8],
        _out: &mut Vec<Vec<u8>>,
    ) -> SecurityStep {
        // Lab links never handshake: a group peer reached us with a Lab HELLO mix-up.
        SecurityStep::Failed(RejectReason::LabCodeMismatch)
    }

    fn seal(&mut self, _now: Instant, records: &[u8]) -> Result<Vec<u8>, SecurityError> {
        Ok(encode_lab_padded(self.lab_tag, records)?)
    }

    fn open(&mut self, _now: Instant, frame: &LinkFrame<'_>) -> Result<Vec<u8>, SecurityError> {
        match *frame {
            LinkFrame::Lab { lab_tag, records } if lab_tag == self.lab_tag => Ok(records.to_vec()),
            LinkFrame::Lab { .. } => Err(SecurityError::Reject(RejectReason::LabCodeMismatch)),
            _ => Err(SecurityError::WrongFrame),
        }
    }

    fn overhead(&self) -> usize {
        PLAIN_OVERHEAD
    }

    fn is_lab(&self) -> bool {
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mesh_wire::frame::{SEC_OVERHEAD, lab_tag};
    use mesh_wire::record::{Ping, Record, encode_records};

    #[test]
    fn plain_pads_to_sec_size_and_checks_the_tag() {
        let tag = lab_tag(1234);
        let mut p = Plain::new(tag);
        assert_eq!(PLAIN_OVERHEAD, SEC_OVERHEAD);
        let recs = encode_records(&[Record::Ping(Ping { id: 1, t_tx: 2 })]).unwrap();
        let frame = p.seal(Instant(0), &recs).unwrap();
        assert_eq!(frame.len(), recs.len() + SEC_OVERHEAD);
        let parsed = LinkFrame::decode(&frame).unwrap();
        let opened = p.open(Instant(0), &parsed).unwrap();
        assert!(opened.starts_with(&recs));
        let mut other = Plain::new(lab_tag(4321));
        assert_eq!(
            other.open(Instant(0), &parsed),
            Err(SecurityError::Reject(RejectReason::LabCodeMismatch))
        );
        let rej = LinkFrame::Reject {
            reason: RejectReason::Version,
        };
        assert_eq!(p.open(Instant(0), &rej), Err(SecurityError::WrongFrame));
        let mut out = Vec::new();
        assert_eq!(p.on_start(Instant(0), &mut out), SecurityStep::Established);
        assert!(out.is_empty());
    }
}
