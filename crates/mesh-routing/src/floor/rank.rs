//! The strict total rank order shared by relay admission and the origin's `local_admit`
//! (DESIGN §6.2):
//!
//! ```text
//! BurstKey   = (origin, sess, burst)
//! t_origin   = local_rx(P) − age(P) − rel(P)    # P = first packet of the burst THIS node saw
//!              rel(P) = ts × 10 ms for VOICE, elapsed_10ms × 10 ms for TALK (0 for START)
//! h          = SHA-256("cvic/v1/floor" ‖ epoch ‖ origin ‖ sess ‖ burst)[0..4] as u32
//! eff_start  = t_origin + (h / 2^32) × T_TIE
//! rank order = (prio DESC, eff_start ASC, origin ASC)
//! ```
//!
//! Integers in the hash input are little-endian (`epoch` u32, `origin` u16), like every
//! wire field; `h` is the first four digest bytes read little-endian. After `origin`,
//! `sess` and `burst` break the (practically impossible) remaining ties so the order is
//! total even for two bursts of one origin.

use core::cmp::Ordering;

use sha2::{Digest, Sha256};

use crate::types::{Instant, NodeId};

/// Domain separation of the floor hash.
pub const FLOOR_HASH_CONTEXT: &[u8] = b"cvic/v1/floor";

/// `(origin, sess, burst)`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct BurstKey {
    pub origin: NodeId,
    pub sess: u8,
    pub burst: u8,
}

impl BurstKey {
    pub const fn new(origin: NodeId, sess: u8, burst: u8) -> Self {
        BurstKey {
            origin,
            sess,
            burst,
        }
    }
}

/// `h = SHA-256("cvic/v1/floor" ‖ epoch ‖ origin ‖ sess ‖ burst)[0..4]` as u32 (LE).
pub fn floor_hash(epoch: u32, key: BurstKey) -> u32 {
    let mut d = Sha256::new();
    d.update(FLOOR_HASH_CONTEXT);
    d.update(epoch.to_le_bytes());
    d.update(key.origin.to_le_bytes());
    d.update([key.sess, key.burst]);
    let out = d.finalize();
    u32::from_le_bytes([out[0], out[1], out[2], out[3]])
}

/// `(h / 2^32) × T_TIE` in µs.
pub fn smear_us(h: u32, t_tie_us: u64) -> i64 {
    ((u128::from(h) * u128::from(t_tie_us)) >> 32) as i64
}

/// `t_origin = local_rx − age − rel` in µs on this node's clock (may be negative).
pub fn t_origin_us(local_rx: Instant, age_us: u64, rel_us: u64) -> i64 {
    local_rx.us() as i64 - age_us as i64 - rel_us as i64
}

/// A burst's position in the total order. `a < b` (by `Ord`) means `a` ranks **higher**
/// than `b`, so sorting ascending lists the best burst first.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Rank {
    pub prio: u8,
    pub eff_start_us: i64,
    pub key: BurstKey,
}

impl Rank {
    /// The rank of `key` started at `t_origin_us` (local clock) with priority `prio`.
    pub fn new(prio: u8, t_origin_us: i64, key: BurstKey, epoch: u32, t_tie_us: u64) -> Self {
        Rank {
            prio,
            eff_start_us: t_origin_us + smear_us(floor_hash(epoch, key), t_tie_us),
            key,
        }
    }

    /// `true` when `self` ranks strictly above `other`.
    pub fn outranks(&self, other: &Rank) -> bool {
        self < other
    }
}

impl Ord for Rank {
    fn cmp(&self, other: &Self) -> Ordering {
        other
            .prio
            .cmp(&self.prio)
            .then(self.eff_start_us.cmp(&other.eff_start_us))
            .then(self.key.origin.cmp(&other.key.origin))
            .then(self.key.sess.cmp(&other.key.sess))
            .then(self.key.burst.cmp(&other.key.burst))
    }
}

impl PartialOrd for Rank {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    #[test]
    fn hash_and_smear() {
        let k = BurstKey::new(0x0102, 7, 3);
        let h = floor_hash(5, k);
        // Deterministic and key-sensitive.
        assert_eq!(h, floor_hash(5, k));
        assert_ne!(h, floor_hash(6, k));
        assert_ne!(h, floor_hash(5, BurstKey::new(0x0102, 7, 4)));
        // Reference digest computed independently over the same bytes.
        let mut d = Sha256::new();
        d.update(b"cvic/v1/floor\x05\x00\x00\x00\x02\x01\x07\x03");
        let o = d.finalize();
        assert_eq!(h, u32::from_le_bytes([o[0], o[1], o[2], o[3]]));
        assert_eq!(smear_us(0, 150_000), 0);
        assert_eq!(smear_us(u32::MAX, 150_000), 149_999);
        assert_eq!(smear_us(1 << 31, 150_000), 75_000);
    }

    #[test]
    fn order() {
        let a = Rank {
            prio: 1,
            eff_start_us: 100,
            key: BurstKey::new(5, 1, 1),
        };
        let leader = Rank {
            prio: 2,
            eff_start_us: 900,
            ..a
        };
        let later = Rank {
            eff_start_us: 200,
            ..a
        };
        let lower_origin = Rank {
            key: BurstKey::new(4, 1, 1),
            ..a
        };
        assert!(leader.outranks(&a));
        assert!(a.outranks(&later));
        assert!(lower_origin.outranks(&a));
        assert!(!a.outranks(&a));
        assert_eq!(
            t_origin_us(Instant::from_ms(1_000), 20_000, 500_000),
            480_000
        );
        assert_eq!(t_origin_us(Instant::from_ms(10), 20_000, 0), -10_000);
    }

    proptest! {
        /// Strict total order: antisymmetric, transitive, total.
        #[test]
        fn total_order(v in proptest::collection::vec((1u8..=2, -1000i64..1000, any::<u16>(), 1u8..4, any::<u8>()), 3)) {
            let r: Vec<Rank> = v.iter().map(|&(p, e, o, s, b)| Rank { prio: p, eff_start_us: e, key: BurstKey::new(o, s, b) }).collect();
            for x in &r {
                for y in &r {
                    if x.key != y.key || x.prio != y.prio || x.eff_start_us != y.eff_start_us {
                        prop_assert!(x.outranks(y) ^ y.outranks(x) || x == y);
                    }
                    for z in &r {
                        if x.outranks(y) && y.outranks(z) {
                            prop_assert!(x.outranks(z));
                        }
                    }
                }
            }
        }
    }
}
