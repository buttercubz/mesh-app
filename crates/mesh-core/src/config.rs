//! Engine configuration (DESIGN §3.2 `EngineConfig`), limits with per-platform defaults
//! (Appendix A) and the session start modes.

use mesh_wire::record::Caps;

/// The platform the engine runs on (CAPS `platform`, HELLO `flags.android`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Platform {
    Ios,
    Android,
    /// Browser simulator (mesh-wasm) and host tests.
    WebSim,
}

impl Platform {
    /// CAPS `platform` byte.
    pub const fn caps_byte(self) -> u8 {
        match self {
            Platform::Ios => Caps::PLATFORM_IOS,
            Platform::Android => Caps::PLATFORM_ANDROID,
            Platform::WebSim => Caps::PLATFORM_WEBSIM,
        }
    }

    /// labrun/1 `device.platform`.
    pub const fn lab(self) -> mesh_lab::record::Platform {
        match self {
            Platform::Ios => mesh_lab::record::Platform::Ios,
            Platform::Android => mesh_lab::record::Platform::Android,
            Platform::WebSim => mesh_lab::record::Platform::Sim,
        }
    }
}

/// One stored group as loaded by the runtime from secure storage (opaque to WP-10A; the
/// `group` module of WP-10B owns the layout of `blob`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredGroup {
    /// Group id.
    pub group_id: [u8; 16],
    /// Persisted state written earlier through `Effect::Persist` (`group/<hex id>`).
    pub blob: Vec<u8>,
}

/// Device description recorded in labrun/1 `run` lines.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceDescriptor {
    /// Hardware model (`iPhone15,2`).
    pub model: String,
    /// OS name and version (`iOS 27.0`).
    pub os: String,
    /// App version string (`0.1.0 (12)`).
    pub app: String,
}

impl Default for DeviceDescriptor {
    fn default() -> Self {
        DeviceDescriptor {
            model: "unknown".into(),
            os: "unknown".into(),
            app: "0.0.0".into(),
        }
    }
}

/// Link targets, caps and timers (Appendix A). `routing` is handed to mesh-routing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    /// Targets, caps, airtime budget and floor timers used by mesh-routing.
    pub routing: mesh_routing::Limits,
    /// Connect attempt timeout (§9.5).
    pub connect_timeout_us: u64,
    /// Handshake timeout (§4.3).
    pub handshake_timeout_us: u64,
    /// Link death without inbound frames (§5.1).
    pub link_death_us: u64,
    /// PING period while a burst is active or in Lab (§5.1).
    pub ping_active_us: u64,
    /// PING period otherwise.
    pub ping_idle_us: u64,
    /// LSTAT period (§4.4).
    pub lstat_period_us: u64,
    /// Per-peer connect backoff: initial and maximum (§9.5).
    pub backoff_initial_us: u64,
    pub backoff_max_us: u64,
    /// `LimitReached` lowers the device's BLE max by 1 for this long (§3.2).
    pub limit_backoff_us: u64,
    /// HELLO `Abort{ignore_s}` for a peer that is not ours (§9.3).
    pub abort_ignore_s: u16,
    /// Snapshot emission period floor (≤ 10 Hz, §3.4).
    pub snapshot_min_interval_us: u64,
    /// `labProgress` period (2 Hz, §3.7).
    pub lab_progress_interval_us: u64,
    /// CAPS `transports` bits this platform supports.
    pub transports: u8,
    /// ATT MTU requested on GATT links.
    pub preferred_att_mtu: u16,
}

impl Limits {
    /// v1 defaults shared by every platform.
    pub const V1: Limits = Limits {
        routing: mesh_routing::Limits::V1,
        connect_timeout_us: 8_000_000,
        handshake_timeout_us: 5_000_000,
        link_death_us: mesh_routing::consts::LINK_DEATH_US,
        ping_active_us: mesh_routing::consts::PING_ACTIVE_US,
        ping_idle_us: mesh_routing::consts::PING_IDLE_US,
        lstat_period_us: 1_000_000,
        backoff_initial_us: 5_000_000,
        backoff_max_us: 60_000_000,
        limit_backoff_us: 600_000_000,
        abort_ignore_s: 60,
        snapshot_min_interval_us: 100_000,
        lab_progress_interval_us: 500_000,
        transports: Caps::T_L2CAP | Caps::T_GATT,
        preferred_att_mtu: 517,
    };

    /// Per-platform defaults (Appendix A, §9.5): Aware transport bits per OS; iOS
    /// negotiates its own ATT MTU (185 on current iPhones), Android requests 517.
    pub const fn for_platform(p: Platform) -> Limits {
        let mut l = Self::V1;
        match p {
            Platform::Ios => {
                l.transports = Caps::T_L2CAP | Caps::T_GATT | Caps::T_AWARE_IOS;
                l.preferred_att_mtu = 185;
            }
            Platform::Android => {
                l.transports = Caps::T_L2CAP | Caps::T_GATT | Caps::T_AWARE_ANDROID;
                l.preferred_att_mtu = 517;
            }
            Platform::WebSim => {}
        }
        l
    }

    /// BLE link target per node.
    pub const fn ble_target(&self) -> u8 {
        self.routing.ble_links_target
    }

    /// BLE link max per node (before any `LimitReached` backoff).
    pub const fn ble_max(&self) -> u8 {
        self.routing.ble_links_max
    }

    /// Node BLE airtime budget in bit/s (§5.6).
    pub const fn node_ble_budget_bps(&self) -> u32 {
        self.routing.node_ble_budget_bps
    }

    /// Idle-link grant in bit/s (§5.6).
    pub const fn idle_grant_bps(&self) -> u32 {
        self.routing.idle_grant_bps
    }
}

/// Engine configuration (DESIGN §3.2).
#[derive(Debug, Clone, PartialEq)]
pub struct EngineConfig {
    pub platform: Platform,
    /// App version (CAPS `app_ver`).
    pub app_version: u16,
    /// Per-group identity + epoch secrets, loaded by the runtime from secure storage.
    pub stored_groups: Vec<StoredGroup>,
    /// CSPRNG seed on device, fixed in the simulator.
    pub rng_seed: [u8; 32],
    /// Link targets, caps, timers.
    pub limits: Limits,
    /// Device description for Lab files.
    pub device: DeviceDescriptor,
    /// Wall clock (Unix ms) at the `now` passed to `Engine::new`; refined by
    /// `PlatformSample::wall_ms`. Only used for Lab run ids and `startedAt`.
    pub wall_ms: u64,
    /// Lab runs stored on the device (rebuilt by the runtime from its index, see
    /// `Effect::Persist` key `lab/index`).
    pub lab_runs: Vec<mesh_api::LabRunInfo>,
}

impl EngineConfig {
    /// A configuration with the platform's default limits and no stored state.
    pub fn new(platform: Platform, rng_seed: [u8; 32]) -> Self {
        EngineConfig {
            platform,
            app_version: 1,
            stored_groups: Vec::new(),
            rng_seed,
            limits: Limits::for_platform(platform),
            device: DeviceDescriptor::default(),
            wall_ms: 0,
            lab_runs: Vec::new(),
        }
    }
}

/// How the runtime starts the engine (DESIGN §3.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionStart {
    /// Lobby: radios on, audio off until Go live.
    Group { group_id: [u8; 16] },
    /// Lab mode (§12) with the shared 4-digit code (0-9999).
    Lab { lab_code: u16 },
    /// Demo ride: handled by the runtime (§3.4), refused by the engine.
    Demo,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_follow_appendix_a() {
        let l = Limits::for_platform(Platform::Ios);
        assert_eq!((l.ble_target(), l.ble_max()), (2, 3));
        assert_eq!(l.node_ble_budget_bps(), 180_000);
        assert_eq!(l.idle_grant_bps(), 8_000);
        assert_eq!(l.link_death_us, 4_000_000);
        assert_eq!((l.ping_active_us, l.ping_idle_us), (500_000, 2_000_000));
        assert_eq!(l.limit_backoff_us, 600_000_000);
        assert_ne!(l.transports & Caps::T_AWARE_IOS, 0);
        let a = Limits::for_platform(Platform::Android);
        assert_ne!(a.transports & Caps::T_AWARE_ANDROID, 0);
        assert_eq!(a.preferred_att_mtu, 517);
        assert_eq!(Limits::for_platform(Platform::WebSim), Limits::V1);
        assert_eq!(Platform::Android.caps_byte(), 1);
    }
}
