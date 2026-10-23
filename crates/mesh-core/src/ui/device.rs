//! Device and audio state reported by the platform (route, IO state, 1 Hz samples,
//! wall clock), read by the snapshot builder and the Lab samples.

use mesh_api::{AudioRouteKind, ThermalState};

use crate::{AudioIoState, AudioRouteInfo, Instant, PlatformSample, Thermal};

/// Platform-reported state.
#[derive(Debug, Clone, PartialEq)]
pub struct DeviceState {
    pub route: AudioRouteInfo,
    pub io: AudioIoState,
    /// Latest 1 Hz sample.
    pub sample: Option<PlatformSample>,
    /// Smoothed capture level 0..1 (RMS, full scale 1).
    pub input_level: f64,
    /// `(instant, unix ms)` pair anchoring the wall clock.
    wall_base: (Instant, u64),
    /// Capture blocks seen (Lab counters).
    pub capture_blocks: u64,
}

impl DeviceState {
    pub fn new(now: Instant, wall_ms: u64) -> Self {
        DeviceState {
            route: AudioRouteInfo::NONE,
            io: AudioIoState::Stopped,
            sample: None,
            input_level: 0.0,
            wall_base: (now, wall_ms),
            capture_blocks: 0,
        }
    }

    /// Wall clock (Unix ms) at `now`.
    pub fn wall_ms(&self, now: Instant) -> u64 {
        let (t, w) = self.wall_base;
        if now >= t {
            w + now.since(t) / 1_000
        } else {
            w.saturating_sub(t.since(now) / 1_000)
        }
    }

    /// Stores a sample; re-anchors the wall clock when it carries one.
    pub fn on_sample(&mut self, now: Instant, s: PlatformSample) {
        if let Some(w) = s.wall_ms {
            self.wall_base = (now, w);
        }
        self.sample = Some(s);
    }

    /// Capture is delivering (IO running or restored).
    pub fn capture_ok(&self) -> bool {
        matches!(self.io, AudioIoState::Running | AudioIoState::Restored)
    }

    /// Render is delivering.
    pub fn render_ok(&self) -> bool {
        matches!(
            self.io,
            AudioIoState::Running | AudioIoState::Restored | AudioIoState::CaptureLost
        )
    }

    /// Route kind for the UI.
    pub fn route_kind(&self) -> AudioRouteKind {
        self.route.kind
    }

    /// Updates the input level from one 10 ms block (allocation-free). Returns `true`
    /// when the change is visible in the snapshot (≥ 0.02).
    pub fn on_capture_block(&mut self, pcm: &[i16]) -> bool {
        self.capture_blocks += 1;
        let mut sum = 0f64;
        for &s in pcm {
            sum += f64::from(s) * f64::from(s);
        }
        let rms = if pcm.is_empty() {
            0.0
        } else {
            (sum / pcm.len() as f64).sqrt() / 32_768.0
        };
        let before = self.input_level;
        self.input_level = 0.8 * self.input_level + 0.2 * rms;
        (quantize(before) - quantize(self.input_level)).abs() >= 0.02
    }

    /// Snapshot thermal state (`nominal` when unknown).
    pub fn thermal(&self) -> ThermalState {
        match self.sample.as_ref().and_then(|s| s.thermal) {
            Some(Thermal::Fair) => ThermalState::Fair,
            Some(Thermal::Serious) => ThermalState::Serious,
            Some(Thermal::Critical) => ThermalState::Critical,
            Some(Thermal::Nominal) | None => ThermalState::Nominal,
        }
    }

    /// labrun/1 thermal string.
    pub fn thermal_str(&self) -> &'static str {
        match self.sample.as_ref().and_then(|s| s.thermal) {
            None => "unknown",
            Some(_) => self.thermal().as_str(),
        }
    }
}

/// Rounds a level to 2 decimals (snapshot resolution).
pub fn quantize(v: f64) -> f64 {
    (v * 100.0).round() / 100.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wall_clock_and_level() {
        let mut d = DeviceState::new(Instant(1_000_000), 1_700_000_000_000);
        assert_eq!(d.wall_ms(Instant(3_000_000)), 1_700_000_002_000);
        d.on_sample(
            Instant(5_000_000),
            PlatformSample {
                wall_ms: Some(1_800_000_000_000),
                ..PlatformSample::default()
            },
        );
        assert_eq!(d.wall_ms(Instant(6_000_000)), 1_800_000_001_000);
        assert!(!d.on_capture_block(&[0; 160]));
        assert!(d.on_capture_block(&[16_000; 160]));
        assert!(d.input_level > 0.05);
        assert_eq!(d.thermal_str(), "unknown");
        assert!(!d.capture_ok());
    }
}
