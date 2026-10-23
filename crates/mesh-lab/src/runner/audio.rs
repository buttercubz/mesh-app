//! T5a / T5b click trains: the runner schedules one MLS click train per repetition and
//! correlates the capture (DESIGN §12.2).

use super::BenchRunner;
use crate::correlator::{ClickTrain, LoopbackCorrelator};
use crate::json::{Json, round};
use crate::kinds::Bench;
use crate::record::RawLoopback;
use crate::time::{Instant, US_PER_MS};

/// Core sample rate (mono, 16 kHz, DESIGN §3.6).
pub const SAMPLE_RATE: u32 = 16_000;
/// Search window for the loopback (covers HFP round trips with margin).
pub const MAX_LOOPBACK_MS: u32 = 500;

impl BenchRunner {
    /// T5a always plays click trains; T5b only in its `echoReturn` check (the other
    /// checks — ducking, buttons, helmet intercom — must not hear bench tones).
    pub(super) fn click_bench(&self) -> bool {
        match self.cfg.params.bench {
            Bench::T5a => true,
            Bench::T5b => self
                .cfg
                .variant
                .iter()
                .any(|(k, v)| k == "check" && *v == crate::json::Json::from("echoReturn")),
            _ => false,
        }
    }

    /// T5a/T5b: when the next repetition is due, returns the click train to render
    /// (into the mixer, ahead of everything else) and starts correlating against the
    /// capture stream. `capture_index` is the capture sample counter at the instant the
    /// first train sample is handed to render (same counter as [`BenchRunner::on_capture`]).
    pub fn poll_click(&mut self, now: Instant, capture_index: u64) -> Option<Vec<i16>> {
        if self.is_finished()
            || !self.click_bench()
            || self.loopback.is_some()
            || now.0 < self.next_click_us
            || self.clicks_done >= u32::from(self.reps())
        {
            return None;
        }
        let mut c =
            LoopbackCorrelator::new(ClickTrain::bench_default(), SAMPLE_RATE, MAX_LOOPBACK_MS);
        c.emit(capture_index);
        let samples = c.train().samples().to_vec();
        self.loopback = Some(c);
        Some(samples)
    }

    fn reps(&self) -> u16 {
        crate::catalog::spec(self.cfg.params.bench).repetitions
    }

    /// Feeds capture samples starting at capture sample index `index`. When a click
    /// train has been fully searched, its result is recorded (`raw.loopback`) and the
    /// next repetition is scheduled `interval_ms` later.
    pub fn on_capture(&mut self, now: Instant, index: u64, samples: &[i16]) {
        let Some(c) = self.loopback.as_mut() else {
            return;
        };
        c.push_capture(index, samples);
        if !c.ready() {
            return;
        }
        let r = c.result();
        self.loopback = None;
        self.series.add(r);
        self.clicks_done += 1;
        self.raw_loopback.push(RawLoopback {
            rep: self.clicks_done,
            delay_ms: r.map(|x| x.delay_ms),
            ncc: r.map(|x| x.ncc),
            erl_db: r.map(|x| x.erl_db),
        });
        let mut fields = vec![("rep".to_string(), Json::from(self.clicks_done))];
        match r {
            Some(x) => {
                fields.push(("delayMs".to_string(), Json::Num(round(x.delay_ms, 3))));
                if x.erl_db.is_finite() {
                    fields.push(("erlDb".to_string(), Json::Num(round(x.erl_db, 1))));
                }
                self.event(now, "loopback", fields);
            }
            None => self.event(now, "loopbackMiss", fields),
        }
        self.next_click_us = now.0 + u64::from(self.cfg.params.interval_ms) * US_PER_MS;
    }

    /// Click-train repetitions measured so far (detected + missed).
    pub fn clicks_done(&self) -> u32 {
        self.clicks_done
    }
}
