//! Run parameters: the `labStart` inputs (mesh-api `LabParams`, mirrored here because
//! mesh-lab depends only on mesh-wire), bench-default resolution, range checks and the
//! `LAB_CTL{start}` params encoding that carries them to the other phones.

use crate::catalog::spec;
use crate::kinds::{Anchor, Audio, Bench, LinkRole, Pattern, Transport};

/// Inputs of a run. `audio` and `anchor` are mandatory (§12.1); `None` fields take the
/// bench defaults of the catalog.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LabParams {
    pub audio: Audio,
    pub anchor: Anchor,
    /// Per-run GPS consent.
    pub gps: bool,
    pub duration_s: Option<u32>,
    pub pattern: Option<Pattern>,
    pub streams: Option<u8>,
    pub payload_b: Option<u16>,
    pub interval_ms: Option<u16>,
    pub ping_hz: Option<u8>,
    pub transport: Option<Transport>,
    pub role: Option<LinkRole>,
    pub headset: Option<bool>,
    pub mixable: Option<bool>,
    /// Free variant label (user content, ≤ 64 B).
    pub variant: Option<String>,
}

impl LabParams {
    /// Only the mandatory fields.
    pub fn new(audio: Audio, anchor: Anchor, gps: bool) -> Self {
        LabParams {
            audio,
            anchor,
            gps,
            duration_s: None,
            pattern: None,
            streams: None,
            payload_b: None,
            interval_ms: None,
            ping_hz: None,
            transport: None,
            role: None,
            headset: None,
            mixable: None,
            variant: None,
        }
    }

    /// The bench's default audio path and anchor, everything else left to defaults.
    pub fn defaults_for(bench: Bench, gps: bool) -> Self {
        let s = spec(bench);
        Self::new(s.audio, s.anchor, gps)
    }
}

/// A parameter that failed its range check.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParamError {
    Duration,
    Streams,
    Payload,
    Interval,
    PingHz,
    Variant,
    /// `LAB_CTL` params bytes are malformed.
    Encoding,
}

/// Fully resolved parameters of a run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunParams {
    pub bench: Bench,
    pub audio: Audio,
    pub anchor: Anchor,
    pub gps: bool,
    /// 0 = until stopped.
    pub duration_s: u32,
    pub pattern: Pattern,
    pub streams: u8,
    pub payload_b: u16,
    pub interval_ms: u16,
    pub ping_hz: u8,
    pub transport: Transport,
    pub role: LinkRole,
    pub headset: bool,
    pub mixable: bool,
    pub variant: Option<String>,
}

/// Smallest ping/probe on-air size (LPROBE fixed part + framing).
pub const MIN_PAYLOAD_B: u16 = 40;
/// Largest on-air unit on a stream link (1024 B frame + 2 B prefix).
pub const MAX_PAYLOAD_B: u16 = 1026;
/// Longest run (24 h).
pub const MAX_DURATION_S: u32 = 86_400;

impl RunParams {
    /// Applies the bench defaults and checks ranges.
    pub fn resolve(bench: Bench, p: &LabParams) -> Result<Self, ParamError> {
        let d = spec(bench).defaults;
        let r = RunParams {
            bench,
            audio: p.audio,
            anchor: p.anchor,
            gps: p.gps,
            duration_s: p.duration_s.unwrap_or(d.duration_s),
            pattern: p.pattern.unwrap_or(d.pattern),
            streams: p.streams.unwrap_or(d.streams),
            payload_b: p.payload_b.unwrap_or(d.payload_b),
            interval_ms: p.interval_ms.unwrap_or(d.interval_ms),
            ping_hz: p.ping_hz.unwrap_or(d.ping_hz),
            transport: p.transport.unwrap_or(if bench == Bench::T4 {
                Transport::Aware
            } else {
                Transport::L2cap
            }),
            role: p.role.unwrap_or(LinkRole::Central),
            headset: p.headset.unwrap_or(p.audio == Audio::Full),
            mixable: p.mixable.unwrap_or(false),
            variant: p.variant.clone(),
        };
        r.check()?;
        Ok(r)
    }

    fn check(&self) -> Result<(), ParamError> {
        if self.duration_s > MAX_DURATION_S || (self.duration_s == 0 && self.bench != Bench::Ride) {
            return Err(ParamError::Duration);
        }
        let voice = matches!(self.pattern, Pattern::VoiceLike | Pattern::PingUnderLoad);
        if self.streams > 4 || (voice && self.streams == 0) {
            return Err(ParamError::Streams);
        }
        let needs_payload = !matches!(self.pattern, Pattern::Idle);
        if needs_payload && !(MIN_PAYLOAD_B..=MAX_PAYLOAD_B).contains(&self.payload_b) {
            return Err(ParamError::Payload);
        }
        if !(5..=10_000).contains(&self.interval_ms) {
            return Err(ParamError::Interval);
        }
        let pings = matches!(self.pattern, Pattern::Ping | Pattern::PingUnderLoad);
        if self.ping_hz > 50 || (pings && self.ping_hz == 0) {
            return Err(ParamError::PingHz);
        }
        if self.variant.as_ref().is_some_and(|v| v.len() > 64) {
            return Err(ParamError::Variant);
        }
        Ok(())
    }

    /// Probes per second a peer running these parameters sends on each link (the
    /// nominal CBR rate; 0 for saturate and passive patterns).
    pub fn nominal_probe_rate(&self) -> f64 {
        let voice = f64::from(self.streams) * 1_000.0 / f64::from(self.interval_ms.max(1));
        match self.pattern {
            Pattern::VoiceLike => voice,
            Pattern::PingUnderLoad => voice + f64::from(self.ping_hz),
            Pattern::Ping => f64::from(self.ping_hz),
            Pattern::Saturate | Pattern::Idle => 0.0,
        }
    }

    /// Run length in µs, `None` when the run lasts until stopped.
    pub fn duration_us(&self) -> Option<u64> {
        (self.duration_s > 0).then(|| u64::from(self.duration_s) * 1_000_000)
    }

    /// `LAB_CTL{start}` params: `ver u8=1 · audio u8 · anchor u8 · flags u8 (b0 gps,
    /// b1 headset, b2 mixable) · duration_s u32 · pattern u8 · streams u8 · payload_b u16
    /// · interval_ms u16 · ping_hz u8 · transport u8 · role u8 · vlen u8 · variant[vlen]`
    /// (little-endian, 18 + vlen B). Indices are positions in the enums' `ALL` lists.
    pub fn encode_ctl(&self) -> Vec<u8> {
        let mut o = Vec::with_capacity(18 + self.variant.as_ref().map_or(0, String::len));
        o.push(1);
        o.push(idx(Audio::ALL, self.audio));
        o.push(idx(Anchor::ALL, self.anchor));
        o.push(u8::from(self.gps) | (u8::from(self.headset) << 1) | (u8::from(self.mixable) << 2));
        o.extend_from_slice(&self.duration_s.to_le_bytes());
        o.push(idx(Pattern::ALL, self.pattern));
        o.push(self.streams);
        o.extend_from_slice(&self.payload_b.to_le_bytes());
        o.extend_from_slice(&self.interval_ms.to_le_bytes());
        o.push(self.ping_hz);
        o.push(idx(Transport::ALL, self.transport));
        o.push(idx(LinkRole::ALL, self.role));
        let v = self.variant.as_deref().unwrap_or("").as_bytes();
        let n = v.len().min(64);
        o.push(n as u8);
        o.extend_from_slice(&v[..n]);
        o
    }

    /// Decodes `LAB_CTL{start}` params for `bench` (the follower's side).
    pub fn decode_ctl(bench: Bench, b: &[u8]) -> Result<Self, ParamError> {
        if b.len() < 18 || b[0] != 1 {
            return Err(ParamError::Encoding);
        }
        let get = |all_len: usize, i: u8| -> Result<usize, ParamError> {
            let i = usize::from(i);
            if i < all_len {
                Ok(i)
            } else {
                Err(ParamError::Encoding)
            }
        };
        let u16_at = |i: usize| u16::from_le_bytes([b[i], b[i + 1]]);
        let vlen = usize::from(b[17]);
        if b.len() != 18 + vlen {
            return Err(ParamError::Encoding);
        }
        let variant = if vlen == 0 {
            None
        } else {
            Some(
                core::str::from_utf8(&b[18..])
                    .map_err(|_| ParamError::Encoding)?
                    .to_string(),
            )
        };
        let r = RunParams {
            bench,
            audio: Audio::ALL[get(Audio::ALL.len(), b[1])?],
            anchor: Anchor::ALL[get(Anchor::ALL.len(), b[2])?],
            gps: b[3] & 1 != 0,
            headset: b[3] & 2 != 0,
            mixable: b[3] & 4 != 0,
            duration_s: u32::from_le_bytes([b[4], b[5], b[6], b[7]]),
            pattern: Pattern::ALL[get(Pattern::ALL.len(), b[8])?],
            streams: b[9],
            payload_b: u16_at(10),
            interval_ms: u16_at(12),
            ping_hz: b[14],
            transport: Transport::ALL[get(Transport::ALL.len(), b[15])?],
            role: LinkRole::ALL[get(LinkRole::ALL.len(), b[16])?],
            variant,
        };
        r.check()?;
        Ok(r)
    }
}

fn idx<T: PartialEq + Copy>(all: &[T], v: T) -> u8 {
    all.iter().position(|x| *x == v).unwrap_or(0) as u8
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_bench_defaults() {
        let p = LabParams::defaults_for(Bench::T1, true);
        let r = RunParams::resolve(Bench::T1, &p).unwrap();
        assert_eq!(r.audio, Audio::RenderOnly);
        assert_eq!(r.anchor, Anchor::Audio);
        assert_eq!(r.pattern, Pattern::PingUnderLoad);
        assert_eq!(
            (r.streams, r.payload_b, r.interval_ms, r.ping_hz),
            (2, 118, 40, 5)
        );
        assert!(!r.headset);
        let t0 = RunParams::resolve(
            Bench::T0,
            &LabParams::new(Audio::Full, Anchor::Audio, false),
        )
        .unwrap();
        assert!(t0.headset, "audio full implies the headset variant");
        assert_eq!(t0.duration_us(), Some(60_000_000));
    }

    #[test]
    fn range_checks() {
        let mut p = LabParams::defaults_for(Bench::T0, false);
        p.streams = Some(5);
        assert_eq!(RunParams::resolve(Bench::T0, &p), Err(ParamError::Streams));
        p.streams = None;
        p.payload_b = Some(2000);
        assert_eq!(RunParams::resolve(Bench::T0, &p), Err(ParamError::Payload));
        p.payload_b = None;
        p.pattern = Some(Pattern::Ping);
        p.ping_hz = Some(0);
        assert_eq!(RunParams::resolve(Bench::T0, &p), Err(ParamError::PingHz));
        p.ping_hz = None;
        p.variant = Some("x".repeat(65));
        assert_eq!(RunParams::resolve(Bench::T0, &p), Err(ParamError::Variant));
        p.variant = None;
        p.duration_s = Some(0);
        assert_eq!(RunParams::resolve(Bench::T0, &p), Err(ParamError::Duration));
        assert!(
            RunParams::resolve(Bench::Ride, &LabParams::defaults_for(Bench::Ride, false)).is_ok()
        );
    }

    #[test]
    fn ctl_params_round_trip() {
        let mut p = LabParams::new(Audio::Full, Anchor::AudioLocation, true);
        p.pattern = Some(Pattern::PingUnderLoad);
        p.streams = Some(3);
        p.transport = Some(Transport::GattWrite);
        p.role = Some(LinkRole::Peripheral);
        p.mixable = Some(true);
        p.variant = Some("pocket ñ".into());
        let r = RunParams::resolve(Bench::T1, &p).unwrap();
        let bytes = r.encode_ctl();
        assert_eq!(bytes.len(), 18 + "pocket ñ".len());
        assert_eq!(RunParams::decode_ctl(Bench::T1, &bytes).unwrap(), r);
        assert_eq!(
            RunParams::decode_ctl(Bench::T1, &bytes[..10]),
            Err(ParamError::Encoding)
        );
        let mut bad = bytes.clone();
        bad[1] = 9;
        assert_eq!(
            RunParams::decode_ctl(Bench::T1, &bad),
            Err(ParamError::Encoding)
        );
    }
}
