//! Group rides in the world (DESIGN §1.3, §8.3): node 0 (by default) creates the group
//! with the real `createGroup` command, every other rider joins from its invite URL
//! (`joinGroup`), each phone starts the group session (`SessionStart::Group`, the
//! platform publishes its L2CAP PSM) and the real discovery / HELLO / Noise / IDPROOF
//! path forms the links over the radio model. Go live, talk mode, PTT and the scripted
//! talk schedules drive the real media module.

use mesh_api::{CommandResult, MemberStatus, RideOp, TalkMode, UiCommand, Vehicle};
use mesh_core::group::state::Group;
use mesh_core::group::store::{self, Stored};
use mesh_core::{Instant, PlatformEvent, SessionStart, rt_flags};

use crate::world::{World, psm_of};

/// A group formed in the world.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RideGroup {
    pub id: [u8; 16],
    /// The UI `groupId`.
    pub hex: String,
    /// The invite URL the creator handed out when the others joined.
    pub url: String,
    /// The creator (leader at creation).
    pub leader: usize,
}

/// Display name of node `i` in scenarios.
pub fn rider_name(i: usize) -> String {
    format!("Rider{i:02}")
}

impl World {
    /// Node `leader` creates a group; `members` join from its invite (in order). The
    /// group is persisted on each phone; no session is started.
    pub fn form_group(&mut self, leader: usize, members: &[usize]) -> RideGroup {
        let r = self.command(
            leader,
            UiCommand::SetDisplayName {
                name: rider_name(leader),
            },
        );
        assert!(r.is_ok(), "setDisplayName: {r:?}");
        let r = self.command(
            leader,
            UiCommand::CreateGroup {
                name: "Sim ride".into(),
                vehicle: Vehicle::Moto,
            },
        );
        let hex = match r {
            CommandResult::GroupId(g) => g.group_id,
            other => panic!("createGroup refused: {other:?}"),
        };
        let id = store::parse_hex(&hex).expect("group id hex");
        let url = self.invite_url(leader, &id).expect("invite URL");
        for &m in members {
            self.join_group(m, &url);
        }
        RideGroup {
            id,
            hex,
            url,
            leader,
        }
    }

    /// Node `i` joins from `url` with its scenario name.
    pub fn join_group(&mut self, i: usize, url: &str) -> CommandResult {
        let r = self.command(
            i,
            UiCommand::JoinGroup {
                url: url.into(),
                display_name: rider_name(i),
            },
        );
        assert!(
            matches!(r, CommandResult::GroupId(_)),
            "joinGroup on node {i}: {r:?}"
        );
        r
    }

    /// The decoded stored group `id` of node `i`.
    pub fn group_of(&self, i: usize, id: &[u8; 16]) -> Option<Group> {
        let blob = self.nodes[i].store.get(&store::group_key(id))?;
        match store::decode(blob).ok()? {
            Stored::Live(g) => Some(*g),
            Stored::Closed(_) => None,
        }
    }

    /// Sets the group's Wi-Fi Aware pairing flag in node `i`'s stored group (the flag
    /// an Aware-enabled invite carries, §9.6) and restarts the app so the engine loads
    /// it, as the runtime does at app start. Use before the session starts.
    pub fn set_aware_pairing(&mut self, i: usize, id: &[u8; 16], on: bool) {
        let key = store::group_key(id);
        let Some(mut g) = self.group_of(i, id) else {
            return;
        };
        g.aware_pairing = on;
        self.nodes[i].store.insert(key, store::encode(&g));
        self.restart_node(i);
    }

    /// The invite URL node `i` would hand out for group `id`.
    pub fn invite_url(&self, i: usize, id: &[u8; 16]) -> Option<String> {
        let blob = self.nodes[i].store.get(&store::group_key(id))?;
        mesh_core::group::invite_url_of_blob(blob)
    }

    /// Per-group `node_id` of node `i`.
    pub fn node_id(&self, i: usize, id: &[u8; 16]) -> Option<u16> {
        self.group_of(i, id).map(|g| g.my_node_id())
    }

    /// The world node whose `node_id` in group `id` is `node`.
    pub fn node_by_id(&self, id: &[u8; 16], node: u16) -> Option<usize> {
        (0..self.nodes.len()).find(|&i| self.node_id(i, id) == Some(node))
    }

    /// Starts node `i`'s group session (lobby: radios on, audio off).
    pub fn start_group(&mut self, i: usize, id: &[u8; 16]) {
        let now = self.now;
        let n = &mut self.nodes[i];
        n.engine
            .start(now, SessionStart::Group { group_id: *id })
            .expect("group start");
        n.psm = psm_of(i);
        n.engine
            .handle_event(now, PlatformEvent::L2capPublished { psm: n.psm });
        self.dirty.insert(i);
        self.settle();
    }

    /// Starts the group session on every node in `nodes`.
    pub fn start_group_all(&mut self, id: &[u8; 16], nodes: &[usize]) {
        for &i in nodes {
            self.start_group(i, id);
        }
    }

    /// Stops node `i`'s engine (leaves the ride, back to Idle).
    pub fn stop_node(&mut self, i: usize) {
        let now = self.now;
        self.nodes[i].engine.stop(now);
        self.dirty.insert(i);
        self.settle();
    }

    /// The leader starts the ride (`rideControl{start}`).
    pub fn start_ride(&mut self, leader: usize) {
        let r = self.command(leader, UiCommand::RideControl { op: RideOp::Start });
        assert!(r.is_ok(), "rideControl start: {r:?}");
    }

    /// Node `i` goes live in `mode` (the plugin's gesture path, §3.2).
    pub fn go_live(&mut self, i: usize, mode: TalkMode) -> bool {
        let r = self.command(i, UiCommand::GoLive {});
        if !r.is_ok() {
            return false;
        }
        self.command(i, UiCommand::SetTalkMode { mode }).is_ok()
    }

    /// PTT down / up on node `i` (RT flag fast path, §3.6).
    pub fn ptt(&mut self, i: usize, down: bool) {
        let now = self.now;
        let flags = if down {
            self.nodes[i].engine.ctx().rt_flags | rt_flags::PTT_DOWN
        } else {
            self.nodes[i].engine.ctx().rt_flags & !rt_flags::PTT_DOWN
        };
        self.nodes[i].engine.handle_timeout(now);
        self.nodes[i].engine.set_rt_flags(flags);
        self.dirty.insert(i);
        self.settle();
    }

    /// Reachability of member `node` as node `i`'s latest snapshot shows it.
    pub fn member_status(&self, i: usize, node: u16) -> Option<(MemberStatus, u8)> {
        self.nodes[i]
            .snapshot
            .as_ref()?
            .members
            .iter()
            .find(|m| m.node_id == node)
            .map(|m| (m.status, m.hops))
    }

    /// Node ids node `i`'s latest snapshot lists as talking.
    pub fn talkers(&self, i: usize) -> Vec<u16> {
        let mut v: Vec<u16> = self.nodes[i]
            .snapshot
            .as_ref()
            .map(|s| s.talkers.iter().map(|t| t.node_id).collect())
            .unwrap_or_default();
        v.sort_unstable();
        v
    }

    /// Runs until `pred` holds (checked every `step_ms`) or `limit_ms` passed; returns
    /// the time it first held.
    pub fn run_until_pred(
        &mut self,
        limit_ms: u64,
        step_ms: u64,
        mut pred: impl FnMut(&World) -> bool,
    ) -> Option<Instant> {
        let end = self.now.plus_ms(limit_ms);
        loop {
            if pred(self) {
                return Some(self.now);
            }
            if self.now >= end {
                return None;
            }
            self.run_ms(step_ms);
        }
    }
}
