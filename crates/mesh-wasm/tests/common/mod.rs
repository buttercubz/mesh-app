//! Helpers shared by the mesh-wasm integration tests: JSON commands, stepping in
//! 20 ms page ticks, and reading the local phone's latest snapshot.

#![allow(dead_code)]

use mesh_wasm::{ConvoyConfig, SimConvoy};
use serde_json::{Value, json};

/// The page's tick (§11.2 `setInterval(20 ms)`).
pub const TICK_US: u64 = 20_000;

/// A convoy with its event log.
pub struct Page {
    pub c: SimConvoy,
    pub events: Vec<Value>,
}

impl Page {
    pub fn new(cfg: &str) -> Page {
        Page {
            c: SimConvoy::new(ConvoyConfig::parse(cfg).expect("scenario")),
            events: Vec::new(),
        }
    }

    /// Runs a command and returns its parsed result.
    pub fn cmd(&mut self, v: Value) -> Value {
        let r = self.c.command_json(&v.to_string());
        self.pump();
        serde_json::from_str(&r).expect("result JSON")
    }

    /// Runs a command that must succeed.
    pub fn ok(&mut self, v: Value) -> Value {
        let r = self.cmd(v.clone());
        assert_eq!(r["ok"], json!(true), "{v} refused: {r}");
        r
    }

    fn pump(&mut self) {
        for e in self.c.poll_events() {
            self.events
                .push(serde_json::from_str(&e).expect("event JSON"));
        }
    }

    /// Advances `ms` of simulated time in page ticks.
    pub fn run_ms(&mut self, ms: u64) {
        for _ in 0..(ms * 1_000).div_ceil(TICK_US) {
            self.c.step(TICK_US);
            self.pump();
        }
    }

    /// Runs (in ticks) until `pred` holds on the latest snapshot or `limit_ms` passed.
    pub fn run_until(&mut self, limit_ms: u64, mut pred: impl FnMut(&Value) -> bool) -> bool {
        let ticks = (limit_ms * 1_000).div_ceil(TICK_US);
        for _ in 0..ticks {
            if self.snapshot().is_some_and(&mut pred) {
                return true;
            }
            self.c.step(TICK_US);
            self.pump();
        }
        self.snapshot().is_some_and(pred)
    }

    /// The latest snapshot's `s`.
    pub fn snapshot(&self) -> Option<&Value> {
        self.events
            .iter()
            .rev()
            .find(|e| e["type"] == "snapshot")
            .map(|e| &e["s"])
    }

    /// Node ids of the latest snapshot's talkers (sorted).
    pub fn talkers(&self) -> Vec<u64> {
        let mut v: Vec<u64> = self
            .snapshot()
            .and_then(|s| s["talkers"].as_array())
            .map(|a| a.iter().filter_map(|t| t["nodeId"].as_u64()).collect())
            .unwrap_or_default();
        v.sort_unstable();
        v
    }

    /// `session.talk` of the latest snapshot.
    pub fn talk_state(&self) -> Option<String> {
        self.snapshot()
            .and_then(|s| s["session"]["talk"].as_str())
            .map(str::to_string)
    }

    /// Bot node ids (`sim.info`).
    pub fn bot_ids(&mut self) -> Vec<u64> {
        let info = self.ok(json!({"type": "sim.info"}));
        info["bots"]
            .as_array()
            .expect("bots")
            .iter()
            .map(|b| b["nodeId"].as_u64().expect("bot nodeId"))
            .collect()
    }

    /// Joins the bots' ride from the sim invite, starts the engine, goes live in PTT
    /// and starts riding; returns the group id.
    pub fn join_bots_ride(&mut self) -> String {
        let url = self.ok(json!({"type": "sim.invite"}))["url"]
            .as_str()
            .expect("url")
            .to_string();
        let j = self.ok(json!({"type": "joinGroup", "url": url, "displayName": "Me"}));
        let gid = j["groupId"].as_str().expect("groupId").to_string();
        self.ok(json!({"type": "sim.startEngine", "groupId": gid}));
        gid
    }

    /// Goes live in PTT and starts riding (a member's start is local).
    pub fn go_live_ptt(&mut self) {
        self.ok(json!({"type": "goLive"}));
        self.ok(json!({"type": "setTalkMode", "mode": "ptt"}));
        self.ok(json!({"type": "rideControl", "op": "start"}));
    }

    /// Events of `type` seen so far.
    pub fn of_type(&self, t: &str) -> Vec<&Value> {
        self.events.iter().filter(|e| e["type"] == t).collect()
    }
}
