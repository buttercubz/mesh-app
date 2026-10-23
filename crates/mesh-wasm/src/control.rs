//! `mw_command`: one JSON command in, one JSON result out.
//!
//! - A mesh-api `UiCommand` (contract C1) goes to the local phone's engine and returns
//!   its `CommandResult`. The ★ gating (`send` vs `gestureCommand`) is the plugin's
//!   web implementation's job (§11.2), exactly as on a phone.
//! - A `sim.*` command is the simulated *runtime and platform* around the engine, the
//!   part `MeshIntercomPlugin` does natively (§11.1): `sim.startEngine`,
//!   `sim.stopEngine`, `sim.ptt` (the RT-flag fast path), `sim.uiAttached`, plus the
//!   simulator's own `sim.invite` (the invite `scanInviteQr` returns), `sim.info`,
//!   `sim.labCsv` (a run's summary CSV; the JSONL comes from `mw_poll_lab`),
//!   `sim.speak` (VOX speech), `sim.botTalk`, `sim.botsTalk`, and members dropping
//!   out and back: `sim.botAway {bot, away}` (out of radio range), `sim.botRestart
//!   {bot}` (app killed and relaunched) and `sim.roam {on}`. Results are
//!   `{ok:true,…}` or `{ok:false,code}` with a mesh-api `ErrorCode`.

use mesh_api::{CommandResult, ErrorCode, UiCommand};
use mesh_core::group::store;
use mesh_core::{EngineMode, rt_flags};
use serde_json::{Map, Value, json};

use crate::convoy::{BotHome, LOCAL, SimConvoy};

/// Longest scripted bot burst (`sim.botTalk`), ms: the PTT MAX_BURST (§6.1).
pub const MAX_BOT_TALK_MS: u64 = 60_000;

fn error(code: ErrorCode) -> Value {
    json!({ "ok": false, "code": code })
}

fn ok() -> Value {
    json!({ "ok": true })
}

/// The summary CSV file from the engine's `LabWrite{SummaryCsv}` lines: one header,
/// then the rows (a write may carry several physical lines, and a repeated header is
/// written only once).
pub fn summary_csv(writes: &[String]) -> String {
    let mut out = String::new();
    let mut header: Option<&str> = None;
    for line in writes
        .iter()
        .flat_map(|w| w.lines())
        .filter(|l| !l.trim().is_empty())
    {
        match header {
            None => header = Some(line),
            Some(h) if h == line => continue,
            Some(_) => {}
        }
        out.push_str(line);
        out.push('\n');
    }
    out
}

fn bool_field(m: &Map<String, Value>, k: &str) -> Option<bool> {
    m.get(k).and_then(Value::as_bool)
}

impl SimConvoy {
    /// Runs one command given as JSON; returns the result JSON.
    pub fn command_json(&mut self, json: &str) -> String {
        let r = match serde_json::from_str::<Value>(json) {
            Ok(Value::Object(m)) => match m.get("type").and_then(Value::as_str) {
                Some(t) if t.starts_with("sim.") => self.sim_command(t, &m),
                Some(_) => match serde_json::from_value::<UiCommand>(Value::Object(m)) {
                    Ok(cmd) => serde_json::to_value(self.ui_command(cmd))
                        .unwrap_or_else(|_| error(ErrorCode::InvalidCommand)),
                    Err(_) => error(ErrorCode::InvalidCommand),
                },
                None => error(ErrorCode::InvalidCommand),
            },
            _ => error(ErrorCode::InvalidCommand),
        };
        self.collect();
        r.to_string()
    }

    /// A UI command on the local phone (the plugin's `send` / `gestureCommand`).
    pub fn ui_command(&mut self, cmd: UiCommand) -> CommandResult {
        if let UiCommand::Ptt { down } = cmd {
            self.local_ptt = down;
            self.sync_local_mic();
        }
        let created = matches!(cmd, UiCommand::CreateGroup { .. });
        let r = self.world.command(LOCAL, cmd);
        if created && let CommandResult::GroupId(g) = &r {
            // The bots "scan" the new group's invite and wait in its lobby.
            if let Some(id) = store::parse_hex(&g.group_id) {
                self.bots_enter_group(id);
            }
        }
        self.sync_local_mic();
        r
    }

    fn sim_command(&mut self, t: &str, m: &Map<String, Value>) -> Value {
        match t {
            "sim.startEngine" => self.start_engine(m),
            "sim.stopEngine" => {
                self.stop_engine();
                ok()
            }
            "sim.ptt" => match bool_field(m, "down") {
                Some(down) => {
                    self.ptt(down);
                    ok()
                }
                None => error(ErrorCode::InvalidParams),
            },
            "sim.speak" => match bool_field(m, "on") {
                Some(on) => {
                    self.local_speak = on;
                    self.sync_local_mic();
                    ok()
                }
                None => error(ErrorCode::InvalidParams),
            },
            "sim.uiAttached" => match bool_field(m, "attached") {
                Some(a) => {
                    self.world.with_engine(LOCAL, |e, _| e.set_ui_attached(a));
                    ok()
                }
                None => error(ErrorCode::InvalidParams),
            },
            "sim.invite" => match self.sim_invite() {
                Some(url) => json!({ "ok": true, "url": url }),
                None => error(ErrorCode::UnknownGroup),
            },
            "sim.info" => self.info(),
            "sim.labCsv" => match m.get("runId").and_then(Value::as_str) {
                Some(run) => {
                    let csv = summary_csv(self.world.nodes[LOCAL].lab_csv(run));
                    if csv.is_empty() {
                        error(ErrorCode::UnknownLabRun)
                    } else {
                        json!({ "ok": true, "csv": csv })
                    }
                }
                None => error(ErrorCode::InvalidParams),
            },
            "sim.botTalk" => self.bot_talk_cmd(m),
            "sim.botsTalk" => match bool_field(m, "on") {
                Some(on) => {
                    self.cfg.bots_talk = on;
                    ok()
                }
                None => error(ErrorCode::InvalidParams),
            },
            "sim.botAway" => match (self.bot_arg(m), bool_field(m, "away")) {
                (Ok(k), Some(away)) => {
                    self.set_away(k, away);
                    ok()
                }
                (Err(e), _) => e,
                (_, None) => error(ErrorCode::InvalidParams),
            },
            "sim.botRestart" => match self.bot_arg(m) {
                Ok(k) => {
                    self.restart_bot(k);
                    ok()
                }
                Err(e) => e,
            },
            "sim.roam" => match bool_field(m, "on") {
                Some(on) => {
                    if on && !self.cfg.roam {
                        for k in 0..self.bots.len() {
                            self.bots[k].next_roam = self.draw_roam_stay();
                        }
                    }
                    self.cfg.roam = on;
                    ok()
                }
                None => error(ErrorCode::InvalidParams),
            },
            _ => error(ErrorCode::InvalidCommand),
        }
    }

    /// The `bot` field (a world node index) as an index into `bots`.
    fn bot_arg(&self, m: &Map<String, Value>) -> Result<usize, Value> {
        let Some(bot) = m.get("bot").and_then(Value::as_u64) else {
            return Err(error(ErrorCode::InvalidParams));
        };
        usize::try_from(bot)
            .ok()
            .and_then(|b| self.bot_index(b))
            .ok_or_else(|| error(ErrorCode::UnknownMember))
    }

    /// The RT-flag PTT fast path (`MeshIntercom.ptt`, §3.6): the flag changes at once
    /// and the microphone follows.
    pub fn ptt(&mut self, down: bool) {
        self.local_ptt = down;
        self.sync_local_mic();
        let flags = self.world.nodes[LOCAL].engine.ctx().rt_flags;
        let flags = if down {
            flags | rt_flags::PTT_DOWN
        } else {
            flags & !rt_flags::PTT_DOWN
        };
        self.world.with_engine(LOCAL, |e, now| {
            e.handle_timeout(now);
            e.set_rt_flags(flags);
        });
        self.sync_local_mic();
    }

    /// `startEngine({groupId?, lab?:{code}, demo?})` (§11.1): a group session in the
    /// lobby, Lab mode (the bots follow with the same code), or the Demo ride (the
    /// local phone joins the bots' ride).
    fn start_engine(&mut self, m: &Map<String, Value>) -> Value {
        if self.local_mode() != EngineMode::Idle {
            return error(ErrorCode::WrongMode);
        }
        if let Some(lab) = m.get("lab").filter(|v| !v.is_null()) {
            let code = match lab.get("code") {
                Some(Value::String(s)) => s.trim().parse::<u16>().ok(),
                Some(Value::Number(n)) => n.as_u64().and_then(|x| u16::try_from(x).ok()),
                _ => None,
            };
            let Some(code) = code.filter(|c| *c <= 9_999) else {
                return error(ErrorCode::InvalidParams);
            };
            self.bots_enter_lab(code);
            return if self.start_lab_node(LOCAL, code) {
                ok()
            } else {
                error(ErrorCode::InvalidParams)
            };
        }
        if bool_field(m, "demo") == Some(true) {
            let id = self.bot_group.id;
            if self.world.group_of(LOCAL, &id).is_none() {
                let url = self.bot_group.url.clone();
                let r = self.world.command(
                    LOCAL,
                    UiCommand::JoinGroup {
                        url,
                        display_name: "Me".into(),
                    },
                );
                if !r.is_ok() && r.error_code() != Some(ErrorCode::AlreadyMember) {
                    return serde_json::to_value(r).unwrap_or_else(|_| ok());
                }
            }
            if self.home != BotHome::Group(id) {
                self.bots_go_home();
            }
            return if self.start_group_node(LOCAL, id) {
                ok()
            } else {
                error(ErrorCode::UnknownGroup)
            };
        }
        let Some(hex) = m.get("groupId").and_then(Value::as_str) else {
            return error(ErrorCode::InvalidParams);
        };
        let Some(id) = store::parse_hex(hex) else {
            return error(ErrorCode::UnknownGroup);
        };
        if matches!(self.home, BotHome::Lab(_)) {
            self.bots_go_home();
        }
        if self.start_group_node(LOCAL, id) {
            ok()
        } else {
            error(ErrorCode::UnknownGroup)
        }
    }

    /// `stopEngine()`: the local phone leaves to Idle; bots that followed it into Lab
    /// mode return to their ride.
    pub fn stop_engine(&mut self) {
        if self.local_ptt {
            self.ptt(false);
        }
        self.local_speak = false;
        self.world.stop_node(LOCAL);
        self.sync_local_mic();
        if matches!(self.home, BotHome::Lab(_)) {
            self.bots_go_home();
        }
    }

    /// The invite `scanInviteQr` returns: the ride the bots are in.
    pub fn sim_invite(&self) -> Option<String> {
        match self.home {
            BotHome::Group(id) => self
                .bots
                .iter()
                .find_map(|b| self.world.invite_url(b.node, &id))
                .or_else(|| Some(self.bot_group.url.clone())),
            BotHome::Lab(_) => Some(self.bot_group.url.clone()),
        }
    }

    fn info(&self) -> Value {
        let group = match self.home {
            BotHome::Group(id) => Some(store::hex(&id)),
            BotHome::Lab(_) => None,
        };
        let bots: Vec<Value> = self
            .bots
            .iter()
            .map(|b| {
                let node_id = match self.home {
                    BotHome::Group(id) => self.world.node_id(b.node, &id),
                    BotHome::Lab(_) => self.world.nodes[b.node].lab_node_id(),
                };
                json!({
                    "index": b.node,
                    "name": mesh_sim::ride::rider_name(b.node),
                    "nodeId": node_id,
                    "live": b.live,
                    "talking": b.talking_until.is_some(),
                    "away": b.away,
                })
            })
            .collect();
        json!({
            "ok": true,
            "preset": self.cfg.preset.name(),
            "nodes": self.cfg.nodes,
            "seed": self.cfg.seed,
            "nowUs": self.now_us(),
            "botsGroupId": group,
            "botsLab": matches!(self.home, BotHome::Lab(_)),
            "botsTalk": self.cfg.bots_talk,
            "roam": self.cfg.roam,
            "bots": bots,
            "droppedEvents": self.dropped_events(),
        })
    }

    /// `sim.botTalk {bot, ms}`: world node `bot` presses PTT and speaks for `ms`.
    fn bot_talk_cmd(&mut self, m: &Map<String, Value>) -> Value {
        let bot = m.get("bot").and_then(Value::as_u64);
        let ms = m.get("ms").and_then(Value::as_u64);
        let (Some(bot), Some(ms)) = (bot, ms) else {
            return error(ErrorCode::InvalidParams);
        };
        if ms == 0 || ms > MAX_BOT_TALK_MS {
            return error(ErrorCode::InvalidParams);
        }
        let Some(k) = self.bots.iter().position(|b| b.node as u64 == bot) else {
            return error(ErrorCode::UnknownMember);
        };
        if !self.bots[k].live {
            return error(ErrorCode::NotLive);
        }
        if self.bot_talk(k, ms) {
            ok()
        } else {
            error(ErrorCode::WrongMode)
        }
    }
}
