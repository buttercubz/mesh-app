//! GPS helpers: LAB_GPS conversion and great-circle distance (T1 `distM`).

use mesh_wire::record::LabGps;

use crate::record::GpsFix;

/// Mean Earth radius (m).
const EARTH_RADIUS_M: f64 = 6_371_008.8;

/// Haversine distance in metres.
pub fn distance_m(a: &GpsFix, b: &GpsFix) -> f64 {
    let (la1, la2) = (a.lat.to_radians(), b.lat.to_radians());
    let dla = la2 - la1;
    let dlo = (b.lon - a.lon).to_radians();
    let h = (dla / 2.0).sin().powi(2) + la1.cos() * la2.cos() * (dlo / 2.0).sin().powi(2);
    2.0 * EARTH_RADIUS_M * h.sqrt().min(1.0).asin()
}

/// A fix as the LAB_GPS record broadcast at 1 Hz (`t_ms` = run milliseconds).
pub fn to_lab_gps(fix: &GpsFix, t_ms: u32, heading_deg: Option<f64>) -> LabGps {
    let e7 = |v: f64| {
        (v * 1e7)
            .round()
            .clamp(f64::from(i32::MIN), f64::from(i32::MAX)) as i32
    };
    let sat16 = |v: f64| v.round().clamp(0.0, f64::from(u16::MAX)) as u16;
    LabGps {
        t_ms,
        lat_e7: e7(fix.lat),
        lon_e7: e7(fix.lon),
        speed_cms: sat16(fix.spd.unwrap_or(0.0) * 100.0),
        acc_dm: sat16(fix.acc.unwrap_or(0.0) * 10.0),
        heading_cdeg: sat16(heading_deg.unwrap_or(0.0).rem_euclid(360.0) * 100.0),
    }
}

/// A received LAB_GPS as a fix.
pub fn from_lab_gps(g: &LabGps) -> GpsFix {
    GpsFix {
        lat: f64::from(g.lat_e7) / 1e7,
        lon: f64::from(g.lon_e7) / 1e7,
        spd: Some(f64::from(g.speed_cms) / 100.0),
        acc: Some(f64::from(g.acc_dm) / 10.0),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fix(lat: f64, lon: f64) -> GpsFix {
        GpsFix {
            lat,
            lon,
            spd: Some(12.5),
            acc: Some(4.8),
        }
    }

    #[test]
    fn distances() {
        // 0.001° of latitude ≈ 111.2 m.
        let d = distance_m(&fix(18.4861, -69.9312), &fix(18.4871, -69.9312));
        assert!((d - 111.2).abs() < 0.5, "{d}");
        assert_eq!(distance_m(&fix(1.0, 2.0), &fix(1.0, 2.0)), 0.0);
        // Santo Domingo → Santiago (DR) ≈ 127 km.
        let far = distance_m(&fix(18.4861, -69.9312), &fix(19.4517, -70.6970));
        assert!((far - 135_000.0).abs() < 10_000.0, "{far}");
    }

    #[test]
    fn lab_gps_round_trip() {
        let f = fix(18.4861234, -69.9312345);
        let g = to_lab_gps(&f, 1_000, Some(-90.0));
        assert_eq!(g.heading_cdeg, 27_000);
        assert_eq!(g.speed_cms, 1_250);
        let back = from_lab_gps(&g);
        assert!((back.lat - f.lat).abs() < 1e-7 && (back.lon - f.lon).abs() < 1e-7);
        assert_eq!(back.acc, Some(4.8));
    }
}
