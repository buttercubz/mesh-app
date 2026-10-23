//! SEC frame sealing and opening with in-band key update (DESIGN §4.3).
//!
//! - Frame: `lf_type 0x01 · kp_ctr u32 · ciphertext · tag[16]`.
//! - Nonce `0x00000000 ‖ u64(kp_ctr)` (LE), AAD `lf_type ‖ kp_ctr` (5 B).
//! - Each direction updates its own key after 12 h or when `ctr` reaches 2³⁰:
//!   `k_next = HKDF-Expand(PRK = k_cur, "cvic/v1/keyupdate", 32)`, flip the phase bit,
//!   restart `ctr` at 0, zeroize `k_cur`.
//! - A receiver that sees the other phase bit derives `k_next` lazily and commits to it
//!   only if the tag verifies; it keeps the previous key for 10 s for reordered frames.
//! - One 1024-bit replay window per direction and phase, updated only after the tag
//!   verifies.

use mesh_wire::WireError;
use mesh_wire::frame::{KEY_UPDATE_CTR, KEY_UPDATE_LABEL, KpCtr, LfType, LinkFrame, TAG_LEN};
use mesh_wire::link::MAX_DATAGRAM_FRAME;

use crate::error::{CryptoError, Result};
use crate::prim::{self, Secret32};
use crate::replay::ReplayWindow;

/// Key update after 12 h of use (µs).
pub const KEY_UPDATE_AFTER_US: u64 = 12 * 3600 * 1_000_000;
/// The previous receive key is kept 10 s after a phase change (µs).
pub const OLD_KEY_GRACE_US: u64 = 10_000_000;

/// `k_next = HKDF-Expand(PRK = k_cur, "cvic/v1/keyupdate", 32)`.
pub fn next_key(k: &Secret32) -> Secret32 {
    prim::hkdf_expand32(k.expose(), &[KEY_UPDATE_LABEL])
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct TxDirection {
    key: Secret32,
    phase: bool,
    ctr: u32,
    phase_started_us: u64,
}

impl TxDirection {
    fn update(&mut self, now_us: u64) {
        self.key = next_key(&self.key);
        self.phase = !self.phase;
        self.ctr = 0;
        self.phase_started_us = now_us;
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct RxPhase {
    key: Secret32,
    phase: bool,
    window: ReplayWindow,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct RxDirection {
    current: RxPhase,
    /// Previous phase and the time (µs) at which it is zeroized.
    previous: Option<(RxPhase, u64)>,
}

/// Both directions of an established link.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SecChannel {
    tx: TxDirection,
    rx: RxDirection,
}

impl SecChannel {
    /// Starts both directions in phase 0 with counter 0.
    pub fn new(tx_key: Secret32, rx_key: Secret32, now_us: u64) -> Self {
        SecChannel {
            tx: TxDirection {
                key: tx_key,
                phase: false,
                ctr: 0,
                phase_started_us: now_us,
            },
            rx: RxDirection {
                current: RxPhase {
                    key: rx_key,
                    phase: false,
                    window: ReplayWindow::new(),
                },
                previous: None,
            },
        }
    }

    /// Current transmit `(phase, next ctr)`.
    pub fn tx_state(&self) -> (bool, u32) {
        (self.tx.phase, self.tx.ctr)
    }

    /// Current receive phase and whether the previous phase key is still retained.
    pub fn rx_state(&self) -> (bool, bool) {
        (self.rx.current.phase, self.rx.previous.is_some())
    }

    /// The current transmit key (golden vectors and tests only).
    #[cfg(any(test, feature = "vectors"))]
    pub fn tx_key(&self) -> &Secret32 {
        &self.tx.key
    }

    /// Forces this direction's key update now (also done automatically by `seal`).
    pub fn update_tx_key(&mut self, now_us: u64) {
        self.tx.update(now_us);
    }

    /// Seals `plaintext` (encoded records) into a complete SEC LinkFrame.
    pub fn seal(&mut self, plaintext: &[u8], now_us: u64) -> Result<Vec<u8>> {
        let total = 1 + 4 + plaintext.len() + TAG_LEN;
        if total > MAX_DATAGRAM_FRAME {
            return Err(CryptoError::Wire(WireError::TooLong {
                what: "SEC frame",
                len: total,
                max: MAX_DATAGRAM_FRAME,
            }));
        }
        if self.tx.ctr >= KEY_UPDATE_CTR
            || now_us.saturating_sub(self.tx.phase_started_us) >= KEY_UPDATE_AFTER_US
        {
            self.tx.update(now_us);
        }
        let kp = KpCtr::new(self.tx.phase, self.tx.ctr);
        if kp.ctr() != self.tx.ctr {
            return Err(CryptoError::CounterExhausted);
        }
        self.tx.ctr += 1;
        let mut out = Vec::with_capacity(total);
        out.push(LfType::Sec as u8);
        out.extend_from_slice(&kp.0.to_le_bytes());
        out.extend_from_slice(plaintext);
        let tag = prim::aead_seal(self.tx.key.expose(), &kp.nonce(), &kp.aad(), &mut out[5..]);
        out.extend_from_slice(&tag);
        Ok(out)
    }

    /// Opens a complete SEC LinkFrame. On any error the channel state is unchanged:
    /// the replay window moves and a new receive phase is committed only after the tag
    /// verified.
    pub fn open(&mut self, frame: &[u8], now_us: u64) -> Result<Vec<u8>> {
        match LinkFrame::decode(frame)? {
            LinkFrame::Sec { kp_ctr, sealed } => self.open_parts(kp_ctr, sealed, now_us),
            _ => Err(CryptoError::WrongFrameType),
        }
    }

    /// Opens `ciphertext ‖ tag` sealed under `kp_ctr`.
    pub fn open_parts(&mut self, kp: KpCtr, sealed: &[u8], now_us: u64) -> Result<Vec<u8>> {
        if let Some((_, expires)) = &self.rx.previous
            && now_us >= *expires
        {
            // Dropping the RxPhase zeroizes its key.
            self.rx.previous = None;
        }
        let Some(ct_len) = sealed.len().checked_sub(TAG_LEN) else {
            return Err(CryptoError::Decrypt);
        };
        let mut tag = [0u8; TAG_LEN];
        tag.copy_from_slice(&sealed[ct_len..]);
        let ctr = kp.ctr();
        let decrypt = |key: &Secret32| -> Result<Vec<u8>> {
            let mut buf = sealed[..ct_len].to_vec();
            prim::aead_open(key.expose(), &kp.nonce(), &kp.aad(), &mut buf, &tag)?;
            Ok(buf)
        };

        if kp.phase() == self.rx.current.phase {
            if !self.rx.current.window.check(ctr) {
                return Err(CryptoError::Replay);
            }
            let pt = decrypt(&self.rx.current.key)?;
            self.rx.current.window.commit(ctr);
            return Ok(pt);
        }

        if let Some((prev, _)) = &mut self.rx.previous {
            // Within 10 s of a phase change the other phase bit is the old phase: the
            // sender cannot update again that fast (12 h or 2³⁰ frames).
            if !prev.window.check(ctr) {
                return Err(CryptoError::Replay);
            }
            let pt = decrypt(&prev.key)?;
            prev.window.commit(ctr);
            return Ok(pt);
        }

        // The sender moved to the next phase: derive lazily, commit only on success.
        let next = next_key(&self.rx.current.key);
        let pt = decrypt(&next)?;
        let mut window = ReplayWindow::new();
        window.commit(ctr);
        let new_current = RxPhase {
            key: next,
            phase: kp.phase(),
            window,
        };
        let old = core::mem::replace(&mut self.rx.current, new_current);
        self.rx.previous = Some((old, now_us.saturating_add(OLD_KEY_GRACE_US)));
        Ok(pt)
    }
}

#[cfg(test)]
mod tests;
