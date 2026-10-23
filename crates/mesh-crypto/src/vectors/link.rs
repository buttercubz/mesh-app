//! `link.json`: one complete link with fixed static and ephemeral keys.

use mesh_wire::frame::KpCtr;
use mesh_wire::record::{Record, encode_records};
use rand_chacha::ChaCha20Rng;
use rand_core::SeedableRng;
use serde_json::json;

use super::{VectorFile, fixture, hx, pattern};
use crate::error::{CryptoError, Result};
use crate::keyring::EpochKeyring;
use crate::prim::Secret32;
use crate::schedule::KeySchedule;
use crate::sec::next_key;
use crate::session::LinkSession;

pub(super) fn file() -> Result<VectorFile> {
    let f = fixture();
    let mut v = VectorFile::new(
        "link.json",
        "Per-link Noise_XXpsk1_25519_ChaChaPoly_SHA256 (DESIGN §4.3, §8.4) with fixed static and ephemeral keys: HS LinkFrames (lf_type 0x00 · hs_step · proto_ver · noise_msg), prologue \"cvic/v1\" ‖ group_id ‖ u32 epoch, PSK psk_e at position 1. Raw split: c1 seals initiator→responder, c2 responder→initiator. SEC LinkFrame = 0x01 · kp_ctr u32 LE · ciphertext · tag; nonce 0^4 ‖ u64 LE(kp_ctr); AAD lf_type ‖ kp_ctr. The first SEC record each way is IDPROOF (rtype 0x0C) over \"cvic/v1/idproof\" ‖ role ‖ handshake_hash. Key update: k_next = HKDF-Expand(PRK = k_cur, \"cvic/v1/keyupdate\", 32), phase bit 31 set, ctr restarts at 0.",
    );
    let sched = KeySchedule::new(f.group_id, f.epoch, Secret32::new(f.epoch_secret));
    let ring = EpochKeyring::new(sched.clone());
    let e_init: [u8; 32] = pattern(0x50);
    let e_resp: [u8; 32] = pattern(0x51);
    let mut rng = ChaCha20Rng::from_seed([0; 32]);
    let (mut ini, m1) = LinkSession::initiator_fixed(&f.member, &sched, 1, 0, &mut rng, e_init)?;
    let mut resp = LinkSession::responder_fixed(&f.leader, &ring, 1, 0, &mut rng, e_resp);
    let m2 = resp
        .on_handshake(&m1, 0)?
        .send
        .ok_or(CryptoError::Handshake)?;
    let m3 = ini
        .on_handshake(&m2, 0)?
        .send
        .ok_or(CryptoError::Handshake)?;
    resp.on_handshake(&m3, 0)?;
    let info = *ini.info().ok_or(CryptoError::NotEstablished)?;
    let c1 = ini
        .sec()
        .ok_or(CryptoError::NotEstablished)?
        .tx_key()
        .clone();
    let c2 = resp
        .sec()
        .ok_or(CryptoError::NotEstablished)?
        .tx_key()
        .clone();

    v.push(
        "handshake",
        json!({
            "group_id": hx(&f.group_id),
            "epoch": f.epoch,
            "psk": hx(sched.psk().expose()),
            "proto_ver": 1,
            "initiator_static_sk": hx(&f.seeds[1].1),
            "initiator_ephemeral_sk": hx(&e_init),
            "responder_static_sk": hx(&f.seeds[0].1),
            "responder_ephemeral_sk": hx(&e_resp),
        }),
        json!({
            "prologue": hx(&mesh_wire::frame::noise_prologue(&f.group_id, f.epoch)),
            "msg1": hx(&m1),
            "msg2": hx(&m2),
            "msg3": hx(&m3),
            "handshake_hash": hx(&info.handshake_hash),
            "c1_initiator_to_responder": hx(c1.expose()),
            "c2_responder_to_initiator": hx(c2.expose()),
        }),
    );

    // First SEC frame each way: the IDPROOF record.
    let p_ini = ini.create_idproof(&f.member)?;
    let p_resp = resp.create_idproof(&f.leader)?;
    let r_ini = encode_records(&[Record::IdProof(p_ini)])?;
    let r_resp = encode_records(&[Record::IdProof(p_resp)])?;
    let sec_i = ini.seal(&r_ini, 1_000)?;
    let sec_r = resp.seal(&r_resp, 1_000)?;
    resp.open(&sec_i, 1_000)?;
    ini.open(&sec_r, 1_000)?;
    let kp0 = KpCtr::new(false, 0);
    v.push(
        "sec/first-initiator",
        json!({ "key": hx(c1.expose()), "kp_ctr": kp0.0, "plaintext": hx(&r_ini) }),
        json!({
            "nonce": hx(&kp0.nonce()),
            "aad": hx(&kp0.aad()),
            "idproof_sig": hx(&p_ini.sig),
            "frame": hx(&sec_i),
        }),
    );
    v.push(
        "sec/first-responder",
        json!({ "key": hx(c2.expose()), "kp_ctr": kp0.0, "plaintext": hx(&r_resp) }),
        json!({
            "nonce": hx(&kp0.nonce()),
            "aad": hx(&kp0.aad()),
            "idproof_sig": hx(&p_resp.sig),
            "frame": hx(&sec_r),
        }),
    );

    // Second frame in phase 0, then a key update of the initiator direction.
    let ping = [0x02u8, 0x06, 0x01, 0x00, 0x10, 0x20, 0x30, 0x40];
    let sec_i2 = ini.seal(&ping, 2_000)?;
    resp.open(&sec_i2, 2_000)?;
    let sec = ini.sec().ok_or(CryptoError::NotEstablished)?;
    sec.update_tx_key(3_000);
    let k1 = sec.tx_key().clone();
    let sec_i3 = ini.seal(&ping, 3_000)?;
    resp.open(&sec_i3, 3_000)?;
    let kp1 = KpCtr::new(true, 0);
    v.push(
        "sec/phase0-ctr1",
        json!({ "key": hx(c1.expose()), "kp_ctr": KpCtr::new(false, 1).0, "plaintext": hx(&ping) }),
        json!({ "frame": hx(&sec_i2) }),
    );
    v.push(
        "sec/key-update-phase1-ctr0",
        json!({ "previous_key": hx(c1.expose()), "kp_ctr": kp1.0, "plaintext": hx(&ping) }),
        json!({
            "next_key": hx(k1.expose()),
            "nonce": hx(&kp1.nonce()),
            "aad": hx(&kp1.aad()),
            "frame": hx(&sec_i3),
        }),
    );
    let k2 = next_key(&k1);
    v.push(
        "keyupdate/chain",
        json!({ "k0": hx(c1.expose()) }),
        json!({ "k1": hx(k1.expose()), "k2": hx(k2.expose()) }),
    );
    Ok(v)
}
