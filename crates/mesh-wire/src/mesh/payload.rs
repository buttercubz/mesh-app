//! Typed view of a MeshPacket payload by `ptype` (DESIGN §4.6), plus the builder that
//! turns a logical control body into SEG-flagged MeshPackets (§4.6.1 "Segmentation").

use super::control::{JoinReq, Rekey, Roster};
use super::misc::{Ext, MeshProbe, PathHint, Position};
use super::presence::Presence;
use super::segment::{Segment, segment_body};
use super::signed::{GroupCtl, Leave, RideCtl};
use super::voice::{FloorReject, Talk, Voice};
use super::{MeshHeader, MeshPacket, PacketType};
use crate::error::{Result, WireError};

/// A decoded payload. Segmented types (ROSTER, JOIN_REQ, REKEY) decode to their
/// [`Segment`]; the logical body is parsed after reassembly with [`Roster::decode`],
/// [`JoinReq::decode`] or [`Rekey::decode`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MeshPayload<'a> {
    Voice(Voice<'a>),
    Talk(Talk),
    FloorReject(FloorReject),
    Presence(Presence),
    /// ROSTER, JOIN_REQ or REKEY segment.
    Segment(PacketType, Segment<'a>),
    Leave(Leave),
    RideCtl(RideCtl),
    MeshProbe(MeshProbe),
    PathHint(PathHint),
    GroupCtl(GroupCtl),
    Position(Position),
    Ext(Ext<'a>),
    /// ALERT (reserved) or an unknown ptype: relayed, not delivered.
    Opaque(PacketType, &'a [u8]),
}

impl<'a> MeshPayload<'a> {
    /// Decodes the payload of `pkt` according to its header.
    pub fn decode(pkt: &MeshPacket<'a>) -> Result<Self> {
        let p = pkt.payload;
        let t = pkt.header.ptype;
        let opaque = matches!(
            t,
            PacketType::Alert | PacketType::Unknown(_) | PacketType::Ext
        );
        if !opaque && t.is_segmented() != pkt.header.seg {
            return Err(WireError::BadValue {
                what: "SEG flag for ptype",
                value: u32::from(t.bits()),
            });
        }
        Ok(match t {
            PacketType::Voice => MeshPayload::Voice(Voice::decode(p)?),
            PacketType::Talk => MeshPayload::Talk(Talk::decode(p)?),
            PacketType::FloorReject => MeshPayload::FloorReject(FloorReject::decode(p)?),
            PacketType::Presence => MeshPayload::Presence(Presence::decode(p)?),
            PacketType::Roster | PacketType::JoinReq | PacketType::Rekey => {
                MeshPayload::Segment(t, Segment::decode(p)?)
            }
            PacketType::Leave => MeshPayload::Leave(Leave::decode(p)?),
            PacketType::RideCtl => MeshPayload::RideCtl(RideCtl::decode(p)?),
            PacketType::MeshProbe => MeshPayload::MeshProbe(MeshProbe::decode(p)?),
            PacketType::PathHint => MeshPayload::PathHint(PathHint::decode(p)?),
            PacketType::GroupCtl => MeshPayload::GroupCtl(GroupCtl::decode(p)?),
            PacketType::Position => MeshPayload::Position(Position::decode(p)?),
            PacketType::Ext => MeshPayload::Ext(Ext::decode(p)?),
            PacketType::Alert | PacketType::Unknown(_) => MeshPayload::Opaque(t, p),
        })
    }

    /// The packet type this payload belongs to.
    pub fn ptype(&self) -> PacketType {
        match self {
            MeshPayload::Voice(_) => PacketType::Voice,
            MeshPayload::Talk(_) => PacketType::Talk,
            MeshPayload::FloorReject(_) => PacketType::FloorReject,
            MeshPayload::Presence(_) => PacketType::Presence,
            MeshPayload::Segment(t, _) | MeshPayload::Opaque(t, _) => *t,
            MeshPayload::Leave(_) => PacketType::Leave,
            MeshPayload::RideCtl(_) => PacketType::RideCtl,
            MeshPayload::MeshProbe(_) => PacketType::MeshProbe,
            MeshPayload::PathHint(_) => PacketType::PathHint,
            MeshPayload::GroupCtl(_) => PacketType::GroupCtl,
            MeshPayload::Position(_) => PacketType::Position,
            MeshPayload::Ext(_) => PacketType::Ext,
        }
    }

    /// Encodes the payload bytes (without the header).
    pub fn encode(&self) -> Result<Vec<u8>> {
        Ok(match self {
            MeshPayload::Voice(v) => v.encode()?,
            MeshPayload::Talk(t) => t.encode(),
            MeshPayload::FloorReject(f) => f.encode(),
            MeshPayload::Presence(p) => p.encode(),
            MeshPayload::Segment(_, s) => s.encode()?,
            MeshPayload::Leave(l) => l.encode(),
            MeshPayload::RideCtl(r) => r.encode(),
            MeshPayload::MeshProbe(m) => m.encode()?,
            MeshPayload::PathHint(h) => h.encode()?,
            MeshPayload::GroupCtl(g) => g.encode()?,
            MeshPayload::Position(p) => p.encode(),
            MeshPayload::Ext(e) => e.encode(),
            MeshPayload::Opaque(_, b) => b.to_vec(),
        })
    }
}

/// Encodes `header` + `payload` into a MESH record body, forcing the header's ptype
/// and SEG flag to match the payload.
pub fn encode_packet(header: &MeshHeader, payload: &MeshPayload<'_>) -> Result<Vec<u8>> {
    let mut h = *header;
    h.ptype = payload.ptype();
    if !matches!(
        h.ptype,
        PacketType::Alert | PacketType::Unknown(_) | PacketType::Ext
    ) {
        h.seg = h.ptype.is_segmented();
    }
    let body = payload.encode()?;
    MeshPacket {
        header: h,
        payload: &body,
    }
    .encode()
}

/// Splits a logical ROSTER/JOIN_REQ/REKEY body into SEG MeshPackets. Each segment is
/// its own packet with its own `seq` (`first_seq`, `first_seq + 1`, … wrapping), so it
/// is deduplicated individually; relays forward segments without reassembly.
pub fn segment_packets(
    header: &MeshHeader,
    first_seq: u16,
    msg_id: u16,
    body: &[u8],
) -> Result<Vec<Vec<u8>>> {
    if !header.ptype.is_segmented() {
        return Err(WireError::BadValue {
            what: "segmented ptype",
            value: u32::from(header.ptype.bits()),
        });
    }
    segment_body(msg_id, body)?
        .iter()
        .enumerate()
        .map(|(i, s)| {
            let mut h = *header;
            h.seg = true;
            h.seq = first_seq.wrapping_add(i as u16);
            let payload = s.encode()?;
            MeshPacket {
                header: h,
                payload: &payload,
            }
            .encode()
        })
        .collect()
}

/// Parses a reassembled logical body of a segmented type.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ControlBody {
    Roster(Roster),
    JoinReq(JoinReq),
    Rekey(Rekey),
}

impl ControlBody {
    /// Decodes `body` as the logical message of `ptype`.
    pub fn decode(ptype: PacketType, body: &[u8]) -> Result<Self> {
        Ok(match ptype {
            PacketType::Roster => ControlBody::Roster(Roster::decode(body)?),
            PacketType::JoinReq => ControlBody::JoinReq(JoinReq::decode(body)?),
            PacketType::Rekey => ControlBody::Rekey(Rekey::decode(body)?),
            other => {
                return Err(WireError::BadValue {
                    what: "control body ptype",
                    value: u32::from(other.bits()),
                });
            }
        })
    }
}
