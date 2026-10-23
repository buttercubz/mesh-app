//! Simulated Wi-Fi Aware data paths (DESIGN §9.6-§9.7, §13.2 "Aware: R50 120 m,
//! 5 ± 2 ms, 20 Mbps"): the native side of the UPGRADE exchange. The core runs the
//! exchange over BLE and mirrors it with `RadioCommand::AwareUpgrade{peer, op}` (1 offer
//! sent, 2 accepted, 3 rejected, 4 ready, 5 retire); once both ends of a pair report
//! op 2, the platform builds the data path after `aware_setup_us` and both engines get
//! an `Up` of kind `WifiAware` (offerer = initiator). Op 3/5 tears it down. The radio
//! availability (`RadioState{aware}`) is per node and set by scenarios.

use std::collections::BTreeSet;

use mesh_core::{LinkDownReason, LinkKind, PlatformEvent, RadioAvail};

use super::{Ev, World};

/// Native Aware state of the world.
#[derive(Debug, Clone, Default)]
pub struct AwareState {
    /// Nodes whose Aware radio is on (they reported `RadioState{aware: On}`).
    pub radio_on: BTreeSet<usize>,
    /// `(node, peer)`: `node` accepted an upgrade with `peer` (op 2).
    accepted: BTreeSet<(usize, usize)>,
    /// `(node, peer)`: `node` sent the offer (op 1).
    offered: BTreeSet<(usize, usize)>,
    /// Pairs `(lo, hi)` whose data path is being built.
    opening: BTreeSet<(usize, usize)>,
    /// Data paths built so far.
    pub paths_built: u64,
}

impl World {
    /// Turns node `i`'s Wi-Fi Aware radio on or off (`RadioState`; BLE stays on).
    pub fn set_aware_radio(&mut self, i: usize, on: bool) {
        let aware = if on {
            self.aware.radio_on.insert(i);
            RadioAvail::On
        } else {
            self.aware.radio_on.remove(&i);
            RadioAvail::Off
        };
        let now = self.now;
        self.nodes[i].engine.handle_event(
            now,
            PlatformEvent::RadioState {
                ble: RadioAvail::On,
                aware,
            },
        );
        self.dirty.insert(i);
        if !on {
            for li in self.aware_links_of(i) {
                self.drop_link(li, [LinkDownReason::RadioOff; 2]);
            }
        }
        self.settle();
    }

    /// Up Aware links that have `i` as an end.
    pub fn aware_links_of(&self, i: usize) -> Vec<usize> {
        (0..self.links.len())
            .filter(|l| {
                let l = &self.links[*l];
                l.up && l.kind == LinkKind::WifiAware && l.peer(i).is_some()
            })
            .collect()
    }

    /// The world node that is `node_id` in its stored group (one group per world).
    pub fn index_of_node_id(&self, node_id: u16) -> Option<usize> {
        (0..self.nodes.len()).find(|&i| {
            self.stored_groups(i).iter().any(|g| {
                matches!(
                    mesh_core::group::store::decode(&g.blob),
                    Ok(mesh_core::group::store::Stored::Live(ref gr)) if gr.my_node_id() == node_id
                )
            })
        })
    }

    /// `RadioCommand::AwareUpgrade{peer, op}` from node `i`.
    pub(crate) fn aware_upgrade(&mut self, i: usize, peer: u16, op: u8) {
        let Some(j) = self.index_of_node_id(peer) else {
            return;
        };
        match op {
            1 => {
                self.aware.offered.insert((i, j));
            }
            2 => {
                self.aware.accepted.insert((i, j));
                let key = (i.min(j), i.max(j));
                if self.aware.accepted.contains(&(j, i))
                    && !self.aware.opening.contains(&key)
                    && self
                        .links_between(i, j)
                        .iter()
                        .all(|l| self.links[*l].kind != LinkKind::WifiAware)
                {
                    self.aware.opening.insert(key);
                    let t = self.now.0 + self.cfg.radio.aware_setup_us;
                    self.schedule(t, Ev::AwareOpen { a: key.0, b: key.1 });
                }
            }
            3 | 5 => {
                self.forget_upgrade(i, j);
                for li in self.links_between(i, j) {
                    if self.links[li].kind == LinkKind::WifiAware {
                        self.drop_link(li, [LinkDownReason::PeerClosed; 2]);
                    }
                }
            }
            _ => {}
        }
    }

    fn forget_upgrade(&mut self, i: usize, j: usize) {
        for k in [(i, j), (j, i)] {
            self.aware.accepted.remove(&k);
            self.aware.offered.remove(&k);
        }
        self.aware.opening.remove(&(i.min(j), i.max(j)));
    }

    /// The data path of the pair is ready: both engines get `Up{WifiAware}`.
    pub(crate) fn aware_open(&mut self, a: usize, b: usize) {
        if !self.aware.opening.remove(&(a, b)) {
            return;
        }
        let both = self.aware.accepted.contains(&(a, b)) && self.aware.accepted.contains(&(b, a));
        let radios = self.aware.radio_on.contains(&a) && self.aware.radio_on.contains(&b);
        let d = self.distance(a, b);
        let pdr = self.radio.aware_pdr(a, b, d, self.now.0);
        if !both || !radios || pdr < self.cfg.discovery.connect_min_pdr {
            self.forget_upgrade(a, b);
            return;
        }
        // The offerer initiates the fresh Noise handshake on the data path.
        let (init, resp) = if self.aware.offered.contains(&(b, a)) {
            (b, a)
        } else {
            (a, b)
        };
        self.create_link(init, resp, LinkKind::WifiAware, None);
        self.aware.paths_built += 1;
        self.settle();
    }

    /// Fault injection: the Aware data path between `a` and `b` fails (the platform
    /// reports it down with a timeout on both ends).
    pub fn fail_aware(&mut self, a: usize, b: usize) -> bool {
        let mut any = false;
        for li in self.links_between(a, b) {
            if self.links[li].kind == LinkKind::WifiAware {
                self.drop_link(li, [LinkDownReason::Timeout; 2]);
                any = true;
            }
        }
        self.forget_upgrade(a, b);
        self.settle();
        any
    }
}
