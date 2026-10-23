//! Caller-supplied monotonic time (DESIGN §3.1): microseconds, never read from the OS.

/// A monotonic instant in microseconds, supplied by the caller. It mirrors mesh-core's
/// `Instant(u64 µs)` (DESIGN §3.2); mesh-core converts with `mesh_lab::Instant(i.0)`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct Instant(pub u64);

impl Instant {
    /// Microseconds since the caller's epoch.
    pub const fn as_micros(self) -> u64 {
        self.0
    }

    /// `self + us` (saturating).
    pub const fn add_micros(self, us: u64) -> Self {
        Instant(self.0.saturating_add(us))
    }

    /// `self + ms` (saturating).
    pub const fn add_millis(self, ms: u64) -> Self {
        Instant(self.0.saturating_add(ms.saturating_mul(1_000)))
    }

    /// `self - earlier` in µs, 0 when `earlier` is later.
    pub const fn since(self, earlier: Instant) -> u64 {
        self.0.saturating_sub(earlier.0)
    }
}

/// µs per second.
pub const US_PER_S: u64 = 1_000_000;
/// µs per millisecond.
pub const US_PER_MS: u64 = 1_000;

/// Seconds (f64) of a µs span, for records.
pub fn secs(us: u64) -> f64 {
    us as f64 / US_PER_S as f64
}

/// Milliseconds (f64) of a signed µs span, for records.
pub fn millis(us: i64) -> f64 {
    us as f64 / US_PER_MS as f64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arithmetic_saturates() {
        let t = Instant(5);
        assert_eq!(t.add_millis(2), Instant(2_005));
        assert_eq!(Instant(u64::MAX).add_micros(1), Instant(u64::MAX));
        assert_eq!(Instant(3).since(Instant(10)), 0);
        assert_eq!(Instant(10).since(Instant(3)), 7);
        assert!((secs(1_500_000) - 1.5).abs() < 1e-12);
        assert!((millis(-2_500) + 2.5).abs() < 1e-12);
    }
}
