//! 1 Hz sample and raw lines, platform samples, live numbers and the summary.

use super::{BenchRunner, LiveSample, PlatformSample};
use crate::geo;
use crate::json::Json;
use crate::kinds::{Bench, Pattern};
use crate::record::{LabRecord, LinkSample, RawLine, SampleLine, SummaryLine};
use crate::stats::{Histogram, infer_conn_interval_ms, p50_95_99};
use crate::time::Instant;
use crate::verdict::{RunFacts, THRESHOLDS, verdict};

impl BenchRunner {
    /// The host's 1 Hz platform sample (§12.1). Anchor transitions are logged as
    /// `anchorLost` / `anchorRestored` events.
    pub fn on_platform(&mut self, now: Instant, s: PlatformSample) {
        if self.is_finished() {
            return;
        }
        let ok = s.anchor.ok;
        match (self.facts.anchor_was_ok, ok) {
            (Some(true), Some(false)) => self.event(now, "anchorLost", Vec::new()),
            (Some(false), Some(true)) => self.event(now, "anchorRestored", Vec::new()),
            _ => {}
        }
        if ok.is_some() {
            self.facts.anchor_was_ok = ok;
        }
        let rel = now.since(self.start);
        if let Some(b) = s.dev.bat {
            if s.dev.chg == Some(true) {
                self.facts.charging_seen = true;
            }
            if self.facts.bat_first.is_none() {
                self.facts.bat_first = Some((rel, b));
            }
            self.facts.bat_last = Some((rel, b));
        }
        if self.facts.xruns_first.is_none() {
            self.facts.xruns_first = Some(s.audio.xruns);
        }
        self.facts.xruns_last = s.audio.xruns;
        self.platform = Some(s);
        self.platform_fresh = true;
    }

    /// Writes the `s` line (and `raw` lines) of the second ending at `at`.
    pub(super) fn write_second(&mut self, at: Instant) {
        let ts = self.ts(at);
        let platform = if self.platform_fresh {
            self.platform
                .clone()
                .unwrap_or_else(PlatformSample::unknown)
        } else {
            // Nothing reported this second: the anchor is not confirmed.
            let mut p = self
                .platform
                .clone()
                .unwrap_or_else(PlatformSample::unknown);
            p.anchor.ok = None;
            p
        };
        self.platform_fresh = false;
        let anchor_ok = platform.anchor.ok == Some(true);
        if !anchor_ok {
            self.facts.anchor_drops += 1;
        }
        let traffic = !matches!(self.cfg.params.pattern, Pattern::Idle);
        let mut links = Vec::with_capacity(self.links.len());
        let mut raws = Vec::new();
        for (id, l) in self.links.iter_mut() {
            let w = l.stats.close_window(at.0);
            self.facts.stalls_total += w.stalls;
            if traffic && anchor_ok {
                // T1 windows: a down link is a lost window; a window with nothing
                // expected (peer idle) is not counted.
                if !l.up {
                    self.facts.valid_windows += 1;
                } else if w.expected > 0 {
                    self.facts.valid_windows += 1;
                    if w.loss < THRESHOLDS.max_loss {
                        self.facts.good_windows += 1;
                    }
                }
            }
            if l.up {
                links.push(LinkSample {
                    id: *id,
                    kind: l.kind,
                    role: l.role,
                    peer: l.peer,
                    rssi: l.rssi,
                    tx_kbps: w.tx_kbps,
                    rx_kbps: w.rx_kbps,
                    loss: w.loss,
                    reorder: w.reorder,
                    rtt_p50: w.rtt_p50_ms,
                    rtt_p95: w.rtt_p95_ms,
                    owd_p95: w.owd_p95_ms,
                    stalls: w.stalls,
                    max_write: l.max_write,
                });
            }
            if !l.raw_pings.is_empty() {
                raws.push(RawLine {
                    ts,
                    link: Some(*id),
                    pings: core::mem::take(&mut l.raw_pings),
                    mesh_probes: Vec::new(),
                    loopback: Vec::new(),
                });
            }
            l.last_window = Some(w);
        }
        let gps = if self.cfg.params.gps {
            platform.gps
        } else {
            None
        };
        let peer_gps: Vec<_> = self.peer_gps.iter().map(|(k, v)| (*k, *v)).collect();
        let dist_m = gps.and_then(|own| {
            peer_gps
                .iter()
                .map(|(_, p)| geo::distance_m(&own, p))
                .min_by(f64::total_cmp)
        });
        self.out.push(LabRecord::Sample(Box::new(SampleLine {
            ts,
            links,
            gps,
            peer_gps,
            dist_m,
            audio: platform.audio,
            dev: platform.dev,
            anchor: platform.anchor,
            bt: platform.bt,
        })));
        if !self.raw_mesh.is_empty() || !self.raw_loopback.is_empty() {
            raws.push(RawLine {
                ts,
                link: None,
                pings: Vec::new(),
                mesh_probes: core::mem::take(&mut self.raw_mesh),
                loopback: core::mem::take(&mut self.raw_loopback),
            });
        }
        self.out
            .extend(raws.into_iter().map(|r| LabRecord::Raw(Box::new(r))));
    }

    /// Live numbers of the last closed second (`labProgress`).
    pub fn live(&self) -> LiveSample {
        let up: Vec<_> = self.links.values().filter(|l| l.up).collect();
        let windows: Vec<_> = up.iter().filter_map(|l| l.last_window.as_ref()).collect();
        let expected: u64 = windows.iter().map(|w| w.expected).sum();
        let lost: f64 = windows.iter().map(|w| w.loss * w.expected as f64).sum();
        let own = self
            .platform
            .as_ref()
            .and_then(|p| p.gps)
            .filter(|_| self.cfg.params.gps);
        LiveSample {
            links: up.len().min(255) as u8,
            peers: up.iter().filter_map(|l| l.peer).count().min(255) as u8,
            tx_kbps: windows.iter().map(|w| w.tx_kbps).sum(),
            rx_kbps: windows.iter().map(|w| w.rx_kbps).sum(),
            loss: if expected == 0 {
                0.0
            } else {
                lost / expected as f64
            },
            rtt_p50_ms: windows
                .iter()
                .filter_map(|w| w.rtt_p50_ms)
                .max_by(f64::total_cmp),
            rtt_p95_ms: windows
                .iter()
                .filter_map(|w| w.rtt_p95_ms)
                .max_by(f64::total_cmp),
            rssi: up.iter().filter_map(|l| l.rssi).max(),
            dist_m: own.and_then(|o| {
                self.peer_gps
                    .values()
                    .map(|p| geo::distance_m(&o, p))
                    .min_by(f64::total_cmp)
            }),
            stalls: self.facts.stalls_total,
            anchor_ok: self
                .platform
                .as_ref()
                .is_none_or(|p| p.anchor.ok != Some(false)),
        }
    }

    /// Ends the run at `now`: closes the open second and queues the summary line.
    pub(super) fn finish(&mut self, now: Instant) {
        if self.is_finished() {
            return;
        }
        if now.0 > self.next_sample_us.saturating_sub(1_000_000) && now.0 < self.next_sample_us {
            // Partial last second.
            self.write_second(now);
        }
        let dur_us = now.since(self.start);
        let duration_s = dur_us as f64 / 1e6;
        let mut rtt = Histogram::new();
        let mut owd = Histogram::new();
        let (mut tx, mut rx, mut received, mut lost, mut reorder) = (0u64, 0u64, 0u64, 0u64, 0u64);
        let mut stalls = 0u32;
        for l in self.links.values() {
            let r = l.stats.run_stats(now.0);
            tx += r.tx_bytes;
            rx += r.rx_bytes;
            received += r.received;
            lost += r.lost;
            reorder += r.reorder;
            stalls += r.stalls;
            rtt.merge(l.stats.rtt_histogram());
            owd.merge(l.stats.owd_histogram());
        }
        let secs = duration_s.max(1e-6);
        let expected = received + lost;
        let loss = if expected == 0 {
            0.0
        } else {
            lost as f64 / expected as f64
        };
        let rtt_ms = rtt.p50_95_99_ms();
        let owd_ms = owd.p50_95_99_ms();
        let loopback_ms = self.series.round_trip_ms();
        let per_leg_ms = self.series.per_leg_ms();
        let battery_pct_h = match (self.facts.bat_first, self.facts.bat_last) {
            (Some((t0, b0)), Some((t1, b1))) if t1 > t0 && !self.facts.charging_seen => {
                Some((b0 - b1) * 100.0 / ((t1 - t0) as f64 / 3.6e9))
            }
            _ => None,
        };
        let facts = RunFacts {
            duration_s,
            loss,
            expected,
            tx_kbps: tx as f64 * 8.0 / secs / 1e3,
            rx_kbps: rx as f64 * 8.0 / secs / 1e3,
            owd_p95_ms: owd_ms.map(|p| p[1]),
            rtt_p95_ms: rtt_ms.map(|p| p[1]),
            valid_windows: self.facts.valid_windows,
            good_windows: self.facts.good_windows,
            anchor_drops: self.facts.anchor_drops,
            hop_p95_ms: p50_95_99(&self.hop_ms).map(|p| p[1]),
            xruns: self
                .facts
                .xruns_last
                .saturating_sub(self.facts.xruns_first.unwrap_or(0)),
            capture_stops: self.facts.capture_stops,
            capture_lost: self.facts.capture_lost,
            rejoins_s: self.facts.rejoins_s.clone(),
            per_leg_ms,
            battery_pct_h,
        };
        let v = verdict(&self.cfg.params, &facts);
        let summary = SummaryLine {
            duration_s,
            tx_kbps_mean: facts.tx_kbps,
            rx_kbps_mean: facts.rx_kbps,
            loss,
            owd_ms,
            rtt_ms,
            stalls,
            reorder,
            conn_interval_ms: infer_conn_interval_ms(&rtt),
            loopback_ms,
            per_leg_ms,
            anchor_drops: self.facts.anchor_drops,
            verdict: v,
        };
        if matches!(self.cfg.params.bench, Bench::T1) {
            let fields = vec![
                (
                    "validWindows".to_string(),
                    Json::from(self.facts.valid_windows),
                ),
                (
                    "goodWindows".to_string(),
                    Json::from(self.facts.good_windows),
                ),
            ];
            self.event(now, "windows", fields);
        }
        self.out.push(LabRecord::Summary(Box::new(summary.clone())));
        self.summary = Some(summary);
    }
}
