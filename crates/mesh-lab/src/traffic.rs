//! Bench traffic generators (DESIGN §12.1): LPROBE patterns (saturate, voice-like CBR
//! padded to exact VOICE frame sizes, ping, ping under load) and MESH_PROBE patterns.
//!
//! LPROBE `flags` (owned by mesh-lab, documented in `docs/lab/LAB_PROTOCOL.md`):
//! bits 1-0 class (0 data, 1 ping = echo requested, 2 reply, 3 reserved), bits 4-2 the
//! voice-like stream index, bits 7-5 reserved (0). A ping is answered, in one frame, by
//! a LECHO (`t_tx` echo, `hold_us`, `rx_count`) and a reply LPROBE carrying the same
//! `probe_id` and the responder's transmit time, which gives the initiator all four
//! NTP timestamps.

use mesh_wire::frame::SEC_OVERHEAD;
use mesh_wire::mesh::MeshProbe;
use mesh_wire::record::{Lprobe, leb128_len};

use crate::kinds::Pattern;
use crate::params::RunParams;
use crate::time::{Instant, US_PER_MS, US_PER_S};

/// LPROBE class bits.
pub mod flags {
    pub const CLASS_MASK: u8 = 0b11;
    pub const DATA: u8 = 0;
    pub const PING: u8 = 1;
    pub const REPLY: u8 = 2;
    pub const STREAM_SHIFT: u8 = 2;
    pub const STREAM_MASK: u8 = 0b111 << STREAM_SHIFT;

    /// Data flags for voice-like stream `i` (0-7).
    pub const fn data(stream: u8) -> u8 {
        DATA | ((stream & 0b111) << STREAM_SHIFT)
    }

    /// The class of `f`.
    pub const fn class(f: u8) -> u8 {
        f & CLASS_MASK
    }

    /// The stream index of `f`.
    pub const fn stream(f: u8) -> u8 {
        (f & STREAM_MASK) >> STREAM_SHIFT
    }
}

/// Stream-link on-air bytes added around the records of a LAB frame padded to the SEC
/// size: `len u16` prefix + SEC overhead (§4.3).
pub const STREAM_FRAME_OVERHEAD: usize = 2 + SEC_OVERHEAD;

/// Pad length of an LPROBE that makes its LAB frame, alone in the frame and padded to
/// the SEC size, exactly `on_air` bytes on a stream link (`len u16` + frame). When the
/// LEB128 length boundary makes the exact size unreachable, the result is 1 B short.
pub fn pad_for_on_air(on_air: u16) -> u16 {
    let records = usize::from(on_air).saturating_sub(STREAM_FRAME_OVERHEAD);
    // record = 1 (rtype) + leb(body) + body
    let mut body = records.saturating_sub(2);
    if leb128_len(body) == 2 {
        body = records.saturating_sub(3);
        if leb128_len(body) == 1 {
            body = 127;
        }
    }
    body.saturating_sub(Lprobe::FIXED) as u16
}

/// On-air size on a stream link of a padded LAB frame holding `records` bytes.
pub const fn on_air_of_records(records: usize) -> usize {
    records + STREAM_FRAME_OVERHEAD
}

/// One generated probe.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Probe {
    pub lprobe: Lprobe,
}

/// Per-link LPROBE generator for one run.
#[derive(Debug, Clone)]
pub struct TrafficGen {
    pattern: Pattern,
    streams: u8,
    interval_us: u64,
    ping_period_us: u64,
    voice_pad: u16,
    ping_pad: u16,
    saturate_pad: u16,
    next_voice: [u64; 4],
    next_ping: u64,
    next_id: u32,
    end: Option<u64>,
    skipped: u64,
    sent: u64,
}

/// Saturate probes fill a 1024 B frame.
pub const SATURATE_ON_AIR: u16 = 1026;
/// Pings in a ping-under-load pattern use the ping size of §12.2 (64 B).
pub const PING_ON_AIR: u16 = 64;

impl TrafficGen {
    /// A generator starting at `start`.
    pub fn new(p: &RunParams, start: Instant) -> Self {
        let interval_us = u64::from(p.interval_ms) * US_PER_MS;
        let streams = p.streams.min(4);
        let mut next_voice = [u64::MAX; 4];
        for (i, slot) in next_voice.iter_mut().enumerate().take(usize::from(streams)) {
            // Spread the streams over the interval: two origins' frames are not
            // phase-aligned (§4.8 uses the uncoalesced rate).
            *slot = start.0 + interval_us * i as u64 / u64::from(streams.max(1));
        }
        let ping_period_us = if p.ping_hz == 0 {
            u64::MAX
        } else {
            US_PER_S / u64::from(p.ping_hz)
        };
        let ping_size = match p.pattern {
            Pattern::Ping => p.payload_b,
            _ => PING_ON_AIR,
        };
        TrafficGen {
            pattern: p.pattern,
            streams,
            interval_us: interval_us.max(1),
            ping_period_us,
            voice_pad: pad_for_on_air(p.payload_b),
            ping_pad: pad_for_on_air(ping_size),
            saturate_pad: pad_for_on_air(SATURATE_ON_AIR),
            next_voice,
            next_ping: start.0,
            next_id: 0,
            end: p.duration_us().map(|d| start.0 + d),
            skipped: 0,
            sent: 0,
        }
    }

    fn voice_on(&self) -> bool {
        matches!(self.pattern, Pattern::VoiceLike | Pattern::PingUnderLoad)
    }

    fn ping_on(&self) -> bool {
        matches!(self.pattern, Pattern::Ping | Pattern::PingUnderLoad)
    }

    fn ended(&self, now: Instant) -> bool {
        self.end.is_some_and(|e| now.0 >= e)
    }

    fn take_id(&mut self) -> u32 {
        let id = self.next_id;
        self.next_id = self.next_id.wrapping_add(1);
        self.sent += 1;
        id
    }

    /// The next due probe, if any. `writable` is false while the link's transmit queue
    /// is full; voice-like slots that fall more than two intervals behind are skipped
    /// (a real voice queue drops them too) and counted in [`TrafficGen::skipped`].
    pub fn poll(&mut self, now: Instant, writable: bool) -> Option<Probe> {
        if self.ended(now) || !writable {
            return None;
        }
        if self.ping_on() && now.0 >= self.next_ping {
            self.next_ping = advance(self.next_ping, self.ping_period_us, now.0, &mut 0);
            let id = self.take_id();
            return Some(probe(id, now, flags::PING, self.ping_pad));
        }
        if self.voice_on() {
            for s in 0..usize::from(self.streams) {
                if now.0 >= self.next_voice[s] {
                    let mut skipped = 0;
                    self.next_voice[s] =
                        advance(self.next_voice[s], self.interval_us, now.0, &mut skipped);
                    self.skipped += skipped;
                    let id = self.take_id();
                    return Some(probe(id, now, flags::data(s as u8), self.voice_pad));
                }
            }
        }
        if self.pattern == Pattern::Saturate {
            let id = self.take_id();
            return Some(probe(id, now, flags::DATA, self.saturate_pad));
        }
        None
    }

    /// When the next probe is due (`None`: nothing scheduled; saturate is always due).
    pub fn next_deadline(&self, now: Instant) -> Option<Instant> {
        if self.ended(now) {
            return None;
        }
        if self.pattern == Pattern::Saturate {
            return Some(now);
        }
        let mut t = u64::MAX;
        if self.ping_on() {
            t = t.min(self.next_ping);
        }
        if self.voice_on() {
            for s in 0..usize::from(self.streams) {
                t = t.min(self.next_voice[s]);
            }
        }
        (t != u64::MAX).then_some(Instant(t))
    }

    /// Voice-like slots skipped because the link was blocked.
    pub fn skipped(&self) -> u64 {
        self.skipped
    }

    /// Probes generated so far.
    pub fn sent(&self) -> u64 {
        self.sent
    }
}

/// Next slot after `slot`; if more than two periods behind `now`, jumps to the slot
/// following `now` and counts the skipped slots.
fn advance(slot: u64, period: u64, now: u64, skipped: &mut u64) -> u64 {
    let next = slot.saturating_add(period);
    if now > next.saturating_add(period) {
        let behind = (now - next) / period;
        *skipped += behind;
        next + behind * period + period
    } else {
        next
    }
}

fn probe(id: u32, now: Instant, f: u8, pad: u16) -> Probe {
    Probe {
        lprobe: Lprobe {
            probe_id: id,
            t_tx: now.0,
            flags: f,
            pad_len: pad,
        },
    }
}

/// MESH_PROBE pattern: one probe every `period` (multi-hop latency over the Lab flood).
#[derive(Debug, Clone)]
pub struct MeshProbeGen {
    period_us: u64,
    next: u64,
    next_id: u32,
}

impl MeshProbeGen {
    pub fn new(period_ms: u32, start: Instant) -> Self {
        MeshProbeGen {
            period_us: u64::from(period_ms.max(1)) * US_PER_MS,
            next: start.0,
            next_id: 0,
        }
    }

    /// The next due MESH_PROBE (`t_origin` = now), for mesh-core to flood.
    pub fn poll(&mut self, now: Instant) -> Option<MeshProbe> {
        if now.0 < self.next {
            return None;
        }
        self.next = advance(self.next, self.period_us, now.0, &mut 0);
        let id = self.next_id;
        self.next_id = self.next_id.wrapping_add(1);
        Some(MeshProbe::new(id, now.0))
    }

    pub fn next_deadline(&self) -> Instant {
        Instant(self.next)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kinds::{Anchor, Audio, Bench};
    use crate::params::LabParams;
    use mesh_wire::frame::encode_lab_padded;
    use mesh_wire::record::{Record, RecordWriter};

    fn on_air_of(pad: u16) -> usize {
        let mut w = RecordWriter::new();
        w.push(&Record::Lprobe(Lprobe {
            probe_id: 1,
            t_tx: 2,
            flags: 0,
            pad_len: pad,
        }))
        .unwrap();
        encode_lab_padded(0xABCD, w.as_bytes()).unwrap().len() + 2
    }

    #[test]
    fn voice_like_probe_is_exactly_one_voice_frame() {
        assert_eq!(pad_for_on_air(118), 80);
        assert_eq!(on_air_of(pad_for_on_air(118)), 118);
        let voice = mesh_wire::budget::voice_sizes(80, mesh_wire::budget::Framing::Stream);
        assert_eq!(on_air_of(pad_for_on_air(voice.on_air as u16)), voice.on_air);
        for target in 40..=1026u16 {
            let got = on_air_of(pad_for_on_air(target));
            assert!(
                got == usize::from(target) || got + 1 == usize::from(target),
                "{target} -> {got}"
            );
        }
        assert_eq!(on_air_of(pad_for_on_air(SATURATE_ON_AIR)), 1026);
    }

    fn params(pattern: Pattern, streams: u8, ping_hz: u8) -> RunParams {
        let mut p = LabParams::new(Audio::Off, Anchor::Audio, false);
        p.pattern = Some(pattern);
        p.streams = Some(streams);
        p.ping_hz = Some(ping_hz);
        p.duration_s = Some(2);
        RunParams::resolve(Bench::T0, &p).unwrap()
    }

    fn run(g: &mut TrafficGen, from: u64, to: u64, step: u64) -> Vec<Probe> {
        let mut out = Vec::new();
        let mut t = from;
        while t < to {
            while let Some(p) = g.poll(Instant(t), true) {
                out.push(p);
            }
            t += step;
        }
        out
    }

    #[test]
    fn voice_like_cbr_and_pings() {
        let mut g = TrafficGen::new(&params(Pattern::PingUnderLoad, 2, 10), Instant(0));
        let ps = run(&mut g, 0, 1_000_000, 1_000);
        let data = ps
            .iter()
            .filter(|p| flags::class(p.lprobe.flags) == flags::DATA)
            .count();
        let pings = ps
            .iter()
            .filter(|p| flags::class(p.lprobe.flags) == flags::PING)
            .count();
        assert_eq!(data, 50, "2 streams x 25 frames/s");
        assert_eq!(pings, 10);
        // Probe ids are consecutive per link.
        for (i, p) in ps.iter().enumerate() {
            assert_eq!(p.lprobe.probe_id, i as u32);
        }
        // Stream 1 is offset by half an interval from stream 0.
        let s1 = ps
            .iter()
            .find(|p| flags::stream(p.lprobe.flags) == 1)
            .unwrap();
        assert_eq!(s1.lprobe.t_tx, 20_000);
        // Nothing after the run ends.
        assert!(g.poll(Instant(2_000_000), true).is_none());
        assert_eq!(g.next_deadline(Instant(2_000_000)), None);
    }

    #[test]
    fn blocked_link_skips_voice_slots() {
        let mut g = TrafficGen::new(&params(Pattern::VoiceLike, 1, 0), Instant(0));
        assert!(g.poll(Instant(0), true).is_some());
        assert!(g.poll(Instant(1_000), false).is_none());
        // 400 ms later: one frame now, 8 slots skipped.
        assert!(g.poll(Instant(400_000), true).is_some());
        assert!(g.skipped() >= 8);
        assert!(g.poll(Instant(400_000), true).is_none());
        assert_eq!(g.next_deadline(Instant(400_000)), Some(Instant(440_000)));
    }

    #[test]
    fn saturate_fills_whenever_writable() {
        let mut g = TrafficGen::new(&params(Pattern::Saturate, 0, 0), Instant(0));
        for _ in 0..100 {
            let p = g.poll(Instant(5), true).unwrap();
            assert_eq!(p.lprobe.pad_len, pad_for_on_air(SATURATE_ON_AIR));
        }
        assert!(g.poll(Instant(5), false).is_none());
        assert_eq!(g.next_deadline(Instant(5)), Some(Instant(5)));
    }

    #[test]
    fn mesh_probe_pattern() {
        let mut m = MeshProbeGen::new(100, Instant(0));
        let a = m.poll(Instant(0)).unwrap();
        assert_eq!((a.probe_id, a.t_origin, a.n), (0, 0, 0));
        assert!(m.poll(Instant(50_000)).is_none());
        assert_eq!(m.poll(Instant(100_000)).unwrap().probe_id, 1);
        assert_eq!(m.next_deadline(), Instant(200_000));
    }
}
