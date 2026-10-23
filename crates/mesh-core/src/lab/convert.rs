//! Conversions between the UI contract (mesh-api), the bench logic (mesh-lab) and the
//! engine's platform state; run ids and ISO time stamps from the wall clock.

use mesh_api::{
    LabAnchor, LabAudio, LabBench, LabLiveSample, LabPercentiles, LabRate, LabSummary, LabVerdict,
};
use mesh_lab::record::{AnchorSample, AudioSample, DevSample, GpsFix, SummaryLine};
use mesh_lab::{Anchor, Audio, Bench, LinkRole, Pattern, Transport, Verdict};

use crate::ui::DeviceState;
use crate::{AudioIoState, Instant, links::RadioAvail};

/// mesh-api bench → mesh-lab bench (same ids).
pub fn bench(b: LabBench) -> Bench {
    Bench::parse(b.as_str()).unwrap_or(Bench::T0)
}

/// mesh-lab bench → mesh-api bench.
pub fn api_bench(b: Bench) -> LabBench {
    b.as_str().parse().unwrap_or(LabBench::T0)
}

pub fn audio(a: LabAudio) -> Audio {
    Audio::parse(a.as_str()).unwrap_or(Audio::Off)
}

pub fn api_audio(a: Audio) -> LabAudio {
    a.as_str().parse().unwrap_or(LabAudio::Off)
}

pub fn anchor(a: LabAnchor) -> Anchor {
    Anchor::parse(a.as_str()).unwrap_or(Anchor::None)
}

pub fn api_anchor(a: Anchor) -> LabAnchor {
    a.as_str().parse().unwrap_or(LabAnchor::None)
}

/// UI `LabParams` → mesh-lab `LabParams` (bench defaults are applied by
/// `RunParams::resolve`).
pub fn params(p: &mesh_api::LabParams) -> mesh_lab::LabParams {
    mesh_lab::LabParams {
        audio: audio(p.audio),
        anchor: anchor(p.anchor),
        gps: p.gps,
        duration_s: p.duration_s,
        pattern: p.pattern.and_then(|x| Pattern::parse(x.as_str())),
        streams: p.streams,
        payload_b: p.payload_b,
        interval_ms: p.interval_ms,
        ping_hz: p.ping_hz,
        transport: p.transport.and_then(|x| Transport::parse(x.as_str())),
        role: p.role.and_then(|x| LinkRole::parse(x.as_str())),
        headset: p.headset,
        mixable: p.mixable,
        variant: p.variant.clone(),
    }
}

fn verdict(v: Verdict) -> LabVerdict {
    match v {
        Verdict::Go => LabVerdict::Go,
        Verdict::NoGo => LabVerdict::NoGo,
        Verdict::Inconclusive => LabVerdict::Inconclusive,
    }
}

fn pcts(p: Option<[f64; 3]>) -> Option<LabPercentiles> {
    p.map(|[p50, p95, p99]| LabPercentiles { p50, p95, p99 })
}

/// The on-device summary of the `labFinished` event.
pub fn summary(s: &SummaryLine) -> LabSummary {
    LabSummary {
        duration_s: s.duration_s,
        tx_kbps: LabRate {
            mean: s.tx_kbps_mean,
        },
        loss: s.loss,
        owd_ms: pcts(s.owd_ms),
        rtt_ms: pcts(s.rtt_ms),
        stalls: s.stalls,
        verdict: verdict(s.verdict),
    }
}

/// The verdict of the UI contract.
pub fn api_verdict(v: Verdict) -> LabVerdict {
    verdict(v)
}

/// `labProgress` live numbers.
pub fn live(l: &mesh_lab::LiveSample) -> LabLiveSample {
    LabLiveSample {
        links: l.links,
        peers: l.peers,
        tx_kbps: l.tx_kbps,
        rx_kbps: l.rx_kbps,
        loss: l.loss,
        rtt_p50_ms: l.rtt_p50_ms,
        rtt_p95_ms: l.rtt_p95_ms,
        rssi: l.rssi,
        dist_m: l.dist_m,
        stalls: l.stalls,
        anchor_ok: l.anchor_ok,
    }
}

/// Whether the run's keep-alive anchor is confirmed by the latest platform sample
/// (§12.1). Without an audio path the foreground app itself is the anchor; with the
/// `none` control variant any delivered sample proves the process ran.
pub fn anchor_ok(d: &DeviceState, run_audio: Audio, run_anchor: Anchor) -> Option<bool> {
    let s = d.sample.as_ref()?;
    if run_audio == Audio::Off {
        return Some(!s.background || run_anchor == Anchor::None);
    }
    match run_anchor {
        Anchor::None => Some(true),
        Anchor::Audio => s.anchor.session,
        Anchor::AudioLocation => match (s.anchor.session, s.anchor.location) {
            (Some(a), Some(b)) => Some(a && b),
            (Some(false), _) | (_, Some(false)) => Some(false),
            _ => None,
        },
    }
}

/// The 1 Hz platform sample handed to the bench runner.
pub fn platform_sample(
    d: &DeviceState,
    ble: RadioAvail,
    run_audio: Audio,
    run_anchor: Anchor,
) -> mesh_lab::PlatformSample {
    let s = d.sample.clone().unwrap_or_default();
    let running = matches!(d.io, AudioIoState::Running | AudioIoState::Restored);
    mesh_lab::PlatformSample {
        gps: s.gps.map(|g| GpsFix {
            lat: g.lat,
            lon: g.lon,
            spd: g.speed_mps,
            acc: g.acc_m,
        }),
        heading_deg: s.gps.and_then(|g| g.heading_deg),
        audio: AudioSample {
            cap_fps: s.audio.capture_fps,
            ren_fps: s.audio.render_fps,
            running,
            route: d.route.kind.as_str().to_string(),
            ladder: "L0".into(),
            xruns: s.audio.xruns,
        },
        dev: DevSample {
            bat: s.battery,
            chg: s.charging,
            therm: d.thermal_str().to_string(),
            app: if s.background {
                "background".into()
            } else {
                "foreground".into()
            },
            locked: s.locked,
        },
        anchor: AnchorSample {
            ok: anchor_ok(d, run_audio, run_anchor),
            session: s.anchor.session,
            location: s.anchor.location,
            live_activity: s.anchor.live_activity,
        },
        bt: ble.lab(),
    }
}

/// Civil date `(year, month, day)` of a day count since 1970-01-01 (proleptic
/// Gregorian, Howard Hinnant's algorithm).
pub fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let y = yoe + era * 400 + i64::from(m <= 2);
    (y, m, d)
}

/// `YYYY-MM-DDTHH:MM:SS.mmmZ` of a Unix time in ms.
pub fn iso_ms(unix_ms: u64) -> String {
    let secs = (unix_ms / 1_000) as i64;
    let (y, mo, d) = civil_from_days(secs.div_euclid(86_400));
    let sod = secs.rem_euclid(86_400);
    format!(
        "{y:04}-{mo:02}-{d:02}T{:02}:{:02}:{:02}.{:03}Z",
        sod / 3_600,
        (sod / 60) % 60,
        sod % 60,
        unix_ms % 1_000
    )
}

/// `r-YYYYMMDD-HHMM-xxxx` from the controller's Unix minutes and the wire run id.
pub fn run_id(unix_minutes: u32, wire_id: u32) -> String {
    let mins = i64::from(unix_minutes);
    let (y, mo, d) = civil_from_days(mins.div_euclid(1_440));
    let mod_ = mins.rem_euclid(1_440);
    format!(
        "r-{y:04}{mo:02}{d:02}-{:02}{:02}-{:04x}",
        mod_ / 60,
        mod_ % 60,
        wire_id & 0xFFFF
    )
}

/// Seconds between two instants.
pub fn secs(a: Instant, b: Instant) -> f64 {
    b.since(a) as f64 / 1e6
}

/// Scalar fields of the ride recorder's `latency` event line from the media latency
/// export (`Ctx::media_latency`, §12.6): frames played and overall p50/p95 in ms
/// (`null` before the first frame). Event lines carry scalars only (labrun/1).
pub fn latency_fields(v: &serde_json::Value) -> Vec<(String, mesh_lab::json::Json)> {
    use mesh_lab::json::Json;
    let ms = |k: &str| v["overall"][k].as_f64().map_or(Json::Null, Json::from);
    vec![
        (
            "frames".to_string(),
            Json::from(v["frames"].as_u64().unwrap_or(0)),
        ),
        ("p50Ms".to_string(), ms("p50Ms")),
        ("p95Ms".to_string(), ms("p95Ms")),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_and_times() {
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        assert_eq!(civil_from_days(20_749), (2026, 10, 23));
        // 2026-10-23T09:12:03.120Z
        let ms = (20_749u64 * 86_400 + 9 * 3_600 + 12 * 60 + 3) * 1_000 + 120;
        assert_eq!(iso_ms(ms), "2026-10-23T09:12:03.120Z");
        let minutes = (ms / 60_000) as u32;
        assert_eq!(run_id(minutes, 0x1234_ab12), "r-20261023-0912-ab12");
    }

    #[test]
    fn latency_export_becomes_scalar_event_fields() {
        use mesh_lab::json::Json;
        let mut e = crate::media::LatencyEstimator::new();
        let empty = latency_fields(&e.export());
        assert_eq!(empty[0], ("frames".to_string(), Json::Int(0)));
        assert_eq!(empty[1].1, Json::Null);
        e.record(crate::media::LatencySample {
            origin: 3,
            hops: 1,
            m2e_us: 120_000,
        });
        let f = latency_fields(&e.export());
        assert_eq!(f[0].1, Json::Int(1));
        assert_eq!(f[1].1, Json::Num(120.0));
        assert_eq!(f[2].1, Json::Num(120.0));
    }

    #[test]
    fn enums_map_by_id() {
        for b in LabBench::ALL {
            assert_eq!(api_bench(bench(*b)), *b);
        }
        for a in LabAudio::ALL {
            assert_eq!(api_audio(audio(*a)), *a);
        }
        for a in LabAnchor::ALL {
            assert_eq!(api_anchor(anchor(*a)), *a);
        }
        let p = mesh_api::LabParams::new(LabAudio::Full, LabAnchor::AudioLocation, true);
        let l = params(&p);
        assert_eq!(
            (l.audio, l.anchor, l.gps),
            (Audio::Full, Anchor::AudioLocation, true)
        );
    }
}
