//! `identity.json`, `schedule.json` and `invite.json`.

use serde_json::json;

use super::{Fixture, VectorFile, fixture, hx, pattern};
use crate::error::Result;
use crate::invite::{Invite, InvitePolicy, sign_invite};
use crate::prim::Secret32;
use crate::schedule::{KeySchedule, lab_tag, t10};

pub(super) fn identity_file() -> VectorFile {
    let f = fixture();
    let mut v = VectorFile::new(
        "identity.json",
        "Per-group identities (DESIGN §8.2): Ed25519 public key of `ed_seed`, X25519 public key of `x_sk`, id_hash = SHA-256(ed_pk ‖ x_pk)[0..16], node_id = u16 LE of id_hash[0..2].",
    );
    let ids = [
        ("leader", &f.leader),
        ("member", &f.member),
        ("member2", &f.member2),
    ];
    for (i, (name, id)) in ids.iter().enumerate() {
        let (ed_seed, x_sk) = f.seeds[i];
        v.push(
            name,
            json!({ "ed_seed": hx(&ed_seed), "x_sk": hx(&x_sk) }),
            json!({
                "ed_pk": hx(&id.ed_pk()),
                "x_pk": hx(&id.x_pk()),
                "id_hash": hx(&id.id_hash()),
                "node_id": id.node_id(),
            }),
        );
    }
    v
}

pub(super) fn schedule_file() -> VectorFile {
    let f = fixture();
    let mut v = VectorFile::new(
        "schedule.json",
        "HKDF-SHA256 key schedule (DESIGN §8.3): PRK = Extract(salt = group_id, IKM = epoch_secret); psk = Expand(PRK, \"cvic/v1/noise-psk\", 32); disc_key = Expand(PRK, \"cvic/v1/discovery\", 32); group_tag = HMAC(disc_key, \"gt\" ‖ u32 t10)[0..4]; node_token = HMAC(disc_key, \"nt\" ‖ u32 t10 ‖ id_hash)[0..4]; aware_pmk = Expand(PRK, \"cvic/v1/aware-pmk\" ‖ handshake_hash, 32); tunnel_rdv = Expand(PRK, \"cvic/v1/tunnel-rendezvous\", 16); lab_tag = SHA-256(\"cvic/lab\" ‖ u16 lab_code)[0..2] (u16 LE).",
    );
    let s = KeySchedule::new(f.group_id, f.epoch, Secret32::new(f.epoch_secret));
    let unix = 1_761_000_123u64;
    let t = t10(unix);
    let hh: [u8; 32] = pattern(0x60);
    v.push(
        "epoch",
        json!({
            "group_id": hx(&f.group_id),
            "epoch": f.epoch,
            "epoch_secret": hx(&f.epoch_secret),
        }),
        json!({
            "prk": hx(s.prk().expose()),
            "psk": hx(s.psk().expose()),
            "disc_key": hx(s.disc_key().expose()),
            "tunnel_rdv": hx(&s.tunnel_rdv()),
        }),
    );
    v.push(
        "discovery",
        json!({
            "unix_seconds": unix,
            "id_hash": hx(&f.member.id_hash()),
        }),
        json!({
            "t10": t,
            "group_tag": hx(&s.group_tag(t)),
            "group_tag_prev": hx(&s.group_tag(t - 1)),
            "group_tag_next": hx(&s.group_tag(t + 1)),
            "node_token": hx(&s.node_token(t, &f.member.id_hash())),
        }),
    );
    v.push(
        "aware_pmk",
        json!({ "ble_handshake_hash": hx(&hh) }),
        json!({ "aware_pmk": hx(s.aware_pmk(&hh).expose()) }),
    );
    for code in [0u16, 1234, 9999] {
        v.push(
            &format!("lab_tag/{code}"),
            json!({ "lab_code": code }),
            json!({ "lab_tag": lab_tag(code) }),
        );
    }
    v
}

pub(super) fn invite_file() -> Result<VectorFile> {
    let f: Fixture = fixture();
    let mut v = VectorFile::new(
        "invite.json",
        "Invite v1 (DESIGN §8.3): ver · flags · group_id · epoch · epoch_secret · leader_ed_pk · policy · [expires_at] · name_len · name · invite_sig; invite_sig = Ed25519(leader, \"cvic/v1/invite\" ‖ group_id ‖ epoch ‖ policy ‖ name_len ‖ name ‖ leader_ed_pk ‖ SHA-256(epoch_secret)); URL convlinkvoice://join?v=1#<base64url, unpadded>.",
    );
    let secret = Secret32::new(f.epoch_secret);
    let cases: [(&str, InvitePolicy, &str, bool, Option<u32>); 2] = [
        ("default", InvitePolicy::DEFAULT, "Sunday ride", false, None),
        (
            "expiry-aware-utf8",
            InvitePolicy::new(6, true, false),
            "Rodada \u{1F3CD}\u{FE0F} Sierra Madre Oriental",
            true,
            Some(29_350_000),
        ),
    ];
    for (name, policy, group_name, aware, expires) in cases {
        let sig = sign_invite(&f.leader, &f.group_id, f.epoch, policy, group_name, &secret);
        let inv = Invite::assemble(
            f.group_id,
            f.epoch,
            secret.clone(),
            f.leader.ed_pk(),
            policy,
            group_name,
            sig,
            aware,
            expires,
        );
        v.push(
            name,
            json!({
                "group_id": hx(&f.group_id),
                "epoch": f.epoch,
                "epoch_secret": hx(&f.epoch_secret),
                "leader_ed_seed": hx(&f.seeds[0].0),
                "policy": policy.0,
                "name": group_name,
                "aware_pairing": aware,
                "expires_at": expires,
            }),
            json!({
                "signed_message": hx(&inv.signed_message()),
                "invite_sig": hx(&inv.invite_sig.0),
                "invite": hx(&inv.encode()),
                "url": inv.to_url(),
                "name_on_wire": inv.name,
            }),
        );
    }
    Ok(v)
}
