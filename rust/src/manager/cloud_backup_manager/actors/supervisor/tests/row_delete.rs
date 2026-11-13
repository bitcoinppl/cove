//! Row deletes: what each row may delete, and what a confirmed delete does once its row changes

use std::collections::BTreeMap;

use super::*;
use crate::manager::cloud_backup_manager::CloudBackupWalletVerificationIssues;
use crate::manager::cloud_backup_manager::actors::RowDeleteReport;
use crate::manager::cloud_backup_manager::delete_consent::CloudBackupDeleteCategory;
use crate::manager::cloud_backup_manager::ops::BatchStop;
use crate::manager::cloud_backup_manager::ops::test_support::{
    begin_operation_writes_for_test, encrypted_wallet_backup_bytes, finish_operation_for_test,
    persist_xpub_wallets,
};
use crate::manager::cloud_backup_manager::owner::delete_authorization::{
    DeleteAuthorization, StaleTarget,
};
use crate::manager::cloud_backup_manager::owner::inventory::{InventoryInput, RecordRead};
use crate::manager::cloud_backup_manager::screen::{
    CloudBackupActionAvailability, CloudBackupDeleteConfirmation, CloudBackupRowControl,
    CloudBackupScreenAction, CloudBackupSection, CloudBackupVerificationControl,
};
use cove_device::cloud_storage::CloudStorageError;

/// A listed backup of a wallet this device doesn't have
struct NotOnDeviceRow {
    namespace: String,
    wallet: WalletMetadata,
    record_id: String,
}

/// Lists one readable backup of a wallet that isn't on this device and loads it as a row
async fn load_not_on_device_row(manager: &RustCloudBackupManager) -> NotOnDeviceRow {
    let namespace = manager.current_namespace_id().unwrap();

    // xpub-only test wallets share one xpub, so another network keeps it apart from local ones
    let mut wallet = xpub_only_wallet_metadata();
    wallet.network = Network::Testnet;
    let record_id = upload_test_wallet_backup(&namespace, &wallet);
    load_rows(manager, &namespace, &[record_id.as_str()]).await;

    NotOnDeviceRow { namespace, wallet, record_id }
}

/// Lists `record_ids` as the namespace's backups and waits for the rows a fetch reads from them,
/// readable or not
async fn load_rows(manager: &RustCloudBackupManager, namespace: &str, record_ids: &[&str]) {
    list_test_wallet_files(namespace, record_ids);
    manager.apply_detail_outcome(
        manager.inventory_scope(),
        CloudBackupDetailOutcome::refreshed(listed_detail(record_ids)),
    );
    call!(manager.supervisor.ensure_cloud_only_fetch()).await.unwrap();

    let loaded = || {
        let cloud_only = manager.inventory.read().cloud_only();
        let Some(rows) = cloud_only.loaded_rows() else { return false };
        record_ids.iter().all(|id| rows.iter().any(|row| row.record_id == *id))
    };

    wait_for_test_condition(Duration::from_secs(5), "cloud-only rows should be loaded", loaded)
        .await;
}

/// The controls the screen draws on the row for `record_id`, or `None` when it draws no such row
async fn row_controls(
    manager: &RustCloudBackupManager,
    record_id: &str,
) -> Option<Vec<CloudBackupRowControl>> {
    drawn_rows(manager)
        .await
        .into_iter()
        .find(|row| row.record_id.as_str() == record_id)
        .map(|row| row.actions.iter().map(|action| action.action.control()).collect())
}

/// What the row for `record_id` offers to delete its record as now
async fn offered_delete(
    manager: &RustCloudBackupManager,
    record_id: &str,
) -> Option<CloudBackupDeleteCategory> {
    row_delete_consent(manager, record_id).await.map(|consent| consent.evidence().category())
}

/// Whether `consent` holds against the inventory as dispatch checks it now
async fn holds(
    manager: &RustCloudBackupManager,
    consent: &CloudBackupRowDeleteConsent,
) -> Result<Arc<DeleteAuthorization>, StaleTarget> {
    call!(manager.supervisor.row_delete_holds_for_test(consent.clone())).await.unwrap()
}

/// The authorization the Delete the row for `record_id` offers now gives
async fn authorize(manager: &RustCloudBackupManager, record_id: &str) -> Arc<DeleteAuthorization> {
    let consent = row_delete_consent(manager, record_id).await.expect("the row offers Delete");
    holds(manager, &consent).await.expect("the consent the screen drew holds")
}

/// Starts the delete the row for `record_id` offers, as a confirmed tap does
async fn start_row_delete(manager: &RustCloudBackupManager, record_id: &str) {
    let consent = row_delete_consent(manager, record_id).await.expect("the row offers Delete");
    call!(manager.supervisor.start_delete_cloud_wallet_operation(consent)).await.unwrap();
}

/// The batch the verification section's Delete Unreadable Backups offers now
async fn batch_consent(manager: &RustCloudBackupManager) -> Option<CloudBackupBatchDeleteConsent> {
    let screen = call!(manager.supervisor.projected_screen_for_test()).await.unwrap();
    let CloudBackupScreenBody::Configured(configured) = screen.body else { return None };

    configured
        .sections
        .into_iter()
        .flat_map(|section| match section {
            CloudBackupSection::Verification(section) => section.controls,
            _ => Vec::new(),
        })
        .find_map(|control| match control {
            CloudBackupVerificationControl::DeleteUnreadableBackups { consent } => Some(consent),
            _ => None,
        })
}

/// Runs one authorized row delete under its own operation, the way a confirmed row runs it
async fn run_row_delete(
    manager: &RustCloudBackupManager,
    authorization: Arc<DeleteAuthorization>,
    record_id: &str,
) -> Result<RowDeleteReport, CloudBackupError> {
    let (claim, writes) =
        begin_operation_writes_for_test(manager, CloudBackupExclusiveOperation::DeleteCloudWallet)
            .await;
    let report =
        manager.delete_authorized_cloud_wallet(writes, authorization, record_id.to_owned()).await;
    finish_operation_for_test(manager, claim).await;

    report
}

/// Backups of wallets this device doesn't have, written under a key it doesn't hold
struct UndecryptableRows {
    namespace: String,
    wallets: Vec<WalletMetadata>,
    /// In record ID order, the order a batch deletes them in
    record_ids: Vec<String>,
}

/// Lists one undecryptable backup for each of `networks`, loads them as rows, and reports the
/// verification that found them, so the screen offers their batch delete
async fn load_undecryptable_rows(
    manager: &RustCloudBackupManager,
    networks: &[Network],
) -> UndecryptableRows {
    let namespace = manager.current_namespace_id().unwrap();
    let other_master_key = cove_cspp::master_key::MasterKey::generate();
    let mut wallets = Vec::new();
    for network in networks {
        let mut wallet = xpub_only_wallet_metadata();
        wallet.network = *network;
        let record_id = cove_cspp::backup_data::wallet_record_id(wallet.id.as_ref());
        test_globals().cloud.set_wallet_backup(
            namespace.clone(),
            record_id,
            encrypted_wallet_backup_bytes(&wallet, &other_master_key, "bad", 1).await,
        );
        wallets.push(wallet);
    }

    wallets.sort_by_key(|wallet| cove_cspp::backup_data::wallet_record_id(wallet.id.as_ref()));
    let record_ids: Vec<String> = wallets
        .iter()
        .map(|wallet| cove_cspp::backup_data::wallet_record_id(wallet.id.as_ref()))
        .collect();

    let listed: Vec<&str> = record_ids.iter().map(String::as_str).collect();
    load_rows(manager, &namespace, &listed).await;
    manager.apply_verification_state(VerificationState::NeedsAttention(DeepVerificationReport {
        master_key_wrapper_repaired: false,
        local_master_key_repaired: false,
        credential_recovered: false,
        wallets_verified: 0,
        wallet_issues: CloudBackupWalletVerificationIssues {
            decryption_failed: u32::try_from(record_ids.len()).unwrap(),
            ..CloudBackupWalletVerificationIssues::default()
        },
        detail: None,
    }));

    UndecryptableRows { namespace, wallets, record_ids }
}

/// Reads the confirmed batch the way a batch delete starts, before it is frozen
async fn prepare_batch(
    manager: &RustCloudBackupManager,
    consent: &CloudBackupBatchDeleteConsent,
) -> CloudBackupPreparedUndecryptableWalletDeletion {
    let read = call!(manager.supervisor.start_unreadable_read_for_test()).await.unwrap();
    let confirmed = consent.record_ids().map(|record_id| record_id.as_str().to_owned()).collect();

    manager
        .prepare_delete_undecryptable_wallet_backups(
            &unowned_writes(
                manager,
                CloudBackupExclusiveOperation::DeleteUndecryptableWalletBackups,
            ),
            confirmed,
            permit(PermitIntent::DeleteUndecryptableWalletBackups),
            read,
        )
        .await
        .unwrap()
}

/// Freezes the confirmed batch to what `prepared` read
async fn freeze_batch(
    manager: &RustCloudBackupManager,
    consent: &CloudBackupBatchDeleteConsent,
    prepared: &CloudBackupPreparedUndecryptableWalletDeletion,
) -> Arc<DeleteAuthorization> {
    call!(
        manager
            .supervisor
            .freeze_unreadable_batch_for_test(consent.clone(), prepared.reads().clone())
    )
    .await
    .unwrap()
    .expect("the inventory is readable")
}

async fn wait_for_operation_to_finish(manager: &RustCloudBackupManager) {
    wait_for_test_condition(Duration::from_secs(5), "the row delete should finish", || {
        manager.projected_exclusive_operation().is_none()
    })
    .await;
}

/// Waits until the rows are a read of the latest listing again, with no fetch's help from the test
async fn wait_for_rows_read_again(manager: &RustCloudBackupManager) {
    wait_for_test_condition(Duration::from_secs(5), "the owner should read the rows again", || {
        let inventory = manager.inventory.read();
        matches!(inventory.cloud_only(), CloudOnlyState::Loaded { .. })
            && inventory.loaded_rows_are_current()
    })
    .await;
}

/// Whether the screen offers Turn Off now
async fn turn_off_availability(
    manager: &RustCloudBackupManager,
) -> Option<CloudBackupActionAvailability> {
    let screen = call!(manager.supervisor.projected_screen_for_test()).await.unwrap();
    screen
        .actions
        .entries
        .into_iter()
        .find(|entry| entry.action == CloudBackupScreenAction::TurnOff)
        .map(|entry| entry.availability)
}

/// Runs the frozen batch under its own operation, the way a confirmed batch runs it
async fn run_batch(
    manager: &RustCloudBackupManager,
    prepared: CloudBackupPreparedUndecryptableWalletDeletion,
    authorization: Arc<DeleteAuthorization>,
) -> crate::manager::cloud_backup_manager::ops::BatchDeleteReport {
    let (claim, writes) = begin_operation_writes_for_test(
        manager,
        CloudBackupExclusiveOperation::DeleteUndecryptableWalletBackups,
    )
    .await;
    let report = manager
        .delete_authorized_undecryptable_wallet_backups(
            prepared.into_reader(),
            authorization,
            writes,
        )
        .await;
    finish_operation_for_test(manager, claim).await;

    report
}

#[tokio::test(flavor = "current_thread")]
async fn a_seed_backup_only_a_watch_only_wallet_matches_may_be_the_only_copy() {
    let _guard = async_test_lock().lock().await;
    let manager = test_supervisor_manager();
    configure_enabled_cloud_backup(&manager, test_globals(), 0);
    let namespace = manager.current_namespace_id().unwrap();
    let secret = test_seed_secret();

    save_local_watch_only_wallet(&secret);
    let (_, seed_record_id) = upload_test_seed_backup(&namespace, &secret);
    load_rows(&manager, &namespace, &[seed_record_id.as_str()]).await;

    // public keys can't sign, so deleting the seed can lose the wallet and asks for that
    let consent =
        row_delete_consent(&manager, &seed_record_id).await.expect("the row offers Delete");
    assert_eq!(consent.evidence().category(), CloudBackupDeleteCategory::NotOnDevice);
    assert_eq!(consent.confirmation(), CloudBackupDeleteConfirmation::MayBeOnlyCopy);
}

#[tokio::test(flavor = "current_thread")]
async fn an_extra_copy_left_by_an_added_seed_waits_for_the_wallet_backup() {
    let _guard = async_test_lock().lock().await;
    let globals = test_globals();
    let manager = test_supervisor_manager();
    configure_enabled_cloud_backup(&manager, globals, 0);
    let namespace = manager.current_namespace_id().unwrap();
    let secret = test_seed_secret();

    let local_wallet = save_local_watch_only_wallet(&secret);
    confirm_own_backup(&namespace, &local_wallet);
    let (_, seed_record_id) = upload_test_seed_backup(&namespace, &secret);
    let seed_record_id = seed_record_id.as_str();
    load_rows(&manager, &namespace, &[seed_record_id]).await;

    let ticket = Some(tap(PermitIntent::RestoreCloudWallet));
    call!(manager.supervisor.start_restore_cloud_wallet_operation(seed_record_id.into(), ticket))
        .await
        .unwrap();
    wait_for_operation_to_finish(&manager).await;
    wait_for_cloud_only_statuses(
        &manager,
        &[(seed_record_id, CloudBackupWalletStatus::AlreadyOnDevice)],
    )
    .await;

    // the wallet now holds the seed, but the cloud has no backup of it that does
    assert_eq!(
        row_controls(&manager, seed_record_id).await,
        Some(vec![CloudBackupRowControl::BackUpFirst, CloudBackupRowControl::CheckAgain])
    );

    let own_record_id = upload_own_backup(&manager, &local_wallet).await;
    read_inventory(&manager, &namespace, &[own_record_id.as_str(), seed_record_id]).await;
    start_row_delete(&manager, seed_record_id).await;
    wait_for_operation_to_finish(&manager).await;

    assert!(!globals.cloud.has_wallet_backup(&namespace, seed_record_id));
    assert!(local_wallet_exists(&local_wallet));
}

/// A sync row that once said Confirmed proves nothing about what the cloud holds now, so an extra
/// copy stays until a read finds the covering wallet's current backup
#[tokio::test(flavor = "current_thread")]
async fn an_extra_copy_whose_wallet_backup_went_missing_stays() {
    let _guard = async_test_lock().lock().await;
    let globals = test_globals();
    let manager = test_supervisor_manager();
    let fixture = seed_local_wallet_with_cloud_duplicate(&manager).await;
    let duplicate_record_id = fixture.duplicate_record_id.as_str();
    confirm_own_backup(&fixture.namespace, &fixture.local_wallet);

    // another device removed the wallet's own backup, and the read sees that
    globals.cloud.remove_wallet_backup(&fixture.namespace, &fixture.local_record_id);
    read_inventory(&manager, &fixture.namespace, &[duplicate_record_id]).await;

    assert_eq!(
        row_controls(&manager, duplicate_record_id).await,
        Some(vec![CloudBackupRowControl::BackUpFirst, CloudBackupRowControl::CheckAgain])
    );
    assert!(globals.cloud.has_wallet_backup(&fixture.namespace, duplicate_record_id));

    // once the wallet is backed up again, the copy can go
    let own_record_id = upload_own_backup(&manager, &fixture.local_wallet).await;
    read_inventory(&manager, &fixture.namespace, &[own_record_id.as_str(), duplicate_record_id])
        .await;

    assert_eq!(
        offered_delete(&manager, duplicate_record_id).await,
        Some(CloudBackupDeleteCategory::ExtraCopy)
    );
}

/// An invalidated inventory offers no Delete, and a consent given before it no longer holds,
/// until the owner's own fetch reads the rows again under the new listing
#[tokio::test(flavor = "current_thread")]
async fn an_invalidation_has_the_owner_read_the_rows_again() {
    let _guard = async_test_lock().lock().await;
    let globals = test_globals();
    let manager = test_supervisor_manager();
    configure_enabled_cloud_backup(&manager, globals, 0);
    let row = load_not_on_device_row(&manager).await;

    let read = vec![CloudBackupRowControl::Restore, CloudBackupRowControl::Delete];
    assert_eq!(row_controls(&manager, &row.record_id).await, Some(read.clone()));
    let consent =
        row_delete_consent(&manager, &row.record_id).await.expect("the row offers Delete");
    assert!(holds(&manager, &consent).await.is_ok());

    // the fetch the invalidation asks for waits on the provider
    let listing = globals.cloud.gate_next_list_wallet_files();
    manager.apply_inventory(InventoryInput::CloudOnlyInventoryInvalidated);
    listing.wait_until_blocked().await;

    assert_eq!(offered_delete(&manager, &row.record_id).await, None);
    assert_eq!(holds(&manager, &consent).await.unwrap_err(), StaleTarget::ListingChanged);

    listing.release();
    wait_for_rows_read_again(&manager).await;

    assert_eq!(row_controls(&manager, &row.record_id).await, Some(read));
    let fresh = row_delete_consent(&manager, &row.record_id).await.expect("the row offers Delete");
    assert_ne!(fresh, consent);
    assert!(holds(&manager, &fresh).await.is_ok());
}

/// A row delete is a lane command: it waits behind the upload in flight and runs after it
#[tokio::test(flavor = "current_thread")]
async fn row_delete_waits_for_the_write_in_flight() {
    let _guard = async_test_lock().lock().await;
    let globals = test_globals();
    let manager = test_supervisor_manager();
    let fixture = seed_local_wallet_with_cloud_duplicate(&manager).await;
    let row = load_not_on_device_row(&manager).await;

    let (upload_gate, upload) =
        hold_wallet_upload_in_flight(&manager, globals, fixture.local_wallet.id.clone()).await;
    start_row_delete(&manager, &row.record_id).await;
    wait_for_queued_lane_write(&manager).await;

    assert!(globals.cloud.has_wallet_backup(&row.namespace, &row.record_id));

    upload_gate.release();
    upload.await.unwrap().unwrap();
    wait_for_test_condition(Duration::from_secs(5), "the row delete runs after the upload", || {
        !globals.cloud.has_wallet_backup(&row.namespace, &row.record_id)
            && manager.projected_exclusive_operation().is_none()
    })
    .await;
}

#[tokio::test(flavor = "current_thread")]
async fn a_queued_row_delete_whose_wallet_came_back_deletes_nothing() {
    let _guard = async_test_lock().lock().await;
    let globals = test_globals();
    let manager = test_supervisor_manager();
    let fixture = seed_local_wallet_with_cloud_duplicate(&manager).await;
    let row = load_not_on_device_row(&manager).await;

    let (upload_gate, upload) =
        hold_wallet_upload_in_flight(&manager, globals, fixture.local_wallet.id.clone()).await;
    start_row_delete(&manager, &row.record_id).await;
    wait_for_queued_lane_write(&manager).await;

    // the wallet is imported again while its confirmed delete waits, so the backup is its own
    Database::global()
        .wallets
        .save_all_wallets(row.wallet.network, row.wallet.wallet_mode, vec![row.wallet.clone()])
        .unwrap();
    manager.apply_local_wallets_changed();

    upload_gate.release();
    upload.await.unwrap().unwrap();
    wait_for_operation_to_finish(&manager).await;

    // the wallet's own backup is still there
    assert!(globals.cloud.has_wallet_backup(&row.namespace, &row.record_id));
    assert!(local_wallet_exists(&row.wallet));
}

#[tokio::test(flavor = "current_thread")]
async fn a_row_delete_of_a_record_already_gone_completes() {
    let _guard = async_test_lock().lock().await;
    let globals = test_globals();
    let manager = test_supervisor_manager();
    configure_enabled_cloud_backup(&manager, globals, 0);
    let row = load_not_on_device_row(&manager).await;
    let authorization = authorize(&manager, &row.record_id).await;

    // another device deleted it first
    globals.cloud.fail_wallet_backup_delete(
        row.namespace.clone(),
        row.record_id.clone(),
        CloudStorageError::NotFound(row.record_id.clone()),
    );

    assert!(matches!(
        run_row_delete(&manager, authorization, &row.record_id).await,
        Ok(RowDeleteReport::Removed { already_absent: true })
    ));
}

/// A row delete ends at the provider's answer. The listing that reads the rows again is the
/// inventory owner's own, so one that never answers holds neither the lane nor Turn Off
#[tokio::test(flavor = "current_thread")]
async fn a_row_delete_whose_follow_up_listing_is_held_frees_the_lane_and_turn_off() {
    let _guard = async_test_lock().lock().await;
    let globals = test_globals();
    let manager = test_supervisor_manager();
    configure_enabled_cloud_backup(&manager, globals, 0);
    let row = load_not_on_device_row(&manager).await;

    let listing = globals.cloud.gate_next_list_wallet_files();
    start_row_delete(&manager, &row.record_id).await;
    listing.wait_until_blocked().await;
    wait_for_operation_to_finish(&manager).await;

    assert!(!globals.cloud.has_wallet_backup(&row.namespace, &row.record_id));

    // while the listing is held, another operation starts and the lane drains for it
    let (claim, writes) =
        begin_operation_writes_for_test(&manager, CloudBackupExclusiveOperation::DeleteCloudWallet)
            .await;
    writes.drain().await.expect("the lane holds nothing");
    finish_operation_for_test(&manager, claim).await;
    assert_eq!(
        turn_off_availability(&manager).await,
        Some(CloudBackupActionAvailability::Available)
    );

    listing.release();
    wait_for_rows_read_again(&manager).await;
    assert_eq!(row_controls(&manager, &row.record_id).await, None);
}

#[tokio::test(flavor = "current_thread")]
async fn an_authorization_for_a_record_that_left_and_came_back_deletes_nothing() {
    let _guard = async_test_lock().lock().await;
    let globals = test_globals();
    let manager = test_supervisor_manager();
    configure_enabled_cloud_backup(&manager, globals, 0);
    let row = load_not_on_device_row(&manager).await;
    let authorization = authorize(&manager, &row.record_id).await;

    // the same record ID is listed again, but it is not the record the user confirmed
    manager.apply_detail_outcome(
        manager.inventory_scope(),
        CloudBackupDetailOutcome::refreshed(listed_detail(&[])),
    );
    load_rows(&manager, &row.namespace, &[row.record_id.as_str()]).await;

    assert!(matches!(
        run_row_delete(&manager, authorization, &row.record_id).await,
        Ok(RowDeleteReport::StaleTarget(StaleTarget::ListingChanged))
    ));
    assert!(globals.cloud.has_wallet_backup(&row.namespace, &row.record_id));
}

/// A confirmed batch deletes only the records still undecryptable when it runs: one that reads
/// again before then is skipped, never deleted under a confirmation of what it no longer is
#[tokio::test(flavor = "current_thread")]
async fn a_batch_deletes_only_the_confirmed_records_still_undecryptable() {
    let _guard = async_test_lock().lock().await;
    let globals = test_globals();
    let manager = test_supervisor_manager();
    configure_enabled_cloud_backup(&manager, globals, 0);
    let rows = load_undecryptable_rows(&manager, &[Network::Testnet, Network::Signet]).await;
    let consent = batch_consent(&manager).await.expect("the screen offers the batch delete");
    let [readable, deleted] = [&rows.record_ids[0], &rows.record_ids[1]];

    // another device rewrote one record under the key this device holds, after the confirmation
    upload_test_wallet_backup(&rows.namespace, &rows.wallets[0]);

    call!(manager.supervisor.dispatch(UserIntent::DeleteUndecryptableWalletBackups(consent)))
        .await
        .unwrap();
    wait_for_operation_to_finish(&manager).await;

    assert!(globals.cloud.has_wallet_backup(&rows.namespace, readable));
    assert!(!globals.cloud.has_wallet_backup(&rows.namespace, deleted));
}

/// A record that reads again after the batch read it, but before its turn, is read once more and
/// kept
#[tokio::test(flavor = "current_thread")]
async fn a_batch_record_that_reads_again_before_its_turn_is_kept() {
    let _guard = async_test_lock().lock().await;
    let globals = test_globals();
    let manager = test_supervisor_manager();
    configure_enabled_cloud_backup(&manager, globals, 0);
    let rows = load_undecryptable_rows(&manager, &[Network::Testnet, Network::Signet]).await;
    let consent = batch_consent(&manager).await.expect("the screen offers the batch delete");

    let prepared = prepare_batch(&manager, &consent).await;
    let authorization = freeze_batch(&manager, &consent, &prepared).await;
    assert_eq!(authorization.record_ids().count(), 2);

    // the first record now holds a backup the key reads
    upload_test_wallet_backup(&rows.namespace, &rows.wallets[0]);

    let (claim, writes) = begin_operation_writes_for_test(
        &manager,
        CloudBackupExclusiveOperation::DeleteUndecryptableWalletBackups,
    )
    .await;
    let report = manager
        .delete_authorized_undecryptable_wallet_backups(
            prepared.into_reader(),
            authorization,
            writes,
        )
        .await;
    finish_operation_for_test(&manager, claim).await;

    assert_eq!(report.removed, 1);
    assert!(report.stopped.is_none(), "{:?}", report.stopped);
    assert!(globals.cloud.has_wallet_backup(&rows.namespace, &rows.record_ids[0]));
    assert!(!globals.cloud.has_wallet_backup(&rows.namespace, &rows.record_ids[1]));

    // the owner has the lane's read, so the row says what the record is now
    assert_eq!(
        offered_delete(&manager, &rows.record_ids[0]).await,
        Some(CloudBackupDeleteCategory::NotOnDevice)
    );
}

/// A batch whose listing changed partway stops at the next record and keeps the rest
#[tokio::test(flavor = "current_thread")]
async fn a_batch_whose_inventory_changed_partway_keeps_the_rest() {
    let _guard = async_test_lock().lock().await;
    let globals = test_globals();
    let manager = test_supervisor_manager();
    configure_enabled_cloud_backup(&manager, globals, 0);
    let rows = load_undecryptable_rows(&manager, &[Network::Testnet, Network::Signet]).await;
    let [first, second] = [rows.record_ids[0].clone(), rows.record_ids[1].clone()];
    let consent = batch_consent(&manager).await.expect("the screen offers the batch delete");

    let prepared = prepare_batch(&manager, &consent).await;
    let authorization = freeze_batch(&manager, &consent, &prepared).await;
    let (claim, writes) = begin_operation_writes_for_test(
        &manager,
        CloudBackupExclusiveOperation::DeleteUndecryptableWalletBackups,
    )
    .await;

    // hold the lane's read of the first record, after its turn was checked
    let reads_so_far =
        globals.cloud.wallet_backup_download_attempt_count_for_record(&rows.namespace, &first);
    let gate = globals.cloud.gate_wallet_backup_download_after_successes(
        rows.namespace.clone(),
        first.clone(),
        reads_so_far,
    );
    let deleting = manager.clone();
    let batch = tokio::spawn(async move {
        deleting
            .delete_authorized_undecryptable_wallet_backups(
                prepared.into_reader(),
                authorization,
                writes,
            )
            .await
    });
    gate.wait_until_blocked().await;

    manager.apply_inventory(InventoryInput::CloudOnlyInventoryInvalidated);
    gate.release();
    let report = batch.await.unwrap();
    finish_operation_for_test(&manager, claim).await;

    assert_eq!(report.removed, 1);
    assert!(matches!(report.stopped, Some(BatchStop::StaleTarget(StaleTarget::ListingChanged))));
    assert!(!globals.cloud.has_wallet_backup(&rows.namespace, &first));
    assert!(globals.cloud.has_wallet_backup(&rows.namespace, &second));
}

/// A wallet imported while the batch read the cloud makes the read stale, so its record, now
/// the wallet's own backup, is never deleted
#[tokio::test(flavor = "current_thread")]
async fn a_batch_read_before_a_wallet_import_deletes_nothing() {
    let _guard = async_test_lock().lock().await;
    let globals = test_globals();
    let manager = test_supervisor_manager();
    configure_enabled_cloud_backup(&manager, globals, 0);
    let rows = load_undecryptable_rows(&manager, &[Network::Testnet]).await;
    let consent = batch_consent(&manager).await.expect("the screen offers the batch delete");

    let prepared = prepare_batch(&manager, &consent).await;
    assert_eq!(
        prepared.reads().reads,
        BTreeMap::from([(rows.record_ids[0].clone(), RecordRead::Undecryptable)])
    );

    // the wallet lands after the read listed it as cloud-only, before the batch is frozen
    persist_xpub_wallets(vec![rows.wallets[0].clone()]);
    manager.apply_local_wallets_changed();

    let authorization = freeze_batch(&manager, &consent, &prepared).await;

    assert_eq!(
        authorization.record_ids().count(),
        0,
        "a read from before the import authorizes nothing"
    );
    assert!(globals.cloud.has_wallet_backup(&rows.namespace, &rows.record_ids[0]));
}

/// A record another device replaced with a readable backup after the batch read it keeps the
/// listing's record set. The fetch since read the record as not on this device, which is newer
/// than the batch's read, so the owner keeps that row and the batch deletes nothing
#[tokio::test(flavor = "current_thread")]
async fn a_batch_record_a_later_read_finds_readable_is_not_authorized() {
    let _guard = async_test_lock().lock().await;
    let globals = test_globals();
    let manager = test_supervisor_manager();
    configure_enabled_cloud_backup(&manager, globals, 0);
    let rows = load_undecryptable_rows(&manager, &[Network::Testnet]).await;
    let record_id = rows.record_ids[0].as_str();
    let consent = batch_consent(&manager).await.expect("the screen offers the batch delete");

    let prepared = prepare_batch(&manager, &consent).await;
    assert_eq!(
        prepared.reads().reads,
        BTreeMap::from([(record_id.to_owned(), RecordRead::Undecryptable)])
    );

    // the same record now holds a backup the key reads, and a fetch reads it as a restorable row
    upload_test_wallet_backup(&rows.namespace, &rows.wallets[0]);
    load_rows(&manager, &rows.namespace, &[record_id]).await;
    assert_eq!(
        offered_delete(&manager, record_id).await,
        Some(CloudBackupDeleteCategory::NotOnDevice)
    );

    let authorization = freeze_batch(&manager, &consent, &prepared).await;

    assert_eq!(authorization.record_ids().count(), 0);
    assert!(globals.cloud.has_wallet_backup(&rows.namespace, record_id));
}

/// A record that reads again while the batch runs gives its read to the inventory owner, even
/// when a later record stops the batch: its row says what it now is, and the Delete offered on
/// the unreadable row no longer holds
#[tokio::test(flavor = "current_thread")]
async fn a_batch_record_read_readable_loses_its_unreadable_delete() {
    let _guard = async_test_lock().lock().await;
    let globals = test_globals();
    let manager = test_supervisor_manager();
    configure_enabled_cloud_backup(&manager, globals, 0);
    let rows = load_undecryptable_rows(&manager, &[Network::Testnet, Network::Signet]).await;
    let [readable, failing] = [rows.record_ids[0].as_str(), rows.record_ids[1].as_str()];
    let consent = batch_consent(&manager).await.expect("the screen offers the batch delete");

    let prepared = prepare_batch(&manager, &consent).await;
    let authorization = freeze_batch(&manager, &consent, &prepared).await;
    assert_eq!(authorization.record_ids().count(), 2);

    // the verified read is the row's own fact, so the row draws the Delete it carries
    let unreadable = row_delete_consent(&manager, readable).await.expect("the row offers Delete");
    assert_eq!(
        unreadable.evidence().category(),
        CloudBackupDeleteCategory::Unreadable { kind: CloudBackupUnreadableKind::Decrypt }
    );

    // another device rewrites the first under the key this device holds, and the provider goes
    // offline before the second is read
    upload_test_wallet_backup(&rows.namespace, &rows.wallets[0]);
    globals.cloud.fail_wallet_backup_download_offline(
        rows.namespace.clone(),
        failing.to_owned(),
        "offline",
    );

    let report = run_batch(&manager, prepared, authorization).await;

    assert_eq!(report.removed, 0);
    assert!(matches!(report.stopped, Some(BatchStop::Failed(_))), "{:?}", report.stopped);
    assert!(globals.cloud.has_wallet_backup(&rows.namespace, readable));
    assert!(globals.cloud.has_wallet_backup(&rows.namespace, failing));

    assert_eq!(
        row_controls(&manager, readable).await,
        Some(vec![CloudBackupRowControl::Restore, CloudBackupRowControl::Delete])
    );
    assert_eq!(
        offered_delete(&manager, readable).await,
        Some(CloudBackupDeleteCategory::NotOnDevice)
    );
    assert_eq!(holds(&manager, &unreadable).await.unwrap_err(), StaleTarget::CategoryChanged);
}

/// The lane reads an unreadable record just before deleting it, so one another device rewrote
/// while the delete waited behind an upload is kept
#[tokio::test(flavor = "current_thread")]
async fn an_unreadable_delete_reads_the_record_when_the_lane_runs_it() {
    let _guard = async_test_lock().lock().await;
    let globals = test_globals();
    let manager = test_supervisor_manager();
    let fixture = seed_local_wallet_with_cloud_duplicate(&manager).await;
    let rows = load_undecryptable_rows(&manager, &[Network::Testnet]).await;
    let record_id = rows.record_ids[0].as_str();
    let consent = batch_consent(&manager).await.expect("the screen offers the batch delete");

    let (upload_gate, upload) =
        hold_wallet_upload_in_flight(&manager, globals, fixture.local_wallet.id.clone()).await;
    call!(manager.supervisor.dispatch(UserIntent::DeleteUndecryptableWalletBackups(consent)))
        .await
        .unwrap();
    wait_for_queued_lane_write(&manager).await;

    // the record becomes readable while its delete waits in the lane
    upload_test_wallet_backup(&rows.namespace, &rows.wallets[0]);
    upload_gate.release();
    upload.await.unwrap().unwrap();
    wait_for_operation_to_finish(&manager).await;

    assert!(globals.cloud.has_wallet_backup(&rows.namespace, record_id));
}

/// Gives the undecryptable rows the batch's read under the verified key, the read their own
/// Delete trusts
async fn verify_unreadable_rows(manager: &RustCloudBackupManager) {
    let consent = batch_consent(manager).await.expect("the screen offers the batch delete");
    let prepared = prepare_batch(manager, &consent).await;
    freeze_batch(manager, &consent, &prepared).await;
}

/// A single unreadable row's Delete follows the batch's rule: the lane reads the record with the
/// verified key just before deleting it, so a record still undecryptable is deleted, and one
/// another device rewrote while the delete waited behind an upload is kept
#[tokio::test(flavor = "current_thread")]
async fn an_unreadable_row_delete_reads_the_record_when_the_lane_runs_it() {
    let _guard = async_test_lock().lock().await;
    let globals = test_globals();
    let manager = test_supervisor_manager();
    let fixture = seed_local_wallet_with_cloud_duplicate(&manager).await;
    let rows = load_undecryptable_rows(&manager, &[Network::Testnet, Network::Signet]).await;
    let [rewritten, still_unreadable] = [rows.record_ids[0].as_str(), rows.record_ids[1].as_str()];
    let unreadable =
        Some(CloudBackupDeleteCategory::Unreadable { kind: CloudBackupUnreadableKind::Decrypt });

    verify_unreadable_rows(&manager).await;
    assert_eq!(offered_delete(&manager, still_unreadable).await, unreadable);
    start_row_delete(&manager, still_unreadable).await;
    wait_for_operation_to_finish(&manager).await;
    assert!(!globals.cloud.has_wallet_backup(&rows.namespace, still_unreadable));

    // the delete's refetch reads the other row again, and the batch's read trusts it again
    wait_for_rows_read_again(&manager).await;
    verify_unreadable_rows(&manager).await;
    let consent = row_delete_consent(&manager, rewritten).await.expect("the row offers Delete");
    assert_eq!(Some(consent.evidence().category()), unreadable);

    let (upload_gate, upload) =
        hold_wallet_upload_in_flight(&manager, globals, fixture.local_wallet.id.clone()).await;
    call!(manager.supervisor.dispatch(UserIntent::DeleteCloudWallet(consent))).await.unwrap();
    wait_for_queued_lane_write(&manager).await;

    // the record becomes readable while its delete waits in the lane
    upload_test_wallet_backup(&rows.namespace, &rows.wallets[0]);
    upload_gate.release();
    upload.await.unwrap().unwrap();
    wait_for_operation_to_finish(&manager).await;

    assert!(globals.cloud.has_wallet_backup(&rows.namespace, rewritten));
    wait_for_rows_read_again(&manager).await;
    assert_eq!(
        offered_delete(&manager, rewritten).await,
        Some(CloudBackupDeleteCategory::NotOnDevice)
    );
}

/// An unreadable record another device deleted before the lane read it is what its delete
/// wanted: the row delete completes without asking the provider to delete it, the row goes, and
/// so does the record's sync state
#[tokio::test(flavor = "current_thread")]
async fn an_unreadable_row_already_gone_completes_its_delete() {
    let _guard = async_test_lock().lock().await;
    let globals = test_globals();
    let manager = test_supervisor_manager();
    configure_enabled_cloud_backup(&manager, globals, 0);
    let rows = load_undecryptable_rows(&manager, &[Network::Testnet]).await;
    let record_id = rows.record_ids[0].as_str();
    verify_unreadable_rows(&manager).await;
    confirm_own_backup(&rows.namespace, &rows.wallets[0]);

    // another device deleted it, and a delete sent to the provider now would fail
    globals.cloud.remove_wallet_backup(&rows.namespace, record_id);
    globals.cloud.set_wallet_files(rows.namespace.clone(), Vec::new());
    globals.cloud.fail_wallet_backup_delete(
        rows.namespace.clone(),
        record_id.to_owned(),
        CloudStorageError::NotFound(record_id.to_owned()),
    );
    let authorization = authorize(&manager, record_id).await;

    assert!(matches!(
        run_row_delete(&manager, authorization, record_id).await,
        Ok(RowDeleteReport::Removed { already_absent: true })
    ));
    assert_eq!(Database::global().cloud_blob_sync_states.get(record_id).unwrap(), None);

    confirm_own_backup(&rows.namespace, &rows.wallets[0]);
    start_row_delete(&manager, record_id).await;
    wait_for_operation_to_finish(&manager).await;

    assert_eq!(row_controls(&manager, record_id).await, None);
    assert!(!matches!(
        manager.inventory.read().cloud_only_operation(),
        CloudOnlyOperation::Failed { .. }
    ));
    assert_eq!(Database::global().cloud_blob_sync_states.get(record_id).unwrap(), None);
}

/// A batch that went stale has the rows read again, and a refresh still waiting on an older
/// listing is replaced instead of waited for
#[tokio::test(flavor = "current_thread")]
async fn a_stale_batch_has_the_rows_read_again() {
    let _guard = async_test_lock().lock().await;
    let globals = test_globals();
    let manager = test_supervisor_manager();
    configure_enabled_cloud_backup(&manager, globals, 0);
    let rows = load_undecryptable_rows(&manager, &[Network::Testnet, Network::Signet]).await;
    let [first, second] = [rows.record_ids[0].clone(), rows.record_ids[1].clone()];
    let consent = batch_consent(&manager).await.expect("the screen offers the batch delete");

    // the preparation reads each record once; hold the lane's read of the first after that
    let reads_so_far =
        globals.cloud.wallet_backup_download_attempt_count_for_record(&rows.namespace, &first);
    let lane_read = globals.cloud.gate_wallet_backup_download_after_successes(
        rows.namespace.clone(),
        first.clone(),
        reads_so_far + 1,
    );
    call!(manager.supervisor.dispatch(UserIntent::DeleteUndecryptableWalletBackups(consent)))
        .await
        .unwrap();
    lane_read.wait_until_blocked().await;

    // the listing moves while the first record's delete runs, and the refresh that asks for
    // never hears back from the provider
    let held = globals.cloud.gate_next_list_wallet_files();
    manager.apply_inventory(InventoryInput::CloudOnlyInventoryInvalidated);
    held.wait_until_blocked().await;
    lane_read.release();
    wait_for_operation_to_finish(&manager).await;

    assert!(!globals.cloud.has_wallet_backup(&rows.namespace, &first));
    assert!(globals.cloud.has_wallet_backup(&rows.namespace, &second));
    let batch = manager
        .state
        .read()
        .configured_state()
        .map(|configured| configured.undecryptable_wallet_deletion);
    assert!(matches!(
        batch,
        Some(CloudBackupUndecryptableWalletDeletionState::Failed(message))
            if message == "Deleted 1 inaccessible wallet backup, then stopped because your \
                           backups changed. Check again before deleting the rest."
    ));

    wait_for_rows_read_again(&manager).await;
    assert!(batch_consent(&manager).await.is_some(), "the second record is offered again");
    held.release();
}

/// A batch preparation a provider failure stops still gives the inventory owner what it read
/// before it stopped: a record read readable loses the Delete its unreadable row carried, and
/// nothing is deleted
#[tokio::test(flavor = "current_thread")]
async fn a_failed_batch_preparation_still_applies_what_it_read() {
    let _guard = async_test_lock().lock().await;
    let globals = test_globals();
    let manager = test_supervisor_manager();
    configure_enabled_cloud_backup(&manager, globals, 0);
    let rows = load_undecryptable_rows(&manager, &[Network::Testnet, Network::Signet]).await;
    let [readable, failing] = [rows.record_ids[0].as_str(), rows.record_ids[1].as_str()];

    // an earlier batch read both under the verified key, so each row offers its own Delete
    let consent = batch_consent(&manager).await.expect("the screen offers the batch delete");
    let prepared = prepare_batch(&manager, &consent).await;
    freeze_batch(&manager, &consent, &prepared).await;
    let unreadable = row_delete_consent(&manager, readable).await.expect("the row offers Delete");

    // another device rewrites the first under the key this device holds, and the provider goes
    // offline for the second
    upload_test_wallet_backup(&rows.namespace, &rows.wallets[0]);
    globals.cloud.fail_wallet_backup_download_offline(
        rows.namespace.clone(),
        failing.to_owned(),
        "offline",
    );

    let consent = batch_consent(&manager).await.expect("the screen offers the batch delete");
    call!(manager.supervisor.dispatch(UserIntent::DeleteUndecryptableWalletBackups(consent)))
        .await
        .unwrap();
    wait_for_operation_to_finish(&manager).await;

    assert!(globals.cloud.has_wallet_backup(&rows.namespace, readable));
    assert!(globals.cloud.has_wallet_backup(&rows.namespace, failing));
    assert_eq!(
        offered_delete(&manager, readable).await,
        Some(CloudBackupDeleteCategory::NotOnDevice)
    );
    assert_eq!(holds(&manager, &unreadable).await.unwrap_err(), StaleTarget::CategoryChanged);
}
