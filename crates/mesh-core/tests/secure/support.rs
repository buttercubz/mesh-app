//! Group-session helpers over the in-memory world of `tests/links/harness.rs`, shared by
//! the `secure`, `group` and `session` test targets through `#[path]`.

#![allow(dead_code)]

use mesh_api::{CommandResult, NoticeKey, PromptKey, UiCommand, UiEvent, Vehicle};
use mesh_core::effects::{AudioCommand, Effect};
use mesh_core::group::state::Group;
use mesh_core::group::store::{self, Stored};
use mesh_core::links::LinkState;
use mesh_core::media::{Cue, MediaRequest};
use mesh_core::{Engine, EngineConfig, Instant, LinkId, Platform, SessionStart, StoredGroup};

use crate::harness::{Node, World, seed};

/// Hex group id of a `GroupId` result.
pub fn gid(r: &CommandResult) -> String {
    match r {
        CommandResult::GroupId(g) => g.group_id.clone(),
        other => panic!("expected a group id, got {other:?}"),
    }
}

pub fn id_bytes(hex: &str) -> [u8; 16] {
    store::parse_hex(hex).expect("hex group id")
}

/// Creates a group on node `i` with display name `name`.
pub fn create(w: &mut World, i: usize, name: &str) -> String {
    assert!(
        w.command(i, UiCommand::SetDisplayName { name: name.into() })
            .is_ok()
    );
    gid(&w.command(
        i,
        UiCommand::CreateGroup {
            name: "Sunday ride".into(),
            vehicle: Vehicle::Moto,
        },
    ))
}

/// Latest persisted blob of group `hex` on node `i`.
pub fn blob(w: &World, i: usize, hex: &str) -> Option<Vec<u8>> {
    let key = format!("group/{hex}");
    w.nodes[i].effects.iter().rev().find_map(|e| match e {
        Effect::Persist { key: k, value } if *k == key => Some(value.clone()),
        _ => None,
    })?
}

/// The stored group `hex` of node `i`, decoded.
pub fn group(w: &World, i: usize, hex: &str) -> Group {
    match store::decode(&blob(w, i, hex).expect("persisted group")).expect("decodes") {
        Stored::Live(g) => *g,
        Stored::Closed(_) => panic!("group is closed"),
    }
}

/// The invite URL node `i` would show for `hex`.
pub fn invite_url(w: &World, i: usize, hex: &str) -> String {
    mesh_core::group::invite_url_of_blob(&blob(w, i, hex).expect("persisted group"))
        .expect("invite available")
}

/// Joins `url` on node `i` as `name`.
pub fn join(w: &mut World, i: usize, url: &str, name: &str) -> String {
    gid(&w.command(
        i,
        UiCommand::JoinGroup {
            url: url.into(),
            display_name: name.into(),
        },
    ))
}

/// Starts the group session `hex` on node `i`.
pub fn start(w: &mut World, i: usize, hex: &str) {
    let now = w.now;
    w.nodes[i]
        .engine
        .start(
            now,
            SessionStart::Group {
                group_id: id_bytes(hex),
            },
        )
        .expect("group start");
    w.collect(i);
}

/// Stops node `i` (back to Idle).
pub fn stop(w: &mut World, i: usize) {
    let now = w.now;
    w.nodes[i].engine.stop(now);
    w.collect(i);
}

/// Peer node ids of the Up links of node `i`.
pub fn up_peers(w: &World, i: usize) -> Vec<u16> {
    let mut v: Vec<u16> = w.nodes[i]
        .engine
        .ctx()
        .links
        .iter()
        .filter(|l| l.state == LinkState::Up)
        .filter_map(|l| l.peer_node.map(|n| n.0))
        .collect();
    v.sort_unstable();
    v
}

/// Up link ids of node `i`.
pub fn up_links(w: &World, i: usize) -> Vec<LinkId> {
    w.nodes[i]
        .engine
        .ctx()
        .links
        .iter()
        .filter(|l| l.state == LinkState::Up)
        .map(|l| l.id)
        .collect()
}

/// `node_id` of node `i` in group `hex` (from its stored identity).
pub fn node_id(w: &World, i: usize, hex: &str) -> u16 {
    group(w, i, hex).my_node_id()
}

/// Non-denied roster node ids of node `i`'s stored group.
pub fn roster(w: &World, i: usize, hex: &str) -> Vec<u16> {
    let mut v: Vec<u16> = group(w, i, hex)
        .roster_indices()
        .into_iter()
        .map(|(n, _)| n)
        .collect();
    v.sort_unstable();
    v
}

/// Notice keys emitted by node `i`.
pub fn notices(w: &World, i: usize) -> Vec<NoticeKey> {
    w.ui_events(i)
        .into_iter()
        .filter_map(|e| match e {
            UiEvent::Notice { key, .. } => Some(*key),
            _ => None,
        })
        .collect()
}

/// Requests node `i` posted to its media module (inter-module bus).
pub fn media_requests(w: &World, i: usize) -> Vec<MediaRequest> {
    w.nodes[i].bus.borrow().clone()
}

/// Cues node `i` asked its media module to play.
pub fn cues(w: &World, i: usize) -> Vec<Cue> {
    media_requests(w, i)
        .into_iter()
        .filter_map(|r| match r {
            MediaRequest::Cue(c) => Some(c),
            _ => None,
        })
        .collect()
}

/// Prompt keys of the cues of node `i`.
pub fn cue_keys(w: &World, i: usize) -> Vec<PromptKey> {
    cues(w, i).into_iter().filter_map(|c| c.prompt).collect()
}

/// Audio commands of node `i`.
pub fn audio(w: &World, i: usize) -> Vec<AudioCommand> {
    w.nodes[i]
        .effects
        .iter()
        .filter_map(|e| match e {
            Effect::Audio(c) => Some(*c),
            _ => None,
        })
        .collect()
}

/// The latest snapshot of node `i`.
pub fn snapshot(w: &World, i: usize) -> Option<mesh_api::Snapshot> {
    w.ui_events(i).into_iter().rev().find_map(|e| match e {
        UiEvent::Snapshot { s } => Some(s.clone()),
        _ => None,
    })
}

/// Replaces node `i` with a fresh engine on `platform` that starts from node `i`'s
/// persisted groups (a restart, or a different OS for the role rules).
pub fn reboot(w: &mut World, i: usize, platform: Platform, salt: u8) {
    let mut cfg = EngineConfig::new(platform, seed(i, salt));
    cfg.wall_ms = 1_792_746_723_120;
    let mut groups: Vec<StoredGroup> = Vec::new();
    for e in &w.nodes[i].effects {
        if let Effect::Persist { key, value } = e
            && let Some(hex) = key.strip_prefix("group/")
            && let Some(id) = store::parse_hex(hex)
        {
            groups.retain(|g| g.group_id != id);
            if let Some(b) = value {
                groups.push(StoredGroup {
                    group_id: id,
                    blob: b.clone(),
                });
            }
        }
    }
    cfg.stored_groups = groups;
    let effects = std::mem::take(&mut w.nodes[i].effects);
    let bus = std::mem::take(&mut *w.nodes[i].bus.borrow_mut());
    w.nodes[i] = Node::new(Engine::new(cfg, w.now), platform);
    w.nodes[i].effects = effects;
    *w.nodes[i].bus.borrow_mut() = bus;
}

/// Two nodes: 0 creates ("Ana", leader), 1 joins ("Ben"); both sessions started and
/// linked (0 initiates), run until the roster lists both.
pub fn pair(salt: u8) -> (World, String) {
    let mut w = World::new(2, Platform::Ios, salt);
    let g = create(&mut w, 0, "Ana");
    let url = invite_url(&w, 0, &g);
    join(&mut w, 1, &url, "Ben");
    start(&mut w, 0, &g);
    start(&mut w, 1, &g);
    w.connect(0, 1);
    w.run_ms(3_000);
    (w, g)
}

/// `true` once the engine at `t` is past `Instant`.
pub fn at(ms: u64) -> Instant {
    Instant::from_ms(ms)
}
