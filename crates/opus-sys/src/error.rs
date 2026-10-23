//! libopus error codes as a Rust error type.

use core::ffi::CStr;
use core::fmt;

use crate::ffi;

/// A negative libopus return code.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Error {
    code: i32,
}

/// Result alias for this crate.
pub type Result<T> = core::result::Result<T, Error>;

impl Error {
    /// Wraps a libopus error code (expected to be negative).
    pub const fn from_code(code: i32) -> Self {
        Self { code }
    }

    /// The raw libopus code (`OPUS_BAD_ARG` = -1 ... `OPUS_ALLOC_FAIL` = -7).
    pub const fn code(self) -> i32 {
        self.code
    }

    /// Upstream's description (`opus_strerror`).
    pub fn message(self) -> &'static str {
        // SAFETY: opus_strerror returns a static NUL-terminated string for any input.
        let s = unsafe { CStr::from_ptr(ffi::opus_strerror(self.code)) };
        s.to_str().unwrap_or("unknown error")
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "libopus error {}: {}", self.code, self.message())
    }
}

impl std::error::Error for Error {}

/// `OPUS_OK` → `Ok(())`, anything else → `Err`.
pub(crate) fn check(code: i32) -> Result<()> {
    if code == ffi::OPUS_OK {
        Ok(())
    } else {
        Err(Error::from_code(code))
    }
}

/// Non-negative counts (bytes, samples) → `Ok(count)`, negative → `Err`.
pub(crate) fn check_count(n: i32) -> Result<usize> {
    usize::try_from(n).map_err(|_| Error::from_code(n))
}
