use std::time::Duration;

use bdk_wallet::KeychainKind;
use cove_types::{AddressIndex, BlockSizeLast};

use super::*;
use crate::database::cloud_backup::{
    CloudBlobConfirmedState, CloudBlobUploadedPendingConfirmationState,
};
use crate::manager::cloud_backup_manager::recovery_coverage::{
    CloudBackupRecovery, CloudBackupWalletRecovery,
};
use crate::manager::cloud_backup_manager::sync_health::SYNC_HEALTH_MISSING_MASTER_KEY_MESSAGE;
use crate::manager::cloud_backup_manager::wallets::decode_cloud_labels_jsonl;
use crate::manager::cloud_backup_manager::{
    CloudBackupRecoveryCoverage, GENERIC_CLOUD_BACKUP_ERROR_MESSAGE,
};
use crate::wallet_secret::WalletSecretExt as _;

const ABANDON_MNEMONIC: &str =
    "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about";

// BIP84 test vector for the all-"abandon" mnemonic on mainnet
const ABANDON_BIP84_FIRST_RECEIVE: &str = "bc1qcr8te4kr609gcawutmrza0j4xv80jy8z306fyu";

const LOCKED_OUTPUT_REF: &str =
    "d97bf8892657980426c879e4ab2001f09342f1ab61cfa602741a7715a3d60290:1";

/// A hot wallet whose local scan state has advanced, with a transaction label and a locked
/// output, saved as the app saves a wallet the user has been using
fn save_used_hot_wallet() -> WalletMetadata {
    let mnemonic = Mnemonic::parse(ABANDON_MNEMONIC).unwrap();
    let secret = cove_device::keychain::WalletSecret::Mnemonic(mnemonic);

    let mut metadata = WalletMetadata::preview_new();
    metadata.name = "Round Trip Savings".into();
    metadata.wallet_type = WalletType::Hot;

    let xpub = secret.xpub(metadata.network);
    let descriptors = secret.clone().into_descriptors(metadata.network, metadata.address_type);
    metadata.master_fingerprint =
        Some(Arc::new(crate::wallet::fingerprint::Fingerprint::from(xpub.fingerprint())));

    // the source device has scanned this wallet, which a restore must not trust
    metadata.internal.address_index =
        Some(AddressIndex { last_seen_index: 12, address_list_hash: 7 });

    metadata.internal.last_scan_finished = Some(Duration::from_secs(1_700_000_000));
    metadata.internal.last_height_fetched = Some(BlockSizeLast {
        block_height: 900_000,
        last_seen: Duration::from_secs(1_700_000_000),
    });

    metadata.internal.performed_full_scan_at = Some(1_700_000_000);

    let keychain = Keychain::global();
    keychain.save_wallet_xpub(&metadata.id, xpub).unwrap();
    keychain
        .save_public_descriptor(
            &metadata.id,
            descriptors.external.extended_descriptor.clone(),
            descriptors.internal.extended_descriptor,
        )
        .unwrap();

    keychain.save_wallet_secret(&metadata.id, secret).unwrap();
    Database::global()
        .wallets()
        .save_all_wallets(metadata.network, metadata.wallet_mode, vec![metadata.clone()])
        .unwrap();

    let labels = format!(
        "{}\n{{\"type\":\"output\",\"ref\":\"{LOCKED_OUTPUT_REF}\",\"spendable\":false}}",
        sample_labels_jsonl()
    );

    LabelManager::try_new(metadata.id.clone())
        .unwrap()
        .import_without_cloud_backup_dirty(&labels)
        .unwrap();

    metadata
}

/// Removes the wallet from this device the way deleting it does, leaving its cloud backup
fn delete_local_wallet(metadata: &WalletMetadata) {
    Database::global()
        .wallets
        .remove_prepared_wallet_metadata(metadata.network, metadata.wallet_mode, &metadata.id)
        .unwrap();
    assert!(Keychain::global().delete_wallet_items(&metadata.id));
    crate::wallet::delete_wallet_specific_data(&metadata.id).unwrap();
}

fn assert_labels_and_lock_survive(exported: &str) {
    assert!(exported.contains("\"label\":\"last txn received\""), "{exported}");
    assert!(exported.contains(&format!("\"ref\":\"{LOCKED_OUTPUT_REF}\"")), "{exported}");
    assert!(exported.contains("\"spendable\":false"), "{exported}");
}

#[tokio::test(flavor = "current_thread")]
async fn cloud_round_trip_restores_labels_locks_and_seed_without_scan_state() {
    let _guard = async_test_lock().lock().await;
    let globals = test_globals();
    let manager = init_manager();
    configure_enabled_cloud_backup(&manager, globals, 0);

    let wallet = save_used_hot_wallet();
    let record_id = wallet_record_id(wallet.id.as_ref());
    let namespace = CloudBackupKeychain::global().namespace_id().unwrap().unwrap();

    backup_wallets_for_test(&manager, std::slice::from_ref(&wallet)).await.unwrap();
    assert!(globals.cloud.has_wallet_backup(&namespace, &record_id));

    // the uploaded blob carries the labels and the lock, but none of the source scan state
    let master_key = cove_cspp::Cspp::new(Keychain::global().clone())
        .load_master_key_from_store()
        .unwrap()
        .unwrap();

    let encrypted = globals.cloud.wallet_backup_bytes(&namespace, &record_id).unwrap();
    let uploaded = cove_cspp::wallet_crypto::decrypt_wallet_backup(
        &serde_json::from_slice(&encrypted).unwrap(),
        &master_key.critical_data_key(),
    )
    .unwrap();

    let uploaded_metadata: WalletMetadata =
        serde_json::from_value(uploaded.metadata.clone()).unwrap();

    assert_eq!(uploaded_metadata.name, wallet.name);
    assert_eq!(uploaded_metadata.internal.address_index, None);
    assert_eq!(uploaded_metadata.internal.last_scan_finished, None);
    assert_eq!(uploaded_metadata.internal.last_height_fetched, None);
    assert_eq!(uploaded_metadata.internal.performed_full_scan_at, None);

    let uploaded_labels =
        decode_cloud_labels_jsonl(&uploaded).unwrap().expect("uploaded backup should carry labels");

    assert_labels_and_lock_survive(&uploaded_labels);

    // deleting the wallet here leaves its cloud copy restorable
    delete_local_wallet(&wallet);
    manager.handle_local_wallets_changed();
    manager.do_upload_wallet_if_dirty(&wallet.id).await.unwrap();
    assert!(globals.cloud.has_wallet_backup(&namespace, &record_id));

    let outcome = manager
        .do_restore_cloud_wallet(
            &unowned_writes(&manager, CloudBackupExclusiveOperation::RestoreCloudWallet),
            &record_id,
            permit(PermitIntent::RestoreCloudWallet),
        )
        .await
        .unwrap();
    assert!(matches!(outcome, CloudOnlyRestoreOutcome::Restored { labels_warning: None }));

    let restored = Database::global()
        .wallets()
        .get(&wallet.id, wallet.network, wallet.wallet_mode)
        .unwrap()
        .expect("restored wallet should be saved");

    assert_eq!(restored.name, wallet.name);
    assert_eq!(restored.wallet_type, WalletType::Hot);
    assert_eq!(
        Keychain::global()
            .get_wallet_key(&restored.id)
            .unwrap()
            .map(|mnemonic| mnemonic.to_string()),
        Some(ABANDON_MNEMONIC.to_string())
    );

    // no scan checkpoint from the source device, so the first scan is a full scan
    assert_eq!(restored.internal.address_index, None);
    assert_eq!(restored.internal.last_scan_finished, None);
    assert_eq!(restored.internal.last_height_fetched, None);
    assert_eq!(restored.internal.performed_full_scan_at, None);

    // the restored wallet derives the seed's addresses from an empty chain
    let mut store = crate::bdk_store::BdkStore::try_new(&restored.id, restored.network).unwrap();
    let bdk = bdk_wallet::Wallet::load().load_wallet(&mut store.conn).unwrap().unwrap();
    assert_eq!(
        bdk.peek_address(KeychainKind::External, 0).address.to_string(),
        ABANDON_BIP84_FIRST_RECEIVE
    );

    assert_eq!(bdk.latest_checkpoint().height(), 0);
    assert_eq!(bdk.transactions().count(), 0);

    let exported = LabelManager::try_new(wallet.id.clone()).unwrap().export().await.unwrap();
    assert_labels_and_lock_survive(&exported);
}

#[tokio::test(flavor = "current_thread")]
async fn a_wallet_restored_from_cloud_backup_counts_as_backed_up() {
    let _guard = async_test_lock().lock().await;
    let globals = test_globals();
    let manager = init_manager();
    configure_enabled_cloud_backup(&manager, globals, 0);

    let mut wallet = save_used_hot_wallet();
    wallet.verified = false;
    let record_id = wallet_record_id(wallet.id.as_ref());

    backup_wallets_for_test(&manager, std::slice::from_ref(&wallet)).await.unwrap();

    // a device that never uploaded the wallet, such as a new phone, holds no row for it
    delete_local_wallet(&wallet);
    Database::global().cloud_blob_sync_states.delete(&record_id).unwrap();
    manager.handle_local_wallets_changed();

    let outcome = manager
        .do_restore_cloud_wallet(
            &unowned_writes(&manager, CloudBackupExclusiveOperation::RestoreCloudWallet),
            &record_id,
            permit(PermitIntent::RestoreCloudWallet),
        )
        .await
        .unwrap();
    assert!(matches!(outcome, CloudOnlyRestoreOutcome::Restored { .. }));

    let restored = Database::global()
        .wallets()
        .get(&wallet.id, wallet.network, wallet.wallet_mode)
        .unwrap()
        .expect("restored wallet should be saved");

    assert_eq!(restored.wallet_type, WalletType::Hot);
    assert!(!restored.verified);
    assert!(
        !CloudBackupRecoveryCoverage::load().needs_backup(&restored),
        "a wallet restored from the active backup must not block the Wipe Data PIN"
    );
}

/// Refreshes the detail from the provider and waits for the supervisor to apply what it read
async fn refresh_detail_from_provider(manager: &RustCloudBackupManager) {
    let refreshed = manager.refresh_cloud_backup_detail(current_config(manager).await).await;
    assert!(
        matches!(refreshed, Some(CloudBackupDetailResult::SuccessWithAuthority { .. })),
        "expected an authoritative refresh, got {refreshed:?}"
    );

    supervisor_checkpoint(manager).await;
}

fn wallet_recovery(wallet: &WalletMetadata) -> CloudBackupWalletRecovery {
    CloudBackupRecovery::load().wallet(wallet.id.clone())
}

/// A wallet whose backup a refresh read and confirmed, with its namespace and record id
async fn confirmed_backup(
    manager: &RustCloudBackupManager,
    globals: &TestGlobals,
) -> (WalletMetadata, String, String) {
    configure_enabled_cloud_backup(manager, globals, 0);
    globals.cloud.set_reflect_uploaded_wallets_in_listing(true);

    let mut wallet = save_used_hot_wallet();
    wallet.verified = false;
    let record_id = wallet_record_id(wallet.id.as_ref());

    backup_wallets_for_test(manager, std::slice::from_ref(&wallet)).await.unwrap();
    refresh_detail_from_provider(manager).await;
    assert_eq!(wallet_recovery(&wallet), CloudBackupWalletRecovery::Confirmed);

    let namespace = CloudBackupKeychain::global().namespace_id().unwrap().unwrap();
    (wallet, namespace, record_id)
}

/// Waits briefly for the sync health the manager shows to satisfy `accept`, and returns the last
/// one it showed
async fn shown_sync_health(
    manager: &RustCloudBackupManager,
    accept: impl Fn(&CloudSyncHealth) -> bool,
) -> CloudSyncHealth {
    let deadline = LoadedHostDeadline::after(Duration::from_millis(500));
    loop {
        let shown = manager.model_snapshot().sync_health;
        if accept(&shown) || deadline.has_passed() {
            return shown;
        }

        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

/// Replaces the state of `record_id`'s row, which must exist
fn rewrite_row_state(
    record_id: &str,
    state: impl FnOnce(&PersistedCloudBlobState) -> PersistedCloudBlobState,
) {
    let table = &Database::global().cloud_blob_sync_states;
    let row = table.get(record_id).unwrap().expect("the wallet should have a row");
    let rewritten = row.with_state(state(&row.state));
    assert!(table.set_if_current(&row, &rewritten).unwrap());
}

#[tokio::test(flavor = "current_thread")]
async fn a_backup_another_device_deleted_no_longer_covers_the_wallet() {
    let _guard = async_test_lock().lock().await;
    let globals = test_globals();
    let manager = init_manager();
    let (wallet, namespace, record_id) = confirmed_backup(&manager, globals).await;

    let upload = globals.cloud.gate_next_wallet_upload();
    globals.cloud.remove_wallet_backup(&namespace, &record_id);
    refresh_detail_from_provider(&manager).await;

    assert_eq!(
        wallet_recovery(&wallet),
        CloudBackupWalletRecovery::Unconfirmed,
        "the provider no longer holds the backup, so deleting the wallet would lose its keys"
    );
    assert!(CloudBackupRecoveryCoverage::load().needs_backup(&wallet));

    let uploading = tokio::time::timeout(Duration::from_secs(2), upload.wait_until_blocked()).await;
    assert!(uploading.is_ok(), "the refresh should upload the deleted backup again");

    upload.release();
    wait_for_test_condition(Duration::from_secs(2), "the backup should be uploaded again", || {
        globals.cloud.wallet_backup_bytes(&namespace, &record_id).is_some()
    })
    .await;
    clear_wallet_upload_runtime_for_test_async(&manager).await;
}

#[tokio::test(flavor = "current_thread")]
async fn a_refresh_that_confirms_the_last_pending_upload_shows_every_wallet_uploaded() {
    let _guard = async_test_lock().lock().await;
    let globals = test_globals();
    let manager = init_manager();
    let (_, namespace, record_id) = confirmed_backup(&manager, globals).await;
    globals.cloud.set_master_key_backup(namespace, vec![1]);

    // the upload landed, but nothing has confirmed it yet
    rewrite_row_state(&record_id, |state| {
        let PersistedCloudBlobState::Confirmed(confirmed) = state else {
            panic!("the refresh should have confirmed the backup, got {state:?}");
        };

        PersistedCloudBlobState::UploadedPendingConfirmation(
            CloudBlobUploadedPendingConfirmationState {
                revision_hash: confirmed.revision_hash.clone(),
                uploaded_at: confirmed.confirmed_at,
                attempt_count: 0,
                last_checked_at: None,
            },
        )
    });

    manager.refresh_sync_health();
    let uploading =
        shown_sync_health(&manager, |health| *health == CloudSyncHealth::Uploading).await;
    assert_eq!(uploading, CloudSyncHealth::Uploading, "Settings should show the pending upload");

    refresh_detail_from_provider(&manager).await;

    let uploaded =
        shown_sync_health(&manager, |health| *health == CloudSyncHealth::AllUploaded).await;
    assert_eq!(
        uploaded,
        CloudSyncHealth::AllUploaded,
        "the refresh read the uploaded revision, so Settings must stop showing it syncing"
    );
    clear_wallet_upload_runtime_for_test_async(&manager).await;
}

#[tokio::test(flavor = "current_thread")]
async fn a_wallet_restored_before_restores_recorded_their_backup_is_covered_once_the_provider_shows_it()
 {
    let _guard = async_test_lock().lock().await;
    let globals = test_globals();
    let manager = init_manager();
    configure_enabled_cloud_backup(&manager, globals, 0);
    globals.cloud.set_reflect_uploaded_wallets_in_listing(true);

    let mut wallet = save_used_hot_wallet();
    wallet.verified = false;
    let record_id = wallet_record_id(wallet.id.as_ref());

    backup_wallets_for_test(&manager, std::slice::from_ref(&wallet)).await.unwrap();

    // a restore from before restores recorded their backup left the wallet with no row
    Database::global().cloud_blob_sync_states.delete(&record_id).unwrap();
    assert_eq!(wallet_recovery(&wallet), CloudBackupWalletRecovery::Unconfirmed);

    refresh_detail_from_provider(&manager).await;

    assert_eq!(
        wallet_recovery(&wallet),
        CloudBackupWalletRecovery::Confirmed,
        "the provider holds this exact backup, so the wallet can be recovered from it"
    );
    assert!(!CloudBackupRecoveryCoverage::load().needs_backup(&wallet));
    clear_wallet_upload_runtime_for_test_async(&manager).await;
}

#[tokio::test(flavor = "current_thread")]
async fn a_backup_icloud_stopped_listing_no_longer_covers_the_wallet() {
    let _guard = async_test_lock().lock().await;
    let globals = test_globals();
    let manager = init_manager();
    let (wallet, namespace, record_id) = confirmed_backup(&manager, globals).await;

    // iCloud drops a copy another device deleted from its listing, and a read of it waits for the
    // copy until it times out
    globals.cloud.remove_wallet_backup(&namespace, &record_id);
    globals.cloud.set_reflect_uploaded_wallets_in_listing(false);
    globals.cloud.fail_wallet_backup_download(
        namespace,
        record_id,
        CloudStorageError::SyncPending("iCloud metadata is still loading".into()),
    );
    refresh_detail_from_provider(&manager).await;

    assert_eq!(
        wallet_recovery(&wallet),
        CloudBackupWalletRecovery::Unconfirmed,
        "the provider no longer lists the backup, so deleting the wallet would lose its keys"
    );
    assert!(CloudBackupRecoveryCoverage::load().needs_backup(&wallet));
    clear_wallet_upload_runtime_for_test_async(&manager).await;
}

#[tokio::test(flavor = "current_thread")]
async fn a_deleted_backup_stops_covering_a_wallet_whose_confirmed_hash_it_no_longer_reproduces() {
    let _guard = async_test_lock().lock().await;
    let globals = test_globals();
    let manager = init_manager();
    let (wallet, namespace, record_id) = confirmed_backup(&manager, globals).await;

    // a restore of a backup written before #988 records the hash of the key's other form
    rewrite_row_state(&record_id, |state| {
        let PersistedCloudBlobState::Confirmed(confirmed) = state else {
            panic!("the refresh should have confirmed the backup, got {state:?}");
        };

        PersistedCloudBlobState::Confirmed(CloudBlobConfirmedState {
            revision_hash: "pre-988-revision".into(),
            confirmed_at: confirmed.confirmed_at,
        })
    });

    globals.cloud.remove_wallet_backup(&namespace, &record_id);
    refresh_detail_from_provider(&manager).await;

    assert_eq!(
        wallet_recovery(&wallet),
        CloudBackupWalletRecovery::Unconfirmed,
        "the provider no longer holds the backup, whatever hash the restore recorded"
    );
    assert!(CloudBackupRecoveryCoverage::load().needs_backup(&wallet));
    clear_wallet_upload_runtime_for_test_async(&manager).await;
}

#[tokio::test(flavor = "current_thread")]
async fn a_deleted_backup_another_device_uploads_again_covers_the_wallet_again() {
    let _guard = async_test_lock().lock().await;
    let globals = test_globals();
    let manager = init_manager();
    let (wallet, namespace, record_id) = confirmed_backup(&manager, globals).await;
    let backup = globals.cloud.wallet_backup_bytes(&namespace, &record_id).unwrap();
    let attempts = globals.cloud.wallet_backup_upload_attempt_count();

    // this device's own upload of the deleted backup can't reach the provider yet
    globals.cloud.fail_next_wallet_backup_upload_offline("offline");
    globals.cloud.remove_wallet_backup(&namespace, &record_id);
    refresh_detail_from_provider(&manager).await;
    assert_eq!(wallet_recovery(&wallet), CloudBackupWalletRecovery::Unconfirmed);

    wait_for_test_condition(Duration::from_secs(2), "the re-upload should fail offline", || {
        let row = Database::global().cloud_blob_sync_states.get(&record_id).unwrap();
        globals.cloud.wallet_backup_upload_attempt_count() > attempts
            && matches!(row.map(|row| row.state), Some(PersistedCloudBlobState::Dirty(_)))
    })
    .await;

    // a while later another device uploads the same revision again
    rewrite_row_state(&record_id, |state| {
        let PersistedCloudBlobState::Dirty(dirty) = state else {
            panic!("the refresh should have marked the backup dirty, got {state:?}");
        };

        PersistedCloudBlobState::Dirty(CloudBlobDirtyState { changed_at: dirty.changed_at - 60 })
    });
    globals.cloud.set_wallet_backup(namespace, record_id, backup);
    refresh_detail_from_provider(&manager).await;

    assert_eq!(
        wallet_recovery(&wallet),
        CloudBackupWalletRecovery::Confirmed,
        "the provider holds this exact backup again, so the wallet can be recovered from it"
    );
    assert!(!CloudBackupRecoveryCoverage::load().needs_backup(&wallet));
    clear_wallet_upload_runtime_for_test_async(&manager).await;
}

#[tokio::test(flavor = "current_thread")]
async fn a_failed_upload_whose_revision_another_device_uploaded_covers_the_wallet() {
    let _guard = async_test_lock().lock().await;
    let globals = test_globals();
    let manager = init_manager();
    let (wallet, _, record_id) = confirmed_backup(&manager, globals).await;

    // this device's upload failed for good a while ago, and another device has since uploaded the
    // same revision
    rewrite_row_state(&record_id, |state| {
        let PersistedCloudBlobState::Confirmed(confirmed) = state else {
            panic!("the refresh should have confirmed the backup, got {state:?}");
        };

        PersistedCloudBlobState::Failed(CloudBlobFailedState {
            revision_hash: Some(confirmed.revision_hash.clone()),
            retryable: false,
            error: "storage quota exceeded".into(),
            issue: None,
            failed_at: confirmed.confirmed_at - 60,
        })
    });
    assert_eq!(wallet_recovery(&wallet), CloudBackupWalletRecovery::Unconfirmed);

    let upload_failed = CloudSyncHealth::Failed(GENERIC_CLOUD_BACKUP_ERROR_MESSAGE.into());
    manager.refresh_sync_health();
    let failing = shown_sync_health(&manager, |health| *health == upload_failed).await;
    assert_eq!(failing, upload_failed, "Settings should show the failed upload");

    refresh_detail_from_provider(&manager).await;

    assert_eq!(
        wallet_recovery(&wallet),
        CloudBackupWalletRecovery::Confirmed,
        "the provider holds this exact backup, so the wallet can be recovered from it"
    );
    assert!(!CloudBackupRecoveryCoverage::load().needs_backup(&wallet));

    // the mock holds no master key, so once the failed upload is gone that is what Settings shows
    let missing_key = CloudSyncHealth::Failed(SYNC_HEALTH_MISSING_MASTER_KEY_MESSAGE.into());
    let healed = shown_sync_health(&manager, |health| *health == missing_key).await;
    assert_eq!(
        healed, missing_key,
        "the confirmed wallet must not keep the failed upload's error in Settings"
    );
    clear_wallet_upload_runtime_for_test_async(&manager).await;
}
