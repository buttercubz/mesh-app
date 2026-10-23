//! Floor control (DESIGN §6): total rank order, floor table, per-link relay admission,
//! near-talker rejection and the origin's `local_admit`. One rank function
//! ([`Rank`]) is shared by relay admission and `local_admit`.

mod admission;
mod rank;
mod table;

pub use admission::{Admission, AdmitDecision};
pub use rank::{BurstKey, FLOOR_HASH_CONTEXT, Rank, floor_hash, smear_us, t_origin_us};
pub use table::{BurstEntry, EndReason, FloorEvent, FloorTable, prio_of};

#[cfg(test)]
mod tests;
