//! Suite runners (DESIGN §2.5, §13.2): `ci` runs every implemented scenario on fixed
//! seeds and re-runs the first seed to prove determinism; `nightly` runs random seeds
//! (supplied by the caller, the binary derives them from the clock) and passes at
//! ≥ 99 %.

use std::fmt::Write as _;

use crate::rng::mix;
use crate::scenario::{Outcome, Scenario, registry, run_one};

/// Which suite.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SuiteKind {
    /// Fixed seeds per PR; every run must pass.
    Ci,
    /// Random seeds; at least 99 % of runs must pass.
    Nightly,
}

impl SuiteKind {
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "ci" => Some(SuiteKind::Ci),
            "nightly" => Some(SuiteKind::Nightly),
            _ => None,
        }
    }
}

/// Nightly pass-rate threshold.
pub const NIGHTLY_PASS_RATE: f64 = 0.99;

/// Suite parameters.
#[derive(Debug, Clone)]
pub struct SuiteConfig {
    pub kind: SuiteKind,
    pub seeds: u32,
    /// Base of the seed sequence (fixed for CI, random for nightly).
    pub base_seed: u64,
    /// Only scenarios whose id is listed (all when empty).
    pub only: Vec<String>,
}

/// The fixed CI base seed.
pub const CI_BASE_SEED: u64 = 0x00C1_5EED;

impl SuiteConfig {
    pub fn ci(seeds: u32) -> Self {
        SuiteConfig {
            kind: SuiteKind::Ci,
            seeds,
            base_seed: CI_BASE_SEED,
            only: Vec::new(),
        }
    }

    pub fn nightly(seeds: u32, base_seed: u64) -> Self {
        SuiteConfig {
            kind: SuiteKind::Nightly,
            seeds,
            base_seed,
            only: Vec::new(),
        }
    }

    /// The `k`-th seed of the suite.
    pub fn seed(&self, k: u32) -> u64 {
        mix(&[self.base_seed, u64::from(k)])
    }

    fn selected(&self) -> Vec<Scenario> {
        registry()
            .into_iter()
            .filter(|s| {
                self.only.is_empty() || self.only.iter().any(|o| o.eq_ignore_ascii_case(s.id))
            })
            .collect()
    }
}

/// Everything a suite run produced.
#[derive(Debug, Clone)]
pub struct SuiteReport {
    pub kind: SuiteKind,
    pub outcomes: Vec<Outcome>,
    /// Determinism re-runs whose digest differed: `(scenario, seed)`.
    pub nondeterministic: Vec<(&'static str, u64)>,
}

impl SuiteReport {
    pub fn runs(&self) -> usize {
        self.outcomes.len()
    }

    pub fn passed_runs(&self) -> usize {
        self.outcomes.iter().filter(|o| o.passed()).count()
    }

    pub fn pass_rate(&self) -> f64 {
        if self.outcomes.is_empty() {
            return 0.0;
        }
        self.passed_runs() as f64 / self.outcomes.len() as f64
    }

    /// The suite verdict: CI needs every run green, nightly ≥ 99 %; both need
    /// determinism and at least one run.
    pub fn passed(&self) -> bool {
        if self.outcomes.is_empty() || !self.nondeterministic.is_empty() {
            return false;
        }
        match self.kind {
            SuiteKind::Ci => self.passed_runs() == self.runs(),
            SuiteKind::Nightly => self.pass_rate() >= NIGHTLY_PASS_RATE,
        }
    }

    /// Human-readable report (one line per run, failures indented).
    pub fn render(&self) -> String {
        let mut s = String::new();
        for o in &self.outcomes {
            let verdict = if o.passed() { "PASS" } else { "FAIL" };
            let _ = writeln!(
                s,
                "{verdict} {} seed={:#018x} sim={:.0}s digest={:#018x}",
                o.id, o.seed, o.sim_s, o.digest
            );
            for (k, v) in &o.metrics {
                let _ = writeln!(s, "    metric   {k} = {v:.3}");
            }
            for (k, v) in &o.reported {
                let _ = writeln!(s, "    reported {k} = {v:.3}");
            }
            for f in &o.failures {
                let _ = writeln!(s, "    FAILED   {f}");
            }
            for v in &o.violations {
                let _ = writeln!(s, "    GLOBAL   {:?} at {:?}: {}", v.kind, v.t, v.detail);
            }
        }
        for (id, seed) in &self.nondeterministic {
            let _ = writeln!(s, "NONDETERMINISTIC {id} seed={seed:#018x}");
        }
        let _ = writeln!(
            s,
            "suite {:?}: {}/{} runs passed ({:.1} %) => {}",
            self.kind,
            self.passed_runs(),
            self.runs(),
            self.pass_rate() * 100.0,
            if self.passed() { "PASS" } else { "FAIL" }
        );
        s
    }
}

/// One run of a suite: a scenario, a seed, and whether it is the CI determinism
/// re-run of an earlier job.
#[derive(Debug, Clone, Copy)]
pub struct Job {
    pub scenario: Scenario,
    pub seed: u64,
    pub rerun: bool,
}

impl SuiteConfig {
    /// Every run of the suite in order. CI adds a determinism re-run of each
    /// scenario's first seed right after it. Jobs are independent: an executor may run
    /// them in any order or in parallel (each world is a pure function of its seed).
    pub fn jobs(&self) -> Vec<Job> {
        let mut v = Vec::new();
        for s in self.selected() {
            for k in 0..self.seeds {
                let seed = self.seed(k);
                v.push(Job {
                    scenario: s,
                    seed,
                    rerun: false,
                });
                if k == 0 && self.kind == SuiteKind::Ci {
                    v.push(Job {
                        scenario: s,
                        seed,
                        rerun: true,
                    });
                }
            }
        }
        v
    }
}

/// Assembles the report from `results[k]`, the outcome of `jobs[k]`.
pub fn report_from(cfg: &SuiteConfig, jobs: &[Job], results: Vec<Outcome>) -> SuiteReport {
    let mut outcomes = Vec::new();
    let mut nondeterministic = Vec::new();
    let mut first: Vec<(&'static str, u64, u64, Vec<String>)> = Vec::new();
    for (j, o) in jobs.iter().zip(results) {
        if j.rerun {
            let same = first.iter().any(|(id, seed, digest, failures)| {
                *id == j.scenario.id
                    && *seed == j.seed
                    && *digest == o.digest
                    && *failures == o.failures
            });
            if !same {
                nondeterministic.push((j.scenario.id, j.seed));
            }
        } else {
            first.push((j.scenario.id, j.seed, o.digest, o.failures.clone()));
            outcomes.push(o);
        }
    }
    SuiteReport {
        kind: cfg.kind,
        outcomes,
        nondeterministic,
    }
}

/// Runs the suite sequentially; `progress` sees each outcome as it completes.
pub fn run_suite(cfg: &SuiteConfig, mut progress: impl FnMut(&Outcome)) -> SuiteReport {
    let jobs = cfg.jobs();
    let mut results = Vec::with_capacity(jobs.len());
    for j in &jobs {
        let o = run_one(&j.scenario, j.seed);
        if !j.rerun {
            progress(&o);
        }
        results.push(o);
    }
    report_from(cfg, &jobs, results)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn verdicts_follow_the_suite_kind() {
        let mut ok = Outcome::new("S10", 1);
        ok.digest = 1;
        let mut bad = Outcome::new("S10", 2);
        bad.failures.push("x".into());
        let mut outcomes = vec![ok.clone(); 99];
        outcomes.push(bad);
        let nightly = SuiteReport {
            kind: SuiteKind::Nightly,
            outcomes: outcomes.clone(),
            nondeterministic: Vec::new(),
        };
        assert!(nightly.passed(), "99 % passes nightly");
        let ci = SuiteReport {
            kind: SuiteKind::Ci,
            outcomes,
            nondeterministic: Vec::new(),
        };
        assert!(!ci.passed(), "CI needs every run");
        assert!(ci.render().contains("FAILED   x"));
        let nd = SuiteReport {
            kind: SuiteKind::Ci,
            outcomes: vec![ok],
            nondeterministic: vec![("S10", 1)],
        };
        assert!(!nd.passed());
        assert_eq!(SuiteKind::parse("nightly"), Some(SuiteKind::Nightly));
        assert_ne!(SuiteConfig::ci(2).seed(0), SuiteConfig::ci(2).seed(1));
        assert_eq!(SuiteConfig::ci(2).seed(1), SuiteConfig::ci(5).seed(1));
    }
}
