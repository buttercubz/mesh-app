//! Pacing (DESIGN §5.6): the delay-based pacer per link (every 500 ms), the
//! voice-aware node airtime budget, the per-link token bucket rate `min(R, grant)`
//! applied to the link layer, caps from pacing with hysteresis published to the router
//! (and advertised as `vcap` in our NBR), and `PATH_HINT{BLE12}` when a link's grant
//! stays below `cost(1)` for 3 s.

use mesh_routing::PacingUpdate;

use super::rl;
use super::state::Mesh;
use crate::engine::Ctx;

impl Mesh {
    /// Runs the pacer control steps that are due, then applies grants and caps.
    ///
    /// The queueing delay `qd = srtt − min_rtt` comes from PING/PONG. Both ends measure
    /// as the frame is sealed (PING `t_tx`, PONG `hold_us`), so neither side's
    /// token-bucket wait (idle floor, budget scaling, demand above `R`) is counted as
    /// path delay, and a PONG whose hold saturated gives no sample: `qd` is used as is.
    pub fn pacing_tick(&mut self, ctx: &mut Ctx) {
        self.sync_profile(ctx.now);
        let now = ctx.now.routing();
        self.observe_queues(ctx);
        let mut stepped = false;
        for id in self.links.keys() {
            let Some(pl) = self.pacing.link_mut(rl(*id)) else {
                continue;
            };
            if pl.pacer.next_control() > now {
                continue;
            }
            let qd = self
                .router
                .neighbors
                .link(rl(*id))
                .map_or(0, |s| s.quality.rtt.queue_delay_us(now));
            stepped |= pl.pacer.tick(now, qd).is_some();
        }
        if stepped {
            let up = self.pacing.refresh(now, &self.router.neighbors);
            self.apply_pacing(ctx, up);
        }
    }

    /// Feeds the pacers' "queue non-empty / backlogged" inputs from the link layer and
    /// the staging queues.
    pub fn observe_queues(&mut self, ctx: &Ctx) {
        for (id, meta) in self.links.iter() {
            let Some(l) = ctx.links.get(*id) else {
                continue;
            };
            let backlog = l.backlog_bytes() > 0;
            let in_debt = l.pace.ready_at().is_some();
            if let Some(pl) = self.pacing.link_mut(rl(*id)) {
                if backlog || !meta.queue.is_empty() {
                    pl.pacer.note_queued();
                }
                if backlog && in_debt {
                    pl.pacer.note_backlogged();
                }
            }
        }
    }

    /// Recomputes grants and caps now (link set changed, a link started voice).
    pub fn refresh_pacing(&mut self, ctx: &mut Ctx) {
        self.sync_profile(ctx.now);
        let up = self
            .pacing
            .refresh(ctx.now.routing(), &self.router.neighbors);
        self.apply_pacing(ctx, up);
    }

    fn apply_pacing(&mut self, ctx: &mut Ctx, up: PacingUpdate) {
        let now = ctx.now;
        // The airtime budget is voice-aware: the 8 kbps control floor of idle links only
        // matters while some link of this node carries voice. With no voice anywhere
        // every link drains at its pacer rate `R` (Σ R ≤ the node budget at the link
        // max), so lobby control traffic (ROSTER, REKEY, SYNC) is not throttled for
        // nothing.
        let rnow = now.routing();
        let any_voice = self.router.neighbors.link_ids().any(|l| {
            self.router
                .neighbors
                .link(l)
                .is_some_and(|s| s.is_voice_link(rnow))
        });
        for (id, meta) in self.links.iter_mut() {
            let Some(pl) = self.pacing.link(rl(*id)) else {
                continue;
            };
            let rate = if any_voice {
                pl.pacer.send_rate_bps()
            } else {
                pl.pacer.rate_bps()
            };
            if meta.applied_rate != Some(rate) {
                meta.applied_rate = Some(rate);
                ctx.links.set_pace(now, *id, Some(rate));
            }
        }
        for (link, cap) in up.cap_changes {
            self.router.set_link_cap(link, cap);
        }
        // Caps that never changed since link-up still need to reach the router once.
        let ids: Vec<_> = self.links.keys().map(|id| rl(*id)).collect();
        for link in ids {
            if let Some(pl) = self.pacing.link(link) {
                let published = pl.published_cap;
                let vcap = self.router.neighbors.link(link).map(|l| l.vcap);
                let want = self.router.cfg.limits.link_cap(pl.kind, published);
                if vcap != Some(want) {
                    self.router.set_link_cap(link, published);
                }
            }
        }
        if up.path_hint {
            let out = self.router.originate_path_hint(now.routing());
            self.stats.path_hints += 1;
            self.apply(ctx, out);
        }
        // A changed vcap or congested flag goes out in the next NBR at once (≤ 4 Hz).
        self.send_nbr(ctx, false);
    }

    /// Housekeeping after anything was queued on the link layer: links that just started
    /// carrying voice get their full grant at once (§5.6: idle links are capped at the
    /// 8 kbps control floor only while idle).
    pub fn after_queue(&mut self, ctx: &mut Ctx) {
        if self.detect_voice(ctx) {
            self.refresh_pacing(ctx);
        }
    }
}
