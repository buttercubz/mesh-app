//! Value types shared by commands, results, events and the snapshot (DESIGN §3.7).

use std::collections::BTreeMap;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::macros::string_enum;

string_enum! {
    /// Vehicle profile; selects the default modes of DESIGN §1.4.
    pub enum Vehicle {
        /// Motorcycle with a helmet headset (VOX default).
        Moto = "moto",
        /// Car (PTT default).
        Car = "car",
    }
}

string_enum! {
    /// Talk mode of the local rider (DESIGN §6.4).
    pub enum TalkMode {
        /// Voice-operated transmit.
        Vox = "vox",
        /// Push-to-talk.
        Ptt = "ptt",
        /// Transmit disabled.
        Muted = "muted",
    }
}

string_enum! {
    /// UI language of spoken-prompt templates (DESIGN §7.7, §11.5).
    pub enum Lang {
        /// Spanish (neutral).
        Es = "es",
        /// English.
        En = "en",
    }
}

string_enum! {
    /// Role of a member inside a group.
    pub enum MemberRole {
        /// Holder of the group leader key.
        Leader = "leader",
        /// Any other member.
        Member = "member",
    }
}

string_enum! {
    /// Operating system of a peer.
    pub enum PeerPlatform {
        /// iPhone.
        Ios = "ios",
        /// Android phone.
        Android = "android",
    }
}

string_enum! {
    /// Audio route reported by the platform (DESIGN §7.8, §10.1.2, §10.2.2).
    pub enum AudioRouteKind {
        /// Bluetooth HFP with mSBC/LC3-SWB (16 kHz or more).
        HfpWideband = "hfpWideband",
        /// Bluetooth HFP with CVSD (8 kHz).
        HfpNarrowband = "hfpNarrowband",
        /// Bluetooth LE Audio.
        LeAudio = "leAudio",
        /// Phone loudspeaker.
        BuiltinSpeaker = "builtinSpeaker",
        /// Phone earpiece.
        BuiltinReceiver = "builtinReceiver",
        /// Wired headset.
        Wired = "wired",
        /// Car hands-free (HFP to the head unit).
        CarBluetooth = "carBluetooth",
        /// No route (audio not running).
        None = "none",
    }
}

/// One parameter of a notice or error: an i18n interpolation value, never prose.
///
/// Serialises as a bare JSON string or number (`Record<string, string | number>` in TS).
#[derive(Debug, Clone, PartialEq)]
pub enum ParamValue {
    /// A string value (a member name, a group name, an id).
    Str(String),
    /// A numeric value (a count, a duration in the unit named by the key).
    Num(f64),
}

/// `Number.MAX_SAFE_INTEGER` (2^53 - 1).
const MAX_SAFE_INTEGER: f64 = 9_007_199_254_740_991.0;

impl Serialize for ParamValue {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        match self {
            ParamValue::Str(v) => s.serialize_str(v),
            // Integral values within the JS safe-integer range print without `.0`,
            // exactly as `JSON.stringify` would.
            ParamValue::Num(v) if v.fract() == 0.0 && v.abs() <= MAX_SAFE_INTEGER => {
                s.serialize_i64(*v as i64)
            }
            ParamValue::Num(v) => s.serialize_f64(*v),
        }
    }
}

impl<'de> Deserialize<'de> for ParamValue {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct Visitor;
        impl serde::de::Visitor<'_> for Visitor {
            type Value = ParamValue;
            fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str("a string or a number")
            }
            fn visit_str<E: serde::de::Error>(self, v: &str) -> Result<ParamValue, E> {
                Ok(ParamValue::Str(v.to_owned()))
            }
            fn visit_string<E: serde::de::Error>(self, v: String) -> Result<ParamValue, E> {
                Ok(ParamValue::Str(v))
            }
            fn visit_f64<E: serde::de::Error>(self, v: f64) -> Result<ParamValue, E> {
                Ok(ParamValue::Num(v))
            }
            fn visit_i64<E: serde::de::Error>(self, v: i64) -> Result<ParamValue, E> {
                // JSON integers are carried as f64, exactly like JavaScript numbers.
                Ok(ParamValue::Num(v as f64))
            }
            fn visit_u64<E: serde::de::Error>(self, v: u64) -> Result<ParamValue, E> {
                // JSON integers are carried as f64, exactly like JavaScript numbers.
                Ok(ParamValue::Num(v as f64))
            }
        }
        d.deserialize_any(Visitor)
    }
}

impl From<&str> for ParamValue {
    fn from(v: &str) -> Self {
        ParamValue::Str(v.to_owned())
    }
}

impl From<String> for ParamValue {
    fn from(v: String) -> Self {
        ParamValue::Str(v)
    }
}

impl From<f64> for ParamValue {
    fn from(v: f64) -> Self {
        ParamValue::Num(v)
    }
}

impl From<u32> for ParamValue {
    fn from(v: u32) -> Self {
        ParamValue::Num(f64::from(v))
    }
}

#[cfg(feature = "ts-export")]
impl ts_rs::TS for ParamValue {
    type WithoutGenerics = Self;
    type OptionInnerType = Self;

    fn name(_: &ts_rs::Config) -> String {
        "ParamValue".to_owned()
    }

    fn decl(cfg: &ts_rs::Config) -> String {
        format!("type {} = {};", Self::name(cfg), Self::inline(cfg))
    }

    fn decl_concrete(cfg: &ts_rs::Config) -> String {
        Self::decl(cfg)
    }

    fn inline(_: &ts_rs::Config) -> String {
        "string | number".to_owned()
    }

    fn docs() -> Option<String> {
        Some(ts_rs::format_docs(&[
            " One i18n interpolation value of a notice or error (never prose).",
        ]))
    }

    fn output_path() -> Option<std::path::PathBuf> {
        Some(std::path::PathBuf::from("ParamValue.ts"))
    }
}

/// i18n interpolation parameters of a notice or error, keyed by placeholder name.
pub type Params = BTreeMap<String, ParamValue>;

/// A member reference shown in confirmation sheets.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts-export", derive(ts_rs::TS), ts(export))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NodeRef {
    /// Mesh node id inside the group.
    pub node_id: u16,
    /// Display name (user content, ≤ 24 UTF-8 bytes).
    pub name: String,
}

/// `deserialize_with` for a required nullable field (`T | null` in TS): `null` is
/// `None`, but a missing field is an error. Plain serde would read a missing `Option`
/// field as `None`; this keeps Rust exactly as strict as the generated TS type and the
/// `@meshapp/engine-api` guards.
pub(crate) fn nullable<'de, D, T>(d: D) -> Result<Option<T>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    Option::<T>::deserialize(d)
}
