use ring::aead::{AES_256_GCM, Aad, LessSafeKey, Nonce, UnboundKey};
use ring::rand::{SecureRandom, SystemRandom};
use thiserror::Error;
use zeroize::ZeroizeOnDrop;

const NONCE_LEN: usize = 12;
const TAG_LEN: usize = 16;

#[derive(Error, Debug)]
pub enum CryptoError {
    #[error("invalid hex key: {0}")]
    InvalidHexKey(String),
    #[error("key must be exactly 32 bytes (64 hex chars), got {0}")]
    InvalidKeyLength(usize),
    #[error("encryption failed")]
    EncryptionFailed,
    #[error("decryption failed: ciphertext too short")]
    CiphertextTooShort,
    #[error("decryption failed")]
    DecryptionFailed,
}

/// AES-256-GCM encryption key.
/// Wraps ring's LessSafeKey for encrypt/decrypt operations.
/// Key bytes are automatically zeroed on drop via `ZeroizeOnDrop`.
#[derive(ZeroizeOnDrop)]
pub struct EncryptionKey {
    #[zeroize(skip)] // LessSafeKey handles its own memory
    cached_key: LessSafeKey,
    key_bytes: [u8; 32],
}

impl EncryptionKey {
    /// Create from a hex-encoded 32-byte key (64 hex characters).
    pub fn from_hex(hex_str: &str) -> Result<Self, CryptoError> {
        let bytes = hex::decode(hex_str).map_err(|e| CryptoError::InvalidHexKey(e.to_string()))?;
        if bytes.len() != 32 {
            return Err(CryptoError::InvalidKeyLength(bytes.len()));
        }
        let mut key_bytes = [0u8; 32];
        key_bytes.copy_from_slice(&bytes);

        let unbound =
            UnboundKey::new(&AES_256_GCM, &key_bytes).map_err(|_| CryptoError::EncryptionFailed)?;
        let cached_key = LessSafeKey::new(unbound);

        Ok(Self {
            cached_key,
            key_bytes,
        })
    }

    /// Access the raw key bytes (e.g. for HMAC operations).
    pub fn key_bytes(&self) -> &[u8; 32] {
        &self.key_bytes
    }

    /// Encrypt plaintext. Returns nonce (12 bytes) || ciphertext || tag (16 bytes).
    pub fn encrypt(&self, plaintext: &[u8]) -> Result<Vec<u8>, CryptoError> {
        let rng = SystemRandom::new();
        let mut nonce_bytes = [0u8; NONCE_LEN];
        rng.fill(&mut nonce_bytes)
            .map_err(|_| CryptoError::EncryptionFailed)?;

        let nonce = Nonce::assume_unique_for_key(nonce_bytes);

        let mut in_out = plaintext.to_vec();
        self.cached_key
            .seal_in_place_append_tag(nonce, Aad::empty(), &mut in_out)
            .map_err(|_| CryptoError::EncryptionFailed)?;

        // Prepend nonce
        let mut result = Vec::with_capacity(NONCE_LEN + in_out.len());
        result.extend_from_slice(&nonce_bytes);
        result.extend_from_slice(&in_out);
        Ok(result)
    }

    /// Decrypt ciphertext produced by [`encrypt`]. Input: nonce || ciphertext || tag.
    pub fn decrypt(&self, data: &[u8]) -> Result<Vec<u8>, CryptoError> {
        if data.len() < NONCE_LEN + TAG_LEN {
            return Err(CryptoError::CiphertextTooShort);
        }

        let (nonce_bytes, ciphertext_and_tag) = data.split_at(NONCE_LEN);
        let mut nonce_arr = [0u8; NONCE_LEN];
        nonce_arr.copy_from_slice(nonce_bytes);

        let nonce = Nonce::assume_unique_for_key(nonce_arr);

        let mut in_out = ciphertext_and_tag.to_vec();
        let plaintext = self
            .cached_key
            .open_in_place(nonce, Aad::empty(), &mut in_out)
            .map_err(|_| CryptoError::DecryptionFailed)?;
        Ok(plaintext.to_vec())
    }

    /// Encrypt a UTF-8 string.
    pub fn encrypt_string(&self, plaintext: &str) -> Result<Vec<u8>, CryptoError> {
        self.encrypt(plaintext.as_bytes())
    }

    /// Decrypt to a UTF-8 string.
    pub fn decrypt_string(&self, data: &[u8]) -> Result<String, CryptoError> {
        let bytes = self.decrypt(data)?;
        String::from_utf8(bytes).map_err(|_| CryptoError::DecryptionFailed)
    }
}

/// HMAC-SHA256 using the given key bytes. Returns hex-encoded digest string.
pub fn hmac_sha256_hex(key_bytes: &[u8; 32], input: &str) -> String {
    use ring::hmac;
    let hmac_key = hmac::Key::new(hmac::HMAC_SHA256, key_bytes);
    let tag = hmac::sign(&hmac_key, input.as_bytes());
    hex::encode(tag.as_ref())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_key() -> EncryptionKey {
        // 32 bytes of 0x01
        EncryptionKey::from_hex(&"01".repeat(32)).unwrap()
    }

    #[test]
    fn test_roundtrip() {
        let key = test_key();
        let plaintext = b"Hello, Mika!";
        let encrypted = key.encrypt(plaintext).unwrap();
        let decrypted = key.decrypt(&encrypted).unwrap();
        assert_eq!(decrypted, plaintext);
    }

    #[test]
    fn test_string_roundtrip() {
        let key = test_key();
        let text = "Executive meeting at 3pm with Sarah";
        let encrypted = key.encrypt_string(text).unwrap();
        let decrypted = key.decrypt_string(&encrypted).unwrap();
        assert_eq!(decrypted, text);
    }

    #[test]
    fn test_different_nonces() {
        let key = test_key();
        let plaintext = b"same input";
        let enc1 = key.encrypt(plaintext).unwrap();
        let enc2 = key.encrypt(plaintext).unwrap();
        // Different nonces should produce different ciphertexts
        assert_ne!(enc1, enc2);
        // But both decrypt to the same plaintext
        assert_eq!(key.decrypt(&enc1).unwrap(), key.decrypt(&enc2).unwrap());
    }

    #[test]
    fn test_invalid_key_length() {
        let result = EncryptionKey::from_hex("0102");
        assert!(result.is_err());
    }

    #[test]
    fn test_tampered_ciphertext() {
        let key = test_key();
        let mut encrypted = key.encrypt(b"secret").unwrap();
        // Tamper with the ciphertext (after nonce)
        encrypted[NONCE_LEN] ^= 0xFF;
        assert!(key.decrypt(&encrypted).is_err());
    }

    #[test]
    fn test_ciphertext_too_short() {
        let key = test_key();
        assert!(key.decrypt(&[0u8; 10]).is_err());
    }

    #[test]
    fn test_key_bytes_accessor() {
        let key = test_key();
        assert_eq!(key.key_bytes(), &[0x01u8; 32]);
    }

    #[test]
    fn test_hmac_sha256_hex() {
        let key_bytes = [0x01u8; 32];
        let hash = hmac_sha256_hex(&key_bytes, "hello");
        // Should be 64 hex chars (32 bytes)
        assert_eq!(hash.len(), 64);
        // Should be deterministic
        let hash2 = hmac_sha256_hex(&key_bytes, "hello");
        assert_eq!(hash, hash2);
        // Different input should produce different hash
        let hash3 = hmac_sha256_hex(&key_bytes, "world");
        assert_ne!(hash, hash3);
    }
}
