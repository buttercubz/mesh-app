//! Snapshot builder (DESIGN §3.7): the base engine fills engine, audio, mesh link
//! counts and device state; modules add session, talkers, members and latency through
//! `CoreModule::fill_snapshot`.

use std::collections::BTreeSet;

use mesh_api::{AudioInfo, DeviceInfo, MeshInfo, MeshProfile, Snapshot};

use super::device::quantize;
use crate::engine::{Ctx, ModuleRegistry};
use crate::lab::LabMode;
use crate::links::{LinkKind, LinkState};

/// Builds the full snapshot.
pub fn build_snapshot(ctx: &Ctx, lab: &LabMode, modules: &ModuleRegistry) -> Snapshot {
    let up: Vec<_> = ctx
        .links
        .iter()
        .filter(|l| l.state == LinkState::Up)
        .collect();
    let neighbors: BTreeSet<u16> = up.iter().filter_map(|l| l.peer_node.map(|n| n.0)).collect();
    let any_aware = up.iter().any(|l| l.info.kind == LinkKind::WifiAware);
    let limits = &ctx.cfg.limits.routing;
    let (reachable, total) = if ctx.is_lab() {
        let peers = lab.peers_heard(ctx.now) as u8;
        (peers, lab.peers_known() as u8)
    } else {
        (neighbors.len() as u8, neighbors.len() as u8)
    };
    let sample = ctx.device.sample.as_ref();
    let mut s = Snapshot {
        engine: ctx.ui.engine(),
        session: None,
        audio: AudioInfo {
            route: ctx.device.route_kind(),
            capture_ok: ctx.device.capture_ok(),
            render_ok: ctx.device.render_ok(),
            input_level: quantize(ctx.device.input_level),
            tx_muted_no_headset: false,
        },
        talkers: Vec::new(),
        members: Vec::new(),
        mesh: MeshInfo {
            links: up.len().min(255) as u8,
            neighbors: neighbors.len().min(255) as u8,
            islands: u8::from(!up.is_empty()),
            reachable,
            total,
            cap_ble: limits.s_max_ble,
            cap_wifi: limits.s_max_aware,
            profile: if any_aware {
                MeshProfile::Wifi
            } else {
                MeshProfile::Ble
            },
        },
        device: DeviceInfo {
            battery: sample.and_then(|s| s.battery).map(quantize),
            thermal: ctx.device.thermal(),
            background: sample.is_some_and(|s| s.background),
            bluetooth: ctx.radio.ble.ui(),
        },
        latency: None,
    };
    modules.fill_snapshot(ctx, &mut s);
    s
}
