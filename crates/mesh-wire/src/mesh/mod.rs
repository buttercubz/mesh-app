//! MeshPacket (DESIGN §4.5-§4.6): 9-byte relayable header + typed payloads.

pub mod control;
mod misc;
pub mod payload;
mod presence;
pub mod segment;
mod signed;
mod voice;

pub use control::{
    CONTROL_BODY_VERSION, JoinReq, MAX_ROSTER_MEMBERS, Rekey, RekeyEntry, Roster, RosterEntry,
    RosterRole, SEALED_LEN,
};
pub use misc::{Ext, MeshProbe, PathHint, Position};
pub use payload::{ControlBody, MeshPayload, encode_packet, segment_packets};
pub use presence::{Presence, PresenceAudio, PresenceCaps, PresenceLinks, PresenceState};
pub use segment::{AssemblyEvent, Segment, SegmentAssembler, segment_body, sync_chunks};
pub use signed::{GroupCtl, GroupCtlOp, Leave, LeaveReason, RideCtl, RideCtlOp, SIG_LEN};
pub use voice::{
    Bandwidth, EncoderProfile, FloorReject, FloorRejectReason, FrameDur, Talk, TalkMode, TalkPrio,
    TalkState, Voice, VoiceCfg,
};

use crate::codec::{PutLe, Reader};
use crate::error::{Result, WireError};

/// Header version bits (7-5) for protocol v1.
pub const MESH_VERSION: u8 = 1;

/// Mesh packet types (5 bits). Unknown values are relayed but not delivered.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum PacketType {
    Voice,
    Talk,
    FloorReject,
    Presence,
    Roster,
    JoinReq,
    Rekey,
    Leave,
    RideCtl,
    MeshProbe,
    PathHint,
    GroupCtl,
    Position,
    /// 0x11, reserved (convLink `ALT:` types).
    Alert,
    /// 0x1F: `ext_type u8` + body (future).
    Ext,
    /// Any other 5-bit value: relayed, not delivered.
    Unknown(u8),
}

impl PacketType {
    /// From the 5-bit ptype field (higher bits ignored).
    pub const fn from_bits(v: u8) -> Self {
        match v & 0x1F {
            0x01 => Self::Voice,
            0x02 => Self::Talk,
            0x03 => Self::FloorReject,
            0x04 => Self::Presence,
            0x05 => Self::Roster,
            0x06 => Self::JoinReq,
            0x07 => Self::Rekey,
            0x08 => Self::Leave,
            0x09 => Self::RideCtl,
            0x0A => Self::MeshProbe,
            0x0B => Self::PathHint,
            0x0C => Self::GroupCtl,
            0x10 => Self::Position,
            0x11 => Self::Alert,
            0x1F => Self::Ext,
            other => Self::Unknown(other),
        }
    }

    /// The 5-bit value.
    pub const fn bits(self) -> u8 {
        match self {
            Self::Voice => 0x01,
            Self::Talk => 0x02,
            Self::FloorReject => 0x03,
            Self::Presence => 0x04,
            Self::Roster => 0x05,
            Self::JoinReq => 0x06,
            Self::Rekey => 0x07,
            Self::Leave => 0x08,
            Self::RideCtl => 0x09,
            Self::MeshProbe => 0x0A,
            Self::PathHint => 0x0B,
            Self::GroupCtl => 0x0C,
            Self::Position => 0x10,
            Self::Alert => 0x11,
            Self::Ext => 0x1F,
            Self::Unknown(v) => v & 0x1F,
        }
    }

    /// Default TTL at the origin (§4.6, Appendix A).
    pub const fn default_ttl(self) -> u8 {
        match self {
            Self::Voice => 4,
            Self::Talk | Self::FloorReject => 6,
            Self::MeshProbe => 8,
            Self::PathHint => 3,
            _ => 15,
        }
    }

    /// Traffic class (§4.6).
    pub const fn class(self) -> TrafficClass {
        match self {
            Self::Voice => TrafficClass::Voice,
            Self::Position => TrafficClass::Bulk,
            Self::MeshProbe => TrafficClass::Lab,
            _ => TrafficClass::Ctrl,
        }
    }

    /// `true` for the types sent with the SEG flag (§4.7).
    pub const fn is_segmented(self) -> bool {
        matches!(self, Self::Roster | Self::JoinReq | Self::Rekey)
    }
}

/// Flags bits 7-6.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(u8)]
pub enum TrafficClass {
    Ctrl = 0,
    Voice = 1,
    Bulk = 2,
    Lab = 3,
}

impl TrafficClass {
    const fn from_bits(v: u8) -> Self {
        match v & 0x03 {
            0 => Self::Ctrl,
            1 => Self::Voice,
            2 => Self::Bulk,
            _ => Self::Lab,
        }
    }
}

/// The 9-byte MeshPacket header (little-endian).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct MeshHeader {
    pub ptype: PacketType,
    /// `ttl_remaining`, 0-15.
    pub ttl: u8,
    /// Link traversals so far, saturating at 15.
    pub hops: u8,
    pub origin: u16,
    /// Non-zero origin session tag.
    pub sess: u8,
    pub seq: u16,
    /// Age in 2 ms units, saturating at 255.
    pub age: u8,
    pub class: TrafficClass,
    pub leader: bool,
    pub seg: bool,
    /// Flags bits 3-0 (reserved, 0 in v1; preserved by relays).
    pub reserved: u8,
}

impl MeshHeader {
    /// Encoded size.
    pub const LEN: usize = 9;
    /// TTL and hops are 4-bit fields.
    pub const MAX_TTL: u8 = 15;
    /// Saturation value of `hops`.
    pub const MAX_HOPS: u8 = 15;
    /// Age unit in µs.
    pub const AGE_UNIT_US: u64 = 2_000;

    /// A header with the type's default TTL and class, hops 0, age 0.
    pub fn new(ptype: PacketType, origin: u16, sess: u8, seq: u16) -> Self {
        MeshHeader {
            ptype,
            ttl: ptype.default_ttl(),
            hops: 0,
            origin,
            sess,
            seq,
            age: 0,
            class: ptype.class(),
            leader: false,
            seg: ptype.is_segmented(),
            reserved: 0,
        }
    }

    /// Parses the header.
    pub fn decode(b: &[u8]) -> Result<Self> {
        let mut r = Reader::new(b);
        let vt = r.u8("mesh vt")?;
        let version = vt >> 5;
        if version != MESH_VERSION {
            return Err(WireError::Version { got: version });
        }
        let th = r.u8("ttl_hops")?;
        let origin = r.u16("origin")?;
        let sess = r.u8("sess")?;
        if sess == 0 {
            return Err(WireError::BadValue {
                what: "sess",
                value: 0,
            });
        }
        let seq = r.u16("seq")?;
        let age = r.u8("age")?;
        let flags = r.u8("flags")?;
        Ok(MeshHeader {
            ptype: PacketType::from_bits(vt),
            ttl: th >> 4,
            hops: th & 0x0F,
            origin,
            sess,
            seq,
            age,
            class: TrafficClass::from_bits(flags >> 6),
            leader: flags & 0x20 != 0,
            seg: flags & 0x10 != 0,
            reserved: flags & 0x0F,
        })
    }

    /// Serializes the header (9 bytes).
    pub fn encode(&self) -> Result<[u8; 9]> {
        if self.ttl > Self::MAX_TTL {
            return Err(WireError::BadValue {
                what: "ttl",
                value: u32::from(self.ttl),
            });
        }
        if self.hops > Self::MAX_HOPS {
            return Err(WireError::BadValue {
                what: "hops",
                value: u32::from(self.hops),
            });
        }
        if self.sess == 0 {
            return Err(WireError::BadValue {
                what: "sess",
                value: 0,
            });
        }
        if self.reserved > 0x0F {
            return Err(WireError::BadValue {
                what: "flags reserved",
                value: u32::from(self.reserved),
            });
        }
        let mut o = Vec::with_capacity(Self::LEN);
        o.put_u8((MESH_VERSION << 5) | self.ptype.bits());
        o.put_u8((self.ttl << 4) | self.hops);
        o.put_u16(self.origin);
        o.put_u8(self.sess);
        o.put_u16(self.seq);
        o.put_u8(self.age);
        o.put_u8(
            ((self.class as u8) << 6)
                | (u8::from(self.leader) << 5)
                | (u8::from(self.seg) << 4)
                | self.reserved,
        );
        let mut out = [0u8; 9];
        out.copy_from_slice(&o);
        Ok(out)
    }

    /// Relay rewrite (§4.5): `ttl −= 1`, `hops += 1` (saturating), `age += added_us`
    /// (local residence + incoming one-way estimate) in 2 ms units, saturating at 255.
    /// Returns `None` when the TTL is already 0 (the packet must not be forwarded).
    pub fn relayed(&self, added_us: u64) -> Option<Self> {
        if self.ttl == 0 {
            return None;
        }
        let mut h = *self;
        h.ttl -= 1;
        h.hops = (h.hops + 1).min(Self::MAX_HOPS);
        h.age = add_age(h.age, added_us);
        Some(h)
    }

    /// Age in µs.
    pub fn age_us(&self) -> u64 {
        u64::from(self.age) * Self::AGE_UNIT_US
    }

    /// The dedup key `(origin, sess, seq)` (§5.3).
    pub fn dedup_key(&self) -> (u16, u8, u16) {
        (self.origin, self.sess, self.seq)
    }
}

/// Adds `added_us` to a 2 ms-unit age, rounding to the nearest unit, saturating at 255.
pub fn add_age(age: u8, added_us: u64) -> u8 {
    let units = (added_us + MeshHeader::AGE_UNIT_US / 2) / MeshHeader::AGE_UNIT_US;
    let total = u64::from(age).saturating_add(units);
    total.min(255) as u8
}

/// A MeshPacket: header + payload borrowed from the MESH record body.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MeshPacket<'a> {
    pub header: MeshHeader,
    pub payload: &'a [u8],
}

impl<'a> MeshPacket<'a> {
    /// Parses a MESH record body.
    pub fn decode(b: &'a [u8]) -> Result<Self> {
        crate::codec::check_max("mesh packet", b.len(), crate::record::MAX_RECORD_BODY)?;
        let header = MeshHeader::decode(b)?;
        let payload = b.get(MeshHeader::LEN..).unwrap_or(&[]);
        Ok(MeshPacket { header, payload })
    }

    /// Serializes header + payload.
    pub fn encode(&self) -> Result<Vec<u8>> {
        let mut o = Vec::with_capacity(MeshHeader::LEN + self.payload.len());
        o.put(&self.header.encode()?);
        o.put(self.payload);
        crate::codec::check_max("mesh packet", o.len(), crate::record::MAX_RECORD_BODY)?;
        Ok(o)
    }
}
