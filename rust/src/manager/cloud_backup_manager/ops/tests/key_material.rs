//! A cloud upload never replaces a record holding key material with one that holds none
//!
//! A hot wallet missing its key opens as watch-only, and a TAPSIGNER wallet missing its backup
//! uploads as watch-only. Their cloud records may be the last copy of that key, so every path that
//! uploads them must leave the record as it is and never report the wallet Confirmed

use std::sync::Arc;

use bitcoin::secp256k1::PublicKey;
use cove_tap_card::{TapSigner, TapSignerState};

use super::*;
use crate::manager::cloud_backup_manager::cloud_inventory::CloudWalletInventory;
use crate::manager::cloud_backup_manager::ops::test_support::MockMissingRecordLookup;
use crate::manager::connectivity_manager::ConnectivityStatus;
use crate::wallet::metadata::HardwareWalletMetadata;

const MNEMONIC: &str =
    "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about";

/// A local wallet that uploads as watch-only, and the secret its cloud record still holds
struct LostSecret {
    metadata: WalletMetadata,
    // a constructor, since a secret zeroizes on drop and can't be cloned
    cloud_secret: fn() -> WalletSecret,
}

impl LostSecret {
    /// A hot wallet whose key is gone, as the wallet manager leaves it once it opens
    fn downgraded_hot_wallet() -> Self {
        let mut metadata = WalletMetadata::preview_new();
        metadata.wallet_type = WalletType::WatchOnly;
        metadata.hardware_metadata = None;
        Keychain::global().delete_wallet_secret(&metadata.id);

        Self { metadata, cloud_secret: || WalletSecret::Mnemonic(MNEMONIC.to_string()) }
    }

    /// A TAPSIGNER wallet whose backup is missing from the keychain
    fn tap_signer_without_backup() -> Self {
        // secp256k1 generator point G, a valid compressed public key
        let pubkey = PublicKey::from_slice(&[
            0x02, 0x79, 0xBE, 0x66, 0x7E, 0xF9, 0xDC, 0xBB, 0xAC, 0x55, 0xA0, 0x62, 0x95, 0xCE,
            0x87, 0x0B, 0x07, 0x02, 0x9B, 0xFC, 0xDB, 0x2D, 0xCE, 0x28, 0xD9, 0x59, 0xF2, 0x81,
            0x5B, 0x16, 0xF8, 0x17, 0x98,
        ])
        .unwrap();
        let tap_signer = Arc::new(TapSigner {
            state: TapSignerState::Sealed,
            card_ident: "TEST-IDENT".to_string(),
            nonce: "deadbeef".to_string(),
            signature: "cafebabe".to_string(),
            pubkey: Arc::new(pubkey),
        });

        let mut metadata = WalletMetadata::preview_new();
        metadata.wallet_type = WalletType::Cold;
        metadata.hardware_metadata = Some(HardwareWalletMetadata::TapSigner(tap_signer));
        Keychain::global().delete_tap_signer_backup(&metadata.id);

        Self { metadata, cloud_secret: || WalletSecret::TapSignerBackup(vec![7; 32]) }
    }

    /// Each case's constructor, called once the test's globals are up since it edits the keychain
    fn cases() -> [fn() -> Self; 2] {
        [Self::downgraded_hot_wallet, Self::tap_signer_without_backup]
    }

    fn record_id(&self) -> String {
        wallet_record_id(self.metadata.id.as_ref())
    }

    /// Saves the local wallet, and puts its key-bearing record in the cloud, returning the bytes
    fn install(&self, globals: &TestGlobals) -> Vec<u8> {
        save_local_wallet(&self.metadata);

        let mut entry = wallet_entry_with_labels(&self.metadata, None);
        entry.secret = (self.cloud_secret)();
        entry.content_revision_hash = "uploaded-with-its-key".into();

        put_cloud_record(globals, &self.record_id(), &entry)
    }
}

fn save_local_wallet(metadata: &WalletMetadata) {
    Database::global()
        .wallets()
        .save_all_wallets(metadata.network, metadata.wallet_mode, vec![metadata.clone()])
        .unwrap();
}

fn put_cloud_record(globals: &TestGlobals, record_id: &str, entry: &WalletEntry) -> Vec<u8> {
    let master_key = cove_cspp::Cspp::new(Keychain::global().clone())
        .load_master_key_from_store()
        .unwrap()
        .unwrap();
    let bytes = encrypted_wallet_backup_bytes_for_entry(entry, &master_key, 1);
    let namespace = CloudBackupKeychain::global().namespace_id().unwrap().unwrap();
    globals.cloud.set_wallet_backup(namespace, record_id.to_owned(), bytes.clone());

    bytes
}

fn cloud_record(globals: &TestGlobals, record_id: &str) -> Option<Vec<u8>> {
    let namespace = CloudBackupKeychain::global().namespace_id().unwrap().unwrap();
    globals.cloud.wallet_backup_bytes(&namespace, record_id)
}

/// The status a fresh detail read reports for `record_id`'s wallet
async fn wallet_status(
    manager: &RustCloudBackupManager,
    record_id: &str,
) -> CloudBackupWalletStatus {
    let namespace = manager.current_namespace_id().unwrap();
    let record_ids = [record_id.to_owned()];
    let remote_truth = manager
        .load_remote_wallet_truth(&namespace, &record_ids, CloudStorage::global_silent_client())
        .await
        .unwrap();
    let detail = CloudWalletInventory::load_with_remote_truth(&record_ids, remote_truth)
        .await
        .unwrap()
        .build_detail()
        .detail;

    detail
        .up_to_date
        .iter()
        .chain(&detail.needs_sync)
        .find(|wallet| wallet.record_id == record_id)
        .map(|wallet| wallet.sync_status.clone())
        .expect("the wallet has a row")
}

fn blob_state(record_id: &str) -> Option<PersistedCloudBlobState> {
    Database::global().cloud_blob_sync_states.get(record_id).unwrap().map(|state| state.state)
}

fn save_test_passkey() {
    let namespace = CloudBackupKeychain::global().namespace_id().unwrap().unwrap();
    CloudBackupKeychain::global()
        .save_passkey_and_namespace(&[1, 2, 3, 4], [9; 32], &namespace)
        .unwrap();
}

/// The iOS launch check auto-syncs wallets whose cloud copy differs, and a wallet that lost its
/// key differs from its key-bearing copy, so it is the first upload the lane refuses
#[tokio::test(flavor = "current_thread")]
async fn startup_auto_sync_keeps_a_cloud_key_this_device_lost() {
    for case in LostSecret::cases() {
        let _guard = async_test_lock().lock().await;
        let globals = test_globals();
        let manager = init_manager();
        configure_enabled_cloud_backup(&manager, globals, 1);
        save_test_passkey();
        let lost = case();

        let record_id = lost.record_id();
        let before = lost.install(globals);

        let warning = manager.verify_backup_integrity_impl().await;

        assert_eq!(warning, None, "{:?}", lost.metadata.wallet_type);
        assert_eq!(cloud_record(globals, &record_id), Some(before));
        assert!(
            !matches!(
                blob_state(&record_id),
                Some(
                    PersistedCloudBlobState::Confirmed(_)
                        | PersistedCloudBlobState::UploadedPendingConfirmation(_)
                )
            ),
            "{:?}",
            blob_state(&record_id)
        );

        let shown = manager
            .model_snapshot()
            .detail
            .and_then(|detail| {
                detail
                    .up_to_date
                    .into_iter()
                    .chain(detail.needs_sync)
                    .find(|wallet| wallet.record_id == record_id)
            })
            .map(|wallet| wallet.sync_status);
        assert_eq!(shown, Some(CloudBackupWalletStatus::SecretMissingLocally));
        assert_eq!(
            wallet_status(&manager, &record_id).await,
            CloudBackupWalletStatus::SecretMissingLocally
        );
    }
}

/// A rename marks the wallet dirty through the ordinary metadata update, and its upload keeps
/// the cloud copy
#[tokio::test(flavor = "current_thread")]
async fn a_rename_keeps_a_cloud_key_this_device_lost() {
    for case in LostSecret::cases() {
        let _guard = async_test_lock().lock().await;
        let globals = test_globals();
        let manager = init_manager();
        configure_enabled_cloud_backup(&manager, globals, 1);
        let lost = case();

        let record_id = lost.record_id();
        let before = lost.install(globals);

        let mut renamed = lost.metadata.clone();
        renamed.name = "Renamed".to_string();
        save_local_wallet(&renamed);

        // offline keeps the dirty mark in place, and flipping the status back without a
        // broadcast schedules nothing, so the upload below is the only one
        CONNECTIVITY_MANAGER.set_status_without_broadcast(ConnectivityStatus::Disconnected);
        manager.handle_wallet_metadata_update(&lost.metadata, &renamed);
        CONNECTIVITY_MANAGER.set_status_without_broadcast(ConnectivityStatus::Connected);
        assert!(matches!(blob_state(&record_id), Some(PersistedCloudBlobState::Dirty(_))));

        manager.do_upload_wallet_if_dirty(&renamed.id).await.unwrap();

        assert_eq!(cloud_record(globals, &record_id), Some(before));
        assert!(
            matches!(
                blob_state(&record_id),
                Some(PersistedCloudBlobState::Failed(CloudBlobFailedState {
                    retryable: false,
                    ..
                }))
            ),
            "{:?}",
            blob_state(&record_id)
        );
        assert_eq!(
            wallet_status(&manager, &record_id).await,
            CloudBackupWalletStatus::SecretMissingLocally
        );
    }
}

/// The dirty-upload queue keeps the cloud copy, and records a failure it won't retry on its own
#[tokio::test(flavor = "current_thread")]
async fn a_dirty_upload_keeps_a_cloud_key_this_device_lost() {
    for case in LostSecret::cases() {
        let _guard = async_test_lock().lock().await;
        let globals = test_globals();
        let manager = init_manager();
        configure_enabled_cloud_backup(&manager, globals, 1);
        let lost = case();

        let record_id = lost.record_id();
        let before = lost.install(globals);
        persist_dirty_blob_state(lost.metadata.id.clone());

        manager.do_upload_wallet_if_dirty(&lost.metadata.id).await.unwrap();

        assert_eq!(cloud_record(globals, &record_id), Some(before));
        assert!(
            matches!(
                blob_state(&record_id),
                Some(PersistedCloudBlobState::Failed(CloudBlobFailedState {
                    retryable: false,
                    ..
                }))
            ),
            "{:?}",
            blob_state(&record_id)
        );
        assert_eq!(
            wallet_status(&manager, &record_id).await,
            CloudBackupWalletStatus::SecretMissingLocally
        );
    }
}

/// A watch-only wallet whose cloud copy is watch-only too loses nothing, so it still uploads and
/// confirms
#[tokio::test(flavor = "current_thread")]
async fn a_watch_only_wallet_still_replaces_its_watch_only_copy() {
    let _guard = async_test_lock().lock().await;
    let globals = test_globals();
    let manager = init_manager();
    configure_enabled_cloud_backup(&manager, globals, 1);
    save_test_passkey();

    let mut metadata = WalletMetadata::preview_new();
    metadata.wallet_type = WalletType::WatchOnly;
    save_local_wallet(&metadata);

    let record_id = wallet_record_id(metadata.id.as_ref());
    let mut entry = wallet_entry_with_labels(&metadata, None);
    entry.content_revision_hash = "an-older-revision".into();
    let before = put_cloud_record(globals, &record_id, &entry);

    let warning = manager.verify_backup_integrity_impl().await;

    assert_eq!(warning, None);
    assert_eq!(globals.cloud.wallet_backup_upload_attempt_count(), 1);
    assert_ne!(cloud_record(globals, &record_id), Some(before));

    // the integrity check may confirm the upload itself, and the pending pass confirms it if not
    verify_pending_uploads_once_for_test_async(&manager).await;

    assert!(matches!(blob_state(&record_id), Some(PersistedCloudBlobState::Confirmed(_))));
    assert_eq!(wallet_status(&manager, &record_id).await, CloudBackupWalletStatus::Confirmed);
}

/// The watch-only record the cloud holds for `metadata`, opened with this device's master key
fn watch_only_cloud_entry(globals: &TestGlobals, metadata: &WalletMetadata) -> WalletEntry {
    let record_id = wallet_record_id(metadata.id.as_ref());
    let bytes = cloud_record(globals, &record_id).expect("the wallet's record is in the cloud");
    let master_key = cove_cspp::Cspp::new(Keychain::global().clone())
        .load_master_key_from_store()
        .unwrap()
        .unwrap();

    let entry = cove_cspp::wallet_crypto::decrypt_wallet_backup(
        &serde_json::from_slice(&bytes).unwrap(),
        &master_key.critical_data_key(),
    )
    .unwrap();

    assert!(matches!(entry.secret, WalletSecret::WatchOnly), "the record is watch-only");
    entry
}

/// On iOS a consent-allowed read of a record the cloud doesn't hold ends in `SyncPending`, never
/// `NotFound`, so Enable must learn that a watch-only wallet's first record is absent without
/// reading it
#[tokio::test(flavor = "current_thread")]
async fn enable_uploads_a_watch_only_wallet_the_cloud_never_held() {
    let _guard = async_test_lock().lock().await;
    let globals = test_globals();
    let manager = init_manager();

    reset_cloud_backup_test_state(&manager, globals);
    CONNECTIVITY_MANAGER.set_connection_state(true);
    globals.cloud.set_reflect_uploaded_wallets_in_listing(true);
    globals.cloud.set_explicit_missing_record_lookup(MockMissingRecordLookup::SyncPending);
    let metadata = xpub_only_wallet_metadata();
    persist_xpub_wallets(vec![metadata.clone()]);
    globals.passkey.set_create_result(Ok(vec![9, 8, 7]));
    globals.passkey.set_authenticate_result(Ok(vec![0x5a; 32]));

    enable_cloud_backup_create_new(&manager).await.unwrap();
    confirm_saved_passkey_session(&manager).await;

    assert_eq!(manager.current_status(), CloudBackupStatus::Enabled);
    let entry = watch_only_cloud_entry(globals, &metadata);
    assert_eq!(entry.wallet_id, metadata.id.to_string());

    reset_cloud_backup_test_state(&manager, globals);
}

/// Sync and the integrity check's fallback back up unsynced wallets as a batch on the
/// consent-allowed client. On iOS a watch-only wallet's first record still uploads there, and a
/// record holding a key this device lost is still read, so it is kept
#[tokio::test(flavor = "current_thread")]
async fn a_batch_backup_on_ios_uploads_a_first_record_and_keeps_a_lost_key() {
    for case in LostSecret::cases() {
        let _guard = async_test_lock().lock().await;
        let globals = test_globals();
        let manager = init_manager();
        configure_enabled_cloud_backup(&manager, globals, 1);
        globals.cloud.set_explicit_missing_record_lookup(MockMissingRecordLookup::SyncPending);

        let mut first = WalletMetadata::preview_new();
        first.wallet_type = WalletType::WatchOnly;
        save_local_wallet(&first);
        let lost = case();
        let before = lost.install(globals);

        backup_wallets_for_test(&manager, &[first.clone(), lost.metadata.clone()]).await.unwrap();

        let entry = watch_only_cloud_entry(globals, &first);
        assert_eq!(entry.wallet_id, first.id.to_string());
        assert_eq!(cloud_record(globals, &lost.record_id()), Some(before));
    }
}
