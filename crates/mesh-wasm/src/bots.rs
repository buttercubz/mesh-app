//! The bots' behaviour: where they ride (a group session or Lab mode), going live in
//! PTT, their random talk schedule, the local phone's synthetic microphone, and the
//! periodic trimming that keeps a long browser session's memory flat.

use mesh_api::{ErrorCode, RideOp, RidePhase, SessionInfo, TalkMode, TalkState, UiCommand};
use mesh_core::{EngineMode, PlatformEvent, SessionStart};
use mesh_sim::ride::rider_name;
use mesh_sim::world::psm_of;

use crate::convoy::{BOT_BURST_MS, BotHome, HOUSEKEEP_US, LOCAL, SimConvoy, TRIM_US};

/// Retry period of a bot's go-live while its session refuses it.
const LIVE_RETRY_US: u64 = 1_000_000;
/// Diagnostic histories longer than this are cut back to their newest half.
const HISTORY_CAP: usize = 256;
/// Speech spans that ended more than this long ago are forgotten.
const SPAN_KEEP_US: u64 = 1_000_000;

/// Rounds `t` up to the 10 ms capture block grid (as `World::script_talk` does).
fn block_ceil(t: u64) -> u64 {
    t.div_ceil(10_000) * 10_000
}

impl SimConvoy {
    /// Starts node `i`'s group session in `id` (lobby) the way the platform does: the
    /// engine starts and the platform publishes its L2CAP PSM.
    pub(crate) fn start_group_node(&mut self, i: usize, id: [u8; 16]) -> bool {
        let psm = psm_of(i);
        let ok = self.world.with_engine(i, |e, now| {
            let r = e.start(now, SessionStart::Group { group_id: id });
            if r.is_ok() {
                e.handle_event(now, PlatformEvent::L2capPublished { psm });
            }
            r.is_ok()
        });
        if ok {
            self.world.nodes[i].psm = psm;
        }
        ok
    }

    /// Starts node `i` in Lab mode with `code`.
    pub(crate) fn start_lab_node(&mut self, i: usize, code: u16) -> bool {
        let psm = psm_of(i);
        let ok = self.world.with_engine(i, |e, now| {
            let r = e.start(now, SessionStart::Lab { lab_code: code });
            if r.is_ok() {
                e.handle_event(now, PlatformEvent::L2capPublished { psm });
            }
            r.is_ok()
        });
        if ok {
            self.world.nodes[i].psm = psm;
        }
        ok
    }

    /// Ends every bot's scripted speech and marks it not live.
    fn bots_reset(&mut self) {
        let now = self.world.now.0;
        for k in 0..self.bots.len() {
            let node = self.bots[k].node;
            if let Some(spans) = self.world.nodes[node].audio.talk.as_mut() {
                spans.clear();
            }
            if self.bots[k].talking_until.take().is_some() {
                self.world.ptt(node, false);
            }
            self.bots[k].live = false;
            self.bots[k].next_live_try = now;
        }
    }

    /// Every bot (re)starts its session in group `id`, joining it from the invite of a
    /// member first when it is not stored yet.
    pub(crate) fn bots_enter_group(&mut self, id: [u8; 16]) {
        self.bots_reset();
        let url = (0..self.world.nodes.len()).find_map(|i| self.world.invite_url(i, &id));
        for k in 0..self.bots.len() {
            let node = self.bots[k].node;
            self.world.stop_node(node);
            if self.world.group_of(node, &id).is_none()
                && let Some(url) = url.as_deref()
            {
                let r = self.world.command(
                    node,
                    UiCommand::JoinGroup {
                        url: url.to_string(),
                        display_name: rider_name(node),
                    },
                );
                if !r.is_ok() {
                    continue;
                }
            }
            self.start_group_node(node, id);
        }
        self.home = BotHome::Group(id);
    }

    /// Every bot starts Lab mode with `code` (followers of the local controller).
    pub(crate) fn bots_enter_lab(&mut self, code: u16) {
        self.bots_reset();
        for k in 0..self.bots.len() {
            let node = self.bots[k].node;
            self.start_lab_node(node, code);
        }
        self.home = BotHome::Lab(code);
    }

    /// Back to the bots' own ride (after a Lab run or when their group goes away).
    pub(crate) fn bots_go_home(&mut self) {
        let id = self.bot_group.id;
        self.bots_enter_group(id);
    }

    /// Bot `k` asks to speak for `ms` from the next capture block. Refused (`false`)
    /// while it is not live or already speaking.
    pub(crate) fn bot_talk(&mut self, k: usize, ms: u64) -> bool {
        let now = self.world.now.0;
        let Some(bot) = self.bots.get(k) else {
            return false;
        };
        if !bot.live || bot.talking_until.is_some_and(|t| t > now) {
            return false;
        }
        let node = bot.node;
        let from = block_ceil(now + 1);
        let until = block_ceil(from + ms.max(10) * 1_000);
        self.world.script_talk(node, from, until, true);
        self.bots[k].talking_until = Some(until);
        true
    }

    /// The local snapshot's session view.
    pub(crate) fn local_session(&self) -> Option<&SessionInfo> {
        self.world.nodes[LOCAL].snapshot.as_ref()?.session.as_ref()
    }

    /// Bot `node` is in the lobby of a ride it should be riding.
    fn bot_should_start_ride(&self, node: usize) -> bool {
        let ride_of = |i: usize| {
            self.world.nodes[i]
                .snapshot
                .as_ref()
                .and_then(|s| s.session.as_ref())
                .map(|s| s.ride)
        };
        if ride_of(node) != Some(RidePhase::Lobby) {
            return false;
        }
        match self.home {
            BotHome::Group(id) if id == self.bot_group.id => true,
            BotHome::Group(_) => ride_of(LOCAL) == Some(RidePhase::Riding),
            BotHome::Lab(_) => false,
        }
    }

    /// Opens or closes the local microphone's speech span: it speaks while the PTT is
    /// held, while the page simulates speech (VOX) or while the engine says it is
    /// talking (latch), so PTT produces real frames through the capture chain.
    pub(crate) fn sync_local_mic(&mut self) {
        let talking = self
            .local_session()
            .is_some_and(|s| s.live && s.talk == TalkState::Talking);
        let want = self.local_ptt || self.local_speak || talking;
        if want == self.local_mic_open {
            return;
        }
        let t = block_ceil(self.world.now.0);
        let spans = self.world.nodes[LOCAL]
            .audio
            .talk
            .get_or_insert_with(Vec::new);
        if want {
            spans.push((t, u64::MAX));
        } else if let Some(last) = spans.last_mut()
            && last.1 == u64::MAX
        {
            last.1 = t.max(last.0);
        }
        self.local_mic_open = want;
    }

    /// One housekeeping pass: bots go live and talk on schedule, the local microphone
    /// follows the page, and (every [`TRIM_US`]) diagnostic records are trimmed.
    pub(crate) fn housekeep(&mut self) {
        let now = self.world.now.0;
        self.next_housekeep = now + HOUSEKEEP_US;
        self.sync_local_mic();
        for k in 0..self.bots.len() {
            if self.cfg.roam && matches!(self.home, BotHome::Group(_)) {
                self.roam_tick(k, now);
            }
            if let BotHome::Group(_) = self.home {
                self.bot_tick(k, now);
            }
        }
        if now >= self.next_trim {
            self.next_trim = now + TRIM_US;
            self.trim(now);
        }
    }

    fn bot_tick(&mut self, k: usize, now: u64) {
        let node = self.bots[k].node;
        if !matches!(
            self.world.nodes[node].engine.mode(),
            EngineMode::Session { .. }
        ) {
            self.bots[k].live = false;
            return;
        }
        if self.bots[k].talking_until.is_some_and(|t| t <= now) {
            self.bots[k].talking_until = None;
        }
        if now >= self.bots[k].next_live_try {
            if !self.bots[k].live {
                self.bots[k].next_live_try = now + LIVE_RETRY_US;
                let r = self.world.command(node, UiCommand::GoLive {});
                let live = r.is_ok() || r.error_code() == Some(ErrorCode::AlreadyLive);
                if live {
                    let m = self.world.command(
                        node,
                        UiCommand::SetTalkMode {
                            mode: TalkMode::Ptt,
                        },
                    );
                    self.bots[k].live = m.is_ok();
                }
                return;
            }
            if self.bot_should_start_ride(node) {
                // In their own ride the bots start riding on their own (the leader's
                // start is signed and flooded, a member's is local); in the page's
                // group they wait until the page's leader has started it.
                self.bots[k].next_live_try = now + LIVE_RETRY_US;
                self.world
                    .command(node, UiCommand::RideControl { op: RideOp::Start });
            }
        }
        if !self.bots[k].live {
            return;
        }
        if self.cfg.bots_talk
            && now >= self.bots[k].next_talk
            && self.bots[k].talking_until.is_none()
        {
            let ms = self.rng.range_u64(BOT_BURST_MS.0, BOT_BURST_MS.1);
            self.bot_talk(k, ms);
            self.bots[k].next_talk = self.draw_pause() + ms * 1_000;
        }
    }

    /// Forgets old speech spans and cuts the per-node diagnostic histories and receive
    /// taps the simulator keeps for scenarios, so memory stays flat over hours.
    fn trim(&mut self, now: u64) {
        let keep_after = now.saturating_sub(SPAN_KEEP_US);
        for n in &mut self.world.nodes {
            if let Some(spans) = n.audio.talk.as_mut() {
                spans.retain(|(_, until)| *until >= keep_after);
            }
            cut(&mut n.talk_hist);
            cut(&mut n.talkers_hist);
            cut(&mut n.notices);
            cut(&mut n.denied);
            let mut tap = n.tap.borrow_mut();
            tap.frames.clear();
            tap.spans.clear();
            tap.talk.clear();
            tap.nbr.clear();
        }
    }
}

fn cut<T>(v: &mut Vec<T>) {
    if v.len() > HISTORY_CAP {
        v.drain(..v.len() - HISTORY_CAP / 2);
    }
}
