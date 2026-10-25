//! UniFFI control plane (contract C4, DESIGN §3.5): the `MeshEngine` object, the
//! `PlatformHooks` callback trait and the free functions. Link data crosses as
//! `Vec<u8>` (never `&[u8]`, UniFFI 0.32 Kotlin direct-ByteBuffer break); the audio path
//! never touches UniFFI (see `crate::rt`).

pub mod convert;
pub mod hooks;
pub mod outputs;
pub mod types;

use std::sync::Arc;

use mesh_api::PromptKey;
use mesh_core::{LinkId, SessionStart};

use self::hooks::PlatformHooks;
use self::outputs::{FfiBleConstants, FfiLogLevel};
use self::types::*;
use crate::runtime::Runtime;
use crate::runtime::config::BaseConfig;
use crate::runtime::inbound::Inbound;

/// The engine runtime handle owned by `EngineHost` (Swift / Kotlin).
#[derive(uniffi::Object)]
pub struct MeshEngine {
    rt: Runtime,
}

impl MeshEngine {
    /// A handle over an explicit runtime (tests inject engines and private RT planes).
    pub fn from_runtime(rt: Runtime) -> Arc<Self> {
        Arc::new(MeshEngine { rt })
    }

    /// The runtime.
    pub fn runtime(&self) -> &Runtime {
        &self.rt
    }
}

#[uniffi::export]
impl MeshEngine {
    /// Validates the configuration; spawns nothing (the engine thread starts lazily).
    #[uniffi::constructor]
    pub fn new(cfg: FfiEngineConfig, hooks: Arc<dyn PlatformHooks>) -> Result<Arc<Self>, FfiError> {
        let (base, store) = BaseConfig::from_ffi(cfg)?;
        Ok(Self::from_runtime(Runtime::new(
            base,
            store,
            hooks,
            &crate::rt::RT,
            Runtime::default_factory(),
        )))
    }

    /// A UI command (JSON) → `CommandResult` (JSON). Works in Idle too.
    pub fn ui_command(&self, json: String) -> String {
        self.rt.ui_command(&json)
    }

    /// Starts a group session or Lab mode: engine thread + radios; audio stays off until
    /// a gesture command starts capture.
    pub fn start(&self, start: FfiSessionStart) -> Result<(), FfiError> {
        let s = match start {
            FfiSessionStart::Group { group_id } => SessionStart::Group {
                group_id: group_id
                    .as_slice()
                    .try_into()
                    .map_err(|_| FfiError::InvalidStart {
                        detail: format!("groupId must be 16 bytes, got {}", group_id.len()),
                    })?,
            },
            FfiSessionStart::Lab { lab_code } => SessionStart::Lab { lab_code },
            FfiSessionStart::Demo => SessionStart::Demo,
        };
        self.rt.start(s)
    }

    /// Stops radios and audio and returns to Idle.
    pub fn stop(&self) {
        self.rt.stop();
    }

    pub fn on_link_up(&self, link: u32, info: FfiLinkInfo) {
        self.rt.post(Inbound::LinkUp {
            link: LinkId(link),
            info: info.into(),
        });
    }

    pub fn on_link_down(&self, link: u32, reason: FfiLinkDownReason) {
        self.rt.post(Inbound::LinkDown {
            link: LinkId(link),
            reason: reason.into(),
        });
    }

    /// One inbound unit: stream chunk, ATT value or datagram.
    pub fn on_link_data(&self, link: u32, data: Vec<u8>) {
        self.rt.post(Inbound::LinkData {
            link: LinkId(link),
            data,
        });
    }

    pub fn on_link_writable(&self, link: u32) {
        self.rt.post(Inbound::Writable { link: LinkId(link) });
    }

    pub fn on_link_metrics(&self, link: u32, rssi: Option<i16>, att_mtu: Option<u16>) {
        self.rt.post(Inbound::Metrics {
            link: LinkId(link),
            rssi,
            att_mtu,
        });
    }

    pub fn on_discovered(&self, d: FfiDiscovery) {
        self.rt.post(Inbound::Discovered(d.into()));
    }

    pub fn on_hello_read(&self, disc_id: u32, bytes: Vec<u8>) {
        self.rt.post(Inbound::HelloRead { disc_id, bytes });
    }

    /// A connect attempt failed before any link existed (`LimitReached` included).
    pub fn on_connect_failed(&self, disc_id: u32, reason: FfiLinkDownReason) {
        self.rt.post(Inbound::ConnectFailed {
            disc_id,
            reason: reason.into(),
        });
    }

    /// The local L2CAP channel was published with `psm` (0 = GATT only).
    pub fn on_l2cap_published(&self, psm: u16) {
        self.rt.post(Inbound::L2capPublished { psm });
    }

    pub fn on_audio_route(&self, r: FfiAudioRoute) {
        self.rt.post(Inbound::AudioRoute(r.into()));
    }

    pub fn on_audio_io(&self, s: FfiAudioIoState) {
        self.rt.post(Inbound::AudioIo(s.into()));
    }

    /// 1 Hz platform sample, including Bluetooth / Wi-Fi Aware availability.
    pub fn on_platform_sample(&self, s: FfiPlatformSample) {
        self.rt.shared().diag.platform(&s);
        let (sample, radio) = s.split();
        self.rt.post(Inbound::Platform { sample, radio });
    }

    /// A rendered spoken prompt (16 kHz mono) for `render_prompts`.
    pub fn on_prompt_pcm(&self, key: String, pcm: Vec<i16>) {
        match key.parse::<PromptKey>() {
            Ok(key) => self.rt.post(Inbound::PromptPcm { key, pcm }),
            Err(_) => self.rt.shared().hooks.log(
                FfiLogLevel::Warn,
                format!("mesh-ffi: on_prompt_pcm with unknown key {key:?}"),
            ),
        }
    }

    /// The Wi-Fi Aware data-path PMK for `peer`, set with `AwareUpgrade{op: 2}` on
    /// Android (`aware_pmk`, DESIGN §8.3, §9.6). Taken once: a second call returns
    /// `None` until the next op 2. Added in C4 1.x (WP-24, additive).
    pub fn take_aware_pmk(&self, peer: u16) -> Option<Vec<u8>> {
        let slot = &self.rt.shared().aware_pmks;
        slot.lock().ok()?.remove(&peer).map(|p| p.to_vec())
    }

    /// The WebView is attached (foreground): gates snapshot emission.
    pub fn set_ui_attached(&self, attached: bool) {
        self.rt.set_ui_attached(attached);
    }

    /// Runtime state (Idle / Running / AudioCheck / Crashed).
    pub fn status(&self) -> FfiRuntimeStatus {
        self.rt.status()
    }

    // ---------------------------------------------- diagnostics (ridelog/1, C4 1.x)

    /// Diagnostics recorder: file writing on/off (the RAM ring always records) and the
    /// build flavour written in every `hdr`.
    pub fn diag_configure(&self, enabled: bool, debug_build: bool) {
        self.rt.shared().diag.configure(enabled, debug_build);
    }

    /// An event the core cannot see, from platform (`src` = "plat") or UI ("ui") code,
    /// stamped with the engine clock. `ev` matches `^[a-z][A-Za-z0-9]*$`;
    /// `fields_json` is a JSON object of scalars or arrays of numbers (no names, no
    /// keys: long or key-like strings are redacted). Returns false when refused.
    pub fn diag_note(&self, src: String, ev: String, fields_json: String) -> bool {
        let d = &self.rt.shared().diag;
        match d.note(d.now(), &src, &ev, &fields_json) {
            Ok(()) => true,
            Err(e) => {
                self.rt.shared().hooks.log(
                    FfiLogLevel::Debug,
                    format!("mesh-ffi: diag_note {ev:?} refused: {e}"),
                );
                false
            }
        }
    }

    /// A user marker with a fixed label (`crackle`, `cantHear`, `notHeard`, `lag`,
    /// `dropped`, `other`). Returns false for an unknown label.
    pub fn diag_mark(&self, label: String) -> bool {
        let d = &self.rt.shared().diag;
        d.mark(d.now(), &label, "ui")
    }

    /// Writes every pending line through `diag_write` now (before an export).
    pub fn diag_flush(&self) {
        let d = &self.rt.shared().diag;
        d.flush(d.now());
    }

    /// The current session as one self-contained JSONL file from the RAM ring (export
    /// fallback when no file was written).
    pub fn diag_snapshot_ring(&self) -> Vec<String> {
        self.rt.shared().diag.ring()
    }

    /// Recorder state for the Settings screen.
    pub fn diag_status(&self) -> FfiDiagStatus {
        let s = self.rt.shared().diag.state();
        FfiDiagStatus {
            enabled: s.enabled,
            session_id: s.session_id,
            ride_key: s.ride_key,
            node: s.node,
            pending_lines: s.pending_lines,
            suppressed: s.suppressed,
            redacted: s.redacted,
        }
    }

    /// The app is terminating: closes the current segment with `end{appTerminate}`.
    pub fn diag_terminate(&self) {
        let d = &self.rt.shared().diag;
        d.terminate(d.now());
    }
}

/// BLE UUIDs and timings (single source of truth for Swift and Kotlin).
#[uniffi::export]
pub fn ble_constants() -> FfiBleConstants {
    mesh_wire::consts::BleConstants::V1.into()
}

/// Wire protocol version.
#[uniffi::export]
pub fn protocol_version() -> u8 {
    mesh_wire::PROTOCOL_VERSION
}

/// UI contract version packed as `major << 8 | minor`; the plugin refuses another major.
#[uniffi::export]
pub fn api_version() -> u16 {
    mesh_api::api_version_u16()
}

/// RT C ABI version (`MESH_RT_ABI_VERSION`), for the native audio hosts' sanity check.
#[uniffi::export]
pub fn rt_abi_version() -> u32 {
    crate::rt::MESH_RT_ABI_VERSION
}
