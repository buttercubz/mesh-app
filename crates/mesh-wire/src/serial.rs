//! Serial-number arithmetic (RFC 1982) for truncated 16-bit fields.
//!
//! `roster_version` and `epoch` are u32 wherever a message is authoritative; PRESENCE
//! `roster_ver16`/`epoch16` and HELLO `epoch_lo` carry `value mod 2¹⁶` and are compared
//! with serial arithmetic (DESIGN §4.6.1). The mesh `seq` is a u16 serial number too
//! (§4.5).

use core::cmp::Ordering;

/// The low 16 bits of an authoritative u32 version (`value mod 2¹⁶`).
pub const fn truncate16(value: u32) -> u16 {
    (value & 0xFFFF) as u16
}

/// Signed distance from `a` to `b` in serial space: positive when `b` is ahead of `a`.
/// The result is in `-32768..=32767`; a distance of exactly 2¹⁵ reads as `-32768`
/// (undefined in RFC 1982; [`serial16_cmp`] reports it as `None`).
pub const fn serial16_diff(a: u16, b: u16) -> i16 {
    b.wrapping_sub(a) as i16
}

/// RFC 1982 comparison of `a` with `b`; `None` when the two are exactly 2¹⁵ apart.
pub fn serial16_cmp(a: u16, b: u16) -> Option<Ordering> {
    let d = b.wrapping_sub(a);
    match d {
        0 => Some(Ordering::Equal),
        1..=0x7FFF => Some(Ordering::Less),
        0x8000 => None,
        _ => Some(Ordering::Greater),
    }
}

/// `true` when `a` is strictly before `b` in serial space.
pub fn serial16_lt(a: u16, b: u16) -> bool {
    serial16_cmp(a, b) == Some(Ordering::Less)
}

/// `true` when the peer's truncated value `theirs16` is behind my authoritative `mine`
/// (anti-entropy push trigger, DESIGN §8.9 rule 3).
pub fn peer_behind(mine: u32, theirs16: u16) -> bool {
    serial16_lt(theirs16, truncate16(mine))
}

/// How many steps the peer's truncated value is ahead of mine (negative when behind).
/// Used for the HELLO `epoch_lo` "1-4 epochs ahead" speculative-link rule (§9.3).
pub fn ahead_by(mine: u32, theirs16: u16) -> i16 {
    serial16_diff(truncate16(mine), theirs16)
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    #[test]
    fn wraps_like_rfc1982() {
        assert_eq!(serial16_cmp(0xFFFF, 0x0000), Some(Ordering::Less));
        assert_eq!(serial16_cmp(0x0000, 0xFFFF), Some(Ordering::Greater));
        assert_eq!(serial16_cmp(5, 5), Some(Ordering::Equal));
        assert_eq!(serial16_cmp(0, 0x8000), None);
        assert!(serial16_lt(0xFFFE, 3));
        assert_eq!(serial16_diff(0xFFFE, 3), 5);
        assert_eq!(serial16_diff(3, 0xFFFE), -5);
    }

    #[test]
    fn truncated_versions() {
        assert_eq!(truncate16(0x0001_0002), 2);
        // epoch 65 537 (low 16 = 1) vs a peer at 65 535 (0xFFFF): the peer is behind.
        assert!(peer_behind(65_537, 0xFFFF));
        assert!(!peer_behind(65_537, 1));
        assert!(!peer_behind(65_537, 2));
        assert_eq!(ahead_by(0x0001_FFFF, 0x0003), 4);
        assert_eq!(ahead_by(10, 9), -1);
    }

    proptest! {
        #[test]
        fn small_steps_compare_correctly(base in any::<u32>(), step in 1u32..0x7FFF) {
            let a = truncate16(base);
            let b = truncate16(base.wrapping_add(step));
            prop_assert!(serial16_lt(a, b));
            prop_assert!(!serial16_lt(b, a));
            prop_assert_eq!(i32::from(serial16_diff(a, b)), step as i32);
        }
    }
}
