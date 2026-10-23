//! Error type of mesh-crypto. Every failure is a value; nothing in this crate panics on
//! peer-controlled input.

use core::fmt;

use mesh_wire::WireError;
use mesh_wire::frame::RejectReason;

/// Result alias of this crate.
pub type Result<T> = core::result::Result<T, CryptoError>;

/// Why a cryptographic operation failed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CryptoError {
    /// The bytes do not parse (bounded wire codec error).
    Wire(WireError),
    /// An Ed25519 signature did not verify (or the public key is not a valid point).
    BadSignature,
    /// A public key is malformed, or an X25519 exchange produced the all-zero output
    /// (low-order point).
    BadKey,
    /// An AEAD tag did not verify. No state was changed.
    Decrypt,
    /// The frame counter was already accepted or is older than the replay window.
    Replay,
    /// The Noise handshake failed (wrong PSK/epoch, wrong group, corrupted message).
    Handshake,
    /// The handshake message arrived out of order or for the wrong role.
    HandshakeOrder,
    /// The initiator chose a protocol version this node does not speak.
    Version,
    /// A call that needs an established link was made before the handshake finished.
    NotEstablished,
    /// A frame of the wrong `lf_type` was given to the session.
    WrongFrameType,
    /// The 31-bit counter space of a key phase is exhausted (never reached in practice:
    /// the key updates at 2³⁰).
    CounterExhausted,
    /// The invite blob or URL is malformed.
    BadInvite(&'static str),
    /// A signed control message is older than (or equal to) the stored state, or does
    /// not apply to this node (monotonic acceptance, DESIGN §4.7).
    Stale(&'static str),
    /// A REKEY has no sealed entry for this node.
    NoEntry,
    /// The message is for another group or epoch.
    WrongGroup,
    /// The peer's identity is on the deny list.
    Denied,
    /// The peer's `node_id` collides with another known identity.
    NodeIdCollision,
    /// The peer's identity does not match the one the Noise handshake authenticated.
    IdentityMismatch,
    /// A message is structurally valid but breaks a protocol rule.
    Invalid(&'static str),
}

impl CryptoError {
    /// The REJECT/CLOSE reason a link should send for this failure, when it is a
    /// link-level refusal (DESIGN §4.3).
    pub fn reject_reason(&self) -> Option<RejectReason> {
        Some(match self {
            CryptoError::Handshake | CryptoError::HandshakeOrder => RejectReason::HandshakeFailure,
            CryptoError::WrongGroup => RejectReason::GroupEpochMismatch,
            CryptoError::Version => RejectReason::Version,
            CryptoError::Denied => RejectReason::MemberRemovedOrDenied,
            CryptoError::NodeIdCollision => RejectReason::NodeIdCollision,
            CryptoError::IdentityMismatch | CryptoError::BadSignature => {
                RejectReason::HandshakeFailure
            }
            _ => return None,
        })
    }
}

impl From<WireError> for CryptoError {
    fn from(e: WireError) -> Self {
        CryptoError::Wire(e)
    }
}

impl fmt::Display for CryptoError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CryptoError::Wire(e) => write!(f, "wire: {e}"),
            CryptoError::BadSignature => f.write_str("signature does not verify"),
            CryptoError::BadKey => f.write_str("invalid public key"),
            CryptoError::Decrypt => f.write_str("AEAD tag does not verify"),
            CryptoError::Replay => f.write_str("replayed or too old frame counter"),
            CryptoError::Handshake => f.write_str("Noise handshake failed"),
            CryptoError::HandshakeOrder => f.write_str("unexpected handshake message"),
            CryptoError::Version => f.write_str("unsupported protocol version"),
            CryptoError::NotEstablished => f.write_str("link not established"),
            CryptoError::WrongFrameType => f.write_str("wrong link frame type"),
            CryptoError::CounterExhausted => f.write_str("frame counter exhausted"),
            CryptoError::BadInvite(w) => write!(f, "bad invite: {w}"),
            CryptoError::Stale(w) => write!(f, "not newer than stored state: {w}"),
            CryptoError::NoEntry => f.write_str("no sealed entry for this node"),
            CryptoError::WrongGroup => f.write_str("wrong group or epoch"),
            CryptoError::Denied => f.write_str("identity is denied"),
            CryptoError::NodeIdCollision => f.write_str("node_id collision"),
            CryptoError::IdentityMismatch => f.write_str("identity does not match the handshake"),
            CryptoError::Invalid(w) => write!(f, "invalid: {w}"),
        }
    }
}

impl core::error::Error for CryptoError {}
