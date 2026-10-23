//! `mesh.json` (header, relay rewrite, every ptype as a full MeshPacket) and
//! `segmentation.json` (SEG packets and SYNC chunks), DESIGN §4.5-§4.7.

use serde_json::json;

use super::control::{sample_join_req, sample_rekey, sample_roster};
use super::{VectorFile, hx, pattern, pattern_array};
use crate::error::Result;
use crate::mesh::{
    Bandwidth, EncoderProfile, Ext, FloorReject, FloorRejectReason, FrameDur, MeshHeader,
    MeshPacket, MeshPayload, MeshProbe, PacketType, PathHint, Position, Presence, PresenceAudio,
    PresenceCaps, PresenceLinks, PresenceState, Talk, TalkMode, TalkPrio, TalkState, TrafficClass,
    Voice, VoiceCfg, encode_packet, segment_packets, sync_chunks,
};
use crate::record::{Record, SyncKind};

const ORIGIN: u16 = 0x0102;
const SESS: u8 = 0x5A;

fn hdr(seq: u16) -> MeshHeader {
    MeshHeader::new(PacketType::Voice, ORIGIN, SESS, seq)
}

pub(super) fn file() -> Result<VectorFile> {
    let mut f = VectorFile::new(
        "mesh.json",
        "MeshPacket (DESIGN §4.5-§4.6): 9-byte header vt (ver<<5 | ptype), ttl_hops (ttl<<4 | \
         hops), origin u16, sess u8 (non-zero), seq u16, age u8 (2 ms units), flags (class<<6 | \
         LEADER<<5 | SEG<<4), then the payload. expected = header || payload. Signatures are \
         filler (real ones in test-vectors/crypto/).",
    );

    let mut h = MeshHeader::new(PacketType::Talk, 0xBEEF, 0x01, 0xFFFF);
    h.leader = true;
    h.age = 255;
    f.push(
        "header/talk-leader-age-saturated",
        json!({ "ptype": 2, "ttl": 6, "hops": 0, "origin": 0xBEEF, "sess": 1, "seq": 0xFFFF,
                "age": 255, "class": 0, "leader": true, "seg": false }),
        &h.encode()?,
    );
    let mut v = hdr(10);
    v.age = 4;
    let relayed = v
        .relayed(13_000)
        .ok_or(crate::error::WireError::NoSpace { what: "ttl" })?;
    f.push(
        "header/voice-relayed-once",
        json!({ "before": hx(&v.encode()?), "added_us": 13_000,
                "rule": "ttl -= 1, hops += 1, age += round(added_us / 2000)" }),
        &relayed.encode()?,
    );

    let opus = pattern(0xF0, 80);
    let cfg0 = VoiceCfg::new(FrameDur::Ms40, Bandwidth::Wide, false, false, true, false);
    let v0 = Voice {
        burst: 3,
        ts: Voice::ts_for_frame(0, FrameDur::Ms40),
        cfg: cfg0,
        opus: &opus,
    };
    f.push(
        "voice/ble16-frame0-first",
        json!({ "burst": 3, "frame_index": 0, "ts": 0, "dur_ms": 40, "bw": "wide", "first": true,
                "opus": hx(&opus) }),
        &encode_packet(&hdr(100), &MeshPayload::Voice(v0))?,
    );
    let cfg2 = VoiceCfg::new(FrameDur::Ms40, Bandwidth::Wide, false, true, false, false);
    let v2 = Voice {
        burst: 3,
        ts: Voice::ts_for_frame(2, FrameDur::Ms40),
        cfg: cfg2,
        opus: &opus,
    };
    f.push(
        "voice/ble-dred-frame2-ts8",
        json!({ "burst": 3, "frame_index": 2, "ts": 8, "dred": true, "opus": hx(&opus) }),
        &encode_packet(&hdr(102), &MeshPayload::Voice(v2))?,
    );

    let talk = |state, elapsed| Talk {
        burst: 3,
        state,
        prio: TalkPrio::Member,
        mode: TalkMode::Vox,
        profile: EncoderProfile::Ble16,
        elapsed_10ms: elapsed,
    };
    for (name, state, elapsed) in [
        ("talk/start", TalkState::Start, 0u16),
        ("talk/keep-elapsed-1s", TalkState::Keep, 100),
        ("talk/stop-elapsed-4.2s", TalkState::Stop, 420),
    ] {
        f.push(
            name,
            json!({ "burst": 3, "state": state as u8, "prio": 1, "mode": 1, "profile": 1, "elapsed_10ms": elapsed }),
            &encode_packet(&hdr(200), &MeshPayload::Talk(talk(state, elapsed)))?,
        );
    }
    let dred_talk = Talk {
        profile: EncoderProfile::BleDred,
        prio: TalkPrio::Leader,
        mode: TalkMode::Ptt,
        ..talk(TalkState::Start, 0)
    };
    f.push(
        "talk/start-leader-ptt-ble-dred",
        json!({ "burst": 3, "state": 1, "prio": 2, "mode": 2, "profile": 5, "elapsed_10ms": 0 }),
        &encode_packet(&hdr(201), &MeshPayload::Talk(dred_talk))?,
    );

    let fr = FloorReject {
        target: 0x0304,
        target_sess: 0x77,
        burst: 9,
        reason: FloorRejectReason::Cap,
        at: 0x0A0B,
    };
    f.push(
        "floor_reject/cap",
        json!({ "target": 0x0304, "target_sess": 0x77, "burst": 9, "reason": 1, "at": 0x0A0B }),
        &encode_packet(&hdr(300), &MeshPayload::FloorReject(fr))?,
    );

    let presence = Presence {
        state: PresenceState::new(
            PresenceState::RIDE_RIDING,
            PresenceState::TALK_TALKING,
            false,
            false,
            true,
            true,
        ),
        audio: PresenceAudio::new(1, PresenceAudio::ROUTE_HFP_WB, false, true, true),
        battery: 87,
        links: PresenceLinks::new(2, 1),
        caps: PresenceCaps::new(0, true, true),
        interval: 20,
        heard: 0x8000_0005,
        ..Presence::default()
    }
    .with_versions(0x0003_0011, 0x0001_0002);
    f.push(
        "presence/riding-talking-radio-off-recently",
        json!({ "roster_version": 0x0003_0011u32, "roster_ver16": 0x0011, "epoch": 0x0001_0002u32,
                "epoch16": 2, "ride": 1, "talk": 1, "capture_ok": true, "live": true, "ladder": 1,
                "route": 3, "render_ok": true, "sharing_distance": true, "battery": 87,
                "links_ble": 2, "links_aware": 1, "platform": 0, "dred_ok": true,
                "radio_off_recently": true, "interval": 20, "heard": 0x8000_0005u32 }),
        &encode_packet(&hdr(400), &MeshPayload::Presence(presence))?,
    );
    f.push(
        "presence/payload-14-bytes",
        json!({ "of": "presence/riding-talking-radio-off-recently" }),
        &presence.encode(),
    );

    super::control::push_signed_packets(&mut f, &hdr(500))?;

    let mut probe = MeshProbe::new(0xCAFE_F00D, 9_876_543_210);
    probe.append(0x0102)?;
    probe.append(0x0304)?;
    let mut ph = hdr(600);
    ph.class = TrafficClass::Lab;
    f.push(
        "mesh_probe/two-relays",
        json!({ "probe_id": 0xCAFE_F00Du32, "t_origin": 9_876_543_210u64, "trace": [0x0102, 0x0304] }),
        &encode_packet(&ph, &MeshPayload::MeshProbe(probe))?,
    );

    let hint = PathHint {
        profile: EncoderProfile::Ble12,
        at: 0x0102,
        target: PathHint::ALL,
    };
    f.push(
        "path_hint/ble12-all",
        json!({ "profile": 2, "at": 0x0102, "target": 0 }),
        &encode_packet(&hdr(700), &MeshPayload::PathHint(hint))?,
    );

    let pos = Position {
        lat_e7: 194_326_077,
        lon_e7: -991_332_080,
        speed_dms: 278,
        heading_2deg: 45,
        acc_m: 5,
        fix_age_100ms: 3,
        rsv: 0,
    };
    f.push(
        "position/moving",
        json!({ "lat_e7": 194_326_077, "lon_e7": -991_332_080, "speed_dms": 278, "heading_2deg": 45,
                "acc_m": 5, "fix_age_100ms": 3, "rsv": 0 }),
        &encode_packet(&hdr(800), &MeshPayload::Position(pos))?,
    );

    let ext_body = [9u8, 8, 7];
    f.push(
        "ext/type1",
        json!({ "ext_type": 1, "body": hx(&ext_body) }),
        &encode_packet(
            &hdr(900),
            &MeshPayload::Ext(Ext {
                ext_type: 1,
                body: &ext_body,
            }),
        )?,
    );

    let unk = MeshHeader::new(PacketType::Unknown(0x15), ORIGIN, SESS, 901);
    let body = [1u8, 2, 3, 4];
    f.push(
        "unknown/0x15-relayed-not-delivered",
        json!({ "ptype": 0x15, "payload": hx(&body) }),
        &MeshPacket {
            header: unk,
            payload: &body,
        }
        .encode()?,
    );
    Ok(f)
}

pub(super) fn segmentation_file() -> Result<VectorFile> {
    let mut f = VectorFile::new(
        "segmentation.json",
        "Segmentation (DESIGN §4.6.1 SEG flag, §4.4 SYNC): payload msg_id u16 || seg u8 \
         (idx<<4 | count-1) || data (<= 900 B), <= 16 segments; each segment is its own \
         MeshPacket with its own seq. SYNC carries the same chunks link-scoped.",
    );
    let gid: [u8; 16] = pattern_array(0x10);
    let roster = sample_roster(&gid, 32).encode()?;
    let rh = MeshHeader::new(PacketType::Roster, ORIGIN, SESS, 0xFFFE);
    f.push_parts(
        "roster/32-entries-3-segments",
        json!({ "origin": ORIGIN, "sess": SESS, "first_seq": 0xFFFE, "msg_id": 0x0042,
                "body": hx(&roster), "note": "seq wraps: 0xfffe, 0xffff, 0x0000" }),
        &segment_packets(&rh, 0xFFFE, 0x0042, &roster)?,
    );
    let join = sample_join_req().encode();
    let jh = MeshHeader::new(PacketType::JoinReq, 0x0304, 0x11, 5);
    f.push_parts(
        "join_req/1-segment",
        json!({ "origin": 0x0304, "sess": 0x11, "first_seq": 5, "msg_id": 7, "body": hx(&join) }),
        &segment_packets(&jh, 5, 7, &join)?,
    );
    let rekey = sample_rekey(31).encode()?;
    let kh = MeshHeader::new(PacketType::Rekey, ORIGIN, SESS, 40);
    f.push_parts(
        "rekey/31-entries-2-segments",
        json!({ "origin": ORIGIN, "sess": SESS, "first_seq": 40, "msg_id": 0x0100, "body": hx(&rekey) }),
        &segment_packets(&kh, 40, 0x0100, &rekey)?,
    );
    let records = sync_chunks(SyncKind::Roster, 0x0042, &roster)?
        .into_iter()
        .map(|s| Record::Sync(s).encode())
        .collect::<Result<Vec<_>>>()?;
    f.push_parts(
        "sync/roster-3-chunks",
        json!({ "kind": 1, "msg_id": 0x0042, "body": hx(&roster) }),
        &records,
    );
    Ok(f)
}
