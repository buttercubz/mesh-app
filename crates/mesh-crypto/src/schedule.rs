//! Per-epoch key schedule (DESIGN §8.3), HKDF-SHA256:
//!
//! ```text
//! PRK_e      = HKDF-Extract(salt = group_id, IKM = epoch_secret_e)
//! psk_e      = HKDF-Expand(PRK_e, "cvic/v1/noise-psk", 32)
//! disc_key_e = HKDF-Expand(PRK_e, "cvic/v1/discovery", 32)
//! t10        = floor(unix_seconds / 600)
//! group_tag  = HMAC-SHA256(disc_key_e, "gt" ‖ u32 t10)[0..4]
//! node_token = HMAC-SHA256(disc_key_e, "nt" ‖ u32 t10 ‖ id_hash)[0..4]
//! aware_pmk  = HKDF-Expand(PRK_e, "cvic/v1/aware-pmk" ‖ ble_handshake_hash, 32)
//! tunnel_rdv = HKDF-Expand(PRK_e, "cvic/v1/tunnel-rendezvous", 16)
//! lab_tag    = SHA-256("cvic/lab" ‖ u16 lab_code)[0..2]
//! ```

use crate::prim::{self, Secret32};

/// Token rotation slot length (600 s, Appendix A).
pub const T10_SECONDS: u64 = 600;

pub const LABEL_PSK: &[u8] = b"cvic/v1/noise-psk";
pub const LABEL_DISCOVERY: &[u8] = b"cvic/v1/discovery";
pub const LABEL_AWARE_PMK: &[u8] = b"cvic/v1/aware-pmk";
pub const LABEL_TUNNEL_RDV: &[u8] = b"cvic/v1/tunnel-rendezvous";

/// `floor(unix_seconds / 600)` as the u32 that goes into the HMACs.
pub fn t10(unix_seconds: u64) -> u32 {
    (unix_seconds / T10_SECONDS) as u32
}

/// The three accepted slots around `t10` (clock skew ±1), current first.
pub fn t10_window(t10: u32) -> [u32; 3] {
    [t10, t10.wrapping_sub(1), t10.wrapping_add(1)]
}

/// `lab_tag` (not a secret; re-exported from mesh-wire, which owns LAB framing).
pub fn lab_tag(lab_code: u16) -> u16 {
    mesh_wire::frame::lab_tag(lab_code)
}

/// All keys of one epoch, derived once from `(group_id, epoch, epoch_secret)`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeySchedule {
    group_id: [u8; 16],
    epoch: u32,
    epoch_secret: Secret32,
    prk: Secret32,
    psk: Secret32,
    disc_key: Secret32,
}

impl KeySchedule {
    pub fn new(group_id: [u8; 16], epoch: u32, epoch_secret: Secret32) -> Self {
        let prk = prim::hkdf_extract(&group_id, epoch_secret.expose());
        let psk = prim::hkdf_expand32(prk.expose(), &[LABEL_PSK]);
        let disc_key = prim::hkdf_expand32(prk.expose(), &[LABEL_DISCOVERY]);
        KeySchedule {
            group_id,
            epoch,
            epoch_secret,
            prk,
            psk,
            disc_key,
        }
    }

    pub fn group_id(&self) -> &[u8; 16] {
        &self.group_id
    }
    pub fn epoch(&self) -> u32 {
        self.epoch
    }
    /// Low 16 bits of the epoch (HELLO `epoch_lo`, PRESENCE `epoch16`).
    pub fn epoch_lo(&self) -> u16 {
        self.epoch as u16
    }
    pub fn epoch_secret(&self) -> &Secret32 {
        &self.epoch_secret
    }
    /// `PRK_e`.
    pub fn prk(&self) -> &Secret32 {
        &self.prk
    }
    /// Noise XXpsk1 PSK.
    pub fn psk(&self) -> &Secret32 {
        &self.psk
    }
    pub fn disc_key(&self) -> &Secret32 {
        &self.disc_key
    }

    /// `HMAC-SHA256(disc_key, "gt" ‖ u32 t10)[0..4]`.
    pub fn group_tag(&self, t10: u32) -> [u8; 4] {
        let m = prim::hmac_sha256(self.disc_key.expose(), &[b"gt", &t10.to_le_bytes()]);
        [m[0], m[1], m[2], m[3]]
    }

    /// `HMAC-SHA256(disc_key, "nt" ‖ u32 t10 ‖ id_hash)[0..4]`.
    pub fn node_token(&self, t10: u32, id_hash: &[u8; 16]) -> [u8; 4] {
        let m = prim::hmac_sha256(
            self.disc_key.expose(),
            &[b"nt", &t10.to_le_bytes(), id_hash],
        );
        [m[0], m[1], m[2], m[3]]
    }

    /// `t10` slot (current, −1 or +1) whose group tag equals `tag`.
    pub fn match_group_tag(&self, tag: &[u8; 4], t10: u32) -> Option<u32> {
        t10_window(t10)
            .into_iter()
            .find(|&t| &self.group_tag(t) == tag)
    }

    /// Maps a node token to a member: the first `id_hash` (in the given order) whose
    /// token matches in the ±1 slot window. At most 3 × 31 HMACs.
    pub fn match_node_token<'a>(
        &self,
        token: &[u8; 4],
        t10: u32,
        members: impl IntoIterator<Item = &'a [u8; 16]> + Clone,
    ) -> Option<[u8; 16]> {
        for t in t10_window(t10) {
            if let Some(m) = members
                .clone()
                .into_iter()
                .find(|id| &self.node_token(t, id) == token)
            {
                return Some(*m);
            }
        }
        None
    }

    /// `HKDF-Expand(PRK, "cvic/v1/aware-pmk" ‖ ble_handshake_hash, 32)`.
    pub fn aware_pmk(&self, ble_handshake_hash: &[u8; 32]) -> Secret32 {
        prim::hkdf_expand32(self.prk.expose(), &[LABEL_AWARE_PMK, ble_handshake_hash])
    }

    /// `HKDF-Expand(PRK, "cvic/v1/tunnel-rendezvous", 16)` (v2 hook only).
    pub fn tunnel_rdv(&self) -> [u8; 16] {
        let mut out = [0u8; 16];
        prim::hkdf_expand(self.prk.expose(), &[LABEL_TUNNEL_RDV], &mut out);
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sched(epoch: u32, fill: u8) -> KeySchedule {
        KeySchedule::new([0xA5; 16], epoch, Secret32::new([fill; 32]))
    }

    #[test]
    fn derivations_follow_the_labels() {
        let s = sched(0, 7);
        let prk = prim::hkdf_extract(&[0xA5; 16], &[7; 32]);
        assert_eq!(s.prk(), &prk);
        let mut psk = [0u8; 32];
        prim::hkdf_expand(prk.expose(), &[b"cvic/v1/noise-psk"], &mut psk);
        assert_eq!(s.psk().expose(), &psk);
        assert_ne!(s.psk(), s.disc_key());
        let tag = prim::hmac_sha256(s.disc_key().expose(), &[b"gt", &5u32.to_le_bytes()]);
        assert_eq!(s.group_tag(5), [tag[0], tag[1], tag[2], tag[3]]);
        assert_ne!(s.tunnel_rdv(), [0; 16]);
        assert_ne!(s.aware_pmk(&[1; 32]), s.aware_pmk(&[2; 32]));
        // Keys depend on (group_id, epoch_secret); the epoch number is bound by the
        // Noise prologue instead.
        assert_eq!(sched(1, 7).psk(), s.psk());
        assert_ne!(sched(1, 8).psk(), s.psk());
        assert_ne!(
            KeySchedule::new([0; 16], 0, Secret32::new([7; 32])).psk(),
            s.psk()
        );
    }

    #[test]
    fn t10_and_skew_window() {
        assert_eq!(t10(0), 0);
        assert_eq!(t10(599), 0);
        assert_eq!(t10(600), 1);
        assert_eq!(t10(1_761_000_000), 2_935_000);
        let s = sched(2, 1);
        let tag = s.group_tag(100);
        assert_eq!(s.match_group_tag(&tag, 100), Some(100));
        assert_eq!(s.match_group_tag(&tag, 101), Some(100));
        assert_eq!(s.match_group_tag(&tag, 99), Some(100));
        assert_eq!(s.match_group_tag(&tag, 102), None);
    }

    #[test]
    fn node_tokens_map_to_members() {
        let s = sched(0, 3);
        let members: Vec<[u8; 16]> = (0..31u8).map(|i| [i; 16]).collect();
        let tok = s.node_token(50, &members[17]);
        assert_eq!(
            s.match_node_token(&tok, 51, members.iter()),
            Some(members[17])
        );
        assert_eq!(s.match_node_token(&tok, 53, members.iter()), None);
        assert_ne!(s.node_token(50, &members[1]), s.node_token(51, &members[1]));
        assert_eq!(lab_tag(1234), mesh_wire::frame::lab_tag(1234));
    }
}
