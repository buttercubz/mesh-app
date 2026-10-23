//! [`Stats`]: everything measured on one Lab link, per 1 s window and per run.

use crate::stats::{
    Histogram, STALL_US, SeqTracker, StallDetector, infer_conn_interval_ms, p50_95_99,
};

/// One closed 1 s window of a link.
#[derive(Debug, Clone, PartialEq)]
pub struct WindowStats {
    pub tx_kbps: f64,
    pub rx_kbps: f64,
    /// Probe loss in the window (0..1): the larger of the sequence-gap loss (gap
    /// losses capped at one window of nominal traffic, so the window in which a long
    /// gap closes is not charged for the whole gap) and the shortfall against the
    /// peer's nominal CBR rate (one probe of boundary jitter tolerated), so a silent
    /// link counts as lost while it is silent. 0 when nothing was expected.
    pub loss: f64,
    /// Probes expected in the window (received + lost, at least the nominal count).
    pub expected: u64,
    /// Probes received in the window.
    pub received: u64,
    pub reorder: u32,
    pub rtt_p50_ms: Option<f64>,
    pub rtt_p95_ms: Option<f64>,
    pub owd_p95_ms: Option<f64>,
    pub stalls: u32,
}

/// Run totals of a link.
#[derive(Debug, Clone, PartialEq)]
pub struct RunStats {
    pub duration_s: f64,
    pub tx_bytes: u64,
    pub rx_bytes: u64,
    pub tx_kbps_mean: f64,
    pub rx_kbps_mean: f64,
    pub loss: f64,
    pub received: u64,
    pub lost: u64,
    pub reorder: u64,
    pub duplicates: u64,
    pub stalls: u32,
    pub longest_stall_ms: f64,
    pub rtt_ms: Option<[f64; 3]>,
    pub owd_ms: Option<[f64; 3]>,
    pub conn_interval_ms: Option<f64>,
    /// Probes the peer reported receiving (LECHO `rx_count`), vs. probes we sent.
    pub peer_rx_count: Option<u32>,
    pub tx_probes: u64,
}

/// Per-link statistics accumulator (sans-IO; times in µs).
#[derive(Debug, Clone)]
pub struct Stats {
    start_us: u64,
    window_start_us: u64,
    tx_bytes_win: u64,
    rx_bytes_win: u64,
    tx_bytes: u64,
    rx_bytes: u64,
    seq: SeqTracker,
    win_base: (u64, u64, u64),
    stall: StallDetector,
    stalls_win: u32,
    rtt_win: Vec<f64>,
    owd_win: Vec<f64>,
    rtt: Histogram,
    owd: Histogram,
    peer_rx_count: Option<u32>,
    tx_probes: u64,
    nominal_per_s: f64,
}

impl Stats {
    /// Statistics without a nominal incoming rate (saturate, passive benches).
    pub fn new(start_us: u64) -> Self {
        Self::with_nominal_rate(start_us, 0.0)
    }

    /// Statistics of a link whose peer sends `nominal_per_s` probes per second (CBR
    /// voice-like streams + pings).
    pub fn with_nominal_rate(start_us: u64, nominal_per_s: f64) -> Self {
        Stats {
            nominal_per_s: nominal_per_s.max(0.0),
            start_us,
            window_start_us: start_us,
            tx_bytes_win: 0,
            rx_bytes_win: 0,
            tx_bytes: 0,
            rx_bytes: 0,
            seq: SeqTracker::new(),
            win_base: (0, 0, 0),
            stall: StallDetector::new(STALL_US),
            stalls_win: 0,
            rtt_win: Vec::new(),
            owd_win: Vec::new(),
            rtt: Histogram::new(),
            owd: Histogram::new(),
            peer_rx_count: None,
            tx_probes: 0,
        }
    }

    /// Bytes handed to the link (on-air estimate).
    pub fn on_tx(&mut self, on_air_bytes: usize, probes: u64) {
        self.tx_bytes_win += on_air_bytes as u64;
        self.tx_bytes += on_air_bytes as u64;
        self.tx_probes += probes;
    }

    /// Bytes received from the link (on-air estimate).
    pub fn on_rx(&mut self, on_air_bytes: usize) {
        self.rx_bytes_win += on_air_bytes as u64;
        self.rx_bytes += on_air_bytes as u64;
    }

    /// A data or ping probe from the peer arrived at `t_us`.
    pub fn on_probe(&mut self, probe_id: u32, t_us: u64) {
        self.seq.on_id(probe_id);
        if self.stall.on_arrival(t_us).is_some() {
            self.stalls_win += 1;
        }
    }

    /// A round trip (µs).
    pub fn on_rtt(&mut self, rtt_us: u64) {
        self.rtt.record(rtt_us);
        self.rtt_win.push(rtt_us as f64 / 1e3);
    }

    /// A one-way estimate (µs); negative estimates (clock error) count as 0 in the
    /// run histogram but are kept in the window.
    pub fn on_owd(&mut self, owd_us: i64) {
        self.owd.record(owd_us.max(0) as u64);
        self.owd_win.push(owd_us as f64 / 1e3);
    }

    /// The peer's LECHO `rx_count`.
    pub fn on_peer_rx_count(&mut self, n: u32) {
        self.peer_rx_count = Some(n);
    }

    /// The link went down: the reconnect gap is not a stall.
    pub fn on_link_down(&mut self) {
        self.stall.reset();
    }

    /// Closes the window ending at `now_us` (normally 1 s long).
    pub fn close_window(&mut self, now_us: u64) -> WindowStats {
        let len_us = now_us.saturating_sub(self.window_start_us).max(1);
        let kbps = |bytes: u64| bytes as f64 * 8.0 / (len_us as f64 / 1e6) / 1e3;
        let (r0, l0, o0) = self.win_base;
        let received = self.seq.received - r0;
        let lost = self.seq.lost.saturating_sub(l0);
        let nominal = self.nominal_per_s * len_us as f64 / 1e6;
        let (loss, expected) = if nominal > 0.0 {
            let capped = (lost as f64).min(nominal.ceil());
            let seq_loss = if capped > 0.0 {
                capped / (received as f64 + capped)
            } else {
                0.0
            };
            let short = if received == 0 {
                1.0
            } else {
                ((nominal - received as f64 - 1.0) / nominal).max(0.0)
            };
            (
                seq_loss.max(short).min(1.0),
                (received + lost).max(nominal.round() as u64),
            )
        } else {
            let e = received + lost;
            (if e == 0 { 0.0 } else { lost as f64 / e as f64 }, e)
        };
        let rtt = p50_95_99(&self.rtt_win);
        let owd = p50_95_99(&self.owd_win);
        let w = WindowStats {
            tx_kbps: kbps(self.tx_bytes_win),
            rx_kbps: kbps(self.rx_bytes_win),
            loss,
            expected,
            received,
            reorder: (self.seq.reordered - o0) as u32,
            rtt_p50_ms: rtt.map(|p| p[0]),
            rtt_p95_ms: rtt.map(|p| p[1]),
            owd_p95_ms: owd.map(|p| p[1]),
            stalls: self.stalls_win,
        };
        self.window_start_us = now_us;
        self.tx_bytes_win = 0;
        self.rx_bytes_win = 0;
        self.stalls_win = 0;
        self.rtt_win.clear();
        self.owd_win.clear();
        self.win_base = (self.seq.received, self.seq.lost, self.seq.reordered);
        w
    }

    /// Run totals up to `now_us`.
    pub fn run_stats(&self, now_us: u64) -> RunStats {
        let dur_us = now_us.saturating_sub(self.start_us).max(1);
        let secs = dur_us as f64 / 1e6;
        RunStats {
            duration_s: secs,
            tx_bytes: self.tx_bytes,
            rx_bytes: self.rx_bytes,
            tx_kbps_mean: self.tx_bytes as f64 * 8.0 / secs / 1e3,
            rx_kbps_mean: self.rx_bytes as f64 * 8.0 / secs / 1e3,
            loss: self.seq.loss(),
            received: self.seq.received,
            lost: self.seq.lost,
            reorder: self.seq.reordered,
            duplicates: self.seq.duplicates,
            stalls: self.stall.stalls,
            longest_stall_ms: self.stall.longest_us as f64 / 1e3,
            rtt_ms: self.rtt.p50_95_99_ms(),
            owd_ms: self.owd.p50_95_99_ms(),
            conn_interval_ms: infer_conn_interval_ms(&self.rtt),
            peer_rx_count: self.peer_rx_count,
            tx_probes: self.tx_probes,
        }
    }

    /// The run RTT histogram (connection-interval inference across links).
    pub fn rtt_histogram(&self) -> &Histogram {
        &self.rtt
    }

    /// The run one-way histogram.
    pub fn owd_histogram(&self) -> &Histogram {
        &self.owd
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn windows_and_totals() {
        let mut s = Stats::new(0);
        // 1 s: 25 probes of 118 B each way, ids 0..25 with 2 lost (5, 6).
        for id in 0..25u32 {
            s.on_tx(118, 1);
            if id != 5 && id != 6 {
                s.on_rx(118);
                s.on_probe(id, u64::from(id) * 40_000);
            }
        }
        for rtt in [30_000u64, 40_000, 50_000] {
            s.on_rtt(rtt);
        }
        s.on_owd(20_000);
        let w = s.close_window(1_000_000);
        assert!((w.tx_kbps - 23.6).abs() < 1e-9);
        assert!((w.rx_kbps - 23.0 * 118.0 * 8.0 / 1e3).abs() < 1e-9);
        assert_eq!(w.expected, 25);
        assert!((w.loss - 2.0 / 25.0).abs() < 1e-12);
        assert_eq!(w.rtt_p50_ms, Some(40.0));
        assert_eq!(w.owd_p95_ms, Some(20.0));
        // Losing ids 5 and 6 leaves a 120 ms gap between arrivals: one stall.
        assert_eq!(w.stalls, 1);
        // Second window: a 300 ms gap is one stall; nothing lost.
        s.on_probe(25, 1_300_000);
        let w2 = s.close_window(2_000_000);
        assert_eq!((w2.stalls, w2.expected, w2.loss), (1, 1, 0.0));
        assert_eq!(w2.rtt_p50_ms, None);
        let r = s.run_stats(2_000_000);
        assert_eq!((r.received, r.lost, r.stalls, r.tx_probes), (24, 2, 2, 25));
        assert!((r.tx_kbps_mean - 11.8).abs() < 1e-9);
        assert!(r.rtt_ms.is_some() && r.owd_ms.is_some());
    }

    #[test]
    fn nominal_rate_shortfall() {
        // 25 probes/s nominal: a silent second is fully lost, a long gap closing in the
        // next second is charged at most one window.
        let mut s = Stats::with_nominal_rate(0, 25.0);
        for id in 0..25u32 {
            s.on_probe(id, u64::from(id) * 40_000);
        }
        assert_eq!(s.close_window(1_000_000).loss, 0.0);
        let silent = s.close_window(2_000_000);
        assert_eq!(
            (silent.loss, silent.expected, silent.received),
            (1.0, 25, 0)
        );
        for id in 75..100u32 {
            s.on_probe(id, 2_000_000 + u64::from(id - 75) * 40_000);
        }
        let resumed = s.close_window(3_000_000);
        assert!((resumed.loss - 0.5).abs() < 1e-9, "{}", resumed.loss);
        for id in 100..124u32 {
            s.on_probe(id, 3_000_000 + u64::from(id - 100) * 40_000);
        }
        assert_eq!(
            s.close_window(4_000_000).loss,
            0.0,
            "one probe of jitter tolerated"
        );
    }

    #[test]
    fn link_down_resets_stall_tracking() {
        let mut s = Stats::new(0);
        s.on_probe(0, 0);
        s.on_link_down();
        s.on_probe(1, 5_000_000);
        assert_eq!(s.close_window(6_000_000).stalls, 0);
    }
}
