//! A test module registered after the group module: it observes every MESH record a
//! node receives (the group module delivers without claiming) and logs PRESENCE.

use std::cell::RefCell;
use std::rc::Rc;

use mesh_core::{CoreModule, Ctx, LinkId};
use mesh_wire::mesh::{MeshPacket, MeshPayload, Presence};
use mesh_wire::record::Record;

pub type Log = Rc<RefCell<Vec<(u16, Presence)>>>;

struct Probe {
    log: Log,
}

impl CoreModule for Probe {
    fn name(&self) -> &'static str {
        "presence-probe"
    }

    fn on_record(&mut self, _ctx: &mut Ctx, _link: LinkId, rec: &Record<'_>) -> bool {
        if let Record::Mesh(b) = rec
            && let Ok(p) = MeshPacket::decode(b)
            && let Ok(MeshPayload::Presence(x)) = MeshPayload::decode(&p)
        {
            self.log.borrow_mut().push((p.header.origin, x));
        }
        false
    }
}

/// Registers the probe on node `i`.
pub fn attach(w: &mut crate::harness::World, i: usize) -> Log {
    let log: Log = Rc::new(RefCell::new(Vec::new()));
    w.nodes[i]
        .engine
        .register_module(Box::new(Probe { log: log.clone() }));
    log
}
