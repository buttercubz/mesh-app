//! Per-link Noise session (DESIGN §4.3, §8.4): `Noise_XXpsk1_25519_ChaChaPoly_SHA256` via
//! `snow` with [`MeshResolver`], prologue `"cvic/v1" ‖ group_id ‖ epoch u32`, PSK `psk_e`.
//! After the handshake, `dangerously_get_raw_split()` yields the two direction keys and
//! SEC frames are sealed by [`SecChannel`] (snow's transport states are not used).
//!
//! The responder tries every retained epoch's PSK on msg1 (HELLO hint first); a stranger
//! without a retained PSK fails at msg1 and the responder never sends msg2, so neither
//! static key is revealed to non-members.

use mesh_wire::frame::{LinkFrame, NOISE_MSG_SIZES, NOISE_PATTERN, noise_prologue};
use rand_chacha::ChaCha20Rng;
use rand_core::RngCore;
use snow::{Builder, HandshakeState};

use crate::error::{CryptoError, Result};
use crate::identity::{GroupIdentity, PublicIdentity};
use crate::idproof::{self, IdProof};
use crate::keyring::EpochKeyring;
use crate::prim::Secret32;
use crate::resolver::MeshResolver;
use crate::schedule::KeySchedule;
use crate::sec::SecChannel;

/// Handshake timeout (Appendix A).
pub const HANDSHAKE_TIMEOUT_US: u64 = 5_000_000;
/// Largest Noise message we ever produce or accept (empty payloads, §4.3).
const MAX_NOISE_MSG: usize = 96;

/// Link role: the connection initiator (BLE central, Aware subscriber) is the Noise
/// initiator.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Role {
    Initiator,
    Responder,
}

impl Role {
    /// IDPROOF role byte: 0 initiator, 1 responder.
    pub const fn wire(self) -> u8 {
        match self {
            Role::Initiator => 0,
            Role::Responder => 1,
        }
    }
    pub const fn peer(self) -> Role {
        match self {
            Role::Initiator => Role::Responder,
            Role::Responder => Role::Initiator,
        }
    }
}

/// What the handshake established.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LinkInfo {
    pub role: Role,
    pub epoch: u32,
    pub proto_ver: u8,
    pub handshake_hash: [u8; 32],
    /// The peer's X25519 static key, authenticated by Noise.
    pub remote_static: [u8; 32],
}

/// Result of feeding one handshake frame.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HsProgress {
    /// HS LinkFrame to send to the peer, if any.
    pub send: Option<Vec<u8>>,
    /// The link is established (seal/open are available).
    pub established: bool,
}

struct Candidate {
    epoch: u32,
    psk: Secret32,
}

enum State {
    InitiatorWaitMsg2 {
        hs: Box<HandshakeState>,
        epoch: u32,
    },
    ResponderWaitMsg1 {
        candidates: Vec<Candidate>,
    },
    ResponderWaitMsg3 {
        hs: Box<HandshakeState>,
        epoch: u32,
    },
    Established {
        info: LinkInfo,
        sec: Box<SecChannel>,
    },
    Failed,
}

/// Initiator/responder state machine yielding HS bytes, then sealing and opening SEC
/// frames with key phases.
pub struct LinkSession {
    role: Role,
    group_id: [u8; 16],
    static_key: Secret32,
    resolver_seed: Secret32,
    builds: u64,
    proto_ver: u8,
    started_us: u64,
    fixed_ephemeral: Option<[u8; 32]>,
    state: State,
    peer: Option<PublicIdentity>,
}

impl core::fmt::Debug for LinkSession {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("LinkSession")
            .field("role", &self.role)
            .field("established", &self.is_established())
            .field("failed", &self.is_failed())
            .finish_non_exhaustive()
    }
}

impl LinkSession {
    /// Starts an initiator under `epoch` (normally the epoch named by the peer's HELLO
    /// `epoch_lo`, else my newest). Returns the session and the HS msg1 frame.
    pub fn initiator(
        identity: &GroupIdentity,
        epoch: &KeySchedule,
        proto_ver: u8,
        now_us: u64,
        rng: &mut ChaCha20Rng,
    ) -> Result<(Self, Vec<u8>)> {
        Self::initiator_inner(identity, epoch, proto_ver, now_us, rng, None)
    }

    /// Starts a responder that accepts msg1 under any retained epoch, trying the epoch
    /// named by my own HELLO hint (`epoch_lo`) first.
    pub fn responder(
        identity: &GroupIdentity,
        keyring: &EpochKeyring,
        hint_epoch_lo: Option<u16>,
        proto_ver: u8,
        now_us: u64,
        rng: &mut ChaCha20Rng,
    ) -> Self {
        Self::responder_inner(
            identity,
            keyring,
            hint_epoch_lo,
            proto_ver,
            now_us,
            rng,
            None,
        )
    }

    /// Initiator with a fixed ephemeral key (golden vectors and tests only).
    #[cfg(any(test, feature = "vectors"))]
    pub fn initiator_fixed(
        identity: &GroupIdentity,
        epoch: &KeySchedule,
        proto_ver: u8,
        now_us: u64,
        rng: &mut ChaCha20Rng,
        ephemeral: [u8; 32],
    ) -> Result<(Self, Vec<u8>)> {
        Self::initiator_inner(identity, epoch, proto_ver, now_us, rng, Some(ephemeral))
    }

    /// Responder with a fixed ephemeral key (golden vectors and tests only).
    #[cfg(any(test, feature = "vectors"))]
    pub fn responder_fixed(
        identity: &GroupIdentity,
        keyring: &EpochKeyring,
        proto_ver: u8,
        now_us: u64,
        rng: &mut ChaCha20Rng,
        ephemeral: [u8; 32],
    ) -> Self {
        Self::responder_inner(
            identity,
            keyring,
            None,
            proto_ver,
            now_us,
            rng,
            Some(ephemeral),
        )
    }

    fn base(
        role: Role,
        identity: &GroupIdentity,
        group_id: [u8; 16],
        proto_ver: u8,
        now_us: u64,
        rng: &mut ChaCha20Rng,
        fixed_ephemeral: Option<[u8; 32]>,
    ) -> Self {
        let mut seed = [0u8; 32];
        rng.fill_bytes(&mut seed);
        LinkSession {
            role,
            group_id,
            static_key: identity.x25519_secret().clone(),
            resolver_seed: Secret32::new(seed),
            builds: 0,
            proto_ver,
            started_us: now_us,
            fixed_ephemeral,
            state: State::Failed,
            peer: None,
        }
    }

    fn initiator_inner(
        identity: &GroupIdentity,
        epoch: &KeySchedule,
        proto_ver: u8,
        now_us: u64,
        rng: &mut ChaCha20Rng,
        fixed_ephemeral: Option<[u8; 32]>,
    ) -> Result<(Self, Vec<u8>)> {
        let mut s = Self::base(
            Role::Initiator,
            identity,
            *epoch.group_id(),
            proto_ver,
            now_us,
            rng,
            fixed_ephemeral,
        );
        let mut hs = s.build(epoch.epoch(), epoch.psk())?;
        let mut buf = [0u8; MAX_NOISE_MSG];
        let n = hs
            .write_message(&[], &mut buf)
            .map_err(|_| CryptoError::Handshake)?;
        let frame = hs_frame(1, proto_ver, &buf[..n])?;
        s.state = State::InitiatorWaitMsg2 {
            hs: Box::new(hs),
            epoch: epoch.epoch(),
        };
        Ok((s, frame))
    }

    fn responder_inner(
        identity: &GroupIdentity,
        keyring: &EpochKeyring,
        hint_epoch_lo: Option<u16>,
        proto_ver: u8,
        now_us: u64,
        rng: &mut ChaCha20Rng,
        fixed_ephemeral: Option<[u8; 32]>,
    ) -> Self {
        let group_id = *keyring.newest().group_id();
        let mut s = Self::base(
            Role::Responder,
            identity,
            group_id,
            proto_ver,
            now_us,
            rng,
            fixed_ephemeral,
        );
        let candidates = keyring
            .handshake_order(hint_epoch_lo)
            .into_iter()
            .map(|k| Candidate {
                epoch: k.epoch(),
                psk: k.psk().clone(),
            })
            .collect();
        s.state = State::ResponderWaitMsg1 { candidates };
        s
    }

    fn build(&mut self, epoch: u32, psk: &Secret32) -> Result<HandshakeState> {
        let params = NOISE_PATTERN.parse().map_err(|_| CryptoError::Handshake)?;
        // Each build gets its own resolver stream range (4 streams per build).
        let resolver = MeshResolver::from_seed(*self.resolver_seed.expose(), self.builds * 4);
        self.builds += 1;
        let prologue = noise_prologue(&self.group_id, epoch);
        let mut b = Builder::with_resolver(params, Box::new(resolver))
            .prologue(&prologue)
            .and_then(|b| b.psk(1, psk.expose()))
            .and_then(|b| b.local_private_key(self.static_key.expose()))
            .map_err(|_| CryptoError::Handshake)?;
        if let Some(e) = &self.fixed_ephemeral {
            b = b.fixed_ephemeral_key_for_testing_only(e);
        }
        match self.role {
            Role::Initiator => b.build_initiator(),
            Role::Responder => b.build_responder(),
        }
        .map_err(|_| CryptoError::Handshake)
    }

    pub fn role(&self) -> Role {
        self.role
    }
    pub fn is_established(&self) -> bool {
        matches!(self.state, State::Established { .. })
    }
    pub fn is_failed(&self) -> bool {
        matches!(self.state, State::Failed)
    }

    /// The handshake has not finished within 5 s of the session start.
    pub fn handshake_timed_out(&self, now_us: u64) -> bool {
        !self.is_established() && now_us.saturating_sub(self.started_us) >= HANDSHAKE_TIMEOUT_US
    }

    /// Link parameters once established.
    pub fn info(&self) -> Option<&LinkInfo> {
        match &self.state {
            State::Established { info, .. } => Some(info),
            _ => None,
        }
    }

    /// The peer's verified identity, after [`LinkSession::verify_peer_idproof`].
    pub fn peer(&self) -> Option<&PublicIdentity> {
        self.peer.as_ref()
    }

    /// Feeds one HS LinkFrame. Any error leaves the session failed (the caller sends
    /// `REJECT` with [`CryptoError::reject_reason`] and closes).
    pub fn on_handshake(&mut self, frame: &[u8], now_us: u64) -> Result<HsProgress> {
        let state = core::mem::replace(&mut self.state, State::Failed);
        let r = self.step(state, frame, now_us);
        if r.is_err() {
            self.state = State::Failed;
        }
        r
    }
}

mod handshake;

fn hs_frame(step: u8, proto_ver: u8, noise_msg: &[u8]) -> Result<Vec<u8>> {
    debug_assert_eq!(noise_msg.len(), NOISE_MSG_SIZES[usize::from(step - 1)]);
    Ok(LinkFrame::Hs {
        step,
        proto_ver,
        noise_msg,
    }
    .encode()?)
}

impl LinkSession {
    /// Seals encoded records into a SEC frame.
    pub fn seal(&mut self, records: &[u8], now_us: u64) -> Result<Vec<u8>> {
        match &mut self.state {
            State::Established { sec, .. } => sec.seal(records, now_us),
            _ => Err(CryptoError::NotEstablished),
        }
    }

    /// Opens a SEC frame. Returns `Err` before touching the replay window when the tag
    /// fails; the AAD is `lf_type ‖ kp_ctr`.
    pub fn open(&mut self, frame: &[u8], now_us: u64) -> Result<Vec<u8>> {
        match &mut self.state {
            State::Established { sec, .. } => sec.open(frame, now_us),
            _ => Err(CryptoError::NotEstablished),
        }
    }

    /// Direct access to the SEC channel (key-phase state, forced updates).
    pub fn sec(&mut self) -> Option<&mut SecChannel> {
        match &mut self.state {
            State::Established { sec, .. } => Some(sec),
            _ => None,
        }
    }

    /// My IDPROOF for this link (the first SEC record I send).
    pub fn create_idproof(&self, identity: &GroupIdentity) -> Result<IdProof> {
        let info = self.info().ok_or(CryptoError::NotEstablished)?;
        if identity.x_pk() != crate::prim::x25519_public(self.static_key.expose()) {
            return Err(CryptoError::IdentityMismatch);
        }
        Ok(idproof::create(identity, self.role, &info.handshake_hash))
    }

    /// Verifies the peer's IDPROOF against this handshake and its Noise static key.
    pub fn verify_peer_idproof(&mut self, proof: &IdProof) -> Result<PublicIdentity> {
        let info = *self.info().ok_or(CryptoError::NotEstablished)?;
        let id = idproof::verify(
            proof,
            self.role.peer(),
            &info.handshake_hash,
            &info.remote_static,
        )?;
        self.peer = Some(id);
        Ok(id)
    }
}

#[cfg(test)]
mod tests;
