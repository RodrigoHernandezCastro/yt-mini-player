//! Watch together: encryption between the people in a session.
//!
//! The server only relays bytes it cannot read:
//!
//! 1. A guest who typed the code says hello with an X25519 public key, in a
//!    box sealed with the code's key (so it proves it knows the code).
//! 2. The host answers with its own X25519 public key and, sealed with a key
//!    derived from the Diffie–Hellman secret *and* the code, the room key.
//!    Someone who recorded the traffic and later learns the code still
//!    cannot open it.
//! 3. Everything else (what plays, pauses, the clock) goes sealed with the
//!    room key (AES-256-GCM), bound to who sent it. When someone is removed,
//!    the host hands the others a new room key.

use aes_gcm::{
    aead::{Aead, KeyInit, Payload},
    Aes256Gcm, Nonce,
};
use base64::{engine::general_purpose::STANDARD as B64, Engine};
use hkdf::Hkdf;
use sha2::Sha256;
use x25519_dalek::{PublicKey, StaticSecret};
use zeroize::Zeroizing;

pub type Key = Zeroizing<[u8; 32]>;

pub fn random32() -> [u8; 32] {
    let mut bytes = [0u8; 32];
    getrandom::fill(&mut bytes).expect("the system's random number generator");
    bytes
}

pub fn random_key() -> Key {
    Zeroizing::new(random32())
}

/// HKDF-SHA256 to 32 bytes.
pub fn hkdf(ikm: &[u8], salt: &[u8], info: &[u8]) -> [u8; 32] {
    let mut out = [0u8; 32];
    Hkdf::<Sha256>::new(Some(salt), ikm).expand(info, &mut out).expect("32 bytes is a valid length");
    out
}

/// Nonce (12 random bytes) followed by the ciphertext, in Base64.
pub fn seal(key: &[u8; 32], aad: &str, plaintext: &[u8]) -> String {
    let cipher = Aes256Gcm::new_from_slice(key).expect("a 32-byte key");
    let mut nonce = [0u8; 12];
    getrandom::fill(&mut nonce).expect("the system's random number generator");
    let sealed = cipher
        .encrypt(Nonce::from_slice(&nonce), Payload { msg: plaintext, aad: aad.as_bytes() })
        .expect("encryption does not fail");
    let mut out = nonce.to_vec();
    out.extend_from_slice(&sealed);
    B64.encode(out)
}

/// `None` if it was not sealed with this key and these associated data
/// (changed, from someone else, or meant for another place).
pub fn open(key: &[u8; 32], aad: &str, sealed: &str) -> Option<Vec<u8>> {
    let bytes = B64.decode(sealed).ok()?;
    if bytes.len() < 12 + 16 {
        return None;
    }
    let (nonce, body) = bytes.split_at(12);
    let cipher = Aes256Gcm::new_from_slice(key).ok()?;
    cipher.decrypt(Nonce::from_slice(nonce), Payload { msg: body, aad: aad.as_bytes() }).ok()
}

/// An X25519 key pair for one handshake.
pub struct Ephemeral {
    secret: StaticSecret,
    pub public: [u8; 32],
}

impl Ephemeral {
    pub fn generate() -> Self {
        let secret = StaticSecret::from(random32());
        let public = PublicKey::from(&secret).to_bytes();
        Self { secret, public }
    }

    pub fn public_b64(&self) -> String {
        B64.encode(self.public)
    }

    /// The Diffie–Hellman secret with the other side's public key (Base64).
    /// `None` for a malformed key or one that gives a trivial secret.
    pub fn agree(&self, peer_b64: &str) -> Option<Key> {
        let peer: [u8; 32] = B64.decode(peer_b64).ok()?.try_into().ok()?;
        let shared = self.secret.diffie_hellman(&PublicKey::from(peer));
        if !shared.was_contributory() {
            return None;
        }
        Some(Zeroizing::new(shared.to_bytes()))
    }
}

/// Key of the host's answer to one guest: the Diffie–Hellman secret mixed
/// with the code's key and the guest's place in the room.
pub fn welcome_key(shared: &[u8; 32], join_key: &[u8; 32], member: &str) -> Key {
    Zeroizing::new(hkdf(shared, join_key, format!("welcome|{member}").as_bytes()))
}

pub fn b64(bytes: &[u8]) -> String {
    B64.encode(bytes)
}

pub fn key_from_b64(text: &str) -> Option<Key> {
    let bytes: [u8; 32] = B64.decode(text).ok()?.try_into().ok()?;
    Some(Zeroizing::new(bytes))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sealed_messages_only_open_with_the_same_key_and_context() {
        let key = random32();
        let sealed = seal(&key, "room|g1|0", b"hello");
        assert_eq!(open(&key, "room|g1|0", &sealed).as_deref(), Some(&b"hello"[..]));
        assert_eq!(open(&key, "room|g2|0", &sealed), None, "from someone else");
        assert_eq!(open(&random32(), "room|g1|0", &sealed), None, "another key");
        let mut bytes = B64.decode(&sealed).unwrap();
        let last = bytes.len() - 1;
        bytes[last] ^= 1;
        assert_eq!(open(&key, "room|g1|0", &B64.encode(bytes)), None, "changed on the way");
        assert_eq!(open(&key, "room|g1|0", "AAAA"), None);
        assert_ne!(seal(&key, "x", b"same"), seal(&key, "x", b"same"), "a new nonce every time");
    }

    #[test]
    fn both_sides_agree_on_the_same_secret() {
        let (a, b) = (Ephemeral::generate(), Ephemeral::generate());
        let ab = a.agree(&b.public_b64()).unwrap();
        let ba = b.agree(&a.public_b64()).unwrap();
        assert_eq!(*ab, *ba);
        assert!(a.agree(&b64(&[0u8; 32])).is_none(), "a trivial key is refused");
        assert!(a.agree("not base64!").is_none());
        let join = random32();
        assert_ne!(*welcome_key(&ab, &join, "g1"), *welcome_key(&ab, &join, "g2"));
    }
}
