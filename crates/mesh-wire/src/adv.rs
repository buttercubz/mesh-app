//! Advertising payload builders (DESIGN §9.4) and the pre-connection initiator rules of
//! §9.2. No mesh data, member identity, name or roster data is ever placed in an
//! advertisement; only the rotating `group_tag` and `node_token` (§8.3, §8.7).
//!
//! | Platform / state | Primary ADV | Scan response |
//! |---|---|---|
//! | Android | Flags (3 B) + Complete List of 128-bit UUIDs `MESH_SVC` (18 B) = 21 B | Service Data 128-bit (AD 0x21): `MESH_SVC` + `flags u8 · group_tag[4] · node_token[4]` = 27 B |
//! | iPhone foreground | service UUID (set by CoreBluetooth) | local name: 6 base32 chars of `group_tag[0..2] ‖ node_token top 14 bits` (8 B AD) |

use core::cmp::Ordering;

use crate::codec::Reader;
use crate::consts::MESH_SVC;
use crate::error::{Result, WireError};

/// AD type: Flags.
pub const AD_FLAGS: u8 = 0x01;
/// AD type: Complete List of 128-bit Service UUIDs.
pub const AD_UUID128_COMPLETE: u8 = 0x07;
/// AD type: Complete Local Name.
pub const AD_LOCAL_NAME: u8 = 0x09;
/// AD type: Service Data - 128-bit UUID.
pub const AD_SERVICE_DATA_128: u8 = 0x21;
/// Flags value: LE General Discoverable, BR/EDR not supported.
pub const FLAGS_LE_GENERAL_NO_BREDR: u8 = 0x06;
/// Legacy advertising payload limit.
pub const LEGACY_ADV_MAX: usize = 31;
/// Characters in the iPhone local name.
pub const LOCAL_NAME_LEN: usize = 6;
/// Service data after the UUID: `flags u8 · group_tag[4] · node_token[4]`.
pub const SERVICE_DATA_LEN: usize = 9;
/// RFC 4648 base32 alphabet (uppercase, no padding).
pub const BASE32_ALPHABET: &[u8; 32] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ234567";

/// Android primary advertising data (21 B): Flags + the 128-bit service UUID list.
pub fn android_adv_data() -> Vec<u8> {
    let mut o = Vec::with_capacity(21);
    o.extend_from_slice(&[2, AD_FLAGS, FLAGS_LE_GENERAL_NO_BREDR]);
    o.push(17);
    o.push(AD_UUID128_COMPLETE);
    o.extend_from_slice(&MESH_SVC.le_bytes());
    o
}

/// Scan-response service data flags: b0 android, b1 lab.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Hash)]
pub struct AdvFlags(pub u8);

impl AdvFlags {
    pub const ANDROID: u8 = 1 << 0;
    pub const LAB: u8 = 1 << 1;

    pub const fn new(android: bool, lab: bool) -> Self {
        AdvFlags((android as u8) | ((lab as u8) << 1))
    }
    pub const fn android(self) -> bool {
        self.0 & Self::ANDROID != 0
    }
    pub const fn lab(self) -> bool {
        self.0 & Self::LAB != 0
    }
}

/// The 9 bytes that follow `MESH_SVC` in the Android scan response.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ServiceData {
    pub flags: AdvFlags,
    pub group_tag: [u8; 4],
    pub node_token: [u8; 4],
}

impl ServiceData {
    /// The 9-byte value (what Android's `ScanRecord.getServiceData(MESH_SVC)` returns).
    pub fn encode(&self) -> [u8; SERVICE_DATA_LEN] {
        let mut o = [0u8; SERVICE_DATA_LEN];
        o[0] = self.flags.0;
        o[1..5].copy_from_slice(&self.group_tag);
        o[5..9].copy_from_slice(&self.node_token);
        o
    }

    /// Parses the 9-byte value; trailing bytes are refused.
    pub fn decode(b: &[u8]) -> Result<Self> {
        let mut r = Reader::new(b);
        let v = ServiceData {
            flags: AdvFlags(r.u8("adv flags")?),
            group_tag: r.array("adv group_tag")?,
            node_token: r.array("adv node_token")?,
        };
        r.finish("adv service data")?;
        Ok(v)
    }

    /// The whole scan-response AD structure (27 B).
    pub fn scan_response(&self) -> Vec<u8> {
        let mut o = Vec::with_capacity(27);
        o.push((1 + 16 + SERVICE_DATA_LEN) as u8);
        o.push(AD_SERVICE_DATA_128);
        o.extend_from_slice(&MESH_SVC.le_bytes());
        o.extend_from_slice(&self.encode());
        o
    }

    /// Finds our service data in raw advertising/scan-response bytes (a sequence of
    /// `len · type · data` AD structures). `Ok(None)` when absent; an error on a
    /// malformed AD sequence.
    pub fn find_in(ad: &[u8]) -> Result<Option<Self>> {
        for s in ad_structures(ad) {
            let (ad_type, data) = s?;
            if ad_type == AD_SERVICE_DATA_128
                && data.len() >= 16
                && data[..16] == MESH_SVC.le_bytes()
            {
                return ServiceData::decode(&data[16..]).map(Some);
            }
        }
        Ok(None)
    }
}

/// Iterates `(ad_type, data)` over raw AD bytes. A zero length byte ends the payload
/// (the rest is padding), as in the Core specification.
pub fn ad_structures(ad: &[u8]) -> impl Iterator<Item = Result<(u8, &[u8])>> {
    let mut pos = 0usize;
    let mut failed = false;
    core::iter::from_fn(move || {
        if failed || pos >= ad.len() {
            return None;
        }
        let len = usize::from(ad[pos]);
        if len == 0 {
            pos = ad.len();
            return None;
        }
        let Some(s) = ad.get(pos + 1..pos + 1 + len) else {
            failed = true;
            return Some(Err(WireError::Truncated {
                what: "AD structure",
            }));
        };
        pos += 1 + len;
        Some(Ok((s[0], &s[1..])))
    })
}

/// The top 14 bits of a node token (big-endian reading of its first two bytes).
pub const fn token_prefix14(node_token: &[u8; 4]) -> u16 {
    ((node_token[0] as u16) << 6) | ((node_token[1] as u16) >> 2)
}

/// The iPhone local name: 6 base32 characters of the 30-bit value
/// `group_tag[0] ‖ group_tag[1] ‖ token_prefix14(node_token)` (MSB first).
pub fn ios_local_name(group_tag: &[u8; 4], node_token: &[u8; 4]) -> String {
    let v: u32 = (u32::from(group_tag[0]) << 22)
        | (u32::from(group_tag[1]) << 14)
        | u32::from(token_prefix14(node_token));
    (0..LOCAL_NAME_LEN)
        .map(|i| {
            let shift = 5 * (LOCAL_NAME_LEN - 1 - i);
            char::from(BASE32_ALPHABET[((v >> shift) & 0x1F) as usize])
        })
        .collect()
}

/// The Complete Local Name AD structure carrying the iPhone name (8 B).
pub fn ios_local_name_ad(group_tag: &[u8; 4], node_token: &[u8; 4]) -> Vec<u8> {
    let name = ios_local_name(group_tag, node_token);
    let mut o = Vec::with_capacity(2 + LOCAL_NAME_LEN);
    o.push((1 + LOCAL_NAME_LEN) as u8);
    o.push(AD_LOCAL_NAME);
    o.extend_from_slice(name.as_bytes());
    o
}

/// What a scanner learns from an iPhone local name.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct LocalNameInfo {
    /// `group_tag[0..2]`.
    pub group_tag_prefix: [u8; 2],
    /// Top 14 bits of the advertiser's node token.
    pub token_prefix14: u16,
}

impl LocalNameInfo {
    /// Parses a 6-character base32 local name (uppercase only, as built).
    pub fn parse(name: &str) -> Result<Self> {
        let b = name.as_bytes();
        if b.len() != LOCAL_NAME_LEN {
            return Err(WireError::BadValue {
                what: "local name length",
                value: b.len() as u32,
            });
        }
        let mut v: u32 = 0;
        for &c in b {
            let Some(d) = BASE32_ALPHABET.iter().position(|&a| a == c) else {
                return Err(WireError::BadValue {
                    what: "local name char",
                    value: u32::from(c),
                });
            };
            v = (v << 5) | d as u32;
        }
        Ok(LocalNameInfo {
            group_tag_prefix: [(v >> 22) as u8, (v >> 14) as u8],
            token_prefix14: (v & 0x3FFF) as u16,
        })
    }

    /// `true` when the name's group-tag prefix matches a full `group_tag`.
    pub fn matches_group_tag(&self, group_tag: &[u8; 4]) -> bool {
        self.group_tag_prefix == [group_tag[0], group_tag[1]]
    }
}

/// iPhone ↔ iPhone foreground rule (§9.2): the side with the lower 14-bit token prefix
/// initiates. `Some(true)`: I initiate; `Some(false)`: the peer does; `None` on equal
/// prefixes (either side may initiate; the duplicate is resolved after IDPROOF).
pub fn ios_pair_initiates(my_token: &[u8; 4], peer_prefix14: u16) -> Option<bool> {
    match token_prefix14(my_token).cmp(&peer_prefix14) {
        Ordering::Less => Some(true),
        Ordering::Greater => Some(false),
        Ordering::Equal => None,
    }
}

/// Android ↔ Android rule (§9.2): the side with the lower `node_token` (compared as a
/// big-endian number) initiates; `None` when equal.
pub fn android_pair_initiates(my_token: &[u8; 4], peer_token: &[u8; 4]) -> Option<bool> {
    match my_token.cmp(peer_token) {
        Ordering::Less => Some(true),
        Ordering::Greater => Some(false),
        Ordering::Equal => None,
    }
}

/// After IDPROOF, a duplicate link is resolved in favour of the link whose initiator has
/// the lower `node_id` (§9.2). Returns `true` when the link initiated by
/// `initiator_node` should be kept against the one initiated by `other_initiator`.
pub fn keep_duplicate(initiator_node: u16, other_initiator: u16) -> bool {
    initiator_node <= other_initiator
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codec::to_hex;
    use proptest::prelude::*;

    #[test]
    fn android_payload_sizes() {
        let adv = android_adv_data();
        assert_eq!(adv.len(), 21);
        assert!(adv.len() <= LEGACY_ADV_MAX);
        assert_eq!(&adv[..5], &[2, 1, 6, 17, 7]);
        let sd = ServiceData {
            flags: AdvFlags::new(true, false),
            group_tag: [1, 2, 3, 4],
            node_token: [5, 6, 7, 8],
        };
        let sr = sd.scan_response();
        assert_eq!(sr.len(), 27);
        assert_eq!(sr[0], 26);
        assert_eq!(to_hex(&sr[18..]), "010102030405060708");
        assert_eq!(ServiceData::find_in(&sr).unwrap(), Some(sd));
        let mut both = adv.clone();
        both.extend_from_slice(&sr);
        assert_eq!(ServiceData::find_in(&both).unwrap(), Some(sd));
        assert_eq!(ServiceData::find_in(&adv).unwrap(), None);
        assert!(ServiceData::find_in(&sr[..20]).is_err());
    }

    #[test]
    fn local_name_carries_14_bit_prefix() {
        let gt = [0xA1, 0xB2, 0x00, 0x00];
        let nt = [0xFF, 0xFF, 0x12, 0x34];
        let name = ios_local_name(&gt, &nt);
        assert_eq!(name.len(), 6);
        let info = LocalNameInfo::parse(&name).unwrap();
        assert_eq!(info.group_tag_prefix, [0xA1, 0xB2]);
        assert_eq!(info.token_prefix14, 0x3FFF);
        assert!(info.matches_group_tag(&gt));
        assert_eq!(ios_local_name_ad(&gt, &nt).len(), 8);
        assert_eq!(ios_local_name(&[0; 4], &[0; 4]), "AAAAAA");
        assert_eq!(ios_local_name(&[0xFF; 4], &[0xFF; 4]), "777777");
        assert!(LocalNameInfo::parse("AAAAA").is_err());
        assert!(LocalNameInfo::parse("AAAAA1").is_err());
        assert!(LocalNameInfo::parse("aaaaaa").is_err());
    }

    #[test]
    fn initiator_rules() {
        let me = [0x00, 0x04, 0, 0];
        assert_eq!(token_prefix14(&me), 1);
        assert_eq!(ios_pair_initiates(&me, 2), Some(true));
        assert_eq!(ios_pair_initiates(&me, 0), Some(false));
        assert_eq!(ios_pair_initiates(&[0x00, 0x07, 9, 9], 1), None);
        assert_eq!(
            android_pair_initiates(&[0, 0, 0, 1], &[0, 0, 0, 2]),
            Some(true)
        );
        assert_eq!(
            android_pair_initiates(&[1, 0, 0, 0], &[0, 0, 0, 2]),
            Some(false)
        );
        assert_eq!(android_pair_initiates(&[1; 4], &[1; 4]), None);
        assert!(keep_duplicate(3, 7));
        assert!(!keep_duplicate(7, 3));
    }

    proptest! {
        #[test]
        fn local_name_round_trip(gt in any::<[u8; 4]>(), nt in any::<[u8; 4]>()) {
            let info = LocalNameInfo::parse(&ios_local_name(&gt, &nt)).unwrap();
            prop_assert_eq!(info.group_tag_prefix, [gt[0], gt[1]]);
            prop_assert_eq!(info.token_prefix14, token_prefix14(&nt));
        }

        #[test]
        fn ad_parsing_never_panics(b in proptest::collection::vec(any::<u8>(), 0..64)) {
            let _ = ServiceData::find_in(&b);
            let _ = ServiceData::decode(&b);
        }
    }
}
