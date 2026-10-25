//! Identities, invite, key schedule, per-link Noise, roster and rekey (contract C8,
//! DESIGN §4.3, §4.7, §8).
//!
//! - [`GroupIdentity`]: per-group Ed25519 + X25519 keys, `id_hash`, `node_id`, collision
//!   helper ([`NodeIdRegistry`]).
//! - [`Invite`] / [`InviteSig`]: invite v1 codec, `invite_sig`, base64url fragment URL.
//! - [`KeySchedule`]: HKDF schedule (PSK, discovery tags/tokens, Aware PMK, tunnel
//!   rendezvous) and [`EpochKeyring`] (last 4 epochs), [`DenyList`].
//! - [`LinkSession`]: `Noise_XXpsk1_25519_ChaChaPoly_SHA256` over `snow` with our own
//!   [`MeshResolver`], raw-split SEC sealing with key phases ([`SecChannel`]) and
//!   [`ReplayWindow`]; [`idproof`] channel binding.
//! - Signed control: [`roster`], [`join`], [`rekey`], [`group_ctl`], [`SignedLeave`],
//!   [`SignedRideCtl`]; [`accept`] holds the monotonic acceptance rules and
//!   [`ControlState`] the stored bodies for anti-entropy.
//!
//! No OS randomness anywhere: every random value comes from the injected
//! `rand_chacha::ChaCha20Rng`, and time is a caller-supplied `u64` of µs.
//!
//! Owned by WP-07 (stub created by WP-01, DESIGN §16.2). Sans-IO crate: the determinism
//! lints below are denied here (clippy.toml holds the list; the workspace allows them for
//! every other crate).
#![deny(clippy::disallowed_methods, clippy::disallowed_types)]
#![forbid(unsafe_code)]

pub mod accept;
pub mod base64url;
pub mod error;
pub mod group_ctl;
pub mod identity;
pub mod idproof;
pub mod invite;
pub mod join;
pub mod keyring;
pub mod open_channel;
pub mod prim;
pub mod rekey;
pub mod replay;
pub mod resolver;
pub mod roster;
pub mod schedule;
pub mod sec;
pub mod session;
pub mod signed;
#[cfg(any(feature = "vectors", test))]
pub mod vectors;

pub use accept::ControlState;
pub use error::{CryptoError, Result};
pub use identity::{CollisionOutcome, GroupIdentity, NodeIdRegistry, PublicIdentity};
pub use idproof::IdProof;
pub use invite::{Invite, InvitePolicy, InviteSig};
pub use keyring::{DenyList, EpochKeyring, RETAINED_EPOCHS};
pub use mesh_wire::mesh::{
    GroupCtl, GroupCtlOp, JoinReq, Rekey, RekeyEntry, Roster, RosterEntry, RosterRole,
};
pub use prim::Secret32;
pub use rand_chacha::ChaCha20Rng;
pub use replay::ReplayWindow;
pub use resolver::MeshResolver;
pub use schedule::KeySchedule;
pub use sec::SecChannel;
pub use session::{HsProgress, LinkInfo, LinkSession, Role};
pub use signed::{SignedLeave, SignedRideCtl};
