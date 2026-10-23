//! LAB_CTL records (DESIGN §4.4, §12.1) as used by mesh-core:
//!
//! - `start`: `run_id u32`, `bench` (mesh-lab wire id), params =
//!   `RunParams::encode_ctl()` ‖ `unix_minutes u32` (the controller's wall clock, so
//!   every phone names the run `r-YYYYMMDD-HHMM-xxxx` identically);
//! - `stop`: `run_id`, empty params;
//! - `mark`: `run_id`, params = the label (UTF-8, ≤ 64 chars);
//! - `announce`: params = `lab_node_id u16, nonce u64`; `run_id` = 0 when the sender
//!   announces itself, 1 when it relays a neighbor's announce one hop further (collision
//!   detection between two neighbors of the same node, which are not linked);
//! - `pattern`: reserved for later bench variants; ignored.

use mesh_lab::{Bench, RunParams};
use mesh_wire::record::{LabCtl, LabCtlOp, Record};

/// `run_id` of a direct announce.
pub const ANNOUNCE_DIRECT: u32 = 0;
/// `run_id` of a relayed announce.
pub const ANNOUNCE_RELAYED: u32 = 1;

/// A decoded `start`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StartCtl {
    pub run_id: u32,
    pub params: RunParams,
    pub unix_minutes: u32,
}

/// Encodes `start` params.
pub fn start_params(params: &RunParams, unix_minutes: u32) -> Vec<u8> {
    let mut p = params.encode_ctl();
    p.extend_from_slice(&unix_minutes.to_le_bytes());
    p
}

/// Decodes a `start` LAB_CTL.
pub fn decode_start(c: &LabCtl<'_>) -> Option<StartCtl> {
    if c.op != LabCtlOp::Start {
        return None;
    }
    let bench = Bench::from_wire_id(c.bench)?;
    let vlen = usize::from(*c.params.get(17)?);
    let split = 18 + vlen;
    let ctl = c.params.get(..split)?;
    let params = RunParams::decode_ctl(bench, ctl).ok()?;
    let unix_minutes = match c.params.get(split..) {
        Some(m) if m.len() == 4 => u32::from_le_bytes([m[0], m[1], m[2], m[3]]),
        Some([]) => 0,
        _ => return None,
    };
    Some(StartCtl {
        run_id: c.run_id,
        params,
        unix_minutes,
    })
}

/// A LAB_CTL record borrowing `params`.
pub fn record<'a>(op: LabCtlOp, run_id: u32, bench: u8, params: &'a [u8]) -> Record<'a> {
    Record::LabCtl(LabCtl {
        op,
        run_id,
        bench,
        params,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use mesh_lab::LabParams;
    use mesh_wire::record::RecordIter;

    #[test]
    fn start_round_trip() {
        let mut p = LabParams::defaults_for(Bench::T0b, false);
        p.variant = Some("pocket".into());
        let rp = RunParams::resolve(Bench::T0b, &p).unwrap();
        let params = start_params(&rp, 29_876_543);
        let rec = record(LabCtlOp::Start, 77, Bench::T0b.wire_id(), &params);
        let body = rec.encode().unwrap();
        let parsed = RecordIter::new(&body).next().unwrap().unwrap();
        let Record::LabCtl(c) = parsed else {
            panic!("not LAB_CTL");
        };
        let s = decode_start(&c).unwrap();
        assert_eq!((s.run_id, s.unix_minutes), (77, 29_876_543));
        assert_eq!(s.params, rp);
        let short = LabCtl {
            params: &params[..10],
            ..c
        };
        assert_eq!(decode_start(&short), None);
    }
}
