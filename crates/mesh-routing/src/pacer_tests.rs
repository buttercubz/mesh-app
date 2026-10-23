use super::*;

fn run(p: &mut Pacer, start: Instant, periods: u64, qd: u64, traffic: bool) -> Vec<u32> {
    run_b(p, start, periods, qd, traffic, false)
}

fn run_b(
    p: &mut Pacer,
    start: Instant,
    periods: u64,
    qd: u64,
    traffic: bool,
    backlog: bool,
) -> Vec<u32> {
    let mut out = Vec::new();
    for i in 1..=periods {
        let now = start.plus_us(PACER_PERIOD_US * i);
        if traffic {
            p.note_queued();
        }
        if backlog {
            p.note_backlogged();
        }
        if let Some(s) = p.tick(now, qd) {
            out.push(s.rate_bps);
        }
    }
    out
}

#[test]
fn initial_rates() {
    let t = Instant::ZERO;
    assert_eq!(Pacer::new(t, PacerPath::L2cap, 1024).rate_bps(), 60_000);
    assert_eq!(Pacer::new(t, PacerPath::Gatt, 1024).rate_bps(), 40_000);
    assert_eq!(Pacer::new(t, PacerPath::Aware, 1200).rate_bps(), 2_000_000);
    assert_eq!(Pacer::new(t, PacerPath::Tunnel, 1024).rate_bps(), 64_000);
}

#[test]
fn step_response_delay() {
    // Step: queueing delay jumps to 60 ms for 2 s, then drops to 0.
    let t = Instant::ZERO;
    let mut p = Pacer::new(t, PacerPath::L2cap, 1024);
    p.set_grant(t, 1_000_000);
    let down = run(&mut p, t, 4, 60_000, true);
    assert_eq!(down, vec![51_000, 43_350, 36_847, 31_319]);
    let up = run(&mut p, t.plus_us(4 * PACER_PERIOD_US), 4, 0, true);
    assert_eq!(up, vec![35_319, 39_319, 43_319, 47_319]);
    // Between the thresholds nothing changes.
    let hold = run(&mut p, t.plus_us(8 * PACER_PERIOD_US), 3, 20_000, true);
    assert_eq!(hold, vec![47_319; 3]);
    // No traffic: no increase.
    let idle = run(&mut p, t.plus_us(11 * PACER_PERIOD_US), 2, 0, false);
    assert_eq!(idle, vec![47_319; 2]);
}

#[test]
fn would_block_and_bounds() {
    let t = Instant::ZERO;
    let mut p = Pacer::new(t, PacerPath::L2cap, 1024);
    p.on_would_block();
    let s = p.tick(t.plus_us(PACER_PERIOD_US), 0).expect("tick");
    assert!(s.decreased);
    assert_eq!(s.rate_bps, 51_000);
    // Recovered by the additive increase within ~1.5 s when the queue is not building,
    // and no further without backlog.
    let r = run(&mut p, t.plus_us(PACER_PERIOD_US), 4, 0, true);
    assert_eq!(r, vec![55_000, 59_000, 63_000, 63_000]);
    // Floor at 16 kbps.
    let low = run(&mut p, t.plus_us(5 * PACER_PERIOD_US), 40, 100_000, true);
    assert_eq!(*low.last().expect("last"), 16_000);
    // Aware multiplies and is capped at 4 Mbps.
    let mut a = Pacer::new(t, PacerPath::Aware, 1200);
    a.set_grant(t, u32::MAX);
    // At its initial rate only real backlog probes upward.
    assert_eq!(run(&mut a, t, 2, 0, true), vec![2_000_000; 2]);
    let r = run_b(&mut a, t.plus_us(2 * PACER_PERIOD_US), 12, 0, true, true);
    assert_eq!(r[0], 2_200_000);
    assert_eq!(*r.last().expect("last"), 4_000_000);
}

#[test]
fn increase_only_up_to_the_grant() {
    let t = Instant::ZERO;
    let mut p = Pacer::new(t, PacerPath::L2cap, 1024);
    p.set_grant(t, 60_000);
    // Demand fits: R stays at its initial rate.
    assert_eq!(run(&mut p, t, 2, 0, true), vec![60_000, 60_000]);
    // Backlogged: 60 ≤ 60 → 64; then 64 > grant: frozen.
    let r = run_b(&mut p, t.plus_us(2 * PACER_PERIOD_US), 3, 0, true, true);
    assert_eq!(r, vec![64_000, 64_000, 64_000]);
    assert_eq!(p.send_rate_bps(), 60_000);
}

#[test]
fn token_bucket_paces_at_send_rate() {
    let t = Instant::ZERO;
    let mut p = Pacer::new(t, PacerPath::L2cap, 118);
    // 60 kbps × 40 ms = 300 B of depth: two 118 B frames pass, the third waits.
    assert!(p.can_send(t, 118));
    p.on_sent(t, 118);
    assert!(p.can_send(t, 118));
    p.on_sent(t, 118);
    assert!(!p.can_send(t, 118));
    // 118 B at 60 kbps ≈ 15.7 ms, minus the 64 B left.
    let w = p.wait_us(t, 118);
    assert!((7_000..=8_000).contains(&w), "wait {w}");
    assert!(p.can_send(t.plus_us(w), 118));
    // Over one second, throughput ≈ R.
    let mut p = Pacer::new(t, PacerPath::L2cap, 118);
    let mut sent = 0u32;
    let mut now = t;
    while now < Instant::from_secs(10) {
        if p.can_send(now, 100) {
            p.on_sent(now, 100);
            sent += 100;
        }
        now = now.plus_us(1_000);
    }
    let bps = sent * 8 / 10;
    assert!((59_000..=61_000).contains(&bps), "{bps}");
}
