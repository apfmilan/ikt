//! Sealed-box encryption for broker credentials (ADR-0004).
//!
//! The API process holds only the engine's X25519 **public** key: it can seal credentials
//! submitted during onboarding but can never open them. The engine holds the secret key
//! (Docker secret file) and opens credentials in memory only when needed.
//!
//! Envelope v1: `0x01 || ephemeral_pub[32] || nonce[12] || AES-256-GCM(ciphertext||tag)`.
//! Key = HKDF-SHA256(ikm = ECDH(eph, engine), salt = eph_pub || engine_pub,
//! info = "tc-vault-v1 aes-256-gcm"). AAD binds the ciphertext to its purpose, e.g.
//! `bitvavo|api_secret|kv1`, so a ciphertext cannot be swapped between fields or brokers.

use aes_gcm::aead::{Aead, KeyInit, Payload};
use aes_gcm::{Aes256Gcm, Nonce};
use base64::engine::general_purpose::STANDARD as B64;
use base64::Engine;
use hkdf::Hkdf;
use rand::rngs::OsRng;
use rand::RngCore;
use secrecy::{ExposeSecret, SecretString};
use sha2::Sha256;
use x25519_dalek::{EphemeralSecret, PublicKey, StaticSecret};
use zeroize::Zeroizing;

const VERSION: u8 = 1;
const INFO: &[u8] = b"tc-vault-v1 aes-256-gcm";

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum VaultError {
    #[error("invalid key encoding")]
    BadKey,
    #[error("ciphertext malformed")]
    Malformed,
    #[error("decryption failed (wrong key, tampered data or wrong context)")]
    Decrypt,
    #[error("encryption failed")]
    Encrypt,
}

/// Engine public key (safe to give to the API process).
#[derive(Clone)]
pub struct SealingKey(PublicKey);

/// Engine secret key. Never leaves the engine process; zeroized on drop.
pub struct OpeningKey(StaticSecret);

impl std::fmt::Debug for OpeningKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("OpeningKey(<redacted>)")
    }
}

impl SealingKey {
    pub fn from_base64(s: &str) -> Result<Self, VaultError> {
        let b = B64.decode(s.trim()).map_err(|_| VaultError::BadKey)?;
        let arr: [u8; 32] = b.try_into().map_err(|_| VaultError::BadKey)?;
        Ok(SealingKey(PublicKey::from(arr)))
    }

    pub fn to_base64(&self) -> String {
        B64.encode(self.0.as_bytes())
    }

    /// Seal `plaintext` for the engine, bound to `context` (AAD).
    pub fn seal(&self, plaintext: &[u8], context: &str) -> Result<Vec<u8>, VaultError> {
        let eph = EphemeralSecret::random_from_rng(OsRng);
        let eph_pub = PublicKey::from(&eph);
        let shared = eph.diffie_hellman(&self.0);
        let key = derive_key(shared.as_bytes(), eph_pub.as_bytes(), self.0.as_bytes())?;
        let cipher = Aes256Gcm::new_from_slice(key.as_ref()).map_err(|_| VaultError::Encrypt)?;
        let mut nonce = [0u8; 12];
        OsRng.fill_bytes(&mut nonce);
        let ct = cipher
            .encrypt(
                Nonce::from_slice(&nonce),
                Payload {
                    msg: plaintext,
                    aad: context.as_bytes(),
                },
            )
            .map_err(|_| VaultError::Encrypt)?;
        let mut out = Vec::with_capacity(1 + 32 + 12 + ct.len());
        out.push(VERSION);
        out.extend_from_slice(eph_pub.as_bytes());
        out.extend_from_slice(&nonce);
        out.extend_from_slice(&ct);
        Ok(out)
    }

    pub fn seal_str(&self, secret: &SecretString, context: &str) -> Result<Vec<u8>, VaultError> {
        self.seal(secret.expose_secret().as_bytes(), context)
    }
}

impl OpeningKey {
    pub fn generate() -> Self {
        OpeningKey(StaticSecret::random_from_rng(OsRng))
    }

    pub fn from_base64(s: &str) -> Result<Self, VaultError> {
        let b = Zeroizing::new(B64.decode(s.trim()).map_err(|_| VaultError::BadKey)?);
        let arr: [u8; 32] = b.as_slice().try_into().map_err(|_| VaultError::BadKey)?;
        Ok(OpeningKey(StaticSecret::from(arr)))
    }

    /// Base64 of the secret key — only for writing the Docker secret file at key generation.
    pub fn to_base64_secret(&self) -> Zeroizing<String> {
        Zeroizing::new(B64.encode(self.0.to_bytes()))
    }

    pub fn sealing_key(&self) -> SealingKey {
        SealingKey(PublicKey::from(&self.0))
    }

    pub fn open(&self, envelope: &[u8], context: &str) -> Result<Zeroizing<Vec<u8>>, VaultError> {
        if envelope.len() < 1 + 32 + 12 + 16 || envelope[0] != VERSION {
            return Err(VaultError::Malformed);
        }
        let eph: [u8; 32] = envelope[1..33]
            .try_into()
            .map_err(|_| VaultError::Malformed)?;
        let eph_pub = PublicKey::from(eph);
        let nonce = &envelope[33..45];
        let shared = self.0.diffie_hellman(&eph_pub);
        let me = PublicKey::from(&self.0);
        let key = derive_key(shared.as_bytes(), eph_pub.as_bytes(), me.as_bytes())?;
        let cipher = Aes256Gcm::new_from_slice(key.as_ref()).map_err(|_| VaultError::Decrypt)?;
        let pt = cipher
            .decrypt(
                Nonce::from_slice(nonce),
                Payload {
                    msg: &envelope[45..],
                    aad: context.as_bytes(),
                },
            )
            .map_err(|_| VaultError::Decrypt)?;
        Ok(Zeroizing::new(pt))
    }

    pub fn open_string(&self, envelope: &[u8], context: &str) -> Result<SecretString, VaultError> {
        let pt = self.open(envelope, context)?;
        let s = std::str::from_utf8(&pt).map_err(|_| VaultError::Decrypt)?;
        Ok(SecretString::from(s.to_owned()))
    }
}

fn derive_key(
    shared: &[u8],
    eph_pub: &[u8],
    recipient_pub: &[u8],
) -> Result<Zeroizing<[u8; 32]>, VaultError> {
    let mut salt = [0u8; 64];
    salt[..32].copy_from_slice(eph_pub);
    salt[32..].copy_from_slice(recipient_pub);
    let hk = Hkdf::<Sha256>::new(Some(&salt), shared);
    let mut okm = Zeroizing::new([0u8; 32]);
    hk.expand(INFO, okm.as_mut())
        .map_err(|_| VaultError::Encrypt)?;
    Ok(okm)
}

/// Standard AAD context string for a credential field.
pub fn context(broker: &str, field: &str, key_version: u32) -> String {
    format!("{broker}|{field}|kv{key_version}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_and_context_binding() {
        let sk = OpeningKey::generate();
        let pk = sk.sealing_key();
        let ctx = context("bitvavo", "api_secret", 1);
        let env = pk.seal(b"super-secret", &ctx).unwrap();
        assert_eq!(sk.open(&env, &ctx).unwrap().as_slice(), b"super-secret");
        // Wrong context (field swap) fails.
        assert_eq!(
            sk.open(&env, &context("bitvavo", "api_key", 1)),
            Err(VaultError::Decrypt)
        );
        // Wrong key fails.
        assert_eq!(
            OpeningKey::generate().open(&env, &ctx),
            Err(VaultError::Decrypt)
        );
    }

    #[test]
    fn tamper_detected_and_nondeterministic() {
        let sk = OpeningKey::generate();
        let pk = sk.sealing_key();
        let a = pk.seal(b"x", "c").unwrap();
        let b = pk.seal(b"x", "c").unwrap();
        assert_ne!(a, b, "ephemeral keys + random nonces");
        let mut t = a.clone();
        let last = t.len() - 1;
        t[last] ^= 1;
        assert_eq!(sk.open(&t, "c"), Err(VaultError::Decrypt));
        assert_eq!(sk.open(&a[..10], "c"), Err(VaultError::Malformed));
    }

    #[test]
    fn key_encoding_roundtrip() {
        let sk = OpeningKey::generate();
        let s = sk.to_base64_secret();
        let sk2 = OpeningKey::from_base64(&s).unwrap();
        assert_eq!(sk.sealing_key().to_base64(), sk2.sealing_key().to_base64());
        assert!(SealingKey::from_base64("not base64!").is_err());
        assert_eq!(format!("{sk:?}"), "OpeningKey(<redacted>)");
    }
}
