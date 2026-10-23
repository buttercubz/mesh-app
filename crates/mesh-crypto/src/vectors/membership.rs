//! `membership.json`: signed control bodies with fixed keys.

use mesh_wire::mesh::{LeaveReason, RideCtlOp, RosterRole};
use serde_json::json;

use super::{VectorFile, fixture, hx, pattern};
use crate::error::Result;
use crate::group_ctl;
use crate::identity::PublicIdentity;
use crate::invite::{InvitePolicy, sign_invite};
use crate::join;
use crate::prim::Secret32;
use crate::rekey::{self, RekeyRecipient};
use crate::roster::{self, entry_for_identity};
use crate::signed::{SignedLeave, SignedRideCtl};

pub(super) fn file() -> Result<VectorFile> {
    let f = fixture();
    let mut v = VectorFile::new(
        "membership.json",
        "Signed control bodies (DESIGN §4.6, §4.7, §8.6) with the identities of identity.json: ROSTER signed over \"cvic/v1/roster\" ‖ preceding bytes; JOIN_REQ over \"cvic/v1/join\" ‖ group_id ‖ preceding; REKEY sealed = ChaCha20-Poly1305(k_i, nonce 0, empty AAD, epoch_secret), k_i = HKDF-SHA256(salt = empty, IKM = X25519(leader_eph, x_pk_i), info = \"cvic/v1/rekey\" ‖ u32 new_epoch ‖ u16 node_id, 32), signed over \"cvic/v1/rekey\" ‖ group_id ‖ preceding; GROUP_CTL over \"cvic/v1/groupctl\" ‖ group_id ‖ preceding; LEAVE over \"cvic/v1/leave\" ‖ group_id ‖ origin ‖ sess ‖ reason; RIDE_CTL over \"cvic/v1/ridectl\" ‖ group_id ‖ epoch ‖ ride_id ‖ op.",
    );
    let gid = f.group_id;
    let secret = Secret32::new(f.epoch_secret);

    // JOIN_REQ from the member.
    let j = join::create(&f.member, &gid, 0, "Ana");
    v.push(
        "join_req",
        json!({ "ed_seed": hx(&f.seeds[1].0), "x_sk": hx(&f.seeds[1].1), "platform": 0, "name": "Ana", "group_id": hx(&gid) }),
        json!({ "signed_message": hx(&j.signed_message(&gid)), "body": hx(&j.encode()) }),
    );

    // ROSTER: leader, two members and one denied identity.
    let denied = PublicIdentity {
        ed_pk: pattern(0x90),
        x_pk: pattern(0x91),
    };
    let isig = sign_invite(
        &f.leader,
        &gid,
        f.epoch,
        InvitePolicy::DEFAULT,
        "Sunday ride",
        &secret,
    );
    let r = roster::build(
        &f.leader,
        gid,
        f.epoch,
        3,
        isig,
        vec![
            entry_for_identity(f.leader.public(), RosterRole::Leader, "Lía"),
            entry_for_identity(f.member.public(), RosterRole::Member, "Ana"),
            entry_for_identity(f.member2.public(), RosterRole::Member, "Bruno"),
            entry_for_identity(&denied, RosterRole::Denied, "Eve"),
        ],
    )?;
    v.push(
        "roster",
        json!({
            "group_id": hx(&gid),
            "epoch": f.epoch,
            "roster_version": 3,
            "invite": "invite.json/default",
            "entries": ["leader (Lía, leader)", "member (Ana)", "member2 (Bruno)", "denied identity ed_pk=pattern(0x90), x_pk=pattern(0x91) (Eve)"],
        }),
        json!({ "invite_sig": hx(&isig.0), "signed_message": hx(&r.signed_message()?), "body": hx(&r.encode()?) }),
    );

    // REKEY to epoch 8 sealed to both members (the denied identity is left out).
    let new_secret = Secret32::new(pattern(0x21));
    let eph = Secret32::new(pattern(0x70));
    let recipients = [&f.member, &f.member2].map(|m| RekeyRecipient {
        node_id: m.node_id(),
        x_pk: m.x_pk(),
    });
    let rk = rekey::seal_with_ephemeral(&f.leader, &gid, 8, 4, &new_secret, &recipients, &eph)?;
    let k0 = rekey::entry_key(&f.member.dh(&rk.leader_eph_pk)?, 8, f.member.node_id());
    v.push(
        "rekey",
        json!({
            "group_id": hx(&gid),
            "new_epoch": 8,
            "roster_version": 4,
            "new_epoch_secret": hx(new_secret.expose()),
            "leader_eph_sk": hx(eph.expose()),
            "recipients": ["member", "member2"],
        }),
        json!({
            "k_member": hx(k0.expose()),
            "signed_message": hx(&rk.signed_message(&gid)?),
            "body": hx(&rk.encode()?),
        }),
    );

    // GROUP_CTL close and transfer-leader.
    let close = group_ctl::close(&f.leader, &gid, 1, 8)?;
    v.push(
        "group_ctl/close",
        json!({ "group_id": hx(&gid), "ctl_version": 1, "epoch": 8 }),
        json!({ "signed_message": hx(&close.signed_message(&gid)?), "payload": hx(&close.encode()?) }),
    );
    let t =
        group_ctl::transfer_leader(&f.leader, &gid, 2, 8, f.member.ed_pk(), f.member.node_id())?;
    v.push(
        "group_ctl/transfer-leader",
        json!({ "group_id": hx(&gid), "ctl_version": 2, "epoch": 8, "new_leader": "member" }),
        json!({ "signed_message": hx(&t.signed_message(&gid)?), "payload": hx(&t.encode()?) }),
    );

    // Signed LEAVE and RIDE_CTL.
    let l = SignedLeave::sign(&f.member, &gid, 0x5C, LeaveReason::User);
    v.push(
        "leave",
        json!({ "signer": "member", "origin": l.origin, "sess": 0x5C, "reason": 1 }),
        json!({
            "signed_message": hx(&mesh_wire::mesh::Leave::signed_message(&gid, l.origin, 0x5C, LeaveReason::User)),
            "payload": hx(&l.leave.encode()),
        }),
    );
    for (name, op) in [
        ("ride_ctl/start", RideCtlOp::Start),
        ("ride_ctl/end", RideCtlOp::End),
    ] {
        let s = SignedRideCtl::sign(&f.leader, &gid, 8, 0xC0FF_EE01, op);
        v.push(
            name,
            json!({ "signer": "leader", "epoch": 8, "ride_id": 0xC0FF_EE01u32, "op": op as u8 }),
            json!({
                "signed_message": hx(&mesh_wire::mesh::RideCtl::signed_message(&gid, 8, 0xC0FF_EE01, op)),
                "payload": hx(&s.ctl.encode()),
            }),
        );
    }
    Ok(v)
}
