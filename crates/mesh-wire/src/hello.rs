//! The 16-byte `HELLO` characteristic value (DESIGN §9.3):
//! `magic u8 = 0xC1 · proto_min u8 · proto_max u8 · flags u8 · psm u16 (0 ⇒ GATT only) ·
//! group_tag[4] (Lab: lab_tag ‖ 0x0000) · node_token[4] · epoch_lo u16`.
//!
//! The core builds it (`SetHello{bytes}`) and parses it in `on_hello_read`; the native
//! layers only move the bytes. `epoch_lo` is `epoch mod 2¹⁶`, compared with serial
//! arithmetic (§4.6.1).

use crate::codec::{PutLe, Reader};
use crate::error::{Result, WireError};
use crate::serial::{ahead_by, truncate16};

/// HELLO `flags`: b0 android, b1 lab, b2 accepting, b3 aware-capable, b4 leader;
/// b7-5 reserved (sent as 0, ignored and preserved on receive).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Hash)]
pub struct HelloFlags(pub u8);

impl HelloFlags {
    pub const ANDROID: u8 = 1 << 0;
    pub const LAB: u8 = 1 << 1;
    pub const ACCEPTING: u8 = 1 << 2;
    pub const AWARE: u8 = 1 << 3;
    pub const LEADER: u8 = 1 << 4;

    pub const fn new(android: bool, lab: bool, accepting: bool, aware: bool, leader: bool) -> Self {
        HelloFlags(
            (android as u8)
                | ((lab as u8) << 1)
                | ((accepting as u8) << 2)
                | ((aware as u8) << 3)
                | ((leader as u8) << 4),
        )
    }
    pub const fn android(self) -> bool {
        self.0 & Self::ANDROID != 0
    }
    pub const fn lab(self) -> bool {
        self.0 & Self::LAB != 0
    }
    pub const fn accepting(self) -> bool {
        self.0 & Self::ACCEPTING != 0
    }
    pub const fn aware(self) -> bool {
        self.0 & Self::AWARE != 0
    }
    pub const fn leader(self) -> bool {
        self.0 & Self::LEADER != 0
    }
}

/// The HELLO value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Hello {
    pub proto_min: u8,
    pub proto_max: u8,
    pub flags: HelloFlags,
    /// L2CAP PSM; 0 means "GATT data path only".
    pub psm: u16,
    pub group_tag: [u8; 4],
    pub node_token: [u8; 4],
    pub epoch_lo: u16,
}

impl Hello {
    /// Encoded size.
    pub const LEN: usize = 16;
    /// First byte.
    pub const MAGIC: u8 = 0xC1;

    /// A group-mode HELLO for protocol v1 with `epoch_lo = epoch mod 2¹⁶`.
    pub fn group(
        flags: HelloFlags,
        psm: u16,
        group_tag: [u8; 4],
        node_token: [u8; 4],
        epoch: u32,
    ) -> Self {
        Hello {
            proto_min: crate::PROTOCOL_VERSION,
            proto_max: crate::PROTOCOL_VERSION,
            flags: HelloFlags(flags.0 & !HelloFlags::LAB),
            psm,
            group_tag,
            node_token,
            epoch_lo: truncate16(epoch),
        }
    }

    /// A Lab HELLO: `flags.lab = 1`, `group_tag = lab_tag (LE) ‖ 0x0000`, epoch_lo 0.
    pub fn lab(flags: HelloFlags, psm: u16, lab_tag: u16, node_token: [u8; 4]) -> Self {
        Hello {
            proto_min: crate::PROTOCOL_VERSION,
            proto_max: crate::PROTOCOL_VERSION,
            flags: HelloFlags(flags.0 | HelloFlags::LAB),
            psm,
            group_tag: lab_group_tag(lab_tag),
            node_token,
            epoch_lo: 0,
        }
    }

    /// Serializes the 16-byte value.
    pub fn encode(&self) -> Result<[u8; 16]> {
        if self.proto_min > self.proto_max {
            return Err(WireError::BadValue {
                what: "HELLO proto range",
                value: u32::from(self.proto_min),
            });
        }
        let mut o = Vec::with_capacity(Self::LEN);
        o.put_u8(Self::MAGIC);
        o.put_u8(self.proto_min);
        o.put_u8(self.proto_max);
        o.put_u8(self.flags.0);
        o.put_u16(self.psm);
        o.put(&self.group_tag);
        o.put(&self.node_token);
        o.put_u16(self.epoch_lo);
        let mut out = [0u8; 16];
        out.copy_from_slice(&o);
        Ok(out)
    }

    /// Parses a HELLO value. Exactly 16 bytes; the magic and a sane version range are
    /// required.
    pub fn decode(b: &[u8]) -> Result<Self> {
        let mut r = Reader::new(b);
        let magic = r.u8("HELLO magic")?;
        if magic != Self::MAGIC {
            return Err(WireError::BadValue {
                what: "HELLO magic",
                value: u32::from(magic),
            });
        }
        let h = Hello {
            proto_min: r.u8("HELLO proto_min")?,
            proto_max: r.u8("HELLO proto_max")?,
            flags: HelloFlags(r.u8("HELLO flags")?),
            psm: r.u16("HELLO psm")?,
            group_tag: r.array("HELLO group_tag")?,
            node_token: r.array("HELLO node_token")?,
            epoch_lo: r.u16("HELLO epoch_lo")?,
        };
        r.finish("HELLO")?;
        if h.proto_min > h.proto_max {
            return Err(WireError::BadValue {
                what: "HELLO proto range",
                value: u32::from(h.proto_min),
            });
        }
        Ok(h)
    }

    /// The highest version both sides support (the initiator writes it in HS
    /// `proto_ver`); `None` means `REJECT{1}` (§4.9).
    pub fn common_version(&self, my_min: u8, my_max: u8) -> Option<u8> {
        let hi = self.proto_max.min(my_max);
        let lo = self.proto_min.max(my_min);
        (lo <= hi).then_some(hi)
    }

    /// `true` when the data path is GATT only (`psm = 0`).
    pub fn gatt_only(&self) -> bool {
        self.psm == 0
    }

    /// How many epochs the peer is ahead of `my_epoch` (negative when behind), by
    /// serial arithmetic on `epoch_lo` (§9.3 speculative-link rule: 1-4 ahead).
    pub fn epoch_ahead_by(&self, my_epoch: u32) -> i16 {
        ahead_by(my_epoch, self.epoch_lo)
    }

    /// `true` when the speculative-link rule applies: the peer is 1-4 epochs ahead.
    pub fn speculative_candidate(&self, my_epoch: u32) -> bool {
        (1..=4).contains(&self.epoch_ahead_by(my_epoch))
    }

    /// The Lab tag carried in `group_tag`, when this is a Lab HELLO.
    pub fn lab_tag(&self) -> Option<u16> {
        (self.flags.lab() && self.group_tag[2..] == [0, 0])
            .then(|| u16::from_le_bytes([self.group_tag[0], self.group_tag[1]]))
    }
}

/// `group_tag` of a Lab HELLO: `lab_tag` (u16 LE) ‖ `0x0000`.
pub const fn lab_group_tag(lab_tag: u16) -> [u8; 4] {
    let t = lab_tag.to_le_bytes();
    [t[0], t[1], 0, 0]
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn sample() -> Hello {
        Hello::group(
            HelloFlags::new(true, false, true, false, true),
            0x0081,
            [0xA1, 0xB2, 0xC3, 0xD4],
            [0x11, 0x22, 0x33, 0x44],
            0x0002_0005,
        )
    }

    #[test]
    fn layout_is_16_bytes() {
        let b = sample().encode().unwrap();
        assert_eq!(
            crate::codec::to_hex(&b),
            "c1010115_8100_a1b2c3d4_11223344_0500".replace('_', "")
        );
        assert_eq!(Hello::decode(&b).unwrap(), sample());
    }

    #[test]
    fn rejects_bad_values() {
        let mut b = sample().encode().unwrap();
        assert!(Hello::decode(&b[..15]).is_err());
        let mut long = b.to_vec();
        long.push(0);
        assert!(Hello::decode(&long).is_err());
        b[0] = 0xC2;
        assert!(Hello::decode(&b).is_err());
        let mut b = sample().encode().unwrap();
        b[1] = 3;
        b[2] = 2;
        assert!(Hello::decode(&b).is_err());
    }

    #[test]
    fn version_and_epoch_rules() {
        let mut h = sample();
        h.proto_min = 1;
        h.proto_max = 3;
        assert_eq!(h.common_version(1, 2), Some(2));
        assert_eq!(h.common_version(4, 5), None);
        // Peer epoch_lo 0x0005 vs my epoch 0x0001_FFFF (low 0xFFFF): 6 ahead.
        assert_eq!(h.epoch_ahead_by(0x0001_FFFF), 6);
        assert!(!h.speculative_candidate(0x0001_FFFF));
        assert!(h.speculative_candidate(0x0002_0002));
        assert!(!h.speculative_candidate(0x0002_0005));
    }

    #[test]
    fn lab_hello() {
        let h = Hello::lab(
            HelloFlags::new(false, false, true, false, false),
            0,
            0xBEEF,
            [1, 2, 3, 4],
        );
        assert!(h.flags.lab());
        assert!(h.gatt_only());
        assert_eq!(h.group_tag, [0xEF, 0xBE, 0, 0]);
        assert_eq!(h.lab_tag(), Some(0xBEEF));
        assert_eq!(sample().lab_tag(), None);
    }

    proptest! {
        #[test]
        fn round_trip(b in proptest::array::uniform16(any::<u8>())) {
            let mut b = b;
            b[0] = Hello::MAGIC;
            if let Ok(h) = Hello::decode(&b) {
                prop_assert_eq!(h.encode().unwrap(), b);
            }
        }

        #[test]
        fn never_panics(b in proptest::collection::vec(any::<u8>(), 0..40)) {
            let _ = Hello::decode(&b);
        }
    }
}
