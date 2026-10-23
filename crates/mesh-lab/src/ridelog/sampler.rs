//! Builds the 1 Hz `s` line from cumulative inputs: every counter becomes a delta over
//! the previous sample (a counter that went backwards was reset by its producer and
//! counts from zero), gauges pass through.

use super::event::FrameType;
use super::sample::{CoreSample, DirCounters, JbCounters, MeshCounters, PlatDiag, RtCounters};
use crate::json::{Json, round};
use crate::time::{Instant, US_PER_S};
use std::collections::BTreeMap;

/// `now - prev`, or `now` when the producer reset the counter.
fn d(now: u64, prev: u64) -> u64 {
    if now >= prev { now - prev } else { now }
}

/// Recorder-side counters reported in `fx`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct FxCounters {
    pub suppressed: u64,
    pub redacted: u64,
    pub dropped_lines: u64,
}

/// What one `s` line is built from (all optional: a runtime without a core sample
/// still writes RT and platform data).
#[derive(Debug, Clone, Copy, Default)]
pub struct SampleInputs<'a> {
    pub core: Option<&'a CoreSample>,
    pub rt: Option<&'a RtCounters>,
    pub plat: Option<&'a PlatDiag>,
    /// Engine tick time spent in the last second (ms).
    pub cpu_ms: Option<f64>,
}

/// Delta state between two samples.
#[derive(Debug, Clone, Default)]
pub struct Sampler {
    links: BTreeMap<u32, (DirCounters, DirCounters, u64)>,
    mesh: Option<MeshCounters>,
    jb: BTreeMap<u16, JbCounters>,
    jb_total: Option<JbCounters>,
    tx_frames: Option<u64>,
    dropped_diag: u64,
    rt: Option<(RtCounters, Instant)>,
    fx: FxCounters,
}

fn opt(v: Option<f32>, d: u32) -> Json {
    v.map(|x| round(f64::from(x), d)).into()
}

fn dir_json(now: &DirCounters, prev: &DirCounters, rx: bool) -> Json {
    let mut by_type = Json::obj();
    let mut by_type_b = Json::obj();
    for ty in FrameType::ALL {
        let n = d(now.by_type[ty.index()], prev.by_type[ty.index()]);
        if n > 0 {
            by_type.set(ty.as_str(), n);
        }
        let b = d(
            now.bytes_by_type[ty.index()],
            prev.bytes_by_type[ty.index()],
        );
        if b > 0 {
            by_type_b.set(ty.as_str(), b);
        }
    }
    let mut o = Json::obj()
        .with("frames", d(now.frames, prev.frames))
        .with("bytes", d(now.bytes, prev.bytes));
    if rx {
        o.set("bad", d(now.bad, prev.bad));
    }
    o.with("byType", by_type).with("byTypeB", by_type_b)
}

fn jb_json(now: &JbCounters, prev: &JbCounters, o: Json) -> Json {
    o.with("recv", d(now.received, prev.received))
        .with("late", d(now.late, prev.late))
        .with("dup", d(now.duplicates, prev.duplicates))
        .with("fec", d(now.fec_recovered, prev.fec_recovered))
        .with("plc", d(now.concealed, prev.concealed))
        .with("expanded", d(now.expanded, prev.expanded))
        .with("dropped", d(now.dropped, prev.dropped))
}

fn cb(v: Option<(u32, u32)>) -> Json {
    v.map_or(Json::Null, |(a, b)| Json::Arr(vec![a.into(), b.into()]))
}

impl Sampler {
    pub fn new() -> Self {
        Self::default()
    }

    /// Forgets every previous value (new engine session).
    pub fn reset(&mut self) {
        *self = Sampler::default();
    }

    /// Builds one `s` line at `ts` seconds. `fx` is cumulative, like everything else.
    pub fn sample(
        &mut self,
        now: Instant,
        ts: f64,
        input: SampleInputs<'_>,
        fx: FxCounters,
    ) -> Json {
        let SampleInputs {
            core,
            rt,
            plat,
            cpu_ms,
        } = input;
        let mut s = Json::obj().with("t", "s").with("ts", round(ts, 3));
        s.set("links", self.links_json(core));
        s.set("mesh", self.mesh_json(core.and_then(|c| c.mesh)));
        s.set(
            "floor",
            core.and_then(|c| c.floor)
                .map_or(Json::Null, |(talkers, me)| {
                    Json::obj().with("talkers", talkers).with("me", me)
                }),
        );
        s.set("audio", self.audio_json(now, core, rt));
        s.set("jb", self.jb_json(core));
        s.set("jbTotal", self.jb_total_json(core.and_then(|c| c.jb_total)));
        let p = plat.cloned().unwrap_or_default();
        s.set(
            "dev",
            if plat.is_some() {
                Json::obj()
                    .with("bat", opt(p.bat, 3))
                    .with("chg", p.chg)
                    .with("therm", p.therm)
                    .with("app", p.app)
                    .with("locked", p.locked)
                    .with("call", p.call)
            } else {
                Json::Null
            },
        );
        s.set(
            "anchor",
            if plat.is_some() {
                Json::obj()
                    .with("session", p.anchor_session)
                    .with("location", p.anchor_location)
            } else {
                Json::Null
            },
        );
        s.set("bt", p.bt);
        s.set("aware", p.aware);
        s.set("distM", core.and_then(|c| c.dist_m));
        s.set(
            "cpuMs",
            cpu_ms.map_or(Json::Null, |v| Json::obj().with("engine", round(v, 2))),
        );
        let dropped_diag = core.map_or(self.dropped_diag, |c| c.dropped_diag);
        let fx_json = Json::obj()
            .with("droppedDiag", d(dropped_diag, self.dropped_diag))
            .with("suppressed", d(fx.suppressed, self.fx.suppressed))
            .with("redacted", d(fx.redacted, self.fx.redacted))
            .with("droppedLines", d(fx.dropped_lines, self.fx.dropped_lines));
        self.dropped_diag = dropped_diag;
        self.fx = fx;
        s.with("fx", fx_json)
    }

    fn links_json(&mut self, core: Option<&CoreSample>) -> Json {
        let Some(core) = core else {
            return Json::Arr(Vec::new());
        };
        let mut out = Vec::with_capacity(core.links.len());
        let mut next = BTreeMap::new();
        for l in &core.links {
            let zero = (DirCounters::default(), DirCounters::default(), 0);
            let prev = self.links.get(&l.id).unwrap_or(&zero);
            let q = Json::obj()
                .with("voice", l.q.voice)
                .with("ctrl", l.q.ctrl)
                .with("bulk", l.q.bulk)
                .with("units", l.q.units);
            out.push(
                Json::obj()
                    .with("id", l.id)
                    .with("peer", l.peer)
                    .with("kind", l.kind.as_str())
                    .with("role", l.role.map(|r| r.as_str()))
                    .with("state", l.state)
                    .with("rssi", l.rssi)
                    .with("mtu", l.mtu)
                    .with("rttP50", opt(l.rtt_p50_ms, 2))
                    .with("rttP95", opt(l.rtt_p95_ms, 2))
                    .with("loss", opt(l.loss.map(|x| x.clamp(0.0, 1.0)), 4))
                    .with("grantKbps", opt(l.grant_kbps, 1))
                    .with("q", q)
                    .with("wb", d(l.would_block, prev.2))
                    .with("tx", dir_json(&l.tx, &prev.0, false))
                    .with("rx", dir_json(&l.rx, &prev.1, true)),
            );
            next.insert(l.id, (l.tx, l.rx, l.would_block));
        }
        self.links = next;
        Json::Arr(out)
    }

    fn mesh_json(&mut self, m: Option<MeshCounters>) -> Json {
        let Some(m) = m else {
            return Json::Null;
        };
        let p = self.mesh.unwrap_or_default();
        self.mesh = Some(m);
        Json::obj()
            .with("nbrs", m.nbrs)
            .with("reach", m.reach)
            .with("islands", m.islands)
            .with("fwd", d(m.fwd, p.fwd))
            .with("dup", d(m.dup, p.dup))
            .with("suppressed", d(m.suppressed, p.suppressed))
            .with("tooOld", d(m.too_old, p.too_old))
            .with("pruned", d(m.pruned, p.pruned))
            .with("floorRefused", d(m.floor_refused, p.floor_refused))
            .with("voiceDropped", d(m.voice_dropped, p.voice_dropped))
            .with("burstsShed", d(m.bursts_shed, p.bursts_shed))
            .with("ownRefused", d(m.own_refused, p.own_refused))
    }

    fn audio_json(
        &mut self,
        now: Instant,
        core: Option<&CoreSample>,
        rt: Option<&RtCounters>,
    ) -> Json {
        let ca = core.and_then(|c| c.audio);
        let mut a = Json::obj().with("running", rt.is_some_and(|r| r.running));
        a.set("route", ca.map(|c| c.route));
        a.set("ladder", ca.map(|c| c.ladder));
        match rt {
            Some(r) => {
                let (prev, t) = self.rt.unwrap_or((RtCounters::default(), now));
                let dt = now.since(t) as f64 / US_PER_S as f64;
                let fps = |a: u64, b: u64| {
                    if dt > 0.0 && self.rt.is_some() {
                        Json::Num(round(d(a, b) as f64 / dt, 1))
                    } else {
                        Json::Null
                    }
                };
                a.set("capFps", fps(r.captured_frames, prev.captured_frames));
                a.set("renFps", fps(r.rendered_frames, prev.rendered_frames));
                a.set("cbCapFrames", cb(r.cb_capture));
                a.set("cbRenFrames", cb(r.cb_render));
                a.set("cbRenMax", r.max_render_frames);
                a.set("renTarget", r.render_target);
                a.set("over", d(r.capture_overruns, prev.capture_overruns));
                a.set("under", d(r.render_underruns, prev.render_underruns));
                a.set(
                    "overSamples",
                    d(r.capture_overrun_samples, prev.capture_overrun_samples),
                );
                a.set(
                    "underSamples",
                    d(r.render_underrun_samples, prev.render_underrun_samples),
                );
                a.set("capDbfs", opt(r.capture_rms_dbfs, 1));
                a.set("renDbfs", opt(r.render_rms_dbfs, 1));
                self.rt = Some((*r, now));
            }
            None => {
                self.rt = None;
                a.set("capDbfs", opt(ca.and_then(|c| c.cap_dbfs), 1));
                a.set("renDbfs", opt(ca.and_then(|c| c.ren_dbfs), 1));
            }
        }
        a.set("vad", ca.map(|c| c.vad));
        a.set("ptt", ca.map(|c| c.ptt));
        a.set("muted", ca.map(|c| c.muted));
        let tx = ca.map(|c| c.tx_frames);
        a.set("txFrames", tx.map(|t| d(t, self.tx_frames.unwrap_or(0))));
        if tx.is_some() {
            self.tx_frames = tx;
        }
        a
    }

    fn jb_json(&mut self, core: Option<&CoreSample>) -> Json {
        let Some(core) = core else {
            return Json::Arr(Vec::new());
        };
        let mut out = Vec::with_capacity(core.jb.len());
        let mut next = BTreeMap::new();
        for j in &core.jb {
            let prev = self.jb.get(&j.origin).copied().unwrap_or_default();
            let o = Json::obj()
                .with("origin", j.origin)
                .with("targetMs", j.target_ms)
                .with("depthMs", j.depth_ms);
            out.push(jb_json(&j.counters, &prev, o).with("owdP95", opt(j.owd_p95_ms, 2)));
            next.insert(j.origin, j.counters);
        }
        self.jb = next;
        Json::Arr(out)
    }

    fn jb_total_json(&mut self, t: Option<JbCounters>) -> Json {
        let Some(t) = t else {
            return Json::Null;
        };
        let prev = self.jb_total.unwrap_or_default();
        self.jb_total = Some(t);
        jb_json(&t, &prev, Json::obj())
    }
}
