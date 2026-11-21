use bdk_wallet::bitcoin::bip32::Xpub;
use bdk_wallet::descriptor::ExtendedDescriptor;
use bip39::Mnemonic;
use cove_device::keychain::{WalletSecret as KeychainWalletSecret, WalletXprv};
use tracing::{info, warn};
use zeroize::Zeroizing;

use crate::backup::error::BackupError;
use crate::backup::model::{WalletBackup, WalletSecret};
use crate::backup::recovery::{
    RestoreArtifactSnapshot, SnapshotCaptureError, StorageOccupancy, ValidatedRestoreWalletId,
    WalletKeychainItems,
};
use crate::wallet::metadata::{WalletId, WalletMetadata};
use crate::wallet_identity::PublicWalletIdentity;
use cove_bdk::descriptor_ext::DescriptorExt as _;

use super::{
    HotWalletWrites, PreparedHotWallet, PreparedPublicWallet, PublicWalletWrites, RestoreCleanup,
    WalletWrites, prepare_public_wallet, public_descriptor_pair, with_restore_journal,
};

/// A cloud restore failure
#[derive(Debug, thiserror::Error)]
pub(crate) enum CloudRestoreError {
    /// The keychain could not be read at all, such as while the device is locked, so the
    /// restore wrote nothing and a retry can succeed
    #[error("local wallet data could not be read")]
    LocalKeychainUnavailable,

    #[error(transparent)]
    Backup(#[from] BackupError),
}

/// Which wallet id a cloud restore put the backup's wallet under
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CloudRestorePlacement {
    /// The backup's own wallet id, keeping any local items that describe the restored wallet
    BackupWalletId,

    /// A new wallet id, because local data under the backup's id belongs to another wallet or
    /// cannot be decoded
    ///
    /// That data stays untouched, and the backup's record is not this wallet's own backup, so
    /// the wallet needs one of its own
    NewWalletId,
}

/// The wallet a cloud restore created
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CloudRestoredWallet {
    metadata: Box<WalletMetadata>,
    placement: CloudRestorePlacement,
}

impl CloudRestoredWallet {
    /// The restored wallet's metadata, whose id is a new one for `NewWalletId`
    pub(crate) fn metadata(&self) -> &WalletMetadata {
        &self.metadata
    }

    pub(crate) fn into_metadata(self) -> WalletMetadata {
        *self.metadata
    }

    pub(crate) fn placement(&self) -> CloudRestorePlacement {
        self.placement
    }
}

/// Local data under the backup's wallet id that a restore must leave untouched
///
/// The restore moves the wallet to a new id instead, so this never fails a restore
#[derive(Debug)]
struct Unadoptable;

/// What a restore does with one keychain item
#[derive(Debug, Clone, Copy)]
enum ItemPlan {
    /// No local item exists, so the restore writes the backup's
    Write,

    /// The local item describes the restored wallet, so the restore keeps it as it is
    Keep,
}

impl ItemPlan {
    /// The value to write, or `None` when the restore keeps the local item
    fn take<T>(self, incoming: Option<T>) -> Option<T> {
        match self {
            Self::Write => incoming,
            Self::Keep => None,
        }
    }
}

/// Decide what a restore does with one keychain item
///
/// A local item is kept only when `describes` confirms it belongs to the restored wallet;
/// any other local item is data the restore must neither adopt nor write over
fn plan_item<L: ?Sized, I: ?Sized>(
    metadata: &WalletMetadata,
    item: &str,
    local: Option<&L>,
    incoming: Option<&I>,
    describes: impl FnOnce(&L, &I) -> bool,
) -> Result<ItemPlan, Unadoptable> {
    let Some(local) = local else {
        return Ok(ItemPlan::Write);
    };

    if let Some(incoming) = incoming
        && describes(local, incoming)
    {
        return Ok(ItemPlan::Keep);
    }

    // every unadoptable item moves the wallet to a new id, so only this log says which one
    let backup = if incoming.is_some() { "a different" } else { "no" };
    warn!(
        "cloud restore cannot adopt wallet id={}: local {item} differs, backup has {backup} {item}",
        metadata.id
    );

    Err(Unadoptable)
}

/// Log a local item the backup kind cannot hold, so the restore cannot adopt the wallet id
fn unverifiable_local_item(metadata: &WalletMetadata, item: &str) -> Unadoptable {
    warn!(
        "cloud restore cannot adopt wallet id={}: local {item} has no backup to verify",
        metadata.id
    );
    Unadoptable
}

/// Whether `local` is the same key as any of `known`, whatever network form each is encoded in
fn is_known_xpub(local: &Xpub, known: &[Xpub]) -> bool {
    known.iter().any(|known| same_xpub_key(local, known))
}

/// Whether two extended public keys are the same key, whatever network form each is encoded in
///
/// A key imported from a descriptor keeps the version bytes it was written with, which can
/// differ from the form a restore derives for the wallet's network
fn same_xpub_key(left: &Xpub, right: &Xpub) -> bool {
    let mut right = *right;
    right.network = left.network;

    *left == right
}

fn same_descriptors(
    left: &(ExtendedDescriptor, ExtendedDescriptor),
    right: &(ExtendedDescriptor, ExtendedDescriptor),
) -> bool {
    PublicWalletIdentity::from_descriptor_pair(&left.0, &left.1)
        == PublicWalletIdentity::from_descriptor_pair(&right.0, &right.1)
}

/// A cloud restore whose every pre-existing local item was checked against the backup
///
/// The validating constructors are the only way to build this type, so planned
/// writes can never skip the comparison that decided them
struct VerifiedCloudRestorePlan<W> {
    snapshot: RestoreArtifactSnapshot,
    writes: W,
}

/// Where a cloud restore can put the backup's wallet
enum LocalFit<W, P> {
    /// Under the backup's wallet id, writing only the items that are missing there
    Adopt(Box<VerifiedCloudRestorePlan<W>>),

    /// Under a new wallet id, since the backup's id holds data the restore must not touch
    Unadoptable(P),
}

impl<W: WalletWrites> VerifiedCloudRestorePlan<W> {
    /// Run the planned writes under a journal that preserves every adopted item
    fn execute(self, metadata: &WalletMetadata) -> Result<CloudRestoredWallet, CloudRestoreError> {
        let Self { snapshot, writes } = self;

        let result = with_restore_journal(metadata, RestoreCleanup::Preserve(&snapshot), || {
            writes.apply(metadata)
        });
        report_cloud_restore(metadata, result)?;

        Ok(CloudRestoredWallet {
            metadata: Box::new(metadata.clone()),
            placement: CloudRestorePlacement::BackupWalletId,
        })
    }
}

/// The local state under a wallet id, with its keychain items read once
struct CapturedLocalWallet {
    snapshot: RestoreArtifactSnapshot,
    items: WalletKeychainItems,
}

/// What a restore found under the backup's wallet id
enum LocalWalletState {
    /// No local data, or keychain items that were each read and fingerprinted, so rollback
    /// can keep them
    Captured(Box<CapturedLocalWallet>),

    /// Keychain items that are stored but cannot be decoded or described, so the restore can
    /// neither compare them nor preserve them through a rollback
    Unadoptable,
}

/// What local storage under the backup's wallet id leaves a restore free to do
enum StorageFit {
    /// Nothing reserves the id, so only its keychain items decide
    Free,

    /// A local wallet already lives under the id, so the backup restores under a new one
    LocalWallet,
}

/// What a restore brings back, which decides whether a local wallet under the backup's wallet id
/// moves it to a new one
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RestoredMaterial {
    /// A seed or master private key, which must never stay stuck in the cloud
    Secret,

    /// Public keys, which a local wallet under the same id most likely already watches
    Public,
}

/// Decide what a restore may do about the local storage reserving `id`
///
/// A settled wallet under the id moves a seed backup beside it, since the seed must never stay
/// stuck in the cloud because of it, such as a wallet that lost its key or a hardware wallet
/// that can't take the seed. A public backup keeps refusing it, since that wallet most likely
/// already watches the same keys. Artifacts a running restore may still be writing reserve the
/// id outright, as `WalletIdOccupied`, so the retry decides once that restore is done
fn storage_fit(
    id: &ValidatedRestoreWalletId,
    occupancy: StorageOccupancy,
    material: RestoredMaterial,
) -> Result<StorageFit, CloudRestoreError> {
    let wallet_id = id.as_wallet_id();

    match occupancy {
        StorageOccupancy::Free => Ok(StorageFit::Free),
        StorageOccupancy::WalletMetadata if id.has_running_restore() => {
            warn!("cloud restore refused wallet id={wallet_id}: another restore of it is running");
            Err(BackupError::WalletIdOccupied(wallet_id.clone()).into())
        }
        StorageOccupancy::WalletMetadata if material == RestoredMaterial::Secret => {
            warn!(
                "cloud restore cannot adopt wallet id={wallet_id}: a local wallet already uses it"
            );
            Ok(StorageFit::LocalWallet)
        }
        StorageOccupancy::WalletMetadata => {
            warn!("cloud restore refused wallet id={wallet_id}: a local wallet already uses it");
            Err(BackupError::WalletIdOccupied(wallet_id.clone()).into())
        }
        StorageOccupancy::Artifacts => {
            warn!(
                "cloud restore refused wallet id={wallet_id}: wallet data without a metadata row already uses it"
            );
            Err(BackupError::WalletIdOccupied(wallet_id.clone()).into())
        }
    }
}

/// Capture the local artifacts a wallet id owns and read its keychain items
///
/// A local wallet under the id moves a seed restore to a new id and refuses a public one, and
/// artifacts with no metadata row reserve the id outright: only keychain items can be verified
/// and adopted
///
/// A keychain the platform cannot read at all stops the restore, since nothing can be
/// written until it can be read
fn capture_local_wallet_state(
    metadata: &WalletMetadata,
    material: RestoredMaterial,
) -> Result<LocalWalletState, CloudRestoreError> {
    let wallet_id = &metadata.id;
    let id = ValidatedRestoreWalletId::validate(wallet_id)?;

    let (snapshot, items) = match RestoreArtifactSnapshot::capture_with_keychain_items(&id) {
        Ok(captured) => captured,
        Err(SnapshotCaptureError::Keychain { error, occupancy }) => {
            storage_fit(&id, occupancy, material)?;

            if error.is_keychain_unavailable() {
                warn!("cloud restore cannot read the keychain for wallet id={wallet_id}: {error}");
                return Err(CloudRestoreError::LocalKeychainUnavailable);
            }

            warn!(
                "cloud restore cannot decode local keychain data for wallet id={wallet_id}: {error}"
            );
            return Ok(LocalWalletState::Unadoptable);
        }
        Err(SnapshotCaptureError::Backup(error)) => return Err(error.into()),
    };

    if let StorageFit::LocalWallet = storage_fit(&id, snapshot.storage_occupancy(), material)? {
        return Ok(LocalWalletState::Unadoptable);
    }

    // rollback can only preserve keychain kinds the snapshot fingerprinted, so a
    // wallet id holding items the snapshot could not describe stays untouched
    if snapshot.keychain_items && snapshot.keychain_entries.is_empty() {
        warn!("cloud restore found undescribed local keychain items for wallet id={wallet_id}");
        return Ok(LocalWalletState::Unadoptable);
    }

    Ok(LocalWalletState::Captured(Box::new(CapturedLocalWallet { snapshot, items })))
}

/// What a hot wallet restore does with each local keychain item
struct HotItemPlans {
    secret: ItemPlan,
    xpub: ItemPlan,
    descriptors: ItemPlan,
}

impl HotItemPlans {
    fn new(
        metadata: &WalletMetadata,
        prepared: &PreparedHotWallet,
        local: &WalletKeychainItems,
    ) -> Result<Self, Unadoptable> {
        // a hot wallet backup carries no TapSigner backup, so a local one is data we cannot verify
        if local.tap_signer_backup.is_some() {
            return Err(unverifiable_local_item(metadata, "TapSigner backup"));
        }

        let network = metadata.network;
        let known_xpubs: Vec<Xpub> =
            std::iter::once(prepared.xpub).chain(prepared.descriptors.external.xpub()).collect();
        let incoming_descriptors = public_descriptor_pair(&prepared.descriptors);

        Ok(Self {
            // a key restored before #988 can be stored in the backup's network form, which is
            // the same key as the wallet-network form `prepared` carries
            secret: plan_item(
                metadata,
                "secret",
                local.secret.as_ref(),
                Some(&prepared.secret),
                |local, incoming| &local.clone().into_network_kind(network.into()) == incoming,
            )?,
            // a wallet imported from its public keys stores its descriptors' account xpub and
            // keeps it when its secret is added later, while a hot wallet created in Cove stores
            // the master xpub, so either key describes this wallet
            xpub: plan_item(
                metadata,
                "xpub",
                local.xpub.as_ref(),
                Some(known_xpubs.as_slice()),
                is_known_xpub,
            )?,
            descriptors: plan_item(
                metadata,
                "descriptors",
                local.descriptors.as_ref(),
                Some(&incoming_descriptors),
                same_descriptors,
            )?,
        })
    }
}

impl VerifiedCloudRestorePlan<HotWalletWrites> {
    fn prepare(
        metadata: &WalletMetadata,
        prepared: PreparedHotWallet,
    ) -> Result<LocalFit<HotWalletWrites, PreparedHotWallet>, CloudRestoreError> {
        let LocalWalletState::Captured(captured) =
            capture_local_wallet_state(metadata, RestoredMaterial::Secret)?
        else {
            return Ok(LocalFit::Unadoptable(prepared));
        };

        let CapturedLocalWallet { snapshot, items: local } = *captured;
        let Ok(plans) = HotItemPlans::new(metadata, &prepared, &local) else {
            return Ok(LocalFit::Unadoptable(prepared));
        };

        let PreparedHotWallet { secret, xpub, descriptors } = prepared;
        let writes = HotWalletWrites {
            secret: plans.secret.take(Some(secret)),
            xpub: plans.xpub.take(Some(xpub)),
            descriptors: plans.descriptors.take(Some(public_descriptor_pair(&descriptors))),
            bdk_descriptors: descriptors,
        };

        Ok(LocalFit::Adopt(Box::new(Self { snapshot, writes })))
    }
}

/// What a public wallet restore does with each local keychain item
struct PublicItemPlans {
    xpub: ItemPlan,
    descriptors: ItemPlan,
    tap_signer_backup: ItemPlan,
}

impl PublicItemPlans {
    fn new(
        metadata: &WalletMetadata,
        prepared: &PreparedPublicWallet,
        local: &WalletKeychainItems,
    ) -> Result<Self, Unadoptable> {
        // a public wallet backup carries no private key, so a local secret is data we cannot verify
        if local.secret.is_some() {
            return Err(unverifiable_local_item(metadata, "secret"));
        }

        // a descriptor backup can carry no xpub while the local item holds its descriptors' key
        let account_xpub = prepared.descriptors.as_ref().and_then(|(external, _)| external.xpub());
        let known_xpubs: Vec<Xpub> = prepared.xpub.into_iter().chain(account_xpub).collect();
        let known_xpubs = (!known_xpubs.is_empty()).then_some(known_xpubs.as_slice());

        Ok(Self {
            xpub: plan_item(metadata, "xpub", local.xpub.as_ref(), known_xpubs, is_known_xpub)?,
            descriptors: plan_item(
                metadata,
                "descriptors",
                local.descriptors.as_ref(),
                prepared.descriptors.as_ref(),
                same_descriptors,
            )?,
            tap_signer_backup: plan_item(
                metadata,
                "TapSigner backup",
                local.tap_signer_backup.as_deref().map(Vec::as_slice),
                prepared.tap_signer_backup.as_deref(),
                |local, incoming| local == incoming,
            )?,
        })
    }
}

impl VerifiedCloudRestorePlan<PublicWalletWrites> {
    fn prepare(
        metadata: &WalletMetadata,
        prepared: PreparedPublicWallet,
    ) -> Result<LocalFit<PublicWalletWrites, PreparedPublicWallet>, CloudRestoreError> {
        let LocalWalletState::Captured(captured) =
            capture_local_wallet_state(metadata, RestoredMaterial::Public)?
        else {
            return Ok(LocalFit::Unadoptable(prepared));
        };

        let CapturedLocalWallet { snapshot, items: local } = *captured;
        let Ok(plans) = PublicItemPlans::new(metadata, &prepared, &local) else {
            return Ok(LocalFit::Unadoptable(prepared));
        };

        let PreparedPublicWallet { xpub, descriptors, tap_signer_backup, .. } = prepared;
        let writes = PublicWalletWrites {
            bdk_descriptors: descriptors.clone(),
            xpub: plans.xpub.take(xpub),
            descriptors: plans.descriptors.take(descriptors),
            tap_signer_backup: plans.tap_signer_backup.take(tap_signer_backup.map(Zeroizing::new)),
        };

        Ok(LocalFit::Adopt(Box::new(Self { snapshot, writes })))
    }
}

/// Restore the backup under a new wallet id, leaving the local data under its own id untouched
///
/// The secret is what the reader cannot afford to lose, so local data the restore cannot adopt
/// never stops it from bringing the wallet back
fn restore_under_new_wallet_id(
    metadata: &WalletMetadata,
    writes: impl WalletWrites,
) -> Result<CloudRestoredWallet, CloudRestoreError> {
    let mut relocated = metadata.clone();
    relocated.id = WalletId::new();
    info!(
        "cloud restore of wallet id={} continues under new wallet id={}",
        metadata.id, relocated.id
    );

    let result =
        with_restore_journal(&relocated, RestoreCleanup::RequireEmpty, || writes.apply(&relocated));
    report_cloud_restore(&relocated, result)?;

    Ok(CloudRestoredWallet {
        metadata: Box::new(relocated),
        placement: CloudRestorePlacement::NewWalletId,
    })
}

pub(crate) fn restore_cloud_mnemonic_wallet(
    metadata: &WalletMetadata,
    mnemonic: Mnemonic,
) -> Result<CloudRestoredWallet, CloudRestoreError> {
    restore_cloud_hot_wallet(metadata, KeychainWalletSecret::Mnemonic(mnemonic))
}

pub(crate) fn restore_cloud_xpriv_wallet(
    metadata: &WalletMetadata,
    xpriv: WalletXprv,
) -> Result<CloudRestoredWallet, CloudRestoreError> {
    restore_cloud_hot_wallet(metadata, KeychainWalletSecret::Xpriv(xpriv))
}

fn restore_cloud_hot_wallet(
    metadata: &WalletMetadata,
    secret: KeychainWalletSecret,
) -> Result<CloudRestoredWallet, CloudRestoreError> {
    let prepared = PreparedHotWallet::from_secret(secret, metadata);

    match VerifiedCloudRestorePlan::<HotWalletWrites>::prepare(metadata, prepared)? {
        LocalFit::Adopt(plan) => plan.execute(metadata),
        LocalFit::Unadoptable(prepared) => {
            restore_under_new_wallet_id(metadata, HotWalletWrites::create_all(prepared))
        }
    }
}

pub(crate) fn restore_cloud_descriptor_wallet(
    metadata: &WalletMetadata,
    backup: &WalletBackup,
) -> Result<CloudRestoredWallet, CloudRestoreError> {
    let prepared = prepare_public_wallet(backup, matches!(&backup.secret, WalletSecret::Unknown))?;

    match VerifiedCloudRestorePlan::<PublicWalletWrites>::prepare(metadata, prepared)? {
        LocalFit::Adopt(plan) => plan.execute(metadata),
        LocalFit::Unadoptable(prepared) => {
            restore_under_new_wallet_id(metadata, PublicWalletWrites::create_all(prepared))
        }
    }
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

    // the wallet id, not the name: names are raw user input
    let wallet_id = &metadata.id;
    for warning in warnings {
        warn!("cloud restore cleanup warning for wallet id={wallet_id}: {warning}");
    }

    result
}

#[cfg(test)]
mod tests {
    use std::str::FromStr as _;

    use bdk_wallet::{bitcoin::bip32::Xpub, descriptor::ExtendedDescriptor};
    use cove_device::keychain::Keychain;

    use crate::database::Database;
    use crate::test_support::{
        WALLET_MNEMONIC_CRYPTOR_KEY_SUFFIX, WALLET_MNEMONIC_KEY_SUFFIX, WALLET_XPUB_KEY_SUFFIX,
        hot_wallet_metadata as hot_metadata, raw_wallet_keychain_entries as raw_keychain_entries,
    };
    use crate::wallet::metadata::{WalletId, WalletType};
    use crate::wallet_secret::WalletSecretExt as _;

    use super::*;

    fn init_cloud_restore_test_state() {
        crate::database::test_support::delete_database();
        crate::test_support::init_test_keychain();
        crate::test_support::shared_mock_keychain().reset();
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

    // a mainnet wallet restored before #988 may hold its key in a backup's tprv form, which is the
    // same key as the xprv form a restore of that wallet now carries
    #[test]
    fn cloud_restore_adopts_a_local_xpriv_stored_in_another_network_form() {
        use bdk_wallet::bitcoin::{NetworkKind, bip32::Xpriv};

        let _guard = crate::test_support::global_state_test_lock().blocking_lock();
        init_cloud_restore_test_state();

        let metadata = cloud_restore_metadata("Legacy tprv wallet", WalletType::Hot);
        assert_eq!(NetworkKind::from(metadata.network), NetworkKind::Main);

        let local_tprv = Xpriv::new_master(NetworkKind::Test, &[8; 32]).unwrap();
        let mut incoming_xprv = local_tprv;
        incoming_xprv.network = NetworkKind::Main;

        let local_secret = KeychainWalletSecret::Xpriv(WalletXprv::try_from(local_tprv).unwrap());
        let (secret, xpub, (external, internal)) = hot_wallet_items(&metadata, local_secret);
        Keychain::global().save_wallet_secret(&metadata.id, secret).unwrap();
        Keychain::global().save_wallet_xpub(&metadata.id, xpub).unwrap();
        Keychain::global().save_public_descriptor(&metadata.id, external, internal).unwrap();
        let before = raw_keychain_entries(&metadata.id);

        restore_cloud_xpriv_wallet(&metadata, WalletXprv::try_from(incoming_xprv).unwrap())
            .expect("adopt the same key in another form");

        // the write-once entries are adopted as they are, not rewritten in the incoming form
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
        assert_eq!(after.get(WALLET_MNEMONIC_KEY_SUFFIX), before.get(WALLET_MNEMONIC_KEY_SUFFIX));
        assert_eq!(
            after.get(WALLET_MNEMONIC_CRYPTOR_KEY_SUFFIX),
            before.get(WALLET_MNEMONIC_CRYPTOR_KEY_SUFFIX)
        );
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
        assert_eq!(after.get(WALLET_XPUB_KEY_SUFFIX), before.get(WALLET_XPUB_KEY_SUFFIX));
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

    // a wallet imported from its descriptor stores the descriptor's account xpub, and keeps it
    // when its seed is added later, so after a reinstall the restore must still adopt it
    #[test]
    fn cloud_restore_adopts_the_account_xpub_of_a_wallet_imported_before_its_seed() {
        let _guard = crate::test_support::global_state_test_lock().blocking_lock();
        init_cloud_restore_test_state();

        let mut metadata = cloud_restore_metadata("Imported then upgraded", WalletType::Hot);
        metadata.network = crate::network::Network::Signet;
        let mnemonic = backup_mnemonic();
        let secret = KeychainWalletSecret::Mnemonic(mnemonic.clone());
        let descriptors = secret.clone().into_descriptors(metadata.network, metadata.address_type);
        let account_xpub = descriptors.external.xpub().expect("descriptor holds an xpub");
        assert_ne!(account_xpub, secret.xpub(metadata.network));

        let keychain = Keychain::global();
        keychain.save_wallet_secret(&metadata.id, secret).unwrap();
        keychain.save_wallet_xpub(&metadata.id, account_xpub).unwrap();
        let (external, internal) = public_descriptor_pair(&descriptors);
        keychain.save_public_descriptor(&metadata.id, external, internal).unwrap();
        let before = raw_keychain_entries(&metadata.id);

        let restored = restore_cloud_mnemonic_wallet(&metadata, mnemonic)
            .expect("adopt the account xpub of the same wallet");

        assert_eq!(restored.placement(), CloudRestorePlacement::BackupWalletId);
        assert_eq!(restored.metadata().id, metadata.id);
        assert_eq!(raw_keychain_entries(&metadata.id), before);
        assert!(restored_metadata_exists(&metadata));
    }

    #[test]
    fn cloud_restore_adopts_an_xpub_stored_in_another_network_form() {
        let _guard = crate::test_support::global_state_test_lock().blocking_lock();
        init_cloud_restore_test_state();

        let mut metadata = cloud_restore_metadata("Signet wallet with an xpub", WalletType::Hot);
        metadata.network = crate::network::Network::Signet;
        let mnemonic = backup_mnemonic();

        // the same master key in mainnet form, as a descriptor written with xpub keys stores it
        let mut mainnet_xpub =
            KeychainWalletSecret::Mnemonic(mnemonic.clone()).xpub(metadata.network);
        mainnet_xpub.network = bdk_wallet::bitcoin::NetworkKind::Main;
        Keychain::global().save_wallet_xpub(&metadata.id, mainnet_xpub).unwrap();
        let before = raw_keychain_entries(&metadata.id);

        let restored = restore_cloud_mnemonic_wallet(&metadata, mnemonic)
            .expect("adopt the same key in another network form");

        assert_eq!(restored.placement(), CloudRestorePlacement::BackupWalletId);
        let after = raw_keychain_entries(&metadata.id);
        assert_eq!(after.get(WALLET_XPUB_KEY_SUFFIX), before.get(WALLET_XPUB_KEY_SUFFIX));
        assert!(restored_metadata_exists(&metadata));
    }

    #[test]
    fn cloud_restore_moves_to_a_new_wallet_id_beside_a_different_local_seed() {
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

        let restored = restore_cloud_mnemonic_wallet(&metadata, backup_mnemonic())
            .expect("restore the backup's seed under a new wallet id");

        assert_eq!(restored.placement(), CloudRestorePlacement::NewWalletId);
        let new_id = &restored.metadata().id;
        assert_ne!(new_id, &metadata.id);
        assert_eq!(
            keychain.get_wallet_secret(new_id).unwrap(),
            Some(KeychainWalletSecret::Mnemonic(backup_mnemonic()))
        );
        assert!(restored_metadata_exists(restored.metadata()));

        // the other seed is left exactly as it was
        assert_eq!(raw_keychain_entries(&metadata.id), before);
        assert!(!restored_metadata_exists(&metadata));
        assert!(!bdk_artifacts_exist(&metadata.id));
    }

    #[test]
    fn cloud_restore_moves_to_a_new_wallet_id_beside_an_undecodable_secret() {
        let _guard = crate::test_support::global_state_test_lock().blocking_lock();
        init_cloud_restore_test_state();

        let metadata = cloud_restore_metadata("Undecodable hot wallet", WalletType::Hot);
        let secret_key = format!("{}::wallet_mnemonic", metadata.id);
        // an encrypted secret whose cryptor is missing cannot be compared to the backup
        crate::test_support::shared_mock_keychain()
            .set_entries(vec![(secret_key.as_str(), "half-written-ciphertext")]);
        let before = raw_keychain_entries(&metadata.id);

        let restored = restore_cloud_mnemonic_wallet(&metadata, backup_mnemonic())
            .expect("restore the backup's seed under a new wallet id");

        assert_eq!(restored.placement(), CloudRestorePlacement::NewWalletId);
        assert_eq!(
            Keychain::global().get_wallet_secret(&restored.metadata().id).unwrap(),
            Some(KeychainWalletSecret::Mnemonic(backup_mnemonic()))
        );
        assert_eq!(raw_keychain_entries(&metadata.id), before);
        assert!(!restored_metadata_exists(&metadata));
    }

    #[test]
    fn cloud_restore_stops_without_writing_while_the_keychain_cannot_be_read() {
        let _guard = crate::test_support::global_state_test_lock().blocking_lock();
        init_cloud_restore_test_state();

        let metadata = cloud_restore_metadata("Locked device", WalletType::Hot);
        let (_, xpub, _) =
            hot_wallet_items(&metadata, KeychainWalletSecret::Mnemonic(backup_mnemonic()));
        Keychain::global().save_wallet_xpub(&metadata.id, xpub).unwrap();
        let before = raw_keychain_entries(&metadata.id);
        let mock = crate::test_support::shared_mock_keychain();
        mock.fail_reads_for(format!("{}{WALLET_XPUB_KEY_SUFFIX}", metadata.id));

        let result = restore_cloud_mnemonic_wallet(&metadata, backup_mnemonic());
        mock.clear_read_failures();

        assert!(matches!(result, Err(CloudRestoreError::LocalKeychainUnavailable)));
        assert_eq!(raw_keychain_entries(&metadata.id), before);
        assert!(!restored_metadata_exists(&metadata));
        assert_eq!(crate::database::Database::global().wallets.all().unwrap().len(), 0);
    }

    #[test]
    fn cloud_restore_moves_a_public_backup_beside_a_local_secret() {
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

        let restored = restore_cloud_descriptor_wallet(&metadata, &backup)
            .expect("restore the public backup under a new wallet id");

        assert_eq!(restored.placement(), CloudRestorePlacement::NewWalletId);
        assert_eq!(keychain.get_wallet_xpub(&restored.metadata().id).unwrap(), Some(xpub));
        assert_eq!(raw_keychain_entries(&metadata.id), before);
        assert!(!restored_metadata_exists(&metadata));
    }

    #[test]
    fn cloud_restore_adopts_the_account_xpub_beside_a_descriptor_only_backup() {
        let _guard = crate::test_support::global_state_test_lock().blocking_lock();
        init_cloud_restore_test_state();

        let metadata = cloud_restore_metadata("Descriptor-only backup", WalletType::Cold);
        let secret = KeychainWalletSecret::Mnemonic(backup_mnemonic());
        let descriptors = secret.into_descriptors(metadata.network, metadata.address_type);
        let account_xpub = descriptors.external.xpub().expect("descriptor holds an xpub");
        Keychain::global().save_wallet_xpub(&metadata.id, account_xpub).unwrap();
        let before = raw_keychain_entries(&metadata.id);
        let backup = public_wallet_backup(
            &metadata,
            None,
            Some(public_descriptor_pair(&descriptors)),
            WalletSecret::None,
        );

        let restored = restore_cloud_descriptor_wallet(&metadata, &backup)
            .expect("adopt the account xpub of the backup's descriptors");

        assert_eq!(restored.placement(), CloudRestorePlacement::BackupWalletId);
        let after = raw_keychain_entries(&metadata.id);
        assert_eq!(after.get(WALLET_XPUB_KEY_SUFFIX), before.get(WALLET_XPUB_KEY_SUFFIX));
        assert!(restored_metadata_exists(&metadata));
    }

    #[test]
    fn cloud_restore_refuses_an_occupied_wallet_id_even_when_its_keychain_is_undecodable() {
        let _guard = crate::test_support::global_state_test_lock().blocking_lock();
        init_cloud_restore_test_state();

        let metadata = cloud_restore_metadata("Occupied and undecodable", WalletType::Hot);
        let secret_key = format!("{}::wallet_mnemonic", metadata.id);
        crate::test_support::shared_mock_keychain()
            .set_entries(vec![(secret_key.as_str(), "half-written-ciphertext")]);
        let artifact = crate::bdk_store::BdkStore::wallet_store_artifact_paths(&metadata.id)
            .into_iter()
            .find(|path| path.to_string_lossy().ends_with("-wal"))
            .expect("wallet store artifact paths include a WAL path");
        if let Some(parent) = artifact.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(&artifact, b"pre-existing WAL").unwrap();
        let wallets_before = crate::database::Database::global().wallets.all().unwrap().len();

        let result = restore_cloud_mnemonic_wallet(&metadata, backup_mnemonic());

        assert!(matches!(
            result,
            Err(CloudRestoreError::Backup(BackupError::WalletIdOccupied(id))) if id == metadata.id
        ));
        assert_eq!(
            crate::database::Database::global().wallets.all().unwrap().len(),
            wallets_before
        );
        std::fs::remove_file(artifact).unwrap();
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
