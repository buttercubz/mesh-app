//! Deterministic two-device simulation: two [`BenchRunner`]s joined by one Lab link
//! with a fixed delay, deterministic jitter, optional loss or blackout and a clock offset
//! between the devices. Used by the tests, by `examples/lab_fixtures.rs` (the
//! lab-analyze fixtures) and available to mesh-sim / the browser demo.

use crate::json::Json;
use crate::record::{AnchorSample, AudioSample, DevSample, DeviceInfo, GpsFix, PeerInfo, Platform};
use crate::{
    BenchRunner, BtState, Instant, LabRecord, LinkKind, LinkRole, PlatformSample, RunConfig,
    RunParams, RunRole,
};

/// The simulated link id.
pub const LINK: u32 = 1;

/// A run configuration with a fixed run id and an iPhone device (simulation only).
pub fn config(p: &RunParams, role: RunRole, lab_node: u16) -> RunConfig {
    let p = p.clone();
    RunConfig {
        // One run id per bench: r-20261023-0912-aNNN (NNN = the LAB_CTL bench byte).
        run_id: format!("r-20261023-0912-a{:03}", p.bench.wire_id()),
        variant: vec![
            ("transport".into(), Json::from(p.transport.as_str())),
            ("role".into(), Json::from(p.role.as_str())),
            (
                "headset".into(),
                Json::from(if p.headset { "hfp-wb" } else { "off" }),
            ),
        ],
        params: p,
        started_at: "2026-10-23T09:12:03.120Z".into(),
        device: DeviceInfo {
            model: "iPhone15,2".into(),
            os: "iOS 27.0".into(),
            app: "0.1.0 (12)".into(),
            platform: Platform::Ios,
        },
        lab_node,
        role,
        peers: vec![PeerInfo {
            lab_node: if lab_node == 4711 { 918 } else { 4711 },
            model: Some("iPhone13,1".into()),
        }],
    }
}

/// A synthetic platform sample `t_s` seconds into the run (moving north slowly).
pub fn platform(t_s: f64, anchor_ok: bool) -> PlatformSample {
    platform_at(t_s, anchor_ok, 0.0)
}

/// [`platform`] for a device `north_m` metres north of the reference vehicle.
pub fn platform_at(t_s: f64, anchor_ok: bool, north_m: f64) -> PlatformSample {
    // 1e-5° of latitude ≈ 1.11 m.
    let lat = 18.4861 + t_s * 1e-5 + north_m / 111_195.0;
    PlatformSample {
        gps: Some(GpsFix {
            lat,
            lon: -69.9312,
            spd: Some(12.0),
            acc: Some(4.8),
        }),
        heading_deg: Some(0.0),
        audio: AudioSample {
            cap_fps: Some(100.0),
            ren_fps: Some(100.0),
            running: true,
            route: "hfpWideband".into(),
            ladder: "L0".into(),
            xruns: 0,
        },
        dev: DevSample {
            bat: Some((0.81 - t_s / 36_000.0).max(0.0)),
            chg: Some(false),
            therm: "nominal".into(),
            app: "background".into(),
            locked: Some(true),
        },
        anchor: AnchorSample {
            ok: Some(anchor_ok),
            session: Some(anchor_ok),
            location: Some(false),
            live_activity: Some(false),
        },
        bt: BtState::On,
    }
}

struct InFlight {
    at: u64,
    to_b: bool,
    on_air: usize,
    records: Vec<u8>,
}

/// The two devices and the link between them. Time is A's clock in µs; B's clock is
/// A's plus `offset_us`.
pub struct PairSim {
    pub a: BenchRunner,
    pub b: BenchRunner,
    pub now: u64,
    pub delay_us: u64,
    pub jitter_us: u64,
    pub offset_us: u64,
    /// Drop every n-th frame A→B (0 = no loss).
    pub drop_every: u64,
    /// Drop everything while true.
    pub blackout: bool,
    /// Anchor state reported by both devices.
    pub anchor_ok: bool,
    /// B's distance ahead of A: `separation_m + separation_m_per_s × t`.
    pub separation_m: f64,
    pub separation_m_per_s: f64,
    sent_ab: u64,
    seed: u64,
    flight: Vec<InFlight>,
    pub lines_a: Vec<LabRecord>,
    pub lines_b: Vec<LabRecord>,
}

impl PairSim {
    /// Starts both runs (A controller, B follower) and brings the link up.
    pub fn new(params: &RunParams, offset_us: u64) -> Self {
        let t0 = 1_000_000;
        let a = BenchRunner::new(config(params, RunRole::Controller, 4711), Instant(t0));
        let b = BenchRunner::new(
            config(params, RunRole::Follower, 918),
            Instant(t0 + offset_us),
        );
        let mut p = PairSim {
            a,
            b,
            now: t0,
            delay_us: 20_000,
            jitter_us: 0,
            offset_us,
            drop_every: 0,
            blackout: false,
            anchor_ok: true,
            separation_m: 0.0,
            separation_m_per_s: 0.0,
            sent_ab: 0,
            seed: 0x1234_5678,
            flight: Vec::new(),
            lines_a: Vec::new(),
            lines_b: Vec::new(),
        };
        p.link_up();
        p
    }

    /// Brings the link up on both devices.
    pub fn link_up(&mut self) {
        let (ta, tb) = (Instant(self.now), Instant(self.now + self.offset_us));
        self.a.on_link_up(
            ta,
            LINK,
            LinkKind::BleL2cap,
            Some(LinkRole::Central),
            Some(918),
        );
        self.b.on_link_up(
            tb,
            LINK,
            LinkKind::BleL2cap,
            Some(LinkRole::Peripheral),
            Some(4711),
        );
        self.a.on_link_info(LINK, Some(-61), None);
        self.b.on_link_info(LINK, Some(-63), None);
    }

    /// Takes the link down on both devices (frames in flight are lost).
    pub fn link_down(&mut self) {
        let (ta, tb) = (Instant(self.now), Instant(self.now + self.offset_us));
        self.a.on_link_down(ta, LINK, "timeout");
        self.b.on_link_down(tb, LINK, "timeout");
        self.flight.clear();
    }

    fn jitter(&mut self) -> u64 {
        if self.jitter_us == 0 {
            return 0;
        }
        self.seed ^= self.seed << 13;
        self.seed ^= self.seed >> 7;
        self.seed ^= self.seed << 17;
        self.seed % self.jitter_us
    }

    /// Runs for `secs` seconds in 1 ms steps.
    pub fn run(&mut self, secs: f64) {
        let end = self.now + (secs * 1e6) as u64;
        while self.now < end {
            self.step();
            self.now += 1_000;
        }
    }

    fn step(&mut self) {
        let (ta, tb) = (Instant(self.now), Instant(self.now + self.offset_us));
        if self.now % 1_000_000 == 500_000 {
            let t = (self.now - 1_000_000) as f64 / 1e6;
            self.a.on_platform(ta, platform(t, self.anchor_ok));
            let north = self.separation_m + self.separation_m_per_s * t;
            self.b
                .on_platform(tb, platform_at(t, self.anchor_ok, north));
        }
        // Deliver due frames.
        let mut due = Vec::new();
        self.flight.retain(|f| {
            if f.at <= self.now {
                due.push((f.to_b, f.on_air, f.records.clone()));
                false
            } else {
                true
            }
        });
        for (to_b, on_air, records) in due {
            if to_b {
                // Frames come from the peer runner, so they always parse.
                let _ = self.b.on_rx(tb, LINK, on_air, &records);
            } else {
                let _ = self.a.on_rx(ta, LINK, on_air, &records);
            }
        }
        // Transmit.
        while let Some(tx) = self.a.poll_tx(ta, LINK, true) {
            self.sent_ab += 1;
            let dropped = self.blackout
                || (self.drop_every > 0 && self.sent_ab.is_multiple_of(self.drop_every));
            if !dropped {
                let at = self.now + self.delay_us + self.jitter();
                self.flight.push(InFlight {
                    at,
                    to_b: true,
                    on_air: tx.on_air,
                    records: tx.records,
                });
            }
        }
        while let Some(tx) = self.b.poll_tx(tb, LINK, true) {
            if !self.blackout {
                let at = self.now + self.delay_us + self.jitter();
                self.flight.push(InFlight {
                    at,
                    to_b: false,
                    on_air: tx.on_air,
                    records: tx.records,
                });
            }
        }
        self.a.tick(ta);
        self.b.tick(tb);
        self.lines_a.extend(self.a.drain());
        self.lines_b.extend(self.b.drain());
    }

    /// Stops both runs and collects the remaining lines.
    pub fn stop(&mut self) {
        let (ta, tb) = (Instant(self.now), Instant(self.now + self.offset_us));
        self.a.stop(ta);
        self.b.stop(tb);
        self.lines_a.extend(self.a.drain());
        self.lines_b.extend(self.b.drain());
    }
}

/// The JSONL text of `lines`.
pub fn jsonl(lines: &[LabRecord]) -> String {
    lines.iter().map(LabRecord::to_jsonl).collect()
}
