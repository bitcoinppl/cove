use bdk_wallet::{bitcoin::bip32::Xpub, descriptor::ExtendedDescriptor};
use bip39::Mnemonic;
use cove_device::keychain::{
    Keychain, KeychainError, WalletSecret as KeychainWalletSecret, WalletXprv,
};
use tracing::warn;
use zeroize::Zeroizing;

use crate::backup::error::BackupError;
use crate::backup::model::{WalletBackup, WalletSecret};
use crate::backup::recovery::{RestoreArtifactSnapshot, ValidatedRestoreWalletId};
use crate::wallet::metadata::{WalletId, WalletMetadata};

use super::{
    HotWalletWrites, PreparedHotWallet, PreparedPublicWallet, PublicWalletWrites, RestoreCleanup,
    WalletWrites, prepare_public_wallet, public_descriptor_pair, with_restore_journal,
};

/// Why a cloud restore refused to touch the local data a wallet id already owns
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub(crate) enum LocalWalletConflict {
    /// Local wallet data exists for this wallet id and differs from the backup
    #[error("local wallet data does not match the backup")]
    Mismatch,

    /// Local wallet data exists for this wallet id but could not be read to compare it
    #[error("local wallet data could not be read")]
    Unreadable,
}

/// A cloud restore failure, keeping local conflicts distinct from every other failure
///
/// A local conflict means the restore wrote nothing and kept local data unchanged,
/// which the reader must be told instead of a generic failure
#[derive(Debug, thiserror::Error)]
pub(crate) enum CloudRestoreError {
    #[error(transparent)]
    LocalConflict(#[from] LocalWalletConflict),

    #[error(transparent)]
    Backup(#[from] BackupError),
}

/// Decide what a restore does with one keychain item
///
/// Local data the backup cannot account for is a conflict: a restore must never
/// write over local data it did not verify
fn plan_entry<T: PartialEq>(
    local: Option<T>,
    incoming: Option<T>,
) -> Result<Option<T>, LocalWalletConflict> {
    match (local, incoming) {
        (None, None) => Ok(None),
        (None, Some(incoming)) => Ok(Some(incoming)),
        (Some(local), Some(incoming)) if local == incoming => Ok(None),
        _ => Err(LocalWalletConflict::Mismatch),
    }
}

/// The keychain items a wallet id already owns on this device
struct LocalWalletItems {
    secret: Option<KeychainWalletSecret>,
    xpub: Option<Xpub>,
    descriptors: Option<(ExtendedDescriptor, ExtendedDescriptor)>,
    tap_signer_backup: Option<Zeroizing<Vec<u8>>>,
}

impl LocalWalletItems {
    /// Read every wallet-owned keychain item for comparison against a backup
    ///
    /// A read failure means local data exists that cannot be compared, so the
    /// restore reports a conflict instead of writing over it
    fn read(id: &WalletId) -> Result<Self, LocalWalletConflict> {
        let keychain = Keychain::global();
        let unreadable = |kind: &str, error: KeychainError| {
            warn!("cloud restore cannot read local {kind} for wallet id={id}: {error}");
            LocalWalletConflict::Unreadable
        };

        Ok(Self {
            secret: keychain
                .get_wallet_secret(id)
                .map_err(|error| unreadable("wallet secret", error))?,
            xpub: keychain.get_wallet_xpub(id).map_err(|error| unreadable("xpub", error))?,
            descriptors: keychain
                .get_public_descriptor(id)
                .map_err(|error| unreadable("descriptors", error))?,
            tap_signer_backup: keychain
                .get_tap_signer_backup(id)
                .map_err(|error| unreadable("TapSigner backup", error))?,
        })
    }
}

/// A cloud restore whose every pre-existing local item was checked against the backup
///
/// The validating constructors are the only way to build this type, so planned
/// writes can never skip the comparison that decided them
struct VerifiedCloudRestorePlan<W> {
    snapshot: RestoreArtifactSnapshot,
    writes: W,
}

impl<W: WalletWrites> VerifiedCloudRestorePlan<W> {
    /// Run the planned writes under a journal that preserves every adopted item
    fn execute(self, metadata: &WalletMetadata) -> Result<Vec<String>, (BackupError, Vec<String>)> {
        let Self { snapshot, writes } = self;

        with_restore_journal(metadata, RestoreCleanup::Preserve(&snapshot), || {
            writes.apply(metadata)
        })
    }
}

/// Capture the local artifacts a wallet id owns and read its keychain items
///
/// Metadata rows, BDK artifacts and occupied wallet-data paths still reserve the
/// wallet id outright: only keychain items can be verified and adopted
fn capture_local_wallet_state(
    metadata: &WalletMetadata,
) -> Result<(RestoreArtifactSnapshot, LocalWalletItems), CloudRestoreError> {
    let wallet_id = &metadata.id;
    let id = ValidatedRestoreWalletId::validate(wallet_id).map_err(CloudRestoreError::Backup)?;

    let snapshot = match RestoreArtifactSnapshot::capture(&id) {
        Ok(snapshot) => snapshot,
        // capture only reads this wallet id, so a keychain failure is local data we cannot compare
        Err(BackupError::Keychain(error)) => {
            warn!(
                "cloud restore cannot snapshot local keychain data for wallet id={wallet_id}: {error}"
            );
            return Err(LocalWalletConflict::Unreadable.into());
        }
        Err(error) => return Err(error.into()),
    };

    if snapshot.metadata || !snapshot.bdk_paths.is_empty() || snapshot.wallet_data_occupied {
        return Err(BackupError::WalletIdOccupied(wallet_id.clone()).into());
    }

    // rollback can only preserve keychain kinds the snapshot fingerprinted, so a
    // wallet id holding items the snapshot could not describe stays untouched
    if snapshot.keychain_items && snapshot.keychain_entries.is_empty() {
        warn!("cloud restore found undescribed local keychain items for wallet id={wallet_id}");
        return Err(LocalWalletConflict::Unreadable.into());
    }

    let items = LocalWalletItems::read(wallet_id)?;

    Ok((snapshot, items))
}

impl VerifiedCloudRestorePlan<HotWalletWrites> {
    fn prepare(
        metadata: &WalletMetadata,
        prepared: PreparedHotWallet,
    ) -> Result<Self, CloudRestoreError> {
        let (snapshot, local) = capture_local_wallet_state(metadata)?;

        // a hot wallet backup carries no TapSigner backup, so a local one is data we cannot verify
        if local.tap_signer_backup.is_some() {
            return Err(LocalWalletConflict::Mismatch.into());
        }

        let PreparedHotWallet { secret, xpub, descriptors } = prepared;
        let writes = HotWalletWrites {
            secret: plan_entry(local.secret, Some(secret))?,
            xpub: plan_entry(local.xpub, Some(xpub))?,
            descriptors: plan_entry(local.descriptors, Some(public_descriptor_pair(&descriptors)))?,
            bdk_descriptors: descriptors,
        };

        Ok(Self { snapshot, writes })
    }
}

impl VerifiedCloudRestorePlan<PublicWalletWrites> {
    fn prepare(
        metadata: &WalletMetadata,
        prepared: PreparedPublicWallet,
    ) -> Result<Self, CloudRestoreError> {
        let (snapshot, local) = capture_local_wallet_state(metadata)?;

        // a public wallet backup carries no private key, so a local secret is data we cannot verify
        if local.secret.is_some() {
            return Err(LocalWalletConflict::Mismatch.into());
        }

        let PreparedPublicWallet { xpub, descriptors, tap_signer_backup, .. } = prepared;
        let writes = PublicWalletWrites {
            bdk_descriptors: descriptors.clone(),
            xpub: plan_entry(local.xpub, xpub)?,
            descriptors: plan_entry(local.descriptors, descriptors)?,
            tap_signer_backup: plan_entry(
                local.tap_signer_backup,
                tap_signer_backup.map(Zeroizing::new),
            )?,
        };

        Ok(Self { snapshot, writes })
    }
}

pub(crate) fn restore_cloud_mnemonic_wallet(
    metadata: &WalletMetadata,
    mnemonic: Mnemonic,
) -> Result<(), CloudRestoreError> {
    restore_cloud_hot_wallet(metadata, KeychainWalletSecret::Mnemonic(mnemonic))
}

pub(crate) fn restore_cloud_xpriv_wallet(
    metadata: &WalletMetadata,
    xpriv: WalletXprv,
) -> Result<(), CloudRestoreError> {
    restore_cloud_hot_wallet(metadata, KeychainWalletSecret::Xpriv(xpriv))
}

fn restore_cloud_hot_wallet(
    metadata: &WalletMetadata,
    secret: KeychainWalletSecret,
) -> Result<(), CloudRestoreError> {
    let prepared = PreparedHotWallet::from_secret(secret, metadata);
    let plan = VerifiedCloudRestorePlan::<HotWalletWrites>::prepare(metadata, prepared)?;

    report_cloud_restore(metadata, plan.execute(metadata))
}

pub(crate) fn restore_cloud_descriptor_wallet(
    metadata: &WalletMetadata,
    backup: &WalletBackup,
) -> Result<(), CloudRestoreError> {
    let prepared =
        prepare_public_wallet(backup, metadata, matches!(&backup.secret, WalletSecret::Unknown))
            .map_err(CloudRestoreError::Backup)?;
    let plan = VerifiedCloudRestorePlan::<PublicWalletWrites>::prepare(metadata, prepared)?;

    report_cloud_restore(metadata, plan.execute(metadata))
}

/// Log the cleanup warnings a cloud restore left behind and drop them from the result
fn report_cloud_restore(
    metadata: &WalletMetadata,
    result: Result<Vec<String>, (BackupError, Vec<String>)>,
) -> Result<(), CloudRestoreError> {
    let (result, warnings) = match result {
        Ok(warnings) => (Ok(()), warnings),
        Err((error, warnings)) => (Err(error.into()), warnings),
    };

    let name = &metadata.name;
    for warning in warnings {
        warn!("cloud restore cleanup warning for {name}: {warning}");
    }

    result
}

#[cfg(test)]
mod tests {
    use std::str::FromStr as _;
    use std::sync::Arc;

    use crate::database::Database;
    use crate::wallet::fingerprint::Fingerprint;
    use crate::wallet::metadata::WalletType;
    use crate::wallet_secret::WalletSecretExt as _;

    use super::*;

    fn hot_metadata(name: &str) -> WalletMetadata {
        let mut metadata = WalletMetadata::preview_new();
        metadata.name = name.to_string();
        metadata.wallet_type = WalletType::Hot;
        metadata.master_fingerprint = Some(Arc::new(Fingerprint::from(
            bdk_wallet::bitcoin::bip32::Fingerprint::from_str("817e7be0").unwrap(),
        )));

        metadata
    }

    const KEYCHAIN_KEY_SUFFIXES: [&str; 6] = [
        "::wallet_mnemonic",
        "::wallet_mnemonic_encryption_key_and_nonce",
        "::wallet_xpub",
        "::wallet_public_descriptor",
        "::tap_signer_backup",
        "::wallet_tap_signer_encryption_key_and_nonce_key_name",
    ];

    fn init_cloud_restore_test_state() {
        crate::database::test_support::delete_database();
        crate::test_support::init_test_keychain();
        crate::test_support::shared_mock_keychain().reset();
    }

    /// The raw stored values, so a rewrite of an adopted item is visible even
    /// when the decrypted value would still compare equal
    fn raw_keychain_entries(id: &WalletId) -> Vec<(String, Option<String>)> {
        let keychain = crate::test_support::shared_mock_keychain();

        KEYCHAIN_KEY_SUFFIXES
            .iter()
            .map(|suffix| {
                let key = format!("{id}{suffix}");
                let value = keychain.get_entry(&key);
                (key, value)
            })
            .collect()
    }

    fn cloud_restore_metadata(name: &str, wallet_type: WalletType) -> WalletMetadata {
        let mut metadata = hot_metadata(name);
        metadata.wallet_type = wallet_type;
        metadata.id = WalletId::preview_new_random();
        metadata
    }

    fn backup_mnemonic() -> Mnemonic {
        Mnemonic::from_str(
            "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about",
        )
        .unwrap()
    }

    fn unrelated_mnemonic() -> Mnemonic {
        Mnemonic::from_str("zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo wrong").unwrap()
    }

    fn hot_wallet_items(
        metadata: &WalletMetadata,
        secret: KeychainWalletSecret,
    ) -> (KeychainWalletSecret, Xpub, (ExtendedDescriptor, ExtendedDescriptor)) {
        let xpub = secret.xpub(metadata.network);
        let descriptors = secret.clone().into_descriptors(metadata.network, metadata.address_type);

        (secret, xpub, public_descriptor_pair(&descriptors))
    }

    fn public_wallet_backup(
        metadata: &WalletMetadata,
        xpub: Option<Xpub>,
        descriptors: Option<(ExtendedDescriptor, ExtendedDescriptor)>,
        secret: WalletSecret,
    ) -> WalletBackup {
        WalletBackup {
            metadata: serde_json::to_value(metadata).unwrap(),
            secret,
            descriptors: descriptors.map(|(external, internal)| {
                crate::backup::model::DescriptorPair {
                    external: external.to_string(),
                    internal: internal.to_string(),
                }
            }),
            xpub: xpub.map(|xpub| xpub.to_string()),
            labels_jsonl: None,
        }
    }

    fn restored_metadata_exists(metadata: &WalletMetadata) -> bool {
        Database::global()
            .wallets
            .get(&metadata.id, metadata.network, metadata.wallet_mode)
            .unwrap()
            .is_some()
    }

    fn bdk_artifacts_exist(id: &WalletId) -> bool {
        crate::bdk_store::BdkStore::wallet_store_artifact_paths(id)
            .into_iter()
            .any(|path| path.exists())
    }

    #[test]
    fn cloud_restore_adopts_matching_hot_wallet_keychain_items() {
        let _guard = crate::test_support::global_state_test_lock().blocking_lock();
        init_cloud_restore_test_state();

        let metadata = cloud_restore_metadata("Reinstalled hot wallet", WalletType::Hot);
        let mnemonic = backup_mnemonic();
        let (secret, xpub, (external, internal)) =
            hot_wallet_items(&metadata, KeychainWalletSecret::Mnemonic(mnemonic.clone()));
        let keychain = Keychain::global();
        keychain.save_wallet_secret(&metadata.id, secret).unwrap();
        keychain.save_wallet_xpub(&metadata.id, xpub).unwrap();
        keychain.save_public_descriptor(&metadata.id, external, internal).unwrap();
        let before = raw_keychain_entries(&metadata.id);

        restore_cloud_mnemonic_wallet(&metadata, mnemonic).expect("adopt matching keychain items");

        assert_eq!(raw_keychain_entries(&metadata.id), before);
        assert!(restored_metadata_exists(&metadata));
    }

    #[test]
    fn cloud_restore_adopts_matching_xpriv_and_creates_missing_items() {
        let _guard = crate::test_support::global_state_test_lock().blocking_lock();
        init_cloud_restore_test_state();

        let metadata = cloud_restore_metadata("Reinstalled xpriv wallet", WalletType::Hot);
        let xpriv = WalletXprv::try_from(
            bdk_wallet::bitcoin::bip32::Xpriv::new_master(
                bdk_wallet::bitcoin::NetworkKind::Main,
                &[7; 32],
            )
            .unwrap(),
        )
        .unwrap();
        let (secret, _, _) =
            hot_wallet_items(&metadata, KeychainWalletSecret::Xpriv(xpriv.clone()));
        Keychain::global().save_wallet_secret(&metadata.id, secret).unwrap();
        let before = raw_keychain_entries(&metadata.id);

        restore_cloud_xpriv_wallet(&metadata, xpriv).expect("adopt matching wallet secret");

        let after = raw_keychain_entries(&metadata.id);
        assert_eq!(after[0], before[0]);
        assert_eq!(after[1], before[1]);
        assert!(Keychain::global().get_wallet_xpub(&metadata.id).unwrap().is_some());
        assert!(Keychain::global().get_public_descriptor(&metadata.id).unwrap().is_some());
        assert!(restored_metadata_exists(&metadata));
    }

    #[test]
    fn cloud_restore_adopts_matching_xpub_and_creates_the_missing_secret() {
        let _guard = crate::test_support::global_state_test_lock().blocking_lock();
        init_cloud_restore_test_state();

        let metadata = cloud_restore_metadata("Reinstalled xpub leftover", WalletType::Hot);
        let mnemonic = backup_mnemonic();
        let (_, xpub, _) =
            hot_wallet_items(&metadata, KeychainWalletSecret::Mnemonic(mnemonic.clone()));
        Keychain::global().save_wallet_xpub(&metadata.id, xpub).unwrap();
        let before = raw_keychain_entries(&metadata.id);

        restore_cloud_mnemonic_wallet(&metadata, mnemonic.clone()).expect("adopt matching xpub");

        let after = raw_keychain_entries(&metadata.id);
        assert_eq!(after[2], before[2]);
        assert_eq!(
            Keychain::global().get_wallet_secret(&metadata.id).unwrap(),
            Some(KeychainWalletSecret::Mnemonic(mnemonic))
        );
        assert!(Keychain::global().get_public_descriptor(&metadata.id).unwrap().is_some());
        assert!(restored_metadata_exists(&metadata));
    }

    #[test]
    fn cloud_restore_adopts_matching_public_wallet_keychain_items() {
        let _guard = crate::test_support::global_state_test_lock().blocking_lock();
        init_cloud_restore_test_state();

        let metadata = cloud_restore_metadata("Reinstalled watch-only wallet", WalletType::Cold);
        let (_, xpub, (external, internal)) =
            hot_wallet_items(&metadata, KeychainWalletSecret::Mnemonic(backup_mnemonic()));
        let keychain = Keychain::global();
        keychain.save_wallet_xpub(&metadata.id, xpub).unwrap();
        keychain.save_public_descriptor(&metadata.id, external.clone(), internal.clone()).unwrap();
        let before = raw_keychain_entries(&metadata.id);
        let backup = public_wallet_backup(
            &metadata,
            Some(xpub),
            Some((external, internal)),
            WalletSecret::None,
        );

        restore_cloud_descriptor_wallet(&metadata, &backup)
            .expect("adopt matching public keychain items");

        assert_eq!(raw_keychain_entries(&metadata.id), before);
        assert!(restored_metadata_exists(&metadata));
    }

    #[test]
    fn cloud_restore_adopts_a_matching_tap_signer_backup() {
        let _guard = crate::test_support::global_state_test_lock().blocking_lock();
        init_cloud_restore_test_state();

        let metadata = cloud_restore_metadata("Reinstalled TapSigner wallet", WalletType::Cold);
        let (_, xpub, (external, internal)) =
            hot_wallet_items(&metadata, KeychainWalletSecret::Mnemonic(backup_mnemonic()));
        let tap_signer_backup = vec![9u8; 64];
        let keychain = Keychain::global();
        keychain.save_wallet_xpub(&metadata.id, xpub).unwrap();
        keychain.save_public_descriptor(&metadata.id, external.clone(), internal.clone()).unwrap();
        keychain.save_tap_signer_backup(&metadata.id, &tap_signer_backup).unwrap();
        let before = raw_keychain_entries(&metadata.id);
        let backup = public_wallet_backup(
            &metadata,
            Some(xpub),
            Some((external, internal)),
            WalletSecret::TapSignerBackup(tap_signer_backup.clone()),
        );

        restore_cloud_descriptor_wallet(&metadata, &backup)
            .expect("adopt matching TapSigner backup");

        assert_eq!(raw_keychain_entries(&metadata.id), before);
        assert_eq!(
            keychain.get_tap_signer_backup(&metadata.id).unwrap().as_deref(),
            Some(&tap_signer_backup)
        );
        assert!(restored_metadata_exists(&metadata));
    }

    #[test]
    fn cloud_restore_keeps_a_mismatched_wallet_secret_and_writes_nothing() {
        let _guard = crate::test_support::global_state_test_lock().blocking_lock();
        init_cloud_restore_test_state();

        let metadata = cloud_restore_metadata("Mismatched hot wallet", WalletType::Hot);
        let (secret, xpub, (external, internal)) =
            hot_wallet_items(&metadata, KeychainWalletSecret::Mnemonic(unrelated_mnemonic()));
        let keychain = Keychain::global();
        keychain.save_wallet_secret(&metadata.id, secret).unwrap();
        keychain.save_wallet_xpub(&metadata.id, xpub).unwrap();
        keychain.save_public_descriptor(&metadata.id, external, internal).unwrap();
        let before = raw_keychain_entries(&metadata.id);

        let result = restore_cloud_mnemonic_wallet(&metadata, backup_mnemonic());

        assert!(matches!(
            result,
            Err(CloudRestoreError::LocalConflict(LocalWalletConflict::Mismatch))
        ));
        assert_eq!(raw_keychain_entries(&metadata.id), before);
        assert!(!restored_metadata_exists(&metadata));
        assert!(!bdk_artifacts_exist(&metadata.id));
    }

    #[test]
    fn cloud_restore_keeps_an_unreadable_wallet_secret_and_writes_nothing() {
        let _guard = crate::test_support::global_state_test_lock().blocking_lock();
        init_cloud_restore_test_state();

        let metadata = cloud_restore_metadata("Unreadable hot wallet", WalletType::Hot);
        let secret_key = format!("{}::wallet_mnemonic", metadata.id);
        // an encrypted secret whose cryptor is missing cannot be compared to the backup
        crate::test_support::shared_mock_keychain()
            .set_entries(vec![(secret_key.as_str(), "half-written-ciphertext")]);
        let before = raw_keychain_entries(&metadata.id);

        let result = restore_cloud_mnemonic_wallet(&metadata, backup_mnemonic());

        assert!(matches!(
            result,
            Err(CloudRestoreError::LocalConflict(LocalWalletConflict::Unreadable))
        ));
        assert_eq!(raw_keychain_entries(&metadata.id), before);
        assert!(!restored_metadata_exists(&metadata));
        assert!(!bdk_artifacts_exist(&metadata.id));
    }

    #[test]
    fn cloud_restore_keeps_a_local_secret_a_public_backup_cannot_explain() {
        let _guard = crate::test_support::global_state_test_lock().blocking_lock();
        init_cloud_restore_test_state();

        let metadata = cloud_restore_metadata("Unexpected secret", WalletType::Cold);
        let (secret, xpub, _) =
            hot_wallet_items(&metadata, KeychainWalletSecret::Mnemonic(backup_mnemonic()));
        let keychain = Keychain::global();
        keychain.save_wallet_secret(&metadata.id, secret).unwrap();
        keychain.save_wallet_xpub(&metadata.id, xpub).unwrap();
        let before = raw_keychain_entries(&metadata.id);
        let backup = public_wallet_backup(&metadata, Some(xpub), None, WalletSecret::None);

        let result = restore_cloud_descriptor_wallet(&metadata, &backup);

        assert!(matches!(
            result,
            Err(CloudRestoreError::LocalConflict(LocalWalletConflict::Mismatch))
        ));
        assert_eq!(raw_keychain_entries(&metadata.id), before);
        assert!(!restored_metadata_exists(&metadata));
    }

    #[test]
    fn cloud_restore_refuses_a_wallet_id_that_already_owns_bdk_artifacts() {
        let _guard = crate::test_support::global_state_test_lock().blocking_lock();
        init_cloud_restore_test_state();

        let metadata = cloud_restore_metadata("Occupied by BDK data", WalletType::Hot);
        let mnemonic = backup_mnemonic();
        let (_, xpub, _) =
            hot_wallet_items(&metadata, KeychainWalletSecret::Mnemonic(mnemonic.clone()));
        Keychain::global().save_wallet_xpub(&metadata.id, xpub).unwrap();
        let artifact = crate::bdk_store::BdkStore::wallet_store_artifact_paths(&metadata.id)
            .into_iter()
            .find(|path| path.to_string_lossy().ends_with("-wal"))
            .expect("wallet store artifact paths include a WAL path");
        if let Some(parent) = artifact.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(&artifact, b"pre-existing WAL").unwrap();
        let before = raw_keychain_entries(&metadata.id);

        let result = restore_cloud_mnemonic_wallet(&metadata, mnemonic);

        assert!(matches!(
            result,
            Err(CloudRestoreError::Backup(BackupError::WalletIdOccupied(id)))
                if id == metadata.id
        ));
        assert_eq!(raw_keychain_entries(&metadata.id), before);
        assert!(!restored_metadata_exists(&metadata));
        std::fs::remove_file(artifact).unwrap();
    }
}
