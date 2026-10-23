//! The scenario suite (DESIGN §13.2). Each scenario builds one or more worlds from a
//! seed, scripts them, and returns an [`Outcome`]: its numeric assertions, report-only
//! metrics, the global-assertion violations of every world it ran, and a digest that
//! is identical for identical seeds.
//!
//! Phase 0 ships S10 (Lab benches against simulated peers). Later milestones add S1,
//! S5, S8, S9 (Phase 1), S2-S4, S6, S11-S13 (Phase 2) and S7 (Phase 4) to
//! [`registry`] without changing the runner.
//!
//! S4 (convoy15-highway) simulates a 30-minute ride: it runs in the release suite
//! (`mesh-sim suite ci`), not under `cargo test`.

pub mod common;
pub mod lab;
pub mod s1;
pub mod s10;
pub mod s2;
pub mod s3;
pub mod s4;
pub mod s5;
pub mod s6;
pub mod s8;
pub mod s9;

use crate::audit::Violation;
use crate::world::World;

/// The result of one scenario run.
#[derive(Debug, Clone, PartialEq)]
pub struct Outcome {
    pub id: &'static str,
    pub seed: u64,
    /// Failed assertions (empty = pass).
    pub failures: Vec<String>,
    /// Asserted metrics `(name, value)`.
    pub metrics: Vec<(String, f64)>,
    /// Report-only metrics (e.g. 3-hop latency, §13.2).
    pub reported: Vec<(String, f64)>,
    /// Global-assertion violations of every world of the run.
    pub violations: Vec<Violation>,
    /// Order-sensitive digest of every world of the run.
    pub digest: u64,
    /// Simulated seconds over all worlds.
    pub sim_s: f64,
}

impl Outcome {
    pub fn new(id: &'static str, seed: u64) -> Self {
        Outcome {
            id,
            seed,
            failures: Vec::new(),
            metrics: Vec::new(),
            reported: Vec::new(),
            violations: Vec::new(),
            digest: 0,
            sim_s: 0.0,
        }
    }

    /// `true` when every assertion held and no global assertion failed.
    pub fn passed(&self) -> bool {
        self.failures.is_empty() && self.violations.is_empty()
    }

    /// Records a failure unless `ok`.
    pub fn check(&mut self, ok: bool, what: impl FnOnce() -> String) -> bool {
        if !ok {
            self.failures.push(what());
        }
        ok
    }

    /// Records an asserted metric and checks it with `ok`.
    pub fn metric(&mut self, name: &str, v: f64, ok: bool, bound: &str) -> bool {
        self.metrics.push((name.to_string(), v));
        self.check(ok, || format!("{name} = {v:.3} violates {bound}"))
    }

    /// Records a report-only metric.
    pub fn report(&mut self, name: &str, v: f64) {
        self.reported.push((name.to_string(), v));
    }

    /// Folds a finished world into the outcome: its violations, digest and time.
    pub fn absorb(&mut self, w: &World) {
        self.violations.extend(w.audit.violations.iter().cloned());
        self.violations.extend(w.stream_cap_violations());
        self.digest = crate::rng::mix(&[self.digest, w.digest()]);
        self.sim_s += w.now.0 as f64 / 1e6;
    }
}

/// A registered scenario.
#[derive(Debug, Clone, Copy)]
pub struct Scenario {
    pub id: &'static str,
    pub title: &'static str,
    /// Delivery phase of the scenario (§14).
    pub phase: u8,
    pub run: fn(u64) -> Outcome,
}

/// Every implemented scenario, in suite order.
pub fn registry() -> Vec<Scenario> {
    vec![
        Scenario {
            id: "S1",
            title: "pair-ble: 2 nodes, 10 m",
            phase: 1,
            run: s1::run,
        },
        Scenario {
            id: "S2",
            title: "chain5-ble: 5 nodes, 45 m spacing",
            phase: 2,
            run: s2::run,
        },
        Scenario {
            id: "S3",
            title: "cluster15: 15 nodes within 30 m, 2 talkers",
            phase: 2,
            run: s3::run,
        },
        Scenario {
            id: "S4",
            title: "convoy15-highway: 15 nodes, 90-110 km/h, log-normal gaps, splits and merges",
            phase: 2,
            run: s4::run,
        },
        Scenario {
            id: "S5",
            title: "floor-contention: 6 nodes, 3-4 near-simultaneous VOX starts, leader among them, one late link-up",
            phase: 1,
            run: s5::run,
        },
        Scenario {
            id: "S6",
            title: "mixed-roles: 4 iOS + 3 Android, iOS backgrounded",
            phase: 2,
            run: s6::run,
        },
        Scenario {
            id: "S8",
            title: "restart: a node's app restarts mid-ride (new sess)",
            phase: 1,
            run: s8::run,
        },
        Scenario {
            id: "S9",
            title: "adversarial neighbor: malformed records, replays, forged origin at hops 0, stranger msg1",
            phase: 1,
            run: s9::run,
        },
        Scenario {
            id: "S10",
            title: "lab-bench-sim: T0/T0b/T0c/T3 against simulated peers, each audio path and anchor",
            phase: 0,
            run: s10::run,
        },
    ]
}

/// The scenario with `id` (case-insensitive).
pub fn find(id: &str) -> Option<Scenario> {
    registry()
        .into_iter()
        .find(|s| s.id.eq_ignore_ascii_case(id))
}

/// Runs `s` with `seed`; a panic anywhere (engine included) is a failure, not an abort.
pub fn run_one(s: &Scenario, seed: u64) -> Outcome {
    let f = s.run;
    match std::panic::catch_unwind(move || f(seed)) {
        Ok(o) => o,
        Err(p) => {
            let msg = p
                .downcast_ref::<String>()
                .cloned()
                .or_else(|| p.downcast_ref::<&str>().map(|s| (*s).to_string()))
                .unwrap_or_else(|| "non-string panic".into());
            let mut o = Outcome::new(s.id, seed);
            o.failures.push(format!("panic: {msg}"));
            o
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn boom(_seed: u64) -> Outcome {
        panic!("engine exploded")
    }

    #[test]
    fn panics_become_failures() {
        let s = Scenario {
            id: "SX",
            title: "panics",
            phase: 0,
            run: boom,
        };
        let o = run_one(&s, 3);
        assert!(!o.passed());
        assert!(
            o.failures[0].contains("engine exploded"),
            "{:?}",
            o.failures
        );
        assert!(find("s10").is_some());
        assert!(find("S99").is_none());
        assert!(find("s4").is_some());
    }

    #[test]
    fn outcome_bookkeeping() {
        let mut o = Outcome::new("S0", 1);
        assert!(o.metric("loss", 0.01, 0.01 <= 0.03, "≤ 0.03"));
        assert!(!o.metric("p95", 300.0, false, "≤ 250 ms"));
        o.report("p95.3hop", 280.0);
        assert!(!o.passed());
        assert_eq!(o.metrics.len(), 2);
        assert!(o.failures[0].contains("p95"));
    }
}
