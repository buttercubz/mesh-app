//! Groups (DESIGN §3.3, §8, owned by WP-10B): stored groups and their persistence,
//! invite create/parse/join/forget with the natively rendered QR (`qrPng`), per-group
//! identities, the signed control state (roster cache, JOIN_REQ admission and
//! retransmission, epochs with a retained keyring, rekey, deny list, leader handover),
//! rotating on-air identifiers and control-state anti-entropy (§8.9).
//!
//! [`GroupCore`] is the `CoreModule` registered by `wire/group.rs`; it also drives the
//! secure links (`crate::secure`) and the ride session (`crate::session`).

pub mod active;
mod apply;
mod commands;
pub mod control;
pub mod core;
pub mod flood;
pub mod leader;
pub mod open;
pub mod png;
pub mod qr;
pub mod radio;
mod run;
mod session_cmd;
pub mod state;
pub mod store;
pub mod sync;

pub use core::GroupCore;

/// The invite URL a stored group blob would hand out (tests, simulator and the native
/// share path; the UI only ever receives the QR image).
pub fn invite_url_of_blob(blob: &[u8]) -> Option<String> {
    match store::decode(blob).ok()? {
        store::Stored::Live(g) => g.invite().map(|i| i.to_url()),
        store::Stored::Closed(_) => None,
    }
}
