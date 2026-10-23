//! `control.json` (ROSTER, JOIN_REQ, REKEY logical bodies and their signed messages,
//! DESIGN §4.7) and the signed mesh payloads LEAVE / RIDE_CTL / GROUP_CTL (§4.6), whose
//! packets go into `mesh.json`.

use serde_json::json;

use super::{VectorFile, hx, pattern_array};
use crate::error::Result;
use crate::mesh::{
    GroupCtl, GroupCtlOp, JoinReq, Leave, LeaveReason, MeshHeader, MeshPayload, Rekey, RekeyEntry,
    RideCtl, RideCtlOp, Roster, RosterEntry, RosterRole, encode_packet,
};

const NAMES: [&str; 4] = ["Ana", "Mónica", "Ζωή", "Juan Carlos de la Fuente Ruiz"];

/// A roster with `n` entries: entry 0 is the leader, entry 1 is denied (when n > 2).
pub(super) fn sample_roster(group_id: &[u8; 16], n: usize) -> Roster {
    let entries = (0..n)
        .map(|i| RosterEntry {
            node_id: 0x0100 + i as u16,
            id_hash: pattern_array(i as u8),
            ed_pk: pattern_array(0x80 ^ i as u8),
            role: match i {
                0 => RosterRole::Leader,
                1 if n > 2 => RosterRole::Denied,
                _ => RosterRole::Member,
            },
            name: format!("{} {i}", NAMES[i % NAMES.len()]),
        })
        .collect();
    Roster {
        group_id: *group_id,
        epoch: 3,
        roster_version: 12,
        invite_sig: pattern_array(0x51),
        entries,
        sig: pattern_array(0x52),
    }
}

pub(super) fn sample_join_req() -> JoinReq {
    JoinReq {
        ed_pk: pattern_array(0x61),
        x_pk: pattern_array(0x62),
        platform: 1,
        name: "Mónica 🏍️".to_owned(),
        sig: pattern_array(0x63),
    }
}

pub(super) fn sample_rekey(n: usize) -> Rekey {
    Rekey {
        new_epoch: 4,
        roster_version: 13,
        leader_eph_pk: pattern_array(0x71),
        entries: (0..n)
            .map(|i| RekeyEntry {
                node_id: 0x0101 + i as u16,
                sealed: pattern_array(0x90 ^ i as u8),
            })
            .collect(),
        sig: pattern_array(0x72),
    }
}

pub(super) fn file() -> Result<VectorFile> {
    let mut f = VectorFile::new(
        "control.json",
        "Control bodies (DESIGN §4.7), logical form before segmentation. ROSTER: ver=1 || \
         group_id[16] || epoch u32 || roster_version u32 || invite_sig[64] || n u8 || n x \
         (node_id u16 || id_hash[16] || ed_pk[32] || role u8 || name_len u8 || name) || sig[64]. \
         JOIN_REQ: ver=1 || ed_pk[32] || x_pk[32] || platform u8 || name_len u8 || name || \
         sig[64]. REKEY: new_epoch u32 || roster_version u32 || leader_eph_pk[32] || n u8 || n x \
         (node_id u16 || sealed[48]) || sig[64]. `*/signed-message` = the exact bytes signed. \
         Signatures and sealed boxes are filler.",
    );
    let gid: [u8; 16] = pattern_array(0x10);

    let roster = sample_roster(&gid, 4);
    f.push(
        "roster/4-entries-leader-denied",
        json!({ "group_id": hx(&gid), "epoch": 3, "roster_version": 12, "n": 4,
                "roles": [1, 255, 0, 0], "names": roster.entries.iter().map(|e| e.name.clone()).collect::<Vec<_>>(),
                "note": "entry 3's name is truncated to 24 B at a grapheme boundary" }),
        &roster.encode()?,
    );
    f.push(
        "roster/signed-message",
        json!({ "of": "roster/4-entries-leader-denied", "rule": "\"cvic/v1/roster\" || preceding bytes" }),
        &roster.signed_message()?,
    );

    let join = sample_join_req();
    f.push(
        "join_req/android",
        json!({ "ed_pk": hx(&join.ed_pk), "x_pk": hx(&join.x_pk), "platform": 1, "name": join.name }),
        &join.encode(),
    );
    f.push(
        "join_req/signed-message",
        json!({ "group_id": hx(&gid), "rule": "\"cvic/v1/join\" || group_id || preceding bytes" }),
        &join.signed_message(&gid),
    );

    let rekey = sample_rekey(3);
    f.push(
        "rekey/3-entries",
        json!({ "new_epoch": 4, "roster_version": 13, "leader_eph_pk": hx(&rekey.leader_eph_pk), "n": 3 }),
        &rekey.encode()?,
    );
    f.push(
        "rekey/signed-message",
        json!({ "group_id": hx(&gid), "rule": "\"cvic/v1/rekey\" || group_id || preceding bytes" }),
        &rekey.signed_message(&gid)?,
    );
    Ok(f)
}

/// LEAVE, RIDE_CTL and GROUP_CTL packets and their signed messages.
pub(super) fn push_signed_packets(f: &mut VectorFile, base: &MeshHeader) -> Result<()> {
    let gid: [u8; 16] = pattern_array(0x10);

    let leave = Leave {
        reason: LeaveReason::User,
        sig: pattern_array(0xE1),
    };
    f.push(
        "leave/user",
        json!({ "reason": 1, "sig": hx(&leave.sig) }),
        &encode_packet(base, &MeshPayload::Leave(leave))?,
    );
    f.push(
        "leave/signed-message",
        json!({ "group_id": hx(&gid), "origin": base.origin, "sess": base.sess, "reason": 1,
                "rule": "\"cvic/v1/leave\" || group_id || origin u16 || sess u8 || reason u8" }),
        &Leave::signed_message(&gid, base.origin, base.sess, LeaveReason::User),
    );

    let ride = RideCtl {
        op: RideCtlOp::Start,
        ride_id: 0x1234_5678,
        sig: pattern_array(0xE2),
    };
    f.push(
        "ride_ctl/start",
        json!({ "op": 1, "ride_id": 0x1234_5678u32, "sig": hx(&ride.sig) }),
        &encode_packet(base, &MeshPayload::RideCtl(ride))?,
    );
    f.push(
        "ride_ctl/signed-message",
        json!({ "group_id": hx(&gid), "epoch": 3, "ride_id": 0x1234_5678u32, "op": 1,
                "rule": "\"cvic/v1/ridectl\" || group_id || epoch u32 || ride_id u32 || op u8" }),
        &RideCtl::signed_message(&gid, 3, 0x1234_5678, RideCtlOp::Start),
    );

    let close = GroupCtl {
        op: GroupCtlOp::Close,
        ctl_version: 2,
        epoch: 3,
        new_leader: None,
        sig: pattern_array(0xE3),
    };
    f.push(
        "group_ctl/close-73",
        json!({ "op": 1, "ctl_version": 2, "epoch": 3, "sig": hx(&close.sig) }),
        &encode_packet(base, &MeshPayload::GroupCtl(close))?,
    );
    f.push(
        "group_ctl/close/signed-message",
        json!({ "group_id": hx(&gid), "rule": "\"cvic/v1/groupctl\" || group_id || preceding bytes" }),
        &close.signed_message(&gid)?,
    );
    let pk: [u8; 32] = pattern_array(0xE4);
    let transfer = GroupCtl {
        op: GroupCtlOp::TransferLeader,
        ctl_version: 3,
        epoch: 3,
        new_leader: Some((pk, 0x0103)),
        sig: pattern_array(0xE5),
    };
    f.push(
        "group_ctl/transfer-leader-107",
        json!({ "op": 2, "ctl_version": 3, "epoch": 3, "new_leader_ed_pk": hx(&pk),
                "new_leader_node": 0x0103, "sig": hx(&transfer.sig) }),
        &encode_packet(base, &MeshPayload::GroupCtl(transfer))?,
    );
    f.push(
        "group_ctl/transfer-leader/signed-message",
        json!({ "group_id": hx(&gid), "rule": "\"cvic/v1/groupctl\" || group_id || preceding bytes" }),
        &transfer.signed_message(&gid)?,
    );
    Ok(())
}
