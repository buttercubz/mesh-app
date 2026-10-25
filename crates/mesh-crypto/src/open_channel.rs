//! Open channels ("open world"): numbered public channels 1-9 that riders join without
//! an invite. Every key of channel `n` is derived deterministically from a fixed,
//! public context and `n`, so any phone on the same channel derives the same
//! `group_id`, epoch-0 secret and therefore the same Noise PSK and discovery tags.
//!
//! The channel key is **not a secret**: anyone who knows the app can derive it. It only
//! keeps channels apart (a phone on channel 4 never completes the handshake with one on
//! channel 3) and keeps casual on-air observers from reading the audio without the app.
//! Per-link Noise still gives forward-secret transport keys, and members prove fresh
//! per-session identities with IDPROOF.
//!
//! ```text
//! PRK        = HKDF-Extract(salt = "cvic/v1/open-channel", ikm = [n])
//! group_id   = HKDF-Expand(PRK, "cvic/v1/open-channel/group-id", 16)
//! secret_0   = HKDF-Expand(PRK, "cvic/v1/open-channel/epoch-secret", 32)
//! ```

use crate::prim::{self, Secret32};

/// Lowest channel number.
pub const MIN_CHANNEL: u8 = 1;
/// Highest channel number.
pub const MAX_CHANNEL: u8 = 9;

/// `group_id` (lowercase hex) of channels 1-9; pinned by a test.
pub const OPEN_CHANNEL_GROUP_IDS_HEX: [&str; 9] = [
    "d90384ef5ad82a04b0d6f6914135d9ad",
    "1e2c165867fd011b98edd952409d8c87",
    "c1c13b68546591c798f558aff95c28ab",
    "4581af6bd200844ad72d7c900eb2861c",
    "e7840e856f7f1fd852a8e14a0d12f883",
    "6db408e092582bd92d707f374f19502a",
    "09c3f3170a33caad415f2dd34bd2e8b9",
    "60f5c0031d57dce432f625b24b5e1b27",
    "574449408958d7d684619924cb73bfbe",
];

/// Public HKDF salt of every open channel.
pub const OPEN_SALT: &[u8] = b"cvic/v1/open-channel";
const LABEL_GROUP_ID: &[u8] = b"cvic/v1/open-channel/group-id";
const LABEL_SECRET: &[u8] = b"cvic/v1/open-channel/epoch-secret";

/// The deterministic keys of one open channel (epoch 0 forever: there is no leader to
/// rekey it).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenChannelKeys {
    pub channel: u8,
    pub group_id: [u8; 16],
    pub epoch_secret: Secret32,
}

/// `true` for a valid channel number (1-9).
pub const fn valid_channel(channel: u8) -> bool {
    channel >= MIN_CHANNEL && channel <= MAX_CHANNEL
}

/// Derives the keys of `channel`; `None` outside 1-9.
pub fn derive(channel: u8) -> Option<OpenChannelKeys> {
    if !valid_channel(channel) {
        return None;
    }
    let prk = prim::hkdf_extract(OPEN_SALT, &[channel]);
    let mut group_id = [0u8; 16];
    prim::hkdf_expand(prk.expose(), &[LABEL_GROUP_ID], &mut group_id);
    let epoch_secret = prim::hkdf_expand32(prk.expose(), &[LABEL_SECRET]);
    Some(OpenChannelKeys {
        channel,
        group_id,
        epoch_secret,
    })
}

/// The open channel whose `group_id` is `id`, if any.
pub fn channel_of(id: &[u8; 16]) -> Option<u8> {
    (MIN_CHANNEL..=MAX_CHANNEL).find(|&c| derive(c).is_some_and(|k| &k.group_id == id))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::KeySchedule;

    #[test]
    fn same_channel_same_keys_and_tags() {
        let a = derive(3).expect("valid");
        let b = derive(3).expect("valid");
        assert_eq!(a, b);
        let sa = KeySchedule::new(a.group_id, 0, a.epoch_secret.clone());
        let sb = KeySchedule::new(b.group_id, 0, b.epoch_secret.clone());
        assert_eq!(sa.psk().expose(), sb.psk().expose());
        assert_eq!(sa.group_tag(42), sb.group_tag(42));
    }

    #[test]
    fn neighbouring_channels_differ() {
        let a = derive(3).expect("valid");
        let b = derive(4).expect("valid");
        assert_ne!(a.group_id, b.group_id);
        assert_ne!(a.epoch_secret.expose(), b.epoch_secret.expose());
        let sa = KeySchedule::new(a.group_id, 0, a.epoch_secret.clone());
        let sb = KeySchedule::new(b.group_id, 0, b.epoch_secret.clone());
        assert_ne!(sa.psk().expose(), sb.psk().expose());
        assert_ne!(sa.group_tag(42), sb.group_tag(42));
    }

    /// Pinned vector: the UI starts channel N by this id (`apps/mobile`
    /// `OPEN_CHANNEL_GROUP_IDS`), so a change here is a protocol change.
    #[test]
    fn group_ids_are_pinned() {
        for (c, want) in OPEN_CHANNEL_GROUP_IDS_HEX.iter().enumerate() {
            let k = derive(c as u8 + 1).expect("valid");
            let hex: String = k.group_id.iter().map(|b| format!("{b:02x}")).collect();
            assert_eq!(&hex, want, "channel {}", c + 1);
        }
    }

    #[test]
    fn range_and_reverse_lookup() {
        assert!(derive(0).is_none());
        assert!(derive(10).is_none());
        for c in MIN_CHANNEL..=MAX_CHANNEL {
            let k = derive(c).expect("valid");
            assert_eq!(channel_of(&k.group_id), Some(c));
        }
        assert_eq!(channel_of(&[0u8; 16]), None);
    }
}
