//! Keychain writes a producer proved off the actor, saved as a write-lane effect
//!
//! Verification and missing-key recovery prove a passkey and a master key without writing the
//! keychain themselves. They hand the writes to the lane with the authority they read first, and
//! the lane decides them at its queue head like any command: a finished operation or a replaced
//! configuration is stale, and background work waits while Turn Off, an account switch or a reset
//! runs. The save then runs on a blocking thread as the lane's in-flight command, so Turn Off's
//! drain and a reset's wait for an idle lane both wait for it instead of clearing beside it

use cove_device::keychain::Keychain;

use crate::manager::cloud_backup_manager::CloudBackupError;
use crate::manager::cloud_backup_manager::keychain::CloudBackupKeychain;

/// A keychain write a producer prepared for the lane to save
pub(crate) enum OwnedKeychainWrite {
    /// The passkey that just decrypted the cloud master key
    Passkey { credential_id: Vec<u8>, prf_salt: [u8; 32] },
    /// The cloud master key that passkey proved, replacing a missing or stale local key
    MasterKey(cove_cspp::master_key::MasterKey),
}

impl std::fmt::Debug for OwnedKeychainWrite {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Passkey { .. } => f.write_str("Passkey(<redacted>)"),
            Self::MasterKey(_) => f.write_str("MasterKey(<redacted>)"),
        }
    }
}

/// What a keychain save did
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum KeychainSave {
    Saved,
    /// The keychain holds another namespace now, so these keys were not saved
    NamespaceChanged,
}

/// Saves `writes` if the keychain still holds `namespace_id`
///
/// A reinitialize replaces the namespace inside one configuration generation, so the generation
/// alone doesn't prove the keys still belong here. Blocks on platform keychain calls, so the lane
/// runs it on a blocking thread
pub(crate) fn save_owned_keychain_writes(
    namespace_id: &str,
    writes: Vec<OwnedKeychainWrite>,
) -> Result<KeychainSave, CloudBackupError> {
    let keychain = Keychain::global();
    let cloud_keychain = CloudBackupKeychain::new(keychain.clone());
    let current = cloud_keychain.namespace_id().map_err(|source| {
        CloudBackupError::internal_context("read cloud backup namespace", source)
    })?;

    if current.as_deref() != Some(namespace_id) {
        return Ok(KeychainSave::NamespaceChanged);
    }

    for write in writes {
        match write {
            OwnedKeychainWrite::Passkey { credential_id, prf_salt } => {
                cloud_keychain.save_passkey(&credential_id, prf_salt).map_err(|source| {
                    CloudBackupError::internal_context("save cloud backup passkey", source)
                })?;
            }

            OwnedKeychainWrite::MasterKey(master_key) => {
                cove_cspp::Cspp::new(keychain.clone()).save_master_key(&master_key).map_err(
                    |source| CloudBackupError::internal_context("save local master key", source),
                )?;
            }
        }
    }

    Ok(KeychainSave::Saved)
}
