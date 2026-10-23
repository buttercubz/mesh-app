//! Lab-link records (DESIGN §4.4, §12): LPROBE, LECHO, LAB_CTL, LAB_GPS.

use crate::codec::{PutLe, Reader, check_max};
use crate::error::{Result, WireError};

/// 0x07 LPROBE: `probe_id u32, t_tx u64, flags u8, pad[..]` (13 + pad). The pad is
/// sent as zero bytes; its content is ignored on receive.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Lprobe {
    pub probe_id: u32,
    pub t_tx: u64,
    pub flags: u8,
    pub pad_len: u16,
}

impl Lprobe {
    /// Fixed part.
    pub const FIXED: usize = 13;

    pub(crate) fn decode(body: &[u8]) -> Result<Self> {
        let mut r = Reader::new(body);
        let probe_id = r.u32("LPROBE probe_id")?;
        let t_tx = r.u64("LPROBE t_tx")?;
        let flags = r.u8("LPROBE flags")?;
        let pad = r.rest();
        Ok(Lprobe {
            probe_id,
            t_tx,
            flags,
            pad_len: pad.len() as u16,
        })
    }

    pub(crate) fn encode(&self, o: &mut Vec<u8>) -> Result<()> {
        check_max(
            "LPROBE pad",
            usize::from(self.pad_len),
            super::MAX_RECORD_BODY - Self::FIXED,
        )?;
        o.put_u32(self.probe_id);
        o.put_u64(self.t_tx);
        o.put_u8(self.flags);
        o.resize(o.len() + usize::from(self.pad_len), 0);
        Ok(())
    }
}

/// 0x08 LECHO: `probe_id u32, t_tx u64 (echo), hold_us u32, rx_count u32` (20 B).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Lecho {
    pub probe_id: u32,
    pub t_tx: u64,
    pub hold_us: u32,
    pub rx_count: u32,
}

impl Lecho {
    pub const LEN: usize = 20;

    pub(crate) fn decode(body: &[u8]) -> Result<Self> {
        let mut r = exact(body, Self::LEN, "LECHO")?;
        Ok(Lecho {
            probe_id: r.u32("probe_id")?,
            t_tx: r.u64("t_tx")?,
            hold_us: r.u32("hold_us")?,
            rx_count: r.u32("rx_count")?,
        })
    }

    pub(crate) fn encode(&self, o: &mut Vec<u8>) {
        o.put_u32(self.probe_id);
        o.put_u64(self.t_tx);
        o.put_u32(self.hold_us);
        o.put_u32(self.rx_count);
    }
}

/// LAB_CTL `op`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum LabCtlOp {
    Start = 1,
    Stop = 2,
    Mark = 3,
    Pattern = 4,
    Announce = 5,
}

impl LabCtlOp {
    pub fn from_u8(v: u8) -> Result<Self> {
        Ok(match v {
            1 => Self::Start,
            2 => Self::Stop,
            3 => Self::Mark,
            4 => Self::Pattern,
            5 => Self::Announce,
            _ => {
                return Err(WireError::BadValue {
                    what: "LAB_CTL op",
                    value: u32::from(v),
                });
            }
        })
    }
}

/// 0x0B LAB_CTL: `op u8, run_id u32, bench u8, params[..]`; `announce` params are
/// `lab_node_id u16, nonce u64`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LabCtl<'a> {
    pub op: LabCtlOp,
    pub run_id: u32,
    pub bench: u8,
    pub params: &'a [u8],
}

impl<'a> LabCtl<'a> {
    pub(crate) fn decode(body: &'a [u8]) -> Result<Self> {
        let mut r = Reader::new(body);
        let ctl = LabCtl {
            op: LabCtlOp::from_u8(r.u8("LAB_CTL op")?)?,
            run_id: r.u32("LAB_CTL run_id")?,
            bench: r.u8("LAB_CTL bench")?,
            params: r.rest(),
        };
        if ctl.op == LabCtlOp::Announce {
            ctl.announce()?;
        }
        Ok(ctl)
    }

    pub(crate) fn encode(&self, o: &mut Vec<u8>) {
        o.put_u8(self.op as u8);
        o.put_u32(self.run_id);
        o.put_u8(self.bench);
        o.put(self.params);
    }

    /// Encodes `announce` params: `lab_node_id u16, nonce u64` (10 B).
    pub fn announce_params(lab_node_id: u16, nonce: u64) -> [u8; 10] {
        let mut p = [0u8; 10];
        p[..2].copy_from_slice(&lab_node_id.to_le_bytes());
        p[2..].copy_from_slice(&nonce.to_le_bytes());
        p
    }

    /// Reads `announce` params as `(lab_node_id, nonce)`.
    pub fn announce(&self) -> Result<(u16, u64)> {
        let mut r = exact(self.params, 10, "LAB_CTL announce")?;
        Ok((r.u16("lab_node_id")?, r.u64("nonce")?))
    }
}

/// 0x0D LAB_GPS: `t_ms u32, lat_e7 i32, lon_e7 i32, speed_cms u16, acc_dm u16,
/// heading_cdeg u16` (18 B).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LabGps {
    pub t_ms: u32,
    pub lat_e7: i32,
    pub lon_e7: i32,
    pub speed_cms: u16,
    pub acc_dm: u16,
    pub heading_cdeg: u16,
}

impl LabGps {
    pub const LEN: usize = 18;

    pub(crate) fn decode(body: &[u8]) -> Result<Self> {
        let mut r = exact(body, Self::LEN, "LAB_GPS")?;
        Ok(LabGps {
            t_ms: r.u32("t_ms")?,
            lat_e7: r.i32("lat_e7")?,
            lon_e7: r.i32("lon_e7")?,
            speed_cms: r.u16("speed_cms")?,
            acc_dm: r.u16("acc_dm")?,
            heading_cdeg: r.u16("heading_cdeg")?,
        })
    }

    pub(crate) fn encode(&self, o: &mut Vec<u8>) {
        o.put_u32(self.t_ms);
        o.put_i32(self.lat_e7);
        o.put_i32(self.lon_e7);
        o.put_u16(self.speed_cms);
        o.put_u16(self.acc_dm);
        o.put_u16(self.heading_cdeg);
    }
}

fn exact<'a>(body: &'a [u8], len: usize, what: &'static str) -> Result<Reader<'a>> {
    if body.len() < len {
        return Err(WireError::Truncated { what });
    }
    if body.len() > len {
        return Err(WireError::TrailingBytes { what });
    }
    Ok(Reader::new(body))
}
