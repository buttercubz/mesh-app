//! `MeshResolver`: our own `snow` `CryptoResolver` (DESIGN §8.4). `snow` is built with
//! `default-features = false`, so it has no primitives of its own; this resolver plugs in
//! X25519 (`x25519-dalek`), ChaChaPoly (`chacha20poly1305`) and SHA-256 (`sha2`) from the
//! pinned crates, and randomness from a ChaCha20Rng seeded by the engine's injected RNG.
//! Nothing here touches OS randomness.

use core::cell::Cell;

use rand_chacha::ChaCha20Rng;
use rand_core::{RngCore, SeedableRng};
use sha2::{Digest, Sha256};
use snow::params::{CipherChoice, DHChoice, HashChoice};
use snow::resolvers::CryptoResolver;
use snow::types::{Cipher, Dh, Hash, Random};
use zeroize::Zeroize;

use crate::prim;

/// Resolver for `Noise_XXpsk1_25519_ChaChaPoly_SHA256` only. Every `resolve_rng` call
/// returns an independent ChaCha20 stream of the seed drawn from the injected RNG.
pub struct MeshResolver {
    seed: [u8; 32],
    next_stream: Cell<u64>,
}

impl MeshResolver {
    /// Draws the resolver seed from the engine's RNG.
    pub fn new(rng: &mut ChaCha20Rng) -> Self {
        let mut seed = [0u8; 32];
        rng.fill_bytes(&mut seed);
        Self::from_seed(seed, 0)
    }

    /// A resolver over a seed already drawn from the injected RNG; `first_stream`
    /// separates resolvers built from the same seed (one per responder PSK candidate).
    pub fn from_seed(seed: [u8; 32], first_stream: u64) -> Self {
        MeshResolver {
            seed,
            next_stream: Cell::new(first_stream),
        }
    }
}

impl Drop for MeshResolver {
    fn drop(&mut self) {
        self.seed.zeroize();
    }
}

impl CryptoResolver for MeshResolver {
    fn resolve_rng(&self) -> Option<Box<dyn Random>> {
        let mut rng = ChaCha20Rng::from_seed(self.seed);
        let stream = self.next_stream.get();
        self.next_stream.set(stream.wrapping_add(1));
        rng.set_stream(stream);
        Some(Box::new(InjectedRandom(rng)))
    }

    fn resolve_dh(&self, choice: &DHChoice) -> Option<Box<dyn Dh>> {
        match choice {
            DHChoice::Curve25519 => Some(Box::new(X25519Dh::default())),
            _ => None,
        }
    }

    fn resolve_hash(&self, choice: &HashChoice) -> Option<Box<dyn Hash>> {
        match choice {
            HashChoice::SHA256 => Some(Box::new(Sha256Hash::default())),
            _ => None,
        }
    }

    fn resolve_cipher(&self, choice: &CipherChoice) -> Option<Box<dyn Cipher>> {
        match choice {
            CipherChoice::ChaChaPoly => Some(Box::new(ChaChaPolyCipher::default())),
            _ => None,
        }
    }
}

struct InjectedRandom(ChaCha20Rng);

impl Random for InjectedRandom {
    fn try_fill_bytes(&mut self, dest: &mut [u8]) -> Result<(), snow::Error> {
        self.0.fill_bytes(dest);
        Ok(())
    }
}

#[derive(Default)]
struct X25519Dh {
    private: [u8; 32],
    public: [u8; 32],
}

impl Drop for X25519Dh {
    fn drop(&mut self) {
        self.private.zeroize();
    }
}

impl Dh for X25519Dh {
    fn name(&self) -> &'static str {
        "25519"
    }
    fn pub_len(&self) -> usize {
        32
    }
    fn priv_len(&self) -> usize {
        32
    }
    fn set(&mut self, privkey: &[u8]) {
        let n = privkey.len().min(32);
        self.private = [0u8; 32];
        self.private[..n].copy_from_slice(&privkey[..n]);
        self.public = prim::x25519_public(&self.private);
    }
    fn generate(&mut self, rng: &mut dyn Random) -> Result<(), snow::Error> {
        let mut k = [0u8; 32];
        rng.try_fill_bytes(&mut k)?;
        self.set(&k);
        k.zeroize();
        Ok(())
    }
    fn pubkey(&self) -> &[u8] {
        &self.public
    }
    fn privkey(&self) -> &[u8] {
        &self.private
    }
    fn dh(&self, pubkey: &[u8], out: &mut [u8]) -> Result<(), snow::Error> {
        let pk: [u8; 32] = pubkey
            .get(..32)
            .and_then(|s| s.try_into().ok())
            .ok_or(snow::Error::Dh)?;
        // Refuses low-order points (all-zero shared secret).
        let shared = prim::x25519(&self.private, &pk).map_err(|_| snow::Error::Dh)?;
        let n = out.len().min(32);
        out[..n].copy_from_slice(&shared.expose()[..n]);
        Ok(())
    }
}

#[derive(Default)]
struct ChaChaPolyCipher {
    key: [u8; 32],
}

impl Drop for ChaChaPolyCipher {
    fn drop(&mut self) {
        self.key.zeroize();
    }
}

/// Noise ChaChaPoly nonce: 32 bits of zeros ‖ little-endian u64 counter.
fn noise_nonce(n: u64) -> [u8; 12] {
    let mut out = [0u8; 12];
    out[4..].copy_from_slice(&n.to_le_bytes());
    out
}

impl Cipher for ChaChaPolyCipher {
    fn name(&self) -> &'static str {
        "ChaChaPoly"
    }
    fn set(&mut self, key: &[u8; 32]) {
        self.key = *key;
    }
    fn encrypt(&self, nonce: u64, authtext: &[u8], plaintext: &[u8], out: &mut [u8]) -> usize {
        let n = plaintext.len();
        if out.len() < n + 16 {
            return 0;
        }
        out[..n].copy_from_slice(plaintext);
        let tag = prim::aead_seal(&self.key, &noise_nonce(nonce), authtext, &mut out[..n]);
        out[n..n + 16].copy_from_slice(&tag);
        n + 16
    }
    fn decrypt(
        &self,
        nonce: u64,
        authtext: &[u8],
        ciphertext: &[u8],
        out: &mut [u8],
    ) -> Result<usize, snow::Error> {
        let Some(n) = ciphertext.len().checked_sub(16) else {
            return Err(snow::Error::Decrypt);
        };
        if out.len() < n {
            return Err(snow::Error::Decrypt);
        }
        let mut tag = [0u8; 16];
        tag.copy_from_slice(&ciphertext[n..]);
        out[..n].copy_from_slice(&ciphertext[..n]);
        prim::aead_open(
            &self.key,
            &noise_nonce(nonce),
            authtext,
            &mut out[..n],
            &tag,
        )
        .map_err(|_| snow::Error::Decrypt)?;
        Ok(n)
    }
}

#[derive(Default)]
struct Sha256Hash {
    hasher: Sha256,
}

impl Hash for Sha256Hash {
    fn name(&self) -> &'static str {
        "SHA256"
    }
    fn block_len(&self) -> usize {
        64
    }
    fn hash_len(&self) -> usize {
        32
    }
    fn reset(&mut self) {
        self.hasher = Sha256::new();
    }
    fn input(&mut self, data: &[u8]) {
        self.hasher.update(data);
    }
    fn result(&mut self, out: &mut [u8]) {
        let d = self.hasher.finalize_reset();
        let n = out.len().min(32);
        out[..n].copy_from_slice(&d[..n]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_only_our_suite() {
        let mut rng = ChaCha20Rng::from_seed([0; 32]);
        let r = MeshResolver::new(&mut rng);
        assert!(r.resolve_dh(&DHChoice::Curve25519).is_some());
        assert!(r.resolve_dh(&DHChoice::Curve448).is_none());
        assert!(r.resolve_hash(&HashChoice::SHA256).is_some());
        assert!(r.resolve_hash(&HashChoice::Blake2s).is_none());
        assert!(r.resolve_cipher(&CipherChoice::ChaChaPoly).is_some());
        assert!(r.resolve_cipher(&CipherChoice::AESGCM).is_none());
    }

    #[test]
    fn rng_streams_are_independent_and_deterministic() {
        let mut rng = ChaCha20Rng::from_seed([1; 32]);
        let r = MeshResolver::new(&mut rng);
        let mut a = [0u8; 32];
        let mut b = [0u8; 32];
        r.resolve_rng().unwrap().try_fill_bytes(&mut a).unwrap();
        r.resolve_rng().unwrap().try_fill_bytes(&mut b).unwrap();
        assert_ne!(a, b);
        let mut rng2 = ChaCha20Rng::from_seed([1; 32]);
        let r2 = MeshResolver::new(&mut rng2);
        let mut c = [0u8; 32];
        r2.resolve_rng().unwrap().try_fill_bytes(&mut c).unwrap();
        assert_eq!(a, c);
    }

    #[test]
    fn sha256_hmac_matches_rfc4231() {
        let mut h = Sha256Hash::default();
        let mut out = [0u8; 32];
        h.hmac(b"Jefe", b"what do ya want for nothing?", &mut out);
        assert_eq!(
            out,
            prim::hmac_sha256(b"Jefe", &[b"what do ya want for nothing?"])
        );
    }

    #[test]
    fn cipher_round_trip_and_tamper() {
        let mut c = ChaChaPolyCipher::default();
        c.set(&[7; 32]);
        let mut ct = [0u8; 21];
        assert_eq!(c.encrypt(3, b"ad", b"hello", &mut ct), 21);
        let mut pt = [0u8; 5];
        assert_eq!(c.decrypt(3, b"ad", &ct, &mut pt).unwrap(), 5);
        assert_eq!(&pt, b"hello");
        assert!(c.decrypt(4, b"ad", &ct, &mut pt).is_err());
        assert!(c.decrypt(3, b"ad", &ct[..10], &mut pt).is_err());
    }
}
