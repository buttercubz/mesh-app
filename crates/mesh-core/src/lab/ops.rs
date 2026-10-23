//! Lab mode operations: LAB frame plaintexts, timers, UI commands, audio and platform
//! inputs.

use mesh_api::{
    CommandResult, ErrorCode, NoticeKey, OkLabRun, OkLabRuns, OkTrue, ParamValue, Params,
    UiCommand, UiEvent,
};
use mesh_lab::json::Json;
use mesh_lab::{RunParams, RunRole, SUMMARY_CSV_HEADER, summary_csv};
use mesh_wire::record::{LabCtlOp, Record, RecordIter};

use super::flood::FloodOutcome;
use super::run::{LabRun, RunSetup};
use super::{LabMode, LabPeer, convert, ctl};
use crate::effects::{Effect, LabFile};
use crate::engine::{Ctx, EngineMode};
use crate::links::{Prio, RadioAvail};
use crate::{AudioIoState, BLOCK_SAMPLES, Instant, LinkId, NodeId};

impl LabMode {
    /// Handles the record plaintext of an authenticated LAB frame.
    pub fn on_plaintext(&mut self, ctx: &mut Ctx, link: LinkId, pt: &[u8]) {
        let now = ctx.now;
        if let Some(p) = self.peers.get_mut(&link) {
            p.last_rx = now;
        }
        for rec in RecordIter::new(pt) {
            let Ok(rec) = rec else {
                break;
            };
            if ctx.links.on_link_record(now, link, &rec, &mut ctx.fx) {
                continue;
            }
            match rec {
                Record::LabCtl(c) => self.on_ctl(ctx, link, &c),
                Record::Mesh(body) => self.on_mesh(ctx, link, body),
                _ => {}
            }
        }
        if let Some(run) = self.run.as_mut() {
            let on_air = ctx.links.get(link).map_or(pt.len(), |l| {
                pt.len() + l.security.overhead() + l.framer.overhead(pt.len())
            });
            // A malformed record ends the frame; the error is counted by the runner.
            let _ = run.runner.on_rx(now.lab(), link.0, on_air, pt);
            run.pump(ctx);
            self.after_run_step(ctx);
        }
    }

    fn on_ctl(&mut self, ctx: &mut Ctx, link: LinkId, c: &mesh_wire::record::LabCtl<'_>) {
        match c.op {
            LabCtlOp::Announce => {
                let Ok((id, nonce)) = c.announce() else {
                    return;
                };
                if c.run_id == ctl::ANNOUNCE_RELAYED {
                    self.check_collision(ctx, id, nonce);
                    return;
                }
                self.on_direct_announce(ctx, link, id, nonce);
            }
            LabCtlOp::Start => {
                if let Some(s) = ctl::decode_start(c) {
                    self.follow(ctx, s);
                }
            }
            LabCtlOp::Stop => {
                if self.run.as_ref().is_some_and(|r| r.wire_id == c.run_id) {
                    self.forward_ctl(ctx, Some(link), LabCtlOp::Stop, c.run_id, c.bench, &[]);
                    self.end_run(ctx, true);
                }
            }
            // Marks are recorded by the runner (on_rx); pattern changes are reserved.
            LabCtlOp::Mark | LabCtlOp::Pattern => {}
        }
    }

    fn on_direct_announce(&mut self, ctx: &mut Ctx, link: LinkId, id: u16, nonce: u64) {
        let first = self.peers.get(&link).is_none_or(|p| p.node.is_none());
        self.peers.insert(
            link,
            LabPeer {
                node: Some(id),
                nonce,
                last_rx: ctx.now,
            },
        );
        ctx.links.set_peer(link, NodeId(id));
        if let (Some(run), Some((kind, role))) = (
            self.run.as_mut(),
            ctx.links.get(link).map(|l| (l.info.kind, l.info.role)),
        ) {
            run.runner.on_link_up(
                ctx.now.lab(),
                link.0,
                kind.lab(),
                Some(super::run::role_of(role)),
                Some(id),
            );
        }
        if first {
            let mut params = Params::new();
            params.insert("labNode".into(), ParamValue::from(u32::from(id)));
            ctx.notice(NoticeKey::LabPeerJoined, params);
            ctx.mark_dirty();
        }
        // One hop further, so two neighbors of this node that are not linked to each
        // other still detect a shared id.
        let others: Vec<(LinkId, LabPeer)> = self
            .peers
            .iter()
            .filter(|(l, _)| **l != link)
            .map(|(l, p)| (*l, *p))
            .collect();
        for (l, p) in others {
            self.announce(ctx, l, id, nonce, ctl::ANNOUNCE_RELAYED);
            if p.node == Some(id) && p.nonce != nonce {
                // Two of my neighbors share an id: tell the newcomer about the other one
                // too, so whichever has the larger nonce redraws.
                self.announce(ctx, link, id, p.nonce, ctl::ANNOUNCE_RELAYED);
            }
        }
        self.check_collision(ctx, id, nonce);
    }

    fn on_mesh(&mut self, ctx: &mut Ctx, link: LinkId, body: &[u8]) {
        let peer = self.peers.get(&link).and_then(|p| p.node);
        match self.flood.on_packet(ctx, link, peer, self.node_id(), body) {
            FloodOutcome::Delivered(d) => {
                if let Some(run) = self.run.as_mut() {
                    run.runner
                        .on_mesh_probe(ctx.now.lab(), d.origin, &d.probe, d.hops, d.age_us);
                }
            }
            FloodOutcome::Collision { their_sess } => {
                // Same id, other session: the larger session tag redraws.
                if ctx.sess >= their_sess {
                    self.redraw(ctx);
                }
            }
            FloodOutcome::Dropped => {}
        }
    }

    /// A controller's `LAB_CTL{start}`: this phone follows the run (no audio command:
    /// capture and sessions start only inside a local gesture, §3.2) and passes the start
    /// on to its other Lab links so multi-hop benches (T0b) reach every phone.
    fn follow(&mut self, ctx: &mut Ctx, s: ctl::StartCtl) {
        if self.run.is_some() {
            return;
        }
        let started_at = convert::iso_ms(ctx.device.wall_ms(ctx.now));
        let peers = self.peer_ids();
        let device = ctx.cfg.device.clone();
        let platform = ctx.cfg.platform;
        let mut run = LabRun::new(
            ctx,
            RunSetup {
                wire_id: s.run_id,
                unix_minutes: s.unix_minutes,
                params: s.params,
                role: RunRole::Follower,
                lab_node: self.node_id(),
                peers,
                started_at: started_at.clone(),
                device: &device,
                platform,
            },
        );
        self.index.started(run.info(started_at), &mut ctx.fx);
        run.flush_lines(ctx);
        self.run = Some(run);
        self.send_start(ctx, None);
        ctx.mark_dirty();
    }

    fn peer_ids(&self) -> Vec<u16> {
        let mut ids: Vec<u16> = self.peers.values().filter_map(|p| p.node).collect();
        ids.sort_unstable();
        ids.dedup();
        ids
    }

    fn forward_ctl(
        &self,
        ctx: &mut Ctx,
        except: Option<LinkId>,
        op: LabCtlOp,
        run_id: u32,
        bench: u8,
        params: &[u8],
    ) {
        let rec = ctl::record(op, run_id, bench, params);
        for l in self.lab_links(ctx) {
            if Some(l) != except {
                ctx.links.send_record(l, Prio::P0, &rec, None);
            }
        }
    }

    /// Timers: the run's seconds, traffic, MESH_PROBE origination, progress.
    pub fn on_timer(&mut self, ctx: &mut Ctx) {
        self.flood.expire(ctx.now);
        let me = self.node_id();
        let Some(run) = self.run.as_mut() else {
            return;
        };
        run.last_tick = Some(ctx.now);
        run.runner.tick(ctx.now.lab());
        run.pump(ctx);
        while let Some(probe) = run.runner.poll_mesh_probe(ctx.now.lab()) {
            self.flood.originate(ctx, me, &probe);
        }
        self.after_run_step(ctx);
    }

    fn after_run_step(&mut self, ctx: &mut Ctx) {
        let Some(run) = self.run.as_mut() else {
            return;
        };
        run.flush_lines(ctx);
        if run.runner.is_finished() {
            self.end_run(ctx, false);
        } else {
            run.progress(ctx);
        }
    }

    /// The next instant Lab mode needs a timer.
    pub fn next_deadline(&self, ctx: &Ctx) -> Option<Instant> {
        self.run.as_ref().and_then(|r| r.next_deadline(ctx))
    }

    /// Ends the active run (`stop`: user or peer stop; otherwise it timed out): summary
    /// line and CSV, `labFinished`, audio closed, index updated.
    pub(crate) fn end_run(&mut self, ctx: &mut Ctx, stop: bool) {
        let Some(mut run) = self.run.take() else {
            return;
        };
        if stop {
            run.runner.stop(ctx.now.lab());
        }
        run.flush_lines(ctx);
        let line = run.runner.run_line();
        if let Some(s) = run.runner.summary().cloned() {
            for l in [SUMMARY_CSV_HEADER.to_string(), summary_csv(&line, &s)] {
                ctx.fx.push(Effect::LabWrite {
                    run_id: run.run_id.clone(),
                    file: LabFile::SummaryCsv,
                    line: l,
                });
            }
            let summary = convert::summary(&s);
            self.index.finished(&run.run_id, &summary, &mut ctx.fx);
            ctx.ui_event(UiEvent::LabFinished {
                run_id: run.run_id.clone(),
                summary,
                files: super::RunIndex::files(&run.run_id),
            });
        }
        run.close_audio(ctx);
        self.click.clear();
        self.click_pos = 0;
        ctx.radio.set_prefer(crate::links::PreferVia::L2cap);
        ctx.mark_dirty();
    }
}

impl LabMode {
    /// Lab UI commands (§3.7): `labStart` (controller), `labStop`, `labMark`,
    /// `labListRuns`, `labDeleteRun`.
    pub fn on_command(&mut self, ctx: &mut Ctx, cmd: &UiCommand) -> CommandResult {
        match cmd {
            UiCommand::LabListRuns {} => CommandResult::LabRuns(OkLabRuns {
                ok: OkTrue,
                runs: self.index.list(),
            }),
            UiCommand::LabDeleteRun { run_id } => {
                if self.run.as_ref().is_some_and(|r| &r.run_id == run_id) {
                    CommandResult::error(ErrorCode::LabRunActive)
                } else if self.index.delete(run_id, &mut ctx.fx) {
                    CommandResult::done()
                } else {
                    CommandResult::error(ErrorCode::UnknownLabRun)
                }
            }
            UiCommand::LabStart { bench, params } => match ctx.mode {
                EngineMode::Idle => CommandResult::error(ErrorCode::EngineNotRunning),
                EngineMode::Session { .. } => CommandResult::error(ErrorCode::WrongMode),
                EngineMode::Lab { .. } => self.start_run(ctx, *bench, params),
            },
            UiCommand::LabStop {} => {
                let Some(run) = self.run.as_ref() else {
                    return CommandResult::error(ErrorCode::NoLabRun);
                };
                if run.role == RunRole::Controller {
                    let (id, bench) = (run.wire_id, run.runner.params().bench.wire_id());
                    self.forward_ctl(ctx, None, LabCtlOp::Stop, id, bench, &[]);
                }
                self.end_run(ctx, true);
                CommandResult::done()
            }
            UiCommand::LabMark { label } => {
                let Some(run) = self.run.as_mut() else {
                    return CommandResult::error(ErrorCode::NoLabRun);
                };
                let label: String = label.chars().take(64).collect();
                run.runner.mark(ctx.now.lab(), &label);
                let (id, bench) = (run.wire_id, run.runner.params().bench.wire_id());
                self.forward_ctl(ctx, None, LabCtlOp::Mark, id, bench, label.as_bytes());
                if let Some(run) = self.run.as_mut() {
                    run.flush_lines(ctx);
                }
                CommandResult::done()
            }
            _ => CommandResult::error(ErrorCode::InvalidCommand),
        }
    }

    fn start_run(
        &mut self,
        ctx: &mut Ctx,
        bench: mesh_api::LabBench,
        params: &mesh_api::LabParams,
    ) -> CommandResult {
        if self.run.is_some() {
            return CommandResult::error(ErrorCode::LabRunActive);
        }
        let Ok(rp) = RunParams::resolve(convert::bench(bench), &convert::params(params)) else {
            return CommandResult::error(ErrorCode::InvalidParams);
        };
        let wall = ctx.device.wall_ms(ctx.now);
        let started_at = convert::iso_ms(wall);
        let wire_id = ctx.rand_u32();
        let peers = self.peer_ids();
        let device = ctx.cfg.device.clone();
        let platform = ctx.cfg.platform;
        let mut run = LabRun::new(
            ctx,
            RunSetup {
                wire_id,
                unix_minutes: (wall / 60_000) as u32,
                params: rp,
                role: RunRole::Controller,
                lab_node: self.node_id(),
                peers,
                started_at: started_at.clone(),
                device: &device,
                platform,
            },
        );
        run.open_audio(ctx);
        self.index.started(run.info(started_at), &mut ctx.fx);
        run.flush_lines(ctx);
        let run_id = run.run_id.clone();
        self.run = Some(run);
        self.send_start(ctx, None);
        ctx.mark_dirty();
        CommandResult::LabRun(OkLabRun { ok: OkTrue, run_id })
    }

    /// 10 ms of capture (audio `full`): feeds the T5a/T5b loopback correlator.
    pub fn on_capture(&mut self, ctx: &mut Ctx, pcm: &[i16; BLOCK_SAMPLES]) {
        let index = self.capture_index;
        self.capture_index += BLOCK_SAMPLES as u64;
        if let Some(run) = self.run.as_mut()
            && run.capture_on
        {
            run.runner.on_capture(ctx.now.lab(), index, pcm);
        }
    }

    /// 10 ms of playout: the bench click train (T5a/T5b), silence otherwise.
    pub fn on_playout(&mut self, ctx: &mut Ctx, out: &mut [i16; BLOCK_SAMPLES]) {
        if self.click_pos >= self.click.len()
            && let Some(run) = self.run.as_mut()
            && run.audio_open
            && let Some(train) = run.runner.poll_click(ctx.now.lab(), self.capture_index)
        {
            self.click = train;
            self.click_pos = 0;
        }
        let rest = &self.click[self.click_pos.min(self.click.len())..];
        let n = rest.len().min(BLOCK_SAMPLES);
        out[..n].copy_from_slice(&rest[..n]);
        self.click_pos += n;
    }

    /// The 1 Hz platform sample for the runner (GPS only with the run's consent; the
    /// runner drops it otherwise).
    pub fn on_platform(&mut self, ctx: &mut Ctx) {
        let ble = ctx.radio.ble;
        if let Some(run) = self.run.as_mut() {
            let p = run.runner.params();
            let s = convert::platform_sample(&ctx.device, ble, p.audio, p.anchor);
            if s.anchor.ok == Some(false) && run.anchor != mesh_api::LabAnchor::None {
                ctx.notice(NoticeKey::LabAnchorLost, Params::new());
            }
            run.runner.on_platform(ctx.now.lab(), s);
            // Ride recorder: the media latency estimate rides along each second
            // (§12.6) while a session's media exports one.
            if run.bench == mesh_api::LabBench::Ride
                && let Some(lat) = ctx.media_latency.as_ref()
            {
                let fields = convert::latency_fields(lat);
                run.runner.on_event(ctx.now.lab(), "latency", fields);
            }
            run.flush_lines(ctx);
        }
    }

    /// Audio route change during a run.
    pub fn on_route(&mut self, ctx: &mut Ctx) {
        let route = ctx.device.route.kind.as_str();
        if let Some(run) = self.run.as_mut() {
            run.runner.on_event(
                ctx.now.lab(),
                "routeChange",
                vec![("route".into(), Json::from(route))],
            );
            run.flush_lines(ctx);
        }
    }

    /// Audio IO state during a run (`captureLost` feeds the T2 silent-mute check).
    pub fn on_audio_io(&mut self, ctx: &mut Ctx, s: AudioIoState) {
        let ev = match s {
            AudioIoState::Interrupted { .. } => "interruption",
            AudioIoState::Restarting => "audioRestarting",
            AudioIoState::CaptureLost => "captureLost",
            AudioIoState::RenderLost => "renderLost",
            AudioIoState::Restored => "audioRestored",
            AudioIoState::Stopped => "captureStopped",
            AudioIoState::Running => "audioRunning",
        };
        if let Some(run) = self.run.as_mut() {
            run.runner.on_event(ctx.now.lab(), ev, Vec::new());
            run.flush_lines(ctx);
        }
    }

    /// Bluetooth availability during a run.
    pub fn on_radio_state(&mut self, ctx: &mut Ctx, ble: RadioAvail) {
        if let Some(run) = self.run.as_mut() {
            run.runner.on_event(
                ctx.now.lab(),
                "bluetooth",
                vec![("state".into(), Json::from(ble.lab().as_str()))],
            );
            run.flush_lines(ctx);
        }
    }
}
