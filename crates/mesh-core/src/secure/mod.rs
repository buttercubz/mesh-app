//! Secure group links (DESIGN §3.3, §4.3, §8.4, owned by WP-10B): the `LinkSecurity`
//! implementation over `mesh_crypto::LinkSession` (Noise `XXpsk1_25519_ChaChaPoly_SHA256`,
//! raw-split SEC sealing with AAD `lf_type ‖ kp_ctr` and in-band key update) and the
//! IDPROOF admission step (deny list, `node_id` collision `REJECT{8}`, retained-epoch
//! admission, duplicate-link resolution by the lower initiator `node_id`) in
//! [`admit`].
//!
//! The handshake result the admission step needs (role, epoch, handshake hash, remote
//! static key) is published by [`SecureLink`] into a shared [`HsSlot`] the group module
//! keeps per link, because the link table owns the `Box<dyn LinkSecurity>`.

pub mod admit;

use std::cell::Cell;
use std::rc::Rc;

use mesh_crypto::session::{LinkInfo as HsInfo, Role};
use mesh_crypto::{CryptoError, EpochKeyring, GroupIdentity, KeySchedule, LinkSession};
use mesh_wire::PROTOCOL_VERSION;
use mesh_wire::frame::{LinkFrame, RejectReason, SEC_OVERHEAD};
use rand_chacha::ChaCha20Rng;

use crate::Instant;
use crate::links::{LinkSecurity, SecurityError, SecurityStep};

/// Handshake result shared with the group module (set once established).
pub type HsSlot = Rc<Cell<Option<HsInfo>>>;

/// How the initiator picks its epoch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EpochChoice {
    /// The epoch whose `epoch_lo` the peer's HELLO advertised, if I retain it.
    Hint(u16),
    /// My newest epoch (speculative link to an epoch-ahead peer, or no HELLO).
    Newest,
}

enum Side {
    /// Initiator before `on_start`.
    Initiator {
        identity: Box<GroupIdentity>,
        sched: Box<KeySchedule>,
    },
    Session(Box<LinkSession>),
    Failed,
}

/// `LinkSecurity` of a group link.
pub struct SecureLink {
    side: Side,
    rng: ChaCha20Rng,
    slot: HsSlot,
}

impl core::fmt::Debug for SecureLink {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("SecureLink")
            .field("established", &self.slot.get().is_some())
            .finish_non_exhaustive()
    }
}

impl SecureLink {
    /// The connection initiator (BLE central / Aware subscriber).
    pub fn initiator(
        identity: &GroupIdentity,
        keyring: &EpochKeyring,
        choice: EpochChoice,
        rng_seed: [u8; 32],
        slot: HsSlot,
    ) -> Self {
        let sched = match choice {
            EpochChoice::Hint(lo) => keyring.by_epoch_lo(lo).unwrap_or(keyring.newest()),
            EpochChoice::Newest => keyring.newest(),
        };
        SecureLink {
            side: Side::Initiator {
                identity: Box::new(identity.clone()),
                sched: Box::new(sched.clone()),
            },
            rng: rand_core::SeedableRng::from_seed(rng_seed),
            slot,
        }
    }

    /// The responder: tries every retained epoch's PSK on msg1, newest first.
    pub fn responder(
        identity: &GroupIdentity,
        keyring: &EpochKeyring,
        now: Instant,
        rng_seed: [u8; 32],
        slot: HsSlot,
    ) -> Self {
        let mut rng: ChaCha20Rng = rand_core::SeedableRng::from_seed(rng_seed);
        let s = LinkSession::responder(identity, keyring, None, PROTOCOL_VERSION, now.0, &mut rng);
        SecureLink {
            side: Side::Session(Box::new(s)),
            rng,
            slot,
        }
    }

    fn fail(&mut self, e: CryptoError, at_msg1: bool) -> SecurityStep {
        self.side = Side::Failed;
        let reason = if at_msg1 {
            // Wrong PSK, group or epoch: the stranger learns nothing else.
            match e {
                CryptoError::Version => RejectReason::Version,
                _ => RejectReason::GroupEpochMismatch,
            }
        } else {
            e.reject_reason().unwrap_or(RejectReason::HandshakeFailure)
        };
        SecurityStep::Failed(reason)
    }

    fn publish(&self) {
        if let Side::Session(s) = &self.side
            && let Some(info) = s.info()
        {
            self.slot.set(Some(*info));
        }
    }
}

impl LinkSecurity for SecureLink {
    fn on_start(&mut self, now: Instant, out: &mut Vec<Vec<u8>>) -> SecurityStep {
        let side = core::mem::replace(&mut self.side, Side::Failed);
        match side {
            Side::Initiator { identity, sched } => {
                match LinkSession::initiator(
                    &identity,
                    &sched,
                    PROTOCOL_VERSION,
                    now.0,
                    &mut self.rng,
                ) {
                    Ok((s, msg1)) => {
                        out.push(msg1);
                        self.side = Side::Session(Box::new(s));
                        SecurityStep::Pending
                    }
                    Err(e) => self.fail(e, false),
                }
            }
            other => {
                self.side = other;
                SecurityStep::Pending
            }
        }
    }

    fn on_hs(
        &mut self,
        now: Instant,
        step: u8,
        proto_ver: u8,
        noise_msg: &[u8],
        out: &mut Vec<Vec<u8>>,
    ) -> SecurityStep {
        let Side::Session(s) = &mut self.side else {
            return SecurityStep::Failed(RejectReason::HandshakeFailure);
        };
        let frame = match (LinkFrame::Hs {
            step,
            proto_ver,
            noise_msg,
        })
        .encode()
        {
            Ok(f) => f,
            Err(_) => return self.fail(CryptoError::Handshake, false),
        };
        let at_msg1 = step == 1 && s.role() == Role::Responder;
        match s.on_handshake(&frame, now.0) {
            Ok(p) => {
                if let Some(reply) = p.send {
                    out.push(reply);
                }
                if p.established {
                    self.publish();
                    SecurityStep::Established
                } else {
                    SecurityStep::Pending
                }
            }
            Err(e) => self.fail(e, at_msg1),
        }
    }

    fn seal(&mut self, now: Instant, records: &[u8]) -> Result<Vec<u8>, SecurityError> {
        match &mut self.side {
            Side::Session(s) => s.seal(records, now.0).map_err(|e| match e {
                CryptoError::Wire(w) => SecurityError::Wire(w),
                _ => SecurityError::NotEstablished,
            }),
            _ => Err(SecurityError::NotEstablished),
        }
    }

    fn open(&mut self, now: Instant, frame: &LinkFrame<'_>) -> Result<Vec<u8>, SecurityError> {
        let LinkFrame::Sec { kp_ctr, sealed } = *frame else {
            return Err(SecurityError::WrongFrame);
        };
        let Side::Session(s) = &mut self.side else {
            return Err(SecurityError::NotEstablished);
        };
        let sec = s.sec().ok_or(SecurityError::NotEstablished)?;
        // Tag (incl. AAD = lf_type ‖ kp_ctr) and replay window are checked before any
        // plaintext leaves; the window moves only after the tag verified.
        sec.open_parts(kp_ctr, sealed, now.0)
            .map_err(|_| SecurityError::Auth)
    }

    fn overhead(&self) -> usize {
        SEC_OVERHEAD
    }

    fn channel_binding(&self) -> Option<[u8; 32]> {
        self.slot.get().map(|i| i.handshake_hash)
    }
}

/// Role of a link in mesh-crypto terms.
pub fn role_of(r: crate::LinkRole) -> Role {
    match r {
        crate::LinkRole::Initiator => Role::Initiator,
        crate::LinkRole::Responder => Role::Responder,
    }
}
