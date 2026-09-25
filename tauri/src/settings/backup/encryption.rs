use std::num::NonZeroU32;

use ring::{
    aead, pbkdf2,
    rand::{SecureRandom, SystemRandom},
};
use zeroize::Zeroizing;

/// Independent magic/version header for encrypted backup files. The header is part of
/// the authenticated data, so tampering with it fails decryption. This is separate from
/// the retired repository-sync archive magic so old `.aitsync` files are never mistaken
/// for encrypted backups.
pub const ENCRYPTION_MAGIC: &[u8] = b"AI-TOOLBOX-BACKUP-ENC-1\0";

const SALT_LENGTH: usize = 32;
const NONCE_LENGTH: usize = 12;
const ITERATIONS: u32 = 600_000;

/// Structured crypto failures. The caller maps these to user-facing errors; none of
/// them may ever lead to a partially restored database.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CryptoError {
    /// Encrypted backup encountered but no password was supplied.
    PasswordRequired,
    /// Not an encrypted backup payload, or the header is truncated/unknown.
    InvalidFormat,
    /// Wrong password or tampered ciphertext (authentication tag mismatch).
    AuthFailed,
    /// Secure random generation failed.
    RandomFailed,
}

pub fn is_encrypted(bytes: &[u8]) -> bool {
    bytes.starts_with(ENCRYPTION_MAGIC)
}

fn derive_key(password: &str, salt: &[u8]) -> Result<aead::LessSafeKey, CryptoError> {
    if password.is_empty() {
        return Err(CryptoError::PasswordRequired);
    }
    let mut derived = Zeroizing::new([0u8; 32]);
    pbkdf2::derive(
        pbkdf2::PBKDF2_HMAC_SHA256,
        NonZeroU32::new(ITERATIONS).unwrap(),
        salt,
        password.as_bytes(),
        derived.as_mut(),
    );
    aead::UnboundKey::new(&aead::AES_256_GCM, derived.as_ref())
        .map(aead::LessSafeKey::new)
        .map_err(|_| CryptoError::InvalidFormat)
}

/// Encrypt a plaintext backup with a fresh random salt/nonce. Output layout:
/// magic | salt | nonce | ciphertext+tag. The header participates in authentication.
pub fn encrypt(plaintext: &[u8], password: &str) -> Result<Vec<u8>, CryptoError> {
    let random = SystemRandom::new();
    let mut salt = [0u8; SALT_LENGTH];
    let mut nonce = [0u8; NONCE_LENGTH];
    random
        .fill(&mut salt)
        .and_then(|_| random.fill(&mut nonce))
        .map_err(|_| CryptoError::RandomFailed)?;
    let mut header = ENCRYPTION_MAGIC.to_vec();
    header.extend_from_slice(&salt);
    header.extend_from_slice(&nonce);
    // Sealing happens in a moved buffer so the plaintext is not copied a second time.
    let mut ciphertext: Vec<u8> = plaintext.to_vec();
    derive_key(password, &salt)?
        .seal_in_place_append_tag(
            aead::Nonce::assume_unique_for_key(nonce),
            aead::Aad::from(header.as_slice()),
            &mut ciphertext,
        )
        .map_err(|_| CryptoError::InvalidFormat)?;
    header.extend_from_slice(&ciphertext);
    Ok(header)
}

/// Decrypt an encrypted backup. The authentication tag is fully verified before the
/// plaintext is released; any failure means the caller must not touch the database.
pub fn decrypt(bytes: &[u8], password: &str) -> Result<Vec<u8>, CryptoError> {
    let header_length = ENCRYPTION_MAGIC.len() + SALT_LENGTH + NONCE_LENGTH;
    if !is_encrypted(bytes) || bytes.len() < header_length + aead::AES_256_GCM.tag_len() {
        return Err(CryptoError::InvalidFormat);
    }
    let salt = &bytes[ENCRYPTION_MAGIC.len()..ENCRYPTION_MAGIC.len() + SALT_LENGTH];
    let nonce: [u8; NONCE_LENGTH] = bytes[ENCRYPTION_MAGIC.len() + SALT_LENGTH..header_length]
        .try_into()
        .map_err(|_| CryptoError::InvalidFormat)?;
    let mut plaintext = Zeroizing::new(bytes[header_length..].to_vec());
    let opened = derive_key(password, salt)?
        .open_in_place(
            aead::Nonce::assume_unique_for_key(nonce),
            aead::Aad::from(&bytes[..header_length]),
            plaintext.as_mut_slice(),
        )
        .map_err(|_| CryptoError::AuthFailed)?;
    Ok(opened.to_vec())
}

#[cfg(test)]
mod tests {
    use super::*;

    const PLAINTEXT: &[u8] = b"AI-TOOLBOX fake backup zip payload \x50\x4b\x03\x04 more bytes";

    #[test]
    fn encrypt_decrypt_round_trip() {
        let encrypted = encrypt(PLAINTEXT, "correct horse").unwrap();
        assert!(is_encrypted(&encrypted));
        assert_ne!(&encrypted[..ENCRYPTION_MAGIC.len()], PLAINTEXT);
        let decrypted = decrypt(&encrypted, "correct horse").unwrap();
        assert_eq!(decrypted.as_slice(), PLAINTEXT);
    }

    #[test]
    fn each_encryption_uses_fresh_salt_and_nonce() {
        let first = encrypt(PLAINTEXT, "same-password").unwrap();
        let second = encrypt(PLAINTEXT, "same-password").unwrap();
        assert_ne!(first, second, "salt/nonce must never repeat across files");
    }

    #[test]
    fn wrong_password_fails_with_auth_error() {
        let encrypted = encrypt(PLAINTEXT, "right").unwrap();
        assert_eq!(
            decrypt(&encrypted, "wrong").unwrap_err(),
            CryptoError::AuthFailed
        );
    }

    #[test]
    fn tampered_ciphertext_or_header_fails() {
        let mut encrypted = encrypt(PLAINTEXT, "right").unwrap();
        let last = encrypted.len() - 1;
        encrypted[last] ^= 0xFF;
        assert_eq!(
            decrypt(&encrypted, "right").unwrap_err(),
            CryptoError::AuthFailed
        );

        let mut tampered_header = encrypt(PLAINTEXT, "right").unwrap();
        tampered_header[ENCRYPTION_MAGIC.len()] ^= 0xFF;
        assert_eq!(
            decrypt(&tampered_header, "right").unwrap_err(),
            CryptoError::AuthFailed,
            "header must participate in authentication"
        );
    }

    #[test]
    fn truncated_or_unknown_payload_is_invalid_format() {
        assert_eq!(decrypt(b"", "pw").unwrap_err(), CryptoError::InvalidFormat);
        assert_eq!(
            decrypt(&ENCRYPTION_MAGIC[..ENCRYPTION_MAGIC.len() - 4], "pw").unwrap_err(),
            CryptoError::InvalidFormat
        );
        assert_eq!(
            decrypt(&ENCRYPTION_MAGIC.to_vec(), "pw").unwrap_err(),
            CryptoError::InvalidFormat
        );
        assert_eq!(
            decrypt(b"PK\x03\x04 not encrypted at all", "pw").unwrap_err(),
            CryptoError::InvalidFormat
        );
    }

    #[test]
    fn empty_password_is_rejected() {
        assert_eq!(
            encrypt(PLAINTEXT, "").unwrap_err(),
            CryptoError::PasswordRequired
        );
        let encrypted = encrypt(PLAINTEXT, "pw").unwrap();
        assert_eq!(
            decrypt(&encrypted, "").unwrap_err(),
            CryptoError::PasswordRequired
        );
    }

    #[test]
    fn plaintext_zip_is_not_detected_as_encrypted() {
        assert!(!is_encrypted(PLAINTEXT));
        assert!(!is_encrypted(b"PK\x03\x04"));
    }
}
