//! On-device go / no-go verdict of one run (DESIGN §12.2). The numbers mirror
//! `tools/lab-analyze/thresholds.json`, which stays the source of truth for the merged,
//! multi-device analysis; a test keeps the two equal.

use crate::kinds::{Bench, Pattern, Verdict};
use crate::params::RunParams;

/// Go / no-go thresholds (DESIGN §12.2, research §9).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Thresholds {
    /// T0/T0b/T0c/T1: probe loss below this.
    pub max_loss: f64,
    /// T0/T0c: p95 one-way below this (ms).
    pub owd_p95_ms: f64,
    /// T0: voice-like streams that must pass with the headset active.
    pub min_streams: u8,
    /// T0: BLE-fast allowed at or above this throughput (kbps).
    pub ble_fast_kbps: f64,
    /// T0b: added p95 latency per hop (ms).
    pub hop_p95_ms: f64,
    /// T1: share of valid 1 s windows below `max_loss`.
    pub window_ok_share: f64,
    /// T1: minimum valid windows for a verdict.
    pub min_windows: u32,
    /// T3: rejoin time (s).
    pub rejoin_s: f64,
    /// T5a: per-leg budget (ms).
    pub leg_ms: f64,
    /// T6: battery drain screen off (%/h).
    pub battery_pct_h: f64,
    /// T6: minimum run length for a battery verdict (s).
    pub battery_min_s: f64,
    /// T4: streams and loss for the Aware upgrade.
    pub aware_streams: u8,
    pub aware_max_loss: f64,
}

/// The compiled-in thresholds.
pub const THRESHOLDS: Thresholds = Thresholds {
    max_loss: 0.05,
    owd_p95_ms: 60.0,
    min_streams: 2,
    ble_fast_kbps: 150.0,
    hop_p95_ms: 40.0,
    window_ok_share: 0.95,
    min_windows: 60,
    rejoin_s: 10.0,
    leg_ms: 25.0,
    battery_pct_h: 5.0,
    battery_min_s: 1_800.0,
    aware_streams: 4,
    aware_max_loss: 0.02,
};

/// What a run measured, as needed by the verdict.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct RunFacts {
    pub duration_s: f64,
    pub loss: f64,
    /// Probes that should have arrived (0 = no data).
    pub expected: u64,
    pub tx_kbps: f64,
    pub rx_kbps: f64,
    pub owd_p95_ms: Option<f64>,
    pub rtt_p95_ms: Option<f64>,
    /// T1: 1 s windows with a confirmed anchor and expected traffic, and how many of
    /// them were below `max_loss`.
    pub valid_windows: u32,
    pub good_windows: u32,
    /// Seconds whose anchor was not confirmed.
    pub anchor_drops: u32,
    /// T0b: p95 per-hop latency from MESH_PROBE ages (ms).
    pub hop_p95_ms: Option<f64>,
    /// Audio render/capture glitches (`mesh_rt_stats` xruns) during the run.
    pub xruns: u32,
    /// T2: capture stops, and how many of them produced CaptureLost.
    pub capture_stops: u32,
    pub capture_lost: u32,
    /// T3: link-down → link-up durations (s).
    pub rejoins_s: Vec<f64>,
    /// T5a: per-leg estimate (ms).
    pub per_leg_ms: Option<f64>,
    /// T6: battery drain (%/h), when measurable.
    pub battery_pct_h: Option<f64>,
}

/// The verdict of one run of `p.bench`.
pub fn verdict(p: &RunParams, f: &RunFacts) -> Verdict {
    let t = &THRESHOLDS;
    match p.bench {
        Bench::T0 | Bench::T0c | Bench::T4 => link_verdict(p, f, t),
        Bench::T0b => {
            if f.expected == 0 {
                return Verdict::Inconclusive;
            }
            match f.hop_p95_ms {
                None => Verdict::Inconclusive,
                Some(h) => go(f.loss < t.max_loss && h <= t.hop_p95_ms),
            }
        }
        Bench::T1 => {
            if f.valid_windows < t.min_windows {
                return Verdict::Inconclusive;
            }
            go(f64::from(f.good_windows) / f64::from(f.valid_windows) >= t.window_ok_share)
        }
        Bench::T2 => {
            if f.duration_s < 60.0 {
                Verdict::Inconclusive
            } else {
                go(f.capture_lost >= f.capture_stops)
            }
        }
        Bench::T3 => {
            if f.rejoins_s.is_empty() {
                Verdict::Inconclusive
            } else {
                go(f.rejoins_s.iter().all(|s| *s <= t.rejoin_s))
            }
        }
        Bench::T5a => match f.per_leg_ms {
            None => Verdict::Inconclusive,
            Some(l) => go(l <= t.leg_ms),
        },
        Bench::T6 => match f.battery_pct_h {
            Some(b) if f.duration_s >= t.battery_min_s => go(b < t.battery_pct_h),
            _ => Verdict::Inconclusive,
        },
        // Checklists and offline analyses: no automatic verdict.
        Bench::T5b | Bench::T8 | Bench::VoxCal | Bench::Ride => Verdict::Inconclusive,
    }
}

fn link_verdict(p: &RunParams, f: &RunFacts, t: &Thresholds) -> Verdict {
    if f.expected == 0 {
        return Verdict::Inconclusive;
    }
    let (max_loss, streams_needed) = if p.bench == Bench::T4 {
        (t.aware_max_loss, t.aware_streams)
    } else {
        (t.max_loss, t.min_streams)
    };
    let owd = f.owd_p95_ms.or(f.rtt_p95_ms.map(|r| r / 2.0));
    match p.pattern {
        Pattern::Saturate => go(f.loss < max_loss && f.tx_kbps.max(f.rx_kbps) >= t.ble_fast_kbps),
        Pattern::VoiceLike | Pattern::PingUnderLoad => {
            let Some(owd) = owd else {
                return if f.loss >= max_loss {
                    Verdict::NoGo
                } else {
                    Verdict::Inconclusive
                };
            };
            let glitch_free = p.bench != Bench::T0c || f.xruns == 0;
            go(p.streams >= streams_needed
                && f.loss < max_loss
                && owd < t.owd_p95_ms
                && glitch_free)
        }
        Pattern::Ping => match owd {
            Some(owd) => go(f.loss < max_loss && owd < t.owd_p95_ms),
            None => Verdict::Inconclusive,
        },
        Pattern::Idle => Verdict::Inconclusive,
    }
}

fn go(ok: bool) -> Verdict {
    if ok { Verdict::Go } else { Verdict::NoGo }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kinds::{Anchor, Audio};
    use crate::params::LabParams;

    fn params(bench: Bench, pattern: Pattern, streams: u8) -> RunParams {
        let mut p = LabParams::new(Audio::Full, Anchor::Audio, false);
        p.pattern = Some(pattern);
        p.streams = Some(streams);
        p.ping_hz = Some(10);
        RunParams::resolve(bench, &p).unwrap()
    }

    fn facts(loss: f64, owd: Option<f64>) -> RunFacts {
        RunFacts {
            duration_s: 60.0,
            loss,
            expected: 3_000,
            owd_p95_ms: owd,
            ..RunFacts::default()
        }
    }

    #[test]
    fn t0_voice_like() {
        let p2 = params(Bench::T0, Pattern::VoiceLike, 2);
        assert_eq!(verdict(&p2, &facts(0.01, Some(31.2))), Verdict::Go);
        assert_eq!(verdict(&p2, &facts(0.06, Some(31.2))), Verdict::NoGo);
        assert_eq!(verdict(&p2, &facts(0.01, Some(75.0))), Verdict::NoGo);
        assert_eq!(verdict(&p2, &facts(0.01, None)), Verdict::Inconclusive);
        let p1 = params(Bench::T0, Pattern::VoiceLike, 1);
        assert_eq!(
            verdict(&p1, &facts(0.0, Some(10.0))),
            Verdict::NoGo,
            "one stream never proves S_MAX = 2"
        );
        let none = RunFacts {
            expected: 0,
            ..facts(0.0, Some(10.0))
        };
        assert_eq!(verdict(&p2, &none), Verdict::Inconclusive);
        let sat = params(Bench::T0, Pattern::Saturate, 0);
        let mut f = facts(0.0, None);
        f.tx_kbps = 180.0;
        assert_eq!(verdict(&sat, &f), Verdict::Go);
        f.tx_kbps = 120.0;
        assert_eq!(verdict(&sat, &f), Verdict::NoGo);
    }

    #[test]
    fn t1_windows_and_others() {
        let p = params(Bench::T1, Pattern::PingUnderLoad, 2);
        let mut f = RunFacts {
            valid_windows: 100,
            good_windows: 96,
            ..RunFacts::default()
        };
        assert_eq!(verdict(&p, &f), Verdict::Go);
        f.good_windows = 90;
        assert_eq!(verdict(&p, &f), Verdict::NoGo);
        f.valid_windows = 10;
        assert_eq!(verdict(&p, &f), Verdict::Inconclusive);

        let t3 = params(Bench::T3, Pattern::Ping, 0);
        let mut f3 = RunFacts {
            rejoins_s: vec![3.2, 8.0],
            ..RunFacts::default()
        };
        assert_eq!(verdict(&t3, &f3), Verdict::Go);
        f3.rejoins_s.push(12.0);
        assert_eq!(verdict(&t3, &f3), Verdict::NoGo);

        let t5 = params(Bench::T5a, Pattern::Idle, 0);
        assert_eq!(
            verdict(
                &t5,
                &RunFacts {
                    per_leg_ms: Some(22.0),
                    ..RunFacts::default()
                }
            ),
            Verdict::Go
        );
        assert_eq!(
            verdict(
                &t5,
                &RunFacts {
                    per_leg_ms: Some(41.0),
                    ..RunFacts::default()
                }
            ),
            Verdict::NoGo
        );

        let t2 = params(Bench::T2, Pattern::VoiceLike, 1);
        let f2 = RunFacts {
            duration_s: 3_600.0,
            capture_stops: 2,
            capture_lost: 1,
            ..RunFacts::default()
        };
        assert_eq!(verdict(&t2, &f2), Verdict::NoGo, "a silent mute");

        let t6 = params(Bench::T6, Pattern::VoiceLike, 2);
        let f6 = RunFacts {
            duration_s: 3_600.0,
            battery_pct_h: Some(4.2),
            ..RunFacts::default()
        };
        assert_eq!(verdict(&t6, &f6), Verdict::Go);
        let t8 = params(Bench::T8, Pattern::Idle, 0);
        assert_eq!(verdict(&t8, &f6), Verdict::Inconclusive);
    }
}
