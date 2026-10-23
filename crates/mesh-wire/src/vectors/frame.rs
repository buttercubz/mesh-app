//! `frame.json`: every LinkFrame type (DESIGN §4.3), SEC nonce/AAD in both key phases,
//! LAB padding, `lab_tag`, Noise prologue.

use serde_json::json;

use super::{VectorFile, hx, pattern, pattern_array};
use crate::error::Result;
use crate::frame::{
    KpCtr, LinkFrame, NOISE_MSG_SIZES, RejectReason, TAG_LEN, encode_lab_padded, lab_tag,
    noise_prologue,
};
use crate::record::{Lprobe, Ping, Record, encode_records};

pub(super) fn file() -> Result<VectorFile> {
    let mut f = VectorFile::new(
        "frame.json",
        "LinkFrame (DESIGN §4.3): lf_type 0x00 HS (hs_step, proto_ver, noise_msg), 0x01 SEC \
         (kp_ctr u32 LE: b31 key phase, b30-0 ctr; ciphertext; tag[16]; nonce 0^4 || u64 LE \
         kp_ctr; AAD lf_type || kp_ctr), 0x02 LAB (lab_tag u16 LE, records, PAD to SEC size), \
         0x03 REJECT (reason). Noise messages and ciphertexts here are fixed filler: real \
         handshake bytes and sealed frames are in test-vectors/crypto/.",
    );

    for (i, size) in NOISE_MSG_SIZES.iter().enumerate() {
        let step = i as u8 + 1;
        let msg = pattern(0x30 + step, *size);
        let fr = LinkFrame::Hs {
            step,
            proto_ver: 1,
            noise_msg: &msg,
        }
        .encode()?;
        f.push(
            &format!("hs/msg{step}"),
            json!({ "hs_step": step, "proto_ver": 1, "noise_msg": hx(&msg) }),
            &fr,
        );
    }

    let cases: [(&str, bool, u32, usize); 4] = [
        ("sec/phase0-ctr0", false, 0, 95),
        ("sec/phase0-ctr1", false, 1, 8),
        ("sec/phase1-ctr0-after-key-update", true, 0, 95),
        ("sec/phase1-ctr-max", true, 0x7FFF_FFFF, 1),
    ];
    for (name, phase, ctr, ct_len) in cases {
        let kp = KpCtr::new(phase, ctr);
        let sealed = pattern(0xA0, ct_len + TAG_LEN);
        let fr = LinkFrame::Sec {
            kp_ctr: kp,
            sealed: &sealed,
        }
        .encode()?;
        f.push(
            name,
            json!({
                "key_phase": u8::from(phase),
                "ctr": ctr,
                "kp_ctr": kp.0,
                "sealed": hx(&sealed),
                "aad": hx(&kp.aad()),
                "nonce": hx(&kp.nonce()),
            }),
            &fr,
        );
        f.push(&format!("{name}/aad"), json!({ "kp_ctr": kp.0 }), &kp.aad());
        f.push(
            &format!("{name}/nonce"),
            json!({ "kp_ctr": kp.0 }),
            &kp.nonce(),
        );
    }

    for code in [0u16, 1234, 9999] {
        f.push(
            &format!("lab_tag/{code:04}"),
            json!({ "lab_code": code, "rule": "SHA-256(\"cvic/lab\" || u16 LE lab_code)[0..2], read LE" }),
            &lab_tag(code).to_le_bytes(),
        );
    }

    let records = encode_records(&[
        Record::Ping(Ping {
            id: 7,
            t_tx: 0x0102_0304,
        }),
        Record::Lprobe(Lprobe {
            probe_id: 42,
            t_tx: 1_000_000,
            flags: 0,
            pad_len: 60,
        }),
    ])?;
    let tag = lab_tag(1234);
    f.push(
        "lab/ping-lprobe-padded",
        json!({ "lab_code": 1234, "lab_tag": tag, "records": hx(&records),
                "note": "PAD record (0x0e, len 16) appended: LAB size = SEC size of the same records" }),
        &encode_lab_padded(tag, &records)?,
    );
    let raw = LinkFrame::Lab {
        lab_tag: tag,
        records: &records,
    }
    .encode()?;
    f.push(
        "lab/unpadded",
        json!({ "lab_tag": tag, "records": hx(&records) }),
        &raw,
    );

    for r in 1u8..=8 {
        let reason = RejectReason::from_u8(r, false)?;
        f.push(
            &format!("reject/{r}"),
            json!({ "reason": r, "meaning": format!("{reason:?}") }),
            &LinkFrame::Reject { reason }.encode()?,
        );
    }

    let gid: [u8; 16] = pattern_array(0x10);
    f.push(
        "noise/prologue",
        json!({ "group_id": hx(&gid), "epoch": 7, "rule": "\"cvic/v1\" || group_id || u32 LE epoch" }),
        &noise_prologue(&gid, 7),
    );
    Ok(f)
}
