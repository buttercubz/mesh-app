//! Wire protocol (contract C2, DESIGN §4, §9.3, §9.4): link framing, LinkFrame, records,
//! the 9-byte MeshPacket header, every packet type, segmentation, control-message body
//! layouts, the HELLO value, advertising payload builders, BLE constants, name encoders,
//! the §4.8 size/cost model and the golden vectors.
//!
//! Every parser is bounded and total: malformed input returns a [`WireError`], it never
//! panics, never recurses and never decompresses. All multi-byte integers are
//! little-endian. Cryptography (sealing, signing) lives in mesh-crypto; this crate only
//! lays out bytes, including the exact messages that get signed.
//!
//! Owned by WP-04 (stub created by WP-01, DESIGN §16.2). Sans-IO crate: the determinism
//! lints below are denied here (clippy.toml holds the list; the workspace allows them for
//! every other crate).
#![deny(clippy::disallowed_methods, clippy::disallowed_types)]
#![forbid(unsafe_code)]

pub mod adv;
pub mod budget;
pub mod codec;
pub mod consts;
pub mod error;
pub mod frame;
pub mod hello;
pub mod link;
pub mod mesh;
pub mod name;
pub mod record;
pub mod serial;
#[cfg(any(feature = "vectors", test))]
pub mod vectors;

pub use error::{Result, WireError};

/// Wire protocol version (DESIGN §4, Appendix A).
pub const PROTOCOL_VERSION: u8 = 1;
