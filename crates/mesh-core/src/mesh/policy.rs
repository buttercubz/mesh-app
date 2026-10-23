//! Tunable policy of the mesh glue: the rejoin strategy that the Phase 0 memo picks
//! (§9.5, T3) and the Phase 4 upgrade switch.

use crate::config::Platform;

/// Rejoin strategy after a lost link (§9.5). The T3 bench of the Phase 0 field campaign
/// decides which variant ships; until the memo exists the iOS default is the pending
/// reconnect (variant a), the only one the core has to drive.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReconnectPolicy {
    /// T3 variant (a): keep an outstanding `connect(peripheral)` to each lost iPhone
    /// peer for up to [`MeshPolicy::reconnect_hold_s`] (`RadioCommand::ArmReconnect`).
    PendingConnect,
    /// T3 variant (b): Live Activity-backed scanning; the native layer keeps scanning,
    /// the core arms nothing.
    LiveActivityScan,
    /// No rejoin help beyond normal discovery.
    Off,
}

/// Mesh glue policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MeshPolicy {
    pub reconnect: ReconnectPolicy,
    /// How long a pending reconnect stays armed (15 min, bounded by the peer's
    /// resolvable-address rotation).
    pub reconnect_hold_s: u16,
    /// Offer Wi-Fi Aware upgrades (Phase 4; also needs both CAPS and the group's
    /// Aware pairing).
    pub aware_upgrade: bool,
    /// Retry an UPGRADE offer to the same neighbor at most this often.
    pub upgrade_retry_us: u64,
}

impl MeshPolicy {
    /// Defaults per platform.
    pub const fn for_platform(p: Platform) -> MeshPolicy {
        MeshPolicy {
            reconnect: match p {
                Platform::Ios => ReconnectPolicy::PendingConnect,
                Platform::Android | Platform::WebSim => ReconnectPolicy::Off,
            },
            reconnect_hold_s: 900,
            aware_upgrade: true,
            upgrade_retry_us: 60_000_000,
        }
    }
}
