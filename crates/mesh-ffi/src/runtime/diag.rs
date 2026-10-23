//! The `ridelog/1` diagnostics recorder of the native runtime (diagnostics spec §3.1).
//!
//! [`DiagHub`] wraps mesh-lab's sans-IO [`RideRecorder`] with the runtime clock, a
//! mutex (platform threads inject notes, the engine thread samples) and the
//! `PlatformHooks::diag_write` output. Lines are batched: the platform file store sees
//! at most one `diag_write` per segment per second (plus explicit flushes before an
//! export). The hooks are always called **outside** the lock, so a platform that calls
//! `diag_note` from inside `diag_write` cannot deadlock.
//!
//! Sessions: a new session (`d-YYYYMMDD-HHMM-xxxx`) starts on every engine `start`; a
//! platform note arriving while no session is open starts an idle session (permission
//! checks at app launch, Bluetooth toggles between rides).

use std::hash::{BuildHasher, Hasher};
use std::sync::{Arc, Mutex, MutexGuard};

use mesh_core::Instant;
use mesh_lab::json::Json;
use mesh_lab::ridelog::{
    CoreSample, DeviceInfo, DiagEvent, DiagRecord, DiagSrc, EndReason, GlitchKind, Header,
    MarkLabel, NoteEvent, PlatDiag, RecorderConfig, RideRecorder, RtCounters, SampleInputs,
    iso8601, session_id,
};

use super::os::Clock;
use crate::api::hooks::PlatformHooks;
use crate::api::outputs::FfiLogLevel;
use crate::api::types::{FfiPlatformSample, FfiRadioAvail, FfiThermal};

/// Sample period while a session runs (µs).
pub const SAMPLE_US: u64 = 1_000_000;
/// Sample period while idle (µs, 0.2 Hz).
pub const IDLE_SAMPLE_US: u64 = 5_000_000;
/// Wall-clock anchor period (µs).
pub const ANCHOR_US: u64 = 60_000_000;
/// `logs` summary period (µs).
pub const LOGS_US: u64 = 10_000_000;
/// Flush to the platform at least this often (µs) …
pub const FLUSH_US: u64 = 1_000_000;
/// … or as soon as this many lines wait.
pub const FLUSH_LINES: usize = 512;

/// Recorder status for the plugin's `diagStatus`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiagState {
    pub enabled: bool,
    pub session_id: Option<String>,
    pub ride_key: Option<String>,
    pub node: Option<u16>,
    pub pending_lines: u32,
    pub suppressed: u64,
    pub redacted: u64,
}

struct Inner {
    rec: RideRecorder,
    device: DeviceInfo,
    running: bool,
    last_sample: Option<Instant>,
    last_anchor: Option<Instant>,
    last_logs: Option<Instant>,
    last_flush: Option<Instant>,
    plat: Option<PlatDiag>,
    core: Option<CoreSample>,
    busy_us: u64,
    logs: [u32; 4],
    salt: u64,
    last_rt: Option<RtCounters>,
}

/// The runtime's recorder (one per `MeshEngine`).
pub struct DiagHub {
    hooks: Arc<dyn PlatformHooks>,
    clock: Clock,
    inner: Mutex<Inner>,
}

/// mesh-core and mesh-lab instants are the same µs clock in two crates.
fn li(now: Instant) -> mesh_lab::Instant {
    mesh_lab::Instant(now.0)
}

fn random_u32(salt: u64, wall_ms: u64) -> u32 {
    let mut h = std::hash::RandomState::new().build_hasher();
    h.write_u64(salt);
    h.write_u64(wall_ms);
    h.finish() as u32
}

impl DiagHub {
    /// A recorder on the runtime's clock (`clock` is the engine clock, so diagnostics
    /// `ts` and core instants agree).
    pub fn new(hooks: Arc<dyn PlatformHooks>, device: DeviceInfo, clock: Clock) -> Self {
        DiagHub {
            hooks,
            clock,
            inner: Mutex::new(Inner {
                rec: RideRecorder::new(RecorderConfig::default()),
                device,
                running: false,
                last_sample: None,
                last_anchor: None,
                last_logs: None,
                last_flush: None,
                plat: None,
                core: None,
                busy_us: 0,
                logs: [0; 4],
                salt: 0,
                last_rt: None,
            }),
        }
    }

    /// Engine time now.
    pub fn now(&self) -> Instant {
        self.clock.now()
    }

    /// `Effect::Diag` from the dispatcher (stage 2 of the recorder: mesh-core emits).
    pub fn record_now(&self, r: DiagRecord) {
        self.record(self.now(), r);
    }

    fn lock(&self) -> MutexGuard<'_, Inner> {
        // A panic while recording must never take diagnostics (or the engine) down.
        self.inner.lock().unwrap_or_else(|p| p.into_inner())
    }

    /// File writing on/off (the RAM ring always records). `debug` sets the `build`.
    pub fn configure(&self, enabled: bool, debug: bool) {
        let now = self.now();
        let mut g = self.lock();
        g.rec.set_enabled(li(now), enabled);
        g.device.build = if debug { "debug" } else { "release" }.into();
    }

    /// Starts a new session (engine start). `ride_key` is known for Lab and Demo; a
    /// group session's key comes from the core's `session` event or [`Self::set_ride_key`].
    pub fn begin(&self, now: Instant, wall_ms: u64, mode: &'static str, ride_key: Option<String>) {
        {
            let mut g = self.lock();
            Self::open(&mut g, now, wall_ms, ride_key);
            g.running = true;
            g.rec.event(li(now), &DiagEvent::EngineStart { mode });
            g.rec.event(
                li(now),
                &DiagEvent::ClkAnchor {
                    wall_ms,
                    mono_us: now.0,
                },
            );
            g.last_anchor = Some(now);
        }
        self.flush(now);
    }

    fn open(g: &mut Inner, now: Instant, wall_ms: u64, ride_key: Option<String>) {
        g.salt = g.salt.wrapping_add(1);
        let id = session_id(wall_ms, random_u32(g.salt, wall_ms));
        let mut header = Header::new(&id, &iso8601(wall_ms), now.0, g.device.clone());
        header.ride_key = ride_key;
        g.rec.begin_session(li(now), header);
        g.core = None;
        g.last_sample = None;
        g.busy_us = 0;
    }

    /// Sets the ride key of the running session (group sessions: computed from the
    /// group id by the runtime at start).
    pub fn set_ride_key(&self, key: String) {
        self.lock().rec.set_identity(Some(key), None, None);
    }

    /// Ends the running session (engine stop or crash).
    pub fn end(&self, now: Instant, crash: Option<&'static str>) {
        {
            let mut g = self.lock();
            if g.rec.session_id().is_none() {
                return;
            }
            match crash {
                Some(at) => g.rec.event(li(now), &DiagEvent::EngineCrash { at }),
                None => g.rec.event(li(now), &DiagEvent::EngineStop),
            }
            Self::sample_locked(&mut g, now);
            g.rec.end_session(li(now), EndReason::SessionEnd);
            g.running = false;
        }
        self.flush(now);
    }

    /// Closes the session because the app is terminating.
    pub fn terminate(&self, now: Instant) {
        self.lock()
            .rec
            .end_session(li(now), EndReason::AppTerminate);
        self.flush(now);
    }

    fn ensure_session(g: &mut Inner, now: Instant) {
        if g.rec.session_id().is_none() {
            Self::open(g, now, super::os::wall_ms(), None);
        }
    }

    /// A core record (`Effect::Diag`): events are written now, samples are kept for
    /// the next 1 Hz line.
    pub fn record(&self, now: Instant, r: DiagRecord) {
        let mut g = self.lock();
        match r {
            DiagRecord::Event(e) => {
                Self::ensure_session(&mut g, now);
                g.rec.event(li(now), &e);
            }
            DiagRecord::Sample(s) => g.core = Some(*s),
        }
    }

    /// A typed runtime event.
    pub fn event(&self, now: Instant, e: &DiagEvent) {
        let mut g = self.lock();
        Self::ensure_session(&mut g, now);
        g.rec.event(li(now), e);
    }

    /// A platform / UI note (`diag_note`). `fields_json` is a JSON object of scalars
    /// or arrays of numbers. Returns why it was refused.
    pub fn note(&self, now: Instant, src: &str, ev: &str, fields_json: &str) -> Result<(), String> {
        let src = match DiagSrc::parse(src) {
            Some(DiagSrc::Core) | None => return Err(format!("bad src {src:?}")),
            Some(s) => s,
        };
        let fields = parse_fields(fields_json)?;
        let n = NoteEvent::new(src, ev, fields).map_err(|e| format!("{e:?}"))?;
        {
            let mut g = self.lock();
            Self::ensure_session(&mut g, now);
            g.rec.note(li(now), &n);
        }
        self.maybe_flush(now);
        Ok(())
    }

    /// A user marker from a fixed chip label.
    pub fn mark(&self, now: Instant, label: &str, src: &str) -> bool {
        let (Some(label), Some(src)) = (MarkLabel::parse(label), DiagSrc::parse(src)) else {
            return false;
        };
        {
            let mut g = self.lock();
            Self::ensure_session(&mut g, now);
            g.rec.mark(li(now), label, src);
        }
        self.flush(now);
        true
    }

    /// The platform's 1 Hz sample (device, anchors, radios).
    pub fn platform(&self, s: &FfiPlatformSample) {
        self.lock().plat = Some(plat_diag(s));
    }

    /// Counts a log line (summarised as one `logs` event per 10 s).
    pub fn count_log(&self, level: FfiLogLevel) {
        let i = match level {
            FfiLogLevel::Debug => 0,
            FfiLogLevel::Info => 1,
            FfiLogLevel::Warn => 2,
            FfiLogLevel::Error => 3,
        };
        let mut g = self.lock();
        g.logs[i] = g.logs[i].saturating_add(1);
    }

    /// Engine-thread tick: accounts `busy_us` of engine work, writes the 1 Hz sample
    /// (0.2 Hz idle), the 60 s clock anchor and the 10 s log summary, then flushes.
    /// `rt` is the claimed RT plane (read only when a sample is due: reading restarts
    /// the callback-size window).
    pub fn tick(&self, now: Instant, rt: Option<&crate::rt::RtShared>, busy_us: u64) {
        let due = {
            let mut g = self.lock();
            g.busy_us = g.busy_us.saturating_add(busy_us);
            let period = if g.running { SAMPLE_US } else { IDLE_SAMPLE_US };
            let due = g.last_sample.is_none_or(|t| now.since(t) >= period);
            if due && g.rec.session_id().is_some() {
                if g.last_anchor.is_none_or(|t| now.since(t) >= ANCHOR_US) {
                    let wall_ms = super::os::wall_ms();
                    g.rec.event(
                        li(now),
                        &DiagEvent::ClkAnchor {
                            wall_ms,
                            mono_us: now.0,
                        },
                    );
                    g.last_anchor = Some(now);
                }
                if g.last_logs.is_none_or(|t| now.since(t) >= LOGS_US) {
                    let [debug, info, warn, error] = std::mem::take(&mut g.logs);
                    if debug + info + warn + error > 0 {
                        g.rec.event(
                            li(now),
                            &DiagEvent::Logs {
                                debug,
                                info,
                                warn,
                                error,
                            },
                        );
                    }
                    g.last_logs = Some(now);
                }
                let rt = rt.map(rt_counters);
                if let Some(r) = &rt {
                    Self::glitches(&mut g, now, r);
                }
                g.last_rt = rt;
                let core = g.core.take();
                let plat = g.plat.clone();
                let cpu_ms = Some(std::mem::take(&mut g.busy_us) as f64 / 1000.0);
                let input = SampleInputs {
                    core: core.as_ref(),
                    rt: rt.as_ref(),
                    plat: plat.as_ref(),
                    cpu_ms,
                };
                g.rec.sample(li(now), input);
                g.last_sample = Some(now);
            }
            due
        };
        if due {
            self.flush(now);
        }
    }

    /// `glitch` events for the second that just ended (the counts are also in `s`;
    /// the events put them on the merged timeline).
    fn glitches(g: &mut Inner, now: Instant, r: &RtCounters) {
        let Some(p) = g.last_rt else {
            return;
        };
        let delta = |a: u64, b: u64| if a >= b { a - b } else { a };
        let ms = |samples: u64| u32::try_from(samples / 48).unwrap_or(u32::MAX);
        let under = delta(r.render_underruns, p.render_underruns);
        if under > 0 {
            let n = u32::try_from(under).unwrap_or(u32::MAX);
            let s = delta(r.render_underrun_samples, p.render_underrun_samples);
            g.rec.event(
                li(now),
                &DiagEvent::Glitch {
                    kind: GlitchKind::Underrun,
                    n,
                    ms: ms(s),
                },
            );
        }
        let over = delta(r.capture_overruns, p.capture_overruns);
        if over > 0 {
            let n = u32::try_from(over).unwrap_or(u32::MAX);
            let s = delta(r.capture_overrun_samples, p.capture_overrun_samples);
            g.rec.event(
                li(now),
                &DiagEvent::Glitch {
                    kind: GlitchKind::Overrun,
                    n,
                    ms: ms(s),
                },
            );
        }
        if let Some((lo, hi)) = r.cb_render
            && lo != hi
        {
            let ms = u32::try_from(u64::from(hi - lo) / 48).unwrap_or(u32::MAX);
            g.rec.event(
                li(now),
                &DiagEvent::Glitch {
                    kind: GlitchKind::CbSizeChange,
                    n: 1,
                    ms,
                },
            );
        }
    }

    fn sample_locked(g: &mut Inner, now: Instant) {
        let core = g.core.take();
        let plat = g.plat.clone();
        let input = SampleInputs {
            core: core.as_ref(),
            rt: None,
            plat: plat.as_ref(),
            cpu_ms: None,
        };
        g.rec.sample(li(now), input);
        g.last_sample = Some(now);
    }

    /// Flushes when a second passed since the last flush or many lines wait.
    pub fn maybe_flush(&self, now: Instant) {
        let due = {
            let g = self.lock();
            g.rec.pending_lines() >= FLUSH_LINES
                || g.last_flush.is_none_or(|t| now.since(t) >= FLUSH_US)
        };
        if due {
            self.flush(now);
        }
    }

    /// Hands every pending batch to `PlatformHooks::diag_write` (outside the lock).
    pub fn flush(&self, now: Instant) {
        let batches = {
            let mut g = self.lock();
            g.last_flush = Some(now);
            g.rec.take_batches()
        };
        for b in batches {
            self.hooks.diag_write(b.session_id, b.seg, b.lines);
        }
    }

    /// The current session as one self-contained JSONL file (RAM ring).
    pub fn ring(&self) -> Vec<String> {
        self.lock().rec.ring_snapshot()
    }

    pub fn state(&self) -> DiagState {
        let g = self.lock();
        let h = g.rec.header();
        let fx = g.rec.counters();
        DiagState {
            enabled: g.rec.is_enabled(),
            session_id: g.rec.session_id().map(str::to_string),
            ride_key: h.and_then(|h| h.ride_key.clone()),
            node: h.and_then(|h| h.node),
            pending_lines: u32::try_from(g.rec.pending_lines()).unwrap_or(u32::MAX),
            suppressed: fx.suppressed,
            redacted: fx.redacted,
        }
    }
}

/// Parses a note's fields: a JSON object whose values are scalars or arrays of numbers.
pub fn parse_fields(json: &str) -> Result<Vec<(String, Json)>, String> {
    if json.trim().is_empty() {
        return Ok(Vec::new());
    }
    let v: serde_json::Value = serde_json::from_str(json).map_err(|e| format!("fields: {e}"))?;
    let serde_json::Value::Object(map) = v else {
        return Err("fields must be a JSON object".into());
    };
    map.into_iter().map(|(k, v)| Ok((k, to_json(v)?))).collect()
}

fn to_json(v: serde_json::Value) -> Result<Json, String> {
    use serde_json::Value as V;
    Ok(match v {
        V::Null => Json::Null,
        V::Bool(b) => Json::Bool(b),
        V::Number(n) => match n.as_i64() {
            Some(i) => Json::Int(i),
            None => Json::Num(n.as_f64().unwrap_or(f64::NAN)),
        },
        V::String(s) => Json::Str(s),
        V::Array(items) => Json::Arr(items.into_iter().map(to_json).collect::<Result<_, _>>()?),
        V::Object(_) => return Err("nested objects are not allowed in a note".into()),
    })
}

fn avail(a: FfiRadioAvail) -> &'static str {
    match a {
        FfiRadioAvail::On => "on",
        FfiRadioAvail::Off => "off",
        FfiRadioAvail::Unauthorized => "unauthorized",
        FfiRadioAvail::Unsupported => "unsupported",
    }
}

/// The `dev`/`anchor`/radio part of the `s` line from the platform sample.
pub fn plat_diag(s: &FfiPlatformSample) -> PlatDiag {
    PlatDiag {
        bat: s.battery.map(|b| b.clamp(0.0, 1.0) as f32),
        chg: s.charging,
        therm: s.thermal.map(|t| match t {
            FfiThermal::Nominal => "nominal",
            FfiThermal::Fair => "fair",
            FfiThermal::Serious => "serious",
            FfiThermal::Critical => "critical",
        }),
        app: Some(if s.background {
            "background"
        } else {
            "foreground"
        }),
        locked: s.locked,
        call: Some(s.on_call),
        bt: Some(avail(s.ble)),
        aware: Some(avail(s.aware)),
        anchor_session: s.anchor_session,
        anchor_location: s.anchor_location,
    }
}

/// RT counters for the sample (cumulative) plus the callback-size window.
pub fn rt_counters(rt: &crate::rt::RtShared) -> RtCounters {
    let st = rt.stats();
    let (cb_capture, cb_render) = rt.take_cb_window();
    RtCounters {
        running: rt.is_active(),
        captured_frames: st.captured_frames,
        rendered_frames: st.rendered_frames,
        capture_overruns: u64::from(st.capture_overruns),
        render_underruns: u64::from(st.render_underruns),
        render_underrun_samples: st.render_underrun_samples,
        capture_overrun_samples: st.capture_overrun_samples,
        max_render_frames: st.max_render_frames,
        max_capture_frames: st.max_capture_frames,
        render_target: st.render_target,
        cb_capture,
        cb_render,
        capture_rms_dbfs: Some(st.capture_rms_dbfs),
        render_rms_dbfs: Some(st.render_rms_dbfs),
    }
}
