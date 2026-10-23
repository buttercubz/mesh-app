//! S8 restart (DESIGN §13.2, §4.5, §5.3): three iPhones ride in PTT; a member talks,
//! then its app restarts mid-ride (a new engine with a fresh `sess`, loaded from the
//! phone's secure storage), rejoins, goes live again and talks. Asserted: zero frames of
//! the new session dropped as duplicates (every new-session frame reaches the listeners
//! and its speech is played), and PRESENCE of the new session back at every peer
//! ≤ 4 s after the restarted phone relinks.

use mesh_api::TalkMode;

use super::Outcome;
use super::common::{Scope, everyone, form_ride, go_live_all, group_config, voice_stats};
use crate::config::WorldConfig;
use crate::world::World;

const RESTARTED: usize = 2;
/// Bound of the presence-back assertion.
const PRESENCE_BACK_US: u64 = 4_000_000;

pub fn run(seed: u64) -> Outcome {
    run_world(seed).0
}

/// Runs S8 and returns the outcome and the final world (diagnostics).
pub fn run_world(seed: u64) -> (Outcome, World) {
    let mut o = Outcome::new("S8", seed);
    let cfg = group_config(WorldConfig::iphones_on_line(seed, 3, 8.0));
    let mut w = World::new(cfg);
    let (g, formed) = form_ride(&mut w, 0);
    if !o.check(formed.is_some(), || "the ride never formed".into()) {
        o.absorb(&w);
        return (o, w);
    }
    o.check(go_live_all(&mut w, &g, TalkMode::Ptt, true), || {
        "go live refused".into()
    });
    // The member talks in its first session (fills every dedup window with its seqs).
    let t0 = w.now.0 + 200_000;
    w.script_talk(RESTARTED, t0, t0 + 3_000_000, true);
    w.run_until(mesh_core::Instant(t0 + 4_000_000));
    let me = w.node_id(RESTARTED, &g.id).unwrap_or(0);
    let old_sess = w.nodes[RESTARTED].engine.ctx().sess;

    // The app restarts: new engine, links lost, the session starts again from storage.
    w.restart_node(RESTARTED);
    let t_restart = w.now.0;
    w.start_group(RESTARTED, &g.id);
    let new_sess = w.nodes[RESTARTED].engine.ctx().sess;
    o.report("sess.changed", f64::from(u8::from(new_sess != old_sess)));
    let relink = w.run_until_pred(30_000, 50, |w| {
        w.up_links()
            .iter()
            .any(|l| w.links[*l].peer(RESTARTED).is_some() && w.links[*l].created_at >= t_restart)
    });
    let Some(relink) = relink else {
        o.check(false, || "the restarted phone never relinked".into());
        o.absorb(&w);
        return (o, w);
    };
    // PRESENCE of the new session back at every peer.
    let back = w.run_until_pred(10_000, 20, |w| {
        (0..3).filter(|p| *p != RESTARTED).all(|p| {
            w.nodes[p]
                .tap
                .borrow()
                .presence
                .contains_key(&(me, new_sess))
        })
    });
    let mut worst = 0u64;
    for p in (0..3).filter(|p| *p != RESTARTED) {
        let first = w.nodes[p]
            .tap
            .borrow()
            .presence
            .get(&(me, new_sess))
            .map(|x| x.0);
        match first {
            Some(t) => worst = worst.max(t.saturating_sub(relink.0)),
            None => worst = u64::MAX,
        }
    }
    o.check(back.is_some(), || {
        "PRESENCE of the new session never reached every peer".into()
    });
    o.metric(
        "presence.back.ms",
        if worst == u64::MAX {
            f64::INFINITY
        } else {
            worst as f64 / 1_000.0
        },
        worst <= PRESENCE_BACK_US,
        "≤ 4000 ms after relink",
    );

    // Go live again (the rider's tap) and talk in the new session.
    w.run_s(2);
    w.mute_mic(RESTARTED);
    w.track_onsets(RESTARTED);
    o.check(w.go_live(RESTARTED, TalkMode::Ptt), || {
        "go live after restart refused".into()
    });
    // A member who rejoins a running ride taps Start locally (§1.3; never flooded).
    w.start_ride(RESTARTED);
    w.run_ms(300);
    let t1 = w.now.0 + 200_000;
    w.script_talk(RESTARTED, t1, t1 + 4_000_000, true);
    w.run_until(mesh_core::Instant(t1 + 5_500_000));
    let spans: Vec<_> = w
        .talk_spans()
        .iter()
        .copied()
        .filter(|s| s.from_us >= t1)
        .collect();
    let st = voice_stats(
        &w,
        &g,
        &spans,
        &Scope {
            listener: &everyone,
            solo_only: true,
        },
    );
    let expected: u64 = st.expected.values().sum();
    let lost = expected.saturating_sub(st.received.values().sum());
    o.check(expected > 0, || {
        "no new-session frame reached any listener".into()
    });
    o.metric(
        "new.sess.frames.lost",
        lost as f64,
        lost == 0,
        "= 0 (none dropped as duplicates)",
    );
    let heard = st.matched as f64 / st.syllables.max(1) as f64;
    o.metric(
        "new.sess.syllables.heard",
        heard,
        heard >= 0.85,
        "≥ 85 % played",
    );
    o.report("restart.relink.s", (relink.0 - t_restart) as f64 / 1e6);
    o.absorb(&w);
    (o, w)
}
