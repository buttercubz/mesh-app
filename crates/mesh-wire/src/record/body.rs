//! Typed record bodies (DESIGN §4.4). Fixed-size bodies must have exactly their size.

use super::lab::{LabCtl, LabGps, Lecho, Lprobe};
use super::rtype;
use crate::codec::{PutLe, Reader, check_max};
use crate::error::{Result, WireError};
use crate::frame::RejectReason;

/// A typed record, borrowing variable-length parts from the plaintext.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Record<'a> {
    /// 0x01: a MeshPacket (header + payload), parsed with [`crate::mesh::MeshPacket`].
    Mesh(&'a [u8]),
    Ping(Ping),
    Pong(Pong),
    Nbr(Nbr),
    Lstat(Lstat),
    Caps(Caps),
    Lprobe(Lprobe),
    Lecho(Lecho),
    Upgrade(Upgrade<'a>),
    Close(Close),
    LabCtl(LabCtl<'a>),
    IdProof(IdProof),
    LabGps(LabGps),
    /// 0x0E: `len` zero bytes, ignored by receivers.
    Pad(u16),
    Sync(Sync<'a>),
    /// 0x10-0xFF or any type this build does not know: skipped by receivers.
    Unknown {
        rtype: u8,
        body: &'a [u8],
    },
}

/// 0x02 PING: `id u16, t_tx u32` (sender µs clock, truncated).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Ping {
    pub id: u16,
    pub t_tx: u32,
}

/// 0x03 PONG: `id u16, t_tx u32` (echo), `hold_us u16`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Pong {
    pub id: u16,
    pub t_tx: u32,
    pub hold_us: u16,
}

/// NBR entry flags: b0 good, b1 congested, b3-2 kind (0 BLE, 1 Aware, 2 Tunnel).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct NbrFlags(pub u8);

impl NbrFlags {
    pub const KIND_BLE: u8 = 0;
    pub const KIND_AWARE: u8 = 1;
    pub const KIND_TUNNEL: u8 = 2;

    /// Builds flags from their parts (`kind` masked to 2 bits).
    pub const fn new(good: bool, congested: bool, kind: u8) -> Self {
        NbrFlags((good as u8) | ((congested as u8) << 1) | ((kind & 0x03) << 2))
    }
    pub const fn good(self) -> bool {
        self.0 & 0x01 != 0
    }
    pub const fn congested(self) -> bool {
        self.0 & 0x02 != 0
    }
    pub const fn kind(self) -> u8 {
        (self.0 >> 2) & 0x03
    }
}

/// One NBR entry: `node_id u16, q u8, flags u8, vcap u8`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct NbrEntry {
    pub node_id: u16,
    pub q: u8,
    pub flags: NbrFlags,
    pub vcap: u8,
}

/// 0x04 NBR v1: `ver u8=1, nseq u8, count u8, count × entry`, count ≤ 8.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Nbr {
    pub nseq: u8,
    pub count: u8,
    pub entries: [NbrEntry; Nbr::MAX_ENTRIES],
}

impl Nbr {
    /// NBR protocol version.
    pub const VERSION: u8 = 1;
    /// At most 8 entries.
    pub const MAX_ENTRIES: usize = 8;

    /// Builds an NBR from up to 8 entries.
    pub fn new(nseq: u8, list: &[NbrEntry]) -> Result<Self> {
        check_max("NBR count", list.len(), Self::MAX_ENTRIES)?;
        let mut entries = [NbrEntry::default(); Self::MAX_ENTRIES];
        entries[..list.len()].copy_from_slice(list);
        Ok(Nbr {
            nseq,
            count: list.len() as u8,
            entries,
        })
    }

    /// The valid entries.
    pub fn entries(&self) -> &[NbrEntry] {
        let n = usize::from(self.count).min(Self::MAX_ENTRIES);
        &self.entries[..n]
    }
}

/// 0x05 LSTAT: `rx_frames u16, rx_bytes u32, gaps u16, pace_kbps u16, queue_ms u8`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Lstat {
    pub rx_frames: u16,
    pub rx_bytes: u32,
    pub gaps: u16,
    pub pace_kbps: u16,
    pub queue_ms: u8,
}

/// 0x06 CAPS (21 B) with the anti-entropy summary (§8.9).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Caps {
    pub proto: u8,
    /// 0 ios, 1 android, 2 websim.
    pub platform: u8,
    pub app_ver: u16,
    /// b0 opus 1.6.1, b1 DRED v-current, b2 OSCE, b3 reserved (v1 is wideband-only).
    pub codec: u16,
    /// b0 l2cap, b1 gatt, b2 aware-ios, b3 aware-android, b4 tunnel.
    pub transports: u8,
    pub max_links: u8,
    /// 0 member, 1 leader.
    pub role: u8,
    pub epoch: u32,
    pub roster_version: u32,
    pub ctl_version: u32,
}

impl Caps {
    pub const PLATFORM_IOS: u8 = 0;
    pub const PLATFORM_ANDROID: u8 = 1;
    pub const PLATFORM_WEBSIM: u8 = 2;
    pub const CODEC_OPUS_161: u16 = 1 << 0;
    pub const CODEC_DRED: u16 = 1 << 1;
    pub const CODEC_OSCE: u16 = 1 << 2;
    pub const T_L2CAP: u8 = 1 << 0;
    pub const T_GATT: u8 = 1 << 1;
    pub const T_AWARE_IOS: u8 = 1 << 2;
    pub const T_AWARE_ANDROID: u8 = 1 << 3;
    pub const T_TUNNEL: u8 = 1 << 4;
    /// Encoded size.
    pub const LEN: usize = 21;
}

/// UPGRADE `op`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum UpgradeOp {
    Offer = 1,
    Accept = 2,
    Reject = 3,
    Ready = 4,
    Retire = 5,
}

/// UPGRADE `transport`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum UpgradeTransport {
    AwareIos = 1,
    AwareAndroid = 2,
    Tunnel = 3,
}

/// 0x09 UPGRADE: `op u8, transport u8, nonce u32, params[..]`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Upgrade<'a> {
    pub op: UpgradeOp,
    pub transport: UpgradeTransport,
    pub nonce: u32,
    pub params: &'a [u8],
}

/// 0x0A CLOSE: `reason u8` (REJECT reasons plus 9 redundant, 10 leaving).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Close {
    pub reason: RejectReason,
}

/// 0x0C IDPROOF: `ed_pk[32], x_pk[32], sig[64]` (128 B).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IdProof {
    pub ed_pk: [u8; 32],
    pub x_pk: [u8; 32],
    pub sig: [u8; 64],
}

impl IdProof {
    /// Signing context (§4.4): `"cvic/v1/idproof" ‖ role u8 ‖ handshake_hash(32)`.
    pub const CONTEXT: &'static [u8] = b"cvic/v1/idproof";

    /// The signed message for `role` (0 initiator, 1 responder).
    pub fn signed_message(role: u8, handshake_hash: &[u8; 32]) -> Vec<u8> {
        let mut m = Vec::with_capacity(Self::CONTEXT.len() + 33);
        m.put(Self::CONTEXT);
        m.put_u8(role);
        m.put(handshake_hash);
        m
    }
}

/// SYNC `kind`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
#[repr(u8)]
pub enum SyncKind {
    Roster = 1,
    Rekey = 2,
    GroupCtl = 3,
    JoinReq = 4,
}

impl SyncKind {
    pub fn from_u8(v: u8) -> Result<Self> {
        Ok(match v {
            1 => Self::Roster,
            2 => Self::Rekey,
            3 => Self::GroupCtl,
            4 => Self::JoinReq,
            _ => {
                return Err(WireError::BadValue {
                    what: "SYNC kind",
                    value: u32::from(v),
                });
            }
        })
    }
}

/// 0x0F SYNC: `kind u8, msg_id u16, seg u8 (idx<<4 | count−1), chunk[≤ 900]`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Sync<'a> {
    pub kind: SyncKind,
    pub msg_id: u16,
    pub idx: u8,
    pub count: u8,
    pub chunk: &'a [u8],
}

impl Sync<'_> {
    /// Largest chunk.
    pub const MAX_CHUNK: usize = crate::mesh::segment::MAX_SEGMENT_DATA;
}

fn fixed<'a>(body: &'a [u8], len: usize, what: &'static str) -> Result<Reader<'a>> {
    if body.len() < len {
        return Err(WireError::Truncated { what });
    }
    if body.len() > len {
        return Err(WireError::TrailingBytes { what });
    }
    Ok(Reader::new(body))
}

impl<'a> Record<'a> {
    /// The record's type byte.
    pub fn rtype(&self) -> u8 {
        match self {
            Record::Mesh(_) => rtype::MESH,
            Record::Ping(_) => rtype::PING,
            Record::Pong(_) => rtype::PONG,
            Record::Nbr(_) => rtype::NBR,
            Record::Lstat(_) => rtype::LSTAT,
            Record::Caps(_) => rtype::CAPS,
            Record::Lprobe(_) => rtype::LPROBE,
            Record::Lecho(_) => rtype::LECHO,
            Record::Upgrade(_) => rtype::UPGRADE,
            Record::Close(_) => rtype::CLOSE,
            Record::LabCtl(_) => rtype::LAB_CTL,
            Record::IdProof(_) => rtype::IDPROOF,
            Record::LabGps(_) => rtype::LAB_GPS,
            Record::Pad(_) => rtype::PAD,
            Record::Sync(_) => rtype::SYNC,
            Record::Unknown { rtype, .. } => *rtype,
        }
    }

    /// Decodes a body of type `rt`.
    pub fn decode(rt: u8, body: &'a [u8]) -> Result<Self> {
        Ok(match rt {
            rtype::MESH => {
                if body.len() < crate::mesh::MeshHeader::LEN {
                    return Err(WireError::Truncated {
                        what: "MESH record",
                    });
                }
                Record::Mesh(body)
            }
            rtype::PING => {
                let mut r = fixed(body, 6, "PING")?;
                Record::Ping(Ping {
                    id: r.u16("id")?,
                    t_tx: r.u32("t_tx")?,
                })
            }
            rtype::PONG => {
                let mut r = fixed(body, 8, "PONG")?;
                Record::Pong(Pong {
                    id: r.u16("id")?,
                    t_tx: r.u32("t_tx")?,
                    hold_us: r.u16("hold")?,
                })
            }
            rtype::NBR => Record::Nbr(decode_nbr(body)?),
            rtype::LSTAT => {
                let mut r = fixed(body, 11, "LSTAT")?;
                Record::Lstat(Lstat {
                    rx_frames: r.u16("rx_frames")?,
                    rx_bytes: r.u32("rx_bytes")?,
                    gaps: r.u16("gaps")?,
                    pace_kbps: r.u16("pace_kbps")?,
                    queue_ms: r.u8("queue_ms")?,
                })
            }
            rtype::CAPS => Record::Caps(decode_caps(body)?),
            rtype::LPROBE => Record::Lprobe(Lprobe::decode(body)?),
            rtype::LECHO => Record::Lecho(Lecho::decode(body)?),
            rtype::UPGRADE => Record::Upgrade(decode_upgrade(body)?),
            rtype::CLOSE => {
                let mut r = fixed(body, 1, "CLOSE")?;
                Record::Close(Close {
                    reason: RejectReason::from_u8(r.u8("reason")?, true)?,
                })
            }
            rtype::LAB_CTL => Record::LabCtl(LabCtl::decode(body)?),
            rtype::IDPROOF => {
                let mut r = fixed(body, 128, "IDPROOF")?;
                Record::IdProof(IdProof {
                    ed_pk: r.array("ed_pk")?,
                    x_pk: r.array("x_pk")?,
                    sig: r.array("sig")?,
                })
            }
            rtype::LAB_GPS => Record::LabGps(LabGps::decode(body)?),
            rtype::PAD => Record::Pad(body.len() as u16),
            rtype::SYNC => Record::Sync(decode_sync(body)?),
            other => Record::Unknown { rtype: other, body },
        })
    }

    /// Encodes the body (without `rtype` and length).
    pub fn encode_body(&self) -> Result<Vec<u8>> {
        let mut o = Vec::new();
        match *self {
            Record::Mesh(p) => {
                if p.len() < crate::mesh::MeshHeader::LEN {
                    return Err(WireError::Truncated {
                        what: "MESH record",
                    });
                }
                o.put(p);
            }
            Record::Ping(p) => {
                o.put_u16(p.id);
                o.put_u32(p.t_tx);
            }
            Record::Pong(p) => {
                o.put_u16(p.id);
                o.put_u32(p.t_tx);
                o.put_u16(p.hold_us);
            }
            Record::Nbr(n) => encode_nbr(&n, &mut o)?,
            Record::Lstat(s) => {
                o.put_u16(s.rx_frames);
                o.put_u32(s.rx_bytes);
                o.put_u16(s.gaps);
                o.put_u16(s.pace_kbps);
                o.put_u8(s.queue_ms);
            }
            Record::Caps(c) => encode_caps(&c, &mut o),
            Record::Lprobe(p) => p.encode(&mut o)?,
            Record::Lecho(e) => e.encode(&mut o),
            Record::Upgrade(u) => {
                o.put_u8(u.op as u8);
                o.put_u8(u.transport as u8);
                o.put_u32(u.nonce);
                o.put(u.params);
            }
            Record::Close(c) => o.put_u8(c.reason as u8),
            Record::LabCtl(c) => c.encode(&mut o),
            Record::IdProof(p) => {
                o.put(&p.ed_pk);
                o.put(&p.x_pk);
                o.put(&p.sig);
            }
            Record::LabGps(g) => g.encode(&mut o),
            Record::Pad(n) => o.resize(usize::from(n), 0),
            Record::Sync(s) => encode_sync(&s, &mut o)?,
            Record::Unknown { body, .. } => o.put(body),
        }
        check_max("record body", o.len(), super::MAX_RECORD_BODY)?;
        Ok(o)
    }

    /// Encodes the whole record (`rtype` · LEB128 len · body).
    pub fn encode(&self) -> Result<Vec<u8>> {
        let body = self.encode_body()?;
        let mut out = Vec::with_capacity(super::record_len(body.len()));
        out.push(self.rtype());
        super::put_leb128(&mut out, body.len())?;
        out.extend_from_slice(&body);
        Ok(out)
    }
}

fn decode_nbr(body: &[u8]) -> Result<Nbr> {
    let mut r = Reader::new(body);
    let ver = r.u8("NBR ver")?;
    if ver != Nbr::VERSION {
        return Err(WireError::BadValue {
            what: "NBR ver",
            value: u32::from(ver),
        });
    }
    let nseq = r.u8("NBR nseq")?;
    let count = r.u8("NBR count")?;
    check_max("NBR count", usize::from(count), Nbr::MAX_ENTRIES)?;
    let mut entries = [NbrEntry::default(); Nbr::MAX_ENTRIES];
    for e in entries.iter_mut().take(usize::from(count)) {
        *e = NbrEntry {
            node_id: r.u16("NBR node_id")?,
            q: r.u8("NBR q")?,
            flags: NbrFlags(r.u8("NBR flags")?),
            vcap: r.u8("NBR vcap")?,
        };
    }
    r.finish("NBR")?;
    Ok(Nbr {
        nseq,
        count,
        entries,
    })
}

fn encode_nbr(n: &Nbr, o: &mut Vec<u8>) -> Result<()> {
    check_max("NBR count", usize::from(n.count), Nbr::MAX_ENTRIES)?;
    o.put_u8(Nbr::VERSION);
    o.put_u8(n.nseq);
    o.put_u8(n.count);
    for e in n.entries() {
        o.put_u16(e.node_id);
        o.put_u8(e.q);
        o.put_u8(e.flags.0);
        o.put_u8(e.vcap);
    }
    Ok(())
}

fn decode_caps(body: &[u8]) -> Result<Caps> {
    let mut r = fixed(body, Caps::LEN, "CAPS")?;
    Ok(Caps {
        proto: r.u8("proto")?,
        platform: r.u8("platform")?,
        app_ver: r.u16("app_ver")?,
        codec: r.u16("codec")?,
        transports: r.u8("transports")?,
        max_links: r.u8("max_links")?,
        role: r.u8("role")?,
        epoch: r.u32("epoch")?,
        roster_version: r.u32("roster_version")?,
        ctl_version: r.u32("ctl_version")?,
    })
}

fn encode_caps(c: &Caps, o: &mut Vec<u8>) {
    o.put_u8(c.proto);
    o.put_u8(c.platform);
    o.put_u16(c.app_ver);
    o.put_u16(c.codec);
    o.put_u8(c.transports);
    o.put_u8(c.max_links);
    o.put_u8(c.role);
    o.put_u32(c.epoch);
    o.put_u32(c.roster_version);
    o.put_u32(c.ctl_version);
}

impl UpgradeOp {
    pub fn from_u8(v: u8) -> Result<Self> {
        Ok(match v {
            1 => Self::Offer,
            2 => Self::Accept,
            3 => Self::Reject,
            4 => Self::Ready,
            5 => Self::Retire,
            _ => {
                return Err(WireError::BadValue {
                    what: "UPGRADE op",
                    value: u32::from(v),
                });
            }
        })
    }
}

impl UpgradeTransport {
    pub fn from_u8(v: u8) -> Result<Self> {
        Ok(match v {
            1 => Self::AwareIos,
            2 => Self::AwareAndroid,
            3 => Self::Tunnel,
            _ => {
                return Err(WireError::BadValue {
                    what: "UPGRADE transport",
                    value: u32::from(v),
                });
            }
        })
    }
}

fn decode_upgrade(body: &[u8]) -> Result<Upgrade<'_>> {
    let mut r = Reader::new(body);
    Ok(Upgrade {
        op: UpgradeOp::from_u8(r.u8("UPGRADE op")?)?,
        transport: UpgradeTransport::from_u8(r.u8("UPGRADE transport")?)?,
        nonce: r.u32("UPGRADE nonce")?,
        params: r.rest(),
    })
}

fn decode_sync(body: &[u8]) -> Result<Sync<'_>> {
    let mut r = Reader::new(body);
    let kind = SyncKind::from_u8(r.u8("SYNC kind")?)?;
    let msg_id = r.u16("SYNC msg_id")?;
    let (idx, count) = crate::mesh::segment::split_seg_byte(r.u8("SYNC seg")?)?;
    let chunk = r.rest();
    check_max("SYNC chunk", chunk.len(), Sync::MAX_CHUNK)?;
    Ok(Sync {
        kind,
        msg_id,
        idx,
        count,
        chunk,
    })
}

fn encode_sync(s: &Sync<'_>, o: &mut Vec<u8>) -> Result<()> {
    check_max("SYNC chunk", s.chunk.len(), Sync::MAX_CHUNK)?;
    let seg = crate::mesh::segment::seg_byte(s.idx, s.count)?;
    o.put_u8(s.kind as u8);
    o.put_u16(s.msg_id);
    o.put_u8(seg);
    o.put(s.chunk);
    Ok(())
}
