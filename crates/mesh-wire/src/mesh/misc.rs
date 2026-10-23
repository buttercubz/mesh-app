//! MESH_PROBE, PATH_HINT, POSITION and EXT payloads (DESIGN §4.6).

use super::voice::EncoderProfile;
use crate::codec::{PutLe, Reader, check_max};
use crate::error::{Result, WireError};

/// 0x0A MESH_PROBE: `probe_id u32, t_origin u64, n u8, trace[n] u16` (relays append
/// their node_id, n ≤ 8): 13 + 2n bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct MeshProbe {
    pub probe_id: u32,
    pub t_origin: u64,
    pub n: u8,
    pub trace: [u16; MeshProbe::MAX_TRACE],
}

impl MeshProbe {
    pub const MAX_TRACE: usize = 8;
    pub const FIXED: usize = 13;

    pub fn new(probe_id: u32, t_origin: u64) -> Self {
        MeshProbe {
            probe_id,
            t_origin,
            n: 0,
            trace: [0; Self::MAX_TRACE],
        }
    }

    /// The trace so far.
    pub fn trace(&self) -> &[u16] {
        &self.trace[..usize::from(self.n).min(Self::MAX_TRACE)]
    }

    /// A relay appends its node_id; refused once the trace holds 8 entries.
    pub fn append(&mut self, node_id: u16) -> Result<()> {
        let n = usize::from(self.n);
        if n >= Self::MAX_TRACE {
            return Err(WireError::NoSpace {
                what: "MESH_PROBE trace",
            });
        }
        self.trace[n] = node_id;
        self.n += 1;
        Ok(())
    }

    pub fn decode(p: &[u8]) -> Result<Self> {
        let mut r = Reader::new(p);
        let mut v = MeshProbe::new(r.u32("probe_id")?, r.u64("t_origin")?);
        let n = r.u8("probe n")?;
        check_max("MESH_PROBE n", usize::from(n), Self::MAX_TRACE)?;
        for slot in v.trace.iter_mut().take(usize::from(n)) {
            *slot = r.u16("probe trace")?;
        }
        v.n = n;
        r.finish("MESH_PROBE")?;
        Ok(v)
    }

    pub fn encode(&self) -> Result<Vec<u8>> {
        check_max("MESH_PROBE n", usize::from(self.n), Self::MAX_TRACE)?;
        let mut o = Vec::with_capacity(Self::FIXED + 2 * usize::from(self.n));
        o.put_u32(self.probe_id);
        o.put_u64(self.t_origin);
        o.put_u8(self.n);
        for t in self.trace() {
            o.put_u16(*t);
        }
        Ok(o)
    }
}

/// 0x0B PATH_HINT (5 B): `profile u8 (1 BLE16, 2 BLE12), at u16, target u16 (0 = all)`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PathHint {
    pub profile: EncoderProfile,
    pub at: u16,
    pub target: u16,
}

impl PathHint {
    pub const LEN: usize = 5;
    /// `target` value meaning every origin.
    pub const ALL: u16 = 0;

    pub fn decode(p: &[u8]) -> Result<Self> {
        let mut r = Reader::new(p);
        let profile = EncoderProfile::from_u8(r.u8("PATH_HINT profile")?)?;
        if !matches!(profile, EncoderProfile::Ble16 | EncoderProfile::Ble12) {
            return Err(WireError::BadValue {
                what: "PATH_HINT profile",
                value: profile as u32,
            });
        }
        let v = PathHint {
            profile,
            at: r.u16("PATH_HINT at")?,
            target: r.u16("PATH_HINT target")?,
        };
        r.finish("PATH_HINT")?;
        Ok(v)
    }

    pub fn encode(&self) -> Result<Vec<u8>> {
        if !matches!(self.profile, EncoderProfile::Ble16 | EncoderProfile::Ble12) {
            return Err(WireError::BadValue {
                what: "PATH_HINT profile",
                value: self.profile as u32,
            });
        }
        let mut o = Vec::with_capacity(Self::LEN);
        o.put_u8(self.profile as u8);
        o.put_u16(self.at);
        o.put_u16(self.target);
        Ok(o)
    }
}

/// 0x10 POSITION (14 B): `lat_e7 i32, lon_e7 i32, speed_dms u16 (0.1 m/s),
/// heading_2deg u8, acc_m u8, fix_age_100ms u8, rsv u8`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Position {
    pub lat_e7: i32,
    pub lon_e7: i32,
    pub speed_dms: u16,
    pub heading_2deg: u8,
    pub acc_m: u8,
    pub fix_age_100ms: u8,
    pub rsv: u8,
}

impl Position {
    pub const LEN: usize = 14;

    pub fn decode(p: &[u8]) -> Result<Self> {
        let mut r = Reader::new(p);
        let v = Position {
            lat_e7: r.i32("lat_e7")?,
            lon_e7: r.i32("lon_e7")?,
            speed_dms: r.u16("speed_dms")?,
            heading_2deg: r.u8("heading_2deg")?,
            acc_m: r.u8("acc_m")?,
            fix_age_100ms: r.u8("fix_age")?,
            rsv: r.u8("rsv")?,
        };
        r.finish("POSITION")?;
        Ok(v)
    }

    pub fn encode(&self) -> Vec<u8> {
        let mut o = Vec::with_capacity(Self::LEN);
        o.put_i32(self.lat_e7);
        o.put_i32(self.lon_e7);
        o.put_u16(self.speed_dms);
        o.put_u8(self.heading_2deg);
        o.put_u8(self.acc_m);
        o.put_u8(self.fix_age_100ms);
        o.put_u8(self.rsv);
        o
    }
}

/// 0x1F EXT: `ext_type u8` + body (future extensions).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Ext<'a> {
    pub ext_type: u8,
    pub body: &'a [u8],
}

impl<'a> Ext<'a> {
    pub fn decode(p: &'a [u8]) -> Result<Self> {
        let mut r = Reader::new(p);
        Ok(Ext {
            ext_type: r.u8("ext_type")?,
            body: r.rest(),
        })
    }

    pub fn encode(&self) -> Vec<u8> {
        let mut o = Vec::with_capacity(1 + self.body.len());
        o.put_u8(self.ext_type);
        o.put(self.body);
        o
    }
}
