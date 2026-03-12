use chacha20poly1305::{ChaCha20Poly1305, KeyInit as _, aead::Aead as _};
use rand::RngExt as _;
use zeroize::Zeroize as _;

use crate::backup_data::{EncryptedWalletBackup, WalletEntry};
use crate::error::CsppError;
use crate::key_derivation::derive_wallet_key;

/// Encrypt a wallet entry for cloud backup
///
/// Generates a random wallet_salt, derives a per-wallet key via HKDF,
/// serializes to JSON, and encrypts with ChaCha20-Poly1305
pub fn encrypt_wallet_entry(
    entry: &WalletEntry,
    critical_data_key: &[u8; 32],
) -> Result<EncryptedWalletBackup, CsppError> {
    let mut wallet_salt = [0u8; 32];
    rand::rng().fill(&mut wallet_salt);

    let mut wallet_key = derive_wallet_key(critical_data_key, &wallet_salt);

    let json = serde_json::to_vec(entry).map_err(|e| CsppError::Serialization(e.to_string()))?;

    let cipher = ChaCha20Poly1305::new((&wallet_key).into());
    wallet_key.zeroize();

    let mut nonce_bytes = [0u8; 12];
    rand::rng().fill(&mut nonce_bytes);
    let nonce = chacha20poly1305::Nonce::from_slice(&nonce_bytes);

    let ciphertext =
        cipher.encrypt(nonce, json.as_slice()).map_err(|e| CsppError::Encrypt(e.to_string()))?;

    Ok(EncryptedWalletBackup { version: 1, wallet_salt, nonce: nonce_bytes, ciphertext })
}

/// Decrypt an encrypted wallet backup
pub fn decrypt_wallet_backup(
    backup: &EncryptedWalletBackup,
    critical_data_key: &[u8; 32],
) -> Result<WalletEntry, CsppError> {
    let mut wallet_key = derive_wallet_key(critical_data_key, &backup.wallet_salt);

    let cipher = ChaCha20Poly1305::new((&wallet_key).into());
    wallet_key.zeroize();

    let nonce = chacha20poly1305::Nonce::from_slice(&backup.nonce);

    let plaintext =
        cipher.decrypt(nonce, backup.ciphertext.as_slice()).map_err(|_| CsppError::WrongKey)?;

    serde_json::from_slice(&plaintext).map_err(|e| CsppError::Deserialization(e.to_string()))
}

#[cfg(test)]
mod tests {
    use crate::backup_data::{DescriptorPair, WalletMode, WalletSecret};

    use super::*;

    fn test_entry() -> WalletEntry {
        WalletEntry {
            wallet_id: "test-wallet-123".to_string(),
            secret: WalletSecret::Mnemonic("abandon abandon abandon".to_string()),
            metadata: serde_json::json!({"name": "Test Wallet", "network": "bitcoin"}),
            descriptors: Some(DescriptorPair {
                external: "wpkh(xpub/0/*)".to_string(),
                internal: "wpkh(xpub/1/*)".to_string(),
            }),
            xpub: Some("xpub661MyMwAqRbcF...".to_string()),
            wallet_mode: WalletMode::Main,
        }
    }

    #[test]
    fn encrypt_decrypt_roundtrip() {
        let entry = test_entry();
        let critical_key = [42u8; 32];

        let encrypted = encrypt_wallet_entry(&entry, &critical_key).unwrap();
        let decrypted = decrypt_wallet_backup(&encrypted, &critical_key).unwrap();

        assert_eq!(decrypted.wallet_id, "test-wallet-123");
        assert!(
            matches!(decrypted.secret, WalletSecret::Mnemonic(ref m) if m == "abandon abandon abandon")
        );
        assert_eq!(decrypted.wallet_mode, WalletMode::Main);
    }

    #[test]
    fn different_wallet_salts_produce_different_ciphertext() {
        let entry = test_entry();
        let critical_key = [42u8; 32];

        let enc1 = encrypt_wallet_entry(&entry, &critical_key).unwrap();
        let enc2 = encrypt_wallet_entry(&entry, &critical_key).unwrap();

        // random salts mean different derived keys and different ciphertext
        assert_ne!(enc1.wallet_salt, enc2.wallet_salt);
    }

    #[test]
    fn wrong_key_fails() {
        let entry = test_entry();
        let critical_key = [42u8; 32];
        let wrong_key = [99u8; 32];

        let encrypted = encrypt_wallet_entry(&entry, &critical_key).unwrap();
        let result = decrypt_wallet_backup(&encrypted, &wrong_key);

        assert!(matches!(result, Err(CsppError::WrongKey)));
    }

    #[test]
    fn all_secret_variants_roundtrip() {
        let critical_key = [42u8; 32];

        let secrets = vec![
            WalletSecret::Mnemonic("test words".to_string()),
            WalletSecret::TapSignerBackup(vec![1, 2, 3]),
            WalletSecret::Descriptor("wpkh(...)".to_string()),
            WalletSecret::WatchOnly,
        ];

        for secret in secrets {
            let entry = WalletEntry {
                wallet_id: "test".to_string(),
                secret,
                metadata: serde_json::json!({}),
                descriptors: None,
                xpub: None,
                wallet_mode: WalletMode::Decoy,
            };

            let encrypted = encrypt_wallet_entry(&entry, &critical_key).unwrap();
            let decrypted = decrypt_wallet_backup(&encrypted, &critical_key).unwrap();
            assert_eq!(decrypted.wallet_id, "test");
            assert_eq!(decrypted.wallet_mode, WalletMode::Decoy);
        }
    }

    #[test]
    fn tampered_ciphertext_fails() {
        let entry = test_entry();
        let critical_key = [42u8; 32];

        let mut encrypted = encrypt_wallet_entry(&entry, &critical_key).unwrap();
        encrypted.ciphertext[0] ^= 0xFF;

        let result = decrypt_wallet_backup(&encrypted, &critical_key);
        assert!(result.is_err());
    }

    #[test]
    fn tampered_nonce_fails() {
        let entry = test_entry();
        let critical_key = [42u8; 32];

        let mut encrypted = encrypt_wallet_entry(&entry, &critical_key).unwrap();
        encrypted.nonce[0] ^= 0xFF;

        let result = decrypt_wallet_backup(&encrypted, &critical_key);
        assert!(result.is_err());
    }
}
