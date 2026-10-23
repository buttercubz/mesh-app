//! `mesh-sim` command line (DESIGN §2.5):
//!
//! ```text
//! mesh-sim suite ci [--seeds N] [--only S10,S1] [--jobs J]    fixed seeds, every run must pass
//! mesh-sim suite nightly [--seeds N] [--base-seed X] [--jobs J]  random seeds, ≥ 99 % pass
//! mesh-sim run S10 [--seed X]                            one scenario, one seed
//! mesh-sim list                                          implemented scenarios
//! ```
//!
//! Exit status 0 on pass, 1 on failure, 2 on usage errors. This binary is the crate's
//! IO edge (it reads arguments and, for nightly runs without `--base-seed`, derives
//! the base seed from the clock) and runs suite jobs on worker threads (`--jobs`,
//! default: available parallelism); the library never does any of that.

use std::process::ExitCode;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

use mesh_sim::Outcome;
use mesh_sim::scenario::{find, registry, run_one};
use mesh_sim::suite::{Job, SuiteConfig, SuiteKind, report_from};

fn usage() -> ExitCode {
    eprintln!(
        "usage: mesh-sim suite <ci|nightly> [--seeds N] [--base-seed X] [--only S10,..] [--jobs J]\n       \
         mesh-sim run <scenario> [--seed X]\n       mesh-sim list"
    );
    ExitCode::from(2)
}

fn parse_u64(s: &str) -> Option<u64> {
    match s.strip_prefix("0x") {
        Some(h) => u64::from_str_radix(h, 16).ok(),
        None => s.parse().ok(),
    }
}

/// Value of `--name` in `args`.
fn opt<'a>(args: &'a [String], name: &str) -> Option<&'a str> {
    args.iter()
        .position(|a| a == name)
        .and_then(|i| args.get(i + 1))
        .map(String::as_str)
}

/// A random base seed for nightly runs (the binary may read the clock; the library
/// never does).
fn clock_seed() -> u64 {
    let t = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0);
    mesh_sim::rng::splitmix64(t ^ u64::from(std::process::id()))
}

fn suite(args: &[String]) -> ExitCode {
    let Some(kind) = args.first().and_then(|k| SuiteKind::parse(k)) else {
        return usage();
    };
    let seeds = match opt(args, "--seeds").map(str::parse::<u32>) {
        None => 8,
        Some(Ok(n)) if n > 0 => n,
        _ => return usage(),
    };
    let mut cfg = match kind {
        SuiteKind::Ci => SuiteConfig::ci(seeds),
        SuiteKind::Nightly => {
            let base = match opt(args, "--base-seed") {
                Some(s) => match parse_u64(s) {
                    Some(b) => b,
                    None => return usage(),
                },
                None => clock_seed(),
            };
            SuiteConfig::nightly(seeds, base)
        }
    };
    if let Some(ids) = opt(args, "--only") {
        cfg.only = ids.split(',').map(str::to_string).collect();
        if cfg.only.iter().any(|id| find(id).is_none()) {
            eprintln!("unknown scenario in --only {ids}");
            return ExitCode::from(2);
        }
    }
    println!(
        "mesh-sim suite {kind:?}: {} scenario(s) x {seeds} seed(s), base seed {:#018x}",
        registry().len(),
        cfg.base_seed
    );
    let workers = match opt(args, "--jobs").map(str::parse::<usize>) {
        None => std::thread::available_parallelism().map_or(1, |n| n.get()),
        Some(Ok(n)) if n > 0 => n,
        _ => return usage(),
    };
    let jobs = cfg.jobs();
    let results = run_parallel(&jobs, workers, |o| {
        let v = if o.passed() { "pass" } else { "FAIL" };
        println!(
            "  {v} {} seed={:#018x} ({:.0} simulated s)",
            o.id, o.seed, o.sim_s
        );
    });
    let report = report_from(&cfg, &jobs, results);
    print!("{}", report.render());
    if report.passed() {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}

/// Runs `jobs` on `workers` threads (each world is a pure function of its seed, so the
/// results do not depend on the schedule) and returns the outcomes in job order.
fn run_parallel(jobs: &[Job], workers: usize, progress: impl Fn(&Outcome) + Sync) -> Vec<Outcome> {
    let next = AtomicUsize::new(0);
    let slots: Mutex<Vec<Option<Outcome>>> = Mutex::new(vec![None; jobs.len()]);
    std::thread::scope(|s| {
        for _ in 0..workers.min(jobs.len()).max(1) {
            s.spawn(|| {
                loop {
                    let k = next.fetch_add(1, Ordering::Relaxed);
                    let Some(j) = jobs.get(k) else { break };
                    let o = run_one(&j.scenario, j.seed);
                    if !j.rerun {
                        progress(&o);
                    }
                    // A poisoned lock only means another worker panicked outside
                    // `run_one`; its slot stays empty and is reported below.
                    if let Ok(mut v) = slots.lock() {
                        v[k] = Some(o);
                    }
                }
            });
        }
    });
    let slots = slots.into_inner().unwrap_or_else(|e| e.into_inner());
    slots
        .into_iter()
        .zip(jobs)
        .map(|(o, j)| {
            o.unwrap_or_else(|| {
                let mut f = Outcome::new(j.scenario.id, j.seed);
                f.failures.push("worker lost the run".into());
                f
            })
        })
        .collect()
}

fn run(args: &[String]) -> ExitCode {
    let Some(s) = args.first().and_then(|id| find(id)) else {
        return usage();
    };
    let seed = match opt(args, "--seed") {
        Some(v) => match parse_u64(v) {
            Some(x) => x,
            None => return usage(),
        },
        None => SuiteConfig::ci(1).seed(0),
    };
    let o = run_one(&s, seed);
    let report = mesh_sim::suite::SuiteReport {
        kind: SuiteKind::Ci,
        outcomes: vec![o],
        nondeterministic: Vec::new(),
    };
    print!("{}", report.render());
    if report.passed() {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("suite") => suite(&args[1..]),
        Some("run") => run(&args[1..]),
        Some("list") => {
            for s in registry() {
                println!("{}  phase {}  {}", s.id, s.phase, s.title);
            }
            ExitCode::SUCCESS
        }
        _ => usage(),
    }
}
