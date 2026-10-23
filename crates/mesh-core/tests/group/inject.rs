//! A test module registered after the group module: on its next timer it pushes queued
//! signed bodies as link-scoped SYNC records on every Up group link, sealed by the real
//! secure link (a member replaying an old ROSTER, or an old leader signing a ROSTER
//! after a transfer, exactly as a malicious or stale peer would).

use std::cell::RefCell;
use std::rc::Rc;

use mesh_core::links::{LinkState, Prio};
use mesh_core::{CoreModule, Ctx};
use mesh_wire::mesh::sync_chunks;
use mesh_wire::record::{Record, SyncKind};

pub type Queue = Rc<RefCell<Vec<(SyncKind, Vec<u8>)>>>;

struct Injector {
    queue: Queue,
    msg_id: u16,
}

impl CoreModule for Injector {
    fn name(&self) -> &'static str {
        "sync-injector"
    }

    fn on_timer(&mut self, ctx: &mut Ctx) {
        let items: Vec<(SyncKind, Vec<u8>)> = self.queue.borrow_mut().drain(..).collect();
        if items.is_empty() {
            return;
        }
        let links: Vec<_> = ctx
            .links
            .iter()
            .filter(|l| l.state == LinkState::Up && !l.is_lab())
            .map(|l| l.id)
            .collect();
        for (kind, body) in items {
            self.msg_id = self.msg_id.wrapping_add(1);
            let chunks = sync_chunks(kind, self.msg_id, &body).expect("sync chunks");
            for &l in &links {
                for c in &chunks {
                    ctx.send_record(l, Prio::P2, &Record::Sync(*c));
                }
            }
        }
    }
}

/// Registers an injector on node `i`.
pub fn attach(w: &mut crate::harness::World, i: usize) -> Queue {
    let queue: Queue = Rc::new(RefCell::new(Vec::new()));
    w.nodes[i].engine.register_module(Box::new(Injector {
        queue: queue.clone(),
        msg_id: 0xE000,
    }));
    queue
}
