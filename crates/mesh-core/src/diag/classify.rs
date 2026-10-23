//! Frame classification for the per-link `byType` counters of the `s` line: a sealed
//! frame carries several records, and it is counted once, under the type of its
//! largest record (a voice frame that piggybacks a PONG counts as voice).

use mesh_lab::ridelog::FrameType;
use mesh_wire::mesh::PacketType;
use mesh_wire::record::rtype;

/// The [`FrameType`] of one MESH packet type.
pub fn mesh_type(p: PacketType) -> FrameType {
    match p {
        PacketType::Voice => FrameType::Voice,
        PacketType::Presence => FrameType::Presence,
        PacketType::Rekey => FrameType::Rekey,
        PacketType::Roster | PacketType::JoinReq => FrameType::Bulk,
        PacketType::Talk
        | PacketType::FloorReject
        | PacketType::Leave
        | PacketType::RideCtl
        | PacketType::MeshProbe
        | PacketType::PathHint
        | PacketType::GroupCtl
        | PacketType::Position
        | PacketType::Alert => FrameType::Ctrl,
        PacketType::Ext | PacketType::Unknown(_) => FrameType::Other,
    }
}

/// The [`FrameType`] of one record (`rt` type byte, `body`).
pub fn record_type(rt: u8, body: &[u8]) -> FrameType {
    match rt {
        rtype::MESH => body
            .first()
            .map_or(FrameType::Other, |b| mesh_type(PacketType::from_bits(*b))),
        rtype::PING => FrameType::Ping,
        rtype::PONG => FrameType::Pong,
        rtype::NBR => FrameType::Nbr,
        rtype::LSTAT => FrameType::Lstat,
        rtype::SYNC => FrameType::Sync,
        rtype::CAPS | rtype::CLOSE | rtype::IDPROOF | rtype::UPGRADE => FrameType::Ctrl,
        rtype::LAB_CTL | rtype::LAB_GPS | rtype::LPROBE | rtype::LECHO => FrameType::Lab,
        _ => FrameType::Other,
    }
}

/// Classifies a record plaintext (a sequence of `type u8, len LEB128, body`) by its
/// largest record. Malformed tails stop the scan; an empty plaintext is `Other`.
pub fn plaintext_type(pt: &[u8]) -> FrameType {
    let mut best = (0usize, FrameType::Other);
    let mut i = 0;
    while i < pt.len() {
        let rt = pt[i];
        let Some((len, leb)) = leb128(&pt[i + 1..]) else {
            break;
        };
        let start = i + 1 + leb;
        let Some(body) = pt.get(start..start + len) else {
            break;
        };
        let size = 1 + leb + len;
        if size > best.0 {
            best = (size, record_type(rt, body));
        }
        i = start + len;
    }
    best.1
}

/// One LEB128 length (at most 2 bytes on the wire): `(value, encoded size)`.
fn leb128(b: &[u8]) -> Option<(usize, usize)> {
    let b0 = *b.first()?;
    if b0 & 0x80 == 0 {
        return Some((usize::from(b0), 1));
    }
    let b1 = *b.get(1)?;
    Some((usize::from(b0 & 0x7F) | (usize::from(b1 & 0x7F) << 7), 2))
}

#[cfg(test)]
mod tests {
    use super::*;
    use mesh_wire::record::{Ping, Record};

    #[test]
    fn the_largest_record_names_the_frame() {
        let mut pt = Record::Ping(Ping { id: 1, t_tx: 2 }).encode().unwrap();
        let mut mesh = vec![rtype::MESH, 12, 0x01 | (1 << 5)];
        mesh.extend_from_slice(&[0; 11]);
        pt.extend_from_slice(&mesh);
        assert_eq!(plaintext_type(&pt), FrameType::Voice);
        let ping = Record::Ping(Ping { id: 1, t_tx: 2 }).encode().unwrap();
        assert_eq!(plaintext_type(&ping), FrameType::Ping);
        assert_eq!(plaintext_type(&[]), FrameType::Other);
        assert_eq!(plaintext_type(&[rtype::NBR, 0x85]), FrameType::Other);
    }
}
