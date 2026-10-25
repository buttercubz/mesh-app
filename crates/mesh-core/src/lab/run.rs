//! One Lab run on this device: the mesh-lab `BenchRunner`, its audio path and anchor
//! (DESIGN §12.1), the traffic pump, `labProgress` at 2 Hz and the end of the run
//! (`labrun/1` lines, summary CSV, `labFinished`, run index).

use mesh_api::{LabAnchor, LabAudio, LabBench, LabRunInfo, UiEvent};
use mesh_lab::json::Json;
use mesh_lab::record::{DeviceInfo, PeerInfo};
use mesh_lab::{Audio, BenchRunner, LabRecord, RunConfig, RunParams, RunRole, Transport};
use mesh_wire::record::{RecordIter, rtype};

use super::convert;
use super::index::RunIndex;
use crate::effects::{AudioCommand, AudioSessionKind, AudioSessionProfile, Effect, LabFile};
use crate::engine::Ctx;
use crate::links::{LinkState, PreferVia, Prio};
use crate::{Instant, LinkId};

/// LAB frames queued per link before the runner sees the link as blocked (keeps the
/// OS-side queue short so probes measure the radio, not our buffer).
const MAX_QUEUED_LAB_FRAMES: usize = 2;

/// A running Lab run.
#[derive(Debug)]
pub struct LabRun {
    pub wire_id: u32,
    /// Controller's wall clock in Unix minutes (names the run on every phone).
    pub unix_minutes: u32,
    pub run_id: String,
    pub role: RunRole,
    pub runner: BenchRunner,
    pub bench: LabBench,
    pub audio: LabAudio,
    pub anchor: LabAnchor,
    pub started: Instant,
    pub(super) audio_open: bool,
    pub(super) capture_on: bool,
    next_progress: Instant,
    /// Instant of the last timer pass (spin guard for "always due" runner deadlines).
    pub(super) last_tick: Option<Instant>,
}

/// Everything needed to create a run.
pub struct RunSetup<'a> {
    pub wire_id: u32,
    pub unix_minutes: u32,
    pub params: RunParams,
    pub role: RunRole,
    pub lab_node: u16,
    pub peers: Vec<u16>,
    pub started_at: String,
    pub device: &'a crate::config::DeviceDescriptor,
    pub platform: crate::config::Platform,
}

fn variant(p: &RunParams) -> Vec<(String, Json)> {
    let mut v = vec![
        ("transport".to_string(), Json::from(p.transport.as_str())),
        ("role".to_string(), Json::from(p.role.as_str())),
        (
            "headset".to_string(),
            Json::from(if p.headset { "hfp" } else { "off" }),
        ),
        ("pattern".to_string(), Json::from(p.pattern.as_str())),
    ];
    if let Some(label) = &p.variant {
        v.push(("label".to_string(), Json::from(label.as_str())));
    }
    v
}

impl LabRun {
    /// Creates the run and its `run` line.
    pub fn new(ctx: &mut Ctx, s: RunSetup<'_>) -> LabRun {
        let run_id = convert::run_id(s.unix_minutes, s.wire_id);
        let cfg = RunConfig {
            run_id: run_id.clone(),
            variant: variant(&s.params),
            started_at: s.started_at,
            device: DeviceInfo {
                model: s.device.model.clone(),
                os: s.device.os.clone(),
                app: s.device.app.clone(),
                platform: s.platform.lab(),
            },
            lab_node: s.lab_node,
            role: s.role,
            peers: s
                .peers
                .iter()
                .map(|n| PeerInfo {
                    lab_node: *n,
                    model: None,
                })
                .collect(),
            params: s.params.clone(),
        };
        let now = ctx.now;
        let mut runner = BenchRunner::new(cfg, now.lab());
        for l in ctx
            .links
            .iter()
            .filter(|l| l.state == LinkState::Up && l.is_lab())
        {
            runner.on_link_up(
                now.lab(),
                l.id.0,
                l.info.kind.lab(),
                Some(role_of(l.info.role)),
                l.peer_node.map(|n| n.0),
            );
        }
        ctx.radio.set_prefer(match s.params.transport {
            Transport::GattNotify | Transport::GattWrite => PreferVia::Gatt,
            Transport::L2cap | Transport::Aware => PreferVia::L2cap,
        });
        LabRun {
            wire_id: s.wire_id,
            unix_minutes: s.unix_minutes,
            run_id,
            role: s.role,
            bench: convert::api_bench(s.params.bench),
            audio: convert::api_audio(s.params.audio),
            anchor: convert::api_anchor(s.params.anchor),
            runner,
            started: now,
            audio_open: false,
            capture_on: false,
            next_progress: now.plus_us(ctx.cfg.limits.lab_progress_interval_us),
            last_tick: None,
        }
    }

    /// The index entry of this run.
    pub fn info(&self, started_at: String) -> LabRunInfo {
        LabRunInfo {
            run_id: self.run_id.clone(),
            bench: self.bench,
            audio: self.audio,
            anchor: self.anchor,
            started_at,
            duration_s: None,
            verdict: None,
            files: RunIndex::files(&self.run_id),
        }
    }

    /// Opens the run's audio path. Only the controller, inside its `labStart` gesture,
    /// does this (§3.2): `off` emits nothing, `renderOnly` only `OpenSession`, `full`
    /// `OpenSession` + `StartCapture`.
    pub fn open_audio(&mut self, ctx: &mut Ctx) {
        let p = self.runner.params().clone();
        let kind = match p.audio {
            Audio::Off => return,
            Audio::RenderOnly => AudioSessionKind::LabRender,
            Audio::Full => AudioSessionKind::LabFull,
        };
        let profile = AudioSessionProfile {
            kind,
            capture: p.audio == Audio::Full,
            mixable: p.mixable,
            location_anchor: p.anchor == mesh_lab::Anchor::AudioLocation,
            bypass_voice_processing: matches!(p.bench, mesh_lab::Bench::T5a),
        };
        ctx.fx.audio(AudioCommand::OpenSession { profile });
        self.audio_open = true;
        if p.audio == Audio::Full {
            ctx.fx.audio(AudioCommand::StartCapture);
            ctx.fx
                .diag(mesh_lab::ridelog::DiagEvent::CaptureStart { why: "lab" });
            self.capture_on = true;
        }
    }

    /// Closes the audio path opened by `open_audio`.
    pub fn close_audio(&mut self, ctx: &mut Ctx) {
        if self.capture_on {
            ctx.fx.audio(AudioCommand::StopCapture);
            ctx.fx
                .diag(mesh_lab::ridelog::DiagEvent::CaptureStop { why: "lab" });
            self.capture_on = false;
        }
        if self.audio_open {
            ctx.fx.audio(AudioCommand::CloseSession);
            self.audio_open = false;
        }
    }

    /// Lets the runner fill every Lab link that can take a frame now.
    pub fn pump(&mut self, ctx: &mut Ctx) {
        let now = ctx.now;
        let links: Vec<(LinkId, bool)> = ctx
            .links
            .iter()
            .filter(|l| l.state == LinkState::Up && l.is_lab())
            .map(|l| {
                let room = l.queues_len(Prio::P3) < MAX_QUEUED_LAB_FRAMES;
                (l.id, l.writable && room)
            })
            .collect();
        for (link, writable) in links {
            let mut writable = writable;
            for _ in 0..64 {
                let Some(tx) = self.runner.poll_tx(now.lab(), link.0, writable) else {
                    break;
                };
                let answer = RecordIter::new(&tx.records)
                    .next_raw()
                    .and_then(Result::ok)
                    .is_some_and(|r| r.rtype == rtype::LECHO || r.rtype == rtype::LAB_GPS);
                let prio = if answer { Prio::P2 } else { Prio::P3 };
                ctx.links.send_plaintext(link, prio, tx.records);
                if prio == Prio::P3 {
                    writable = ctx.links.get(link).is_some_and(|l| {
                        l.writable && l.queues_len(Prio::P3) < MAX_QUEUED_LAB_FRAMES
                    });
                }
            }
        }
    }

    /// Writes the lines produced so far.
    pub fn flush_lines(&mut self, ctx: &mut Ctx) {
        for rec in self.runner.drain() {
            let line = rec.to_jsonl();
            ctx.fx.push(Effect::LabWrite {
                run_id: self.run_id.clone(),
                file: LabFile::Jsonl,
                line,
            });
            if let LabRecord::Run(_) = rec {
                ctx.mark_dirty();
            }
        }
    }

    /// `labProgress` at 2 Hz.
    pub fn progress(&mut self, ctx: &mut Ctx) {
        if ctx.now < self.next_progress {
            return;
        }
        self.next_progress = ctx.now.plus_us(ctx.cfg.limits.lab_progress_interval_us);
        ctx.ui_event(UiEvent::LabProgress {
            run_id: self.run_id.clone(),
            elapsed_s: convert::secs(self.started, ctx.now),
            sample: convert::live(&self.runner.live()),
        });
    }

    /// The next instant the run needs a timer.
    pub fn next_deadline(&self, ctx: &Ctx) -> Option<Instant> {
        let now = ctx.now;
        let mut t = self.runner.next_deadline(now.lab()).map(|i| Instant(i.0));
        // Saturate traffic is "always due": while every Lab link is full, wait for the
        // transmit side instead of spinning.
        if t == Some(now) && !self.can_send(ctx) {
            t = Some(now.plus_us(1_000));
        }
        // Click trains (T5a/T5b) are driven by `pull_playout` every 10 ms, not by
        // timers; and a deadline the last timer pass did not move would spin.
        if let (Some(d), Some(last)) = (t, self.last_tick)
            && d <= last
        {
            t = Some(last.plus_us(10_000));
        }
        Some(t.map_or(self.next_progress, |t| t.min(self.next_progress)))
    }

    fn can_send(&self, ctx: &Ctx) -> bool {
        ctx.links.iter().any(|l| {
            l.state == LinkState::Up
                && l.is_lab()
                && l.writable
                && l.queues_len(Prio::P3) < MAX_QUEUED_LAB_FRAMES
        })
    }
}

/// labrun/1 link role of the local phone.
pub fn role_of(r: crate::links::LinkRole) -> mesh_lab::LinkRole {
    match r {
        crate::links::LinkRole::Initiator => mesh_lab::LinkRole::Central,
        crate::links::LinkRole::Responder => mesh_lab::LinkRole::Peripheral,
    }
}
