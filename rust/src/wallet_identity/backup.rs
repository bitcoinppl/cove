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
        return Ok(public_identity_key(metadata, identity));
    }

    Ok(fallback_identity_key_for_backup(metadata))
}

/// The key of a wallet restored from only the public half of `backup`
///
/// A record whose metadata says hot can still hold no secret, and the wallet restored from it
/// cannot sign, so its key never claims the hot fingerprint a later seed backup checks for
pub(crate) fn public_identity_key_for_backup(
    metadata: &WalletMetadata,
    backup: &WalletBackup,
) -> WalletIdentityKey {
    match public_identity_from_backup(metadata, backup) {
        Ok(Some(identity)) => public_identity_key(metadata, identity),
        Ok(None) | Err(_) => public_fallback_identity_key(metadata),
    }
}

fn public_identity_key(
    metadata: &WalletMetadata,
    identity: PublicWalletIdentity,
) -> WalletIdentityKey {
    WalletIdentityKey::PublicIdentity {
        identity,
        fingerprint: metadata.master_fingerprint.as_deref().copied(),
        wallet_id: no_fingerprint_wallet_id(metadata),
        network: metadata.network,
        mode: metadata.wallet_mode,
    }
}

fn public_fallback_identity_key(metadata: &WalletMetadata) -> WalletIdentityKey {
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

    public_fallback_identity_key(metadata)
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
        .map_err(WalletIdentityError::BackupDescriptor)?;

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
        .map_err(WalletIdentityError::BackupXpub)?;

        return Ok(Some(identity));
    }

    Ok(None)
}
