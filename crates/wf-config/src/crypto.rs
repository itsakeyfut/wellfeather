use std::{fs, path::Path};

use aes_gcm::{
    Aes256Gcm, Key, Nonce,
    aead::{Aead, KeyInit},
};
use anyhow::Context as _;
use base64::{Engine as _, engine::general_purpose::STANDARD as B64};
// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Encrypts `plaintext` with AES-256-GCM using the supplied 32-byte `key`.
///
/// Returns a Base64-encoded `"<nonce>:<ciphertext>"` string where:
/// - `nonce` is 12 random bytes (AES-GCM standard)
/// - `ciphertext` includes the 16-byte GCM authentication tag appended by the `aes-gcm` crate
///
/// The returned string is safe to store in `config.toml` as `password_encrypted`.
pub fn encrypt(plaintext: &str, key: &[u8; 32]) -> String {
    let cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(key));

    let nonce_bytes: [u8; 12] = rand::random();
    let nonce = Nonce::from_slice(&nonce_bytes);

    // Encryption cannot fail for well-formed inputs; the only error path in
    // aes-gcm is when the buffer is too large (>= 2^36 bytes), which will
    // never be reached for a password string.
    let ciphertext = cipher
        .encrypt(nonce, plaintext.as_bytes())
        .expect("AES-256-GCM encryption failed");

    format!("{}:{}", B64.encode(nonce_bytes), B64.encode(ciphertext))
}

/// Decrypts a `"<nonce>:<ciphertext>"` Base64 string produced by [`encrypt`].
///
/// # Errors
///
/// Returns `Err` if:
/// - The string is not in `nonce:ciphertext` format
/// - Either part is invalid Base64
/// - The nonce is not exactly 12 bytes
/// - The GCM authentication tag does not match (tampered or corrupt data)
/// - The decrypted bytes are not valid UTF-8
pub fn decrypt(ciphertext: &str, key: &[u8; 32]) -> anyhow::Result<String> {
    let (nonce_b64, ct_b64) = ciphertext
        .split_once(':')
        .ok_or_else(|| anyhow::anyhow!("invalid ciphertext format: expected '<nonce>:<ct>'"))?;

    let nonce_bytes = B64
        .decode(nonce_b64)
        .context("failed to Base64-decode nonce")?;
    let ct_bytes = B64
        .decode(ct_b64)
        .context("failed to Base64-decode ciphertext")?;

    anyhow::ensure!(
        nonce_bytes.len() == 12,
        "nonce must be 12 bytes, got {}",
        nonce_bytes.len()
    );

    let cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(key));
    let nonce = Nonce::from_slice(&nonce_bytes);

    let plaintext_bytes = cipher
        .decrypt(nonce, ct_bytes.as_slice())
        .map_err(|_| anyhow::anyhow!("decryption failed: GCM authentication tag mismatch"))?;

    String::from_utf8(plaintext_bytes).context("decrypted bytes are not valid UTF-8")
}

/// Loads the application encryption key from `dir/.wellfeather.key`.
///
/// If the key file does not exist, a fresh 32-byte key is generated with the
/// OS CSPRNG (`rand::random`), written to disk, and returned.
///
/// # Errors
///
/// Returns `Err` on I/O failure or if an existing key file contains a wrong
/// number of bytes (anything other than 32).
pub fn load_or_create_key(dir: &Path) -> anyhow::Result<[u8; 32]> {
    let key_path = dir.join(".wellfeather.key");

    if key_path.exists() {
        let bytes = fs::read(&key_path)
            .with_context(|| format!("failed to read key file {}", key_path.display()))?;
        let key: [u8; 32] = bytes.try_into().map_err(|v: Vec<u8>| {
            anyhow::anyhow!(
                "key file {} has wrong length: expected 32 bytes, got {}",
                key_path.display(),
                v.len()
            )
        })?;
        return Ok(key);
    }

    let key: [u8; 32] = rand::random();
    fs::create_dir_all(dir)
        .with_context(|| format!("failed to create key directory {}", dir.display()))?;
    fs::write(&key_path, key)
        .with_context(|| format!("failed to write key file {}", key_path.display()))?;
    Ok(key)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    const TEST_KEY: &[u8; 32] = b"an-exactly-32-byte-test-key-here";

    #[test]
    fn encrypt_decrypt_should_roundtrip() {
        let plaintext = "super-secret-password-123!";
        let encrypted = encrypt(plaintext, TEST_KEY);
        let decrypted = decrypt(&encrypted, TEST_KEY).expect("decrypt should succeed");
        assert_eq!(decrypted, plaintext);
    }

    #[test]
    fn decrypt_should_fail_on_tampered_ciphertext() {
        let encrypted = encrypt("my-password", TEST_KEY);

        // Split and tamper with one byte in the ciphertext half
        let (nonce_b64, ct_b64) = encrypted.split_once(':').unwrap();
        let mut ct_bytes = base64::engine::general_purpose::STANDARD
            .decode(ct_b64)
            .unwrap();
        ct_bytes[0] ^= 0xFF; // flip all bits in the first byte
        let tampered = format!(
            "{}:{}",
            nonce_b64,
            base64::engine::general_purpose::STANDARD.encode(&ct_bytes)
        );

        let result = decrypt(&tampered, TEST_KEY);
        assert!(result.is_err(), "expected Err for tampered ciphertext");
    }

    #[test]
    fn load_or_create_key_should_generate_and_persist_new_key() {
        let dir = tempfile::tempdir().expect("failed to create temp dir");
        let key_path = dir.path().join(".wellfeather.key");

        assert!(!key_path.exists(), "key file should not exist yet");

        let key = load_or_create_key(dir.path()).expect("should succeed");

        assert_eq!(key.len(), 32);
        assert!(key_path.exists(), "key file should be created");

        let stored = fs::read(&key_path).unwrap();
        assert_eq!(stored, key, "stored key should match returned key");
    }

    #[test]
    fn load_or_create_key_should_load_existing_key() {
        let dir = tempfile::tempdir().expect("failed to create temp dir");
        let key_path = dir.path().join(".wellfeather.key");

        let expected: [u8; 32] = *b"known-32-byte-key-for-testing-xx";
        fs::write(&key_path, expected).unwrap();

        let loaded = load_or_create_key(dir.path()).expect("should succeed");
        assert_eq!(loaded, expected);
    }
}
