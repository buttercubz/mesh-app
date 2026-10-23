//! Per-link transport framing over mesh-wire (DESIGN §4.2): stream deframing with any
//! chunk boundary, GATT chunking/reassembly per ATT value, one datagram per frame.

use mesh_wire::WireError;
use mesh_wire::link::{
    GattChunker, GattEvent, GattReassembler, MAX_DATAGRAM_FRAME, MAX_STREAM_FRAME, StreamDeframer,
    check_datagram, encode_stream,
};

use super::types::{LinkKind, LinkUnit};

/// What happened to inbound bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Inbound {
    /// Zero or more complete LinkFrames.
    Frames(Vec<Vec<u8>>),
    /// A GATT value was dropped (counter mismatch, overflow, no START); the link stays.
    Dropped,
    /// The stream is broken (invalid length): the link must be closed, no resync.
    Fatal(WireError),
}

/// Framing state of one link (both directions).
#[derive(Debug, Clone)]
pub enum Framer {
    Stream(StreamDeframer),
    Gatt {
        rx: GattReassembler,
        tx: GattChunker,
    },
    Datagram,
}

impl Framer {
    /// The framer for a link's unit; `att_mtu` sizes GATT chunks (23 when unknown).
    pub fn new(unit: LinkUnit, att_mtu: Option<u16>) -> Self {
        match unit {
            LinkUnit::Stream => Framer::Stream(StreamDeframer::new()),
            LinkUnit::Message => Framer::Gatt {
                rx: GattReassembler::new(),
                tx: GattChunker::new(att_mtu.unwrap_or(mesh_wire::link::ATT_MTU_MIN)),
            },
            LinkUnit::Datagram => Framer::Datagram,
        }
    }

    /// Updates the ATT MTU after an exchange (GATT only).
    pub fn set_att_mtu(&mut self, mtu: u16) {
        if let Framer::Gatt { tx, .. } = self {
            tx.set_att_mtu(mtu);
        }
    }

    /// Largest LinkFrame this framer can carry for a link of `kind` whose platform
    /// announced `max_frame`.
    pub fn frame_limit(&self, kind: LinkKind, max_frame: u16) -> usize {
        let cap = match self {
            Framer::Datagram => MAX_DATAGRAM_FRAME,
            _ => MAX_STREAM_FRAME,
        };
        usize::from(max_frame).clamp(1, cap).min(kind.max_frame())
    }

    /// Consumes one `handle_packet` unit.
    pub fn inbound(&mut self, bytes: &[u8]) -> Inbound {
        match self {
            Framer::Stream(d) => match d.feed(bytes) {
                Ok(frames) => Inbound::Frames(frames),
                Err(e) => Inbound::Fatal(e),
            },
            Framer::Gatt { rx, .. } => match rx.push(bytes) {
                GattEvent::Frame(f) => Inbound::Frames(vec![f]),
                GattEvent::Pending => Inbound::Frames(Vec::new()),
                GattEvent::Dropped(_) => Inbound::Dropped,
            },
            Framer::Datagram => match check_datagram(bytes) {
                Ok(()) => Inbound::Frames(vec![bytes.to_vec()]),
                Err(_) => Inbound::Dropped,
            },
        }
    }

    /// Frames one LinkFrame into the units to send, in order.
    pub fn outbound(&mut self, frame: &[u8]) -> Result<Vec<Vec<u8>>, WireError> {
        match self {
            Framer::Stream(_) => Ok(vec![encode_stream(frame)?]),
            Framer::Gatt { tx, .. } => tx.chunk(frame),
            Framer::Datagram => {
                check_datagram(frame)?;
                Ok(vec![frame.to_vec()])
            }
        }
    }

    /// Framing bytes added per frame (for on-air size estimates).
    pub fn overhead(&self, frame_len: usize) -> usize {
        match self {
            Framer::Stream(_) => 2,
            Framer::Gatt { tx, .. } => frame_len.div_ceil(tx.chunk_payload().max(1)),
            Framer::Datagram => 0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stream_round_trip_any_boundary() {
        let mut tx = Framer::new(LinkUnit::Stream, None);
        let mut rx = Framer::new(LinkUnit::Stream, None);
        let mut wire = Vec::new();
        for f in [&b"abc"[..], &[7u8; 300][..]] {
            for u in tx.outbound(f).unwrap() {
                wire.extend_from_slice(&u);
            }
        }
        let mut got = Vec::new();
        for chunk in wire.chunks(7) {
            match rx.inbound(chunk) {
                Inbound::Frames(fs) => got.extend(fs),
                other => panic!("{other:?}"),
            }
        }
        assert_eq!(got, vec![b"abc".to_vec(), vec![7u8; 300]]);
        assert!(matches!(rx.inbound(&[0, 0]), Inbound::Fatal(_)));
    }

    #[test]
    fn gatt_chunks_and_reassembles() {
        let mut tx = Framer::new(LinkUnit::Message, Some(23));
        let mut rx = Framer::new(LinkUnit::Message, Some(23));
        let frame = vec![9u8; 100];
        let units = tx.outbound(&frame).unwrap();
        assert_eq!(units.len(), 100usize.div_ceil(19));
        let mut out = Vec::new();
        for u in &units {
            if let Inbound::Frames(fs) = rx.inbound(u) {
                out.extend(fs);
            }
        }
        assert_eq!(out, vec![frame]);
        assert_eq!(rx.inbound(&[]), Inbound::Dropped);
        assert_eq!(tx.overhead(100), 6);
    }

    #[test]
    fn datagram_limits() {
        let mut d = Framer::new(LinkUnit::Datagram, None);
        assert_eq!(d.inbound(&[1, 2]), Inbound::Frames(vec![vec![1, 2]]));
        assert_eq!(d.inbound(&[0u8; 1201]), Inbound::Dropped);
        assert!(d.outbound(&[0u8; 1201]).is_err());
        assert_eq!(d.frame_limit(LinkKind::WifiAware, 1200), 1200);
        let s = Framer::new(LinkUnit::Stream, None);
        assert_eq!(s.frame_limit(LinkKind::BleL2cap, 5000), 1024);
        assert_eq!(s.frame_limit(LinkKind::BleL2cap, 300), 300);
    }
}
