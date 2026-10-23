//! Shared vocabulary: caller-supplied time, link and node identifiers, link kinds.

use mesh_wire::record::NbrFlags;

/// Node identifier (u16 from the per-group `id_hash`, DESIGN §3.2, §8.2).
pub type NodeId = u16;

/// Monotonic time in µs, supplied by the caller (DESIGN §3.1). Never read from a clock.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct Instant(pub u64);

impl Instant {
    /// Time zero.
    pub const ZERO: Instant = Instant(0);

    /// From milliseconds.
    pub const fn from_ms(ms: u64) -> Self {
        Instant(ms * 1_000)
    }

    /// From seconds.
    pub const fn from_secs(s: u64) -> Self {
        Instant(s * 1_000_000)
    }

    /// µs value.
    pub const fn us(self) -> u64 {
        self.0
    }

    /// `self + us` (saturating).
    pub const fn plus_us(self, us: u64) -> Self {
        Instant(self.0.saturating_add(us))
    }

    /// `self + ms` (saturating).
    pub const fn plus_ms(self, ms: u64) -> Self {
        self.plus_us(ms.saturating_mul(1_000))
    }

    /// µs elapsed from `earlier` to `self` (0 when `earlier` is later).
    pub const fn since(self, earlier: Instant) -> u64 {
        self.0.saturating_sub(earlier.0)
    }
}

/// Link identifier, assigned by the platform (DESIGN §3.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct LinkId(pub u32);

/// Transport kind of a link (one link per kind per neighbor, §5.1). L2CAP and GATT are
/// both [`LinkKind::Ble`]: they share the radio, the cap and the airtime budget.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum LinkKind {
    Ble,
    Aware,
    Tunnel,
}

impl LinkKind {
    /// All kinds, in bundle-slot order.
    pub const ALL: [LinkKind; 3] = [LinkKind::Ble, LinkKind::Aware, LinkKind::Tunnel];

    /// Bundle slot index.
    pub const fn index(self) -> usize {
        match self {
            LinkKind::Ble => 0,
            LinkKind::Aware => 1,
            LinkKind::Tunnel => 2,
        }
    }

    /// NBR `flags.kind` value (§4.4).
    pub const fn nbr_kind(self) -> u8 {
        match self {
            LinkKind::Ble => NbrFlags::KIND_BLE,
            LinkKind::Aware => NbrFlags::KIND_AWARE,
            LinkKind::Tunnel => NbrFlags::KIND_TUNNEL,
        }
    }

    /// From the NBR `flags.kind` value (3 is reserved and reads as `None`).
    pub const fn from_nbr_kind(v: u8) -> Option<Self> {
        match v {
            NbrFlags::KIND_BLE => Some(LinkKind::Ble),
            NbrFlags::KIND_AWARE => Some(LinkKind::Aware),
            NbrFlags::KIND_TUNNEL => Some(LinkKind::Tunnel),
            _ => None,
        }
    }

    /// `S_MAX` of the kind (§6.1).
    pub const fn s_max(self) -> u8 {
        match self {
            LinkKind::Ble => crate::consts::S_MAX_BLE,
            LinkKind::Aware => crate::consts::S_MAX_AWARE,
            LinkKind::Tunnel => crate::consts::S_MAX_TUNNEL,
        }
    }

    /// Preference when two kinds to the same neighbor are equally good (more capacity
    /// first): Aware, Tunnel, BLE.
    pub const fn preference(self) -> u8 {
        match self {
            LinkKind::Aware => 3,
            LinkKind::Tunnel => 2,
            LinkKind::Ble => 1,
        }
    }
}

/// Who initiated the link (used by the duplicate-link rule, §5.7).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum LinkRole {
    Initiator,
    Responder,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn instant_math() {
        let t = Instant::from_ms(5);
        assert_eq!(t.us(), 5_000);
        assert_eq!(t.plus_ms(2).since(t), 2_000);
        assert_eq!(t.since(t.plus_ms(1)), 0);
        assert_eq!(Instant::from_secs(2), Instant(2_000_000));
    }

    #[test]
    fn kinds() {
        for k in LinkKind::ALL {
            assert_eq!(LinkKind::from_nbr_kind(k.nbr_kind()), Some(k));
        }
        assert_eq!(LinkKind::from_nbr_kind(3), None);
        assert_eq!(LinkKind::Ble.s_max(), 2);
        assert_eq!(LinkKind::Aware.s_max(), 4);
        assert!(LinkKind::Aware.preference() > LinkKind::Ble.preference());
    }
}
