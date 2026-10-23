//! Serialization of [`LabRecord`] lines and of the one-row summary CSV (§12.3).

use crate::json::{Json, round};
use crate::record::{
    AnchorSample, AudioSample, DevSample, GpsFix, LABRUN_VERSION, LINE_TS_DECIMALS, LabRecord,
    LinkSample, Pcts, RawLine, RunLine, SampleLine, SummaryLine,
};

fn opt_round(v: Option<f64>, d: u32) -> Json {
    v.map(|x| round(x, d)).into()
}

fn pcts(p: Option<Pcts>) -> Json {
    match p {
        Some([a, b, c]) => Json::obj()
            .with("p50", round(a, 3))
            .with("p95", round(b, 3))
            .with("p99", round(c, 3)),
        None => Json::Null,
    }
}

fn gps(g: &GpsFix) -> Json {
    Json::obj()
        .with("lat", round(g.lat, 7))
        .with("lon", round(g.lon, 7))
        .with("spd", opt_round(g.spd, 2))
        .with("acc", opt_round(g.acc, 1))
}

fn run_json(r: &RunLine) -> Json {
    let p = &r.params;
    let variant = Json::Obj(r.variant.clone());
    let device = Json::obj()
        .with("model", r.device.model.as_str())
        .with("os", r.device.os.as_str())
        .with("app", r.device.app.as_str())
        .with("platform", r.device.platform.as_str());
    let peers: Vec<Json> = r
        .peers
        .iter()
        .map(|peer| {
            let mut o = Json::obj().with("labNode", peer.lab_node);
            if let Some(m) = &peer.model {
                o.set("model", m.as_str());
            }
            o
        })
        .collect();
    let mut params = Json::obj()
        .with("durationS", p.duration_s)
        .with("pattern", p.pattern.as_str())
        .with("streams", p.streams)
        .with("payloadB", p.payload_b)
        .with("intervalMs", p.interval_ms)
        .with("pingHz", p.ping_hz)
        .with("transport", p.transport.as_str())
        .with("linkRole", p.role.as_str())
        .with("headset", p.headset)
        .with("mixable", p.mixable)
        .with("gps", p.gps);
    if let Some(v) = &p.variant {
        params.set("label", v.as_str());
    }
    Json::obj()
        .with("t", "run")
        .with("v", LABRUN_VERSION)
        .with("runId", r.run_id.as_str())
        .with("bench", p.bench.as_str())
        .with("variant", variant)
        .with("audio", p.audio.as_str())
        .with("anchor", p.anchor.as_str())
        .with("startedAt", r.started_at.as_str())
        .with("device", device)
        .with("labNode", r.lab_node)
        .with("role", r.role.as_str())
        .with("peers", peers)
        .with("params", params)
}

fn link_json(l: &LinkSample) -> Json {
    Json::obj()
        .with("id", l.id)
        .with("kind", l.kind.as_str())
        .with("role", l.role.map(|r| r.as_str()))
        .with("peer", l.peer)
        .with("rssi", l.rssi)
        .with("txKbps", round(l.tx_kbps, 1))
        .with("rxKbps", round(l.rx_kbps, 1))
        .with("loss", round(l.loss, 4))
        .with("reorder", l.reorder)
        .with("rttP50", opt_round(l.rtt_p50, 2))
        .with("rttP95", opt_round(l.rtt_p95, 2))
        .with("owdP95", opt_round(l.owd_p95, 2))
        .with("stalls", l.stalls)
        .with("maxWrite", l.max_write)
}

fn audio_json(a: &AudioSample) -> Json {
    Json::obj()
        .with("capFps", opt_round(a.cap_fps, 1))
        .with("renFps", opt_round(a.ren_fps, 1))
        .with("running", a.running)
        .with("route", a.route.as_str())
        .with("ladder", a.ladder.as_str())
        .with("xruns", a.xruns)
}

fn dev_json(d: &DevSample) -> Json {
    Json::obj()
        .with("bat", opt_round(d.bat, 3))
        .with("chg", d.chg)
        .with("therm", d.therm.as_str())
        .with("app", d.app.as_str())
        .with("locked", d.locked)
}

fn anchor_json(a: &AnchorSample) -> Json {
    Json::obj()
        .with("ok", a.ok)
        .with("session", a.session)
        .with("location", a.location)
        .with("liveActivity", a.live_activity)
}

fn sample_json(s: &SampleLine) -> Json {
    let peer_gps: Vec<Json> = s
        .peer_gps
        .iter()
        .map(|(node, g)| {
            Json::obj()
                .with("labNode", *node)
                .with("lat", round(g.lat, 7))
                .with("lon", round(g.lon, 7))
        })
        .collect();
    Json::obj()
        .with("t", "s")
        .with("ts", round(s.ts, LINE_TS_DECIMALS))
        .with("links", s.links.iter().map(link_json).collect::<Vec<_>>())
        .with("gps", s.gps.as_ref().map_or(Json::Null, gps))
        .with("peerGps", peer_gps)
        .with("distM", opt_round(s.dist_m, 1))
        .with("audio", audio_json(&s.audio))
        .with("dev", dev_json(&s.dev))
        .with("anchor", anchor_json(&s.anchor))
        .with("bt", s.bt.as_str())
}

fn raw_json(r: &RawLine) -> Json {
    let mut o = Json::obj()
        .with("t", "raw")
        .with("ts", round(r.ts, LINE_TS_DECIMALS));
    if let Some(l) = r.link {
        o.set("link", l);
    }
    if !r.pings.is_empty() {
        let pings: Vec<Json> = r
            .pings
            .iter()
            .map(|p| {
                let mut a = vec![
                    Json::from(p.probe_id),
                    Json::Num(round(p.t1_s, 6)),
                    Json::Num(round(p.t4_s, 6)),
                    Json::Num(round(p.hold_s, 6)),
                ];
                if let Some(t3) = p.t3_remote_s {
                    a.push(Json::Num(round(t3, 6)));
                }
                Json::Arr(a)
            })
            .collect();
        o.set("pings", pings);
    }
    if !r.mesh_probes.is_empty() {
        let mp: Vec<Json> = r
            .mesh_probes
            .iter()
            .map(|m| {
                Json::obj()
                    .with("origin", m.origin)
                    .with("id", m.probe_id)
                    .with("tOrigin", round(m.t_origin_s, 6))
                    .with("tRx", round(m.t_rx_s, 6))
                    .with("hops", m.hops)
                    .with("ageMs", round(m.age_ms, 1))
                    .with(
                        "trace",
                        m.trace.iter().map(|n| Json::from(*n)).collect::<Vec<_>>(),
                    )
            })
            .collect();
        o.set("meshProbes", mp);
    }
    if !r.loopback.is_empty() {
        let lb: Vec<Json> = r
            .loopback
            .iter()
            .map(|l| {
                Json::obj()
                    .with("rep", l.rep)
                    .with("delayMs", opt_round(l.delay_ms, 3))
                    .with("ncc", opt_round(l.ncc, 3))
                    .with("erlDb", opt_round(l.erl_db.filter(|v| v.is_finite()), 1))
            })
            .collect();
        o.set("loopback", lb);
    }
    o
}

fn summary_json(s: &SummaryLine) -> Json {
    let mut o = Json::obj()
        .with("t", "summary")
        .with("durationS", round(s.duration_s, 1))
        .with("txKbps", Json::obj().with("mean", round(s.tx_kbps_mean, 1)))
        .with("rxKbps", Json::obj().with("mean", round(s.rx_kbps_mean, 1)))
        .with("loss", round(s.loss, 4))
        .with("owdMs", pcts(s.owd_ms))
        .with("rttMs", pcts(s.rtt_ms))
        .with("stalls", s.stalls)
        .with("reorder", s.reorder)
        .with("connIntervalMs", opt_round(s.conn_interval_ms, 2))
        .with("anchorDrops", s.anchor_drops);
    if s.loopback_ms.is_some() {
        o.set("loopbackMs", pcts(s.loopback_ms));
        o.set("perLegMs", opt_round(s.per_leg_ms, 2));
    }
    o.with("verdict", s.verdict.as_str())
}

impl LabRecord {
    /// The record as a JSON value.
    pub fn to_json(&self) -> Json {
        match self {
            LabRecord::Run(r) => run_json(r),
            LabRecord::Sample(s) => sample_json(s),
            LabRecord::Event { ts, ev, fields } => {
                let mut o = Json::obj()
                    .with("t", "e")
                    .with("ts", round(*ts, LINE_TS_DECIMALS))
                    .with("ev", ev.as_str());
                for (k, v) in fields {
                    o.set(k, v.clone());
                }
                o
            }
            LabRecord::Mark { ts, label } => Json::obj()
                .with("t", "mark")
                .with("ts", round(*ts, LINE_TS_DECIMALS))
                .with("label", label.as_str()),
            LabRecord::Raw(r) => raw_json(r),
            LabRecord::Summary(s) => summary_json(s),
        }
    }

    /// One JSON Lines line, terminated by `\n` (concatenating lines forms the file).
    pub fn to_jsonl(&self) -> String {
        let mut s = self.to_json().to_json_string();
        s.push('\n');
        s
    }
}

/// Header of the `…-summary.csv` file (one row per run).
pub const SUMMARY_CSV_HEADER: &str = "runId,bench,audio,anchor,startedAt,durationS,txKbps,loss,owdP95,rttP95,stalls,anchorDrops,verdict";

/// The `…-summary.csv` content (header + one row) of a finished run.
pub fn summary_csv(run: &RunLine, s: &SummaryLine) -> String {
    let opt = |v: Option<f64>| v.map(|x| round(x, 3).to_string()).unwrap_or_default();
    format!(
        "{SUMMARY_CSV_HEADER}\n{},{},{},{},{},{},{},{},{},{},{},{},{}\n",
        csv_field(&run.run_id),
        run.params.bench.as_str(),
        run.params.audio.as_str(),
        run.params.anchor.as_str(),
        csv_field(&run.started_at),
        round(s.duration_s, 1),
        round(s.tx_kbps_mean, 1),
        round(s.loss, 4),
        opt(s.owd_ms.map(|p| p[1])),
        opt(s.rtt_ms.map(|p| p[1])),
        s.stalls,
        s.anchor_drops,
        s.verdict.as_str(),
    )
}

/// RFC 4180 quoting when needed.
fn csv_field(v: &str) -> String {
    if v.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", v.replace('"', "\"\""))
    } else {
        v.to_string()
    }
}
