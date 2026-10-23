//! Thin wrappers over the pinned primitive crates (SHA-256, HMAC, HKDF, X25519, Ed25519,
//! ChaCha20-Poly1305) plus [`Secret32`], a 32-byte secret that zeroizes on drop.
//!
//! Randomness only ever comes from the injected [`ChaCha20Rng`].

use core::fmt;

use chacha20poly1305::aead::AeadInPlace;
use chacha20poly1305::{ChaCha20Poly1305, KeyInit, Tag};
use ed25519_dalek::{Signature, Signer, SigningKey, VerifyingKey};
use hkdf::Hkdf;
use hmac::{Hmac, Mac};
use rand_chacha::ChaCha20Rng;
use rand_core::RngCore;
use sha2::{Digest, Sha256};
use zeroize::Zeroize;

use crate::error::{CryptoError, Result};

/// A 32-byte secret (key, seed, epoch secret) that is zeroized when dropped and never
/// printed by `Debug`.
#[derive(Clone)]
pub struct Secret32([u8; 32]);

/// Constant-time comparison (no early exit on the first differing byte).
impl PartialEq for Secret32 {
    fn eq(&self, other: &Self) -> bool {
        self.0
            .iter()
            .zip(other.0.iter())
            .fold(0u8, |acc, (a, b)| acc | (a ^ b))
            == 0
    }
}

impl Eq for Secret32 {}

impl Secret32 {
    /// Wraps raw secret bytes.
    pub const fn new(bytes: [u8; 32]) -> Self {
        Secret32(bytes)
    }

    /// Draws 32 bytes from the injected RNG.
    pub fn random(rng: &mut ChaCha20Rng) -> Self {
        let mut b = [0u8; 32];
        rng.fill_bytes(&mut b);
        Secret32(b)
    }

    /// The secret bytes.
    pub fn expose(&self) -> &[u8; 32] {
        &self.0
    }
}

impl Drop for Secret32 {
    fn drop(&mut self) {
        self.0.zeroize();
    }
}

impl fmt::Debug for Secret32 {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Secret32(..)")
    }
}

/// SHA-256 over the concatenation of `parts`.
pub fn sha256(parts: &[&[u8]]) -> [u8; 32] {
    let mut h = Sha256::new();
    for p in parts {
        h.update(p);
    }
    h.finalize().into()
}

/// HMAC-SHA256(`key`, concatenation of `parts`).
pub fn hmac_sha256(key: &[u8], parts: &[&[u8]]) -> [u8; 32] {
    // HMAC accepts keys of any length; the error branch is unreachable.
    let mut m = match <Hmac<Sha256> as Mac>::new_from_slice(key) {
        Ok(m) => m,
        Err(_) => return [0u8; 32],
    };
    for p in parts {
        m.update(p);
    }
    m.finalize().into_bytes().into()
}

/// HKDF-Extract(salt, ikm) → PRK.
pub fn hkdf_extract(salt: &[u8], ikm: &[u8]) -> Secret32 {
    let (prk, _) = Hkdf::<Sha256>::extract(Some(salt), ikm);
    let mut out = [0u8; 32];
    out.copy_from_slice(&prk);
    Secret32(out)
}

/// HKDF-Expand(PRK, info, L) with `info` given as concatenated parts. `out.len()` must
/// be at most 255 × 32 bytes (always true here: outputs are 16 or 32 bytes).
pub fn hkdf_expand(prk: &[u8; 32], info: &[&[u8]], out: &mut [u8]) {
    match Hkdf::<Sha256>::from_prk(prk) {
        Ok(h) => {
            if h.expand_multi_info(info, out).is_err() {
                out.zeroize();
            }
        }
        Err(_) => out.zeroize(),
    }
}

/// HKDF-Expand to 32 bytes.
pub fn hkdf_expand32(prk: &[u8; 32], info: &[&[u8]]) -> Secret32 {
    let mut out = [0u8; 32];
    hkdf_expand(prk, info, &mut out);
    Secret32(out)
}

/// X25519 public key of a private scalar (clamped as RFC 7748).
pub fn x25519_public(private: &[u8; 32]) -> [u8; 32] {
    x25519_dalek::x25519(*private, x25519_dalek::X25519_BASEPOINT_BYTES)
}

/// X25519(private, public). Fails on the all-zero output (low-order public key).
pub fn x25519(private: &[u8; 32], public: &[u8; 32]) -> Result<Secret32> {
    let out = Secret32(x25519_dalek::x25519(*private, *public));
    // Constant-time enough for a rejection check: a fold over all bytes.
    let acc = out.0.iter().fold(0u8, |a, b| a | b);
    if acc == 0 {
        return Err(CryptoError::BadKey);
    }
    Ok(out)
}

/// Ed25519 public key of a 32-byte seed.
pub fn ed25519_public(seed: &[u8; 32]) -> [u8; 32] {
    SigningKey::from_bytes(seed).verifying_key().to_bytes()
}

/// Ed25519 signature (deterministic, RFC 8032) of `msg` with the key of `seed`.
pub fn ed25519_sign(seed: &[u8; 32], msg: &[u8]) -> [u8; 64] {
    SigningKey::from_bytes(seed).sign(msg).to_bytes()
}

/// Strict Ed25519 verification (rejects small-order keys and non-canonical encodings).
pub fn ed25519_verify(public: &[u8; 32], msg: &[u8], sig: &[u8; 64]) -> Result<()> {
    let vk = VerifyingKey::from_bytes(public).map_err(|_| CryptoError::BadSignature)?;
    vk.verify_strict(msg, &Signature::from_bytes(sig))
        .map_err(|_| CryptoError::BadSignature)
}

/// ChaCha20-Poly1305 seal in place: returns the tag; `buf` becomes the ciphertext.
pub fn aead_seal(key: &[u8; 32], nonce: &[u8; 12], aad: &[u8], buf: &mut [u8]) -> [u8; 16] {
    let cipher = ChaCha20Poly1305::new(key.into());
    match cipher.encrypt_in_place_detached(nonce.into(), aad, buf) {
        Ok(tag) => tag.into(),
        // Only fails for plaintexts over 256 GiB; unreachable with link frames.
        Err(_) => [0u8; 16],
    }
}

/// ChaCha20-Poly1305 open in place. The tag is verified before anything is decrypted,
/// so on a tag failure `buf` still holds the ciphertext.
pub fn aead_open(
    key: &[u8; 32],
    nonce: &[u8; 12],
    aad: &[u8],
    buf: &mut [u8],
    tag: &[u8; 16],
) -> Result<()> {
    let cipher = ChaCha20Poly1305::new(key.into());
    cipher
        .decrypt_in_place_detached(nonce.into(), aad, buf, Tag::from_slice(tag))
        .map_err(|_| CryptoError::Decrypt)
}

/// Draws a u32 from the injected RNG.
pub fn random_u32(rng: &mut ChaCha20Rng) -> u32 {
    rng.next_u32()
}

#[cfg(test)]
mod tests {
    use super::*;
    use mesh_wire::codec::from_hex;

    fn h(s: &str) -> Vec<u8> {
        from_hex(s).unwrap()
    }

    /// RFC 5869 test case 1 (HKDF-SHA256).
    #[test]
    fn hkdf_rfc5869_case1() {
        let ikm = h("0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b");
        let salt = h("000102030405060708090a0b0c");
        let info = h("f0f1f2f3f4f5f6f7f8f9");
        let prk = hkdf_extract(&salt, &ikm);
        assert_eq!(
            prk.expose().to_vec(),
            h("077709362c2e32df0ddc3f0dc47bba6390b6c73bb50f9c3122ec844ad7c2b3e5")
        );
        let mut okm = [0u8; 42];
        hkdf_expand(prk.expose(), &[&info[..5], &info[5..]], &mut okm);
        assert_eq!(
            okm.to_vec(),
            h(
                "3cb25f25faacd57a90434f64d0362f2a2d2d0a90cf1a5a4c5db02d56ecc4c5bf34007208d5b887185865"
            )
        );
    }

    /// RFC 4231 test case 2 (HMAC-SHA256).
    #[test]
    fn hmac_rfc4231_case2() {
        let mac = hmac_sha256(b"Jefe", &[b"what do ya want ", b"for nothing?"]);
        assert_eq!(
            mac.to_vec(),
            h("5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843")
        );
    }

    /// FIPS 180-2 "abc".
    #[test]
    fn sha256_abc() {
        assert_eq!(
            sha256(&[b"a", b"bc"]).to_vec(),
            h("ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad")
        );
    }

    /// RFC 7748 §6.1 Diffie-Hellman example.
    #[test]
    fn x25519_rfc7748() {
        let a: [u8; 32] = h("77076d0a7318a57d3c16c17251b26645df4c2f87ebc0992ab177fba51db92c2a")
            .try_into()
            .unwrap();
        let b: [u8; 32] = h("5dab087e624a8a4b79e17f8b83800ee66f3bb1292618b6fd1c2f8b27ff88e0eb")
            .try_into()
            .unwrap();
        let a_pub = x25519_public(&a);
        assert_eq!(
            a_pub.to_vec(),
            h("8520f0098930a754748b7ddcb43ef75a0dbf3a0d26381af4eba4a98eaa9b4e6a")
        );
        let k = x25519(&a, &x25519_public(&b)).unwrap();
        assert_eq!(
            k.expose().to_vec(),
            h("4a5d9d5ba4ce2de1728e3bf480350f25e07e21c947d19e3376f09b3c1e161742")
        );
        assert_eq!(x25519(&a, &[0u8; 32]), Err(CryptoError::BadKey));
    }

    /// RFC 8032 §7.1 TEST 1 (empty message).
    #[test]
    fn ed25519_rfc8032_test1() {
        let seed: [u8; 32] = h("9d61b19deffd5a60ba844af492ec2cc44449c5697b326919703bac031cae7f60")
            .try_into()
            .unwrap();
        let pk = ed25519_public(&seed);
        assert_eq!(
            pk.to_vec(),
            h("d75a980182b10ab7d54bfed3c964073a0ee172f3daa62325af021a68f707511a")
        );
        let sig = ed25519_sign(&seed, b"");
        assert_eq!(
            sig.to_vec(),
            h(
                "e5564300c360ac729086e2cc806e828a84877f1eb8e5d974d873e065224901555fb8821590a33bacc61e39701cf9b46bd25bf5f0595bbe24655141438e7a100b"
            )
        );
        assert!(ed25519_verify(&pk, b"", &sig).is_ok());
        assert_eq!(
            ed25519_verify(&pk, b"x", &sig),
            Err(CryptoError::BadSignature)
        );
    }

    /// RFC 8439 §2.8.2 AEAD example.
    #[test]
    fn chacha20poly1305_rfc8439() {
        let key: [u8; 32] = h("808182838485868788898a8b8c8d8e8f909192939495969798999a9b9c9d9e9f")
            .try_into()
            .unwrap();
        let nonce: [u8; 12] = h("070000004041424344454647").try_into().unwrap();
        let aad = h("50515253c0c1c2c3c4c5c6c7");
        let mut buf = b"Ladies and Gentlemen of the class of '99: If I could offer you only one tip for the future, sunscreen would be it.".to_vec();
        let tag = aead_seal(&key, &nonce, &aad, &mut buf);
        assert_eq!(tag.to_vec(), h("1ae10b594f09e26a7e902ecbd0600691"));
        assert_eq!(buf[..8].to_vec(), h("d31a8d34648e60db"));
        let mut bad = buf.clone();
        let mut wrong = tag;
        wrong[0] ^= 1;
        assert_eq!(
            aead_open(&key, &nonce, &aad, &mut bad, &wrong),
            Err(CryptoError::Decrypt)
        );
        assert_eq!(bad, buf, "a failed open leaves the buffer untouched");
        aead_open(&key, &nonce, &aad, &mut buf, &tag).unwrap();
        assert!(buf.starts_with(b"Ladies and Gentlemen"));
    }
}
