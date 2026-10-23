use super::*;
use mesh_wire::codec::from_hex;
use mesh_wire::frame::{KpCtr, LinkFrame};
use mesh_wire::mesh::{GroupCtl, JoinReq, Rekey, Roster};

fn find<'a>(files: &'a [VectorFile], file: &str, name: &str) -> &'a Value {
    let f = files.iter().find(|f| f.file_name == file).unwrap();
    &f.vectors.iter().find(|v| v.name == name).unwrap().expected
}

fn bytes(v: &Value, key: &str) -> Vec<u8> {
    from_hex(v[key].as_str().unwrap()).unwrap()
}

fn arr<const N: usize>(v: &Value, key: &str) -> [u8; N] {
    bytes(v, key).try_into().unwrap()
}

#[test]
fn generation_is_byte_stable_and_names_unique() {
    let a: Vec<String> = generate().unwrap().iter().map(VectorFile::render).collect();
    let b: Vec<String> = generate().unwrap().iter().map(VectorFile::render).collect();
    assert_eq!(a, b);
    for f in generate().unwrap() {
        let mut names: Vec<&str> = f.vectors.iter().map(|v| v.name.as_str()).collect();
        let n = names.len();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), n, "duplicate vector name in {}", f.file_name);
        assert!(n > 0);
    }
}

/// The committed files must equal the generator output (the same check as
/// `cargo xtask vectors --check`, run here so `cargo test` catches drift too).
#[test]
fn committed_vectors_match() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../test-vectors/crypto");
    for f in generate().unwrap() {
        let path = dir.join(&f.file_name);
        let committed = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("{}: {e} (run `cargo xtask vectors`)", path.display()));
        assert!(
            committed == f.render(),
            "{} drifted (run `cargo xtask vectors`)",
            f.file_name
        );
    }
}

/// Independent re-check of the link vectors with the primitives only.
#[test]
fn link_vectors_reverify() {
    let files = generate().unwrap();
    let hs = find(&files, "link.json", "handshake");
    let sizes = [3 + 48, 3 + 96, 3 + 64];
    for (i, k) in ["msg1", "msg2", "msg3"].iter().enumerate() {
        let m = bytes(hs, k);
        assert_eq!(m.len(), sizes[i]);
        assert!(matches!(LinkFrame::decode(&m), Ok(LinkFrame::Hs { .. })));
    }
    let c1: [u8; 32] = arr(hs, "c1_initiator_to_responder");
    let hh: [u8; 32] = arr(hs, "handshake_hash");
    let f = fixture();
    let first = find(&files, "link.json", "sec/first-initiator");
    let frame = bytes(first, "frame");
    let mut ch = crate::sec::SecChannel::new(Secret32::new([0; 32]), Secret32::new(c1), 0);
    let pt = ch.open(&frame, 0).unwrap();
    // IDPROOF record: rtype 0x0C, LEB128 128 = 0x80 0x01.
    assert_eq!(&pt[..3], &[0x0C, 0x80, 0x01]);
    let proof = mesh_wire::record::IdProof {
        ed_pk: pt[3..35].try_into().unwrap(),
        x_pk: pt[35..67].try_into().unwrap(),
        sig: pt[67..131].try_into().unwrap(),
    };
    let id = crate::idproof::verify(
        &proof,
        crate::session::Role::Initiator,
        &hh,
        &f.member.x_pk(),
    )
    .unwrap();
    assert_eq!(id.id_hash(), f.member.id_hash());
    assert_eq!(bytes(first, "aad"), KpCtr::new(false, 0).aad().to_vec());
    let upd = find(&files, "link.json", "sec/key-update-phase1-ctr0");
    assert_eq!(
        bytes(upd, "next_key"),
        crate::sec::next_key(&Secret32::new(c1)).expose().to_vec()
    );
    assert_eq!(bytes(upd, "frame")[4] & 0x80, 0x80);
}

/// Independent re-check of the membership and invite vectors.
#[test]
fn membership_vectors_reverify() {
    let files = generate().unwrap();
    let f = fixture();
    let gid = f.group_id;
    let r = Roster::decode(&bytes(find(&files, "membership.json", "roster"), "body")).unwrap();
    crate::roster::verify(&r, &gid, &f.leader.ed_pk()).unwrap();
    assert_eq!(r.entries.len(), 4);
    let j = JoinReq::decode(&bytes(find(&files, "membership.json", "join_req"), "body")).unwrap();
    assert_eq!(
        crate::join::verify(&j, &gid).unwrap().id_hash(),
        f.member.id_hash()
    );
    let rk = Rekey::decode(&bytes(find(&files, "membership.json", "rekey"), "body")).unwrap();
    let s = crate::rekey::open(&rk, &gid, &f.leader.ed_pk(), &f.member2).unwrap();
    assert_eq!(s.expose(), &pattern::<32>(0x21));
    assert_eq!(
        crate::rekey::open(&rk, &gid, &f.leader.ed_pk(), &f.leader),
        Err(crate::CryptoError::NoEntry)
    );
    for name in ["group_ctl/close", "group_ctl/transfer-leader"] {
        let c = GroupCtl::decode(&bytes(find(&files, "membership.json", name), "payload")).unwrap();
        crate::group_ctl::verify(&c, &gid, &f.leader.ed_pk()).unwrap();
    }
    let inv = find(&files, "invite.json", "default");
    let parsed = crate::Invite::from_url(inv["url"].as_str().unwrap()).unwrap();
    parsed.verify_leader_sig().unwrap();
    assert_eq!(parsed.invite_sig.0.to_vec(), bytes(inv, "invite_sig"));
    // The roster publishes the same invite_sig.
    assert_eq!(r.invite_sig.to_vec(), bytes(inv, "invite_sig"));
}
