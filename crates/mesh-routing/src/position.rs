//! Position math for the distance list (DESIGN §5.8): when a member's POSITION is newer
//! than 30 s and my own fix is valid, the snapshot carries the haversine distance, the
//! initial bearing and "ahead/behind" relative to my heading.

use mesh_wire::mesh::Position;

use crate::consts::POSITION_FRESH_US;
use crate::types::Instant;

/// Mean Earth radius (IUGG), metres.
pub const EARTH_RADIUS_M: f64 = 6_371_008.8;

/// A position fix.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Fix {
    pub lat_deg: f64,
    pub lon_deg: f64,
    /// Heading in degrees clockwise from north, if moving.
    pub heading_deg: Option<f64>,
    /// When the fix was taken (local clock).
    pub at: Instant,
}

impl Fix {
    /// From a POSITION payload received at `rx` (its `fix_age_100ms` is subtracted).
    pub fn from_position(p: &Position, rx: Instant) -> Self {
        let heading = (p.speed_dms > 0).then(|| f64::from(p.heading_2deg) * 2.0);
        Fix {
            lat_deg: f64::from(p.lat_e7) / 1e7,
            lon_deg: f64::from(p.lon_e7) / 1e7,
            heading_deg: heading,
            at: Instant(rx.us().saturating_sub(u64::from(p.fix_age_100ms) * 100_000)),
        }
    }

    /// `true` when the coordinates are on Earth.
    pub fn valid(&self) -> bool {
        self.lat_deg.is_finite()
            && self.lon_deg.is_finite()
            && (-90.0..=90.0).contains(&self.lat_deg)
            && (-180.0..=180.0).contains(&self.lon_deg)
    }
}

/// Where a member is relative to me.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RelativePosition {
    pub distance_m: f64,
    /// Initial great-circle bearing from me to them, degrees [0, 360).
    pub bearing_deg: f64,
    /// `Some(true)` ahead, `Some(false)` behind, `None` without my heading.
    pub ahead: Option<bool>,
}

/// Haversine distance in metres.
pub fn haversine_m(a_lat: f64, a_lon: f64, b_lat: f64, b_lon: f64) -> f64 {
    let (p1, p2) = (a_lat.to_radians(), b_lat.to_radians());
    let dp = (b_lat - a_lat).to_radians();
    let dl = (b_lon - a_lon).to_radians();
    let h = (dp / 2.0).sin().powi(2) + p1.cos() * p2.cos() * (dl / 2.0).sin().powi(2);
    2.0 * EARTH_RADIUS_M * h.sqrt().min(1.0).asin()
}

/// Initial bearing in degrees [0, 360).
pub fn bearing_deg(a_lat: f64, a_lon: f64, b_lat: f64, b_lon: f64) -> f64 {
    let (p1, p2) = (a_lat.to_radians(), b_lat.to_radians());
    let dl = (b_lon - a_lon).to_radians();
    let y = dl.sin() * p2.cos();
    let x = p1.cos() * p2.sin() - p1.sin() * p2.cos() * dl.cos();
    (y.atan2(x).to_degrees() + 360.0) % 360.0
}

/// Smallest absolute angle between two headings, degrees [0, 180].
pub fn angle_diff_deg(a: f64, b: f64) -> f64 {
    let d = (a - b).rem_euclid(360.0);
    if d > 180.0 { 360.0 - d } else { d }
}

/// The relative position of `them` seen from `me`, or `None` when their fix is older
/// than 30 s or either fix is invalid.
pub fn relative(now: Instant, me: &Fix, them: &Fix) -> Option<RelativePosition> {
    if !me.valid() || !them.valid() || now.since(them.at) >= POSITION_FRESH_US {
        return None;
    }
    let distance_m = haversine_m(me.lat_deg, me.lon_deg, them.lat_deg, them.lon_deg);
    let bearing = bearing_deg(me.lat_deg, me.lon_deg, them.lat_deg, them.lon_deg);
    let ahead = me.heading_deg.map(|h| angle_diff_deg(bearing, h) < 90.0);
    Some(RelativePosition {
        distance_m,
        bearing_deg: bearing,
        ahead,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fix(lat: f64, lon: f64, heading: Option<f64>) -> Fix {
        Fix {
            lat_deg: lat,
            lon_deg: lon,
            heading_deg: heading,
            at: Instant::from_secs(100),
        }
    }

    #[test]
    fn distances_and_bearings() {
        // One degree of latitude ≈ 111.195 km.
        let d = haversine_m(0.0, 0.0, 1.0, 0.0);
        assert!((d - 111_195.0).abs() < 5.0, "{d}");
        // Madrid → Barcelona ≈ 505 km, initial bearing ≈ 75.8°.
        let d = haversine_m(40.4168, -3.7038, 41.3874, 2.1686);
        assert!((d - 505_000.0).abs() < 3_000.0, "{d}");
        let b = bearing_deg(40.4168, -3.7038, 41.3874, 2.1686);
        assert!((b - 75.75).abs() < 0.1, "{b}");
        assert!((bearing_deg(0.0, 0.0, -1.0, 0.0) - 180.0).abs() < 1e-9);
        assert!((bearing_deg(0.0, 0.0, 0.0, -1.0) - 270.0).abs() < 1e-9);
        assert_eq!(haversine_m(10.0, 10.0, 10.0, 10.0), 0.0);
        assert_eq!(angle_diff_deg(350.0, 10.0), 20.0);
    }

    #[test]
    fn ahead_behind_and_freshness() {
        let now = Instant::from_secs(110);
        let me = fix(0.0, 0.0, Some(0.0)); // heading north
        let north = fix(0.001, 0.0, None);
        let south = fix(-0.001, 0.0, None);
        let r = relative(now, &me, &north).expect("rel");
        assert_eq!(r.ahead, Some(true));
        assert!((r.distance_m - 111.2).abs() < 0.5);
        assert_eq!(
            relative(now, &me, &south).and_then(|r| r.ahead),
            Some(false)
        );
        let still = fix(0.0, 0.0, None);
        assert_eq!(relative(now, &still, &north).and_then(|r| r.ahead), None);
        // Older than 30 s, or invalid: nothing.
        assert!(relative(Instant::from_secs(130), &me, &north).is_none());
        assert!(relative(now, &fix(95.0, 0.0, None), &north).is_none());
    }

    #[test]
    fn from_wire() {
        let p = Position {
            lat_e7: 404_168_000,
            lon_e7: -37_038_000,
            speed_dms: 100,
            heading_2deg: 45,
            acc_m: 5,
            fix_age_100ms: 10,
            rsv: 0,
        };
        let f = Fix::from_position(&p, Instant::from_secs(10));
        assert!((f.lat_deg - 40.4168).abs() < 1e-9);
        assert_eq!(f.heading_deg, Some(90.0));
        assert_eq!(f.at, Instant::from_secs(9));
        let stopped = Position { speed_dms: 0, ..p };
        assert_eq!(
            Fix::from_position(&stopped, Instant::from_secs(10)).heading_deg,
            None
        );
    }
}
