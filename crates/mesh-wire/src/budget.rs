//! Sizes and per-link load (DESIGN §4.8) and the normative cost model used by the pacer
//! and the per-link stream cap: `cost(k, profile) = k × r_stream(profile) + C_CTRL`,
//! with uncoalesced per-stream rates. All rates are integer bits per second, so the
//! §4.8 table is reproduced exactly (23.6 kbps = 23 600 bps).

use crate::frame::SEC_OVERHEAD;
use crate::link::STREAM_PREFIX;
use crate::mesh::{EncoderProfile, MeshHeader, Position, Presence, Voice};
use crate::record::record_len;

/// Control reserve per link (PRESENCE + POSITION + NBR/LSTAT/PING), 3 kbps.
pub const C_CTRL_BPS: u32 = 3_000;
/// Per-link BLE budget of the research (50 % of ~120 kbps).
pub const BLE_LINK_BUDGET_BPS: u32 = 60_000;
/// The pacer admits `k` streams while `cost(k) ≤ 0.9 × grant` (§5.6, Appendix A).
pub const CAP_UTILISATION_PERMILLE: u32 = 900;

/// How the frame reaches the radio.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Framing {
    /// BLE L2CAP CoC / tunnel: `len u16` prefix per frame.
    Stream,
    /// Wi-Fi Aware: one datagram per frame, no prefix.
    Datagram,
}

/// Nominal parameters of an encoder profile (§7.4): bitrate, frame duration, framing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProfileSpec {
    pub bitrate_bps: u32,
    pub frame_ms: u32,
    pub framing: Framing,
}

impl ProfileSpec {
    /// The §7.4 parameters of `p`.
    pub const fn of(p: EncoderProfile) -> Self {
        match p {
            EncoderProfile::Ble16 => Self {
                bitrate_bps: 16_000,
                frame_ms: 40,
                framing: Framing::Stream,
            },
            EncoderProfile::Ble12 => Self {
                bitrate_bps: 12_000,
                frame_ms: 40,
                framing: Framing::Stream,
            },
            EncoderProfile::Wifi20 => Self {
                bitrate_bps: 20_000,
                frame_ms: 20,
                framing: Framing::Datagram,
            },
            EncoderProfile::Wifi24Fec => Self {
                bitrate_bps: 24_000,
                frame_ms: 20,
                framing: Framing::Datagram,
            },
            EncoderProfile::BleDred => Self {
                bitrate_bps: 20_000,
                frame_ms: 40,
                framing: Framing::Stream,
            },
        }
    }

    /// Nominal Opus packet size: `bitrate × frame / 8`.
    pub const fn opus_bytes(&self) -> usize {
        (self.bitrate_bps * self.frame_ms / 8_000) as usize
    }
}

/// The byte chain of one VOICE frame: MeshPacket → record → SEC frame → on-air unit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VoiceSizes {
    pub mesh_packet: usize,
    pub record: usize,
    pub sec_frame: usize,
    /// L2CAP bytes (frame + 2) on stream links, the datagram on Wi-Fi.
    pub on_air: usize,
}

/// Sizes of one uncoalesced VOICE frame carrying `opus` bytes.
pub const fn voice_sizes(opus: usize, framing: Framing) -> VoiceSizes {
    let mesh_packet = MeshHeader::LEN + Voice::HEADER + opus;
    let record = record_len(mesh_packet);
    let sec_frame = record + SEC_OVERHEAD;
    let on_air = match framing {
        Framing::Stream => sec_frame + STREAM_PREFIX,
        Framing::Datagram => sec_frame,
    };
    VoiceSizes {
        mesh_packet,
        record,
        sec_frame,
        on_air,
    }
}

/// Uncoalesced per-stream rate of a profile in bps (`on_air × 8 / frame`).
pub const fn r_stream_bps(p: EncoderProfile) -> u32 {
    let s = ProfileSpec::of(p);
    let v = voice_sizes(s.opus_bytes(), s.framing);
    (v.on_air as u32) * 8 * 1_000 / s.frame_ms
}

/// Rate of `k` streams of `p` coalesced into one frame per frame period (the §4.8
/// "42.6 kbps when coalesced" row; informative, the cost model is uncoalesced).
pub const fn coalesced_bps(k: u32, p: EncoderProfile) -> u32 {
    if k == 0 {
        return 0;
    }
    let s = ProfileSpec::of(p);
    let v = voice_sizes(s.opus_bytes(), s.framing);
    let prefix = match s.framing {
        Framing::Stream => STREAM_PREFIX,
        Framing::Datagram => 0,
    };
    let bytes = (k as usize) * v.record + SEC_OVERHEAD + prefix;
    (bytes as u32) * 8 * 1_000 / s.frame_ms
}

/// `cost(k, profile) = k × r_stream(profile) + C_CTRL` in bps (normative, §4.8).
pub const fn cost_bps(k: u32, p: EncoderProfile) -> u32 {
    k * r_stream_bps(p) + C_CTRL_BPS
}

/// `cap_pacing`: the largest `k` with `cost(k) ≤ 0.9 × grant` (0 when even one stream
/// does not fit). Capped at `s_max` (S_MAX of the link kind, §6.1).
pub const fn cap_pacing(grant_bps: u32, p: EncoderProfile, s_max: u32) -> u32 {
    let budget = (grant_bps as u64) * (CAP_UTILISATION_PERMILLE as u64) / 1_000;
    let mut k = 0u32;
    while k < s_max && (cost_bps(k + 1, p) as u64) <= budget {
        k += 1;
    }
    k
}

/// Record size of a PRESENCE packet (25 B).
pub const PRESENCE_RECORD: usize = record_len(MeshHeader::LEN + Presence::LEN);
/// Record size of a POSITION packet (25 B).
pub const POSITION_RECORD: usize = record_len(MeshHeader::LEN + Position::LEN);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn section_4_8_table() {
        let ble16 = voice_sizes(80, Framing::Stream);
        assert_eq!(
            ble16,
            VoiceSizes {
                mesh_packet: 93,
                record: 95,
                sec_frame: 116,
                on_air: 118
            }
        );
        assert_eq!(r_stream_bps(EncoderProfile::Ble16), 23_600);
        assert_eq!(2 * r_stream_bps(EncoderProfile::Ble16), 47_200);
        assert_eq!(coalesced_bps(2, EncoderProfile::Ble16), 42_600);

        let dred = voice_sizes(100, Framing::Stream);
        assert_eq!(
            dred,
            VoiceSizes {
                mesh_packet: 113,
                record: 115,
                sec_frame: 136,
                on_air: 138
            }
        );
        assert_eq!(r_stream_bps(EncoderProfile::BleDred), 27_600);

        let wifi = voice_sizes(50, Framing::Datagram);
        assert_eq!(
            wifi,
            VoiceSizes {
                mesh_packet: 63,
                record: 65,
                sec_frame: 86,
                on_air: 86
            }
        );
        assert_eq!(r_stream_bps(EncoderProfile::Wifi20), 34_400);

        assert_eq!(r_stream_bps(EncoderProfile::Ble12), 19_600);
        assert_eq!(PRESENCE_RECORD, 25);
        assert_eq!(POSITION_RECORD, 25);
        // PRESENCE from 15 members every 2 s ≈ 1.5 kbps; POSITION every 5 s ≈ 0.6 kbps.
        assert_eq!(15 * PRESENCE_RECORD * 8 / 2, 1_500);
        assert_eq!(15 * POSITION_RECORD * 8 / 5, 600);
    }

    #[test]
    fn cost_model() {
        assert_eq!(cost_bps(2, EncoderProfile::Ble16), 50_200);
        assert!(cost_bps(2, EncoderProfile::Ble16) <= BLE_LINK_BUDGET_BPS * 9 / 10);
        assert_eq!(cost_bps(0, EncoderProfile::Ble16), C_CTRL_BPS);
        assert_eq!(cap_pacing(BLE_LINK_BUDGET_BPS, EncoderProfile::Ble16, 2), 2);
        assert_eq!(cap_pacing(BLE_LINK_BUDGET_BPS, EncoderProfile::Ble16, 4), 2);
        assert_eq!(
            cap_pacing(BLE_LINK_BUDGET_BPS, EncoderProfile::BleDred, 2),
            1
        );
        assert_eq!(cap_pacing(8_000, EncoderProfile::Ble16, 2), 0);
        assert_eq!(cap_pacing(2_000_000, EncoderProfile::Wifi20, 4), 4);
    }
}
