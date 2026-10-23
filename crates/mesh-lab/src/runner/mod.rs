//! [`BenchRunner`]: one Lab run on one device, sans-IO and [`Instant`]-driven.
//!
//! mesh-core owns the Lab links (HELLO `flags.lab`, LAB frames with `lab_tag`, PAD to
//! the SEC size) and drives the runner:
//!
//! - link lifecycle: [`BenchRunner::on_link_up`], [`BenchRunner::on_link_down`],
//!   [`BenchRunner::on_link_info`];
//! - traffic: [`BenchRunner::poll_tx`] (records for one LAB frame) and
//!   [`BenchRunner::on_rx`] (records of a received LAB frame);
//! - multi-hop: [`BenchRunner::poll_mesh_probe`] / [`BenchRunner::on_mesh_probe`];
//! - platform: [`BenchRunner::on_platform`] (1 Hz), [`BenchRunner::on_event`];
//! - audio benches: [`BenchRunner::poll_click`] / [`BenchRunner::on_capture`];
//! - time: [`BenchRunner::tick`] and [`BenchRunner::next_deadline`];
//! - output: [`BenchRunner::drain`] yields the `labrun/1` lines in file order.

mod audio;
mod rx;
mod sample;

use std::collections::BTreeMap;

use crate::clock::ClockEstimator;
use crate::correlator::{LoopbackCorrelator, LoopbackSeries};
use crate::json::Json;
use crate::kinds::{Bench, BtState, LinkKind, LinkRole, RunRole};
use crate::linkstats::Stats;
use crate::params::RunParams;
use crate::record::{
    AnchorSample, AudioSample, DevSample, DeviceInfo, GpsFix, LabRecord, PeerInfo, RawLoopback,
    RawMeshProbe, RawPing, RunLine, SummaryLine,
};
use crate::time::{Instant, US_PER_S};
use crate::traffic::{MeshProbeGen, TrafficGen};

/// Everything the `run` line needs.
#[derive(Debug, Clone, PartialEq)]
pub struct RunConfig {
    pub run_id: String,
    pub params: RunParams,
    /// Variant values (from the catalog variant or the UI).
    pub variant: Vec<(String, Json)>,
    pub started_at: String,
    pub device: DeviceInfo,
    pub lab_node: u16,
    pub role: RunRole,
    pub peers: Vec<PeerInfo>,
}

/// Platform state reported by the host at 1 Hz (§12.1 "Platform samples").
#[derive(Debug, Clone, PartialEq)]
pub struct PlatformSample {
    /// GPS fix (only with the run's GPS consent).
    pub gps: Option<GpsFix>,
    pub heading_deg: Option<f64>,
    pub audio: AudioSample,
    pub dev: DevSample,
    pub anchor: AnchorSample,
    pub bt: BtState,
}

impl PlatformSample {
    /// What the runner records when the host has not reported this second.
    pub fn unknown() -> Self {
        PlatformSample {
            gps: None,
            heading_deg: None,
            audio: AudioSample {
                cap_fps: None,
                ren_fps: None,
                running: false,
                route: "unknown".into(),
                ladder: "L0".into(),
                xruns: 0,
            },
            dev: DevSample {
                bat: None,
                chg: None,
                therm: "unknown".into(),
                app: "foreground".into(),
                locked: None,
            },
            anchor: AnchorSample {
                ok: None,
                session: None,
                location: None,
                live_activity: None,
            },
            bt: BtState::Unknown,
        }
    }
}

/// Records for one LAB frame on `link`. mesh-core prepends the LAB header, appends the
/// PAD record (§4.3) and frames it for the transport.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LabTx {
    pub link: u32,
    /// Encoded records (plaintext of the LAB frame, before PAD).
    pub records: Vec<u8>,
    /// Estimated on-air size (records + SEC-size padding + transport framing).
    pub on_air: usize,
}

/// Live numbers for `labProgress` (mesh-api `LabLiveSample`).
#[derive(Debug, Clone, PartialEq)]
pub struct LiveSample {
    pub links: u8,
    pub peers: u8,
    pub tx_kbps: f64,
    pub rx_kbps: f64,
    pub loss: f64,
    pub rtt_p50_ms: Option<f64>,
    pub rtt_p95_ms: Option<f64>,
    pub rssi: Option<i16>,
    pub dist_m: Option<f64>,
    pub stalls: u32,
    pub anchor_ok: bool,
}

/// A ping answer waiting for the next frame on its link.
#[derive(Debug, Clone, Copy)]
struct PendingReply {
    probe_id: u32,
    t_tx: u64,
    rx_at: u64,
}

#[derive(Debug, Clone)]
struct LinkState {
    kind: LinkKind,
    role: Option<LinkRole>,
    peer: Option<u16>,
    up: bool,
    down_at: Option<u64>,
    traffic: TrafficGen,
    stats: Stats,
    clock: ClockEstimator,
    replies: Vec<PendingReply>,
    rx_count: u32,
    rssi: Option<i16>,
    max_write: Option<u16>,
    gps_due_us: u64,
    raw_pings: Vec<RawPing>,
    last_window: Option<crate::linkstats::WindowStats>,
}

/// One Lab run on one device.
#[derive(Debug, Clone)]
pub struct BenchRunner {
    cfg: RunConfig,
    start: Instant,
    end_us: Option<u64>,
    links: BTreeMap<u32, LinkState>,
    out: Vec<LabRecord>,
    next_sample_us: u64,
    platform: Option<PlatformSample>,
    platform_fresh: bool,
    peer_gps: BTreeMap<u16, GpsFix>,
    mesh_gen: Option<MeshProbeGen>,
    hop_ms: Vec<f64>,
    raw_mesh: Vec<RawMeshProbe>,
    raw_loopback: Vec<RawLoopback>,
    loopback: Option<LoopbackCorrelator>,
    series: LoopbackSeries,
    next_click_us: u64,
    clicks_done: u32,
    facts: Facts,
    summary: Option<SummaryLine>,
}

/// Counters that feed the verdict.
#[derive(Debug, Clone, Default)]
struct Facts {
    valid_windows: u32,
    good_windows: u32,
    anchor_drops: u32,
    anchor_was_ok: Option<bool>,
    capture_stops: u32,
    capture_lost: u32,
    rejoins_s: Vec<f64>,
    xruns_first: Option<u32>,
    xruns_last: u32,
    bat_first: Option<(u64, f64)>,
    bat_last: Option<(u64, f64)>,
    charging_seen: bool,
    stalls_total: u32,
}

impl BenchRunner {
    /// Starts a run at `now` and queues its `run` line.
    pub fn new(cfg: RunConfig, now: Instant) -> Self {
        let p = &cfg.params;
        let mesh_gen = (p.bench == Bench::T0b && cfg.role == RunRole::Controller)
            .then(|| MeshProbeGen::new(MESH_PROBE_PERIOD_MS, now));
        let mut r = BenchRunner {
            end_us: p.duration_us().map(|d| now.0 + d),
            start: now,
            links: BTreeMap::new(),
            out: Vec::new(),
            next_sample_us: now.0 + US_PER_S,
            platform: None,
            platform_fresh: false,
            peer_gps: BTreeMap::new(),
            mesh_gen,
            hop_ms: Vec::new(),
            raw_mesh: Vec::new(),
            raw_loopback: Vec::new(),
            loopback: None,
            series: LoopbackSeries::new(),
            next_click_us: now.0 + CLICK_LEAD_US,
            clicks_done: 0,
            facts: Facts::default(),
            summary: None,
            cfg,
        };
        let line = r.run_line();
        r.out.push(LabRecord::Run(Box::new(line)));
        r
    }

    /// The `run` line of this run.
    pub fn run_line(&self) -> RunLine {
        RunLine {
            run_id: self.cfg.run_id.clone(),
            variant: self.cfg.variant.clone(),
            started_at: self.cfg.started_at.clone(),
            device: self.cfg.device.clone(),
            lab_node: self.cfg.lab_node,
            role: self.cfg.role,
            peers: self.cfg.peers.clone(),
            params: self.cfg.params.clone(),
        }
    }

    pub fn params(&self) -> &RunParams {
        &self.cfg.params
    }

    /// Seconds since the run started.
    fn ts(&self, now: Instant) -> f64 {
        now.since(self.start) as f64 / 1e6
    }

    /// Takes the lines produced so far, in file order.
    pub fn drain(&mut self) -> Vec<LabRecord> {
        core::mem::take(&mut self.out)
    }

    /// The run has ended (stopped or timed out) and its summary line is queued.
    pub fn is_finished(&self) -> bool {
        self.summary.is_some()
    }

    pub fn summary(&self) -> Option<&SummaryLine> {
        self.summary.as_ref()
    }

    /// Records a mark (button for passengers or at stops).
    pub fn mark(&mut self, now: Instant, label: &str) {
        if self.is_finished() {
            return;
        }
        let ts = self.ts(now);
        self.out.push(LabRecord::Mark {
            ts,
            label: label.chars().take(64).collect(),
        });
    }

    /// Records a host event (`interruption`, `routeChange`, `captureLost`, …). The
    /// `captureStopped` and `captureLost` names feed the T2 silent-mute check.
    pub fn on_event(&mut self, now: Instant, ev: &str, fields: Vec<(String, Json)>) {
        if self.is_finished() {
            return;
        }
        match ev {
            "captureStopped" => self.facts.capture_stops += 1,
            "captureLost" => self.facts.capture_lost += 1,
            _ => {}
        }
        self.event(now, ev, fields);
    }

    fn event(&mut self, now: Instant, ev: &str, fields: Vec<(String, Json)>) {
        let ts = self.ts(now);
        self.out.push(LabRecord::Event {
            ts,
            ev: ev.to_string(),
            fields,
        });
    }

    /// When the runner next needs [`BenchRunner::tick`] or a poll.
    pub fn next_deadline(&self, now: Instant) -> Option<Instant> {
        if self.is_finished() {
            return None;
        }
        let mut t = self.next_sample_us;
        if let Some(e) = self.end_us {
            t = t.min(e);
        }
        for l in self.links.values().filter(|l| l.up) {
            if let Some(d) = l.traffic.next_deadline(now) {
                t = t.min(d.0);
            }
            if !l.replies.is_empty() {
                t = t.min(now.0);
            }
        }
        if let Some(m) = &self.mesh_gen {
            t = t.min(m.next_deadline().0);
        }
        if self.click_bench() && self.loopback.is_none() {
            t = t.min(self.next_click_us);
        }
        Some(Instant(t.max(now.0)))
    }

    /// Advances time: writes the 1 Hz sample (and raw) lines for every elapsed second
    /// and finishes the run when its duration is over.
    pub fn tick(&mut self, now: Instant) {
        if self.is_finished() {
            return;
        }
        while now.0 >= self.next_sample_us {
            let at = self.next_sample_us;
            if self.end_us.is_some_and(|e| at > e) {
                break;
            }
            self.write_second(Instant(at));
            self.next_sample_us += US_PER_S;
        }
        if self.end_us.is_some_and(|e| now.0 >= e) {
            let end = Instant(self.end_us.unwrap_or(now.0));
            self.finish(end);
        }
    }

    /// Stops the run now (user Stop or `LAB_CTL{stop}`) and queues the summary.
    pub fn stop(&mut self, now: Instant) -> Option<&SummaryLine> {
        if !self.is_finished() {
            self.tick(now);
        }
        if !self.is_finished() {
            self.finish(now);
        }
        self.summary.as_ref()
    }
}

/// MESH_PROBE period of the T0b origin (5 Hz).
pub const MESH_PROBE_PERIOD_MS: u32 = 200;
/// The first click train starts 1 s into a T5a/T5b run (route settles).
pub const CLICK_LEAD_US: u64 = 1_000_000;
/// LAB_GPS period on every Lab link (1 Hz).
pub const GPS_PERIOD_US: u64 = 1_000_000;
