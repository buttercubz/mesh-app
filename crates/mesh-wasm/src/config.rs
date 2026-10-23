//! The scenario JSON `mw_world_new` takes (DESIGN §11.2 `{preset, nodes, loss, seed}`,
//! the `?sim=chain5&loss=0.1` query parameters of the web build).
//!
//! Every field is optional. Unknown fields are refused so a typo in a query parameter
//! does not silently fall back to a default.

use mesh_sim::Preset;
use serde_json::{Map, Value};

/// Default geometry of the browser simulator: the local phone plus 6 bots riding the
/// convoy road (§11.2 "local node 0 + 6 bots on a looping road scenario").
pub const DEFAULT_PRESET: Preset = Preset::Convoy15;
/// Default number of phones (local + 6 bots).
pub const DEFAULT_NODES: usize = 7;
/// Largest world the browser simulator accepts (the biggest §13.2 scenario).
pub const MAX_NODES: usize = 15;
/// Default mean pause between two bursts of one bot (seconds).
pub const DEFAULT_BOT_TALK_MEAN_S: f64 = 12.0;
/// Default mean time a roaming bot stays in range before it drops out (seconds).
pub const DEFAULT_ROAM_MEAN_S: f64 = 60.0;

/// A parsed scenario.
#[derive(Debug, Clone, PartialEq)]
pub struct ConvoyConfig {
    pub preset: Preset,
    /// Phones in the world, the local one included (2..=15).
    pub nodes: usize,
    /// Extra per-transmission loss in `[0, 1]`.
    pub loss: Option<f64>,
    pub seed: u64,
    /// Wall clock at world creation (ms since the Unix epoch): Lab run ids and
    /// `startedAt` need one and the sans-IO core has none.
    pub wall_ms: u64,
    /// Bots talk on their own random schedule (`false` in tests that script them).
    pub bots_talk: bool,
    /// Mean pause between two bursts of one bot (seconds).
    pub bot_talk_mean_s: f64,
    /// Pairs of phones that never hear each other (a radio shadow): forces a topology,
    /// e.g. `[[0,2]]` turns a 3-phone `pair` into the line 0-1-2.
    pub shadow: Vec<(usize, usize)>,
    /// Bots drop out of range (and come back) on their own random schedule.
    pub roam: bool,
    /// Mean time a roaming bot stays in range before it drops out (seconds).
    pub roam_mean_s: f64,
}

impl Default for ConvoyConfig {
    fn default() -> Self {
        ConvoyConfig {
            preset: DEFAULT_PRESET,
            nodes: DEFAULT_NODES,
            loss: None,
            seed: 1,
            wall_ms: 1_793_000_000_000,
            bots_talk: true,
            bot_talk_mean_s: DEFAULT_BOT_TALK_MEAN_S,
            shadow: Vec::new(),
            roam: false,
            roam_mean_s: DEFAULT_ROAM_MEAN_S,
        }
    }
}

/// Why a scenario was refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigError(pub String);

impl core::fmt::Display for ConfigError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(&self.0)
    }
}

fn err<T>(msg: impl Into<String>) -> Result<T, ConfigError> {
    Err(ConfigError(msg.into()))
}

/// A non-negative integer given as a JSON number or a decimal string (query params).
fn as_u64(v: &Value, field: &str) -> Result<u64, ConfigError> {
    match v {
        Value::Number(n) => match n.as_u64() {
            Some(x) => Ok(x),
            None => err(format!("{field}: expected a non-negative integer")),
        },
        Value::String(s) => s
            .trim()
            .parse::<u64>()
            .or_else(|_| err(format!("{field}: expected a non-negative integer"))),
        _ => err(format!("{field}: expected a non-negative integer")),
    }
}

fn as_f64(v: &Value, field: &str) -> Result<f64, ConfigError> {
    let x = match v {
        Value::Number(n) => n.as_f64(),
        Value::String(s) => s.trim().parse::<f64>().ok(),
        _ => None,
    };
    match x {
        Some(x) if x.is_finite() => Ok(x),
        _ => err(format!("{field}: expected a number")),
    }
}

fn as_bool(v: &Value, field: &str) -> Result<bool, ConfigError> {
    match v {
        Value::Bool(b) => Ok(*b),
        Value::String(s) if s == "1" || s == "true" => Ok(true),
        Value::String(s) if s == "0" || s == "false" => Ok(false),
        _ => err(format!("{field}: expected a boolean")),
    }
}

impl ConvoyConfig {
    /// Parses the scenario JSON (an empty string or `{}` gives the defaults).
    pub fn parse(json: &str) -> Result<ConvoyConfig, ConfigError> {
        if json.trim().is_empty() {
            return Ok(ConvoyConfig::default());
        }
        let v: Value = serde_json::from_str(json)
            .or_else(|e| err(format!("scenario is not valid JSON: {e}")))?;
        let Value::Object(map) = v else {
            return err("scenario must be a JSON object");
        };
        Self::from_map(&map)
    }

    fn from_map(map: &Map<String, Value>) -> Result<ConvoyConfig, ConfigError> {
        let mut c = ConvoyConfig::default();
        let mut nodes = None;
        for (k, v) in map {
            if v.is_null() {
                continue;
            }
            match k.as_str() {
                "preset" | "sim" => {
                    let Some(s) = v.as_str() else {
                        return err("preset: expected a string");
                    };
                    c.preset = match Preset::parse(s) {
                        Some(p) => p,
                        None => return err(format!("preset: unknown preset {s:?}")),
                    };
                }
                "nodes" => nodes = Some(as_u64(v, "nodes")?),
                "loss" => {
                    let x = as_f64(v, "loss")?;
                    if !(0.0..=1.0).contains(&x) {
                        return err("loss: expected a value in [0, 1]");
                    }
                    c.loss = Some(x);
                }
                "seed" => c.seed = as_u64(v, "seed")?,
                "wallMs" => c.wall_ms = as_u64(v, "wallMs")?,
                "botsTalk" => c.bots_talk = as_bool(v, "botsTalk")?,
                "botTalkMeanS" => {
                    let x = as_f64(v, "botTalkMeanS")?;
                    if !(1.0..=3_600.0).contains(&x) {
                        return err("botTalkMeanS: expected a value in [1, 3600]");
                    }
                    c.bot_talk_mean_s = x;
                }
                "shadow" => c.shadow = parse_shadow(v)?,
                "roam" => c.roam = as_bool(v, "roam")?,
                "roamMeanS" => {
                    let x = as_f64(v, "roamMeanS")?;
                    if !(5.0..=3_600.0).contains(&x) {
                        return err("roamMeanS: expected a value in [5, 3600]");
                    }
                    c.roam_mean_s = x;
                }
                other => return err(format!("unknown scenario field {other:?}")),
            }
        }
        c.nodes = match nodes {
            Some(n) => {
                let n = usize::try_from(n).unwrap_or(usize::MAX);
                if !(2..=MAX_NODES).contains(&n) {
                    return err(format!("nodes: expected 2..={MAX_NODES}"));
                }
                n
            }
            // An explicit preset without `nodes` keeps the preset's own size (capped).
            None if map.contains_key("preset") || map.contains_key("sim") => {
                c.preset.default_nodes().min(MAX_NODES)
            }
            None => DEFAULT_NODES,
        };
        if let Some((a, b)) = c
            .shadow
            .iter()
            .find(|(a, b)| *a >= c.nodes || *b >= c.nodes || a == b)
        {
            return err(format!("shadow: bad pair [{a},{b}]"));
        }
        Ok(c)
    }
}

/// `[[a,b],…]`: pairs of world node indices.
fn parse_shadow(v: &Value) -> Result<Vec<(usize, usize)>, ConfigError> {
    let Some(pairs) = v.as_array() else {
        return err("shadow: expected [[a,b],…]");
    };
    pairs
        .iter()
        .map(|p| match p.as_array().map(Vec::as_slice) {
            Some([a, b]) => {
                let a = as_u64(a, "shadow")?;
                let b = as_u64(b, "shadow")?;
                Ok((a as usize, b as usize))
            }
            _ => err("shadow: expected [[a,b],…]"),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_and_overrides() {
        assert_eq!(ConvoyConfig::parse("").unwrap(), ConvoyConfig::default());
        assert_eq!(ConvoyConfig::parse("{}").unwrap(), ConvoyConfig::default());
        let c = ConvoyConfig::parse(
            r#"{"preset":"chain5","loss":"0.1","seed":"42","botsTalk":false,"wallMs":5}"#,
        )
        .unwrap();
        assert_eq!(c.preset, Preset::Chain5);
        assert_eq!(c.nodes, 5);
        assert_eq!(c.loss, Some(0.1));
        assert_eq!(c.seed, 42);
        assert!(!c.bots_talk);
        assert_eq!(c.wall_ms, 5);
        let c = ConvoyConfig::parse(r#"{"roam":"1","roamMeanS":30}"#).unwrap();
        assert!(c.roam);
        assert_eq!(c.roam_mean_s, 30.0);
        let c = ConvoyConfig::parse(r#"{"sim":"pair","nodes":3,"shadow":[[0,2]]}"#).unwrap();
        assert_eq!((c.preset, c.nodes), (Preset::Pair, 3));
        assert_eq!(c.shadow, vec![(0, 2)]);
    }

    #[test]
    fn refuses_bad_scenarios() {
        for bad in [
            "[]",
            "nope",
            r#"{"preset":"ring9"}"#,
            r#"{"nodes":1}"#,
            r#"{"nodes":99}"#,
            r#"{"loss":2}"#,
            r#"{"seed":-1}"#,
            r#"{"colour":"red"}"#,
            r#"{"botsTalk":"maybe"}"#,
            r#"{"roam":"yes"}"#,
            r#"{"roamMeanS":1}"#,
            r#"{"nodes":3,"shadow":[[0,3]]}"#,
            r#"{"nodes":3,"shadow":[[1,1]]}"#,
            r#"{"shadow":[0,2]}"#,
        ] {
            assert!(ConvoyConfig::parse(bad).is_err(), "{bad} accepted");
        }
    }
}
