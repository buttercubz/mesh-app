//! Handshake transitions of [`LinkSession`].

use mesh_wire::PROTOCOL_VERSION;
use mesh_wire::frame::LinkFrame;
use snow::HandshakeState;

use super::{HsProgress, LinkInfo, LinkSession, MAX_NOISE_MSG, Role, State, hs_frame};
use crate::error::{CryptoError, Result};
use crate::prim::Secret32;
use crate::sec::SecChannel;

impl LinkSession {
    pub(super) fn step(&mut self, state: State, frame: &[u8], now_us: u64) -> Result<HsProgress> {
        let LinkFrame::Hs {
            step,
            proto_ver,
            noise_msg,
        } = LinkFrame::decode(frame)?
        else {
            return Err(CryptoError::WrongFrameType);
        };
        if noise_msg.len() > MAX_NOISE_MSG {
            return Err(CryptoError::Handshake);
        }
        let mut payload = [0u8; MAX_NOISE_MSG];
        match (state, step) {
            (State::ResponderWaitMsg1 { candidates }, 1) => {
                if proto_ver != PROTOCOL_VERSION || proto_ver != self.proto_ver {
                    return Err(CryptoError::Version);
                }
                // Try every retained PSK; each attempt gets a fresh HandshakeState
                // because a failed read leaves snow's state unusable.
                let mut accepted = None;
                for c in &candidates {
                    let mut hs = self.build(c.epoch, &c.psk)?;
                    if hs.read_message(noise_msg, &mut payload).is_ok() {
                        accepted = Some((hs, c.epoch));
                        break;
                    }
                }
                // Wrong PSK, epoch or group: refuse before revealing the static key.
                let (mut hs, epoch) = accepted.ok_or(CryptoError::Handshake)?;
                let mut out = [0u8; MAX_NOISE_MSG];
                let n = hs
                    .write_message(&[], &mut out)
                    .map_err(|_| CryptoError::Handshake)?;
                let reply = hs_frame(2, self.proto_ver, &out[..n])?;
                self.state = State::ResponderWaitMsg3 {
                    hs: Box::new(hs),
                    epoch,
                };
                Ok(HsProgress {
                    send: Some(reply),
                    established: false,
                })
            }
            (State::InitiatorWaitMsg2 { mut hs, epoch }, 2) => {
                if proto_ver != self.proto_ver {
                    return Err(CryptoError::Version);
                }
                hs.read_message(noise_msg, &mut payload)
                    .map_err(|_| CryptoError::Handshake)?;
                let mut out = [0u8; MAX_NOISE_MSG];
                let n = hs
                    .write_message(&[], &mut out)
                    .map_err(|_| CryptoError::Handshake)?;
                let reply = hs_frame(3, self.proto_ver, &out[..n])?;
                self.finish(*hs, epoch, now_us)?;
                Ok(HsProgress {
                    send: Some(reply),
                    established: true,
                })
            }
            (State::ResponderWaitMsg3 { mut hs, epoch }, 3) => {
                if proto_ver != self.proto_ver {
                    return Err(CryptoError::Version);
                }
                hs.read_message(noise_msg, &mut payload)
                    .map_err(|_| CryptoError::Handshake)?;
                self.finish(*hs, epoch, now_us)?;
                Ok(HsProgress {
                    send: None,
                    established: true,
                })
            }
            _ => Err(CryptoError::HandshakeOrder),
        }
    }

    fn finish(&mut self, mut hs: HandshakeState, epoch: u32, now_us: u64) -> Result<()> {
        if !hs.is_handshake_finished() {
            return Err(CryptoError::HandshakeOrder);
        }
        let mut handshake_hash = [0u8; 32];
        handshake_hash.copy_from_slice(
            hs.get_handshake_hash()
                .get(..32)
                .ok_or(CryptoError::Handshake)?,
        );
        let mut remote_static = [0u8; 32];
        remote_static.copy_from_slice(
            hs.get_remote_static()
                .and_then(|r| r.get(..32))
                .ok_or(CryptoError::Handshake)?,
        );
        // (c1, c2): c1 seals initiator→responder, c2 responder→initiator.
        let (c1, c2) = hs.dangerously_get_raw_split();
        let (c1, c2) = (Secret32::new(c1), Secret32::new(c2));
        let (tx, rx) = match self.role {
            Role::Initiator => (c1, c2),
            Role::Responder => (c2, c1),
        };
        self.state = State::Established {
            info: LinkInfo {
                role: self.role,
                epoch,
                proto_ver: self.proto_ver,
                handshake_hash,
                remote_static,
            },
            sec: Box::new(SecChannel::new(tx, rx, now_us)),
        };
        Ok(())
    }
}
