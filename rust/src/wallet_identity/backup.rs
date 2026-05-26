use crate::backup::model::WalletBackup;
use crate::wallet::metadata::{WalletId, WalletMetadata, WalletType};

use super::{PublicWalletIdentity, WalletIdentityError, WalletIdentityKey};

pub(crate) fn identity_key_for_backup(
    metadata: &WalletMetadata,
    backup: &WalletBackup,
) -> Result<WalletIdentityKey, WalletIdentityError> {
    if metadata.wallet_type == WalletType::Hot && metadata.master_fingerprint.is_some() {
        return Ok(fallback_identity_key_for_backup(metadata));
    }

    if let Some(identity) = public_identity_from_backup(metadata, backup)? {
        return Ok(WalletIdentityKey::PublicIdentity {
            identity,
            fingerprint: metadata.master_fingerprint.as_deref().copied(),
            wallet_id: no_fingerprint_wallet_id(metadata),
            network: metadata.network,
            mode: metadata.wallet_mode,
        });
    }

    Ok(fallback_identity_key_for_backup(metadata))
}

pub(crate) fn fallback_identity_key_for_backup(metadata: &WalletMetadata) -> WalletIdentityKey {
    if metadata.wallet_type == WalletType::Hot
        && let Some(fingerprint) = metadata.master_fingerprint.as_deref().copied()
    {
        return WalletIdentityKey::HotFingerprint {
            fingerprint,
            network: metadata.network,
            mode: metadata.wallet_mode,
        };
    }

    if let Some(fingerprint) = metadata.master_fingerprint.as_deref().copied() {
        return WalletIdentityKey::Fingerprint {
            fingerprint,
            network: metadata.network,
            mode: metadata.wallet_mode,
        };
    }

    WalletIdentityKey::WalletId {
        id: metadata.id.clone(),
        network: metadata.network,
        mode: metadata.wallet_mode,
    }
}

pub(crate) fn no_fingerprint_wallet_id(metadata: &WalletMetadata) -> Option<WalletId> {
    metadata.master_fingerprint.is_none().then(|| metadata.id.clone())
}

fn public_identity_from_backup(
    metadata: &WalletMetadata,
    backup: &WalletBackup,
) -> Result<Option<PublicWalletIdentity>, WalletIdentityError> {
    if let Some(descriptors) = &backup.descriptors {
        let identity = PublicWalletIdentity::from_descriptor_strs(
            &descriptors.external,
            &descriptors.internal,
        )
        .map_err(|source| WalletIdentityError::BackupDescriptor {
            wallet_name: metadata.name.clone(),
            source,
        })?;

        return Ok(Some(identity));
    }

    if let Some(xpub) = &backup.xpub {
        let fingerprint = metadata.master_fingerprint.as_deref().copied();
        let identity = PublicWalletIdentity::from_xpub_str_default_address_type(
            xpub,
            fingerprint,
            metadata.network,
            metadata.address_type,
        )
        .map_err(|source| WalletIdentityError::BackupXpub {
            wallet_name: metadata.name.clone(),
            source,
        })?;

        return Ok(Some(identity));
    }

    Ok(None)
}
