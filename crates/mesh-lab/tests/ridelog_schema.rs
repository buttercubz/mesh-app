//! Contract C7b: every line the `ridelog/1` recorder writes validates against
//! `schema/ridelog-1.schema.json` (boon, draft 2020-12); rate limits, rotation,
//! redaction, the RAM ring and the enable switch behave as specified.

use boon::{Compiler, SchemaIndex, Schemas};
use mesh_lab::ridelog::*;
use mesh_lab::{Instant, LinkKind, LinkRole};
use serde_json::Value;

const URL: &str = "https://convlink.voice/schema/ridelog-1.schema.json";

fn schema() -> (Schemas, SchemaIndex) {
    let mut schemas = Schemas::new();
    let mut c = Compiler::new();
    let v: Value = serde_json::from_str(RIDELOG_SCHEMA_JSON).expect("schema is JSON");
    c.add_resource(URL, v).expect("resource");
    let idx = c.compile(URL, &mut schemas).expect("schema compiles");
    (schemas, idx)
}

fn check(s: &Schemas, idx: SchemaIndex, line: &str) -> Result<(), String> {
    let v: Value = serde_json::from_str(line).map_err(|e| format!("not JSON: {e}: {line}"))?;
    s.validate(&v, idx)
        .map_err(|e| format!("{e:#}\nline: {line}"))
}

fn device() -> DeviceInfo {
    DeviceInfo {
        platform: "android".into(),
        model: "SM-X520".into(),
        os: "Android 16 (API 36)".into(),
        app: "0.1.0 (12)".into(),
        build: "debug".into(),
        engine: "ffi".into(),
    }
}

fn header(id: &str) -> Header {
    let mut h = Header::new(id, "2026-10-22T20:04:11.512Z", 81_234_567_890, device());
    h.caps.push(("l2cap".into(), true.into()));
    h.settings.push(("ptt".into(), "vox".into()));
    h
}

fn all_events() -> Vec<DiagEvent> {
    use DiagEvent as E;
    vec![
        E::Session {
            ride_key: Some(ride_key(&[7; 16], 20261022)),
            node: Some(4711),
            role: Some("leader"),
        },
        E::RadioState {
            ble: "on",
            aware: "unsupported",
        },
        E::DiscoveryMode {
            mode: "forming",
            scan: Some("lowLatency"),
            adv: Some("250ms"),
            duty: Some(1.0),
        },
        E::PeerSeen {
            disc: 9,
            rssi: Some(-70),
            group_match: true,
            epoch_behind: false,
        },
        E::ConnectTry {
            disc: 9,
            via: "l2cap",
            attempt: 1,
        },
        E::ConnectFail {
            disc: 9,
            reason: "timeout",
            attempt: 2,
            backoff_s: 4,
        },
        E::Abort {
            disc: 9,
            ignore_s: 30,
            why: "duplicate",
        },
        E::LinkUp {
            link: 3,
            kind: LinkKind::BleL2cap,
            role: Some(LinkRole::Central),
            mtu: Some(247),
            psm: Some(129),
        },
        E::Handshake {
            link: 3,
            peer: Some(918),
            ok: true,
            ms: 120,
            err: None,
        },
        E::LinkDown {
            link: 3,
            peer: Some(918),
            reason: "supervisionTimeout",
            up_s: 212,
            tx_b: 12_000,
            rx_b: 11_000,
        },
        E::LinkParams {
            link: 3,
            prio: "high",
            grant_kbps: 48.0,
        },
        E::WouldBlockBurst {
            link: 3,
            count: 14,
            ms: 1000,
        },
        E::ReconnectArmed {
            peer: 918,
            until_s: 60,
        },
        E::ClkOffset {
            link: 3,
            peer: 918,
            offset_us: -1_532_000,
            rtt_us: 38_000,
        },
        E::NbrChange {
            peer: 918,
            q: 0.93,
            good: true,
            congested: false,
            kind: LinkKind::BleGatt,
        },
        E::Reach {
            nodes: 3,
            added: vec![918],
            removed: vec![],
        },
        E::Partition {
            islands: 2,
            mine: 2,
            others: vec![1],
        },
        E::Heard {
            peer: 918,
            last_heard_s: 111,
            lost: true,
        },
        E::Floor {
            kind: FloorKind::Grant,
            origin: 918,
            reason: None,
            streams: 1,
        },
        E::Floor {
            kind: FloorKind::Reject,
            origin: 919,
            reason: Some("maxStreams"),
            streams: 2,
        },
        E::Rekey { epoch: 4 },
        E::MemberRemoved {
            peer: 919,
            epoch: 5,
        },
        E::LeaderChange {
            peer: 918,
            epoch: 5,
        },
        E::BudgetCap {
            cap_kbps: 120.0,
            level: 1,
        },
        E::CaptureStart { why: "goLive" },
        E::CaptureStop { why: "ptt" },
        E::CaptureLost {
            reason: "builtInMicOff",
            detail: Some("wind"),
        },
        E::MicState {
            state: "paused",
            cause: Some("builtInMicOff"),
        },
        E::Route {
            route: "hfp",
            prev: Some("builtIn"),
            reason: Some("newDevice"),
            hfp_wb: Some(true),
        },
        E::JbRetarget {
            origin: 918,
            from_ms: 60,
            to_ms: 80,
            why: "late",
        },
        E::Vad { on: true, p: 0.87 },
        E::Ptt { on: false },
        E::PromptPlay { key: "prompt.lost" },
        E::Glitch {
            kind: GlitchKind::Underrun,
            n: 2,
            ms: 10,
        },
        E::EngineStart { mode: "group" },
        E::EngineStop,
        E::EngineCrash {
            at: "crates/mesh-core/src/engine.rs:584",
        },
        E::EffectOverflow { dropped: 3 },
        E::Logs {
            debug: 10,
            info: 4,
            warn: 1,
            error: 0,
        },
        E::ClkAnchor {
            wall_ms: 1_792_699_451_512,
            mono_us: 81_234_567_890,
        },
    ]
}

fn core_sample(k: u64) -> CoreSample {
    let mut tx = DirCounters::default();
    let mut rx = DirCounters::default();
    for _ in 0..k {
        tx.count(FrameType::Nbr, 20);
        tx.count(FrameType::Ping, 18);
        rx.count(FrameType::Lstat, 26);
    }
    CoreSample {
        links: vec![CoreLink {
            id: 3,
            peer: Some(918),
            kind: LinkKind::BleL2cap,
            role: Some(LinkRole::Central),
            state: "up",
            rssi: Some(-61),
            mtu: Some(247),
            rtt_p50_ms: Some(38.0),
            rtt_p95_ms: Some(61.5),
            loss: Some(0.004),
            grant_kbps: Some(48.0),
            q: QueueDepth {
                voice: 2,
                ctrl: 0,
                bulk: 5,
                units: 3,
            },
            would_block: 4 * k,
            tx,
            rx,
        }],
        mesh: Some(MeshCounters {
            nbrs: 2,
            reach: 3,
            islands: 1,
            fwd: 12 * k,
            dup: 40 * k,
            ..Default::default()
        }),
        floor: Some((1, "idle")),
        audio: Some(CoreAudio {
            route: "builtIn",
            ladder: "L0",
            vad: false,
            ptt: false,
            muted: false,
            tx_frames: 25 * k,
            cap_dbfs: Some(-42.1),
            ren_dbfs: None,
        }),
        jb: vec![CoreJb {
            origin: 918,
            target_ms: 60,
            depth_ms: 40,
            owd_p95_ms: Some(31.2),
            counters: JbCounters {
                received: 25 * k,
                late: k,
                concealed: k,
                ..Default::default()
            },
        }],
        jb_total: Some(JbCounters {
            received: 25 * k,
            late: k,
            concealed: k,
            expanded: 1,
            ..Default::default()
        }),
        dist_m: None,
        dropped_diag: 0,
    }
}

fn rt(k: u64) -> RtCounters {
    RtCounters {
        running: true,
        captured_frames: 48_000 * k,
        rendered_frames: 48_000 * k,
        render_underruns: k / 2,
        render_underrun_samples: 96 * (k / 2),
        max_render_frames: 480,
        max_capture_frames: 192,
        render_target: 960,
        cb_capture: Some((192, 192)),
        cb_render: Some((192, 480)),
        capture_rms_dbfs: Some(-40.0),
        ..Default::default()
    }
}

fn plat() -> PlatDiag {
    PlatDiag {
        bat: Some(0.81),
        chg: Some(false),
        therm: Some("nominal"),
        app: Some("foreground"),
        locked: Some(false),
        call: Some(false),
        bt: Some("on"),
        aware: Some("unsupported"),
        anchor_session: Some(true),
        anchor_location: Some(false),
    }
}

fn lines(r: &mut RideRecorder) -> Vec<String> {
    r.take_batches().into_iter().flat_map(|b| b.lines).collect()
}

#[test]
fn every_line_kind_validates() {
    let (s, idx) = schema();
    let mut r = RideRecorder::new(RecorderConfig::default());
    let t0 = Instant(5_000_000);
    r.begin_session(t0, header("d-20261022-2004-7k2q"));
    for (i, e) in all_events().iter().enumerate() {
        r.event(t0.add_millis(i as u64), e);
    }
    let note = NoteEvent::new(
        DiagSrc::Plat,
        "perm",
        vec![
            ("name".into(), "microphone".into()),
            ("state".into(), "granted".into()),
            ("api".into(), "checkSelfPermission".into()),
            ("ids".into(), vec![mesh_lab::json::Json::Int(1)].into()),
        ],
    )
    .unwrap();
    r.note(t0.add_millis(50), &note);
    r.mark(t0.add_millis(60), MarkLabel::Crackle, DiagSrc::Ui);
    for k in 1..=3u64 {
        let now = t0.add_millis(1000 * k);
        let (c, t, p) = (core_sample(k), rt(k), plat());
        let input = SampleInputs {
            core: Some(&c),
            rt: Some(&t),
            plat: Some(&p),
            cpu_ms: Some(18.4),
        };
        r.sample(now, input);
    }
    r.sample(t0.add_millis(4000), SampleInputs::default());
    r.end_session(t0.add_millis(4500), EndReason::SessionEnd);
    let out = lines(&mut r);
    let mut kinds = std::collections::BTreeSet::new();
    for l in &out {
        if let Err(e) = check(&s, idx, l) {
            panic!("{e}");
        }
        let v: Value = serde_json::from_str(l).unwrap();
        kinds.insert(v["t"].as_str().unwrap().to_string());
    }
    assert_eq!(
        kinds.into_iter().collect::<Vec<_>>(),
        ["e", "end", "hdr", "mark", "s"]
    );
    // Deltas, not totals: the third sample saw one more second of everything.
    let s3: Value = serde_json::from_str(
        out.iter()
            .filter(|l| l.contains("\"t\":\"s\""))
            .nth(2)
            .unwrap(),
    )
    .unwrap();
    assert_eq!(s3["links"][0]["tx"]["byType"]["nbr"], 1);
    assert_eq!(s3["links"][0]["wb"], 4);
    assert_eq!(s3["mesh"]["dup"], 40);
    assert_eq!(s3["audio"]["capFps"], 48_000.0);
    assert_eq!(s3["audio"]["cbRenFrames"], serde_json::json!([192, 480]));
    assert_eq!(s3["jb"][0]["recv"], 25);
    assert_eq!(s3["jbTotal"]["expanded"], 0);
    // The session event fills the identity of later headers.
    assert_eq!(r.header(), None);
}

#[test]
fn rotation_rate_limits_and_ring() {
    let (s, idx) = schema();
    let cfg = RecorderConfig {
        seg_max_bytes: 1024,
        ring_lines: 64,
        ..Default::default()
    };
    let mut r = RideRecorder::new(cfg);
    let t0 = Instant(0);
    r.begin_session(t0, header("d-20261022-2004-aaaa"));
    r.event(
        t0,
        &DiagEvent::Session {
            ride_key: Some([0x9f, 0x3a, 0x71, 0xc2]),
            node: Some(7),
            role: None,
        },
    );
    // 200 identical events in one second: 40 pass (burst), the rest are suppressed.
    for i in 0..200u64 {
        r.event(Instant(i * 1000), &DiagEvent::Ptt { on: i % 2 == 0 });
    }
    r.sample(Instant(1_000_000), SampleInputs::default());
    let out = lines(&mut r);
    let ptt = out.iter().filter(|l| l.contains("\"ev\":\"ptt\"")).count();
    assert!((40..=45).contains(&ptt), "{ptt}");
    let sup: Value = serde_json::from_str(
        out.iter()
            .find(|l| l.contains("\"ev\":\"suppressed\""))
            .unwrap(),
    )
    .unwrap();
    assert_eq!(sup["of"], "ptt");
    assert_eq!(sup["n"].as_u64().unwrap() as usize, 200 - ptt);
    // Segments rotate by size, each starting with a header that carries the identity.
    let hdrs: Vec<Value> = out
        .iter()
        .filter(|l| l.starts_with("{\"t\":\"hdr\""))
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    assert!(hdrs.len() >= 2, "{}", hdrs.len());
    assert_eq!(hdrs[1]["seg"], 2);
    assert_eq!(hdrs[1]["rideKey"], "9f3a71c2");
    assert_eq!(hdrs[1]["node"], 7);
    assert!(out.iter().any(|l| l.contains("\"reason\":\"rotate\"")));
    for l in &out {
        check(&s, idx, l).unwrap();
    }
    // The ring keeps the newest lines and always exports a header first.
    let ring = r.ring_snapshot();
    assert!(ring.len() <= 65);
    assert!(ring[0].starts_with("{\"t\":\"hdr\""));
    // Disabled: nothing reaches the file store, the ring still records.
    r.set_enabled(Instant(2_000_000), false);
    r.mark(Instant(2_000_000), MarkLabel::Lag, DiagSrc::Ui);
    assert!(r.take_batches().is_empty());
    assert!(
        r.ring_snapshot()
            .last()
            .unwrap()
            .contains("\"label\":\"lag\"")
    );
    // Back on mid-session: a new segment starts with its header (never a headless file).
    r.set_enabled(Instant(2_100_000), true);
    r.mark(Instant(2_200_000), MarkLabel::Other, DiagSrc::Ui);
    let back = r.take_batches();
    let last = back.last().unwrap();
    assert!(
        last.lines[0].starts_with("{\"t\":\"hdr\""),
        "{:?}",
        last.lines
    );
    assert!(last.lines.iter().any(|l| l.contains("\"label\":\"other\"")));
}

#[test]
fn notes_are_validated_and_redacted() {
    assert_eq!(
        NoteEvent::new(DiagSrc::Core, "x", vec![]),
        Err(NoteError::BadSrc)
    );
    assert_eq!(
        NoteEvent::new(DiagSrc::Plat, "Bad", vec![]),
        Err(NoteError::BadEv)
    );
    assert!(NoteEvent::new(DiagSrc::Plat, "fgs", vec![("ts".into(), 1u8.into())]).is_err());
    let obj = mesh_lab::json::Json::obj().with("a", 1u8);
    assert!(NoteEvent::new(DiagSrc::Plat, "fgs", vec![("x".into(), obj)]).is_err());
    let secret = "a1b2c3d4e5f60718293a4b5c6d7e8f90a1b2c3d4";
    let n = NoteEvent::new(DiagSrc::Ui, "cmd", vec![("err".into(), secret.into())]).unwrap();
    let mut r = RideRecorder::new(RecorderConfig::default());
    r.begin_session(Instant(0), header("d-20261022-2004-bbbb"));
    r.note(Instant(1), &n);
    let out = lines(&mut r);
    assert!(out.iter().all(|l| !l.contains(secret)));
    assert!(out.iter().any(|l| l.contains("<redacted>")));
    assert_eq!(r.counters().redacted, 1);
}

#[test]
fn no_key_material_from_a_seeded_group_reaches_the_log() {
    // Property: whatever bytes a group secret has, its hex/base64 renderings never
    // survive a note (the only path for strings from platform code).
    let mut x: u64 = 0x9e37_79b9_7f4a_7c15;
    for _ in 0..200 {
        let mut key = [0u8; 32];
        for b in key.iter_mut() {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            *b = x as u8;
        }
        let hex: String = key.iter().map(|b| format!("{b:02x}")).collect();
        let n = NoteEvent::new(
            DiagSrc::Plat,
            "leak",
            vec![("v".into(), hex.as_str().into())],
        )
        .unwrap();
        let mut r = RideRecorder::new(RecorderConfig::default());
        r.begin_session(Instant(0), header("d-20261022-2004-cccc"));
        r.note(Instant(1), &n);
        for l in lines(&mut r) {
            assert!(!l.contains(&hex[..24]), "{l}");
        }
    }
}

#[test]
fn ride_key_is_stable_per_group_and_day() {
    let a = ride_key(&[1; 16], 20261022);
    assert_eq!(a, ride_key(&[1; 16], 20261022));
    assert_ne!(a, ride_key(&[1; 16], 20261023));
    assert_ne!(a, ride_key(&[2; 16], 20261022));
    assert_eq!(hex4(&[0x9f, 0x3a, 0x71, 0xc2]), "9f3a71c2");
    // Cross-language vectors (packages/engine-web/test/diag.test.ts computes the same).
    assert_eq!(hex4(&ride_key(&[1; 16], 20261022)), "b212d3d5");
    assert_eq!(lab_ride_key(mesh_wire::frame::lab_tag(1234)), "lab-bbb2");
    assert_eq!(lab_ride_key(0x1a2b), "lab-1a2b");
}
