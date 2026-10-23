//! MSRV probe: exercises the public API surface the workspace relies on from every
//! pinned dependency, so `cargo +<pin> check` proves the pinned toolchain can build
//! the whole pinned graph with the intended features (DESIGN §2.6).
//! Not part of the Cargo workspace and never shipped.

#![forbid(unsafe_code)]

// Crates used only by build scripts, tools, tests or the wasm32 guard: linking them is
// the point of the probe; their APIs are exercised by the owning work packages.
use cbindgen as _;
use cc as _;
use getrandom as _;
use proptest as _;
use uniffi as _;

use chacha20poly1305::aead::{Aead, KeyInit, Payload};
use chacha20poly1305::{ChaCha20Poly1305, Nonce};
use hkdf::Hkdf;
use hmac::{Hmac, Mac};
use rand_chacha::ChaCha20Rng;
use rand_core::{RngCore, SeedableRng};
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use zeroize::Zeroize;

/// A UI-contract style type: serde + ts-rs derive (mesh-api pattern).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
#[serde(rename_all = "camelCase")]
pub struct ProbeEvent {
    pub node_id: u32,
    pub label: String,
}

/// Derives deterministic X25519 + Ed25519 identities from an injected seeded RNG
/// (no OS randomness), mirroring the mesh-crypto key generation rule.
pub fn deterministic_identity(seed: [u8; 32]) -> ([u8; 32], [u8; 32]) {
    let mut rng = ChaCha20Rng::from_seed(seed);
    let mut dh = [0u8; 32];
    let mut sig = [0u8; 32];
    rng.fill_bytes(&mut dh);
    rng.fill_bytes(&mut sig);
    let dh_secret = x25519_dalek::StaticSecret::from(dh);
    let dh_public = x25519_dalek::PublicKey::from(&dh_secret);
    let signing = ed25519_dalek::SigningKey::from_bytes(&sig);
    let verifying = signing.verifying_key();
    dh.zeroize();
    sig.zeroize();
    (dh_public.to_bytes(), verifying.to_bytes())
}

/// HKDF-SHA256 + HMAC-SHA256 + ChaCha20-Poly1305 round trip (link AEAD pattern).
pub fn aead_round_trip(ikm: &[u8], plaintext: &[u8]) -> Option<Vec<u8>> {
    let hk = Hkdf::<Sha256>::new(None, ikm);
    let mut key = [0u8; 32];
    hk.expand(b"msrv-probe/v1", &mut key).ok()?;
    let mut mac = <Hmac<Sha256> as Mac>::new_from_slice(&key).ok()?;
    mac.update(plaintext);
    let tag = mac.finalize().into_bytes();
    let cipher = ChaCha20Poly1305::new_from_slice(&key).ok()?;
    key.zeroize();
    let nonce = Nonce::default();
    let sealed = cipher
        .encrypt(
            &nonce,
            Payload {
                msg: plaintext,
                aad: &tag,
            },
        )
        .ok()?;
    cipher
        .decrypt(
            &nonce,
            Payload {
                msg: &sealed,
                aad: &tag,
            },
        )
        .ok()
}

/// Noise pattern string of DESIGN Appendix A parses with the pinned snow.
pub fn noise_params_parse() -> bool {
    "Noise_XXpsk1_25519_ChaChaPoly_SHA256"
        .parse::<snow::params::NoiseParams>()
        .is_ok()
}

/// Wait-free SPSC ring (RT audio plane) + MPSC command channel (engine thread).
pub fn rings_and_channels() -> Option<i16> {
    let (mut producer, mut consumer) = rtrb::RingBuffer::<i16>::new(4096);
    producer.push(7).ok()?;
    let (tx, rx) = crossbeam_channel::bounded::<i16>(2048);
    tx.send(consumer.pop().ok()?).ok()?;
    rx.recv().ok()
}

/// Denoiser frame size used by the capture chain.
pub fn denoise_frame_size() -> usize {
    nnnoiseless::DenoiseState::FRAME_SIZE
}

/// serde_json round trip of the UI-contract type.
pub fn json_round_trip(event: &ProbeEvent) -> Option<ProbeEvent> {
    let text = serde_json::to_string(event).ok()?;
    serde_json::from_str(&text).ok()
}
