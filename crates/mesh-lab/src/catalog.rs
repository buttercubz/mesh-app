//! Bench catalog (DESIGN §12.2): setup, variant axes, default audio path and keep-alive
//! anchor, and default traffic parameters of every bench. Each variant combination is a
//! separate run; [`BenchSpec::variants`] expands the matrix.

use crate::kinds::{Anchor, Audio, Bench, Pattern};

/// Default traffic parameters of a bench (applied to every `LabParams` field left out).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Defaults {
    /// Run length in seconds; 0 = until stopped (ride recorder).
    pub duration_s: u32,
    pub pattern: Pattern,
    /// Voice-like streams (1-4).
    pub streams: u8,
    /// On-air unit size per voice-like probe on a stream link (118 B = one BLE16 VOICE
    /// frame, DESIGN §4.8) or per ping (64 B).
    pub payload_b: u16,
    /// Voice-like frame interval in ms.
    pub interval_ms: u16,
    /// Echoed probes per second (0 = no pings).
    pub ping_hz: u8,
}

/// One variant axis: a key of the `run` line `variant` object and its values.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Axis {
    pub key: &'static str,
    pub values: &'static [&'static str],
}

/// A variant that is not part of the matrix (a later-phase or device-only extra).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Extra {
    pub name: &'static str,
    /// First phase that runs it.
    pub phase: u8,
    pub audio: Audio,
    pub anchor: Anchor,
    pub note: &'static str,
}

/// Catalog entry of one bench.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BenchSpec {
    pub bench: Bench,
    /// First phase in which the bench runs (§14).
    pub phase: u8,
    pub title: &'static str,
    pub setup: &'static str,
    /// Phones needed (min, max).
    pub devices: (u8, u8),
    /// Default audio path (headset-off variants for T0/T0b).
    pub audio: Audio,
    /// Default keep-alive anchor (never `none`).
    pub anchor: Anchor,
    /// Anchors the variants may use (the default first).
    pub anchors: &'static [Anchor],
    pub defaults: Defaults,
    pub axes: &'static [Axis],
    pub extras: &'static [Extra],
    /// Repetitions per variant (T3 trials, T5a click trains, T8 presses).
    pub repetitions: u16,
    /// What is captured per second (§12.2 column), for the Field test UI.
    pub captured: &'static str,
    /// Go / no-go rule (thresholds in `tools/lab-analyze/thresholds.json`).
    pub criterion: &'static str,
}

/// One expanded variant (one run).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Variant {
    pub bench: Bench,
    /// `(axis key, value)` pairs, recorded in the `run` line `variant` object.
    pub values: Vec<(&'static str, &'static str)>,
    pub audio: Audio,
    pub anchor: Anchor,
    pub phase: u8,
}

impl Variant {
    /// The value of axis `key`, if present.
    pub fn get(&self, key: &str) -> Option<&'static str> {
        self.values.iter().find(|(k, _)| *k == key).map(|(_, v)| *v)
    }
}

impl BenchSpec {
    /// Audio path of a variant: `full` when a headset (HFP) is active, else the default.
    pub fn audio_for(&self, headset_on: bool) -> Audio {
        if headset_on { Audio::Full } else { self.audio }
    }

    /// Expands the variant matrix (cartesian product of the axes, in axis order),
    /// followed by the extras. Axis keys `headset` and `anchor` set the audio path and
    /// the anchor of the variant.
    pub fn variants(&self) -> Vec<Variant> {
        let mut out = vec![Vec::new()];
        for axis in self.axes {
            let mut next = Vec::with_capacity(out.len() * axis.values.len());
            for prefix in &out {
                for v in axis.values {
                    let mut p: Vec<(&'static str, &'static str)> = prefix.clone();
                    p.push((axis.key, *v));
                    next.push(p);
                }
            }
            out = next;
        }
        let mut variants: Vec<Variant> = out
            .into_iter()
            .map(|values| {
                let headset_on = values.iter().any(|(k, v)| *k == "headset" && *v != "off");
                let anchor = values
                    .iter()
                    .find(|(k, _)| *k == "anchor")
                    .and_then(|(_, v)| Anchor::parse(v))
                    .unwrap_or(self.anchor);
                Variant {
                    bench: self.bench,
                    audio: self.audio_for(headset_on),
                    anchor,
                    phase: self.phase,
                    values,
                }
            })
            .collect();
        variants.extend(self.extras.iter().map(|e| Variant {
            bench: self.bench,
            values: vec![("extra", e.name)],
            audio: e.audio,
            anchor: e.anchor,
            phase: e.phase,
        }));
        variants
    }

    /// Default traffic of a T0-style `pattern` axis value (`voice1`…`voice4`, `saturate`,
    /// `ping`, `pingUnderLoad`): pattern, streams and duration.
    pub fn pattern_axis(value: &str) -> Option<(Pattern, u8, u32)> {
        Some(match value {
            "saturate" => (Pattern::Saturate, 0, 20),
            "voice1" => (Pattern::VoiceLike, 1, 60),
            "voice2" => (Pattern::VoiceLike, 2, 60),
            "voice3" => (Pattern::VoiceLike, 3, 60),
            "voice4" => (Pattern::VoiceLike, 4, 60),
            "ping" => (Pattern::Ping, 0, 60),
            "pingUnderLoad" => (Pattern::PingUnderLoad, 2, 60),
            _ => return None,
        })
    }
}

const VOICE2: Defaults = Defaults {
    duration_s: 60,
    pattern: Pattern::VoiceLike,
    streams: 2,
    payload_b: 118,
    interval_ms: 40,
    ping_hz: 10,
};

const T0_PATTERNS: &[&str] = &[
    "saturate",
    "voice1",
    "voice2",
    "voice3",
    "voice4",
    "ping",
    "pingUnderLoad",
];
const TRANSPORTS: &[&str] = &["l2cap", "gattNotify", "gattWrite"];
const ROLES: &[&str] = &["central", "peripheral"];
const HEADSET: &[&str] = &["off", "hfp"];
const A_AUDIO_LOC: &[Anchor] = &[Anchor::Audio, Anchor::AudioLocation];
const A_ALL: &[Anchor] = &[Anchor::Audio, Anchor::AudioLocation, Anchor::None];
const A_AUDIO_NONE: &[Anchor] = &[Anchor::Audio, Anchor::None];
const A_AUDIO: &[Anchor] = &[Anchor::Audio];

/// The bench catalog (DESIGN §12.2), in table order, plus the ride recorder.
pub const CATALOG: &[BenchSpec] = &[
    BenchSpec {
        bench: Bench::T0,
        phase: 0,
        title: "BLE throughput / latency / loss",
        setup: "2 iPhones at 1-5 m, then 20 m",
        devices: (2, 2),
        audio: Audio::Off,
        anchor: Anchor::Audio,
        anchors: A_AUDIO,
        defaults: VOICE2,
        axes: &[
            Axis {
                key: "transport",
                values: TRANSPORTS,
            },
            Axis {
                key: "role",
                values: ROLES,
            },
            Axis {
                key: "headset",
                values: HEADSET,
            },
            Axis {
                key: "pattern",
                values: T0_PATTERNS,
            },
            Axis {
                key: "distance",
                values: &["near", "far"],
            },
        ],
        extras: &[],
        repetitions: 1,
        captured: "tx/rx kbps, loss, reorder, stalls > 100 ms, RTT p50/p95/p99, one-way estimate, ATT max write, RSSI, inferred connection interval",
        criterion: ">= 2 voice-like streams at < 5 % loss and p95 one-way < 60 ms with the headset active => S_MAX_BLE = 2; >= 150 kbps => BLE-fast",
    },
    BenchSpec {
        bench: Bench::T0b,
        phase: 0,
        title: "Relay chain",
        setup: "3 iPhones A-B-C, A and C out of direct range (or LAB_CTL blacklist)",
        devices: (3, 3),
        audio: Audio::Off,
        anchor: Anchor::Audio,
        anchors: A_AUDIO,
        defaults: VOICE2,
        axes: &[
            Axis {
                key: "headset",
                values: HEADSET,
            },
            Axis {
                key: "pattern",
                values: &["voice1", "voice2"],
            },
        ],
        extras: &[],
        repetitions: 1,
        captured: "as T0 + per-hop latency via MESH_PROBE trace, B's node airtime",
        criterion: "2 streams relayed at < 5 % loss, added p95 latency per hop <= 40 ms",
    },
    BenchSpec {
        bench: Bench::T0c,
        phase: 0,
        title: "Multi-link hub",
        setup: "4-5 iPhones: hub H with 3, then 4 BLE links + active HFP headset on H",
        devices: (3, 5),
        audio: Audio::Full,
        anchor: Anchor::Audio,
        anchors: A_AUDIO,
        defaults: VOICE2,
        axes: &[
            Axis {
                key: "links",
                values: &["2", "3", "4"],
            },
            Axis {
                key: "pacer",
                values: &["free", "capped"],
            },
        ],
        extras: &[],
        repetitions: 1,
        captured: "per-link and node-total tx kbps, loss, p95 one-way, WouldBlock/drop counters, HFP glitches, thermal",
        criterion: "target 3 / max 4 only if H sustains 3 links x cost(2) (~150 kbps) at < 5 % loss, HFP glitch-free",
    },
    BenchSpec {
        bench: Bench::T1,
        phase: 0,
        title: "Range between vehicles",
        setup: "2-3 vehicles, phones mounted / pocket / tank bag (marked), 30-60 min mixed road",
        devices: (2, 3),
        audio: Audio::RenderOnly,
        anchor: Anchor::Audio,
        anchors: A_AUDIO_LOC,
        defaults: Defaults {
            duration_s: 1800,
            pattern: Pattern::PingUnderLoad,
            streams: 2,
            payload_b: 118,
            interval_ms: 40,
            ping_hz: 5,
        },
        axes: &[
            Axis {
                key: "mount",
                values: &["mounted", "pocket", "tankBag"],
            },
            Axis {
                key: "anchor",
                values: &["audio", "audioLocation"],
            },
        ],
        extras: &[],
        repetitions: 1,
        captured: "own + peer GPS, distance, speed, RSSI, loss, RTT p95, link up/down, reconnect duration, anchor state",
        criterion: "useful separation (>= 95 % of 1 s windows < 5 % loss) >= typical convoy gap; windows whose anchor dropped are excluded",
    },
    BenchSpec {
        bench: Bench::T2,
        phase: 0,
        title: "Background survival",
        setup: "1 iPhone (+ 1 peer for link continuity), session live, phone locked",
        devices: (1, 2),
        audio: Audio::Full,
        anchor: Anchor::Audio,
        anchors: A_ALL,
        defaults: Defaults {
            duration_s: 14_400,
            pattern: Pattern::VoiceLike,
            streams: 1,
            payload_b: 118,
            interval_ms: 40,
            ping_hz: 1,
        },
        axes: &[
            Axis {
                key: "anchor",
                values: &["audio", "audioLocation", "none"],
            },
            Axis {
                key: "session",
                values: &["nonMixable", "mixable"],
            },
        ],
        extras: &[Extra {
            name: "liveActivity8h",
            phase: 0,
            audio: Audio::Full,
            anchor: Anchor::Audio,
            note: "long run past the 8 h Live Activity limit",
        }],
        repetitions: 1,
        captured: "capture/render fps, VPIO running, interruptions, route changes, ladder, BLE links up, thermal, battery %, app state",
        criterion: "0 silent mutes (every capture stop produced CaptureLost + notification); automatic recovery or one guided tap",
    },
    BenchSpec {
        bench: Bench::T3,
        phase: 0,
        title: "Rediscovery",
        setup: "2 iPhones (3 for relay rejoin); break by LAB_CTL blacklist for 30 s and by walking apart",
        devices: (2, 3),
        audio: Audio::RenderOnly,
        anchor: Anchor::Audio,
        anchors: A_AUDIO_NONE,
        defaults: Defaults {
            duration_s: 300,
            pattern: Pattern::Ping,
            streams: 0,
            payload_b: 64,
            interval_ms: 40,
            ping_hz: 5,
        },
        axes: &[
            Axis {
                key: "state",
                values: &[
                    "bothFg",
                    "oneBg",
                    "bothBgScreenOff",
                    "bothBgLiveActivity",
                    "pendingConnect",
                    "behindEpoch",
                ],
            },
            Axis {
                key: "break",
                values: &["blacklist30s", "walkApart"],
            },
            Axis {
                key: "anchor",
                values: &["audio", "none"],
            },
        ],
        extras: &[Extra {
            name: "androidOverflowBitmask",
            phase: 3,
            audio: Audio::RenderOnly,
            anchor: Anchor::Audio,
            note: "[DEV] Android scanner detects a backgrounded iPhone via the overflow bitmask",
        }],
        repetitions: 5,
        captured: "time to adv seen, connect, HELLO, handshake, first probe; app, screen and anchor state",
        criterion: "rejoin <= 10 s in the variants the product uses",
    },
    BenchSpec {
        bench: Bench::T5a,
        phase: 0,
        title: "Headset loopback latency",
        setup: "1 iPhone + helmet; helmet speaker against helmet mic in a quiet room; VPIO BypassVoiceProcessing = 1",
        devices: (1, 1),
        audio: Audio::Full,
        anchor: Anchor::Audio,
        anchors: A_AUDIO,
        defaults: Defaults {
            duration_s: 180,
            pattern: Pattern::Idle,
            streams: 0,
            payload_b: 0,
            interval_ms: 2_000,
            ping_hz: 0,
        },
        axes: &[Axis {
            key: "hfp",
            values: &["wideband", "narrowband"],
        }],
        extras: &[],
        repetitions: 50,
        captured: "MLS click train round trip (OS + both HFP legs), per-leg estimate",
        criterion: "per-leg estimate fits the 25 ms/leg budget",
    },
    BenchSpec {
        bench: Bench::T5b,
        phase: 1,
        title: "Headset behaviour",
        setup: "1-2 iPhones + each helmet model the user owns",
        devices: (1, 2),
        audio: Audio::Full,
        anchor: Anchor::Audio,
        anchors: A_AUDIO,
        defaults: Defaults {
            duration_s: 600,
            pattern: Pattern::Idle,
            streams: 0,
            payload_b: 0,
            interval_ms: 2_000,
            ping_hz: 0,
        },
        axes: &[Axis {
            key: "check",
            values: &["echoReturn", "ducking", "buttons", "helmetIntercom"],
        }],
        extras: &[],
        repetitions: 20,
        captured: "echo return loss (click train, VPIO on), rider-marked artefacts, route changes, glitches, button events",
        criterion: "per-model note in the Phase 1 report",
    },
    BenchSpec {
        bench: Bench::T6,
        phase: 0,
        title: "Battery and thermal",
        setup: "1-2 iPhones on a real ride, 2-4 h (Phase 0: >= 1 h per variant)",
        devices: (1, 2),
        audio: Audio::Full,
        anchor: Anchor::Audio,
        anchors: A_AUDIO_LOC,
        defaults: Defaults {
            duration_s: 3_600,
            pattern: Pattern::VoiceLike,
            streams: 2,
            payload_b: 118,
            interval_ms: 40,
            ping_hz: 1,
        },
        axes: &[
            Axis {
                key: "screen",
                values: &["off", "offNav", "onMount"],
            },
            Axis {
                key: "charging",
                values: &["no", "yes"],
            },
            Axis {
                key: "anchor",
                values: &["audio", "audioLocation"],
            },
        ],
        extras: &[],
        repetitions: 1,
        captured: "battery %/h, thermal state, throttling events, links, ladder",
        criterion: "records %/h; proposed target < 5 %/h screen off",
    },
    BenchSpec {
        bench: Bench::T8,
        phase: 1,
        title: "UI and PTT input latency",
        setup: "1 iPhone, Demo world with 15 simulated peers",
        devices: (1, 1),
        audio: Audio::Full,
        anchor: Anchor::Audio,
        anchors: A_AUDIO,
        defaults: Defaults {
            duration_s: 300,
            pattern: Pattern::Idle,
            streams: 0,
            payload_b: 0,
            interval_ms: 40,
            ping_hz: 0,
        },
        axes: &[],
        extras: &[],
        repetitions: 100,
        captured: "touch timestamp in JS, native receipt, mesh_rt_set_flags time, WebView frame times",
        criterion: "touch -> mesh_rt_set_flags p95 < 50 ms and no dropped frames > 50 ms",
    },
    BenchSpec {
        bench: Bench::VoxCal,
        phase: 1,
        title: "VOX calibration",
        setup: "1 rider with helmet riding; PTT held while actually speaking (passenger or at stops)",
        devices: (1, 1),
        audio: Audio::Full,
        anchor: Anchor::Audio,
        anchors: A_AUDIO,
        defaults: Defaults {
            duration_s: 1_800,
            pattern: Pattern::Idle,
            streams: 0,
            payload_b: 0,
            interval_ms: 40,
            ping_hz: 0,
        },
        axes: &[
            Axis {
                key: "sensitivity",
                values: &["low", "medium", "high"],
            },
            Axis {
                key: "wind",
                values: &["calm", "windy"],
            },
        ],
        extras: &[],
        repetitions: 1,
        captured: "VAD decisions vs PTT ground truth, p, noise floor; optional consented 60 s clips (local only)",
        criterion: "false triggers <= 1/min at 80 km/h and onset delay <= 150 ms at Medium",
    },
    BenchSpec {
        bench: Bench::T4,
        phase: 4,
        title: "Wi-Fi Aware",
        setup: "2-3 paired iPhones",
        devices: (2, 3),
        audio: Audio::Off,
        anchor: Anchor::Audio,
        anchors: A_AUDIO,
        defaults: Defaults {
            duration_s: 60,
            pattern: Pattern::VoiceLike,
            streams: 4,
            payload_b: 88,
            interval_ms: 20,
            ping_hz: 10,
        },
        axes: &[
            Axis {
                key: "mode",
                values: &["realtime", "bulk"],
            },
            Axis {
                key: "topology",
                values: &["singleRole", "bothRoles"],
            },
        ],
        extras: &[
            Extra {
                name: "batteryRealtime",
                phase: 4,
                audio: Audio::Full,
                anchor: Anchor::Audio,
                note: "battery %/h with a realtime Aware link (1 h)",
            },
            Extra {
                name: "batteryBleOnly",
                phase: 4,
                audio: Audio::Full,
                anchor: Anchor::Audio,
                note: "battery %/h BLE-only baseline for the opt-in copy (1 h)",
            },
        ],
        repetitions: 1,
        captured: "as T0 + max connectable devices + battery",
        criterion: "upgrade is worth it if >= 4 streams at < 2 % loss; battery cost recorded",
    },
    BenchSpec {
        bench: Bench::Ride,
        phase: 2,
        title: "Ride recorder",
        setup: "real rides, if enabled",
        devices: (1, 15),
        audio: Audio::Full,
        anchor: Anchor::Audio,
        anchors: A_AUDIO_LOC,
        defaults: Defaults {
            duration_s: 0,
            pattern: Pattern::Idle,
            streams: 0,
            payload_b: 0,
            interval_ms: 40,
            ping_hz: 0,
        },
        axes: &[],
        extras: &[],
        repetitions: 1,
        captured: "1 Hz samples of all links, reachability, floor events, audio counters, GPS (if permitted)",
        criterion: "none (feeds T1/T2 data)",
    },
];

/// The catalog entry of `bench`.
pub fn spec(bench: Bench) -> &'static BenchSpec {
    // Every Bench has exactly one entry (checked by the tests).
    CATALOG
        .iter()
        .find(|s| s.bench == bench)
        .unwrap_or(&CATALOG[0])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_bench_has_one_entry() {
        for b in Bench::ALL {
            assert_eq!(CATALOG.iter().filter(|s| s.bench == *b).count(), 1, "{b}");
            assert_eq!(spec(*b).bench, *b);
        }
    }

    #[test]
    fn defaults_match_design_12_2() {
        // T1/T3 default anchor `audio`; T1 renderOnly, T3 renderOnly.
        assert_eq!(spec(Bench::T1).anchor, Anchor::Audio);
        assert_eq!(spec(Bench::T1).audio, Audio::RenderOnly);
        assert_eq!(spec(Bench::T3).anchor, Anchor::Audio);
        assert_eq!(spec(Bench::T3).audio, Audio::RenderOnly);
        // T0 headset-on, T2, T5a, T5b, T6, T8, VOX-cal use audio `full`.
        assert_eq!(spec(Bench::T0).audio_for(true), Audio::Full);
        assert_eq!(spec(Bench::T0).audio_for(false), Audio::Off);
        for b in [
            Bench::T0c,
            Bench::T2,
            Bench::T5a,
            Bench::T5b,
            Bench::T6,
            Bench::T8,
            Bench::VoxCal,
        ] {
            assert_eq!(spec(b).audio, Audio::Full, "{b}");
        }
        // No bench defaults to the `none` control anchor.
        assert!(CATALOG.iter().all(|s| s.anchor != Anchor::None));
        assert!(CATALOG.iter().all(|s| s.anchors.first() == Some(&s.anchor)));
        // Voice-like probes are exactly one BLE16 VOICE frame on air.
        let v = mesh_wire::budget::voice_sizes(80, mesh_wire::budget::Framing::Stream);
        assert_eq!(usize::from(spec(Bench::T0).defaults.payload_b), v.on_air);
        let w = mesh_wire::budget::voice_sizes(50, mesh_wire::budget::Framing::Datagram);
        assert_eq!(
            usize::from(spec(Bench::T4).defaults.payload_b),
            w.on_air + 2
        );
    }

    #[test]
    fn variant_matrix() {
        let t0 = spec(Bench::T0).variants();
        assert_eq!(t0.len(), 3 * 2 * 2 * 7 * 2);
        for v in &t0 {
            let headset = v.get("headset").unwrap();
            assert_eq!(v.audio == Audio::Full, headset == "hfp");
            assert_eq!(v.anchor, Anchor::Audio);
            assert!(BenchSpec::pattern_axis(v.get("pattern").unwrap()).is_some());
        }
        let t2 = spec(Bench::T2).variants();
        assert_eq!(t2.len(), 3 * 2 + 1);
        assert!(t2.iter().any(|v| v.anchor == Anchor::None));
        let t3 = spec(Bench::T3).variants();
        let android = t3
            .iter()
            .find(|v| v.get("extra") == Some("androidOverflowBitmask"))
            .unwrap();
        assert_eq!(android.phase, 3);
        let t4 = spec(Bench::T4).variants();
        assert!(t4.iter().any(|v| v.get("extra") == Some("batteryRealtime")));
        assert_eq!(spec(Bench::T8).variants().len(), 1);
        assert_eq!(
            BenchSpec::pattern_axis("voice4"),
            Some((Pattern::VoiceLike, 4, 60))
        );
        assert_eq!(BenchSpec::pattern_axis("nope"), None);
    }
}
