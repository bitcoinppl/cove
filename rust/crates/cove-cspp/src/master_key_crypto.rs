use chacha20poly1305::{ChaCha20Poly1305, KeyInit as _, aead::Aead as _};
use rand::RngExt as _;

use crate::backup_data::EncryptedMasterKeyBackup;
use crate::error::CsppError;
use crate::master_key::MasterKey;

/// Encrypt a master key with a PRF-derived wrapping key
pub fn encrypt_master_key(
    master_key: &MasterKey,
    prf_key: &[u8; 32],
    prf_salt: &[u8; 32],
) -> Result<EncryptedMasterKeyBackup, CsppError> {
    let cipher = ChaCha20Poly1305::new(prf_key.into());

    let mut nonce_bytes = [0u8; 12];
    rand::rng().fill(&mut nonce_bytes);
    let nonce = chacha20poly1305::Nonce::from_slice(&nonce_bytes);

    let ciphertext = cipher
        .encrypt(nonce, master_key.as_bytes().as_slice())
        .map_err(|e| CsppError::Encrypt(e.to_string()))?;

    Ok(EncryptedMasterKeyBackup { version: 1, prf_salt: *prf_salt, nonce: nonce_bytes, ciphertext })
}

/// Decrypt a master key backup using a PRF-derived wrapping key
pub fn decrypt_master_key(
    backup: &EncryptedMasterKeyBackup,
    prf_key: &[u8; 32],
) -> Result<MasterKey, CsppError> {
    let cipher = ChaCha20Poly1305::new(prf_key.into());
    let nonce = chacha20poly1305::Nonce::from_slice(&backup.nonce);

    let plaintext =
        cipher.decrypt(nonce, backup.ciphertext.as_slice()).map_err(|_| CsppError::WrongKey)?;

    let bytes: [u8; 32] = plaintext
        .try_into()
        .map_err(|_| CsppError::Decrypt("decrypted master key is not 32 bytes".to_string()))?;

    Ok(MasterKey::from_bytes(bytes))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encrypt_decrypt_roundtrip() {
        let master_key = MasterKey::generate();
        let prf_key = [42u8; 32];
        let prf_salt = [1u8; 32];

        let encrypted = encrypt_master_key(&master_key, &prf_key, &prf_salt).unwrap();
        let decrypted = decrypt_master_key(&encrypted, &prf_key).unwrap();

        assert_eq!(master_key.as_bytes(), decrypted.as_bytes());
    }

    #[test]
    fn wrong_key_fails() {
        let master_key = MasterKey::generate();
        let prf_key = [42u8; 32];
        let wrong_key = [99u8; 32];
        let prf_salt = [1u8; 32];

        let encrypted = encrypt_master_key(&master_key, &prf_key, &prf_salt).unwrap();
        let result = decrypt_master_key(&encrypted, &wrong_key);

        assert!(matches!(result, Err(CsppError::WrongKey)));
    }

    #[test]
    fn prf_salt_is_preserved() {
        let master_key = MasterKey::generate();
        let prf_key = [42u8; 32];
        let prf_salt = [0xABu8; 32];

        let encrypted = encrypt_master_key(&master_key, &prf_key, &prf_salt).unwrap();
        assert_eq!(encrypted.prf_salt, prf_salt);
    }

    #[test]
    fn different_encryptions_produce_different_nonces() {
        let master_key = MasterKey::generate();
        let prf_key = [42u8; 32];
        let prf_salt = [1u8; 32];

        let enc1 = encrypt_master_key(&master_key, &prf_key, &prf_salt).unwrap();
        let enc2 = encrypt_master_key(&master_key, &prf_key, &prf_salt).unwrap();

        assert_ne!(enc1.nonce, enc2.nonce);
    }

    #[test]
    fn tampered_ciphertext_fails() {
        let master_key = MasterKey::generate();
        let prf_key = [42u8; 32];
        let prf_salt = [1u8; 32];

        let mut encrypted = encrypt_master_key(&master_key, &prf_key, &prf_salt).unwrap();
        encrypted.ciphertext[0] ^= 0xFF;

        let result = decrypt_master_key(&encrypted, &prf_key);
        assert!(result.is_err());
    }

    #[test]
    fn tampered_nonce_fails() {
        let master_key = MasterKey::generate();
        let prf_key = [42u8; 32];
        let prf_salt = [1u8; 32];

        let mut encrypted = encrypt_master_key(&master_key, &prf_key, &prf_salt).unwrap();
        encrypted.nonce[0] ^= 0xFF;

        let result = decrypt_master_key(&encrypted, &prf_key);
        assert!(result.is_err());
    }
}
