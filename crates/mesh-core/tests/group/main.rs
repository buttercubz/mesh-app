//! Groups (WP-10B, DESIGN §1.3 F1/F2/F9, §8.2-§8.9): create/parse/join/list/forget and
//! the natively rendered invite, the leader-signed invite re-shared by members and the
//! first-ROSTER leader-key cross-check, persistence, JOIN_REQ admission and
//! retransmission, signed roster/rekey/group-control ingest with monotonic acceptance,
//! lock/remove with the retained keyring (strangers with an old QR refused, a member
//! several epochs behind readmitted via a speculative link and brought up to date,
//! denied members refused), anti-entropy convergence, transfer and close.

#[path = "../links/harness.rs"]
mod harness;
#[path = "../secure/support.rs"]
mod support;

mod epochs;
mod inject;
mod invites;
mod leader;
mod open;
mod sync;

use mesh_core::links::LinkState;
use mesh_core::{LinkId, Platform};
use mesh_wire::frame::RejectReason;
use support::*;

/// `n` nodes: node 0 creates ("Ana", leader), every other node joins with the leader's
/// invite; every session started. No links yet.
pub fn members(n: usize, platform: Platform, salt: u8) -> (harness::World, String) {
    let mut w = harness::World::new(n, platform, salt);
    let g = create(&mut w, 0, "Ana");
    let url = invite_url(&w, 0, &g);
    for i in 1..n {
        join(&mut w, i, &url, &format!("R{i}"));
    }
    for i in 0..n {
        start(&mut w, i, &g);
    }
    (w, g)
}

/// The stored group's `(epoch, roster_version, ctl_version)` on node `i`.
pub fn summary(w: &harness::World, i: usize, g: &str) -> (u32, u32, u32) {
    group(w, i, g).control.summary()
}

/// Close reasons node `i` sent (REJECT/CLOSE through `RadioCommand::Close`).
pub fn close_reasons(w: &harness::World, i: usize) -> Vec<RejectReason> {
    w.radio_cmds(i)
        .into_iter()
        .filter_map(|c| match c {
            mesh_core::effects::RadioCommand::Close {
                reason: Some(r), ..
            } => Some(*r),
            _ => None,
        })
        .collect()
}

/// State of node `i`'s link `l`.
pub fn link_state(w: &harness::World, i: usize, l: LinkId) -> Option<LinkState> {
    w.nodes[i].engine.ctx().links.get(l).map(|x| x.state)
}

/// Index of the newest harness link.
pub fn last_link(w: &harness::World) -> usize {
    w.links.len() - 1
}
