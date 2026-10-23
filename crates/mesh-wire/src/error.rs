//! Error type shared by every codec in this crate.
//!
//! Every parser is bounded and total: malformed input yields a [`WireError`], never a
//! panic (DESIGN §4: "every parser is bounded").

use core::fmt;

/// Why a decode or encode was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WireError {
    /// The input ended before the field `what` was complete.
    Truncated { what: &'static str },
    /// Bytes were left over after a fixed-size structure (`what`).
    TrailingBytes { what: &'static str },
    /// A length field or a buffer exceeds its protocol bound.
    TooLong {
        what: &'static str,
        len: usize,
        max: usize,
    },
    /// A length is below its protocol minimum (for example an empty frame).
    TooShort {
        what: &'static str,
        len: usize,
        min: usize,
    },
    /// A field holds a value the protocol does not define.
    BadValue { what: &'static str, value: u32 },
    /// The mesh header carries a version other than 1.
    Version { got: u8 },
    /// A LEB128 length is not in its shortest form or exceeds two bytes.
    BadLeb128,
    /// A frame holds more than [`crate::record::MAX_RECORDS_PER_FRAME`] records.
    TooManyRecords,
    /// A name is not valid UTF-8.
    BadUtf8,
    /// The writer has no room left for the item (`what`).
    NoSpace { what: &'static str },
}

impl fmt::Display for WireError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            WireError::Truncated { what } => write!(f, "truncated input in {what}"),
            WireError::TrailingBytes { what } => write!(f, "trailing bytes after {what}"),
            WireError::TooLong { what, len, max } => {
                write!(f, "{what} too long: {len} > {max}")
            }
            WireError::TooShort { what, len, min } => {
                write!(f, "{what} too short: {len} < {min}")
            }
            WireError::BadValue { what, value } => write!(f, "invalid {what}: {value}"),
            WireError::Version { got } => write!(f, "unsupported mesh header version {got}"),
            WireError::BadLeb128 => f.write_str("invalid LEB128 record length"),
            WireError::TooManyRecords => f.write_str("more than 32 records in one frame"),
            WireError::BadUtf8 => f.write_str("name is not valid UTF-8"),
            WireError::NoSpace { what } => write!(f, "no space left for {what}"),
        }
    }
}

impl std::error::Error for WireError {}

/// Result alias for this crate.
pub type Result<T> = core::result::Result<T, WireError>;
