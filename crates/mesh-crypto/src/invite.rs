//! Invite v1 (DESIGN §8.3): QR / deep link `convlinkvoice://join?v=1#<base64url(invite)>`.
//!
//! ```text
//! ver u8=1 · flags u8 (b0 aware-pairing suggested, b1 has-expiry) · group_id[16] · epoch u32
//! · epoch_secret[32] (CURRENT epoch) · leader_ed_pk[32] · policy u8
//! · [expires_at u32 (unix minutes) if flags.b1] · name_len u8 · name[≤ 32 B] · invite_sig[64]
//! invite_sig = Ed25519(leader, "cvic/v1/invite" ‖ group_id ‖ epoch ‖ policy ‖ name_len ‖ name
//!                      ‖ leader_ed_pk ‖ SHA-256(epoch_secret))
//! ```
//!
//! `flags` and `expires_at` are not signed (soft, leader-local settings). Because the
//! signature commits to the epoch secret only through its hash, the leader publishes it
//! in every ROSTER and any member re-assembles a byte-identical, leader-signed invite.

use mesh_wire::name::{GROUP_NAME_MAX, decode_name, truncate_graphemes};

use crate::base64url;
use crate::error::{CryptoError, Result};
use crate::identity::GroupIdentity;
use crate::prim::{self, Secret32};

/// Invite format version.
pub const INVITE_VERSION: u8 = 1;
/// URL prefix before the fragment.
pub const INVITE_URL_PREFIX: &str = "convlinkvoice://join?v=1#";
/// Signing context of `invite_sig`.
pub const INVITE_CONTEXT: &[u8] = b"cvic/v1/invite";
/// Largest encoded invite (with expiry and a 32-byte name).
pub const MAX_INVITE_LEN: usize = 1 + 1 + 16 + 4 + 32 + 32 + 1 + 4 + 1 + GROUP_NAME_MAX + 64;
/// Expiry tolerance of the soft clock check (±10 min, §8.3).
pub const EXPIRY_TOLERANCE_S: u64 = 600;

/// `flags` bit 0: Wi-Fi Aware pairing suggested.
pub const FLAG_AWARE_PAIRING: u8 = 0x01;
/// `flags` bit 1: `expires_at` present.
pub const FLAG_HAS_EXPIRY: u8 = 0x02;

/// The leader's Ed25519 signature over the invite's signed fields.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InviteSig(pub [u8; 64]);

/// Group policy byte: b7-5 `voice_ttl − 1`, b4 BLE frame (0 = 40 ms, 1 = 20 ms),
/// b3 leader priority, b2-0 reserved (0).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InvitePolicy(pub u8);

impl InvitePolicy {
    /// Compiled-in defaults: voice TTL 4, 40 ms BLE frames, leader priority on.
    pub const DEFAULT: InvitePolicy = InvitePolicy::new(4, false, true);

    /// Builds a policy. `voice_ttl` is clamped to the policy range 3-6.
    pub const fn new(voice_ttl: u8, ble_frame_20ms: bool, leader_priority: bool) -> Self {
        let ttl = if voice_ttl < 3 {
            3
        } else if voice_ttl > 6 {
            6
        } else {
            voice_ttl
        };
        InvitePolicy(
            ((ttl - 1) << 5) | ((ble_frame_20ms as u8) << 4) | ((leader_priority as u8) << 3),
        )
    }
    pub const fn voice_ttl(self) -> u8 {
        (self.0 >> 5) + 1
    }
    pub const fn ble_frame_20ms(self) -> bool {
        self.0 & 0x10 != 0
    }
    pub const fn leader_priority(self) -> bool {
        self.0 & 0x08 != 0
    }
}

/// A decoded invite. The epoch secret zeroizes on drop.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Invite {
    pub flags: u8,
    pub group_id: [u8; 16],
    pub epoch: u32,
    pub epoch_secret: Secret32,
    pub leader_ed_pk: [u8; 32],
    pub policy: InvitePolicy,
    /// Unix minutes; present exactly when `flags` has [`FLAG_HAS_EXPIRY`].
    pub expires_at: Option<u32>,
    /// Group name (UTF-8, ≤ 32 B; encoders truncate at a grapheme boundary).
    pub name: String,
    pub invite_sig: InviteSig,
}

/// The exact message covered by `invite_sig`.
pub fn invite_signed_message(
    group_id: &[u8; 16],
    epoch: u32,
    policy: InvitePolicy,
    name: &str,
    leader_ed_pk: &[u8; 32],
    epoch_secret: &Secret32,
) -> Vec<u8> {
    let name = truncate_graphemes(name, GROUP_NAME_MAX);
    let mut m = Vec::with_capacity(INVITE_CONTEXT.len() + 16 + 4 + 1 + 1 + name.len() + 64);
    m.extend_from_slice(INVITE_CONTEXT);
    m.extend_from_slice(group_id);
    m.extend_from_slice(&epoch.to_le_bytes());
    m.push(policy.0);
    m.push(name.len() as u8);
    m.extend_from_slice(name.as_bytes());
    m.extend_from_slice(leader_ed_pk);
    m.extend_from_slice(&prim::sha256(&[epoch_secret.expose()]));
    m
}

/// The leader signs this epoch's invite (published in every ROSTER, §4.7).
pub fn sign_invite(
    leader: &GroupIdentity,
    group_id: &[u8; 16],
    epoch: u32,
    policy: InvitePolicy,
    name: &str,
    epoch_secret: &Secret32,
) -> InviteSig {
    let m = invite_signed_message(group_id, epoch, policy, name, &leader.ed_pk(), epoch_secret);
    InviteSig(leader.sign(&m))
}

impl Invite {
    /// Builds an invite from the group state a member holds plus the `invite_sig` of the
    /// current epoch taken from the verified ROSTER. The result is byte-identical to the
    /// leader's own invite when `flags` and `expires_at` match.
    #[allow(clippy::too_many_arguments)]
    pub fn assemble(
        group_id: [u8; 16],
        epoch: u32,
        epoch_secret: Secret32,
        leader_ed_pk: [u8; 32],
        policy: InvitePolicy,
        name: &str,
        invite_sig: InviteSig,
        aware_pairing: bool,
        expires_at: Option<u32>,
    ) -> Self {
        let mut flags = 0u8;
        if aware_pairing {
            flags |= FLAG_AWARE_PAIRING;
        }
        if expires_at.is_some() {
            flags |= FLAG_HAS_EXPIRY;
        }
        Invite {
            flags,
            group_id,
            epoch,
            epoch_secret,
            leader_ed_pk,
            policy,
            expires_at,
            name: truncate_graphemes(name, GROUP_NAME_MAX).to_owned(),
            invite_sig,
        }
    }

    /// The signed message of this invite.
    pub fn signed_message(&self) -> Vec<u8> {
        invite_signed_message(
            &self.group_id,
            self.epoch,
            self.policy,
            &self.name,
            &self.leader_ed_pk,
            &self.epoch_secret,
        )
    }

    /// `leaderSigOk`: the signature verifies against the invite's own `leader_ed_pk`.
    pub fn verify_leader_sig(&self) -> Result<()> {
        prim::ed25519_verify(
            &self.leader_ed_pk,
            &self.signed_message(),
            &self.invite_sig.0,
        )
    }

    /// Soft expiry check against the phone's clock with ±10 min tolerance.
    pub fn is_expired(&self, now_unix_s: u64) -> bool {
        match self.expires_at {
            None => false,
            Some(min) => now_unix_s > u64::from(min) * 60 + EXPIRY_TOLERANCE_S,
        }
    }

    pub fn aware_pairing(&self) -> bool {
        self.flags & FLAG_AWARE_PAIRING != 0
    }

    /// Binary encoding.
    pub fn encode(&self) -> Vec<u8> {
        let name = truncate_graphemes(&self.name, GROUP_NAME_MAX);
        let mut flags = self.flags & !FLAG_HAS_EXPIRY;
        if self.expires_at.is_some() {
            flags |= FLAG_HAS_EXPIRY;
        }
        let mut o = Vec::with_capacity(MAX_INVITE_LEN);
        o.push(INVITE_VERSION);
        o.push(flags);
        o.extend_from_slice(&self.group_id);
        o.extend_from_slice(&self.epoch.to_le_bytes());
        o.extend_from_slice(self.epoch_secret.expose());
        o.extend_from_slice(&self.leader_ed_pk);
        o.push(self.policy.0);
        if let Some(e) = self.expires_at {
            o.extend_from_slice(&e.to_le_bytes());
        }
        o.push(name.len() as u8);
        o.extend_from_slice(name.as_bytes());
        o.extend_from_slice(&self.invite_sig.0);
        o
    }

    /// Bounded decoder.
    pub fn decode(b: &[u8]) -> Result<Self> {
        if b.len() > MAX_INVITE_LEN {
            return Err(CryptoError::BadInvite("invite too long"));
        }
        let mut r = Cursor { b, pos: 0 };
        if r.u8()? != INVITE_VERSION {
            return Err(CryptoError::BadInvite("unsupported invite version"));
        }
        let flags = r.u8()?;
        let group_id = r.arr::<16>()?;
        let epoch = u32::from_le_bytes(r.arr()?);
        let epoch_secret = Secret32::new(r.arr()?);
        let leader_ed_pk = r.arr()?;
        let policy = InvitePolicy(r.u8()?);
        let expires_at = if flags & FLAG_HAS_EXPIRY != 0 {
            Some(u32::from_le_bytes(r.arr()?))
        } else {
            None
        };
        let name_len = usize::from(r.u8()?);
        if name_len > GROUP_NAME_MAX {
            return Err(CryptoError::BadInvite("group name too long"));
        }
        let name = decode_name(r.take(name_len)?)
            .map_err(|_| CryptoError::BadInvite("group name is not UTF-8"))?
            .to_owned();
        let invite_sig = InviteSig(r.arr()?);
        if r.pos != b.len() {
            return Err(CryptoError::BadInvite("trailing bytes"));
        }
        Ok(Invite {
            flags,
            group_id,
            epoch,
            epoch_secret,
            leader_ed_pk,
            policy,
            expires_at,
            name,
            invite_sig,
        })
    }

    /// `convlinkvoice://join?v=1#<base64url(invite)>`.
    pub fn to_url(&self) -> String {
        let mut s = String::from(INVITE_URL_PREFIX);
        s.push_str(&base64url::encode(&self.encode()));
        s
    }

    /// Parses a deep link or scanned QR text. The invite lives in the fragment.
    pub fn from_url(url: &str) -> Result<Self> {
        let url = url.trim();
        let (head, fragment) = url
            .split_once('#')
            .ok_or(CryptoError::BadInvite("missing fragment"))?;
        let head = head.to_ascii_lowercase();
        let Some(query) = head.strip_prefix("convlinkvoice://join?") else {
            return Err(CryptoError::BadInvite("not a convlinkvoice join link"));
        };
        if !query.split('&').any(|kv| kv == "v=1") {
            return Err(CryptoError::BadInvite("unsupported link version"));
        }
        Self::decode(&base64url::decode(fragment, MAX_INVITE_LEN)?)
    }
}

/// Minimal bounded cursor (the wire crate's reader is private to it).
struct Cursor<'a> {
    b: &'a [u8],
    pos: usize,
}

impl<'a> Cursor<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8]> {
        let end = self
            .pos
            .checked_add(n)
            .filter(|&e| e <= self.b.len())
            .ok_or(CryptoError::BadInvite("truncated"))?;
        let s = &self.b[self.pos..end];
        self.pos = end;
        Ok(s)
    }
    fn arr<const N: usize>(&mut self) -> Result<[u8; N]> {
        let mut out = [0u8; N];
        out.copy_from_slice(self.take(N)?);
        Ok(out)
    }
    fn u8(&mut self) -> Result<u8> {
        Ok(self.arr::<1>()?[0])
    }
}

#[cfg(test)]
mod tests;
