//! The on-device verdict thresholds equal `tools/lab-analyze/thresholds.json`.

use mesh_lab::verdict::THRESHOLDS;
use serde_json::Value;

#[test]
fn rust_thresholds_match_lab_analyze() {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../tools/lab-analyze/thresholds.json"
    );
    let j: Value = serde_json::from_str(&std::fs::read_to_string(path).expect("thresholds.json"))
        .expect("JSON");
    let f = |bench: &str, key: &str| {
        j[bench][key]
            .as_f64()
            .unwrap_or_else(|| panic!("{bench}.{key}"))
    };
    let t = THRESHOLDS;
    assert_eq!(j["version"], 1);
    for b in ["T0", "T0b", "T0c", "T1"] {
        assert_eq!(f(b, "maxLoss"), t.max_loss, "{b}");
    }
    assert_eq!(f("T0", "owdP95Ms"), t.owd_p95_ms);
    assert_eq!(f("T0c", "owdP95Ms"), t.owd_p95_ms);
    assert_eq!(f("T0c", "maxXruns"), 0.0);
    assert_eq!(f("T0", "minStreams"), f64::from(t.min_streams));
    assert_eq!(f("T0", "bleFastKbps"), t.ble_fast_kbps);
    assert_eq!(f("T0b", "hopP95Ms"), t.hop_p95_ms);
    assert_eq!(f("T1", "windowOkShare"), t.window_ok_share);
    assert_eq!(f("T1", "minWindows"), f64::from(t.min_windows));
    assert_eq!(f("T2", "maxSilentMutes"), 0.0);
    assert_eq!(f("T3", "rejoinS"), t.rejoin_s);
    assert_eq!(f("T4", "streams"), f64::from(t.aware_streams));
    assert_eq!(f("T4", "maxLoss"), t.aware_max_loss);
    assert_eq!(f("T5a", "legMs"), t.leg_ms);
    assert_eq!(f("T6", "batteryPctPerH"), t.battery_pct_h);
    assert_eq!(f("T6", "minDurationS"), t.battery_min_s);
}
