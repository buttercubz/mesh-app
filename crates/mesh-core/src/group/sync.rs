//! Link-scoped anti-entropy (DESIGN §4.4 SYNC, §8.9): the stored signed bodies are
//! pushed to a neighbor that is behind (after its CAPS, or on a PRESENCE mismatch) in
//! the order REKEY → ROSTER → GROUP_CTL, chunked into SYNC records (P2), rate limited to
//! one push per (link, kind) per 10 s. A JOIN_REQ rides the same record (kind 4).
//! Received chunks are reassembled per link and `msg_id` and then processed exactly like
//! the flooded message.

use mesh_wire::mesh::{AssemblyEvent, Segment, sync_chunks};
use mesh_wire::record::{Record, Sync, SyncKind};

use super::active::{Active, PUSH_INTERVAL_US};
use super::control::Control;
use crate::LinkId;
use crate::engine::Ctx;
use crate::links::Prio;

/// Sends `body` as SYNC records of `kind` on `link`.
pub fn send(ctx: &mut Ctx, act: &mut Active, link: LinkId, kind: SyncKind, body: &[u8]) {
    let msg_id = act.next_msg_id();
    let Ok(chunks) = sync_chunks(kind, msg_id, body) else {
        return;
    };
    for c in chunks {
        ctx.links
            .send_record(link, Prio::P2, &Record::Sync(c), None);
    }
}

/// Pushes the stored bodies of `kinds` (already in REKEY → ROSTER → GROUP_CTL order)
/// that are not rate limited on `link`.
pub fn push(ctx: &mut Ctx, act: &mut Active, control: &Control, link: LinkId, kinds: &[SyncKind]) {
    let now = ctx.now;
    for &k in kinds {
        let Some(body) = control.body(k).map(<[u8]>::to_vec) else {
            continue;
        };
        let Some(l) = act.links.get_mut(&link) else {
            return;
        };
        if l.pushed
            .get(&k)
            .is_some_and(|t| now.since(*t) < PUSH_INTERVAL_US)
        {
            continue;
        }
        l.pushed.insert(k, now);
        send(ctx, act, link, k, &body);
    }
}

/// The peer caught up for `kinds` (its PRESENCE shows it): a later push is allowed at
/// once (§8.9 rule 3, "cancelled if the neighbor caught up").
pub fn caught_up(act: &mut Active, link: LinkId, kinds: &[SyncKind]) {
    if let Some(l) = act.links.get_mut(&link) {
        for k in [SyncKind::Rekey, SyncKind::Roster, SyncKind::GroupCtl] {
            if !kinds.contains(&k) {
                l.pushed.remove(&k);
            }
        }
    }
}

/// A SYNC record from `link`: returns the complete body when the last chunk arrived.
pub fn ingest(
    ctx: &Ctx,
    act: &mut Active,
    link: LinkId,
    s: &Sync<'_>,
) -> Option<(SyncKind, Vec<u8>)> {
    let l = act.links.get_mut(&link)?;
    let seg = Segment {
        msg_id: s.msg_id,
        idx: s.idx,
        count: s.count,
        data: s.chunk,
    };
    match l.sync_asm.push(ctx.now.0, (), &seg).ok()? {
        AssemblyEvent::Complete(body) => Some((s.kind, body)),
        _ => None,
    }
}
