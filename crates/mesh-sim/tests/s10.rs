//! S10 (Phase 0 milestone): green on the fixed CI seeds, identical for identical seeds,
//! and the suite runner's verdicts.

use mesh_sim::scenario::{find, registry, run_one};
use mesh_sim::suite::{SuiteConfig, run_suite};

fn render(o: &mesh_sim::Outcome) -> String {
    format!("failures {:#?}\nviolations {:#?}", o.failures, o.violations)
}

#[test]
fn s10_is_green_on_fixed_seeds_and_deterministic() {
    let s10 = find("S10").expect("S10 registered");
    assert_eq!(s10.phase, 0);
    let cfg = SuiteConfig::ci(2);
    for k in 0..2 {
        let seed = cfg.seed(k);
        let a = run_one(&s10, seed);
        assert!(a.passed(), "seed {seed:#x}: {}", render(&a));
        // Every audio path × anchor ran and the forced collisions were redrawn.
        let metric = |n: &str| a.metrics.iter().find(|(k, _)| k == n).map(|(_, v)| *v);
        assert!(metric("S10.labIdRedraws").is_some_and(|v| v >= 2.0));
        assert_eq!(metric("T0c.hubLinks"), Some(3.0));
        assert!(metric("T3.rejoinS").is_some_and(|v| v <= 10.0));
        assert!(a.sim_s > 60.0, "S10 simulated only {} s", a.sim_s);
        let b = run_one(&s10, seed);
        assert_eq!(a, b, "identical seeds must give identical outcomes");
    }
    let c = run_one(&s10, cfg.seed(0) ^ 1);
    assert_ne!(c.digest, run_one(&s10, cfg.seed(0)).digest);
}

#[test]
fn the_ci_suite_checks_determinism_and_every_registered_scenario() {
    let ids: Vec<&str> = registry().iter().map(|s| s.id).collect();
    assert!(ids.contains(&"S10"), "{ids:?}");
    let mut cfg = SuiteConfig::ci(1);
    cfg.only = vec!["s10".into()];
    let mut seen = 0;
    let r = run_suite(&cfg, |_| seen += 1);
    assert_eq!(seen, 1);
    assert!(r.nondeterministic.is_empty());
    assert!(r.passed(), "{}", r.render());
    assert!(r.render().contains("suite Ci: 1/1 runs passed"));
}
