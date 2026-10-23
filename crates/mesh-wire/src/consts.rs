//! BLE constants (DESIGN §9.3-§9.5, Appendix A): the single source of truth for GATT
//! UUIDs and radio timings. mesh-ffi exports them through `ble_constants()` so Swift and
//! Kotlin never hard-code a UUID or a timing.

use crate::link::{ATT_MTU_MAX, MAX_DATAGRAM_FRAME, MAX_STREAM_FRAME};

/// A 128-bit UUID in canonical (big-endian, RFC 4122 string) byte order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Uuid128(pub [u8; 16]);

impl Uuid128 {
    /// Parses `xxxxxxxx-xxxx-xxxx-xxxx-xxxxxxxxxxxx` (hex digits in either case).
    pub const fn parse(s: &str) -> Option<Self> {
        let b = s.as_bytes();
        if b.len() != 36 {
            return None;
        }
        let mut out = [0u8; 16];
        let (mut i, mut o) = (0usize, 0usize);
        while i < 36 {
            if i == 8 || i == 13 || i == 18 || i == 23 {
                if b[i] != b'-' {
                    return None;
                }
                i += 1;
                continue;
            }
            let (Some(hi), Some(lo)) = (nibble(b[i]), nibble(b[i + 1])) else {
                return None;
            };
            out[o] = (hi << 4) | lo;
            o += 1;
            i += 2;
        }
        Some(Uuid128(out))
    }

    /// Canonical byte order (as written in the string).
    pub const fn bytes(&self) -> [u8; 16] {
        self.0
    }

    /// Byte order used inside BLE AD structures and ATT PDUs (little-endian, reversed).
    pub const fn le_bytes(&self) -> [u8; 16] {
        let mut out = [0u8; 16];
        let mut i = 0;
        while i < 16 {
            out[i] = self.0[15 - i];
            i += 1;
        }
        out
    }

    /// Uppercase canonical string.
    pub fn to_canonical(&self) -> String {
        const D: &[u8; 16] = b"0123456789ABCDEF";
        let mut s = String::with_capacity(36);
        for (i, b) in self.0.iter().enumerate() {
            if matches!(i, 4 | 6 | 8 | 10) {
                s.push('-');
            }
            s.push(char::from(D[usize::from(b >> 4)]));
            s.push(char::from(D[usize::from(b & 0x0F)]));
        }
        s
    }
}

const fn nibble(c: u8) -> Option<u8> {
    match c {
        b'0'..=b'9' => Some(c - b'0'),
        b'a'..=b'f' => Some(c - b'a' + 10),
        b'A'..=b'F' => Some(c - b'A' + 10),
        _ => None,
    }
}

const fn uuid(s: &str) -> Uuid128 {
    match Uuid128::parse(s) {
        Some(u) => u,
        None => panic!("invalid UUID literal"),
    }
}

/// Primary service `MESH_SVC`.
pub const MESH_SVC: Uuid128 = uuid("31B8CD85-9D3A-4622-A465-3CDCA2C32DC5");
/// `HELLO` characteristic (read, unencrypted, 16 B).
pub const HELLO_CHR: Uuid128 = uuid("86841842-A7C3-4844-84D2-5B281A3F616C");
/// `RX` characteristic (write without response, central → peripheral).
pub const RX_CHR: Uuid128 = uuid("14E91943-7673-440A-979A-F85B4DBD4DBF");
/// `TX` characteristic (notify, peripheral → central).
pub const TX_CHR: Uuid128 = uuid("A76E3168-B4F6-4BE1-8252-35AB8AF17302");

/// UUIDs and timings handed to the platform layers (DESIGN §3.5 `ble_constants()`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BleConstants {
    pub service: Uuid128,
    pub hello: Uuid128,
    pub rx: Uuid128,
    pub tx: Uuid128,
    /// HELLO value size.
    pub hello_len: u16,
    /// Max LinkFrame on stream/GATT links and on datagram links.
    pub max_stream_frame: u16,
    pub max_datagram_frame: u16,
    /// MTU requested on Android (and the largest the chunker uses).
    pub preferred_att_mtu: u16,
    /// BLE links target / max per node, neighbors max (all kinds).
    pub ble_links_target: u8,
    pub ble_links_max: u8,
    pub neighbors_max: u8,
    /// Connect attempt timeout; iOS cancels a pending connect after this unless the
    /// core armed a pending reconnect.
    pub connect_timeout_ms: u32,
    /// Noise handshake timeout.
    pub handshake_timeout_ms: u32,
    /// Per-peer backoff: initial, maximum (doubling, reset on a new token).
    pub backoff_initial_ms: u32,
    pub backoff_max_ms: u32,
    /// A link is dead after this long without inbound frames.
    pub link_death_ms: u32,
    /// PING interval while a burst is active / idle.
    pub ping_active_ms: u32,
    pub ping_idle_ms: u32,
    /// Android advertising interval while Forming (BALANCED) / Stable (LOW_POWER).
    pub adv_interval_forming_ms: u32,
    pub adv_interval_stable_ms: u32,
    /// Android filtered-scan restart period and start quota (starts per window).
    pub scan_restart_ms: u32,
    pub scan_quota_starts: u8,
    pub scan_quota_window_ms: u32,
    /// Rotation period of group_tag/node_token/local name (`t10`), accepted ± 1 slot.
    pub token_rotation_s: u32,
    /// Pending reconnect lifetime for a lost iPhone peer (§9.5 rejoin strategy a).
    pub pending_reconnect_ms: u32,
    /// Connection-limit backoff: `LimitReached` lowers the BLE max by one for this long.
    pub limit_backoff_ms: u32,
    /// Length of the iPhone foreground local name (base32 characters).
    pub local_name_len: u8,
}

impl BleConstants {
    /// The compiled-in v1 values.
    pub const V1: BleConstants = BleConstants {
        service: MESH_SVC,
        hello: HELLO_CHR,
        rx: RX_CHR,
        tx: TX_CHR,
        hello_len: crate::hello::Hello::LEN as u16,
        max_stream_frame: MAX_STREAM_FRAME as u16,
        max_datagram_frame: MAX_DATAGRAM_FRAME as u16,
        preferred_att_mtu: ATT_MTU_MAX,
        ble_links_target: 2,
        ble_links_max: 3,
        neighbors_max: 6,
        connect_timeout_ms: 8_000,
        handshake_timeout_ms: 5_000,
        backoff_initial_ms: 5_000,
        backoff_max_ms: 60_000,
        link_death_ms: 4_000,
        ping_active_ms: 500,
        ping_idle_ms: 2_000,
        adv_interval_forming_ms: 250,
        adv_interval_stable_ms: 1_000,
        scan_restart_ms: 9 * 60_000,
        scan_quota_starts: 5,
        scan_quota_window_ms: 30_000,
        token_rotation_s: 600,
        pending_reconnect_ms: 15 * 60_000,
        limit_backoff_ms: 10 * 60_000,
        local_name_len: crate::adv::LOCAL_NAME_LEN as u8,
    };
}

impl Default for BleConstants {
    fn default() -> Self {
        Self::V1
    }
}

/// The compiled-in constants (what `mesh_ffi::ble_constants()` returns).
pub const fn ble_constants() -> BleConstants {
    BleConstants::V1
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uuids_round_trip() {
        assert_eq!(
            MESH_SVC.to_canonical(),
            "31B8CD85-9D3A-4622-A465-3CDCA2C32DC5"
        );
        assert_eq!(
            HELLO_CHR.to_canonical(),
            "86841842-A7C3-4844-84D2-5B281A3F616C"
        );
        assert_eq!(
            RX_CHR.to_canonical(),
            "14E91943-7673-440A-979A-F85B4DBD4DBF"
        );
        assert_eq!(
            TX_CHR.to_canonical(),
            "A76E3168-B4F6-4BE1-8252-35AB8AF17302"
        );
        assert_eq!(MESH_SVC.le_bytes()[0], 0xC5);
        assert_eq!(MESH_SVC.le_bytes()[15], 0x31);
        assert_eq!(
            Uuid128::parse("31b8cd85-9d3a-4622-a465-3cdca2c32dc5"),
            Some(MESH_SVC)
        );
        assert_eq!(Uuid128::parse("31B8CD859D3A4622A4653CDCA2C32DC5"), None);
        assert_eq!(Uuid128::parse("31B8CD85-9D3A-4622-A465-3CDCA2C32DCZ"), None);
    }

    #[test]
    fn timings_match_design() {
        let c = ble_constants();
        assert_eq!(c.hello_len, 16);
        assert_eq!(
            (c.ble_links_target, c.ble_links_max, c.neighbors_max),
            (2, 3, 6)
        );
        assert_eq!(
            (c.connect_timeout_ms, c.handshake_timeout_ms),
            (8_000, 5_000)
        );
        assert_eq!(c.preferred_att_mtu, 517);
        assert_eq!(c.local_name_len, 6);
        assert_eq!(c.max_stream_frame, 1024);
        assert_eq!(c.max_datagram_frame, 1200);
    }
}
