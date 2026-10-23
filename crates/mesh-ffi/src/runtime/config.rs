//! Engine configuration built from `FfiEngineConfig` and the persisted key/value store.
//!
//! The runtime mirrors every `Persist` effect, so after an engine-thread crash it can
//! rebuild an equivalent configuration (stored groups, Lab run index) for the fresh
//! engine without asking the platform to reload secure storage.

use std::collections::BTreeMap;

use mesh_core::lab::RunIndex;
use mesh_core::lab::index::INDEX_KEY;
use mesh_core::{DeviceDescriptor, EngineConfig, Limits, Platform, StoredGroup};

use crate::api::types::{FfiEngineConfig, FfiError};

/// Persist key prefix of a stored group (`group/<32 hex>`).
pub const GROUP_KEY_PREFIX: &str = "group/";

/// Everything needed to build an `EngineConfig`, except the per-engine RNG seed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BaseConfig {
    pub platform: Platform,
    pub app_version: u16,
    pub device: DeviceDescriptor,
    pub rng_seed: [u8; 32],
}

impl BaseConfig {
    /// Device identity of the diagnostics `hdr` (no names, no addresses).
    pub fn diag_device(&self) -> mesh_lab::ridelog::DeviceInfo {
        let nonempty = |s: &str| {
            if s.is_empty() {
                "unknown".to_owned()
            } else {
                s.to_owned()
            }
        };
        mesh_lab::ridelog::DeviceInfo {
            platform: match self.platform {
                Platform::Ios => "ios",
                Platform::Android => "android",
                Platform::WebSim => "sim",
            }
            .into(),
            model: nonempty(&self.device.model),
            os: nonempty(&self.device.os),
            app: nonempty(&self.device.app),
            build: if cfg!(debug_assertions) {
                "debug"
            } else {
                "release"
            }
            .into(),
            engine: "ffi".into(),
        }
    }

    /// Validates an FFI configuration and returns it with its persisted store.
    pub fn from_ffi(cfg: FfiEngineConfig) -> Result<(Self, PersistStore), FfiError> {
        let rng_seed: [u8; 32] =
            cfg.rng_seed
                .as_slice()
                .try_into()
                .map_err(|_| FfiError::InvalidConfig {
                    detail: format!("rngSeed must be 32 bytes, got {}", cfg.rng_seed.len()),
                })?;
        if rng_seed.iter().all(|&b| b == 0) {
            return Err(FfiError::InvalidConfig {
                detail: "rngSeed is all zero".into(),
            });
        }
        let mut store = PersistStore::default();
        for e in cfg.persisted {
            store.apply(&e.key, Some(e.value));
        }
        Ok((
            BaseConfig {
                platform: cfg.platform.into(),
                app_version: cfg.app_version,
                device: DeviceDescriptor {
                    model: cfg.device_model,
                    os: cfg.device_os,
                    app: cfg.app_label,
                },
                rng_seed,
            },
            store,
        ))
    }

    /// The engine configuration for engine number `generation` (0 for the first). Later
    /// engines (after a crash) derive a different seed so they never replay the
    /// random stream of the engine they replace.
    pub fn engine_config(
        &self,
        store: &PersistStore,
        generation: u32,
        wall_ms: u64,
    ) -> EngineConfig {
        let mut seed = self.rng_seed;
        for (i, b) in generation.to_le_bytes().iter().enumerate() {
            if let Some(s) = seed.get_mut(i) {
                *s ^= b;
            }
        }
        EngineConfig {
            platform: self.platform,
            app_version: self.app_version,
            stored_groups: store.stored_groups(),
            rng_seed: seed,
            limits: Limits::for_platform(self.platform),
            device: self.device.clone(),
            wall_ms,
            lab_runs: store.lab_runs(),
        }
    }
}

/// Mirror of the engine's persisted keys.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PersistStore {
    entries: BTreeMap<String, Vec<u8>>,
}

impl PersistStore {
    /// Applies a `Persist` effect (`None` deletes).
    pub fn apply(&mut self, key: &str, value: Option<Vec<u8>>) {
        match value {
            Some(v) => {
                self.entries.insert(key.to_owned(), v);
            }
            None => {
                self.entries.remove(key);
            }
        }
    }

    /// Value of `key`.
    pub fn get(&self, key: &str) -> Option<&[u8]> {
        self.entries.get(key).map(Vec::as_slice)
    }

    /// Stored groups: every `group/<32 hex>` key with a valid id.
    pub fn stored_groups(&self) -> Vec<StoredGroup> {
        self.entries
            .iter()
            .filter_map(|(k, v)| {
                let hex = k.strip_prefix(GROUP_KEY_PREFIX)?;
                Some(StoredGroup {
                    group_id: parse_group_id(hex)?,
                    blob: v.clone(),
                })
            })
            .collect()
    }

    /// Lab runs from the `lab/index` key.
    pub fn lab_runs(&self) -> Vec<mesh_api::LabRunInfo> {
        self.get(INDEX_KEY).map(RunIndex::parse).unwrap_or_default()
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

/// Parses 32 hex chars into a group id.
pub fn parse_group_id(hex: &str) -> Option<[u8; 16]> {
    let b = hex.as_bytes();
    if b.len() != 32 {
        return None;
    }
    let mut out = [0u8; 16];
    let (pairs, _) = b.as_chunks::<2>();
    for (o, pair) in out.iter_mut().zip(pairs) {
        let s = std::str::from_utf8(pair).ok()?;
        *o = u8::from_str_radix(s, 16).ok()?;
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::types::{FfiPersistedEntry, FfiPlatform};

    fn ffi(seed: Vec<u8>, persisted: Vec<FfiPersistedEntry>) -> FfiEngineConfig {
        FfiEngineConfig {
            platform: FfiPlatform::Ios,
            app_version: 3,
            rng_seed: seed,
            device_model: "iPhone15,2".into(),
            device_os: "iOS 27.0".into(),
            app_label: "0.1.0 (3)".into(),
            persisted,
        }
    }

    #[test]
    fn seed_must_be_32_nonzero_bytes() {
        assert!(matches!(
            BaseConfig::from_ffi(ffi(vec![1; 31], vec![])),
            Err(FfiError::InvalidConfig { .. })
        ));
        assert!(matches!(
            BaseConfig::from_ffi(ffi(vec![0; 32], vec![])),
            Err(FfiError::InvalidConfig { .. })
        ));
        assert!(BaseConfig::from_ffi(ffi(vec![9; 32], vec![])).is_ok());
    }

    #[test]
    fn groups_and_lab_runs_come_from_persisted_keys() {
        let gid = "00112233445566778899aabbccddeeff";
        let (base, mut store) = BaseConfig::from_ffi(ffi(
            vec![5; 32],
            vec![
                FfiPersistedEntry {
                    key: format!("group/{gid}"),
                    value: vec![1, 2, 3],
                },
                FfiPersistedEntry {
                    key: "group/not-hex".into(),
                    value: vec![9],
                },
                FfiPersistedEntry {
                    key: "lab/index".into(),
                    value: b"[]".to_vec(),
                },
            ],
        ))
        .unwrap();
        let cfg = base.engine_config(&store, 0, 42);
        assert_eq!(cfg.stored_groups.len(), 1);
        assert_eq!(cfg.stored_groups[0].group_id[0], 0x00);
        assert_eq!(cfg.stored_groups[0].group_id[15], 0xff);
        assert_eq!(cfg.stored_groups[0].blob, vec![1, 2, 3]);
        assert_eq!(cfg.wall_ms, 42);
        assert_eq!(cfg.device.model, "iPhone15,2");
        assert_eq!(cfg.platform, Platform::Ios);
        assert_eq!(cfg.rng_seed, [5; 32]);
        assert_ne!(base.engine_config(&store, 1, 0).rng_seed, [5; 32]);
        store.apply(&format!("group/{gid}"), None);
        assert!(base.engine_config(&store, 0, 0).stored_groups.is_empty());
        assert_eq!(store.len(), 2);
    }

    #[test]
    fn group_ids_parse_only_from_32_hex_chars() {
        assert_eq!(parse_group_id(&"ab".repeat(16)), Some([0xab; 16]));
        assert_eq!(parse_group_id("ab"), None);
        assert_eq!(parse_group_id(&"zz".repeat(16)), None);
    }
}
