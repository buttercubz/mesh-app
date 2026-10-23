//! The `hdr` line (first line of every segment) and the `end` line.

use super::RIDELOG_SCHEMA_ID;
use super::event_fields::hex4;
use crate::json::Json;

/// Device identity written in every `hdr` (no names, no MAC addresses).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceInfo {
    /// `ios`, `android`, `web` or `sim`.
    pub platform: String,
    pub model: String,
    pub os: String,
    /// App version and build (`0.1.0 (12)`).
    pub app: String,
    /// `debug` or `release`.
    pub build: String,
    /// `ffi`, `wasm` or `sim`.
    pub engine: String,
}

/// Values of the `hdr` line that stay the same across the segments of one session
/// (identity fields are filled in once the core reports them).
#[derive(Debug, Clone, PartialEq)]
pub struct Header {
    pub session_id: String,
    /// ISO-8601 UTC wall time of the session start (`2026-10-22T20:04:11.512Z`).
    pub started_at: String,
    /// The runtime's monotonic clock at the session start (µs).
    pub mono_us: u64,
    pub device: DeviceInfo,
    pub ride_key: Option<String>,
    pub node: Option<u16>,
    pub role: Option<String>,
    /// Platform capabilities (`l2cap`, `aware`, …): scalars only.
    pub caps: Vec<(String, Json)>,
    /// Settings snapshot (`ptt`, `builtInMicTx`, …): scalars only.
    pub settings: Vec<(String, Json)>,
    /// Verbose per-frame lines were on when the segment started.
    pub verbose: bool,
}

impl Header {
    /// A header with only the mandatory values.
    pub fn new(session_id: &str, started_at: &str, mono_us: u64, device: DeviceInfo) -> Self {
        Header {
            session_id: session_id.to_string(),
            started_at: started_at.to_string(),
            mono_us,
            device,
            ride_key: None,
            node: None,
            role: None,
            caps: Vec::new(),
            settings: Vec::new(),
            verbose: false,
        }
    }

    /// Sets the ride key from its 4 bytes.
    pub fn set_ride_key(&mut self, k: [u8; 4]) {
        self.ride_key = Some(hex4(&k));
    }

    /// The JSON line for segment `seg` (1-based).
    pub fn to_json(&self, seg: u32) -> Json {
        let d = &self.device;
        let device = Json::obj()
            .with("platform", d.platform.as_str())
            .with("model", d.model.as_str())
            .with("os", d.os.as_str())
            .with("app", d.app.as_str())
            .with("build", d.build.as_str())
            .with("engine", d.engine.as_str());
        Json::obj()
            .with("t", "hdr")
            .with("v", 1u8)
            .with("schema", RIDELOG_SCHEMA_ID)
            .with("seg", seg)
            .with("sessionId", self.session_id.as_str())
            .with("rideKey", self.ride_key.clone())
            .with("startedAt", self.started_at.as_str())
            .with("monoUs", self.mono_us)
            .with("device", device)
            .with("node", self.node)
            .with("role", self.role.clone())
            .with("caps", Json::Obj(self.caps.clone()))
            .with("settings", Json::Obj(self.settings.clone()))
            .with(
                "privacy",
                Json::obj().with("gps", "off").with("names", "none"),
            )
            .with("verbose", self.verbose)
    }
}

/// Why a segment ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EndReason {
    Rotate,
    SessionEnd,
    AppTerminate,
}

impl EndReason {
    pub const fn as_str(self) -> &'static str {
        match self {
            EndReason::Rotate => "rotate",
            EndReason::SessionEnd => "sessionEnd",
            EndReason::AppTerminate => "appTerminate",
        }
    }
}

/// Session id `d-YYYYMMDD-HHMM-xxxx` from a UTC wall time (ms since the Unix epoch)
/// and 20 random bits supplied by the caller.
pub fn session_id(wall_ms: u64, random: u32) -> String {
    let (y, mo, d, h, mi, _, _) = civil(wall_ms);
    const B36: &[u8; 36] = b"0123456789abcdefghijklmnopqrstuvwxyz";
    let mut r = random;
    let mut tail = [0u8; 4];
    for c in tail.iter_mut() {
        *c = B36[(r % 36) as usize];
        r /= 36;
    }
    let tail: String = tail.iter().map(|&c| c as char).collect();
    format!("d-{y:04}{mo:02}{d:02}-{h:02}{mi:02}-{tail}")
}

/// ISO-8601 UTC (`2026-10-22T20:04:11.512Z`) of a wall time in ms.
pub fn iso8601(wall_ms: u64) -> String {
    let (y, mo, d, h, mi, s, ms) = civil(wall_ms);
    format!("{y:04}-{mo:02}-{d:02}T{h:02}:{mi:02}:{s:02}.{ms:03}Z")
}

/// UTC date `YYYYMMDD` as a number (the ride-key salt).
pub fn utc_date(wall_ms: u64) -> u32 {
    let (y, mo, d, ..) = civil(wall_ms);
    y * 10_000 + mo * 100 + d
}

/// (year, month, day, hour, minute, second, ms) of a Unix time in ms (proleptic
/// Gregorian, days-from-civil inverse by H. Hinnant).
fn civil(wall_ms: u64) -> (u32, u32, u32, u32, u32, u32, u32) {
    let secs = wall_ms / 1000;
    let ms = (wall_ms % 1000) as u32;
    let days = (secs / 86_400) as i64;
    let sod = (secs % 86_400) as u32;
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let y = (yoe + era * 400 + i64::from(m <= 2)) as u32;
    (y, m, d, sod / 3600, sod % 3600 / 60, sod % 60, ms)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn civil_time() {
        // 2026-10-22T20:04:11.512Z
        let t = 1_792_699_451_512;
        assert_eq!(iso8601(t), "2026-10-22T20:04:11.512Z");
        assert_eq!(utc_date(t), 20261022);
        assert_eq!(session_id(t, 0), "d-20261022-2004-0000");
        assert_eq!(iso8601(0), "1970-01-01T00:00:00.000Z");
        assert_eq!(iso8601(951_782_400_000), "2000-02-29T00:00:00.000Z");
    }
}
