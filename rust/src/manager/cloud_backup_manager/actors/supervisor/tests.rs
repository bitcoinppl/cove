
use super::*;
use crate::manager::cloud_backup_manager::owner::inventory::test_support::{
    CloudOnlyStateTestExt as _, listed_rows, row_wallet,
};
use crate::manager::cloud_backup_manager::screen::CloudBackupUnreadableKind;
use crate::manager::cloud_backup_manager::{CloudOnlyFetch, CloudOnlyRow, InventoryReadAt};
use cove_tokio::task::spawn_actor;
use crate::manager::cloud_backup_manager::owner::operation::test_support::operation_id;
use crate::manager::cloud_backup_manager::screen::{
    CloudBackupDisableImpact, CloudBackupDisableNotStartedReason, CloudBackupInventoryAssessment,
    CloudBackupListedRow, CloudBackupOutcome, CloudBackupRowActionKind, CloudBackupSection,
};
use crate::manager::cloud_backup_manager::delete_consent::CloudBackupRowDeleteConsent;
use crate::manager::cloud_backup_manager::admission::{
    Admission, Rejection, ReviewExit, TurnOffAdmission, UserIntent, evaluate_admission,
};
use crate::manager::cloud_backup_manager::turn_off::{
    DeleteAuthority, DisableImpact, EvidenceUnavailable, FinishRun, FinishStart, TurnOffReview,
};
use crate::manager::cloud_backup_manager::turn_off::test_support::{
    empty_evidence, empty_review, empty_review_assessed, review_not_on_device,
};
use crate::manager::cloud_backup_manager::owner::lifecycle::{
    CommittedDisable, DisablingRecord,
};
use super::enable::{
    EnableRecoveryFinalization, EnableUploadFinalization,
};
use super::verification::{DeepVerificationContinuation, VerificationRun};
use crate::manager::cloud_backup_manager::wallets::{
    PasskeyRequestAuthority, PermitGrant, PermitIntent,
};
use cove_cspp::{MasterKeyPromotionActiveState, MasterKeyPromotionStatus};
use cove_tokio::FutureTimeoutExt as _;
use crate::test_support::loaded_host_wait;
use cove_cspp::CsppStore as _;
use crate::database::cloud_backup::{
    PersistedDisableDisposition, PersistedDisablingCloudBackup, PersistedDriveAccountSwitch,
    PersistedDriveAccountSwitchPhase,
};
use crate::manager::cloud_backup_manager::CloudBackupWalletStatus;
use crate::manager::cloud_backup_manager::CloudBackupDetailState;
use crate::manager::cloud_backup_manager::keychain::CSPP_PASSKEY_PROVIDER_HINT_KEY;
use crate::manager::cloud_backup_manager::ops::test_support::{
    begin_operation_writes_for_test, finish_operation_for_test,
    async_test_lock, complete_cloud_only_fetch, complete_loaded_outcome,
    configure_enabled_cloud_backup, encrypted_wallet_backup_bytes_for_entry,
    hold_wallet_upload_in_flight, listed_detail, permit, persist_dirty_blob_state, tap,
    wait_for_queued_lane_write,
    persisted_enabled_cloud_backup_state, reset_cloud_backup_test_state,
    seed_verifiable_cloud_master_key, test_globals,
    staged_pending_enable_journal, supervisor_checkpoint, unowned_writes, current_config,
    wait_for_background_refreshes, wait_for_test_condition, wallet_entry_with_labels,
    xpub_only_wallet_metadata,
};
use crate::manager::cloud_backup_manager::reconcile::CloudBackupReconcileMessage;
use crate::manager::cloud_backup_manager::verify::coordinator::{
    CloudBackupVerificationEffect, VerificationDetail,
};
use crate::manager::cloud_backup_manager::wallets::{
    StagedPrfKey, UnpersistedPrfKey,
};
use crate::manager::cloud_backup_manager::screen::CloudBackupScreenBody;
use crate::manager::cloud_backup_manager::screen::CloudBackupDriveAccountSwitchStage;
use crate::manager::cloud_backup_manager::{DriveAccountSwitchPlatformResult, RequestedDriveAccountSwitchStep};
use crate::manager::cloud_backup_manager::{
    CloudBackupDetail, CloudBackupInventoryAuthority, CloudBackupLifecycle, CloudBackupListedDetail,
    CloudBackupOtherBackupsSummary,
    CloudBackupPendingEnableCleanupState, CloudBackupPendingEnableRecovery,
    CloudBackupSettingsRowStatus, CloudOnlyState, PendingEnableJournal,
    PendingEnableNamespaceOwnership, PendingEnableJournalPhase, PendingEnablePasskeyMetadata,
};
use crate::manager::deferred_sender::SingleOrMany;
use crate::network::Network;
use crate::wallet::WalletAddressType;
use crate::wallet::fingerprint::Fingerprint;
use crate::wallet::metadata::{WalletId, WalletMetadata, WalletType};
use crate::wallet_secret::WalletSecretExt as _;
use cove_device::keychain::WalletSecret as KeychainWalletSecret;

const VALID_NAMESPACE_ID: &str = "0123456789abcdef0123456789abcdef";

/// The namespace the keychain holds, which a cloud-only fetch lists its records in
fn keychain_namespace() -> String {
    CloudBackupKeychain::global().namespace_id().unwrap().expect("a namespace in the keychain")
}

/// A standalone supervisor with its own test writer, beside the manager's spawned one
fn test_supervisor(
    manager: Weak<RustCloudBackupManager>,
    writes: Addr<CloudBackupWriteSupervisor>,
) -> CloudBackupSupervisor {
    CloudBackupSupervisor::new(
        manager,
        writes,
        crate::manager::cloud_backup_manager::owner::lifecycle::test_support::owner_writer(),
        None,
    )
}

fn test_supervisor_manager() -> Arc<RustCloudBackupManager> {
    let globals = test_globals();

    // a new manager resumes any persisted transition on init and blocks its writes for it, so a
    // transition an earlier test left behind must be gone first
    let _ = Database::global().cloud_backup_state.delete();
    let manager = crate::manager::cloud_backup_manager::initialize_cloud_backup_manager();
    reset_cloud_backup_test_state(&manager, globals);
    manager
}

fn persist_enabled_for_supervisor_test() {
    Database::global()
        .cloud_backup_state
        .set(&PersistedCloudBackupState::mark_enabled_reset_verification(0, 0))
        .unwrap();
}

/// Seeds a Configured record under a supervisor the test drives directly; it rebases on the
/// seed as it would on its own commit
fn persist_enabled_under(supervisor: &mut CloudBackupSupervisor) {
    persist_enabled_for_supervisor_test();
    supervisor.rebase_on_seeded_record();
}

fn test_committed_disable() -> CommittedDisable {
    match CommittedDisable::from_record(&test_disabling_state()) {
        DisablingRecord::Committed(committed) => committed,
        DisablingRecord::Unnormalized => panic!("the test record is committed"),
    }
}

fn test_disabling_state() -> PersistedDisablingCloudBackup {
    persist_enabled_for_supervisor_test();
    let PersistedCloudBackupState::Configured(previous_configured) =
        Database::global().cloud_backup_state.get().unwrap()
    else {
        panic!("expected configured cloud backup state");
    };

    PersistedDisablingCloudBackup {
        previous_configured,
        namespace_id: VALID_NAMESPACE_ID.into(),
        disable_generation: 7,
        started_at: 1,
        delete_started_at: Some(2),
        last_error: None,
        retry_after: None,
        disposition: PersistedDisableDisposition::DeleteRemote,
        try_again_failed: false,
    }
}

fn test_enable_passkey(credential_id: Vec<u8>) -> UnpersistedPrfKey {
    UnpersistedPrfKey { prf_key: [7; 32], prf_salt: [9; 32], credential_id, provider_hint: None }
}

fn test_staged_passkey(credential_id: Vec<u8>) -> StagedPrfKey {
    StagedPrfKey { prf_salt: [9; 32], credential_id, provider_hint: None }
}


fn prepare_restore_all_marker(manager: &RustCloudBackupManager) {
    Database::global()
        .cloud_backup_state
        .set(&persisted_enabled_cloud_backup_state(Some(0)))
        .unwrap();
    CloudBackupKeychain::global().save_namespace_id(VALID_NAMESPACE_ID).unwrap();
    crate::manager::cloud_backup_manager::owner::lifecycle::test_support::writer()
        .persist_restore_all_marker(VALID_NAMESPACE_ID.into())
        .unwrap();
    manager.sync_persisted_state();
}

/// Starts Restore All the way the supervisor does, with the test's cancellation flag
fn begin_test_restore_all(
    supervisor: &mut CloudBackupSupervisor,
    cancellation: Arc<AtomicBool>,
) -> CloudBackupExclusiveOperationClaim {
    let claim = supervisor.begin_restore_all_exclusive_operation().unwrap();
    supervisor.runtime.restore_all_mut(claim).unwrap().cancellation = cancellation;
    claim
}

fn test_cloud_only_wallet(record_id: &str) -> CloudBackupWalletItem {
    CloudBackupWalletItem {
        name: format!("Wallet {record_id}"),
        network: None,
        wallet_mode: None,
        wallet_type: None,
        fingerprint: None,
        label_count: None,
        backup_updated_at: None,
        sync_status: CloudBackupWalletStatus::DeletedFromDevice,
        restore_failure: None,
        record_id: record_id.into(),
    }
}

fn test_cloud_backup_detail() -> CloudBackupDetail {
    CloudBackupDetail {
        last_sync: None,
        up_to_date: Vec::new(),
        needs_sync: Vec::new(),
    }
}

/// Waits until the loaded rows hold every expected record and none of the replaced ones, which
/// only a fetch that read the provider again can produce
async fn wait_for_cloud_only_rows(
    manager: &RustCloudBackupManager,
    present: &[&str],
    absent: &[&str],
) -> Vec<CloudBackupWalletItem> {
    let loaded_rows = || manager.inventory.read().cloud_only().loaded_wallets();

    wait_for_test_condition(Duration::from_secs(5), "cloud-only rows should be refetched", || {
        loaded_rows().is_some_and(|wallets| {
            present.iter().all(|id| wallets.iter().any(|wallet| wallet.record_id == *id))
                && absent.iter().all(|id| wallets.iter().all(|wallet| wallet.record_id != *id))
        })
    })
    .await;

    loaded_rows().expect("cloud-only rows should be loaded")
}

/// Waits for the rows a finished Restore All reads again after it restored a wallet
///
/// The run's final supervisor message clears the operation before it resets the rows and asks
/// for that fetch, so a test that saw the operation clear can still read the rows from before it
async fn wait_for_rows_refetched_after_restore_all(
    manager: &RustCloudBackupManager,
    present: &[&str],
    absent: &[&str],
) -> Vec<CloudBackupWalletItem> {
    // the supervisor handles messages in order, so once this returns the final message has reset
    // the rows and any loaded rows come from the fetch it asked for
    call!(manager.supervisor.ensure_cloud_only_fetch()).await.unwrap();
    wait_for_cloud_only_rows(manager, present, absent).await
}

/// The wallets of the rows a fetch could read
fn fetch_wallets(fetch: &CloudOnlyFetch) -> Vec<CloudBackupWalletItem> {
    fetch.rows.iter().filter_map(row_wallet).cloned().collect()
}

fn cloud_only_row_statuses(rows: &[CloudBackupWalletItem]) -> Vec<(&str, CloudBackupWalletStatus)> {
    rows.iter().map(|row| (row.record_id.as_str(), row.sync_status.clone())).collect()
}

/// The detail the frontends read, including one retained while a refresh runs or after it failed
fn public_detail(manager: &RustCloudBackupManager) -> Option<CloudBackupDetail> {
    match manager.test_inventory().detail {
        CloudBackupDetailState::Complete { state } => Some(state.detail),
        CloudBackupDetailState::Checking { retained }
        | CloudBackupDetailState::Failed { retained, .. } => retained.map(|state| state.detail),
        CloudBackupDetailState::NotLoaded => None,
    }
}

/// The cloud-only state the frontends read, which only exists once the detail has loaded
fn public_cloud_only(manager: &RustCloudBackupManager) -> Option<CloudOnlyState> {
    if !matches!(manager.model_state().lifecycle, CloudBackupLifecycle::Configured(_)) {
        return None;
    }

    match manager.test_inventory().detail {
        CloudBackupDetailState::Complete { state } => Some(state.cloud_only),
        CloudBackupDetailState::Checking { retained }
        | CloudBackupDetailState::Failed { retained, .. } => {
            retained.map(|retained| retained.cloud_only)
        }
        CloudBackupDetailState::NotLoaded => None,
    }
}

/// Uploads an encrypted backup of the wallet under its own record ID without listing it
fn upload_test_wallet_backup(namespace: &str, wallet: &WalletMetadata) -> String {
    upload_test_wallet_entry(namespace, wallet, &wallet_entry_with_labels(wallet, None))
}

fn upload_test_wallet_entry(
    namespace: &str,
    wallet: &WalletMetadata,
    entry: &cove_cspp::backup_data::WalletEntry,
) -> String {
    let master_key = cove_cspp::Cspp::new(Keychain::global().clone())
        .load_master_key_from_store()
        .unwrap()
        .unwrap();
    let record_id = cove_cspp::backup_data::wallet_record_id(wallet.id.as_ref());
    test_globals().cloud.set_wallet_backup(
        namespace.into(),
        record_id.clone(),
        encrypted_wallet_backup_bytes_for_entry(entry, &master_key, 1),
    );

    record_id
}

fn test_seed_secret() -> KeychainWalletSecret {
    let mnemonic = bip39::Mnemonic::parse(
        "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about",
    )
    .unwrap();

    KeychainWalletSecret::Mnemonic(mnemonic)
}

fn test_seed_fingerprint(secret: &KeychainWalletSecret, network: Network) -> Arc<Fingerprint> {
    Arc::new(secret.xpub(network).fingerprint().into())
}

/// Uploads a seed backup of `secret` under a new wallet's record ID without listing it
fn upload_test_seed_backup(
    namespace: &str,
    secret: &KeychainWalletSecret,
) -> (WalletMetadata, String) {
    let (wallet, entry) = test_seed_backup_entry(secret);
    let record_id = upload_test_wallet_entry(namespace, &wallet, &entry);

    (wallet, record_id)
}

/// A new hot wallet and the cloud backup entry holding its seed
fn test_seed_backup_entry(
    secret: &KeychainWalletSecret,
) -> (WalletMetadata, cove_cspp::backup_data::WalletEntry) {
    let KeychainWalletSecret::Mnemonic(mnemonic) = secret else {
        panic!("seed backups in these tests hold a mnemonic");
    };

    let mut wallet = WalletMetadata::preview_new();
    wallet.id = WalletId::new();
    wallet.wallet_type = WalletType::Hot;
    wallet.master_fingerprint = Some(test_seed_fingerprint(secret, wallet.network));

    let mut entry = wallet_entry_with_labels(&wallet, None);
    entry.secret = cove_cspp::backup_data::WalletSecret::Mnemonic(mnemonic.to_string());

    (wallet, entry)
}

/// Saves a local watch-only wallet with the public keys of `secret`, as importing its
/// descriptors would
fn save_local_watch_only_wallet(secret: &KeychainWalletSecret) -> WalletMetadata {
    save_local_watch_only_wallet_with_address_type(secret, WalletAddressType::NativeSegwit)
}

fn save_local_watch_only_wallet_with_address_type(
    secret: &KeychainWalletSecret,
    address_type: WalletAddressType,
) -> WalletMetadata {
    let mut wallet = WalletMetadata::preview_new();
    wallet.id = WalletId::new();
    wallet.address_type = address_type;
    wallet.name = "Watch-only".into();
    wallet.wallet_type = WalletType::WatchOnly;
    wallet.verified = false;
    wallet.master_fingerprint = Some(test_seed_fingerprint(secret, wallet.network));

    let descriptors = secret.clone().into_descriptors(wallet.network, wallet.address_type);
    wallet.origin = descriptors.origin().ok();

    let keychain = Keychain::global();
    keychain.save_wallet_xpub(&wallet.id, secret.xpub(wallet.network)).unwrap();
    keychain
        .save_public_descriptor(
            &wallet.id,
            descriptors.external.extended_descriptor,
            descriptors.internal.extended_descriptor,
        )
        .unwrap();
    Database::global().wallets.save_restored_wallet_metadata(wallet.clone()).unwrap();

    wallet
}

fn local_wallets_like(wallet: &WalletMetadata) -> Vec<WalletMetadata> {
    Database::global().wallets.get_all(wallet.network, wallet.wallet_mode).unwrap()
}

/// Waits until the loaded rows are exactly `expected`, in any order
async fn wait_for_cloud_only_statuses(
    manager: &RustCloudBackupManager,
    expected: &[(&str, CloudBackupWalletStatus)],
) {
    let mut expected: Vec<_> =
        expected.iter().map(|(record_id, status)| (record_id.to_string(), status.clone())).collect();
    expected.sort_by(|left, right| left.0.cmp(&right.0));

    wait_for_test_condition(Duration::from_secs(5), "cloud-only statuses should settle", || {
        let Some(wallets) = manager.inventory.read().cloud_only().loaded_wallets() else {
            return false;
        };

        let mut statuses: Vec<_> =
            wallets.into_iter().map(|wallet| (wallet.record_id, wallet.sync_status)).collect();
        statuses.sort_by(|left, right| left.0.cmp(&right.0));
        statuses == expected
    })
    .await;
}

/// Uploads readable backups of distinct wallets without listing them, returning their record IDs
fn upload_distinct_test_wallet_backups(manager: &RustCloudBackupManager) -> [String; 3] {
    let namespace = manager.current_namespace_id().unwrap();

    // xpub-only test wallets share one xpub, so each needs its own network to be a distinct wallet
    [Network::Bitcoin, Network::Testnet, Network::Signet].map(|network| {
        let mut wallet = xpub_only_wallet_metadata();
        wallet.network = network;
        upload_test_wallet_backup(&namespace, &wallet)
    })
}

/// Dispatches `intent` through the manager's supervisor, returning how admission decided it and
/// the operation it left holding the slot
async fn dispatch_restore_all(
    manager: &RustCloudBackupManager,
    intent: UserIntent,
) -> (Option<Admission>, Option<CloudBackupExclusiveOperationClaim>) {
    call!(manager.supervisor.dispatch(intent)).await.unwrap();
    let outcome = call!(manager.supervisor.harness_last_outcome()).await.unwrap();
    let claim = call!(manager.supervisor.operation_claim_for_test()).await.unwrap();

    (outcome.map(|outcome| outcome.admission), claim)
}

/// How many wallets Restore All would restore, or `None` while the inventory is not classified
fn restore_all_batch_size(manager: &RustCloudBackupManager) -> Option<usize> {
    manager.restore_all_batch().map(|batch| batch.wallets().len())
}

fn list_test_wallet_files(namespace: &str, record_ids: &[&str]) {
    test_globals().cloud.set_wallet_files(
        namespace.into(),
        record_ids
            .iter()
            .map(|record_id| cove_cspp::backup_data::wallet_filename_from_record_id(record_id))
            .collect(),
    );
}

/// Uploads the wallet's own backup the way a sync does, returning its record ID
async fn upload_own_backup(manager: &RustCloudBackupManager, wallet: &WalletMetadata) -> String {
    persist_dirty_blob_state(wallet.id.clone());
    manager.do_upload_wallet_if_dirty(&wallet.id).await.unwrap();

    cove_cspp::backup_data::wallet_record_id(wallet.id.as_ref())
}

/// Reads the detail and the cloud-only rows from the provider listing `listed`, as Check Again
/// does, so an own row is current only if the cloud holds the wallet's current revision
async fn read_inventory(manager: &RustCloudBackupManager, namespace: &str, listed: &[&str]) {
    list_test_wallet_files(namespace, listed);
    let config = current_config(manager).await;
    let refreshed = manager.refresh_cloud_backup_detail(config).await.expect("a detail refresh");
    apply_refresh_detail_result(manager, manager.inventory_scope(), &refreshed);

    call!(manager.supervisor.ensure_cloud_only_fetch()).await.unwrap();
    wait_for_test_condition(Duration::from_secs(5), "the rows should be read", || {
        manager.inventory.read().loaded_rows_are_current()
    })
    .await;
}

/// Writes a sync row saying the wallet's own backup was confirmed once, which proves nothing
/// about what the cloud holds now
fn confirm_own_backup(namespace: &str, wallet: &WalletMetadata) {
    use crate::database::cloud_backup::{
        CloudBlobConfirmedState, PersistedCloudBlobState, PersistedCloudBlobSyncState,
    };

    Database::global()
        .cloud_blob_sync_states
        .set(&PersistedCloudBlobSyncState::wallet(
            namespace.into(),
            wallet.id.clone(),
            cove_cspp::backup_data::wallet_record_id(wallet.id.as_ref()),
            PersistedCloudBlobState::Confirmed(CloudBlobConfirmedState {
                revision_hash: "confirmed".into(),
                confirmed_at: 1,
            }),
        ))
        .unwrap();
}

fn local_wallet_exists(wallet: &WalletMetadata) -> bool {
    Database::global()
        .wallets
        .get_all(wallet.network, wallet.wallet_mode)
        .unwrap()
        .iter()
        .any(|local| local.id == wallet.id)
}

/// A local wallet restored under its own record ID, plus a listed cloud record of the same wallet
/// identity under another record ID
struct LocalWalletDuplicateFixture {
    namespace: String,
    local_wallet: WalletMetadata,
    local_record_id: String,
    duplicate_record_id: String,
}

async fn seed_local_wallet_with_cloud_duplicate(
    manager: &RustCloudBackupManager,
) -> LocalWalletDuplicateFixture {
    configure_enabled_cloud_backup(manager, test_globals(), 0);
    let namespace = manager.current_namespace_id().unwrap();

    let local_wallet = xpub_only_wallet_metadata();
    let local_record_id = upload_test_wallet_backup(&namespace, &local_wallet);
    let local_restore = manager.prepare_active_namespace_wallet_restore(&unowned_writes(manager, CloudBackupExclusiveOperation::RestoreCloudWallet), permit(PermitIntent::RestoreCloudWallet)).await.unwrap();
    local_restore.restore_record(&local_record_id).await.unwrap();
    assert!(local_wallet_exists(&local_wallet));

    // test xpub-only wallets share one xpub, so this is the same wallet under another record ID
    let duplicate_record_id = upload_test_wallet_backup(&namespace, &xpub_only_wallet_metadata());
    list_test_wallet_files(&namespace, &[duplicate_record_id.as_str()]);

    LocalWalletDuplicateFixture { namespace, local_wallet, local_record_id, duplicate_record_id }
}

fn begin_test_detail_refresh(
    supervisor: &mut CloudBackupSupervisor,
    manager: &RustCloudBackupManager,
) -> DetailRefreshClaim {
    supervisor.detail_workflow.open();
    let DetailRefreshPlan::Start(claim) = supervisor.detail_workflow.request_refresh(DetailRefreshOrigin::Tap) else {
        panic!("expected detail refresh to start");
    };
    manager.apply_detail_outcome(manager.inventory_scope(), CloudBackupDetailOutcome::Checking);

    claim
}

/// Starts a detail refresh and completes it from a trusted local snapshot, leaving it active
/// until the provider confirms
async fn start_trusted_local_refresh(
    supervisor: &mut CloudBackupSupervisor,
    manager: &RustCloudBackupManager,
    listed: CloudBackupListedDetail,
) -> (DetailRefreshClaim, CloudBackupDetailProviderConfirmation) {
    let claim = begin_test_detail_refresh(supervisor, manager);
    let confirmation = CloudBackupDetailProviderConfirmation {
        namespace: manager.current_namespace_id().unwrap(),
    };

    supervisor
        .complete_refresh_detail_from_snapshot(
            Some(CloudBackupDetailSnapshotCompletion::TrustedLocal {
                listed,
                confirmation: confirmation.clone(),
            }),
            DetailRefreshAttempt::Initial,
            claim,
            manager.inventory_scope(),
        )
        .await
        .unwrap();

    (claim, confirmation)
}

fn prepare_restore_all_queue_fixture(
    manager: &RustCloudBackupManager,
    wallets: Vec<(WalletMetadata, cove_cspp::backup_data::WalletEntry)>,
) -> (String, Vec<CloudBackupWalletItem>) {
    let globals = test_globals();
    configure_enabled_cloud_backup(manager, globals, 0);
    let namespace = manager.current_namespace_id().unwrap();
    let master_key = cove_cspp::Cspp::new(Keychain::global().clone())
        .load_master_key_from_store()
        .unwrap()
        .unwrap();
    let mut record_ids = Vec::with_capacity(wallets.len());
    let mut items = Vec::with_capacity(wallets.len());

    for (metadata, entry) in wallets {
        let record_id = cove_cspp::backup_data::wallet_record_id(metadata.id.as_ref());
        globals.cloud.set_wallet_backup(
            namespace.clone(),
            record_id.clone(),
            encrypted_wallet_backup_bytes_for_entry(&entry, &master_key, 1),
        );
        record_ids.push(record_id.clone());
        items.push(CloudBackupWalletItem {
            name: metadata.name,
            network: Some(metadata.network),
            wallet_mode: Some(metadata.wallet_mode),
            wallet_type: Some(metadata.wallet_type),
            fingerprint: metadata
                .master_fingerprint
                .as_ref()
                .map(|fingerprint| fingerprint.as_uppercase()),
            label_count: Some(entry.labels_count),
            backup_updated_at: Some(entry.updated_at),
            sync_status: CloudBackupWalletStatus::DeletedFromDevice,
            restore_failure: None,
            record_id,
        });
    }
    globals.cloud.set_wallet_files(
        namespace.clone(),
        record_ids
            .iter()
            .map(|record_id| cove_cspp::backup_data::wallet_filename_from_record_id(record_id))
            .collect(),
    );
    manager.apply_detail_outcome(
        manager.inventory_scope(),
        CloudBackupDetailOutcome::refreshed(listed_detail(
            &record_ids.iter().map(String::as_str).collect::<Vec<_>>(),
        )),
    );
    manager.apply_cloud_only_fetch_outcome(
        manager.inventory_scope(),
        complete_loaded_outcome(manager, items.clone()),
    );
    assert!(matches!(
        manager.projected_restore_all_state(),
        CloudBackupRestoreAllState::StartAvailable { .. }
    ));

    (namespace, items)
}

impl CloudBackupSupervisor {
    /// Reads the journal and runs the recovery on it, as a relaunch does, returning its error
    /// instead of entering the interrupted-Enable recovery
    fn recover_pending_enable_after_restart(
        &mut self,
        manager: &RustCloudBackupManager,
    ) -> Result<(), CloudBackupError> {
        let journal = CloudBackupKeychain::global().load_pending_enable_journal().map_err(
            |source| CloudBackupError::internal_context("load pending enable", source),
        )?;

        self.recover_loaded_journal(manager, journal).map(|_| ())
    }
}

const TEST_UPLOAD_CREDENTIAL_ID: &[u8] = &[1, 2, 3];

fn test_enable_upload_finalization() -> (
    EnableUploadFinalization,
    cove_cspp::master_key::MasterKey,
) {
    let master_key = cove_cspp::master_key::MasterKey::generate();
    let namespace_id = master_key.namespace_id();
    let pending_completion = test_pending_completion(
        namespace_id,
        vec![PendingVerificationUpload::master_key_wrapper()],
    );

    (
        EnableUploadFinalization {
            context: CloudBackupEnableContext::settings_manual(),
            pending_completion,
        },
        master_key,
    )
}

fn prepare_test_enable_local_promotion(
    manager: &RustCloudBackupManager,
    finalization: &EnableUploadFinalization,
    master_key: &cove_cspp::master_key::MasterKey,
) {
    prepare_test_enable_local_promotion_with_hint(manager, finalization, master_key, None);
}

fn prepare_test_enable_local_promotion_with_hint(
    manager: &RustCloudBackupManager,
    finalization: &EnableUploadFinalization,
    master_key: &cove_cspp::master_key::MasterKey,
    provider_hint: Option<cove_cspp::backup_data::PasskeyProviderHint>,
) {
    let cloud_keychain = CloudBackupKeychain::global();
    let cspp = cove_cspp::Cspp::new(Keychain::global().clone());
    cspp.save_staged_master_key(master_key).unwrap();

    let mut journal = staged_pending_enable_journal(
        finalization.context,
        finalization.pending_completion.namespace_id().to_owned(),
        PendingEnableNamespaceOwnership::FreshOwned,
        cloud_keychain.snapshot_passkey_metadata().unwrap(),
    );
    let mut passkey = test_enable_passkey(TEST_UPLOAD_CREDENTIAL_ID.to_vec());
    passkey.provider_hint = provider_hint;
    assert!(journal.register_passkey(PendingEnablePasskeyMetadata {
        credential_id: passkey.credential_id.clone(),
        prf_salt: passkey.prf_salt,
        provider_hint: passkey.provider_hint.clone(),
    }));
    assert!(journal.mark_remote_writes_started());
    cloud_keychain.save_pending_enable_journal(&journal).unwrap();

    manager
        .pending_enable
        .begin_pending_enable_local_promotion(master_key, &passkey)
        .unwrap();
}

const TEST_RECOVERY_CREDENTIAL_ID: &[u8] = &[1, 2, 3];
const TEST_RECOVERY_PRF_SALT: [u8; 32] = [9; 32];

fn test_enable_recovery_finalization(namespace_id: &str) -> EnableRecoveryFinalization {
    EnableRecoveryFinalization {
        context: CloudBackupEnableContext::settings_manual(),
        namespace_id: namespace_id.into(),
        active_critical_key: zeroize::Zeroizing::new([0; 32]),
        pending_completion: test_pending_completion(
            namespace_id.into(),
            vec![PendingVerificationUpload::master_key_wrapper()],
        ),
        cleanup_sources: Vec::new(),
    }
}

fn prepare_test_enable_recovery_local_promotion(
    manager: &RustCloudBackupManager,
    finalization: &EnableRecoveryFinalization,
    master_key: &cove_cspp::master_key::MasterKey,
) {
    assert_eq!(finalization.namespace_id, master_key.namespace_id());
    let cloud_keychain = CloudBackupKeychain::global();
    let cspp = cove_cspp::Cspp::new(Keychain::global().clone());
    cspp.save_staged_master_key(master_key).unwrap();

    let mut journal = staged_pending_enable_journal(
        finalization.context,
        finalization.namespace_id.clone(),
        PendingEnableNamespaceOwnership::RecoveredExisting,
        cloud_keychain.snapshot_passkey_metadata().unwrap(),
    );
    let passkey = PendingEnablePasskeyMetadata {
        credential_id: TEST_RECOVERY_CREDENTIAL_ID.to_vec(),
        prf_salt: TEST_RECOVERY_PRF_SALT,
        provider_hint: None,
    };
    assert!(journal.register_passkey(passkey.clone()));
    assert!(journal.mark_remote_writes_started());
    cloud_keychain.save_pending_enable_journal(&journal).unwrap();

    manager
        .pending_enable
        .begin_enable_recovery_local_promotion(&finalization.namespace_id, passkey)
        .unwrap();
}

fn test_pending_completion(
    namespace_id: String,
    pending_uploads: Vec<PendingVerificationUpload>,
) -> PendingVerificationCompletion {
    PendingVerificationCompletion::new(
        DeepVerificationReport {
            master_key_wrapper_repaired: false,
            local_master_key_repaired: false,
            credential_recovered: false,
            wallets_verified: 0,
            wallet_issues: Default::default(),
            detail: None,
        },
        namespace_id,
        pending_uploads,
    )
}

fn emitted_enable_completed(manager: &RustCloudBackupManager) -> bool {
    manager.reconciler.receiver().try_iter().any(|messages| match messages {
        SingleOrMany::Single(message) => {
            matches!(message, CloudBackupReconcileMessage::EnableCompleted(_))
        }
        SingleOrMany::Many(messages) => messages
            .into_iter()
            .any(|message| matches!(message, CloudBackupReconcileMessage::EnableCompleted(_))),
    })
}

#[test]
fn enable_recovery_finalization_debug_redacts_nested_items() {
    let finalization = EnableRecoveryFinalization {
        context: CloudBackupEnableContext::settings_manual(),
        namespace_id: "active-namespace-secret".into(),
        active_critical_key: zeroize::Zeroizing::new([0; 32]),
        pending_completion: test_pending_completion(
            "active-namespace-secret".into(),
            vec![PendingVerificationUpload::new(
                "pending-wallet-record-secret".into(),
                "pending-wallet-revision-secret".into(),
            )],
        ),
        cleanup_sources: vec![CleanupSourceNamespace {
            namespace_id: "cleanup-namespace-secret".into(),
            expected_wallets: Vec::new(),
        }],
    };

    let debug = format!("{finalization:?}");

    assert!(debug.contains("pending_uploads_count: 1"), "{debug}");
    assert!(debug.contains("cleanup_sources_count: 1"), "{debug}");
    assert!(!debug.contains("active-namespace-secret"), "{debug}");
    assert!(!debug.contains("pending-wallet-record-secret"), "{debug}");
    assert!(!debug.contains("pending-wallet-revision-secret"), "{debug}");
    assert!(!debug.contains("cleanup-namespace-secret"), "{debug}");
}

#[test]
fn enable_recovery_completion_debug_redacts_nested_items() {
    let completion = CloudBackupEnableRecoveryCompletion {
        context: CloudBackupEnableContext::settings_manual(),
        namespace_id: "active-namespace-secret".into(),
        passkey: PendingEnablePasskeyMetadata {
            credential_id: vec![1, 2, 3],
            prf_salt: [9; 32],
            provider_hint: None,
        },
        active_critical_key: zeroize::Zeroizing::new([0; 32]),
        uploaded_wallets: vec![CloudBackupUploadedWallet::new(
            WalletId::from("uploaded-wallet-id-secret".to_string()),
            "uploaded-wallet-record-secret".into(),
            "uploaded-wallet-revision-secret".into(),
        )],
        pending_uploads: vec![PendingVerificationUpload::new(
            "pending-wallet-record-secret".into(),
            "pending-wallet-revision-secret".into(),
        )],
        cleanup_sources: vec![CleanupSourceNamespace {
            namespace_id: "cleanup-namespace-secret".into(),
            expected_wallets: Vec::new(),
        }],
    };

    let debug = format!("{completion:?}");

    assert!(debug.contains("uploaded_wallets_count: 1"), "{debug}");
    assert!(debug.contains("pending_uploads_count: 1"), "{debug}");
    assert!(debug.contains("cleanup_sources_count: 1"), "{debug}");
    assert!(debug.contains("credential_id: <redacted len=3>"), "{debug}");
    assert!(!debug.contains("active-namespace-secret"), "{debug}");
    assert!(!debug.contains("uploaded-wallet-id-secret"), "{debug}");
    assert!(!debug.contains("uploaded-wallet-record-secret"), "{debug}");
    assert!(!debug.contains("uploaded-wallet-revision-secret"), "{debug}");
    assert!(!debug.contains("pending-wallet-record-secret"), "{debug}");
    assert!(!debug.contains("pending-wallet-revision-secret"), "{debug}");
    assert!(!debug.contains("cleanup-namespace-secret"), "{debug}");
}

#[tokio::test(flavor = "current_thread")]
async fn supervisor_rejects_second_exclusive_operation_while_active() {
    let _guard = async_test_lock().lock().await;
    let manager = test_supervisor_manager();
    let mut supervisor = test_supervisor(
        Arc::downgrade(&manager),
        spawn_actor(CloudBackupWriteSupervisor::new(Weak::new())),
    );

    let first = supervisor
        .begin_exclusive_operation(&manager, CloudBackupExclusiveOperation::Enable)
        .unwrap();
    let second =
        supervisor.begin_exclusive_operation(&manager, CloudBackupExclusiveOperation::Restore);

    assert!(second.is_none());
    assert_eq!(supervisor.runtime, Some(first));
    assert_eq!(manager.projected_exclusive_operation(), Some(first));

    supervisor.complete_exclusive_operation(first).await.unwrap();

    assert_eq!(supervisor.runtime, None);
    assert_eq!(manager.projected_exclusive_operation(), None);
}

#[test]
fn local_reset_cancels_restore_and_revokes_its_claim() {
    let claim = CloudBackupExclusiveOperationClaim::new(
        CloudBackupExclusiveOperation::Restore,
        operation_id(u64::MAX),
    );
    let cancellation = Arc::new(AtomicBool::new(false));
    let mut active = Runtime::Disabled { op: None };
    active.start_restore(RestoreRun::new(claim, Arc::clone(&cancellation))).unwrap();

    let (ready, mut effects_settled) = oneshot::channel();
    let start = active.begin_reset(ready);

    // a restore is cancellable, so the reset neither waits for it nor lets it report again
    assert_eq!(start, ResetStart::Cancelled(claim));
    assert!(cancellation.load(Ordering::Acquire));
    assert!(effects_settled.try_recv().is_ok());
    assert!(!active.accepts_completion(claim, "restore completion"));
    assert!(active.restore(claim).is_none());
}

/// A cancelled restore keeps its claim while the worker drains, but its permit is revoked at the
/// cancel, so a passkey request the worker had not started yet can't present
#[test]
fn restore_cancellation_revokes_its_permit_and_keeps_its_claim() {
    let claim = CloudBackupExclusiveOperationClaim::new(
        CloudBackupExclusiveOperation::Restore,
        operation_id(u64::MAX),
    );
    let cancellation = Arc::new(AtomicBool::new(false));
    let mut active = Runtime::Disabled { op: None };
    active.start_restore(RestoreRun::new(claim, Arc::clone(&cancellation))).unwrap();
    active.bind_grant(claim, PermitGrant::new(tap(PermitIntent::RestoreFromCloudBackup)));
    let PasskeyRequestAuthority::Interactive(permit) = active.authority(claim) else {
        panic!("the restore holds its tap's permit");
    };
    assert!(permit.check_live().is_ok());

    assert!(active.cancel_restore(claim));

    assert!(cancellation.load(Ordering::Acquire));
    assert!(permit.check_live().is_err(), "the cancel revoked the permit");
    assert!(matches!(active.authority(claim), PasskeyRequestAuthority::Silent));
    assert!(active.accepts_completion(claim, "restore completion"));
    assert!(!active.cancel_restore(claim), "a second cancel is a no-op");
}

#[tokio::test(flavor = "current_thread")]
async fn drive_account_switch_holds_a_second_switch_and_an_unchanged_account_releases_claim() {
    use DriveAccountSwitchPlatformResult as R;

    let _guard = async_test_lock().lock().await;
    let manager = test_supervisor_manager();
    persist_enabled_for_supervisor_test();
    let mut supervisor = test_supervisor(
        Arc::downgrade(&manager),
        spawn_actor(CloudBackupWriteSupervisor::new(Arc::downgrade(&manager))),
    );

    supervisor.dispatch(UserIntent::SwitchDriveAccount).await.unwrap().await.unwrap();
    let transition = *RustCloudBackupManager::load_persisted_state()
        .drive_account_switch()
        .expect("the switch is recorded");
    let transition_id = transition.transition_id.value();
    let claim = supervisor.runtime.claim().expect("the switch runs");
    write_drain_completed(&mut supervisor, claim).await;

    assert_eq!(manager.drained_drive_account_switch_steps(), vec![RequestedDriveAccountSwitchStep::SelectAccount(transition_id)]);

    // a second Switch is held off and leaves the first one's record and claim
    supervisor.dispatch(UserIntent::SwitchDriveAccount).await.unwrap().await.unwrap();

    assert_eq!(RustCloudBackupManager::load_persisted_state().drive_account_switch(), Some(&transition));
    assert_eq!(supervisor.runtime.claim(), Some(claim));

    // the switch waits on the account picker, so a commit for another transition moves nothing
    platform_result(&mut supervisor, R::Committed(transition_id + 1)).await;

    assert_eq!(persisted_drive_account_switch_phase(), Some(PersistedDriveAccountSwitchPhase::AwaitingAccountSelection));
    assert_eq!(manager.state.read().drive_account_switch_recovery(), None);
    assert_eq!(manager.drained_drive_account_switch_steps(), vec![]);

    // the user backed out of the picker: the platform discards the staged account
    platform_result(&mut supervisor, R::AccountUnchanged(transition_id)).await;

    assert_eq!(manager.drained_drive_account_switch_steps(), vec![RequestedDriveAccountSwitchStep::Rollback(transition_id)]);

    platform_result(&mut supervisor, R::RolledBack(transition_id)).await;

    assert!(RustCloudBackupManager::load_persisted_state().drive_account_switch().is_none());
    assert_eq!(supervisor.runtime, None);
    assert_eq!(manager.projected_exclusive_operation(), None);
    assert_eq!(manager.state.read().drive_account_switch_recovery(), None);
}

#[tokio::test(flavor = "current_thread")]
async fn drive_account_switch_restart_without_staged_account_requests_rollback() {
    let _guard = async_test_lock().lock().await;
    let manager = test_supervisor_manager();
    persist_enabled_for_supervisor_test();
    let transition = PersistedDriveAccountSwitch {
        transition_id: 7.into(),
        phase: PersistedDriveAccountSwitchPhase::AwaitingAccountSelection,
    };
    let mut persisted = RustCloudBackupManager::load_persisted_state();
    assert!(persisted.set_drive_account_switch(transition));
    Database::global().cloud_backup_state.set(&persisted).unwrap();
    let mut supervisor = test_supervisor(
        Arc::downgrade(&manager),
        spawn_actor(CloudBackupWriteSupervisor::new(Arc::downgrade(&manager))),
    );

    platform_result(
        &mut supervisor,
        DriveAccountSwitchPlatformResult::State(DriveAccountSwitchPlatformState::NoTransition),
    )
    .await;

    assert_eq!(manager.drained_drive_account_switch_steps(), vec![RequestedDriveAccountSwitchStep::Rollback(7)]);
    assert_eq!(
        RustCloudBackupManager::load_persisted_state()
            .drive_account_switch()
            .map(|account_switch| account_switch.phase),
        Some(PersistedDriveAccountSwitchPhase::AwaitingAccountRollback)
    );
}

#[tokio::test(flavor = "current_thread")]
async fn drive_account_switch_restart_preserves_failed_reinitialization_for_retry() {
    let _guard = async_test_lock().lock().await;
    let manager = test_supervisor_manager();
    persist_enabled_for_supervisor_test();
    let transition = PersistedDriveAccountSwitch {
        transition_id: 7.into(),
        phase: PersistedDriveAccountSwitchPhase::AwaitingReinitializationRetry,
    };
    let mut persisted = RustCloudBackupManager::load_persisted_state();
    assert!(persisted.set_drive_account_switch(transition));
    Database::global().cloud_backup_state.set(&persisted).unwrap();
    let mut supervisor = test_supervisor(
        Arc::downgrade(&manager),
        spawn_actor(CloudBackupWriteSupervisor::new(Arc::downgrade(&manager))),
    );

    platform_result(
        &mut supervisor,
        DriveAccountSwitchPlatformResult::State(DriveAccountSwitchPlatformState::Staged(7)),
    )
    .await;

    // the failed reinitialize may ask for a passkey, so it waits for the user's Try Again
    assert_eq!(manager.drained_drive_account_switch_steps(), vec![]);
    assert_eq!(supervisor.runtime, None);
    assert!(manager.state.read().drive_account_switch_recovery().is_some());
    assert_eq!(
        RustCloudBackupManager::load_persisted_state().drive_account_switch(),
        Some(&transition)
    );
}

#[tokio::test(flavor = "current_thread")]
async fn a_recorded_drive_account_switch_admits_only_its_retry_and_cancel() {
    let _guard = async_test_lock().lock().await;
    let manager = test_supervisor_manager();
    persist_enabled_for_supervisor_test();
    let transition = PersistedDriveAccountSwitch {
        transition_id: 7.into(),
        phase: PersistedDriveAccountSwitchPhase::AwaitingReinitializationRetry,
    };
    let mut persisted = RustCloudBackupManager::load_persisted_state();
    assert!(persisted.set_drive_account_switch(transition));
    Database::global().cloud_backup_state.set(&persisted).unwrap();
    let mut supervisor = test_supervisor(
        Arc::downgrade(&manager),
        spawn_actor(CloudBackupWriteSupervisor::new(Arc::downgrade(&manager))),
    );

    let snapshot = supervisor.admission_snapshot(&manager, &manager.inventory_facts());
    let switch_holds = Admission::Rejected(Rejection::DriveAccountSwitch);
    assert_eq!(evaluate_admission(&UserIntent::TurnOff, &snapshot), switch_holds);
    assert_eq!(evaluate_admission(&UserIntent::SyncUnsynced, &snapshot), switch_holds);
    assert_eq!(evaluate_admission(&UserIntent::RecreateManifest, &snapshot), switch_holds);

    // the switch stopped with nothing running, so Try Again picks it up; its reinitialize may
    // have begun moving this device to the new backup, so Cancel can't bring the previous one back
    assert_eq!(
        evaluate_admission(&UserIntent::RetryDriveAccountSwitchRecovery, &snapshot),
        Admission::Accepted
    );
    assert_eq!(
        evaluate_admission(&UserIntent::CancelDriveAccountSwitch, &snapshot),
        Admission::Rejected(Rejection::NothingToCancel)
    );

    // the Turn Off review never opens, so nothing pauses beside the switch
    supervisor.dispatch(UserIntent::TurnOff).await.unwrap().await.unwrap();
    assert_eq!(supervisor.runtime.turn_off_admission(), TurnOffAdmission::None);

    // a second switch never replaces the recorded one
    supervisor.dispatch(UserIntent::SwitchDriveAccount).await.unwrap().await.unwrap();
    assert_eq!(RustCloudBackupManager::load_persisted_state().drive_account_switch(), Some(&transition));
    assert_eq!(supervisor.runtime, None);
}

#[tokio::test(flavor = "current_thread")]
async fn drive_account_switch_reconcile_does_not_restart_live_reinitialization() {
    let _guard = async_test_lock().lock().await;
    let manager = test_supervisor_manager();
    persist_enabled_for_supervisor_test();
    let transition = PersistedDriveAccountSwitch {
        transition_id: 7.into(),
        phase: PersistedDriveAccountSwitchPhase::Reinitializing,
    };
    let mut persisted = RustCloudBackupManager::load_persisted_state();
    assert!(persisted.set_drive_account_switch(transition));
    Database::global().cloud_backup_state.set(&persisted).unwrap();
    let mut supervisor = test_supervisor(
        Arc::downgrade(&manager),
        spawn_actor(CloudBackupWriteSupervisor::new(Arc::downgrade(&manager))),
    );
    let claim = CloudBackupExclusiveOperationClaim::drive_account_switch(
        transition.transition_id,
        OperationId::next(),
    );
    supervisor.runtime.start_standard(claim).expect("test runtime body admits the operation");
    manager.project_exclusive_operation_started(claim);

    platform_result(
        &mut supervisor,
        DriveAccountSwitchPlatformResult::State(DriveAccountSwitchPlatformState::Staged(7)),
    )
    .await;

    assert_eq!(manager.drained_drive_account_switch_steps(), vec![]);
    assert_eq!(supervisor.runtime.claim(), Some(claim));
    assert_eq!(
        RustCloudBackupManager::load_persisted_state().drive_account_switch(),
        Some(&transition)
    );
}

#[tokio::test(flavor = "current_thread")]
async fn stale_drive_account_switch_completion_cannot_finish_live_operation() {
    let _guard = async_test_lock().lock().await;
    let manager = test_supervisor_manager();
    persist_enabled_for_supervisor_test();
    let transition = PersistedDriveAccountSwitch {
        transition_id: 7.into(),
        phase: PersistedDriveAccountSwitchPhase::Reinitializing,
    };
    let mut persisted = RustCloudBackupManager::load_persisted_state();
    assert!(persisted.set_drive_account_switch(transition));
    Database::global().cloud_backup_state.set(&persisted).unwrap();
    let mut supervisor = test_supervisor(
        Arc::downgrade(&manager),
        spawn_actor(CloudBackupWriteSupervisor::new(Arc::downgrade(&manager))),
    );
    let claim = CloudBackupExclusiveOperationClaim::drive_account_switch(
        transition.transition_id,
        OperationId::next(),
    );
    supervisor.runtime.start_standard(claim).expect("test runtime body admits the operation");
    manager.project_exclusive_operation_started(claim);

    supervisor.finish_enable_operation(manager.clone(), claim);
    supervisor.finish_enable_operation(manager.clone(), claim);

    assert_eq!(supervisor.runtime.claim(), Some(claim));
    assert_eq!(manager.projected_exclusive_operation(), Some(claim));
    assert_eq!(
        RustCloudBackupManager::load_persisted_state()
            .drive_account_switch()
            .map(|account_switch| account_switch.phase),
        Some(PersistedDriveAccountSwitchPhase::AwaitingAccountCommitSucceeded)
    );
    let commit_requests = manager
        .reconciler
        .receiver()
        .try_iter()
        .map(|messages| match messages {
            SingleOrMany::Single(message) => usize::from(matches!(
                message,
                CloudBackupReconcileMessage::DriveAccountSwitchCommitRequired(7)
            )),
            SingleOrMany::Many(messages) => messages
                .into_iter()
                .filter(|message| {
                    matches!(
                        message,
                        CloudBackupReconcileMessage::DriveAccountSwitchCommitRequired(7)
                    )
                })
                .count(),
        })
        .sum::<usize>();
    assert_eq!(commit_requests, 1);
}

#[tokio::test(flavor = "current_thread")]
async fn failed_drive_account_reinitialization_does_not_request_account_commit() {
    let _guard = async_test_lock().lock().await;
    let manager = test_supervisor_manager();
    persist_enabled_for_supervisor_test();
    let transition = PersistedDriveAccountSwitch {
        transition_id: 7.into(),
        phase: PersistedDriveAccountSwitchPhase::Reinitializing,
    };
    let mut persisted = RustCloudBackupManager::load_persisted_state();
    assert!(persisted.set_drive_account_switch(transition));
    Database::global().cloud_backup_state.set(&persisted).unwrap();
    let mut supervisor = test_supervisor(
        Arc::downgrade(&manager),
        spawn_actor(CloudBackupWriteSupervisor::new(Arc::downgrade(&manager))),
    );
    let claim = CloudBackupExclusiveOperationClaim::drive_account_switch(
        transition.transition_id,
        OperationId::next(),
    );
    supervisor.runtime.start_standard(claim).expect("test runtime body admits the operation");
    manager.project_exclusive_operation_started(claim);

    assert_eq!(
        supervisor.drive_account_switch_reinitialization_finished(
            &manager,
            claim,
            DriveAccountSwitchReinitializationEnd::Failed,
        ),
        DriveAccountSwitchReinitializationCompletion::Handled
    );

    assert_eq!(
        RustCloudBackupManager::load_persisted_state()
            .drive_account_switch()
            .map(|account_switch| account_switch.phase),
        Some(PersistedDriveAccountSwitchPhase::AwaitingReinitializationRetry)
    );
    assert_eq!(supervisor.runtime, None);
    assert_eq!(manager.projected_exclusive_operation(), None);
    assert!(!manager.reconciler.receiver().try_iter().any(|messages| match messages {
        SingleOrMany::Single(message) => matches!(
            message,
            CloudBackupReconcileMessage::DriveAccountSwitchCommitRequired(7)
        ),
        SingleOrMany::Many(messages) => messages.into_iter().any(|message| matches!(
            message,
            CloudBackupReconcileMessage::DriveAccountSwitchCommitRequired(7)
        )),
    }));
}

#[tokio::test(flavor = "current_thread")]
async fn drive_account_switch_reports_busy_while_disable_is_persisted() {
    let _guard = async_test_lock().lock().await;
    let manager = test_supervisor_manager();
    let disabling = test_disabling_state();
    Database::global()
        .cloud_backup_state
        .set(&PersistedCloudBackupState::disabling_transition(disabling))
        .unwrap();
    let mut supervisor = test_supervisor(
        Arc::downgrade(&manager),
        spawn_actor(CloudBackupWriteSupervisor::new(Arc::downgrade(&manager))),
    );

    supervisor.dispatch(UserIntent::SwitchDriveAccount).await.unwrap().await.unwrap();

    assert!(RustCloudBackupManager::load_persisted_state().drive_account_switch().is_none());
    assert_eq!(manager.drained_drive_account_switch_steps(), vec![]);
    assert_eq!(supervisor.runtime, None);
    assert_eq!(manager.projected_exclusive_operation(), None);
}

#[tokio::test(flavor = "current_thread")]
async fn drive_account_switch_restart_finalizes_platform_only_commit() {
    let _guard = async_test_lock().lock().await;
    let manager = test_supervisor_manager();
    persist_enabled_for_supervisor_test();
    let mut supervisor = test_supervisor(
        Arc::downgrade(&manager),
        spawn_actor(CloudBackupWriteSupervisor::new(Arc::downgrade(&manager))),
    );

    platform_result(
        &mut supervisor,
        DriveAccountSwitchPlatformResult::State(DriveAccountSwitchPlatformState::Committed(7)),
    )
    .await;

    assert_eq!(manager.drained_drive_account_switch_steps(), vec![RequestedDriveAccountSwitchStep::Finalize(7)]);
}

#[tokio::test(flavor = "current_thread")]
async fn drive_account_switch_restart_keeps_fence_for_mismatched_platform_transition() {
    let _guard = async_test_lock().lock().await;
    let manager = test_supervisor_manager();
    persist_enabled_for_supervisor_test();
    let transition = PersistedDriveAccountSwitch {
        transition_id: 7.into(),
        phase: PersistedDriveAccountSwitchPhase::AwaitingAccountCommitSucceeded,
    };
    let mut persisted = RustCloudBackupManager::load_persisted_state();
    assert!(persisted.set_drive_account_switch(transition));
    Database::global().cloud_backup_state.set(&persisted).unwrap();
    let mut supervisor = test_supervisor(
        Arc::downgrade(&manager),
        spawn_actor(CloudBackupWriteSupervisor::new(Arc::downgrade(&manager))),
    );

    platform_result(
        &mut supervisor,
        DriveAccountSwitchPlatformResult::State(DriveAccountSwitchPlatformState::Committed(8)),
    )
    .await;

    assert_eq!(manager.drained_drive_account_switch_steps(), vec![]);
    assert_eq!(
        RustCloudBackupManager::load_persisted_state().drive_account_switch(),
        Some(&transition)
    );
    assert_eq!(
        supervisor.runtime.claim().and_then(CloudBackupExclusiveOperationClaim::drive_account_switch_id),
        None
    );

    // the manager's supervisor publishes the screen, after the recovery this one reported
    supervisor_checkpoint(&manager).await;
    let CloudBackupScreenBody::DriveAccountSwitch(screen) = manager.published_screen().body else {
        panic!("the recovery card is not on screen: {:?}", manager.published_screen().body);
    };
    assert!(
        matches!(screen.stage, CloudBackupDriveAccountSwitchStage::NeedsAttention(_)),
        "the switch does not ask for the user: {:?}",
        screen.stage
    );
}

fn begin_exclusive(
    operation: CloudBackupExclusiveOperation,
) -> impl FnOnce(&mut CloudBackupSupervisor, &Arc<RustCloudBackupManager>) -> CloudBackupExclusiveOperationClaim
{
    move |supervisor, manager| supervisor.begin_exclusive_operation(manager, operation).unwrap()
}

/// Starts an operation on a configured backup, which only a Configured record has a place for
fn begin_configured(
    operation: CloudBackupExclusiveOperation,
) -> impl FnOnce(&mut CloudBackupSupervisor, &Arc<RustCloudBackupManager>) -> CloudBackupExclusiveOperationClaim
{
    move |supervisor, manager| {
        persist_enabled_under(supervisor);
        supervisor.begin_exclusive_operation(manager, operation).unwrap()
    }
}

/// Starts a Try Again run of a committed Turn Off, which only a Disabling record has a place for
fn begin_finishing()
-> impl FnOnce(&mut CloudBackupSupervisor, &Arc<RustCloudBackupManager>) -> CloudBackupExclusiveOperationClaim
{
    |supervisor, manager| {
        let disabling = test_disabling_state();
        Database::global()
            .cloud_backup_state
            .set(&PersistedCloudBackupState::disabling_transition(disabling))
            .unwrap();
        supervisor.rebase_on_seeded_record();
        let finish = supervisor.runtime.start_finish(DeleteAuthority::TryAgain).unwrap();
        let claim = finish.run.claim;
        supervisor.begin_finish(manager, finish);
        claim
    }
}

/// The Try Again run `claim` names, as a completion of [`begin_finishing`]'s run carries it
fn try_again_run(claim: CloudBackupExclusiveOperationClaim) -> FinishRun {
    FinishRun { claim, start: FinishStart::Delete(DeleteAuthority::TryAgain) }
}

/// Replays one completion handler with a stale claim and then the live claim, so a handler that
/// skips the claim check clears the operation on the stale call and fails here
async fn assert_completion_ignores_stale_claim(
    begin: impl FnOnce(
        &mut CloudBackupSupervisor,
        &Arc<RustCloudBackupManager>,
    ) -> CloudBackupExclusiveOperationClaim,
    mut complete: impl AsyncFnMut(&mut CloudBackupSupervisor, CloudBackupExclusiveOperationClaim),
) {
    let _guard = async_test_lock().lock().await;
    let manager = test_supervisor_manager();
    let mut supervisor = test_supervisor(
        Arc::downgrade(&manager),
        spawn_actor(CloudBackupWriteSupervisor::new(Weak::new())),
    );

    let current = begin(&mut supervisor, &manager);
    let stale = CloudBackupExclusiveOperationClaim::new(current.operation(), operation_id(u64::MAX));

    complete(&mut supervisor, stale).await;

    assert_eq!(supervisor.runtime, Some(current));
    assert_eq!(manager.projected_exclusive_operation(), Some(current));

    complete(&mut supervisor, current).await;

    assert_eq!(supervisor.runtime, None);
    assert_eq!(manager.projected_exclusive_operation(), None);
}

#[tokio::test(flavor = "current_thread")]
async fn supervisor_ignores_stale_exclusive_operation_completion() {
    assert_completion_ignores_stale_claim(begin_exclusive(CloudBackupExclusiveOperation::Enable), async |supervisor, claim| {
            supervisor.complete_exclusive_operation(claim).await.unwrap();
    })
    .await;
}

#[tokio::test(flavor = "current_thread")]
async fn supervisor_ignores_stale_delete_cloud_wallet_completion() {
    assert_completion_ignores_stale_claim(begin_configured(CloudBackupExclusiveOperation::DeleteCloudWallet), async |supervisor, claim| {
            supervisor
                .complete_delete_cloud_wallet(claim, "wallet-record".into(), Err(CloudBackupError::Internal("completion".into())))
                .await
                .unwrap();
    })
    .await;
}

#[tokio::test(flavor = "current_thread")]
async fn supervisor_ignores_stale_restore_cloud_wallet_completion() {
    assert_completion_ignores_stale_claim(begin_configured(CloudBackupExclusiveOperation::RestoreCloudWallet), async |supervisor, claim| {
            let outcome = Err(CloudBackupError::Internal("completion".into()));
            supervisor
                .complete_restore_cloud_wallet(claim, "wallet-record".into(), 0, outcome)
                .await
                .unwrap();
    })
    .await;
}

#[tokio::test(flavor = "current_thread")]
async fn supervisor_ignores_stale_repair_passkey_wrapper_completion() {
    assert_completion_ignores_stale_claim(begin_configured(CloudBackupExclusiveOperation::RepairPasskey), async |supervisor, claim| {
            supervisor.complete_repair_passkey_wrapper(claim, Err(CloudBackupError::Internal("completion".into()))).await.unwrap();
    })
    .await;
}

#[tokio::test(flavor = "current_thread")]
async fn supervisor_ignores_stale_repair_passkey_finalization_completion() {
    assert_completion_ignores_stale_claim(begin_configured(CloudBackupExclusiveOperation::RepairPasskey), async |supervisor, claim| {
            let finalization = Err(CloudBackupError::Internal("completion".into()));
            supervisor.complete_repair_passkey_finalization(claim, finalization).await.unwrap();
    })
    .await;
}

#[tokio::test(flavor = "current_thread")]
async fn supervisor_ignores_stale_repair_passkey_wrapper_upload_completion() {
    assert_completion_ignores_stale_claim(begin_configured(CloudBackupExclusiveOperation::RepairPasskey), async |supervisor, claim| {
            let upload = Err(CloudBackupError::Internal("completion".into()));
            supervisor.complete_repair_passkey_wrapper_upload(claim, upload).await.unwrap();
    })
    .await;
}

#[tokio::test(flavor = "current_thread")]
async fn supervisor_ignores_stale_recreate_manifest_recovery_completion() {
    assert_completion_ignores_stale_claim(begin_configured(CloudBackupExclusiveOperation::RecreateManifest), async |supervisor, claim| {
            supervisor.complete_recreate_manifest_recovery(claim, Err(CloudBackupError::Internal("completion".into()))).await.unwrap();
    })
    .await;
}

#[tokio::test(flavor = "current_thread")]
async fn supervisor_ignores_stale_recreate_manifest_finalization_completion() {
    assert_completion_ignores_stale_claim(begin_configured(CloudBackupExclusiveOperation::RecreateManifest), async |supervisor, claim| {
            supervisor.complete_recreate_manifest_finalization(claim, Err(CloudBackupError::Internal("completion".into()))).await.unwrap();
    })
    .await;
}

#[tokio::test(flavor = "current_thread")]
async fn supervisor_ignores_stale_recreate_manifest_verification_completion() {
    assert_completion_ignores_stale_claim(begin_configured(CloudBackupExclusiveOperation::RecreateManifest), async |supervisor, claim| {
            supervisor
                .complete_verification(
                    VerificationRun {
                        claim,
                        continuation: DeepVerificationContinuation::RecreateManifest,
                        basis: supervisor.evidence_basis(),
                        read_at: InventoryReadAt::default(),
                    },
                    CloudBackupDeepVerificationStep::Complete(DeepVerificationResult::NotEnabled),
                )
                .await
                .unwrap();
    })
    .await;
}

#[tokio::test(flavor = "current_thread")]
async fn supervisor_ignores_stale_deep_verification_wrapper_repair_upload_completion() {
    assert_completion_ignores_stale_claim(begin_configured(CloudBackupExclusiveOperation::Verify), async |supervisor, claim| {
            let continuation = DeepVerificationContinuation::Manual { force_discoverable: true };
            supervisor
                .complete_deep_verification_wrapper_repair_upload(VerificationRun { claim, continuation, basis: supervisor.evidence_basis(), read_at: InventoryReadAt::default() }, Err(CloudBackupError::Internal("completion".into())))
                .await
                .unwrap();
    })
    .await;
}

#[tokio::test(flavor = "current_thread")]
async fn supervisor_ignores_stale_deep_verification_wrapper_repair_resume_completion() {
    assert_completion_ignores_stale_claim(begin_configured(CloudBackupExclusiveOperation::Verify), async |supervisor, claim| {
            let continuation = DeepVerificationContinuation::Manual { force_discoverable: true };
            supervisor
                .complete_deep_verification_wrapper_repair_resume(
                    VerificationRun { claim, continuation, basis: supervisor.evidence_basis(), read_at: InventoryReadAt::default() },
                    CloudBackupDeepVerificationStep::Complete(DeepVerificationResult::NotEnabled),
                )
                .await
                .unwrap();
    })
    .await;
}

#[tokio::test(flavor = "current_thread")]
async fn supervisor_ignores_stale_deep_verification_auto_sync_upload_completion() {
    assert_completion_ignores_stale_claim(begin_configured(CloudBackupExclusiveOperation::Verify), async |supervisor, claim| {
            let continuation = DeepVerificationContinuation::Manual { force_discoverable: true };
            supervisor
                .complete_deep_verification_auto_sync_upload(
                    VerificationRun { claim, continuation, basis: supervisor.evidence_basis(), read_at: InventoryReadAt::default() },
                    Err(DeepVerificationResult::NotEnabled),
                )
                .await
                .unwrap();
    })
    .await;
}

#[tokio::test(flavor = "current_thread")]
async fn supervisor_ignores_stale_deep_verification_auto_sync_finalization_completion() {
    assert_completion_ignores_stale_claim(begin_configured(CloudBackupExclusiveOperation::Verify), async |supervisor, claim| {
            let continuation = DeepVerificationContinuation::Manual { force_discoverable: true };
            supervisor
                .complete_deep_verification_auto_sync_finalization(
                    VerificationRun { claim, continuation, basis: supervisor.evidence_basis(), read_at: InventoryReadAt::default() },
                    Err(DeepVerificationResult::NotEnabled),
                )
                .await
                .unwrap();
    })
    .await;
}

#[tokio::test(flavor = "current_thread")]
async fn supervisor_ignores_stale_deep_verification_auto_sync_resume_completion() {
    assert_completion_ignores_stale_claim(begin_configured(CloudBackupExclusiveOperation::Verify), async |supervisor, claim| {
            let continuation = DeepVerificationContinuation::Manual { force_discoverable: true };
            supervisor
                .complete_deep_verification_auto_sync_resume(
                    VerificationRun { claim, continuation, basis: supervisor.evidence_basis(), read_at: InventoryReadAt::default() },
                    CloudBackupDeepVerificationAutoSyncCompletion::complete(
                        DeepVerificationResult::NotEnabled,
                    ),
                )
                .await
                .unwrap();
    })
    .await;
}

#[tokio::test(flavor = "current_thread")]
async fn supervisor_ignores_stale_reinitialize_backup_completion() {
    assert_completion_ignores_stale_claim(begin_configured(CloudBackupExclusiveOperation::ReinitializeBackup), async |supervisor, claim| {
            supervisor.complete_enable_preparation(claim, Err(CloudBackupError::Internal("completion".into()))).await.unwrap();
    })
    .await;
}

#[tokio::test(flavor = "current_thread")]
async fn supervisor_ignores_stale_reinitialize_verification_completion() {
    assert_completion_ignores_stale_claim(begin_configured(CloudBackupExclusiveOperation::VerifyReinitializedBackup), async |supervisor, claim| {
            supervisor
                .complete_verification(
                    VerificationRun {
                        claim,
                        continuation: DeepVerificationContinuation::ReinitializeBackup,
                        basis: supervisor.evidence_basis(),
                        read_at: InventoryReadAt::default(),
                    },
                    CloudBackupDeepVerificationStep::Complete(DeepVerificationResult::NotEnabled),
                )
                .await
                .unwrap();
    })
    .await;
}

#[tokio::test(flavor = "current_thread")]
async fn supervisor_ignores_stale_saved_passkey_confirmation_completion() {
    assert_completion_ignores_stale_claim(begin_exclusive(CloudBackupExclusiveOperation::Enable), async |supervisor, claim| {
            supervisor
                .complete_saved_passkey_confirmation(
                    claim,
                    CloudBackupSavedPasskeyConfirmation::Failed(CloudBackupError::Internal(
                        "completion".into(),
                    )),
                )
                .await
                .unwrap();
    })
    .await;
}

#[tokio::test(flavor = "current_thread")]
async fn supervisor_ignores_stale_enable_passkey_registration_completion() {
    assert_completion_ignores_stale_claim(begin_exclusive(CloudBackupExclusiveOperation::EnableForceNew), async |supervisor, claim| {
            supervisor
                .complete_enable_passkey_registration(
                    claim,
                    Ok(CloudBackupEnablePasskeyRegistration::Cancelled { context: CloudBackupEnableContext::settings_manual() }),
                )
                .await
                .unwrap();
    })
    .await;
}

#[tokio::test(flavor = "current_thread")]
async fn supervisor_ignores_stale_enable_preparation_completion() {
    assert_completion_ignores_stale_claim(begin_exclusive(CloudBackupExclusiveOperation::Enable), async |supervisor, claim| {
            supervisor
                .complete_enable_preparation(
                    claim,
                    Ok(CloudBackupEnablePreparation::ExistingBackupFound {
                        context: CloudBackupEnableContext::settings_manual(),
                        passkey_hint: None,
                    }),
                )
                .await
                .unwrap();
    })
    .await;
}

#[tokio::test(flavor = "current_thread")]
async fn supervisor_ignores_stale_create_new_enable_passkey_completion() {
    assert_completion_ignores_stale_claim(begin_exclusive(CloudBackupExclusiveOperation::Enable), async |supervisor, claim| {
            supervisor
                .complete_create_new_enable_passkey(
                    claim,
                    Ok(CloudBackupEnablePasskeyPreparation::Cancelled { context: CloudBackupEnableContext::settings_manual() }),
                )
                .await
                .unwrap();
    })
    .await;
}

#[tokio::test(flavor = "current_thread")]
async fn supervisor_ignores_stale_enable_recovery_completion() {
    assert_completion_ignores_stale_claim(begin_exclusive(CloudBackupExclusiveOperation::Enable), async |supervisor, claim| {
            supervisor.complete_enable_recovery(claim, Err(CloudBackupError::Internal("completion".into()))).await.unwrap();
    })
    .await;
}

#[tokio::test(flavor = "current_thread")]
async fn supervisor_ignores_stale_enable_recovery_finalization_completion() {
    assert_completion_ignores_stale_claim(begin_exclusive(CloudBackupExclusiveOperation::Enable), async |supervisor, claim| {
            let namespace_id: String = "current-namespace".into();
            supervisor
                .complete_enable_recovery_finalization(
                    claim,
                    EnableRecoveryFinalization {
                        context: CloudBackupEnableContext::settings_manual(),
                        namespace_id: namespace_id.clone(),
                        active_critical_key: zeroize::Zeroizing::new([0; 32]),
                        pending_completion: test_pending_completion(namespace_id, Vec::new()),
                        cleanup_sources: Vec::new(),
                    },
                    Err(CloudBackupError::Internal("completion".into())),
                )
                .await
                .unwrap();
    })
    .await;
}

#[tokio::test(flavor = "current_thread")]
async fn supervisor_ignores_stale_enable_recovery_preparation_completion() {
    assert_completion_ignores_stale_claim(begin_exclusive(CloudBackupExclusiveOperation::Enable), async |supervisor, claim| {
            supervisor.complete_enable_recovery_preparation(claim, Err(CloudBackupError::Internal("completion".into()))).await.unwrap();
    })
    .await;
}

#[tokio::test(flavor = "current_thread")]
async fn supervisor_ignores_stale_no_discovery_enable_preparation_completion() {
    assert_completion_ignores_stale_claim(begin_exclusive(CloudBackupExclusiveOperation::EnableNoDiscovery), async |supervisor, claim| {
            supervisor
                .complete_no_discovery_enable_preparation(
                    claim,
                    Ok(CloudBackupNoDiscoveryEnablePreparation::ExistingBackupFound {
                        context: CloudBackupEnableContext::settings_manual(),
                        passkey_hint: None,
                    }),
                )
                .await
                .unwrap();
    })
    .await;
}

#[tokio::test(flavor = "current_thread")]
async fn supervisor_ignores_stale_enable_upload_completion() {
    assert_completion_ignores_stale_claim(begin_exclusive(CloudBackupExclusiveOperation::Enable), async |supervisor, claim| {
            supervisor.complete_enable_upload(claim, Err(CloudBackupError::Internal("completion".into()))).await.unwrap();
    })
    .await;
}

#[tokio::test(flavor = "current_thread")]
async fn supervisor_ignores_stale_enable_upload_finalization_completion() {
    assert_completion_ignores_stale_claim(begin_exclusive(CloudBackupExclusiveOperation::Enable), async |supervisor, claim| {
            supervisor
                .complete_enable_upload_finalization(
                    claim,
                    test_enable_upload_finalization().0,
                    Err(CloudBackupError::Internal("completion".into())),
                )
                .await
                .unwrap();
    })
    .await;
}

#[tokio::test(flavor = "current_thread")]
async fn supervisor_ignores_stale_turn_off_drain_completion() {
    assert_completion_ignores_stale_claim(begin_configured(CloudBackupExclusiveOperation::Disable), async |supervisor, claim| {
            let review = empty_review(VALID_NAMESPACE_ID);
            supervisor.complete_turn_off_drain(claim, review, Err(CloudBackupError::Internal("completion".into()))).await.unwrap();
    })
    .await;
}

#[tokio::test(flavor = "current_thread")]
async fn supervisor_ignores_stale_turn_off_recheck_completion() {
    assert_completion_ignores_stale_claim(begin_configured(CloudBackupExclusiveOperation::Disable), async |supervisor, claim| {
            let review = empty_review(VALID_NAMESPACE_ID);
            supervisor
                .complete_turn_off_recheck(claim, review, Err(EvidenceUnavailable::Offline))
                .await
                .unwrap();
    })
    .await;
}

#[tokio::test(flavor = "current_thread")]
async fn supervisor_ignores_stale_disable_delete_completion() {
    assert_completion_ignores_stale_claim(begin_finishing(), async |supervisor, claim| {
            supervisor
                .complete_disable_namespace_delete(try_again_run(claim), test_committed_disable(), Err(CloudBackupError::Internal("completion".into())))
                .await
                .unwrap();
    })
    .await;
}

#[tokio::test(flavor = "current_thread")]
async fn supervisor_ignores_stale_disable_local_cleanup_completion() {
    assert_completion_ignores_stale_claim(begin_finishing(), async |supervisor, claim| {
            supervisor
                .complete_disable_local_cleanup(try_again_run(claim), test_committed_disable(), Err(CloudBackupError::Internal("completion".into())))
                .await
                .unwrap();
    })
    .await;
}

#[tokio::test(flavor = "current_thread")]
async fn supervisor_ignores_stale_recover_other_backups_completion() {
    assert_completion_ignores_stale_claim(|supervisor, manager| {
            persist_enabled_under(supervisor);
            CloudBackupSupervisor::begin_other_backups_operation(
                supervisor,
                manager,
                CloudBackupExclusiveOperation::RecoverOtherBackups,
                CloudBackupOtherBackupsOutcome::Recovering,
            )
            .unwrap()
        }, async |supervisor, claim| {
            supervisor.complete_recover_other_backups(claim, Err(CloudBackupError::Internal("completion".into()))).await.unwrap();
    })
    .await;
}

#[tokio::test(flavor = "current_thread")]
async fn supervisor_ignores_stale_delete_other_backups_completion() {
    assert_completion_ignores_stale_claim(|supervisor, manager| {
            persist_enabled_under(supervisor);
            CloudBackupSupervisor::begin_other_backups_operation(
                supervisor,
                manager,
                CloudBackupExclusiveOperation::DeleteOtherBackups,
                CloudBackupOtherBackupsOutcome::Deleting,
            )
            .unwrap()
        }, async |supervisor, claim| {
            supervisor.complete_delete_other_backups(claim, Err(CloudBackupError::Internal("completion".into()))).await.unwrap();
    })
    .await;
}







#[tokio::test(flavor = "current_thread")]
async fn supervisor_ignores_stale_repair_passkey_refresh_completion() {
    let _guard = async_test_lock().lock().await;
    let manager = test_supervisor_manager();
    let mut supervisor = test_supervisor(
        Arc::downgrade(&manager),
        spawn_actor(CloudBackupWriteSupervisor::new(Weak::new())),
    );

    persist_enabled_under(&mut supervisor);
    let current = supervisor
        .begin_exclusive_operation(&manager, CloudBackupExclusiveOperation::RepairPasskey)
        .unwrap();
    let stale = CloudBackupExclusiveOperationClaim::new(
        CloudBackupExclusiveOperation::RepairPasskey,
        operation_id(u64::MAX),
    );

    let detail_claim = supervisor.detail_workflow.start_operation_result(manager.inventory_scope());
    supervisor
        .complete_repair_passkey_refresh_detail(stale, detail_claim, None)
        .await
        .unwrap();

    assert_eq!(supervisor.runtime, Some(current));
    assert_eq!(manager.projected_exclusive_operation(), Some(current));

    supervisor
        .complete_repair_passkey_refresh_detail(current, detail_claim, None)
        .await
        .unwrap();

    assert_eq!(supervisor.runtime, None);
    assert_eq!(manager.projected_exclusive_operation(), None);
}

#[tokio::test(flavor = "current_thread")]
async fn repair_passkey_refresh_failure_resolves_superseded_detail_refresh() {
    let _guard = async_test_lock().lock().await;
    let manager = test_supervisor_manager();
    configure_enabled_cloud_backup(&manager, test_globals(), 0);
    let mut supervisor = test_supervisor(
        Arc::downgrade(&manager),
        spawn_actor(CloudBackupWriteSupervisor::new(Weak::new())),
    );

    supervisor.detail_workflow.open();
    let DetailRefreshPlan::Start(superseded_refresh) =
        supervisor.detail_workflow.request_refresh(DetailRefreshOrigin::Tap)
    else {
        panic!("expected detail refresh to start");
    };
    manager.apply_detail_outcome(manager.inventory_scope(), CloudBackupDetailOutcome::Checking);

    let operation_claim = supervisor
        .begin_exclusive_operation(&manager, CloudBackupExclusiveOperation::RepairPasskey)
        .unwrap();
    let repair_detail_claim =
        supervisor.detail_workflow.start_operation_result(manager.inventory_scope());
    supervisor
        .complete_repair_passkey_refresh_detail(
            operation_claim,
            repair_detail_claim,
            Some(CloudBackupDetailResult::AccessError(CloudBackupError::Offline(
                "offline".into(),
            ))),
        )
        .await
        .unwrap();

    let configured = manager.test_inventory();
    assert!(matches!(
        configured.detail,
        CloudBackupDetailState::Failed {
            reason: CloudBackupInventoryIncompleteReason::Offline,
            ..
        }
    ));

    supervisor
        .complete_refresh_detail(
            Some(CloudBackupDetailResult::SuccessWithAuthority {
                listed: listed_detail(&[]),
                authority: CloudBackupInventoryAuthority::ProviderConfirmed,
            }),
            DetailRefreshAttempt::Initial,
            superseded_refresh,
            manager.inventory_scope(),
        )
        .await
        .unwrap();

    let configured = manager.test_inventory();
    assert!(matches!(configured.detail, CloudBackupDetailState::Failed { .. }));
}

#[tokio::test(flavor = "current_thread")]
async fn trusted_local_detail_stays_active_until_provider_confirmation() {
    let _guard = async_test_lock().lock().await;
    let manager = test_supervisor_manager();
    configure_enabled_cloud_backup(&manager, test_globals(), 2);
    let mut supervisor = test_supervisor(
        Arc::downgrade(&manager),
        spawn_actor(CloudBackupWriteSupervisor::new(Weak::new())),
    );
    let listed = listed_detail(&["first", "second"]);
    let (claim, confirmation) =
        start_trusted_local_refresh(&mut supervisor, &manager, listed.clone()).await;

    assert!(supervisor.detail_workflow.is_refresh_active(claim));

    let wallets = vec![test_cloud_only_wallet("first"), test_cloud_only_wallet("second")];
    manager.apply_cloud_only_fetch_outcome(
        manager.inventory_scope(),
        complete_loaded_outcome(&manager, wallets.clone()),
    );
    assert_eq!(
        manager.projected_restore_all_state(),
        CloudBackupRestoreAllState::StartDisabled { wallet_count: 2 },
    );

    supervisor
        .complete_confirm_refresh_detail(
            confirmation,
            Some(CloudBackupDetailResult::SuccessWithAuthority {
                listed,
                authority: CloudBackupInventoryAuthority::ProviderConfirmed,
            }),
            DetailRefreshAttempt::Initial,
            claim,
            manager.inventory_scope(),
        )
        .await
        .unwrap();

    assert!(!supervisor.detail_workflow.is_refresh_active(claim));
    assert_eq!(
        manager.projected_restore_all_state(),
        CloudBackupRestoreAllState::StartAvailable { wallet_count: 2 },
    );

    let configured = manager.test_inventory();
    let CloudBackupDetailState::Complete { state } = configured.detail else {
        panic!("expected complete cloud backup detail");
    };

    assert_eq!(state.inventory_authority, CloudBackupInventoryAuthority::ProviderConfirmed);
    assert_eq!(state.cloud_only, CloudOnlyState::Loaded {
            rows: listed_rows(&wallets),
            incomplete: false,
            namespace_id: crate::manager::cloud_backup_manager::owner::inventory::test_support::LISTED_NAMESPACE.into(),
        });
}

#[tokio::test(flavor = "current_thread")]
async fn provider_confirmation_failure_marks_trusted_local_detail_incomplete() {
    let _guard = async_test_lock().lock().await;
    let manager = test_supervisor_manager();
    configure_enabled_cloud_backup(&manager, test_globals(), 2);
    let mut supervisor = test_supervisor(
        Arc::downgrade(&manager),
        spawn_actor(CloudBackupWriteSupervisor::new(Weak::new())),
    );
    let (claim, confirmation) =
        start_trusted_local_refresh(&mut supervisor, &manager, listed_detail(&[])).await;

    supervisor
        .complete_confirm_refresh_detail(
            confirmation,
            Some(CloudBackupDetailResult::AccessError(CloudBackupError::Internal(
                "metadata timed out".into(),
            ))),
            DetailRefreshAttempt::Initial,
            claim,
            manager.inventory_scope(),
        )
        .await
        .unwrap();

    assert!(!supervisor.detail_workflow.is_refresh_active(claim));
    assert!(matches!(
        manager.test_inventory(),
        ref configured
            if matches!(
                configured.detail,
                CloudBackupDetailState::Failed { retained: Some(ref state), .. }
                    if state.inventory_authority
                        == CloudBackupInventoryAuthority::LocalSnapshotMatchesKnownCount
            )
    ));
}

#[tokio::test(flavor = "current_thread")]
async fn provider_confirmation_connectivity_failure_retries_once() {
    let _guard = async_test_lock().lock().await;
    let manager = test_supervisor_manager();
    configure_enabled_cloud_backup(&manager, test_globals(), 2);
    let mut supervisor = test_supervisor(
        Arc::downgrade(&manager),
        spawn_actor(CloudBackupWriteSupervisor::new(Weak::new())),
    );
    let (claim, confirmation) =
        start_trusted_local_refresh(&mut supervisor, &manager, listed_detail(&[])).await;

    supervisor
        .complete_confirm_refresh_detail(
            confirmation.clone(),
            Some(CloudBackupDetailResult::AccessError(CloudBackupError::Offline(
                "offline".into(),
            ))),
            DetailRefreshAttempt::Initial,
            claim,
            manager.inventory_scope(),
        )
        .await
        .unwrap();

    assert!(supervisor.detail_workflow.is_refresh_active(claim));

    supervisor
        .complete_confirm_refresh_detail(
            confirmation,
            Some(CloudBackupDetailResult::AccessError(CloudBackupError::Offline(
                "still offline".into(),
            ))),
            DetailRefreshAttempt::AutomaticConnectivityRetry,
            claim,
            manager.inventory_scope(),
        )
        .await
        .unwrap();

    assert!(!supervisor.detail_workflow.is_refresh_active(claim));
    assert!(matches!(
        manager.test_inventory(),
        ref configured
            if matches!(
                configured.detail,
                CloudBackupDetailState::Failed { retained: Some(ref state), .. }
                    if state.inventory_authority
                        == CloudBackupInventoryAuthority::LocalSnapshotMatchesKnownCount
            )
    ));
}

#[tokio::test(flavor = "current_thread")]
async fn trailing_detail_refresh_is_scheduled_after_provider_confirmation() {
    let _guard = async_test_lock().lock().await;
    let manager = test_supervisor_manager();
    configure_enabled_cloud_backup(&manager, test_globals(), 2);
    let mut supervisor = test_supervisor(
        Arc::downgrade(&manager),
        spawn_actor(CloudBackupWriteSupervisor::new(Weak::new())),
    );
    let (claim, confirmation) =
        start_trusted_local_refresh(&mut supervisor, &manager, listed_detail(&[])).await;

    assert_eq!(supervisor.detail_workflow.request_refresh(DetailRefreshOrigin::Tap), DetailRefreshPlan::Queued);

    supervisor
        .complete_confirm_refresh_detail(
            confirmation,
            Some(CloudBackupDetailResult::SuccessWithAuthority {
                listed: listed_detail(&[]),
                authority: CloudBackupInventoryAuthority::ProviderConfirmed,
            }),
            DetailRefreshAttempt::Initial,
            claim,
            manager.inventory_scope(),
        )
        .await
        .unwrap();

    assert!(!supervisor.detail_workflow.is_refresh_active(claim));
    assert_eq!(supervisor.detail_workflow.request_refresh(DetailRefreshOrigin::Tap), DetailRefreshPlan::Queued);
}

#[tokio::test(flavor = "current_thread")]
async fn stale_provider_confirmation_does_not_overwrite_newer_operation_result() {
    let _guard = async_test_lock().lock().await;
    let manager = test_supervisor_manager();
    configure_enabled_cloud_backup(&manager, test_globals(), 2);
    let mut supervisor = test_supervisor(
        Arc::downgrade(&manager),
        spawn_actor(CloudBackupWriteSupervisor::new(Weak::new())),
    );
    let (claim, confirmation) =
        start_trusted_local_refresh(&mut supervisor, &manager, listed_detail(&[])).await;

    supervisor.detail_workflow.start_operation_result(manager.inventory_scope());
    let newer = listed_detail(&["only"]);
    let newer_detail = newer.detail.clone();
    manager.apply_detail_outcome(
        manager.inventory_scope(),
        CloudBackupDetailOutcome::refreshed(newer),
    );

    supervisor
        .complete_confirm_refresh_detail(
            confirmation,
            Some(CloudBackupDetailResult::SuccessWithAuthority {
                listed: listed_detail(&[]),
                authority: CloudBackupInventoryAuthority::ProviderConfirmed,
            }),
            DetailRefreshAttempt::Initial,
            claim,
            manager.inventory_scope(),
        )
        .await
        .unwrap();

    assert!(!supervisor.detail_workflow.is_refresh_active(claim));
    assert_eq!(manager.model_snapshot().detail, Some(newer_detail));
}



#[tokio::test(flavor = "current_thread")]
async fn supervisor_ignores_stale_sync_request_completion() {
    let _guard = async_test_lock().lock().await;
    let manager = test_supervisor_manager();
    let mut supervisor = test_supervisor(
        Arc::downgrade(&manager),
        spawn_actor(CloudBackupWriteSupervisor::new(Weak::new())),
    );
    supervisor.active_sync_request = Some(7);

    supervisor
        .complete_sync_request(
            6,
            SyncRequestOutcome::Failed(CloudBackupError::Internal("stale completion".into())),
        )
        .await
        .unwrap();

    assert_eq!(supervisor.active_sync_request, Some(7));

    supervisor
        .complete_sync_request(
            7,
            SyncRequestOutcome::Failed(CloudBackupError::Internal("current completion".into())),
        )
        .await
        .unwrap();

    assert_eq!(supervisor.active_sync_request, None);
}

#[tokio::test(flavor = "current_thread")]
async fn supervisor_ignores_stale_sync_refresh_completion() {
    let _guard = async_test_lock().lock().await;
    let manager = test_supervisor_manager();
    let mut supervisor = test_supervisor(
        Arc::downgrade(&manager),
        spawn_actor(CloudBackupWriteSupervisor::new(Weak::new())),
    );
    supervisor.active_sync_request = Some(7);
    let detail_claim = supervisor.detail_workflow.start_operation_result(manager.inventory_scope());

    supervisor
        .complete_sync_request_refresh_detail(6, detail_claim, None)
        .await
        .unwrap();

    assert_eq!(supervisor.active_sync_request, Some(7));

    supervisor
        .complete_sync_request_refresh_detail(7, detail_claim, None)
        .await
        .unwrap();

    assert_eq!(supervisor.active_sync_request, None);
}

#[tokio::test(flavor = "current_thread")]
async fn supervisor_ignores_stale_cloud_only_fetch_completion() {
    let _guard = async_test_lock().lock().await;
    let manager = test_supervisor_manager();
    let mut supervisor = test_supervisor(
        Arc::downgrade(&manager),
        spawn_actor(CloudBackupWriteSupervisor::new(Weak::new())),
    );
    supervisor.active_cloud_only_fetch_request = Some(7);

    supervisor
        .complete_cloud_only_fetch_request(
            6,
            manager.inventory_scope(),
            Ok(complete_cloud_only_fetch(&manager, Vec::new())),
        )
        .await
        .unwrap();

    assert_eq!(supervisor.active_cloud_only_fetch_request, Some(7));

    supervisor
        .complete_cloud_only_fetch_request(
            7,
            manager.inventory_scope(),
            Ok(complete_cloud_only_fetch(&manager, Vec::new())),
        )
        .await
        .unwrap();

    assert_eq!(supervisor.active_cloud_only_fetch_request, None);
}















#[tokio::test(flavor = "current_thread")]
async fn enable_recovery_finalization_requires_durable_pending_completion_before_success() {
    let _guard = async_test_lock().lock().await;
    let manager = test_supervisor_manager();
    let master_key = cove_cspp::master_key::MasterKey::generate();
    let namespace_id = master_key.namespace_id();
    let finalization = test_enable_recovery_finalization(&namespace_id);
    let mut supervisor = test_supervisor(
        Arc::downgrade(&manager),
        spawn_actor(CloudBackupWriteSupervisor::new(Weak::new())),
    );
    let context = CloudBackupEnableContext::settings_manual();
    manager.project_enable_context_started(context);
    let claim = supervisor
        .begin_exclusive_operation(&manager, CloudBackupExclusiveOperation::Enable)
        .unwrap();
    // the Enable worker writes the Configured record while its operation runs
    Database::global()
        .cloud_backup_state
        .set(&persisted_enabled_cloud_backup_state(Some(0)))
        .unwrap();
    prepare_test_enable_recovery_local_promotion(&manager, &finalization, &master_key);

    supervisor
        .complete_enable_recovery_finalization(claim, finalization, Ok(()))
        .await
        .unwrap();

    assert!(matches!(manager.model_snapshot().status, CloudBackupStatus::Error(_)));
    assert!(manager.pending_verification_completion().is_none());
    assert!(!emitted_enable_completed(&manager));
    assert_eq!(supervisor.runtime, None);
    let journal = CloudBackupKeychain::global().load_pending_enable_journal().unwrap().unwrap();
    assert!(matches!(journal.phase(), PendingEnableJournalPhase::LocalPromotionStarted(_)));
    assert_eq!(journal.namespace_ownership(), PendingEnableNamespaceOwnership::RecoveredExisting);
}

#[tokio::test(flavor = "current_thread")]
async fn enable_recovery_finalization_failure_rolls_the_recovered_backup_all_the_way_back() {
    let _guard = async_test_lock().lock().await;
    let manager = test_supervisor_manager();
    let cspp = cove_cspp::Cspp::new(Keychain::global().clone());
    let prior_master_key = cove_cspp::master_key::MasterKey::generate();
    let prior_master_key_bytes = *prior_master_key.as_bytes();
    cspp.save_master_key(&prior_master_key).unwrap();
    let cloud_keychain = CloudBackupKeychain::global();
    cloud_keychain.save_passkey_and_namespace(&[7, 8, 9], [8; 32], "prior-namespace").unwrap();
    let prior_metadata = cloud_keychain.snapshot_passkey_metadata().unwrap();

    let recovered_master_key = cove_cspp::master_key::MasterKey::generate();
    let finalization =
        test_enable_recovery_finalization(&recovered_master_key.namespace_id());
    let mut supervisor = test_supervisor(
        Arc::downgrade(&manager),
        spawn_actor(CloudBackupWriteSupervisor::new(Weak::new())),
    );
    let context = CloudBackupEnableContext::settings_manual();
    manager.project_enable_context_started(context);
    let claim = supervisor
        .begin_exclusive_operation(&manager, CloudBackupExclusiveOperation::Enable)
        .unwrap();
    // the Enable worker writes the Configured record while its operation runs
    Database::global()
        .cloud_backup_state
        .set(&persisted_enabled_cloud_backup_state(Some(0)))
        .unwrap();
    prepare_test_enable_recovery_local_promotion(
        &manager,
        &finalization,
        &recovered_master_key,
    );

    supervisor
        .complete_enable_recovery_finalization(
            claim,
            finalization,
            Err(CloudBackupError::Internal("finalization failed".into())),
        )
        .await
        .unwrap();

    assert_eq!(
        cspp.load_master_key_from_store().unwrap().unwrap().as_bytes(),
        &prior_master_key_bytes
    );
    assert_eq!(cloud_keychain.snapshot_passkey_metadata().unwrap(), prior_metadata);
    // a recovered backup has no pause to keep its stage for, so nothing is left for a retry to
    // refuse: the next Enable recovers the backup again from the start
    assert_eq!(cspp.master_key_promotion_status().unwrap(), MasterKeyPromotionStatus::None);
    assert!(cloud_keychain.load_pending_enable_journal().unwrap().is_none());
    assert_eq!(manager.pending_enable.pending_work(), PendingWork::None);
    assert!(!emitted_enable_completed(&manager));
    assert_eq!(supervisor.runtime, None);
}

#[tokio::test(flavor = "current_thread")]
async fn enable_recovery_finalization_projects_success_after_durable_pending_completion() {
    let _guard = async_test_lock().lock().await;
    let manager = test_supervisor_manager();
    let master_key = cove_cspp::master_key::MasterKey::generate();
    let namespace_id = master_key.namespace_id();
    let finalization = test_enable_recovery_finalization(&namespace_id);
    let mut persisted_state = persisted_enabled_cloud_backup_state(Some(0));
    assert!(persisted_state.replace_pending_verification_completion(
        finalization.pending_completion.clone()
    ));
    let mut supervisor = test_supervisor(
        Arc::downgrade(&manager),
        spawn_actor(CloudBackupWriteSupervisor::new(Weak::new())),
    );
    let context = CloudBackupEnableContext::settings_manual();
    manager.project_enable_context_started(context);
    let claim = supervisor
        .begin_exclusive_operation(&manager, CloudBackupExclusiveOperation::Enable)
        .unwrap();
    // the Enable worker writes the Configured record while its operation runs
    Database::global().cloud_backup_state.set(&persisted_state).unwrap();
    prepare_test_enable_recovery_local_promotion(&manager, &finalization, &master_key);

    supervisor
        .complete_enable_recovery_finalization(claim, finalization, Ok(()))
        .await
        .unwrap();

    assert_eq!(manager.model_snapshot().status, CloudBackupStatus::Enabled);
    assert!(manager.pending_verification_completion().is_some());
    assert!(emitted_enable_completed(&manager));
    assert_eq!(supervisor.runtime, None);
    assert!(CloudBackupKeychain::global().load_pending_enable_journal().unwrap().is_none());
    assert_eq!(CloudBackupKeychain::global().namespace_id().unwrap().as_deref(), Some(namespace_id.as_str()));
    assert_eq!(
        cove_cspp::Cspp::new(Keychain::global().clone())
            .load_master_key_from_store()
            .unwrap()
            .unwrap()
            .as_bytes(),
        master_key.as_bytes()
    );
}



#[tokio::test(flavor = "current_thread")]
async fn supervisor_accepts_registered_enable_passkey_confirmation() {
    let _guard = async_test_lock().lock().await;
    let manager = test_supervisor_manager();
    let mut supervisor = test_supervisor(
        Arc::downgrade(&manager),
        spawn_actor(CloudBackupWriteSupervisor::new(Weak::new())),
    );
    let master_key = cove_cspp::master_key::MasterKey::generate();
    let expected_namespace = master_key.namespace_id();
    let expected_credential_id = vec![1, 2, 3];

    let current = supervisor
        .begin_exclusive_operation(&manager, CloudBackupExclusiveOperation::EnableForceNew)
        .unwrap();
    let registered = CloudBackupRegisteredEnablePasskey {
        master_key: zeroize::Zeroizing::new(master_key),
        passkey: zeroize::Zeroizing::new(test_staged_passkey(expected_credential_id.clone())),
        context: CloudBackupEnableContext::settings_manual(),
    };

    supervisor
        .complete_enable_passkey_registration(
            current,
            Ok(CloudBackupEnablePasskeyRegistration::Registered(registered)),
        )
        .await
        .unwrap();

    assert_eq!(supervisor.runtime, Some(current));
    assert_eq!(manager.projected_exclusive_operation(), Some(current));
    let pending = supervisor.enable_continuation.take().unwrap().into_session();
    let (pending_master_key, pending_passkey) = pending.into_staged_parts();
    assert_eq!(pending_master_key.namespace_id(), expected_namespace);
    assert_eq!(pending_passkey.credential_id, expected_credential_id);
}

#[tokio::test(flavor = "current_thread")]
async fn enable_upload_finalization_requires_durable_pending_completion_before_success() {
    let _guard = async_test_lock().lock().await;
    let manager = test_supervisor_manager();
    let mut supervisor = test_supervisor(
        Arc::downgrade(&manager),
        spawn_actor(CloudBackupWriteSupervisor::new(Weak::new())),
    );
    let context = CloudBackupEnableContext::settings_manual();
    manager.project_enable_context_started(context);
    let claim = supervisor
        .begin_exclusive_operation(&manager, CloudBackupExclusiveOperation::Enable)
        .unwrap();
    // the Enable worker writes the Configured record while its operation runs
    Database::global()
        .cloud_backup_state
        .set(&persisted_enabled_cloud_backup_state(Some(0)))
        .unwrap();

    supervisor
        .complete_enable_upload_finalization(claim, test_enable_upload_finalization().0, Ok(()))
        .await
        .unwrap();

    assert!(matches!(manager.model_snapshot().status, CloudBackupStatus::Error(_)));
    assert!(manager.pending_verification_completion().is_none());
    assert!(!emitted_enable_completed(&manager));
    assert_eq!(supervisor.runtime, None);
}

#[tokio::test(flavor = "current_thread")]
async fn enable_upload_finalization_projects_success_after_durable_pending_completion() {
    let _guard = async_test_lock().lock().await;
    let manager = test_supervisor_manager();
    let (finalization, master_key) = test_enable_upload_finalization();
    prepare_test_enable_local_promotion(&manager, &finalization, &master_key);
    let mut persisted_state = persisted_enabled_cloud_backup_state(Some(0));
    assert!(persisted_state.replace_pending_verification_completion(
        finalization.pending_completion.clone()
    ));
    let mut supervisor = test_supervisor(
        Arc::downgrade(&manager),
        spawn_actor(CloudBackupWriteSupervisor::new(Weak::new())),
    );
    let context = CloudBackupEnableContext::settings_manual();
    manager.project_enable_context_started(context);
    let claim = supervisor
        .begin_exclusive_operation(&manager, CloudBackupExclusiveOperation::Enable)
        .unwrap();
    // the Enable worker writes the Configured record while its operation runs
    Database::global()
        .cloud_backup_state
        .set(&persisted_state)
        .unwrap();

    supervisor
        .complete_enable_upload_finalization(claim, finalization, Ok(()))
        .await
        .unwrap();

    assert_eq!(manager.model_snapshot().status, CloudBackupStatus::Enabled);
    assert!(manager.pending_verification_completion().is_some());
    assert!(emitted_enable_completed(&manager));
    assert_eq!(supervisor.runtime, None);
}

#[tokio::test(flavor = "current_thread")]
async fn a_promoted_passkeys_provider_hint_stays_with_its_credential() {
    let _guard = async_test_lock().lock().await;
    let manager = test_supervisor_manager();
    let cloud_keychain = CloudBackupKeychain::global();
    let (finalization, master_key) = test_enable_upload_finalization();
    let hint = cove_cspp::backup_data::PasskeyProviderHint {
        aaguid: "ea9b8d66-4d01-1d21-3ce4-b6b48cb575d4".into(),
        registered_platform: cove_cspp::backup_data::PasskeyRegistrationPlatform::Ios,
        registered_at: 1_700_000_000,
        name_suffix: "1700000000".into(),
    };
    prepare_test_enable_local_promotion_with_hint(
        &manager,
        &finalization,
        &master_key,
        Some(hint.clone()),
    );

    assert_eq!(cloud_keychain.provider_hint_for(&[1, 2, 3]).unwrap(), Some(hint.clone()));
    assert_eq!(cloud_keychain.provider_hint_for(&[4, 5, 6]).unwrap(), None);

    // a failed restore activation puts the hint back with the rest of the passkey metadata
    let snapshot = cloud_keychain.snapshot_passkey_metadata().unwrap();
    cloud_keychain.clear_local_state_for_test().unwrap();
    assert_eq!(Keychain::global().get(CSPP_PASSKEY_PROVIDER_HINT_KEY.into()).unwrap(), None);

    cloud_keychain.restore_passkey_metadata(&snapshot).unwrap();
    assert_eq!(cloud_keychain.provider_hint_for(&[1, 2, 3]).unwrap(), Some(hint));

    // a credential saved without a hint never reads the earlier credential's
    cloud_keychain
        .save_passkey_and_namespace(&[4, 5, 6], [8; 32], &master_key.namespace_id())
        .unwrap();
    assert_eq!(cloud_keychain.known_provider_hint(&[4, 5, 6]), None);
}

#[tokio::test(flavor = "current_thread")]
async fn pending_enable_restart_discards_stage_interrupted_after_ownership_claim() {
    let _guard = async_test_lock().lock().await;
    let manager = test_supervisor_manager();
    let cloud_keychain = CloudBackupKeychain::global();
    let cspp = cove_cspp::Cspp::new(Keychain::global().clone());
    let staged_master_key = cove_cspp::master_key::MasterKey::generate();
    cloud_keychain
        .save_pending_enable_journal(&PendingEnableJournal::staging(
            CloudBackupEnableContext::settings_manual(),
            staged_master_key.namespace_id(),
            PendingEnableNamespaceOwnership::RecoveredExisting,
            cloud_keychain.snapshot_passkey_metadata().unwrap(),
        ))
        .unwrap();
    cspp.save_staged_master_key(&staged_master_key).unwrap();
    let mut supervisor = test_supervisor(
        Arc::downgrade(&manager),
        spawn_actor(CloudBackupWriteSupervisor::new(Weak::new())),
    );

    supervisor.recover_pending_enable_after_restart(&manager).unwrap();

    assert_eq!(cspp.master_key_promotion_status().unwrap(), MasterKeyPromotionStatus::None);
    assert!(cloud_keychain.load_pending_enable_journal().unwrap().is_none());
}

#[tokio::test(flavor = "current_thread")]
async fn pending_enable_restart_discards_only_unregistered_staged_material() {
    let _guard = async_test_lock().lock().await;
    let manager = test_supervisor_manager();
    let cloud_keychain = CloudBackupKeychain::global();
    let cspp = cove_cspp::Cspp::new(Keychain::global().clone());
    let prior_master_key = cove_cspp::master_key::MasterKey::generate();
    let staged_master_key = cove_cspp::master_key::MasterKey::generate();
    cspp.save_master_key(&prior_master_key).unwrap();
    cloud_keychain
        .save_passkey_and_namespace(&[4, 5, 6], [8; 32], &prior_master_key.namespace_id())
        .unwrap();
    let previous_metadata = cloud_keychain.snapshot_passkey_metadata().unwrap();
    cspp.save_staged_master_key(&staged_master_key).unwrap();
    cloud_keychain
        .save_pending_enable_journal(&staged_pending_enable_journal(
            CloudBackupEnableContext::settings_manual(),
            staged_master_key.namespace_id(),
            PendingEnableNamespaceOwnership::FreshOwned,
            previous_metadata.clone(),
        ))
        .unwrap();
    let mut supervisor = test_supervisor(
        Arc::downgrade(&manager),
        spawn_actor(CloudBackupWriteSupervisor::new(Weak::new())),
    );

    supervisor.recover_pending_enable_after_restart(&manager).unwrap();

    assert_eq!(
        cspp.load_master_key_from_store().unwrap().unwrap().as_bytes(),
        prior_master_key.as_bytes()
    );
    assert_eq!(cspp.master_key_promotion_status().unwrap(), cove_cspp::MasterKeyPromotionStatus::None);
    assert_eq!(cloud_keychain.snapshot_passkey_metadata().unwrap(), previous_metadata);
    assert!(cloud_keychain.load_pending_enable_journal().unwrap().is_none());
}

/// A provider hint is display metadata, so one that can't be read never blocks cleaning up an
/// interrupted Enable; the restore puts it back as it was
#[tokio::test(flavor = "current_thread")]
async fn pending_enable_restart_cleans_up_beside_an_unreadable_provider_hint() {
    let _guard = async_test_lock().lock().await;
    let manager = test_supervisor_manager();
    let cloud_keychain = CloudBackupKeychain::global();
    let cspp = cove_cspp::Cspp::new(Keychain::global().clone());
    let prior_master_key = cove_cspp::master_key::MasterKey::generate();
    let staged_master_key = cove_cspp::master_key::MasterKey::generate();
    cspp.save_master_key(&prior_master_key).unwrap();
    cloud_keychain
        .save_passkey_namespace_and_hint(&[4, 5, 6], [8; 32], &prior_master_key.namespace_id(), None)
        .unwrap();
    Keychain::global()
        .save(CSPP_PASSKEY_PROVIDER_HINT_KEY.into(), "not a hint".into())
        .unwrap();
    let previous_metadata = cloud_keychain.snapshot_passkey_metadata().unwrap();
    cspp.save_staged_master_key(&staged_master_key).unwrap();
    cloud_keychain
        .save_pending_enable_journal(&staged_pending_enable_journal(
            CloudBackupEnableContext::settings_manual(),
            staged_master_key.namespace_id(),
            PendingEnableNamespaceOwnership::FreshOwned,
            previous_metadata.clone(),
        ))
        .unwrap();
    let mut supervisor = test_supervisor(
        Arc::downgrade(&manager),
        spawn_actor(CloudBackupWriteSupervisor::new(Weak::new())),
    );

    supervisor.recover_pending_enable_after_restart(&manager).unwrap();

    assert_eq!(
        cspp.load_master_key_from_store().unwrap().unwrap().as_bytes(),
        prior_master_key.as_bytes()
    );
    assert_eq!(cloud_keychain.snapshot_passkey_metadata().unwrap(), previous_metadata);
    assert!(cloud_keychain.load_pending_enable_journal().unwrap().is_none());
}

#[tokio::test(flavor = "current_thread")]
async fn pending_enable_restart_hydrates_registered_passkey_for_manual_confirmation() {
    let _guard = async_test_lock().lock().await;
    let manager = test_supervisor_manager();
    let cloud_keychain = CloudBackupKeychain::global();
    let cspp = cove_cspp::Cspp::new(Keychain::global().clone());
    let staged_master_key = cove_cspp::master_key::MasterKey::generate();
    let expected_namespace = staged_master_key.namespace_id();
    cspp.save_staged_master_key(&staged_master_key).unwrap();
    let passkey = test_staged_passkey(vec![1, 2, 3]);
    let mut journal = staged_pending_enable_journal(
        CloudBackupEnableContext::settings_manual(),
        expected_namespace.clone(),
        PendingEnableNamespaceOwnership::FreshOwned,
        cloud_keychain.snapshot_passkey_metadata().unwrap(),
    );
    assert!(journal.register_passkey(PendingEnablePasskeyMetadata {
        credential_id: passkey.credential_id.clone(),
        prf_salt: passkey.prf_salt,
        provider_hint: passkey.provider_hint.clone(),
    }));
    cloud_keychain.save_pending_enable_journal(&journal).unwrap();
    let mut supervisor = test_supervisor(
        Arc::downgrade(&manager),
        spawn_actor(CloudBackupWriteSupervisor::new(Weak::new())),
    );

    supervisor.recover_pending_enable_after_restart(&manager).unwrap();

    let pending = supervisor.enable_continuation.take().unwrap().into_session();
    assert_eq!(pending.namespace_id(), expected_namespace);
    assert_eq!(
        manager.model_snapshot().enable_state,
        CloudBackupEnableState::AwaitingSavedPasskeyConfirmation(
            SavedPasskeyConfirmationMode::Manual
        )
    );
    assert!(cloud_keychain.load_pending_enable_journal().unwrap().is_some());
}

#[tokio::test(flavor = "current_thread")]
async fn pending_enable_restart_rolls_back_recovered_existing_instead_of_hydrating_fresh_upload() {
    let _guard = async_test_lock().lock().await;
    let manager = test_supervisor_manager();
    let cloud_keychain = CloudBackupKeychain::global();
    let cspp = cove_cspp::Cspp::new(Keychain::global().clone());
    let prior_master_key = cove_cspp::master_key::MasterKey::generate();
    let recovered_master_key = cove_cspp::master_key::MasterKey::generate();
    cspp.save_master_key(&prior_master_key).unwrap();
    cloud_keychain
        .save_passkey_and_namespace(&[4, 5, 6], [8; 32], &prior_master_key.namespace_id())
        .unwrap();
    let previous_metadata = cloud_keychain.snapshot_passkey_metadata().unwrap();
    cspp.save_staged_master_key(&recovered_master_key).unwrap();
    let passkey = test_staged_passkey(vec![1, 2, 3]);
    let mut journal = staged_pending_enable_journal(
        CloudBackupEnableContext::settings_manual(),
        recovered_master_key.namespace_id(),
        PendingEnableNamespaceOwnership::RecoveredExisting,
        previous_metadata.clone(),
    );
    assert!(journal.register_passkey(PendingEnablePasskeyMetadata {
        credential_id: passkey.credential_id.clone(),
        prf_salt: passkey.prf_salt,
        provider_hint: passkey.provider_hint.clone(),
    }));
    assert!(journal.mark_remote_writes_started());
    cloud_keychain.save_pending_enable_journal(&journal).unwrap();
    let mut supervisor = test_supervisor(
        Arc::downgrade(&manager),
        spawn_actor(CloudBackupWriteSupervisor::new(Weak::new())),
    );

    supervisor.recover_pending_enable_after_restart(&manager).unwrap();

    assert!(supervisor.enable_continuation.is_none());
    assert_eq!(
        cspp.load_master_key_from_store().unwrap().unwrap().as_bytes(),
        prior_master_key.as_bytes()
    );
    assert_eq!(
        cspp.master_key_promotion_status().unwrap(),
        cove_cspp::MasterKeyPromotionStatus::None
    );
    assert_eq!(cloud_keychain.snapshot_passkey_metadata().unwrap(), previous_metadata);
    assert!(cloud_keychain.load_pending_enable_journal().unwrap().is_none());

    cspp.save_staged_master_key(&recovered_master_key).unwrap();
    let mut promoted_journal = staged_pending_enable_journal(
        CloudBackupEnableContext::settings_manual(),
        recovered_master_key.namespace_id(),
        PendingEnableNamespaceOwnership::RecoveredExisting,
        previous_metadata.clone(),
    );
    assert!(promoted_journal.register_passkey(PendingEnablePasskeyMetadata {
        credential_id: vec![1, 2, 3],
        prf_salt: [9; 32],
        provider_hint: None,
    }));
    assert!(promoted_journal.mark_remote_writes_started());
    assert!(promoted_journal.mark_local_promotion_started());
    cloud_keychain.save_pending_enable_journal(&promoted_journal).unwrap();
    cspp.promote_staged_master_key().unwrap();
    cloud_keychain
        .save_passkey_and_namespace(
            &[1, 2, 3],
            [9; 32],
            &recovered_master_key.namespace_id(),
        )
        .unwrap();

    supervisor.recover_pending_enable_after_restart(&manager).unwrap();

    assert!(supervisor.enable_continuation.is_none());
    assert_eq!(
        cspp.load_master_key_from_store().unwrap().unwrap().as_bytes(),
        prior_master_key.as_bytes()
    );
    assert_eq!(
        cspp.master_key_promotion_status().unwrap(),
        cove_cspp::MasterKeyPromotionStatus::None
    );
    assert_eq!(cloud_keychain.snapshot_passkey_metadata().unwrap(), previous_metadata);
    assert!(cloud_keychain.load_pending_enable_journal().unwrap().is_none());
}

#[tokio::test(flavor = "current_thread")]
async fn pending_enable_restart_finishes_promoted_state_with_matching_completion() {
    let _guard = async_test_lock().lock().await;
    let manager = test_supervisor_manager();
    let (finalization, master_key) = test_enable_upload_finalization();
    prepare_test_enable_local_promotion(&manager, &finalization, &master_key);
    let mut persisted_state = persisted_enabled_cloud_backup_state(Some(0));
    assert!(persisted_state.replace_pending_verification_completion(
        finalization.pending_completion.clone()
    ));
    Database::global().cloud_backup_state.set(&persisted_state).unwrap();
    let mut supervisor = test_supervisor(
        Arc::downgrade(&manager),
        spawn_actor(CloudBackupWriteSupervisor::new(Weak::new())),
    );

    supervisor.recover_pending_enable_after_restart(&manager).unwrap();

    let cloud_keychain = CloudBackupKeychain::global();
    let cspp = cove_cspp::Cspp::new(Keychain::global().clone());
    assert_eq!(cspp.master_key_promotion_status().unwrap(), cove_cspp::MasterKeyPromotionStatus::None);
    assert_eq!(
        cspp.load_master_key_from_store().unwrap().unwrap().as_bytes(),
        master_key.as_bytes()
    );
    assert_eq!(cloud_keychain.namespace_id().unwrap().as_deref(), Some(finalization.pending_completion.namespace_id()));
    assert_eq!(cloud_keychain.load_credential_id().unwrap().as_deref(), Some(TEST_UPLOAD_CREDENTIAL_ID));
    assert!(cloud_keychain.load_pending_enable_journal().unwrap().is_none());
}

#[tokio::test(flavor = "current_thread")]
async fn pending_enable_restart_finishes_after_commit_cleared_staging_slot() {
    let _guard = async_test_lock().lock().await;
    let globals = test_globals();
    let manager = test_supervisor_manager();
    let (finalization, master_key) = test_enable_upload_finalization();
    prepare_test_enable_local_promotion(&manager, &finalization, &master_key);
    let mut persisted_state = persisted_enabled_cloud_backup_state(Some(0));
    assert!(persisted_state.replace_pending_verification_completion(
        finalization.pending_completion.clone()
    ));
    Database::global().cloud_backup_state.set(&persisted_state).unwrap();
    let cspp = cove_cspp::Cspp::new(Keychain::global().clone());

    globals.keychain.fail_delete_at(3);
    assert!(cspp.commit_master_key_promotion().is_err());
    assert!(globals
        .keychain
        .get_entry("cspp::v1::staged_master_key_encryption_key_and_nonce")
        .is_none());
    assert!(globals.keychain.get_entry("cspp::v1::staged_master_key").is_none());
    assert_eq!(
        cspp.master_key_promotion_status().unwrap(),
        MasterKeyPromotionStatus::Pending(MasterKeyPromotionActiveState::Staged)
    );

    cove_cspp::Cspp::<Keychain>::clear_cached_master_key();
    let mut supervisor = test_supervisor(
        Arc::downgrade(&manager),
        spawn_actor(CloudBackupWriteSupervisor::new(Weak::new())),
    );

    supervisor.resume_pending_enable_after_restart().await.unwrap();

    assert_eq!(cspp.master_key_promotion_status().unwrap(), MasterKeyPromotionStatus::None);
    assert_eq!(
        cspp.load_master_key_from_store().unwrap().unwrap().as_bytes(),
        master_key.as_bytes()
    );
    assert_eq!(
        CloudBackupKeychain::global().namespace_id().unwrap().as_deref(),
        Some(finalization.pending_completion.namespace_id())
    );
    assert!(CloudBackupKeychain::global()
        .load_pending_enable_journal()
        .unwrap()
        .is_none());
    assert!(!matches!(manager.model_state().lifecycle, CloudBackupLifecycle::PendingEnableRecovery(_)));
}

#[tokio::test(flavor = "current_thread")]
async fn pending_enable_restart_restores_prior_state_and_hydrates_when_completion_is_missing() {
    let _guard = async_test_lock().lock().await;
    let manager = test_supervisor_manager();
    let cloud_keychain = CloudBackupKeychain::global();
    let cspp = cove_cspp::Cspp::new(Keychain::global().clone());
    let prior_master_key = cove_cspp::master_key::MasterKey::generate();
    cspp.save_master_key(&prior_master_key).unwrap();
    cloud_keychain
        .save_passkey_and_namespace(&[4, 5, 6], [8; 32], &prior_master_key.namespace_id())
        .unwrap();
    let previous_metadata = cloud_keychain.snapshot_passkey_metadata().unwrap();
    let (finalization, staged_master_key) = test_enable_upload_finalization();
    prepare_test_enable_local_promotion(&manager, &finalization, &staged_master_key);
    let mut supervisor = test_supervisor(
        Arc::downgrade(&manager),
        spawn_actor(CloudBackupWriteSupervisor::new(Weak::new())),
    );

    supervisor.recover_pending_enable_after_restart(&manager).unwrap();

    assert_eq!(
        cspp.master_key_promotion_status().unwrap(),
        cove_cspp::MasterKeyPromotionStatus::Pending(
            cove_cspp::MasterKeyPromotionActiveState::Prior
        )
    );
    assert_eq!(
        cspp.load_master_key_from_store().unwrap().unwrap().as_bytes(),
        prior_master_key.as_bytes()
    );
    assert_eq!(cloud_keychain.snapshot_passkey_metadata().unwrap(), previous_metadata);
    assert!(supervisor.enable_continuation.is_some());
    assert!(matches!(
        cloud_keychain.load_pending_enable_journal().unwrap().unwrap().phase(),
        PendingEnableJournalPhase::RemoteWritesStarted(_)
    ));
}

#[tokio::test(flavor = "current_thread")]
async fn pending_enable_restart_fails_closed_on_conflicting_completion() {
    let _guard = async_test_lock().lock().await;
    let manager = test_supervisor_manager();
    let (finalization, master_key) = test_enable_upload_finalization();
    prepare_test_enable_local_promotion(&manager, &finalization, &master_key);
    let mut persisted_state = persisted_enabled_cloud_backup_state(Some(0));
    assert!(persisted_state.replace_pending_verification_completion(test_pending_completion(
        "ffffffffffffffffffffffffffffffff".into(),
        vec![PendingVerificationUpload::master_key_wrapper()],
    )));
    Database::global().cloud_backup_state.set(&persisted_state).unwrap();
    let mut supervisor = test_supervisor(
        Arc::downgrade(&manager),
        spawn_actor(CloudBackupWriteSupervisor::new(Weak::new())),
    );

    let error = supervisor.recover_pending_enable_after_restart(&manager).unwrap_err();

    assert!(error.to_string().contains("pending verification completion did not match"));
    assert!(matches!(
        CloudBackupKeychain::global()
            .load_pending_enable_journal()
            .unwrap()
            .unwrap()
            .phase(),
        PendingEnableJournalPhase::LocalPromotionStarted(_)
    ));
    assert_eq!(
        cove_cspp::Cspp::new(Keychain::global().clone())
            .master_key_promotion_status()
            .unwrap(),
        cove_cspp::MasterKeyPromotionStatus::Pending(
            cove_cspp::MasterKeyPromotionActiveState::Staged
        )
    );

    supervisor.resume_pending_enable_after_restart().await.unwrap();

    assert!(matches!(
        manager.model_state().lifecycle,
        CloudBackupLifecycle::PendingEnableRecovery(CloudBackupPendingEnableRecovery {
            support_code,
            cleanup: CloudBackupPendingEnableCleanupState::SupportOnly,
        }) if support_code == "CB-PE-004"
    ));
}

#[tokio::test(flavor = "current_thread")]
async fn pending_enable_restart_fails_closed_on_unowned_cspp_staging() {
    let _guard = async_test_lock().lock().await;
    let manager = test_supervisor_manager();
    let cspp = cove_cspp::Cspp::new(Keychain::global().clone());
    cspp.save_staged_master_key(&cove_cspp::master_key::MasterKey::generate()).unwrap();
    let mut supervisor = test_supervisor(
        Arc::downgrade(&manager),
        spawn_actor(CloudBackupWriteSupervisor::new(Weak::new())),
    );

    let error = supervisor.recover_pending_enable_after_restart(&manager).unwrap_err();

    assert!(error.to_string().contains("had no ownership journal"));
    assert_eq!(
        cspp.master_key_promotion_status().unwrap(),
        cove_cspp::MasterKeyPromotionStatus::Staged
    );
}

#[tokio::test(flavor = "current_thread")]
async fn pending_enable_restart_projects_privacy_safe_recovery_lifecycle() {
    let _guard = async_test_lock().lock().await;
    let manager = test_supervisor_manager();
    let cspp = cove_cspp::Cspp::new(Keychain::global().clone());
    cspp.save_staged_master_key(&cove_cspp::master_key::MasterKey::generate()).unwrap();
    let mut supervisor = test_supervisor(
        Arc::downgrade(&manager),
        spawn_actor(CloudBackupWriteSupervisor::new(Weak::new())),
    );

    supervisor.resume_pending_enable_after_restart().await.unwrap();

    assert!(matches!(
        manager.model_state().lifecycle,
        CloudBackupLifecycle::PendingEnableRecovery(CloudBackupPendingEnableRecovery {
            support_code,
            cleanup: CloudBackupPendingEnableCleanupState::Available,
        }) if support_code == "CB-PE-001"
    ));
    assert_eq!(supervisor.projected_screen(&manager).settings_row, CloudBackupSettingsRowStatus::RecoveryRequired);
}

#[tokio::test(flavor = "current_thread")]
async fn pending_enable_recovery_survives_platform_persisted_state_sync() {
    let _guard = async_test_lock().lock().await;
    let manager = test_supervisor_manager();
    let cspp = cove_cspp::Cspp::new(Keychain::global().clone());
    cspp.save_staged_master_key(&cove_cspp::master_key::MasterKey::generate()).unwrap();

    // the manager's own supervisor resumes, since the sync below asks it to project its runtime;
    // a second supervisor would leave that runtime empty and the projection would undo recovery
    call!(manager.supervisor.resume_pending_enable_after_restart()).await.unwrap();

    // both platforms sync persisted state after bootstrap, which can land after the projection
    manager.sync_persisted_state();
    crate::manager::cloud_backup_manager::ops::test_support::supervisor_checkpoint(&manager).await;

    assert!(matches!(
        manager.model_state().lifecycle,
        CloudBackupLifecycle::PendingEnableRecovery(CloudBackupPendingEnableRecovery {
            support_code,
            cleanup: CloudBackupPendingEnableCleanupState::Available,
        }) if support_code == "CB-PE-001"
    ));
    assert_eq!(manager.published_screen().settings_row, CloudBackupSettingsRowStatus::RecoveryRequired);

    call!(manager.supervisor.confirm_pending_enable_cleanup()).await.unwrap();

    assert_eq!(cspp.master_key_promotion_status().unwrap(), MasterKeyPromotionStatus::None);
    assert!(matches!(manager.model_state().lifecycle, CloudBackupLifecycle::Disabled));
}

#[tokio::test(flavor = "current_thread")]
async fn pending_enable_recovery_survives_persisted_state_reconcile() {
    let _guard = async_test_lock().lock().await;
    let manager = test_supervisor_manager();
    let cspp = cove_cspp::Cspp::new(Keychain::global().clone());
    cspp.save_master_key(&cove_cspp::master_key::MasterKey::generate()).unwrap();
    cspp.save_staged_master_key(&cove_cspp::master_key::MasterKey::generate()).unwrap();
    let mut supervisor = test_supervisor(
        Arc::downgrade(&manager),
        spawn_actor(CloudBackupWriteSupervisor::new(Weak::new())),
    );

    supervisor.resume_pending_enable_after_restart().await.unwrap();

    // a configured backup's verifier persists its result while the orphaned stage remains
    manager.reconcile_persisted_state(&persisted_enabled_cloud_backup_state(Some(0)));

    assert!(matches!(
        manager.model_state().lifecycle,
        CloudBackupLifecycle::PendingEnableRecovery(_)
    ));
}

#[tokio::test(flavor = "current_thread")]
async fn no_journal_staged_cleanup_preserves_active_key_and_metadata_without_remote_delete() {
    let _guard = async_test_lock().lock().await;
    let globals = test_globals();
    let manager = test_supervisor_manager();
    let cloud_keychain = CloudBackupKeychain::global();
    let cspp = cove_cspp::Cspp::new(Keychain::global().clone());
    let active_master_key = cove_cspp::master_key::MasterKey::generate();
    let staged_master_key = cove_cspp::master_key::MasterKey::generate();
    cspp.save_master_key(&active_master_key).unwrap();
    cloud_keychain
        .save_passkey_and_namespace(&[4, 5, 6], [8; 32], &active_master_key.namespace_id())
        .unwrap();
    let previous_metadata = cloud_keychain.snapshot_passkey_metadata().unwrap();
    Database::global()
        .cloud_backup_state
        .set(&persisted_enabled_cloud_backup_state(Some(0)))
        .unwrap();
    cspp.save_staged_master_key(&staged_master_key).unwrap();
    let remote_deletes_before = globals.cloud.delete_namespace_attempt_count();
    let mut supervisor = test_supervisor(
        Arc::downgrade(&manager),
        spawn_actor(CloudBackupWriteSupervisor::new(Weak::new())),
    );

    supervisor.resume_pending_enable_after_restart().await.unwrap();
    assert!(matches!(
        manager.model_state().lifecycle,
        CloudBackupLifecycle::PendingEnableRecovery(CloudBackupPendingEnableRecovery {
            support_code,
            cleanup: CloudBackupPendingEnableCleanupState::Available,
        }) if support_code == "CB-PE-001"
    ));

    supervisor.confirm_pending_enable_cleanup().await.unwrap();

    assert_eq!(
        cspp.load_master_key_from_store().unwrap().unwrap().as_bytes(),
        active_master_key.as_bytes()
    );
    assert_eq!(cloud_keychain.snapshot_passkey_metadata().unwrap(), previous_metadata);
    assert!(cloud_keychain.load_pending_enable_journal().unwrap().is_none());
    assert_eq!(cspp.master_key_promotion_status().unwrap(), MasterKeyPromotionStatus::None);
    assert_eq!(globals.cloud.delete_namespace_attempt_count(), remote_deletes_before);
    assert!(matches!(manager.model_state().lifecycle, CloudBackupLifecycle::Configured(_)));
}

#[tokio::test(flavor = "current_thread")]
async fn no_journal_staged_cleanup_revalidates_before_discarding() {
    let _guard = async_test_lock().lock().await;
    let manager = test_supervisor_manager();
    let cspp = cove_cspp::Cspp::new(Keychain::global().clone());
    cspp.save_master_key(&cove_cspp::master_key::MasterKey::generate()).unwrap();
    cspp.save_staged_master_key(&cove_cspp::master_key::MasterKey::generate()).unwrap();
    let mut supervisor = test_supervisor(
        Arc::downgrade(&manager),
        spawn_actor(CloudBackupWriteSupervisor::new(Weak::new())),
    );
    supervisor.resume_pending_enable_after_restart().await.unwrap();
    assert!(matches!(
        manager.model_state().lifecycle,
        CloudBackupLifecycle::PendingEnableRecovery(CloudBackupPendingEnableRecovery {
            cleanup: CloudBackupPendingEnableCleanupState::Available,
            ..
        })
    ));
    cspp.promote_staged_master_key().unwrap();

    supervisor.confirm_pending_enable_cleanup().await.unwrap();

    assert!(matches!(
        manager.model_state().lifecycle,
        CloudBackupLifecycle::PendingEnableRecovery(CloudBackupPendingEnableRecovery {
            support_code,
            cleanup: CloudBackupPendingEnableCleanupState::SupportOnly,
        }) if support_code == "CB-PE-001"
    ));
    assert!(matches!(
        cspp.master_key_promotion_status().unwrap(),
        MasterKeyPromotionStatus::Pending(_)
    ));
}

#[tokio::test(flavor = "current_thread")]
async fn pending_enable_restart_projects_promotion_mismatch_as_support_only() {
    let _guard = async_test_lock().lock().await;
    let manager = test_supervisor_manager();
    let cloud_keychain = CloudBackupKeychain::global();
    let cspp = cove_cspp::Cspp::new(Keychain::global().clone());
    let staged_master_key = cove_cspp::master_key::MasterKey::generate();
    cspp.save_staged_master_key(&staged_master_key).unwrap();
    let mut journal = staged_pending_enable_journal(
        CloudBackupEnableContext::settings_manual(),
        staged_master_key.namespace_id(),
        PendingEnableNamespaceOwnership::FreshOwned,
        cloud_keychain.snapshot_passkey_metadata().unwrap(),
    );
    assert!(journal.register_passkey(PendingEnablePasskeyMetadata {
        credential_id: vec![1, 2, 3],
        prf_salt: [9; 32],
        provider_hint: None,
    }));
    cloud_keychain.save_pending_enable_journal(&journal).unwrap();
    cspp.promote_staged_master_key().unwrap();
    let mut supervisor = test_supervisor(
        Arc::downgrade(&manager),
        spawn_actor(CloudBackupWriteSupervisor::new(Weak::new())),
    );

    supervisor.resume_pending_enable_after_restart().await.unwrap();

    assert!(matches!(
        manager.model_state().lifecycle,
        CloudBackupLifecycle::PendingEnableRecovery(CloudBackupPendingEnableRecovery {
            support_code,
            cleanup: CloudBackupPendingEnableCleanupState::SupportOnly,
        }) if support_code == "CB-PE-003"
    ));
}

#[tokio::test(flavor = "current_thread")]
async fn pending_enable_restart_projects_unreadable_evidence_without_diagnostics() {
    let _guard = async_test_lock().lock().await;
    let globals = test_globals();
    let manager = test_supervisor_manager();
    globals.keychain.set_entries(vec![(
        crate::manager::cloud_backup_manager::keychain::CSPP_PENDING_ENABLE_JOURNAL_KEY,
        "not valid journal json",
    )]);
    let mut supervisor = test_supervisor(
        Arc::downgrade(&manager),
        spawn_actor(CloudBackupWriteSupervisor::new(Weak::new())),
    );

    supervisor.resume_pending_enable_after_restart().await.unwrap();

    let CloudBackupLifecycle::PendingEnableRecovery(recovery) = manager.model_state().lifecycle else {
        panic!("expected pending enable recovery");
    };
    assert_eq!(recovery.support_code, "CB-PE-005");
    assert_eq!(recovery.cleanup, CloudBackupPendingEnableCleanupState::SupportOnly);
}

#[tokio::test(flavor = "current_thread")]
async fn pending_enable_local_cleanup_preserves_prior_state_and_never_deletes_remote_data() {
    let _guard = async_test_lock().lock().await;
    let globals = test_globals();
    let manager = test_supervisor_manager();
    let cloud_keychain = CloudBackupKeychain::global();
    let cspp = cove_cspp::Cspp::new(Keychain::global().clone());
    let prior_master_key = cove_cspp::master_key::MasterKey::generate();
    let staged_master_key = cove_cspp::master_key::MasterKey::generate();
    cspp.save_master_key(&prior_master_key).unwrap();
    cloud_keychain
        .save_passkey_and_namespace(&[4, 5, 6], [8; 32], &prior_master_key.namespace_id())
        .unwrap();
    let previous_metadata = cloud_keychain.snapshot_passkey_metadata().unwrap();
    Database::global()
        .cloud_backup_state
        .set(&persisted_enabled_cloud_backup_state(Some(0)))
        .unwrap();
    cspp.save_staged_master_key(&staged_master_key).unwrap();
    cloud_keychain
        .save_pending_enable_journal(&staged_pending_enable_journal(
            CloudBackupEnableContext::settings_manual(),
            staged_master_key.namespace_id(),
            PendingEnableNamespaceOwnership::FreshOwned,
            previous_metadata.clone(),
        ))
        .unwrap();
    let remote_deletes_before = globals.cloud.delete_namespace_attempt_count();
    let mut supervisor = test_supervisor(
        Arc::downgrade(&manager),
        spawn_actor(CloudBackupWriteSupervisor::new(Weak::new())),
    );

    supervisor.confirm_pending_enable_cleanup().await.unwrap();

    assert_eq!(
        cspp.load_master_key_from_store().unwrap().unwrap().as_bytes(),
        prior_master_key.as_bytes()
    );
    assert_eq!(cloud_keychain.snapshot_passkey_metadata().unwrap(), previous_metadata);
    assert_eq!(cspp.master_key_promotion_status().unwrap(), MasterKeyPromotionStatus::None);
    assert!(cloud_keychain.load_pending_enable_journal().unwrap().is_none());
    assert_eq!(globals.cloud.delete_namespace_attempt_count(), remote_deletes_before);
    assert!(matches!(manager.model_state().lifecycle, CloudBackupLifecycle::Configured(_)));
}

#[tokio::test(flavor = "current_thread")]
async fn pending_enable_cleanup_revalidates_stale_evidence_before_mutating() {
    let _guard = async_test_lock().lock().await;
    let manager = test_supervisor_manager();
    let cloud_keychain = CloudBackupKeychain::global();
    let cspp = cove_cspp::Cspp::new(Keychain::global().clone());
    let staged_master_key = cove_cspp::master_key::MasterKey::generate();
    cspp.save_staged_master_key(&staged_master_key).unwrap();
    cloud_keychain
        .save_pending_enable_journal(&staged_pending_enable_journal(
            CloudBackupEnableContext::settings_manual(),
            "ffffffffffffffffffffffffffffffff".into(),
            PendingEnableNamespaceOwnership::FreshOwned,
            cloud_keychain.snapshot_passkey_metadata().unwrap(),
        ))
        .unwrap();
    let mut supervisor = test_supervisor(
        Arc::downgrade(&manager),
        spawn_actor(CloudBackupWriteSupervisor::new(Weak::new())),
    );

    supervisor.confirm_pending_enable_cleanup().await.unwrap();

    assert_eq!(cspp.master_key_promotion_status().unwrap(), MasterKeyPromotionStatus::Staged);
    assert!(matches!(
        manager.model_state().lifecycle,
        CloudBackupLifecycle::PendingEnableRecovery(CloudBackupPendingEnableRecovery {
            support_code,
            cleanup: CloudBackupPendingEnableCleanupState::SupportOnly,
        }) if support_code == "CB-PE-002"
    ));
}

#[tokio::test(flavor = "current_thread")]
async fn pending_enable_cleanup_failure_recomputes_retry_availability() {
    let _guard = async_test_lock().lock().await;
    let globals = test_globals();
    let manager = test_supervisor_manager();
    let cloud_keychain = CloudBackupKeychain::global();
    let cspp = cove_cspp::Cspp::new(Keychain::global().clone());
    let staged_master_key = cove_cspp::master_key::MasterKey::generate();
    cspp.save_staged_master_key(&staged_master_key).unwrap();
    cloud_keychain
        .save_pending_enable_journal(&staged_pending_enable_journal(
            CloudBackupEnableContext::settings_manual(),
            staged_master_key.namespace_id(),
            PendingEnableNamespaceOwnership::FreshOwned,
            cloud_keychain.snapshot_passkey_metadata().unwrap(),
        ))
        .unwrap();
    globals.keychain.fail_delete_at(3);
    let mut supervisor = test_supervisor(
        Arc::downgrade(&manager),
        spawn_actor(CloudBackupWriteSupervisor::new(Weak::new())),
    );

    supervisor.confirm_pending_enable_cleanup().await.unwrap();

    assert!(matches!(
        manager.model_state().lifecycle,
        CloudBackupLifecycle::PendingEnableRecovery(CloudBackupPendingEnableRecovery {
            support_code,
            cleanup: CloudBackupPendingEnableCleanupState::Available,
        }) if support_code == "CB-PE-006"
    ));
    assert!(cloud_keychain.load_pending_enable_journal().unwrap().is_some());
}



#[tokio::test(flavor = "current_thread")]
async fn turn_off_recheck_after_the_record_moved_on_deletes_nothing() {
    let _guard = async_test_lock().lock().await;
    let globals = test_globals();
    let manager = test_supervisor_manager();
    let mut supervisor = test_supervisor(
        Arc::downgrade(&manager),
        spawn_actor(CloudBackupWriteSupervisor::new(Weak::new())),
    );

    persist_enabled_under(&mut supervisor);
    let claim = supervisor
        .begin_exclusive_operation(&manager, CloudBackupExclusiveOperation::Disable)
        .unwrap();
    test_disabling_state();
    Database::global().cloud_backup_state.set(&PersistedCloudBackupState::Disabled).unwrap();

    // the inventory never loaded, as in the review, so the recheck finds nothing new and reaches
    // the commit, where the record has moved on
    let unknown = CloudBackupInventoryAssessment::Unknown {
        reason: CloudBackupInventoryIncompleteReason::Unknown,
    };
    let review = empty_review_assessed(VALID_NAMESPACE_ID, unknown);
    supervisor
        .complete_turn_off_recheck(claim, review, Ok(empty_evidence(VALID_NAMESPACE_ID)))
        .await
        .unwrap();

    assert_eq!(supervisor.runtime, None);
    assert_eq!(manager.projected_exclusive_operation(), None);
    assert_eq!(globals.cloud.delete_namespace_attempt_count(), 0);
    assert_eq!(Database::global().cloud_backup_state.get().unwrap(), PersistedCloudBackupState::Disabled);
}

#[tokio::test(flavor = "current_thread")]
async fn stale_completion_is_discarded_without_touching_the_running_operation() {
    let _guard = async_test_lock().lock().await;
    let manager = test_supervisor_manager();
    persist_enabled_for_supervisor_test();
    call!(manager.supervisor.rebase_after_test_write()).await.unwrap();

    let (finished, _) =
        begin_operation_writes_for_test(&manager, CloudBackupExclusiveOperation::RecreateManifest)
            .await;
    finish_operation_for_test(&manager, finished).await;
    let (running, _) =
        begin_operation_writes_for_test(&manager, CloudBackupExclusiveOperation::RepairPasskey)
            .await;

    // the reset and the upload verifier each request a sync-health refresh that lands later
    wait_for_background_refreshes(&manager).await;

    // background refreshes drive sync health and completions never do, and a refresh requested
    // before the snapshot can still land after it, so the comparison leaves sync health out;
    // the state has no PartialEq, so its debug form stands in for it
    let operation_state = || {
        let mut state = manager.model_state();
        if let CloudBackupLifecycle::Configured(configured) = &mut state.lifecycle {
            configured.sync_health = cove_device::cloud_storage::CloudSyncHealth::Unknown;
        }

        format!("{state:?}")
    };
    let before = operation_state();

    // the finished operation's late failure and success both arrive while the next one runs
    call!(
        manager
            .supervisor
            .fail_exclusive_operation(finished, CloudBackupError::Internal("late failure".into()))
    )
    .await
    .unwrap();
    call!(manager.supervisor.complete_exclusive_operation(finished)).await.unwrap();

    let claim = call!(manager.supervisor.operation_claim_for_test()).await.unwrap();
    let last_outcome = call!(manager.supervisor.harness_last_outcome()).await.unwrap();
    assert_eq!(claim, Some(running));
    assert_eq!(manager.projected_exclusive_operation(), Some(running));
    assert_eq!(operation_state(), before);
    assert_eq!(last_outcome, None, "a stale completion is never a rejection");

    call!(manager.supervisor.complete_exclusive_operation(running)).await.unwrap();

    let claim = call!(manager.supervisor.operation_claim_for_test()).await.unwrap();
    assert_eq!(claim, None);
    assert_eq!(manager.projected_exclusive_operation(), None);
}


/// A configuration replaced while the Turn Off review is on screen makes the review stale:
/// Confirm is rejected, so consent never carries over to a backup the user didn't review
#[tokio::test(flavor = "current_thread")]
async fn a_configuration_change_makes_the_turn_off_review_stale() {
    let _guard = async_test_lock().lock().await;
    let manager = test_supervisor_manager();
    let mut supervisor = test_supervisor(
        Arc::downgrade(&manager),
        spawn_actor(CloudBackupWriteSupervisor::new(Weak::new())),
    );

    persist_enabled_under(&mut supervisor);
    let generation = supervisor.runtime.generation();
    let review = empty_review(VALID_NAMESPACE_ID);
    let review_id = review.review_id.get();
    supervisor.runtime.set_turn_off_review(Some(TurnOffReview::Reviewing(Box::new(review))));
    let confirm = UserIntent::ConfirmDisable { review_id };
    assert_eq!(
        evaluate_admission(&confirm, &supervisor.admission_snapshot(&manager, &manager.inventory_facts())),
        Admission::Accepted
    );

    // a reinitialize replaces the configuration once it ends
    let claim = supervisor
        .begin_exclusive_operation(&manager, CloudBackupExclusiveOperation::ReinitializeBackup)
        .unwrap();
    supervisor.complete_exclusive_operation(claim).await.unwrap();

    assert_ne!(supervisor.runtime.generation(), generation);
    assert_eq!(supervisor.runtime.turn_off_review(), None);
    assert_eq!(
        evaluate_admission(&confirm, &supervisor.admission_snapshot(&manager, &manager.inventory_facts())),
        Admission::Rejected(Rejection::ReviewStale)
    );
}

/// Restore or Verify First from the review starts its own operation, which ends the review, so
/// the next Turn Off tap prepares a fresh one from what that operation left
#[tokio::test(flavor = "current_thread")]
async fn an_operation_started_from_the_review_ends_it() {
    let _guard = async_test_lock().lock().await;
    let manager = test_supervisor_manager();
    let mut supervisor = test_supervisor(
        Arc::downgrade(&manager),
        spawn_actor(CloudBackupWriteSupervisor::new(Weak::new())),
    );

    persist_enabled_under(&mut supervisor);
    let review = empty_review(VALID_NAMESPACE_ID);
    supervisor.runtime.set_turn_off_review(Some(TurnOffReview::Reviewing(Box::new(review))));
    assert!(supervisor.runtime.turn_off_review().is_some());

    let claim = supervisor
        .begin_exclusive_operation(&manager, CloudBackupExclusiveOperation::RestoreCloudWallet)
        .unwrap();

    assert_eq!(supervisor.runtime.turn_off_review(), None);
    assert_eq!(supervisor.runtime.turn_off_admission(), TurnOffAdmission::None);

    // while that operation runs, Turn Off waits for it instead of reviewing beside it
    assert!(matches!(
        evaluate_admission(&UserIntent::TurnOff, &supervisor.admission_snapshot(&manager, &manager.inventory_facts())),
        Admission::Rejected(Rejection::OperationRunning(_))
    ));

    supervisor.complete_exclusive_operation(claim).await.unwrap();

    assert_eq!(supervisor.runtime.turn_off_review(), None);
    assert_eq!(
        evaluate_admission(&UserIntent::TurnOff, &supervisor.admission_snapshot(&manager, &manager.inventory_facts())),
        Admission::Accepted
    );
}

/// A Restore Them First whose restore can't start on this inventory starts nothing and leaves
/// the same review open: one wallet needs the inventory loaded, and several need Restore All
#[tokio::test(flavor = "current_thread")]
async fn a_restore_first_the_inventory_cannot_start_leaves_the_review_open() {
    let _guard = async_test_lock().lock().await;
    let manager = test_supervisor_manager();
    let mut supervisor = test_supervisor(
        Arc::downgrade(&manager),
        spawn_actor(CloudBackupWriteSupervisor::new(Weak::new())),
    );

    persist_enabled_under(&mut supervisor);
    let cases: [(&[&str], Rejection); 2] = [
        (&["Cold Storage"], Rejection::DetailInventoryNotLoaded),
        (&["Cold Storage", "Spending"], Rejection::RestoreAllUnavailable),
    ];
    for (names, rejection) in cases {
        let review = review_not_on_device(VALID_NAMESPACE_ID, names);
        let review_id = review.review_id.get();
        supervisor.runtime.set_turn_off_review(Some(TurnOffReview::Reviewing(Box::new(review))));
        let restore_first = UserIntent::LeaveReview { review_id, to: ReviewExit::RestoreFirst };
        let snapshot = supervisor.admission_snapshot(&manager, &manager.inventory_facts());
        assert_eq!(evaluate_admission(&restore_first, &snapshot), Admission::Rejected(rejection));

        supervisor.dispatch(restore_first).await.unwrap();

        assert_eq!(supervisor.runtime.turn_off_admission().review_id(), Some(review_id));
        assert_eq!(supervisor.runtime.claim(), None, "{names:?} started an operation");
    }
}

#[tokio::test(flavor = "current_thread")]
async fn turn_off_drain_failure_before_commit_leaves_backup_configured() {
    let _guard = async_test_lock().lock().await;
    let manager = test_supervisor_manager();
    let mut supervisor = test_supervisor(
        Arc::downgrade(&manager),
        spawn_actor(CloudBackupWriteSupervisor::new(Weak::new())),
    );

    persist_enabled_under(&mut supervisor);
    let claim = supervisor
        .begin_exclusive_operation(&manager, CloudBackupExclusiveOperation::Disable)
        .unwrap();
    let review = empty_review(VALID_NAMESPACE_ID);

    supervisor
        .complete_turn_off_drain(claim, review, Err(CloudBackupError::Internal("drain".into())))
        .await
        .unwrap();

    // nothing was committed, so the record is still configured and the user is told why
    assert_eq!(supervisor.runtime, None);
    assert_eq!(manager.projected_exclusive_operation(), None);
    assert!(matches!(
        Database::global().cloud_backup_state.get().unwrap(),
        PersistedCloudBackupState::Configured(_)
    ));
    assert_eq!(
        supervisor.projected_screen(&manager).last_outcome.map(|last| last.outcome),
        Some(CloudBackupOutcome::DisableNotStarted {
            reason: CloudBackupDisableNotStartedReason::WritesDidNotDrain,
        })
    );
}

#[tokio::test(flavor = "current_thread")]
async fn cloud_only_refetch_clears_exhausted_restore_all_marker() {
    let _guard = async_test_lock().lock().await;
    let manager = test_supervisor_manager();
    prepare_restore_all_marker(&manager);
    let mut supervisor = test_supervisor(
        Arc::downgrade(&manager),
        spawn_actor(CloudBackupWriteSupervisor::new(Weak::new())),
    );
    manager.apply_detail_outcome(
        manager.inventory_scope(),
        CloudBackupDetailOutcome::refreshed(listed_detail(&[])),
    );
    supervisor.active_cloud_only_fetch_request = Some(7);

    supervisor
        .complete_cloud_only_fetch_request(
            7,
            manager.inventory_scope(),
            Ok(complete_cloud_only_fetch(&manager, Vec::new())),
        )
        .await
        .unwrap();

    assert!(RustCloudBackupManager::load_persisted_state().pending_restore_all().is_none());
}

#[tokio::test(flavor = "current_thread")]
async fn overlapping_cloud_only_refetch_preserves_active_restore_all_marker_and_progress() {
    let _guard = async_test_lock().lock().await;
    let manager = test_supervisor_manager();
    prepare_restore_all_marker(&manager);
    let mut supervisor = test_supervisor(
        Arc::downgrade(&manager),
        spawn_actor(CloudBackupWriteSupervisor::new(Weak::new())),
    );
    let claim = begin_test_restore_all(&mut supervisor, Arc::new(AtomicBool::new(false)));
    manager.apply_restore_all_started(claim, 2);
    supervisor.active_cloud_only_fetch_request = Some(10);

    supervisor
        .complete_cloud_only_fetch_request(
            10,
            manager.inventory_scope(),
            Ok(complete_cloud_only_fetch(&manager, Vec::new())),
        )
        .await
        .unwrap();

    assert!(RustCloudBackupManager::load_persisted_state().pending_restore_all().is_some());
    assert!(matches!(
        manager.projected_restore_all_state(),
        CloudBackupRestoreAllState::Running { total: 2, .. }
    ));
    assert_eq!(supervisor.runtime, Some(claim));
}

#[tokio::test(flavor = "current_thread")]
async fn cloud_only_refetch_retains_restore_all_marker_when_wallets_remain() {
    let _guard = async_test_lock().lock().await;
    let manager = test_supervisor_manager();
    prepare_restore_all_marker(&manager);
    let mut supervisor = test_supervisor(
        Arc::downgrade(&manager),
        spawn_actor(CloudBackupWriteSupervisor::new(Weak::new())),
    );
    supervisor.active_cloud_only_fetch_request = Some(8);

    supervisor
        .complete_cloud_only_fetch_request(
            8,
            manager.inventory_scope(),
            Ok(complete_cloud_only_fetch(&manager, vec![test_cloud_only_wallet("remaining")])),
        )
        .await
        .unwrap();

    assert!(RustCloudBackupManager::load_persisted_state().pending_restore_all().is_some());
}

#[tokio::test(flavor = "current_thread")]
async fn cloud_only_refetch_failure_retains_restore_all_marker() {
    let _guard = async_test_lock().lock().await;
    let manager = test_supervisor_manager();
    prepare_restore_all_marker(&manager);
    let mut supervisor = test_supervisor(
        Arc::downgrade(&manager),
        spawn_actor(CloudBackupWriteSupervisor::new(Weak::new())),
    );
    supervisor.active_cloud_only_fetch_request = Some(9);

    supervisor
        .complete_cloud_only_fetch_request(
            9,
            manager.inventory_scope(),
            Err(CloudBackupError::Offline("offline".into())),
        )
        .await
        .unwrap();

    assert!(RustCloudBackupManager::load_persisted_state().pending_restore_all().is_some());
}

#[tokio::test(flavor = "current_thread")]
async fn restore_all_whose_cloud_only_rows_were_reset_mid_run_keeps_its_marker() {
    let _guard = async_test_lock().lock().await;
    let manager = test_supervisor_manager();
    prepare_restore_all_marker(&manager);
    manager.apply_detail_outcome(
        manager.inventory_scope(),
        CloudBackupDetailOutcome::refreshed(listed_detail(&["a", "b"])),
    );
    manager.apply_cloud_only_fetch_outcome(
        manager.inventory_scope(),
        complete_loaded_outcome(
            &manager,
            vec![test_cloud_only_wallet("a"), test_cloud_only_wallet("b")],
        ),
    );
    let mut supervisor = test_supervisor(
        Arc::downgrade(&manager),
        spawn_actor(CloudBackupWriteSupervisor::new(Weak::new())),
    );
    let claim = begin_test_restore_all(&mut supervisor, Arc::new(AtomicBool::new(false)));
    let detail_claim = supervisor.detail_workflow.start_operation_result(manager.inventory_scope());

    // the listing replaced wallet a with c at the same count
    supervisor
        .complete_restore_all_record_refresh(
            claim,
            detail_claim,
            Some(CloudBackupDetailResult::SuccessWithAuthority {
                listed: listed_detail(&["b", "c"]),
                authority: CloudBackupInventoryAuthority::ProviderConfirmed,
            }),
        )
        .await
        .unwrap()
        .await
        .unwrap();

    // the reset asks for a replacement fetch, which may already be running
    assert!(!manager
        .inventory
        .read()
        .cloud_only()
        .loaded_wallets()
        .is_some_and(|wallets| wallets.iter().any(|wallet| wallet.record_id == "a")));

    supervisor.complete_restore_all_queue_finished(claim).await.unwrap();

    assert_eq!(supervisor.runtime, None);
    assert!(RustCloudBackupManager::load_persisted_state().pending_restore_all().is_some());
}

#[tokio::test(flavor = "current_thread")]
async fn cloud_only_operation_refresh_that_resets_the_cache_starts_a_fetch() {
    let _guard = async_test_lock().lock().await;
    let manager = test_supervisor_manager();
    configure_enabled_cloud_backup(&manager, test_globals(), 2);
    let [a, b, c] = upload_distinct_test_wallet_backups(&manager);
    let [a, b, c] = [a.as_str(), b.as_str(), c.as_str()];
    manager.apply_detail_outcome(
        manager.inventory_scope(),
        CloudBackupDetailOutcome::refreshed(listed_detail(&[a, b])),
    );
    manager.apply_cloud_only_fetch_outcome(
        manager.inventory_scope(),
        complete_loaded_outcome(
            &manager,
            vec![test_cloud_only_wallet(a), test_cloud_only_wallet(b)],
        ),
    );
    list_test_wallet_files(&manager.current_namespace_id().unwrap(), &[b, c]);
    let mut supervisor = test_supervisor(
        Arc::downgrade(&manager),
        spawn_actor(CloudBackupWriteSupervisor::new(Weak::new())),
    );
    let claim = supervisor
        .begin_exclusive_operation(&manager, CloudBackupExclusiveOperation::RestoreCloudWallet)
        .unwrap();
    let detail_claim = supervisor.detail_workflow.start_operation_result(manager.inventory_scope());

    supervisor
        .complete_operation_refresh_detail(
            claim,
            detail_claim,
            Some(CloudBackupDetailResult::SuccessWithAuthority {
                listed: listed_detail(&[b, c]),
                authority: CloudBackupInventoryAuthority::ProviderConfirmed,
            }),
        )
        .await
        .unwrap();

    wait_for_cloud_only_rows(&manager, &[b, c], &[a]).await;
}

#[tokio::test(flavor = "current_thread")]
async fn trusted_local_detail_resets_cloud_only_rows_the_listing_replaced() {
    let _guard = async_test_lock().lock().await;
    let manager = test_supervisor_manager();
    configure_enabled_cloud_backup(&manager, test_globals(), 2);
    let [first, second, third] = upload_distinct_test_wallet_backups(&manager);
    let [first, second, third] = [first.as_str(), second.as_str(), third.as_str()];
    manager.apply_detail_outcome(
        manager.inventory_scope(),
        CloudBackupDetailOutcome::refreshed(listed_detail(&[first, second])),
    );
    manager.apply_cloud_only_fetch_outcome(
        manager.inventory_scope(),
        complete_loaded_outcome(
            &manager,
            vec![test_cloud_only_wallet(first), test_cloud_only_wallet(second)],
        ),
    );
    list_test_wallet_files(&manager.current_namespace_id().unwrap(), &[second, third]);
    let mut supervisor = test_supervisor(
        Arc::downgrade(&manager),
        spawn_actor(CloudBackupWriteSupervisor::new(Weak::new())),
    );
    let claim = begin_test_detail_refresh(&mut supervisor, &manager);

    // the trusted snapshot starts the fetch on this supervisor, so it must run as an actor for
    // that fetch to complete
    let supervisor = spawn_actor(supervisor);
    let confirmation = CloudBackupDetailProviderConfirmation {
        namespace: manager.current_namespace_id().unwrap(),
    };

    // same count and local rows, so only the carried record IDs show the replacement
    let snapshot = Some(CloudBackupDetailSnapshotCompletion::TrustedLocal {
        listed: listed_detail(&[second, third]),
        confirmation,
    });
    let scope = manager.inventory_scope();
    call!(supervisor.complete_refresh_detail_from_snapshot(
        snapshot,
        DetailRefreshAttempt::Initial,
        claim,
        scope
    ))
    .await
    .unwrap();

    wait_for_cloud_only_rows(&manager, &[second, third], &[first]).await;
}

#[tokio::test(flavor = "current_thread")]
async fn restore_all_queue_completion_without_remaining_wallets_clears_marker_and_claim() {
    let _guard = async_test_lock().lock().await;
    let manager = test_supervisor_manager();
    prepare_restore_all_marker(&manager);
    manager.apply_detail_outcome(
        manager.inventory_scope(),
        CloudBackupDetailOutcome::refreshed(listed_detail(&[])),
    );
    manager.apply_cloud_only_fetch_outcome(
        manager.inventory_scope(),
        complete_loaded_outcome(&manager, Vec::new()),
    );
    let mut supervisor = test_supervisor(
        Arc::downgrade(&manager),
        spawn_actor(CloudBackupWriteSupervisor::new(Weak::new())),
    );
    let claim = begin_test_restore_all(&mut supervisor, Arc::new(AtomicBool::new(false)));

    supervisor.complete_restore_all_queue_finished(claim).await.unwrap();

    assert_eq!(supervisor.runtime, None);
    assert!(RustCloudBackupManager::load_persisted_state().pending_restore_all().is_none());
}

#[tokio::test(flavor = "current_thread")]
async fn restore_all_cancellation_keeps_claim_until_record_boundary() {
    let _guard = async_test_lock().lock().await;
    let manager = test_supervisor_manager();
    persist_enabled_for_supervisor_test();
    let mut supervisor = test_supervisor(
        Arc::downgrade(&manager),
        spawn_actor(CloudBackupWriteSupervisor::new(Weak::new())),
    );
    let cancellation = Arc::new(AtomicBool::new(false));
    let claim = begin_test_restore_all(&mut supervisor, cancellation.clone());
    manager.apply_restore_all_started(claim, 1);

    supervisor.request_restore_all_cancellation();

    assert!(cancellation.load(Ordering::Acquire));
    assert_eq!(supervisor.runtime, Some(claim));
    assert_eq!(manager.projected_exclusive_operation(), Some(claim));
}

#[tokio::test(flavor = "current_thread")]
async fn supplemental_inventory_does_not_cancel_restore_all() {
    let _guard = async_test_lock().lock().await;
    let manager = test_supervisor_manager();
    persist_enabled_for_supervisor_test();
    let mut supervisor = test_supervisor(
        Arc::downgrade(&manager),
        spawn_actor(CloudBackupWriteSupervisor::new(Weak::new())),
    );
    let cancellation = Arc::new(AtomicBool::new(false));
    let claim = begin_test_restore_all(&mut supervisor, cancellation.clone());
    manager.apply_restore_all_started(claim, 1);

    supervisor.ensure_supplemental_inventory_discovery(manager.clone());

    assert!(!cancellation.load(Ordering::Acquire));
    assert_eq!(supervisor.runtime, Some(claim));
    assert_eq!(manager.projected_exclusive_operation(), Some(claim));
}


#[tokio::test(flavor = "current_thread")]
async fn completed_other_backups_scan_remains_cached() {
    let _guard = async_test_lock().lock().await;
    let manager = test_supervisor_manager();
    let mut supervisor = test_supervisor(
        Arc::downgrade(&manager),
        spawn_actor(CloudBackupWriteSupervisor::new(Weak::new())),
    );
    let loaded = CloudBackupOtherBackupsState::Loaded {
        summary: CloudBackupOtherBackupsSummary::default(),
    };
    manager.apply_other_backups_state(manager.inventory_scope(), loaded.clone());

    supervisor.ensure_supplemental_inventory_discovery(manager.clone());

    assert_eq!(manager.inventory.read().other_backups_state(), loaded);
}

#[tokio::test(flavor = "current_thread")]
async fn restore_all_mixed_failure_continues_through_later_success_and_settles() {
    let _guard = async_test_lock().lock().await;
    let manager = test_supervisor_manager();
    let first_wallet = xpub_only_wallet_metadata();
    let mut failed_wallet = xpub_only_wallet_metadata();
    failed_wallet.network = Network::Testnet;
    let mut final_wallet = xpub_only_wallet_metadata();
    final_wallet.network = Network::Signet;
    let first_entry = wallet_entry_with_labels(&first_wallet, None);
    let mut failed_entry = wallet_entry_with_labels(&failed_wallet, None);
    failed_entry.xpub = Some("not-an-xpub".into());
    let final_entry = wallet_entry_with_labels(&final_wallet, None);
    let (_, items) = prepare_restore_all_queue_fixture(
        &manager,
        vec![
            (first_wallet.clone(), first_entry),
            (failed_wallet.clone(), failed_entry),
            (final_wallet.clone(), final_entry),
        ],
    );

    call!(manager.supervisor.start_restore_all_operation(false, Some(tap(PermitIntent::RestoreAll)))).await.unwrap();
    wait_for_test_condition(Duration::from_secs(5), "Restore All should settle", || {
        manager.projected_exclusive_operation().is_none()
    })
    .await;

    assert!(
        Database::global()
            .wallets()
            .get(&first_wallet.id, first_wallet.network, first_wallet.wallet_mode)
            .unwrap()
            .is_some()
    );
    assert!(
        Database::global()
            .wallets()
            .get(&final_wallet.id, final_wallet.network, final_wallet.wallet_mode)
            .unwrap()
            .is_some()
    );
    assert!(
        Database::global()
            .wallets()
            .get(&failed_wallet.id, failed_wallet.network, failed_wallet.wallet_mode)
            .unwrap()
            .is_none()
    );
    // the restored wallets make the run read the rows again, and the failure survives that
    let failed_record_id = items[1].record_id.as_str();
    let rows = wait_for_rows_refetched_after_restore_all(&manager, &[failed_record_id], &[]).await;
    let retained_failure = rows
        .iter()
        .find(|wallet| wallet.record_id == failed_record_id)
        .and_then(|wallet| wallet.restore_failure.as_ref());
    assert!(retained_failure.is_some());
    assert!(RustCloudBackupManager::load_persisted_state().pending_restore_all().is_some());
    assert!(matches!(
        manager.projected_restore_all_state(),
        CloudBackupRestoreAllState::RetryAvailable { wallet_count: 1 }
    ));
}

#[tokio::test(flavor = "current_thread")]
async fn restore_all_provider_failure_stops_before_scheduling_the_next_wallet() {
    let _guard = async_test_lock().lock().await;
    let manager = test_supervisor_manager();
    let first_wallet = xpub_only_wallet_metadata();
    let mut second_wallet = xpub_only_wallet_metadata();
    second_wallet.network = Network::Testnet;
    let (namespace, items) = prepare_restore_all_queue_fixture(
        &manager,
        vec![
            (first_wallet.clone(), wallet_entry_with_labels(&first_wallet, None)),
            (second_wallet.clone(), wallet_entry_with_labels(&second_wallet, None)),
        ],
    );
    let globals = test_globals();
    globals.cloud.fail_wallet_backup_download_after_successes(
        namespace.clone(),
        items[0].record_id.clone(),
        1,
        CloudStorageError::Offline("provider disconnected".into()),
    );

    call!(manager.supervisor.start_restore_all_operation(false, Some(tap(PermitIntent::RestoreAll)))).await.unwrap();
    wait_for_test_condition(Duration::from_secs(2), "provider stop should settle", || {
        manager.projected_exclusive_operation().is_none()
    })
    .await;

    assert_eq!(
        globals
            .cloud
            .wallet_backup_download_attempt_count_for_record(&namespace, &items[0].record_id),
        2
    );
    assert_eq!(
        globals
            .cloud
            .wallet_backup_download_attempt_count_for_record(&namespace, &items[1].record_id),
        1
    );
    assert!(
        Database::global()
            .wallets()
            .get(&first_wallet.id, first_wallet.network, first_wallet.wallet_mode)
            .unwrap()
            .is_none()
    );
    assert!(
        Database::global()
            .wallets()
            .get(&second_wallet.id, second_wallet.network, second_wallet.wallet_mode)
            .unwrap()
            .is_none()
    );
    assert!(RustCloudBackupManager::load_persisted_state().pending_restore_all().is_some());
    assert!(matches!(
        manager.projected_restore_all_state(),
        CloudBackupRestoreAllState::RetryAvailable { wallet_count: 2 }
    ));
}

/// Sol PR 3 finding 2: Cancel ends Restore All's authority to present at once, while the run
/// keeps its claim until its preparation drains. The local key goes missing while the listing is
/// held, so the preparation reaches key recovery after the cancel, and recovery presents nothing
#[tokio::test(flavor = "current_thread")]
async fn restore_all_cancellation_revokes_key_recovery_still_to_come() {
    let _guard = async_test_lock().lock().await;
    let manager = test_supervisor_manager();
    // Restore All shows only with two or more wallets to restore
    let first_wallet = xpub_only_wallet_metadata();
    let mut second_wallet = xpub_only_wallet_metadata();
    second_wallet.network = Network::Testnet;
    let (namespace, items) = prepare_restore_all_queue_fixture(
        &manager,
        vec![
            (first_wallet.clone(), wallet_entry_with_labels(&first_wallet, None)),
            (second_wallet.clone(), wallet_entry_with_labels(&second_wallet, None)),
        ],
    );
    let globals = test_globals();

    // the cloud copy of the key and a discoverable passkey would recover it if anything asked
    let master_key = cove_cspp::Cspp::new(Keychain::global().clone())
        .load_master_key_from_store()
        .unwrap()
        .unwrap();
    let encrypted_master =
        cove_cspp::master_key_crypto::encrypt_master_key(&master_key, &[7; 32], &[9; 32]).unwrap();
    globals
        .cloud
        .set_master_key_backup(namespace.clone(), serde_json::to_vec(&encrypted_master).unwrap());
    globals.passkey.set_discover_result(Ok(cove_device::passkey::DiscoveredPasskeyResult {
        prf_output: vec![7; 32],
        credential_id: vec![1, 2, 3],
    }));

    let listing = globals.cloud.gate_wallet_backup_download_after_successes(
        namespace.clone(),
        items[0].record_id.clone(),
        0,
    );
    call!(manager.supervisor.start_restore_all_operation(false, Some(tap(PermitIntent::RestoreAll)))).await.unwrap();
    listing
        .wait_until_blocked()
        .with_timeout(loaded_host_wait(Duration::from_secs(2)))
        .await
        .expect("Restore All's listing should reach the held read");

    CloudBackupKeychain::global().delete_master_key().unwrap();
    cove_cspp::Cspp::<Keychain>::clear_cached_master_key();
    call!(manager.supervisor.cancel_restore_all_operation()).await.unwrap();
    assert!(matches!(
        manager.projected_restore_all_state(),
        CloudBackupRestoreAllState::Running { cancellation_requested: true, .. }
    ));

    listing.release();
    wait_for_test_condition(Duration::from_secs(5), "cancelled Restore All should settle", || {
        manager.projected_exclusive_operation().is_none()
    })
    .await;

    // the preparation reached key recovery, and recovery asked for no passkey
    assert_eq!(globals.cloud.master_key_download_attempt_count(), 1);
    assert_eq!(globals.passkey.discover_count(), 0);
    assert_eq!(globals.passkey.authenticate_count(), 0);

    // a cancelled run that drains clears its marker, as one cancelled during preparation does
    assert!(RustCloudBackupManager::load_persisted_state().pending_restore_all().is_none());
    for wallet in [&first_wallet, &second_wallet] {
        assert!(
            Database::global()
                .wallets()
                .get(&wallet.id, wallet.network, wallet.wallet_mode)
                .unwrap()
                .is_none()
        );
    }
}

#[tokio::test(flavor = "current_thread")]
async fn restore_all_cancellation_finishes_the_active_wallet_and_skips_the_next() {
    let _guard = async_test_lock().lock().await;
    let manager = test_supervisor_manager();
    let first_wallet = xpub_only_wallet_metadata();
    let mut second_wallet = xpub_only_wallet_metadata();
    second_wallet.network = Network::Testnet;
    let (namespace, items) = prepare_restore_all_queue_fixture(
        &manager,
        vec![
            (first_wallet.clone(), wallet_entry_with_labels(&first_wallet, None)),
            (second_wallet.clone(), wallet_entry_with_labels(&second_wallet, None)),
        ],
    );
    let globals = test_globals();
    let gate = globals.cloud.gate_wallet_backup_download_after_successes(
        namespace.clone(),
        items[0].record_id.clone(),
        1,
    );

    call!(manager.supervisor.start_restore_all_operation(false, Some(tap(PermitIntent::RestoreAll)))).await.unwrap();
    gate.wait_until_blocked()
        .with_timeout(loaded_host_wait(Duration::from_secs(2)))
        .await
        .expect("first wallet restore should reach atomic download");
    call!(manager.supervisor.cancel_restore_all_operation()).await.unwrap();
    assert!(matches!(
        manager.projected_restore_all_state(),
        CloudBackupRestoreAllState::Running { cancellation_requested: true, .. }
    ));
    gate.release();
    wait_for_test_condition(Duration::from_secs(5), "cancelled Restore All should settle", || {
        manager.projected_exclusive_operation().is_none()
    })
    .await;

    assert!(
        Database::global()
            .wallets()
            .get(&first_wallet.id, first_wallet.network, first_wallet.wallet_mode)
            .unwrap()
            .is_some()
    );
    assert!(
        Database::global()
            .wallets()
            .get(&second_wallet.id, second_wallet.network, second_wallet.wallet_mode)
            .unwrap()
            .is_none()
    );
    assert!(RustCloudBackupManager::load_persisted_state().pending_restore_all().is_some());

    // the restored first wallet makes the run read the rows again, and the retry counts the rest
    wait_for_rows_refetched_after_restore_all(
        &manager,
        &[items[1].record_id.as_str()],
        &[items[0].record_id.as_str()],
    )
    .await;
    assert!(matches!(
        manager.projected_restore_all_state(),
        CloudBackupRestoreAllState::RetryAvailable { wallet_count: 1 }
    ));
}

#[tokio::test(flavor = "current_thread")]
async fn reentering_detail_keeps_the_running_restore_all_batch() {
    let _guard = async_test_lock().lock().await;
    let manager = test_supervisor_manager();
    let first_wallet = xpub_only_wallet_metadata();
    let mut second_wallet = xpub_only_wallet_metadata();
    second_wallet.network = Network::Testnet;
    let (namespace, items) = prepare_restore_all_queue_fixture(
        &manager,
        vec![
            (first_wallet.clone(), wallet_entry_with_labels(&first_wallet, None)),
            (second_wallet.clone(), wallet_entry_with_labels(&second_wallet, None)),
        ],
    );

    let globals = test_globals();
    let gate = globals.cloud.gate_wallet_backup_download_after_successes(
        namespace.clone(),
        items[0].record_id.clone(),
        1,
    );

    call!(manager.supervisor.start_restore_all_operation(false, Some(tap(PermitIntent::RestoreAll)))).await.unwrap();
    gate.wait_until_blocked()
        .with_timeout(loaded_host_wait(Duration::from_secs(2)))
        .await
        .expect("first wallet restore should reach atomic download");

    let claim = manager.projected_exclusive_operation().expect("Restore All should be running");

    // leaving detail sends nothing to rust, so coming back is the only navigation it sees
    call!(manager.supervisor.start_enter_detail()).await.unwrap();
    assert_eq!(manager.projected_exclusive_operation(), Some(claim));
    assert!(matches!(
        manager.projected_restore_all_state(),
        CloudBackupRestoreAllState::Running { cancellation_requested: false, .. }
    ));

    gate.release();
    wait_for_test_condition(Duration::from_secs(5), "Restore All should settle", || {
        manager.projected_exclusive_operation().is_none()
    })
    .await;

    for wallet in [&first_wallet, &second_wallet] {
        assert!(
            Database::global()
                .wallets()
                .get(&wallet.id, wallet.network, wallet.wallet_mode)
                .unwrap()
                .is_some()
        );
    }

    assert!(RustCloudBackupManager::load_persisted_state().pending_restore_all().is_none());
}

#[tokio::test(flavor = "current_thread")]
async fn restore_all_cancellation_during_preparation_clears_marker_after_completion() {
    let _guard = async_test_lock().lock().await;
    let manager = test_supervisor_manager();
    prepare_restore_all_marker(&manager);
    let mut supervisor = test_supervisor(
        Arc::downgrade(&manager),
        spawn_actor(CloudBackupWriteSupervisor::new(Weak::new())),
    );
    let cancellation = Arc::new(AtomicBool::new(true));
    let claim = begin_test_restore_all(&mut supervisor, cancellation.clone());

    supervisor
        .complete_restore_all_preparation(
            claim,
            manager.inventory_scope(),
            Err(CloudBackupError::Offline("offline".into())),
        )
        .await
        .unwrap();

    assert_eq!(supervisor.runtime, None);
    assert!(RustCloudBackupManager::load_persisted_state().pending_restore_all().is_none());
}

#[tokio::test(flavor = "current_thread")]
async fn stale_restore_all_record_dispatch_does_not_mutate_current_claim() {
    let _guard = async_test_lock().lock().await;
    let manager = test_supervisor_manager();
    persist_enabled_for_supervisor_test();
    let mut supervisor = test_supervisor(
        Arc::downgrade(&manager),
        spawn_actor(CloudBackupWriteSupervisor::new(Weak::new())),
    );
    let current = begin_test_restore_all(&mut supervisor, Arc::new(AtomicBool::new(false)));
    manager.apply_restore_all_started(current, 1);
    let stale = CloudBackupExclusiveOperationClaim::new(
        CloudBackupExclusiveOperation::RestoreAllCloudWallets,
        operation_id(u64::MAX),
    );

    let applied = supervisor
        .begin_restore_all_record(stale, 0, test_cloud_only_wallet("stale"))
        .await
        .unwrap()
        .await
        .unwrap();

    assert!(!applied);
    assert_eq!(supervisor.runtime, Some(current));
    assert_eq!(manager.projected_exclusive_operation(), Some(current));
}

#[tokio::test(flavor = "current_thread")]
async fn restore_all_provider_failure_during_success_refresh_stops_with_marker_retained() {
    let _guard = async_test_lock().lock().await;
    let manager = test_supervisor_manager();
    prepare_restore_all_marker(&manager);
    let mut supervisor = test_supervisor(
        Arc::downgrade(&manager),
        spawn_actor(CloudBackupWriteSupervisor::new(Weak::new())),
    );
    let claim = begin_test_restore_all(&mut supervisor, Arc::new(AtomicBool::new(false)));
    let detail_claim = supervisor.detail_workflow.start_operation_result(manager.inventory_scope());

    let should_continue = supervisor
        .complete_restore_all_record_refresh(
            claim,
            detail_claim,
            Some(CloudBackupDetailResult::AccessError(CloudBackupError::Offline(
                "offline".into(),
            ))),
        )
        .await
        .unwrap()
        .await
        .unwrap();

    assert!(!should_continue);
    assert_eq!(supervisor.runtime, None);
    assert!(RustCloudBackupManager::load_persisted_state().pending_restore_all().is_some());
}

#[tokio::test(flavor = "current_thread")]
async fn ordinary_restore_all_record_failure_keeps_batch_claim_for_next_record() {
    let _guard = async_test_lock().lock().await;
    let manager = test_supervisor_manager();
    Database::global()
        .cloud_backup_state
        .set(&persisted_enabled_cloud_backup_state(Some(0)))
        .unwrap();
    manager.sync_persisted_state();
    let wallet = test_cloud_only_wallet("ordinary-failure");
    manager.apply_cloud_only_fetch_outcome(
        manager.inventory_scope(),
        complete_loaded_outcome(&manager, vec![wallet.clone()]),
    );
    let mut supervisor = test_supervisor(
        Arc::downgrade(&manager),
        spawn_actor(CloudBackupWriteSupervisor::new(Weak::new())),
    );
    let claim = begin_test_restore_all(&mut supervisor, Arc::new(AtomicBool::new(false)));

    let listing_generation = manager.inventory.read().cloud_only_listing_generation();
    let should_continue = supervisor
        .complete_restore_all_record(
            claim,
            1,
            wallet,
            listing_generation,
            Err(CloudBackupError::Internal("invalid wallet".into())),
        )
        .await
        .unwrap()
        .await
        .unwrap();

    assert!(should_continue);
    assert_eq!(supervisor.runtime, Some(claim));
}

#[tokio::test(flavor = "current_thread")]
async fn cloud_only_fetch_started_before_a_listing_change_is_discarded_and_refetched() {
    let _guard = async_test_lock().lock().await;
    let manager = test_supervisor_manager();
    configure_enabled_cloud_backup(&manager, test_globals(), 2);
    let [a, b, c] = upload_distinct_test_wallet_backups(&manager);
    let [a, b, c] = [a.as_str(), b.as_str(), c.as_str()];
    manager.apply_detail_outcome(
        manager.inventory_scope(),
        CloudBackupDetailOutcome::refreshed(listed_detail(&[a, b])),
    );
    let mut supervisor = test_supervisor(
        Arc::downgrade(&manager),
        spawn_actor(CloudBackupWriteSupervisor::new(Weak::new())),
    );
    supervisor.active_cloud_only_fetch_request = Some(5);
    let stale_fetch = complete_cloud_only_fetch(
        &manager,
        vec![test_cloud_only_wallet(a), test_cloud_only_wallet(b)],
    );
    manager.apply_cloud_only_fetch_outcome(
        manager.inventory_scope(),
        CloudBackupCloudOnlyFetchOutcome::Started,
    );

    // provider confirmation replaces wallet a with c at the same count while the fetch runs
    list_test_wallet_files(&manager.current_namespace_id().unwrap(), &[b, c]);
    manager.apply_detail_outcome(
        manager.inventory_scope(),
        CloudBackupDetailOutcome::refreshed(listed_detail(&[b, c])),
    );
    supervisor
        .complete_cloud_only_fetch_request(5, manager.inventory_scope(), Ok(stale_fetch))
        .await
        .unwrap();

    assert_eq!(supervisor.active_cloud_only_fetch_request, None);
    assert!(!manager
        .inventory
        .read()
        .cloud_only()
        .loaded_wallets()
        .is_some_and(|wallets| wallets.iter().any(|wallet| wallet.record_id == a)));
    wait_for_cloud_only_rows(&manager, &[b, c], &[a]).await;
}

#[tokio::test(flavor = "current_thread")]
async fn restore_all_keeps_its_marker_when_a_listed_record_cannot_be_read() {
    let _guard = async_test_lock().lock().await;
    let globals = test_globals();
    let manager = test_supervisor_manager();
    configure_enabled_cloud_backup(&manager, globals, 0);
    let namespace = manager.current_namespace_id().unwrap();
    crate::manager::cloud_backup_manager::owner::lifecycle::test_support::writer().persist_restore_all_marker(namespace.clone()).unwrap();
    manager.sync_persisted_state();

    // the provider lists the record but its download is not available yet
    let record_id = cove_cspp::backup_data::wallet_record_id("listed-but-unreadable");
    globals.cloud.set_wallet_files(
        namespace,
        vec![cove_cspp::backup_data::wallet_filename_from_record_id(&record_id)],
    );
    manager.apply_detail_outcome(
        manager.inventory_scope(),
        CloudBackupDetailOutcome::refreshed(listed_detail(&[record_id.as_str()])),
    );
    let mut supervisor = test_supervisor(
        Arc::downgrade(&manager),
        spawn_actor(CloudBackupWriteSupervisor::new(Weak::new())),
    );
    supervisor.active_cloud_only_fetch_request = Some(3);
    let generation = manager.inventory.read().cloud_only_listing_generation();
    let fetch = manager
        .do_fetch_cloud_only_wallets_with_client(CloudStorage::global_silent_client(), generation)
        .await
        .unwrap();
    let unreadable =
        CloudOnlyRow::unreadable(record_id, CloudBackupUnreadableKind::NotDownloadable);
    assert_eq!(fetch.rows, vec![unreadable.clone()]);

    supervisor
        .complete_cloud_only_fetch_request(3, manager.inventory_scope(), Ok(fetch))
        .await
        .unwrap();

    // the unread record keeps the list incomplete, so the retry stays shown with no restorable
    // row, but disabled, since nothing proves what it would restore
    assert!(RustCloudBackupManager::load_persisted_state().pending_restore_all().is_some());
    assert_eq!(
        public_cloud_only(&manager),
        Some(CloudOnlyState::Loaded { rows: vec![unreadable], incomplete: true, namespace_id: keychain_namespace() })
    );
    assert_eq!(
        manager.projected_restore_all_state(),
        CloudBackupRestoreAllState::RetryDisabled { wallet_count: 0 }
    );
    assert_eq!(
        dispatch_restore_all(&manager, UserIntent::RetryRestoreAllRemaining).await,
        (Some(Admission::Rejected(Rejection::RestoreAllUnavailable)), None)
    );
}

/// A confirmed listing whose fetch reads two restorable wallets and not a third record keeps
/// Restore All off, so a start can't restore the readable subset while the third may be a wallet
#[tokio::test(flavor = "current_thread")]
async fn restore_all_of_a_mixed_fetch_starts_only_once_every_listed_record_is_read() {
    let _guard = async_test_lock().lock().await;
    let manager = test_supervisor_manager();
    configure_enabled_cloud_backup(&manager, test_globals(), 0);
    let namespace = manager.current_namespace_id().unwrap();

    // the provider lists a third record whose download is not available yet
    let [a, b, c] = upload_distinct_test_wallet_backups(&manager);
    let [a, b, c] = [a.as_str(), b.as_str(), c.as_str()];
    let unread = cove_cspp::backup_data::wallet_record_id("listed-but-unreadable");
    let unread = unread.as_str();
    list_test_wallet_files(&namespace, &[a, b, unread]);
    manager.apply_detail_outcome(
        manager.inventory_scope(),
        CloudBackupDetailOutcome::refreshed(listed_detail(&[a, b, unread])),
    );

    call!(manager.supervisor.ensure_cloud_only_fetch()).await.unwrap();
    wait_for_test_condition(Duration::from_secs(5), "the mixed fetch should load", || {
        matches!(public_cloud_only(&manager), Some(CloudOnlyState::Loaded { incomplete: true, .. }))
    })
    .await;

    assert!(manager.listing_is_provider_confirmed());
    assert_eq!(
        manager.projected_restore_all_state(),
        CloudBackupRestoreAllState::StartDisabled { wallet_count: 2 }
    );
    assert_eq!(
        dispatch_restore_all(&manager, UserIntent::StartRestoreAll).await,
        (Some(Admission::Rejected(Rejection::RestoreAllUnavailable)), None)
    );
    assert!(RustCloudBackupManager::load_persisted_state().pending_restore_all().is_none());

    // the provider now lists a readable third wallet, and Check Again reads every record
    list_test_wallet_files(&namespace, &[a, b, c]);
    call!(manager.supervisor.start_refresh_detail()).await.unwrap();
    wait_for_cloud_only_rows(&manager, &[a, b, c], &[unread]).await;

    assert_eq!(
        manager.projected_restore_all_state(),
        CloudBackupRestoreAllState::StartAvailable { wallet_count: 3 }
    );
    assert_eq!(restore_all_batch_size(&manager), Some(3));
}

#[tokio::test(flavor = "current_thread")]
async fn incomplete_cloud_only_list_keeps_its_retry_and_check_again_reads_it_again() {
    let _guard = async_test_lock().lock().await;
    let manager = test_supervisor_manager();
    configure_enabled_cloud_backup(&manager, test_globals(), 0);
    let namespace = manager.current_namespace_id().unwrap();
    crate::manager::cloud_backup_manager::owner::lifecycle::test_support::writer().persist_restore_all_marker(namespace.clone()).unwrap();
    manager.sync_persisted_state();

    // the provider lists the record before its download is available
    let wallet = xpub_only_wallet_metadata();
    let record_id = cove_cspp::backup_data::wallet_record_id(wallet.id.as_ref());
    let record_id = record_id.as_str();
    list_test_wallet_files(&namespace, &[record_id]);
    manager.apply_detail_outcome(
        manager.inventory_scope(),
        CloudBackupDetailOutcome::refreshed(listed_detail(&[record_id])),
    );

    call!(manager.supervisor.ensure_cloud_only_fetch()).await.unwrap();
    let unreadable =
        CloudOnlyRow::unreadable(record_id.into(), CloudBackupUnreadableKind::NotDownloadable);
    wait_for_test_condition(Duration::from_secs(5), "the incomplete list should load", || {
        public_cloud_only(&manager)
            == Some(CloudOnlyState::Loaded { rows: vec![unreadable.clone()], incomplete: true, namespace_id: keychain_namespace() })
    })
    .await;
    assert_eq!(
        manager.projected_restore_all_state(),
        CloudBackupRestoreAllState::RetryDisabled { wallet_count: 0 }
    );
    assert_eq!(
        dispatch_restore_all(&manager, UserIntent::RetryRestoreAllRemaining).await,
        (Some(Admission::Rejected(Rejection::RestoreAllUnavailable)), None)
    );

    // Check Again refreshes the detail, which reads the incomplete list again
    upload_test_wallet_backup(&namespace, &wallet);
    call!(manager.supervisor.start_refresh_detail()).await.unwrap();

    let rows = wait_for_cloud_only_rows(&manager, &[record_id], &[]).await;
    assert_eq!(
        cloud_only_row_statuses(&rows),
        vec![(record_id, CloudBackupWalletStatus::DeletedFromDevice)]
    );
    assert!(matches!(
        public_cloud_only(&manager),
        Some(CloudOnlyState::Loaded { incomplete: false, .. })
    ));
    assert_eq!(
        manager.projected_restore_all_state(),
        CloudBackupRestoreAllState::RetryAvailable { wallet_count: 1 }
    );
}

#[tokio::test(flavor = "current_thread")]
async fn restore_all_preparation_with_only_unreadable_wallets_keeps_its_marker() {
    let _guard = async_test_lock().lock().await;
    let globals = test_globals();
    let manager = test_supervisor_manager();
    configure_enabled_cloud_backup(&manager, globals, 0);
    let namespace = manager.current_namespace_id().unwrap();
    crate::manager::cloud_backup_manager::owner::lifecycle::test_support::writer().persist_restore_all_marker(namespace.clone()).unwrap();
    manager.sync_persisted_state();

    // the selected wallet is listed but its download fails, so preparation queues nothing
    let record_id = cove_cspp::backup_data::wallet_record_id("selected-but-unreadable");
    globals.cloud.set_wallet_files(
        namespace,
        vec![cove_cspp::backup_data::wallet_filename_from_record_id(&record_id)],
    );
    manager.apply_detail_outcome(
        manager.inventory_scope(),
        CloudBackupDetailOutcome::refreshed(listed_detail(&[record_id.as_str()])),
    );
    let generation = manager.inventory.read().cloud_only_listing_generation();
    let prepared = manager
        .prepare_restore_all_cloud_wallets(
&unowned_writes(&manager,CloudBackupExclusiveOperation::RestoreAllCloudWallets),
vec![test_cloud_only_wallet(&record_id)], generation, permit(PermitIntent::RestoreAll))
        .await;

    let mut supervisor = test_supervisor(
        Arc::downgrade(&manager),
        spawn_actor(CloudBackupWriteSupervisor::new(Weak::new())),
    );
    let claim = begin_test_restore_all(&mut supervisor, Arc::new(AtomicBool::new(false)));
    supervisor
        .complete_restore_all_preparation(claim, manager.inventory_scope(), prepared)
        .await
        .unwrap();

    assert_eq!(supervisor.runtime, None);
    assert!(RustCloudBackupManager::load_persisted_state().pending_restore_all().is_some());
}

#[tokio::test(flavor = "current_thread")]
async fn restore_all_clears_its_marker_when_the_only_listed_record_duplicates_a_local_wallet() {
    let _guard = async_test_lock().lock().await;
    let manager = test_supervisor_manager();
    let fixture = seed_local_wallet_with_cloud_duplicate(&manager).await;
    let duplicate_record_id = fixture.duplicate_record_id.as_str();
    crate::manager::cloud_backup_manager::owner::lifecycle::test_support::writer().persist_restore_all_marker(fixture.namespace.clone()).unwrap();
    manager.sync_persisted_state();
    manager.apply_detail_outcome(
        manager.inventory_scope(),
        CloudBackupDetailOutcome::refreshed(listed_detail(&[duplicate_record_id])),
    );

    let generation = manager.inventory.read().cloud_only_listing_generation();
    let prepared = manager
        .prepare_restore_all_cloud_wallets(
            &unowned_writes(&manager,CloudBackupExclusiveOperation::RestoreAllCloudWallets),
            vec![test_cloud_only_wallet(duplicate_record_id)],
            generation,
            permit(PermitIntent::RestoreAll),
        )
        .await
        .unwrap();

    // the duplicate is read as a row that Restore All never queues
    assert_eq!(
        cloud_only_row_statuses(&fetch_wallets(prepared.authoritative_fetch())),
        vec![(duplicate_record_id, CloudBackupWalletStatus::AlreadyOnDevice)]
    );
    assert!(prepared.ordered_queue().is_empty());

    let mut supervisor = test_supervisor(
        Arc::downgrade(&manager),
        spawn_actor(CloudBackupWriteSupervisor::new(Weak::new())),
    );
    let claim = begin_test_restore_all(&mut supervisor, Arc::new(AtomicBool::new(false)));
    supervisor
        .complete_restore_all_preparation(claim, manager.inventory_scope(), Ok(prepared))
        .await
        .unwrap();

    assert_eq!(supervisor.runtime, None);
    assert!(RustCloudBackupManager::load_persisted_state().pending_restore_all().is_none());
    let Some(wallets) = manager.inventory.read().cloud_only().loaded_wallets() else {
        panic!("expected the duplicate row to stay loaded");
    };
    assert_eq!(
        cloud_only_row_statuses(&wallets),
        vec![(duplicate_record_id, CloudBackupWalletStatus::AlreadyOnDevice)]
    );
}

#[tokio::test(flavor = "current_thread")]
async fn identity_duplicate_row_can_be_deleted_and_then_leaves_the_turn_off_impact() {
    let _guard = async_test_lock().lock().await;
    let globals = test_globals();
    let manager = test_supervisor_manager();
    let fixture = seed_local_wallet_with_cloud_duplicate(&manager).await;
    let duplicate_record_id = fixture.duplicate_record_id.as_str();
    manager.apply_detail_outcome(
        manager.inventory_scope(),
        CloudBackupDetailOutcome::refreshed(listed_detail(&[duplicate_record_id])),
    );

    call!(manager.supervisor.ensure_cloud_only_fetch()).await.unwrap();
    let rows = wait_for_cloud_only_rows(&manager, &[duplicate_record_id], &[]).await;

    // the duplicate shows as a row, but Restore All has nothing to restore
    assert_eq!(
        cloud_only_row_statuses(&rows),
        vec![(duplicate_record_id, CloudBackupWalletStatus::AlreadyOnDevice)]
    );
    assert_eq!(restore_all_batch_size(&manager), Some(0));

    assert_eq!(turn_off_impact(&manager).await.extra_copies, 1);

    // an extra copy is deletable once a read finds the local wallet's own backup current
    let own_record_id = upload_own_backup(&manager, &fixture.local_wallet).await;
    read_inventory(&manager, &fixture.namespace, &[own_record_id.as_str(), duplicate_record_id])
        .await;
    let consent =
        row_delete_consent(&manager, duplicate_record_id).await.expect("the row offers Delete");
    call!(manager.supervisor.start_delete_cloud_wallet_operation(consent)).await.unwrap();
    wait_for_test_condition(Duration::from_secs(5), "the duplicate delete should finish", || {
        !globals.cloud.has_wallet_backup(&fixture.namespace, duplicate_record_id)
            && manager.projected_exclusive_operation().is_none()
    })
    .await;

    // only the cloud record went away, the local wallet and its own backup stay
    assert!(local_wallet_exists(&fixture.local_wallet));
    assert!(globals.cloud.has_wallet_backup(&fixture.namespace, &fixture.local_record_id));
    assert_eq!(turn_off_impact(&manager).await.extra_copies, 0);
}

/// The impact a Turn Off review would show now
async fn turn_off_impact(manager: &RustCloudBackupManager) -> CloudBackupDisableImpact {
    let evidence = manager.gather_disable_evidence().await.expect("Turn Off evidence");
    let facts = super::disable::impact_facts(manager);
    DisableImpact::assess(&evidence, &facts).screen()
}

#[tokio::test(flavor = "current_thread")]
async fn deleting_the_matching_local_wallet_makes_its_cloud_duplicate_restorable() {
    let _guard = async_test_lock().lock().await;
    let manager = test_supervisor_manager();
    let fixture = seed_local_wallet_with_cloud_duplicate(&manager).await;
    let duplicate_record_id = fixture.duplicate_record_id.as_str();
    manager.apply_detail_outcome(
        manager.inventory_scope(),
        CloudBackupDetailOutcome::refreshed(listed_detail(&[duplicate_record_id])),
    );

    call!(manager.supervisor.ensure_cloud_only_fetch()).await.unwrap();
    let rows = wait_for_cloud_only_rows(&manager, &[duplicate_record_id], &[]).await;
    assert_eq!(
        cloud_only_row_statuses(&rows),
        vec![(duplicate_record_id, CloudBackupWalletStatus::AlreadyOnDevice)]
    );

    let local_wallet = &fixture.local_wallet;
    Database::global()
        .wallets
        .remove_prepared_wallet_metadata(
            local_wallet.network,
            local_wallet.wallet_mode,
            &local_wallet.id,
        )
        .unwrap();
    manager.handle_local_wallets_changed();

    // the loaded row still says already on this device until a fetch reads it again
    wait_for_test_condition(Duration::from_secs(5), "the duplicate should become restorable", || {
        manager.inventory.read().cloud_only().loaded_wallets().is_some_and(|wallets| {
            cloud_only_row_statuses(&wallets)
                == vec![(duplicate_record_id, CloudBackupWalletStatus::DeletedFromDevice)]
        })
    })
    .await;
}

#[tokio::test(flavor = "current_thread")]
async fn deleted_wallet_leaves_the_local_rows_while_the_detail_refresh_fails() {
    let _guard = async_test_lock().lock().await;
    let manager = test_supervisor_manager();
    configure_enabled_cloud_backup(&manager, test_globals(), 0);
    let namespace = manager.current_namespace_id().unwrap();

    let local_wallet = xpub_only_wallet_metadata();
    let local_record_id = upload_test_wallet_backup(&namespace, &local_wallet);
    let local_restore = manager.prepare_active_namespace_wallet_restore(&unowned_writes(&manager, CloudBackupExclusiveOperation::RestoreCloudWallet), permit(PermitIntent::RestoreCloudWallet)).await.unwrap();
    local_restore.restore_record(&local_record_id).await.unwrap();
    list_test_wallet_files(&namespace, &[local_record_id.as_str()]);

    // the detail lists the wallet as backed up from this device
    let mut listed = listed_detail(&[]);
    listed.detail.up_to_date.push(CloudBackupWalletItem {
        sync_status: CloudBackupWalletStatus::Confirmed,
        ..test_cloud_only_wallet(&local_record_id)
    });
    manager.apply_detail_outcome(
        manager.inventory_scope(),
        CloudBackupDetailOutcome::refreshed(listed),
    );

    // from here every listing fails, so no detail refresh can correct the rows
    test_globals().cloud.fail_list_wallet_files("provider unavailable");

    Database::global()
        .wallets
        .remove_prepared_wallet_metadata(
            local_wallet.network,
            local_wallet.wallet_mode,
            &local_wallet.id,
        )
        .unwrap();
    manager.handle_local_wallets_changed();

    wait_for_test_condition(Duration::from_secs(5), "the deleted wallet leaves the local rows", || {
        public_detail(&manager).is_some_and(|detail| detail.up_to_date.is_empty())
    })
    .await;

    // its backup is still in the cloud, now as a record that is not on this device
    let detail = public_detail(&manager).unwrap();
    assert!(detail.needs_sync.is_empty());
}

#[tokio::test(flavor = "current_thread")]
async fn detail_built_before_a_wallet_deletion_does_not_list_it_as_local_again() {
    let _guard = async_test_lock().lock().await;
    let manager = test_supervisor_manager();
    configure_enabled_cloud_backup(&manager, test_globals(), 0);
    let namespace = manager.current_namespace_id().unwrap();

    let local_wallet = xpub_only_wallet_metadata();
    let local_record_id = upload_test_wallet_backup(&namespace, &local_wallet);
    let local_restore = manager.prepare_active_namespace_wallet_restore(&unowned_writes(&manager, CloudBackupExclusiveOperation::RestoreCloudWallet), permit(PermitIntent::RestoreCloudWallet)).await.unwrap();
    local_restore.restore_record(&local_record_id).await.unwrap();

    // a refresh and a verification both read the wallet as local before it is deleted
    let read_at = manager.inventory_read_at();
    let mut stale = listed_detail(&[]);
    stale.detail.up_to_date.push(CloudBackupWalletItem {
        sync_status: CloudBackupWalletStatus::Confirmed,
        ..test_cloud_only_wallet(&local_record_id)
    });
    manager.apply_detail_outcome(
        manager.inventory_scope(),
        CloudBackupDetailOutcome::refreshed(stale.clone()),
    );

    Database::global()
        .wallets
        .remove_prepared_wallet_metadata(
            local_wallet.network,
            local_wallet.wallet_mode,
            &local_wallet.id,
        )
        .unwrap();
    manager.handle_local_wallets_changed();

    wait_for_test_condition(Duration::from_secs(5), "the deleted wallet leaves the local rows", || {
        public_detail(&manager).is_some_and(|detail| detail.up_to_date.is_empty())
    })
    .await;

    // the cloud-only rows read after the deletion list its backup as not on this device
    manager.apply_cloud_only_fetch_outcome(
        manager.inventory_scope(),
        complete_loaded_outcome(&manager, vec![test_cloud_only_wallet(&local_record_id)]),
    );

    // the refresh finishes late, with the listing it read while the wallet was local
    manager.apply_detail_outcome(
        manager.inventory_scope(),
        CloudBackupDetailOutcome::refreshed(stale.clone()),
    );

    let detail = public_detail(&manager).unwrap();
    assert!(detail.up_to_date.is_empty(), "the deleted wallet is listed as local again");

    let cloud_only = public_cloud_only(&manager);
    assert!(
        matches!(
            &cloud_only,
            Some(CloudOnlyState::Loaded { rows, incomplete: false, .. })
                if rows.len() == 1 && rows[0].record_id == local_record_id
        ),
        "the late listing should still match the cloud-only rows, got {cloud_only:?}"
    );

    // the verification's detail finishes late too
    manager.handle_deep_verification_result(
        DeepVerificationResult::PasskeyConfirmed(Some(stale.detail)),
        read_at,
    );

    let detail = public_detail(&manager).unwrap();
    assert!(detail.up_to_date.is_empty(), "the verification listed the deleted wallet as local");
}

#[tokio::test(flavor = "current_thread")]
async fn verification_detail_read_before_a_restore_does_not_hide_the_restored_wallet() {
    let _guard = async_test_lock().lock().await;
    let manager = test_supervisor_manager();
    configure_enabled_cloud_backup(&manager, test_globals(), 0);
    let namespace = manager.current_namespace_id().unwrap();
    let restored_record_id = upload_test_wallet_backup(&namespace, &xpub_only_wallet_metadata());

    // the verification reads the local wallets before the restore adds one
    let read_at = manager.inventory_read_at();
    let read_before_restore = listed_detail(&[]).detail;
    manager.apply_detail_outcome(
        manager.inventory_scope(),
        CloudBackupDetailOutcome::refreshed(listed_detail(&[])),
    );

    let restore = manager.prepare_active_namespace_wallet_restore(&unowned_writes(&manager, CloudBackupExclusiveOperation::RestoreCloudWallet), permit(PermitIntent::RestoreCloudWallet)).await.unwrap();
    restore.restore_record(&restored_record_id).await.unwrap();
    manager.handle_local_wallets_changed();
    supervisor_checkpoint(&manager).await;
    assert_ne!(manager.inventory_read_at(), read_at, "the restore should move the epoch");

    // the refresh the change asked for lists the restored wallet as local
    let refreshed = manager.refresh_cloud_backup_detail(current_config(&manager).await).await.expect("a detail refresh");
    super::apply_refresh_detail_result(&manager, manager.inventory_scope(), &refreshed);

    let shows_restored = |manager: &RustCloudBackupManager| {
        public_detail(manager).is_some_and(|detail| {
            detail
                .up_to_date
                .iter()
                .chain(&detail.needs_sync)
                .any(|wallet| wallet.record_id == restored_record_id)
        })
    };
    assert!(shows_restored(&manager), "the refresh should list the restored wallet");

    // the restore is not a wallet-set change for the verification's own basis, so only the
    // inventory's epoch tells this detail is older than the restored wallet
    manager.handle_deep_verification_result(
        DeepVerificationResult::PasskeyConfirmed(Some(read_before_restore)),
        read_at,
    );

    assert!(shows_restored(&manager), "the verification's older detail hid the restored wallet");
}

#[tokio::test(flavor = "current_thread")]
async fn background_verification_that_resets_loaded_rows_starts_a_fetch() {
    let _guard = async_test_lock().lock().await;
    let manager = test_supervisor_manager();
    configure_enabled_cloud_backup(&manager, test_globals(), 2);
    let [a, b, c] = upload_distinct_test_wallet_backups(&manager);
    let [a, b, c] = [a.as_str(), b.as_str(), c.as_str()];
    manager.apply_detail_outcome(
        manager.inventory_scope(),
        CloudBackupDetailOutcome::refreshed(listed_detail(&[a, b])),
    );
    manager.apply_cloud_only_fetch_outcome(
        manager.inventory_scope(),
        complete_loaded_outcome(
            &manager,
            vec![test_cloud_only_wallet(a), test_cloud_only_wallet(b)],
        ),
    );
    let namespace = manager.current_namespace_id().unwrap();
    list_test_wallet_files(&namespace, &[b, c]);

    // the refetch is held on the newly listed record, so its rows cannot land before the checks
    let gate =
        test_globals().cloud.gate_wallet_backup_download_after_successes(namespace, c.into(), 0);

    // a background report carries no listing, so the loaded rows cannot be matched and reset
    manager.apply_verification_effect(CloudBackupVerificationEffect {
        detail: Some(VerificationDetail {
            detail: test_cloud_backup_detail(),
            read_at: manager.inventory_read_at(),
        }),
        ..Default::default()
    });

    // the reset is synchronous, but the supervisor may already have started the fetch it asked for
    assert!(matches!(
        manager.inventory.read().cloud_only(),
        CloudOnlyState::NotFetched | CloudOnlyState::Loading
    ));

    gate.wait_until_blocked()
        .with_timeout(loaded_host_wait(Duration::from_secs(5)))
        .await
        .expect("the reset should start a fetch that reads the new listing");
    assert_eq!(manager.inventory.read().cloud_only(), CloudOnlyState::Loading);

    gate.release();
    wait_for_cloud_only_rows(&manager, &[b, c], &[a]).await;
}

#[tokio::test(flavor = "current_thread")]
async fn seed_backup_matching_only_a_watch_only_wallet_restores_into_that_wallet() {
    let _guard = async_test_lock().lock().await;
    let manager = test_supervisor_manager();
    configure_enabled_cloud_backup(&manager, test_globals(), 0);
    let namespace = manager.current_namespace_id().unwrap();
    let secret = test_seed_secret();

    // a hardware import marks a public wallet verified without any recovery words
    let mut local_wallet = save_local_watch_only_wallet(&secret);
    local_wallet.verified = true;
    Database::global().wallets.update_wallet_metadata(local_wallet.clone()).unwrap();
    let (seed_wallet, seed_record_id) = upload_test_seed_backup(&namespace, &secret);
    assert!(!seed_wallet.verified);
    let seed_record_id = seed_record_id.as_str();
    list_test_wallet_files(&namespace, &[seed_record_id]);
    manager.apply_detail_outcome(
        manager.inventory_scope(),
        CloudBackupDetailOutcome::refreshed(listed_detail(&[seed_record_id])),
    );

    call!(manager.supervisor.ensure_cloud_only_fetch()).await.unwrap();
    let rows = wait_for_cloud_only_rows(&manager, &[seed_record_id], &[]).await;

    // the watch-only wallet cannot sign, so the seed backup is restorable rather than an extra copy
    assert_eq!(
        cloud_only_row_statuses(&rows),
        vec![(seed_record_id, CloudBackupWalletStatus::DeletedFromDevice)]
    );
    assert_eq!(restore_all_batch_size(&manager), Some(1));

    call!(manager.supervisor.start_restore_cloud_wallet_operation(seed_record_id.into(), Some(tap(PermitIntent::RestoreCloudWallet))))
        .await
        .unwrap();
    wait_for_test_condition(Duration::from_secs(5), "the restore should finish", || {
        manager.projected_exclusive_operation().is_none()
    })
    .await;

    // the seed went into the existing wallet instead of a second copy of it
    let local_wallets = local_wallets_like(&local_wallet);
    assert_eq!(local_wallets.len(), 1);
    assert_eq!(local_wallets[0].id, local_wallet.id);
    assert_eq!(local_wallets[0].wallet_type, WalletType::Hot);
    assert_eq!(Keychain::global().get_wallet_secret(&local_wallet.id).unwrap(), Some(secret));

    // the seed's words were never verified, so the wallet keeps warning before delete or wipe
    assert!(!local_wallets[0].verified);
    assert!(!local_wallet_exists(&seed_wallet));

    // the local wallet now holds the seed, so the cloud record is an extra copy
    wait_for_cloud_only_statuses(&manager, &[(
        seed_record_id,
        CloudBackupWalletStatus::AlreadyOnDevice,
    )])
    .await;
}

#[tokio::test(flavor = "current_thread")]
async fn seed_backup_matching_a_local_hot_wallet_is_already_on_device() {
    let _guard = async_test_lock().lock().await;
    let manager = test_supervisor_manager();
    configure_enabled_cloud_backup(&manager, test_globals(), 0);
    let namespace = manager.current_namespace_id().unwrap();
    let secret = test_seed_secret();

    let (local_wallet, local_record_id) = upload_test_seed_backup(&namespace, &secret);
    let local_restore = manager.prepare_active_namespace_wallet_restore(&unowned_writes(&manager, CloudBackupExclusiveOperation::RestoreCloudWallet), permit(PermitIntent::RestoreCloudWallet)).await.unwrap();
    local_restore.restore_record(&local_record_id).await.unwrap();
    assert!(local_wallet_exists(&local_wallet));

    let (_, copy_record_id) = upload_test_seed_backup(&namespace, &secret);
    let copy_record_id = copy_record_id.as_str();
    list_test_wallet_files(&namespace, &[copy_record_id]);
    manager.apply_detail_outcome(
        manager.inventory_scope(),
        CloudBackupDetailOutcome::refreshed(listed_detail(&[copy_record_id])),
    );

    call!(manager.supervisor.ensure_cloud_only_fetch()).await.unwrap();
    let rows = wait_for_cloud_only_rows(&manager, &[copy_record_id], &[]).await;

    assert_eq!(
        cloud_only_row_statuses(&rows),
        vec![(copy_record_id, CloudBackupWalletStatus::AlreadyOnDevice)]
    );
    assert_eq!(restore_all_batch_size(&manager), Some(0));
}

#[tokio::test(flavor = "current_thread")]
async fn restore_all_adds_a_seed_backup_to_the_matching_watch_only_wallet() {
    let _guard = async_test_lock().lock().await;
    let manager = test_supervisor_manager();
    let secret = test_seed_secret();
    let (seed_wallet, seed_entry) = test_seed_backup_entry(&secret);
    let mut other_wallet = xpub_only_wallet_metadata();
    other_wallet.network = Network::Testnet;
    let (_, items) = prepare_restore_all_queue_fixture(
        &manager,
        vec![
            (seed_wallet.clone(), seed_entry),
            (other_wallet.clone(), wallet_entry_with_labels(&other_wallet, None)),
        ],
    );
    let seed_record_id = items[0].record_id.as_str();

    // the fixture starts from no local wallets, so the watch-only wallet is added afterwards
    let local_wallet = save_local_watch_only_wallet(&secret);

    call!(manager.supervisor.start_restore_all_operation(false, Some(tap(PermitIntent::RestoreAll)))).await.unwrap();
    wait_for_test_condition(Duration::from_secs(5), "Restore All should settle", || {
        manager.projected_exclusive_operation().is_none()
    })
    .await;

    // the seed went into the existing wallet instead of a second copy of it
    let local_wallets = local_wallets_like(&local_wallet);
    assert_eq!(local_wallets.len(), 1);
    assert_eq!(local_wallets[0].id, local_wallet.id);
    assert_eq!(local_wallets[0].wallet_type, WalletType::Hot);
    assert_eq!(Keychain::global().get_wallet_secret(&local_wallet.id).unwrap(), Some(secret));
    assert!(local_wallet_exists(&other_wallet));
    assert!(RustCloudBackupManager::load_persisted_state().pending_restore_all().is_none());

    // the seed record stays in the cloud, now as an extra copy of the upgraded wallet
    wait_for_cloud_only_statuses(&manager, &[(
        seed_record_id,
        CloudBackupWalletStatus::AlreadyOnDevice,
    )])
    .await;
}

#[tokio::test(flavor = "current_thread")]
async fn record_deleted_between_the_detail_and_fetch_listings_keeps_the_list_incomplete() {
    let _guard = async_test_lock().lock().await;
    let manager = test_supervisor_manager();
    configure_enabled_cloud_backup(&manager, test_globals(), 0);
    let namespace = manager.current_namespace_id().unwrap();
    crate::manager::cloud_backup_manager::owner::lifecycle::test_support::writer().persist_restore_all_marker(namespace.clone()).unwrap();
    manager.sync_persisted_state();

    // the detail lists a record that another device deletes before the fetch lists again
    manager.apply_detail_outcome(
        manager.inventory_scope(),
        CloudBackupDetailOutcome::refreshed(listed_detail(&["deleted-elsewhere"])),
    );
    list_test_wallet_files(&namespace, &[]);

    call!(manager.supervisor.ensure_cloud_only_fetch()).await.unwrap();
    wait_for_test_condition(Duration::from_secs(5), "the fetch should load", || {
        public_cloud_only(&manager)
            == Some(CloudOnlyState::Loaded { rows: listed_rows(&Vec::new()), incomplete: true, namespace_id: keychain_namespace() })
    })
    .await;

    // the two listings disagree, so the marker stays, but a retry on an incomplete listing
    // can't prove the run done, so it stays disabled until Check Again
    assert!(RustCloudBackupManager::load_persisted_state().pending_restore_all().is_some());
    assert_eq!(
        manager.projected_restore_all_state(),
        CloudBackupRestoreAllState::RetryDisabled { wallet_count: 0 }
    );

    // Check Again lists the provider again, which settles the listing and clears the marker
    call!(manager.supervisor.start_refresh_detail()).await.unwrap();
    wait_for_test_condition(Duration::from_secs(5), "the new listing should settle", || {
        public_cloud_only(&manager)
            == Some(CloudOnlyState::Loaded { rows: listed_rows(&Vec::new()), incomplete: false, namespace_id: keychain_namespace() })
            && RustCloudBackupManager::load_persisted_state().pending_restore_all().is_none()
    })
    .await;
    assert_eq!(manager.projected_restore_all_state(), CloudBackupRestoreAllState::NotShown);
}

#[tokio::test(flavor = "current_thread")]
async fn restoring_one_of_three_copies_leaves_the_others_already_on_device() {
    let _guard = async_test_lock().lock().await;
    let manager = test_supervisor_manager();
    configure_enabled_cloud_backup(&manager, test_globals(), 0);
    let namespace = manager.current_namespace_id().unwrap();

    // test xpub-only wallets share one xpub, so these are three copies of one wallet
    let [first, second, third] = [(); 3]
        .map(|()| upload_test_wallet_backup(&namespace, &xpub_only_wallet_metadata()));
    let [first, second, third] = [first.as_str(), second.as_str(), third.as_str()];
    list_test_wallet_files(&namespace, &[first, second, third]);
    manager.apply_detail_outcome(
        manager.inventory_scope(),
        CloudBackupDetailOutcome::refreshed(listed_detail(&[first, second, third])),
    );

    call!(manager.supervisor.ensure_cloud_only_fetch()).await.unwrap();
    wait_for_cloud_only_statuses(&manager, &[
        (first, CloudBackupWalletStatus::DeletedFromDevice),
        (second, CloudBackupWalletStatus::DeletedFromDevice),
        (third, CloudBackupWalletStatus::DeletedFromDevice),
    ])
    .await;
    assert_eq!(
        manager.projected_restore_all_state(),
        CloudBackupRestoreAllState::StartAvailable { wallet_count: 3 }
    );

    call!(manager.supervisor.start_restore_cloud_wallet_operation(first.into(), Some(tap(PermitIntent::RestoreCloudWallet)))).await.unwrap();

    // the same record IDs stay listed, but the restored wallet now covers both of them
    wait_for_cloud_only_statuses(&manager, &[
        (second, CloudBackupWalletStatus::AlreadyOnDevice),
        (third, CloudBackupWalletStatus::AlreadyOnDevice),
    ])
    .await;
    wait_for_test_condition(Duration::from_secs(5), "the restore should finish", || {
        manager.projected_exclusive_operation().is_none()
    })
    .await;
    assert_eq!(restore_all_batch_size(&manager), Some(0));
    assert_eq!(manager.projected_restore_all_state(), CloudBackupRestoreAllState::NotShown);
}

#[tokio::test(flavor = "current_thread")]
async fn restore_all_restores_the_next_copy_after_its_matching_local_wallet_is_deleted() {
    let _guard = async_test_lock().lock().await;
    let manager = test_supervisor_manager();

    // test xpub-only wallets share one xpub, so these are two copies of one wallet
    let first_wallet = xpub_only_wallet_metadata();
    let second_wallet = xpub_only_wallet_metadata();
    let (namespace, items) = prepare_restore_all_queue_fixture(
        &manager,
        vec![
            (first_wallet.clone(), wallet_entry_with_labels(&first_wallet, None)),
            (second_wallet.clone(), wallet_entry_with_labels(&second_wallet, None)),
        ],
    );
    let first_record_id = items[0].record_id.as_str();
    let second_record_id = items[1].record_id.as_str();

    // preparation reads the second copy once, so the download that is held is its restore
    let gate = test_globals().cloud.gate_wallet_backup_download_after_successes(
        namespace,
        second_record_id.into(),
        1,
    );

    call!(manager.supervisor.start_restore_all_operation(false, Some(tap(PermitIntent::RestoreAll)))).await.unwrap();
    gate.wait_until_blocked()
        .with_timeout(loaded_host_wait(Duration::from_secs(2)))
        .await
        .expect("the second copy's restore download should block");
    assert!(local_wallet_exists(&first_wallet));

    // the restored first copy is deleted while the second copy is still downloading
    Database::global()
        .wallets
        .remove_prepared_wallet_metadata(
            first_wallet.network,
            first_wallet.wallet_mode,
            &first_wallet.id,
        )
        .unwrap();
    manager.handle_local_wallets_changed();
    gate.release();
    wait_for_test_condition(Duration::from_secs(5), "Restore All should settle", || {
        manager.projected_exclusive_operation().is_none()
    })
    .await;

    // nothing on the device held the second copy anymore, so it was restored rather than skipped
    assert!(local_wallet_exists(&second_wallet));

    // the deleted wallet's record is cloud-only again, and the second wallet now covers it
    wait_for_cloud_only_statuses(&manager, &[(
        first_record_id,
        CloudBackupWalletStatus::AlreadyOnDevice,
    )])
    .await;

    // the deletion reset the cloud-only rows mid-run, so the run can end before the replacement
    // fetch proves nothing is left, and then that fetch clears the marker rather than the run
    wait_for_test_condition(Duration::from_secs(5), "the Restore All marker should clear", || {
        RustCloudBackupManager::load_persisted_state().pending_restore_all().is_none()
    })
    .await;
}

#[tokio::test(flavor = "current_thread")]
async fn seed_backup_stays_restorable_when_the_local_secret_cannot_be_read() {
    let _guard = async_test_lock().lock().await;
    let manager = test_supervisor_manager();
    configure_enabled_cloud_backup(&manager, test_globals(), 0);
    let namespace = manager.current_namespace_id().unwrap();
    let secret = test_seed_secret();

    // a hot wallet whose stored seed cannot be decrypted cannot sign with it
    let mut local_wallet = save_local_watch_only_wallet(&secret);
    local_wallet.wallet_type = WalletType::Hot;
    Database::global().wallets.update_wallet_metadata(local_wallet.clone()).unwrap();
    let secret_key = format!("{}::wallet_mnemonic", local_wallet.id);
    let cryptor_key = format!("{}::wallet_mnemonic_encryption_key_and_nonce", local_wallet.id);
    Keychain::global().save(secret_key.clone(), "garbage-secret".into()).unwrap();
    Keychain::global().save(cryptor_key.clone(), "garbage-cryptor".into()).unwrap();

    let (_, seed_record_id) = upload_test_seed_backup(&namespace, &secret);
    let seed_record_id = seed_record_id.as_str();
    list_test_wallet_files(&namespace, &[seed_record_id]);
    manager.apply_detail_outcome(
        manager.inventory_scope(),
        CloudBackupDetailOutcome::refreshed(listed_detail(&[seed_record_id])),
    );

    call!(manager.supervisor.ensure_cloud_only_fetch()).await.unwrap();
    wait_for_cloud_only_statuses(&manager, &[(
        seed_record_id,
        CloudBackupWalletStatus::DeletedFromDevice,
    )])
    .await;

    call!(manager.supervisor.start_restore_cloud_wallet_operation(seed_record_id.into(), Some(tap(PermitIntent::RestoreCloudWallet))))
        .await
        .unwrap();
    wait_for_test_condition(Duration::from_secs(5), "the restore should finish", || {
        manager.projected_exclusive_operation().is_none()
    })
    .await;

    // the restore fails without touching the unreadable seed, and the backup is not a copy
    let Some(wallets) = manager.inventory.read().cloud_only().loaded_wallets() else {
        panic!("expected the seed row to stay loaded");
    };
    assert_eq!(
        cloud_only_row_statuses(&wallets),
        vec![(seed_record_id, CloudBackupWalletStatus::DeletedFromDevice)]
    );
    assert!(wallets[0].restore_failure.is_some());
    assert_eq!(Keychain::global().get(secret_key).unwrap(), Some("garbage-secret".to_string()));
    assert_eq!(Keychain::global().get(cryptor_key).unwrap(), Some("garbage-cryptor".to_string()));
    assert_eq!(local_wallets_like(&local_wallet).len(), 1);
}

#[tokio::test(flavor = "current_thread")]
async fn unreadable_hot_wallet_does_not_cover_other_public_backups_with_its_fingerprint() {
    let _guard = async_test_lock().lock().await;
    let manager = test_supervisor_manager();
    configure_enabled_cloud_backup(&manager, test_globals(), 0);
    let namespace = manager.current_namespace_id().unwrap();
    let secret = test_seed_secret();

    // a hot wallet whose stored seed cannot be decrypted still has its own public descriptors
    let mut local_wallet = save_local_watch_only_wallet(&secret);
    local_wallet.wallet_type = WalletType::Hot;
    Database::global().wallets.update_wallet_metadata(local_wallet.clone()).unwrap();
    let keychain = Keychain::global();
    keychain.save(format!("{}::wallet_mnemonic", local_wallet.id), "garbage-secret".into()).unwrap();
    keychain
        .save(
            format!("{}::wallet_mnemonic_encryption_key_and_nonce", local_wallet.id),
            "garbage-cryptor".into(),
        )
        .unwrap();

    // another public wallet under the same fingerprint, backed up only in the cloud
    let mut public_wallet = WalletMetadata::preview_new();
    public_wallet.id = WalletId::new();
    public_wallet.wallet_type = WalletType::WatchOnly;
    public_wallet.master_fingerprint = local_wallet.master_fingerprint.clone();
    let public_record_id = upload_test_wallet_backup(&namespace, &public_wallet);
    let public_record_id = public_record_id.as_str();
    list_test_wallet_files(&namespace, &[public_record_id]);
    manager.apply_detail_outcome(
        manager.inventory_scope(),
        CloudBackupDetailOutcome::refreshed(listed_detail(&[public_record_id])),
    );

    call!(manager.supervisor.ensure_cloud_only_fetch()).await.unwrap();

    // the local wallet cannot sign and does not hold that public wallet, so its backup is no copy
    wait_for_cloud_only_statuses(&manager, &[(
        public_record_id,
        CloudBackupWalletStatus::DeletedFromDevice,
    )])
    .await;
}

#[tokio::test(flavor = "current_thread")]
async fn seed_backup_watched_under_two_address_types_restores_as_its_own_wallet() {
    let _guard = async_test_lock().lock().await;
    let manager = test_supervisor_manager();
    configure_enabled_cloud_backup(&manager, test_globals(), 0);
    let namespace = manager.current_namespace_id().unwrap();
    let secret = test_seed_secret();
    let native_segwit = save_local_watch_only_wallet(&secret);
    let wrapped_segwit =
        save_local_watch_only_wallet_with_address_type(&secret, WalletAddressType::WrappedSegwit);
    let (seed_wallet, seed_record_id) = upload_test_seed_backup(&namespace, &secret);

    let prepared = manager.prepare_active_namespace_wallet_restore(&unowned_writes(&manager, CloudBackupExclusiveOperation::RestoreCloudWallet), permit(PermitIntent::RestoreCloudWallet)).await.unwrap();
    let outcome = prepared.restore_record(&seed_record_id).await.unwrap();

    // both watch-only wallets match the seed, so it becomes its own hot wallet
    assert!(matches!(outcome, CloudOnlyRestoreOutcome::Restored { .. }));
    let local_wallets = local_wallets_like(&seed_wallet);
    let wallet_type = |id: &WalletId| {
        local_wallets.iter().find(|wallet| &wallet.id == id).map(|wallet| wallet.wallet_type)
    };
    assert_eq!(local_wallets.len(), 3);
    assert_eq!(wallet_type(&seed_wallet.id), Some(WalletType::Hot));
    assert_eq!(wallet_type(&native_segwit.id), Some(WalletType::WatchOnly));
    assert_eq!(wallet_type(&wrapped_segwit.id), Some(WalletType::WatchOnly));
}

/// The write lane drained for the switch `claim` started, which asks the platform for an account
///
/// The supervisor under test has no actor address, so the drain's completion is handed to it here
async fn write_drain_completed(supervisor: &mut CloudBackupSupervisor, claim: CloudBackupExclusiveOperationClaim) {
    supervisor.complete_drive_account_switch_write_drain(claim, true).await.unwrap().await.unwrap();
}

/// Hands `result` to the supervisor under test and waits until it has handled it
async fn platform_result(supervisor: &mut CloudBackupSupervisor, result: DriveAccountSwitchPlatformResult) {
    supervisor.drive_account_switch_platform_result(result).await.unwrap().await.unwrap();
}

fn persisted_drive_account_switch_phase() -> Option<PersistedDriveAccountSwitchPhase> {
    RustCloudBackupManager::load_persisted_state()
        .drive_account_switch()
        .map(|account_switch| account_switch.phase)
}

fn configured_cloud_backup_state(
    manager: &RustCloudBackupManager,
) -> crate::manager::cloud_backup_manager::model::CloudBackupConfiguredState {
    let CloudBackupLifecycle::Configured(configured) = manager.model_state().lifecycle else {
        panic!("expected configured cloud backup lifecycle");
    };

    configured
}

/// Drains the reconcile queue and counts the commit requests for one transition
fn drained_drive_account_commit_requests(
    manager: &RustCloudBackupManager,
    transition_id: u64,
) -> usize {
    let is_commit_request = |message: &CloudBackupReconcileMessage| {
        matches!(
            message,
            CloudBackupReconcileMessage::DriveAccountSwitchCommitRequired(id)
                if *id == transition_id
        )
    };

    manager
        .reconciler
        .receiver()
        .try_iter()
        .map(|messages| match messages {
            SingleOrMany::Single(message) => usize::from(is_commit_request(&message)),
            SingleOrMany::Many(messages) => {
                messages.iter().filter(|message| is_commit_request(message)).count()
            }
        })
        .sum()
}

#[tokio::test(flavor = "current_thread")]
async fn drive_account_switch_continue_reinitializes_retries_and_commits() {
    use crate::manager::cloud_backup_manager::model::CloudBackupDestructiveOperationState as Destructive;

    let _guard = async_test_lock().lock().await;
    let manager = test_supervisor_manager();
    configure_enabled_cloud_backup(&manager, test_globals(), 0);
    seed_verifiable_cloud_master_key(test_globals());
    let mut supervisor = test_supervisor(
        Arc::downgrade(&manager),
        spawn_actor(CloudBackupWriteSupervisor::new(Arc::downgrade(&manager))),
    );

    supervisor.dispatch(UserIntent::SwitchDriveAccount).await.unwrap().await.unwrap();
    let transition_id = RustCloudBackupManager::load_persisted_state()
        .drive_account_switch()
        .expect("the switch is recorded")
        .transition_id;
    let claim = supervisor.runtime.claim().expect("the switch runs");
    assert_eq!(claim.drive_account_switch_id(), Some(transition_id));
    write_drain_completed(&mut supervisor, claim).await;

    // a selection staged for another transition moves nothing
    platform_result(
        &mut supervisor,
        DriveAccountSwitchPlatformResult::AccountSelected(transition_id.value() + 1),
    )
    .await;

    assert_eq!(
        persisted_drive_account_switch_phase(),
        Some(PersistedDriveAccountSwitchPhase::AwaitingAccountSelection)
    );

    // the user picked the new account: the backup is reinitialized there under the switch's claim
    platform_result(
        &mut supervisor,
        DriveAccountSwitchPlatformResult::AccountSelected(transition_id.value()),
    )
    .await;

    assert_eq!(
        persisted_drive_account_switch_phase(),
        Some(PersistedDriveAccountSwitchPhase::Reinitializing)
    );
    assert_eq!(supervisor.runtime.claim(), Some(claim));
    assert_eq!(manager.projected_exclusive_operation(), Some(claim));
    assert_eq!(
        configured_cloud_backup_state(&manager).destructive_operation,
        Destructive::ReinitializingBackup
    );

    // a repeated selection answers no step the switch waits on, so the running reinitialize goes
    // on alone
    platform_result(
        &mut supervisor,
        DriveAccountSwitchPlatformResult::AccountSelected(transition_id.value()),
    )
    .await;

    assert_eq!(supervisor.runtime.claim(), Some(claim));

    // reinitializing in the new account fails: the switch waits for a retry, never a commit
    supervisor
        .complete_enable_preparation(
            claim,
            Err(CloudBackupError::Internal("new account upload failed".into())),
        )
        .await
        .unwrap();

    assert_eq!(
        persisted_drive_account_switch_phase(),
        Some(PersistedDriveAccountSwitchPhase::AwaitingReinitializationRetry)
    );
    assert_eq!(supervisor.runtime, None);
    assert_eq!(manager.projected_exclusive_operation(), None);
    assert_eq!(configured_cloud_backup_state(&manager).destructive_operation, Destructive::Idle);
    assert_eq!(drained_drive_account_commit_requests(&manager, transition_id.value()), 0);

    // Try Again on the switch's recovery restarts the same switch in a new run, not a plain
    // reinitialize
    assert!(manager.state.read().drive_account_switch_recovery().is_some());
    supervisor.dispatch(UserIntent::RetryDriveAccountSwitchRecovery).await.unwrap().await.unwrap();
    let failed_run = claim;
    let claim = supervisor.runtime.claim().expect("the retry runs");

    assert_ne!(claim, failed_run);
    assert_eq!(claim.drive_account_switch_id(), Some(transition_id));
    assert_eq!(
        persisted_drive_account_switch_phase(),
        Some(PersistedDriveAccountSwitchPhase::Reinitializing)
    );
    assert_eq!(manager.projected_exclusive_operation(), Some(claim));
    assert_eq!(
        configured_cloud_backup_state(&manager).destructive_operation,
        Destructive::ReinitializingBackup
    );

    // the retry succeeds: the platform is asked to commit the account exactly once
    supervisor.finish_enable_operation(manager.clone(), claim);

    assert_eq!(
        persisted_drive_account_switch_phase(),
        Some(PersistedDriveAccountSwitchPhase::AwaitingAccountCommitSucceeded)
    );
    assert_eq!(drained_drive_account_commit_requests(&manager, transition_id.value()), 1);
    assert_eq!(supervisor.runtime.claim(), Some(claim));

    // once the platform commits, the switch ends like any reinitialize and the new account's backup
    // is verified
    platform_result(
        &mut supervisor,
        DriveAccountSwitchPlatformResult::Committed(transition_id.value()),
    )
    .await;

    assert_eq!(persisted_drive_account_switch_phase(), None);
    assert_eq!(
        manager.drained_drive_account_switch_steps(),
        vec![RequestedDriveAccountSwitchStep::Finalize(transition_id.value())]
    );
    assert_reinitialized_backup_verification_reads_cloud(&mut supervisor, &manager, claim).await;
}

#[tokio::test(flavor = "current_thread")]
async fn reinitialize_backup_verification_reads_the_new_backup() {
    use crate::manager::cloud_backup_manager::model::CloudBackupDestructiveOperationState as Destructive;

    let _guard = async_test_lock().lock().await;
    let manager = test_supervisor_manager();
    configure_enabled_cloud_backup(&manager, test_globals(), 0);
    seed_verifiable_cloud_master_key(test_globals());
    let mut supervisor = test_supervisor(
        Arc::downgrade(&manager),
        spawn_actor(CloudBackupWriteSupervisor::new(Arc::downgrade(&manager))),
    );

    supervisor.begin_recovery_operation(RecoveryAction::ReinitializeBackup, Some(tap(PermitIntent::Reinitialize)));
    let claim = supervisor.runtime.claim().unwrap();

    assert_eq!(claim.operation(), CloudBackupExclusiveOperation::ReinitializeBackup);
    assert_eq!(manager.state.read().status(), CloudBackupStatus::Enabling);
    assert_eq!(
        configured_cloud_backup_state(&manager).destructive_operation,
        Destructive::ReinitializingBackup
    );

    supervisor.finish_enable_operation(manager.clone(), claim);

    assert_reinitialized_backup_verification_reads_cloud(&mut supervisor, &manager, claim).await;
}

/// Runs the verification a finished reinitialize scheduled and checks it read the new backup
///
/// The supervisor here is not spawned, so this calls the same preparation step its scheduled
/// verification runs and hands the result back the way that task would
async fn assert_reinitialized_backup_verification_reads_cloud(
    supervisor: &mut CloudBackupSupervisor,
    manager: &Arc<RustCloudBackupManager>,
    reinitialize_claim: CloudBackupExclusiveOperationClaim,
) {
    use crate::manager::cloud_backup_manager::model::{
        CloudBackupDestructiveOperationState as Destructive, CloudBackupVerificationState,
    };

    let globals = test_globals();

    assert_eq!(
        configured_cloud_backup_state(manager).verification,
        CloudBackupVerificationState::Running
    );

    assert_eq!(
        configured_cloud_backup_state(manager).destructive_operation,
        Destructive::ReinitializingBackup
    );

    let master_key_reads = globals.cloud.master_key_download_attempt_count();
    let claim = supervisor.runtime.claim().unwrap();
    let authority = supervisor.runtime.authority(claim);
    let step = manager
        .prepare_deep_verify_cloud_backup(
            CloudBackupWriteClient::for_operation(supervisor.write.clone(), claim),
            supervisor.runtime.generation(),
            false,
            authority,
        )
        .await;

    assert!(
        !matches!(step, CloudBackupDeepVerificationStep::Complete(DeepVerificationResult::NotEnabled)),
        "verification stopped before reading the cloud"
    );

    assert!(globals.cloud.master_key_download_attempt_count() > master_key_reads);

    // it could read the cloud because the backup reads as enabled again, while the verification
    // holds the slot under its own claim and the reinitialize stays on screen
    let verification_claim = supervisor.runtime.claim().unwrap();

    assert_eq!(manager.state.read().status(), CloudBackupStatus::Enabled);
    assert_ne!(supervisor.projected_screen(manager).settings_row, CloudBackupSettingsRowStatus::SettingUp);
    assert_ne!(verification_claim, reinitialize_claim);
    assert_eq!(
        verification_claim.operation(),
        CloudBackupExclusiveOperation::VerifyReinitializedBackup
    );
    assert_eq!(manager.projected_exclusive_operation(), Some(verification_claim));

    // a completion still carrying the reinitialize claim is stale
    supervisor
        .complete_verification(
            VerificationRun {
                claim: reinitialize_claim,
                continuation: DeepVerificationContinuation::ReinitializeBackup,
                basis: supervisor.evidence_basis(),
                read_at: manager.inventory_read_at(),
            },
            CloudBackupDeepVerificationStep::Complete(DeepVerificationResult::NotEnabled),
        )
        .await
        .unwrap();

    assert_eq!(supervisor.runtime.claim(), Some(verification_claim));

    // the verification's own result reports the new backup verified, and releases the slot and
    // the spinner
    supervisor
        .complete_verification(
            VerificationRun {
                claim: verification_claim,
                continuation: DeepVerificationContinuation::ReinitializeBackup,
                basis: supervisor.evidence_basis(),
                read_at: manager.inventory_read_at(),
            },
            step,
        )
        .await
        .unwrap();

    assert_eq!(supervisor.runtime, None);
    assert_eq!(manager.projected_exclusive_operation(), None);
    assert_eq!(configured_cloud_backup_state(manager).destructive_operation, Destructive::Idle);
    assert!(matches!(
        configured_cloud_backup_state(manager).verification,
        CloudBackupVerificationState::Verified { .. }
    ));
}

fn persisted_verification() -> crate::database::cloud_backup::PersistedBackupVerificationState {
    let PersistedCloudBackupState::Configured(configured) =
        RustCloudBackupManager::load_persisted_state()
    else {
        panic!("expected a configured Cloud Backup record");
    };

    configured.verification
}

/// Verification evidence that arrives after a wallet change has one writer: the owner drops it
#[tokio::test(flavor = "current_thread")]
async fn verification_evidence_from_before_a_wallet_change_does_not_undo_its_reset() {
    use crate::database::cloud_backup::PersistedBackupVerificationState;
    use crate::manager::cloud_backup_manager::model::CloudBackupVerificationState;

    let _guard = async_test_lock().lock().await;
    let manager = test_supervisor_manager();
    persist_enabled_for_supervisor_test();
    let mut supervisor = test_supervisor(
        Arc::downgrade(&manager),
        spawn_actor(CloudBackupWriteSupervisor::new(Weak::new())),
    );
    let verified = || {
        CloudBackupDeepVerificationStep::Complete(DeepVerificationResult::Verified(
            test_pending_completion(VALID_NAMESPACE_ID.into(), Vec::new()).report(),
        ))
    };

    // a manual verification reads the wallets, then a wallet is added before its evidence lands
    let Produces::Value(run) = supervisor.begin_verification_run_for_test(false).await.unwrap()
    else {
        panic!("verification run was not produced");
    };
    supervisor.require_verification_after_wallet_change(20).await.unwrap();
    supervisor.complete_verification(run, verified()).await.unwrap();

    assert!(
        matches!(
            persisted_verification(),
            PersistedBackupVerificationState::Required { requested_at: Some(20), .. }
        ),
        "old evidence overwrote the wallet-change reset: {:?}",
        persisted_verification()
    );

    // nor does the screen show the older result as verified
    assert!(
        !matches!(
            configured_cloud_backup_state(&manager).verification,
            CloudBackupVerificationState::Verified { .. }
        ),
        "old evidence showed as verified: {:?}",
        configured_cloud_backup_state(&manager).verification
    );

    // a verification that starts after the change reads the new wallet set, so it counts
    let Produces::Value(run) = supervisor.begin_verification_run_for_test(false).await.unwrap()
    else {
        panic!("verification run was not produced");
    };
    supervisor.complete_verification(run, verified()).await.unwrap();

    assert!(matches!(persisted_verification(), PersistedBackupVerificationState::Verified { .. }));
    assert!(matches!(
        configured_cloud_backup_state(&manager).verification,
        CloudBackupVerificationState::Verified { .. }
    ));
}

#[tokio::test(flavor = "current_thread")]
async fn write_lane_result_for_an_ended_operation_is_discarded() {
    let _guard = async_test_lock().lock().await;
    let manager = test_supervisor_manager();
    persist_enabled_for_supervisor_test();
    let mut supervisor = test_supervisor(
        Arc::downgrade(&manager),
        spawn_actor(CloudBackupWriteSupervisor::new(Weak::new())),
    );
    let wallet_count = || RustCloudBackupManager::load_persisted_state().wallet_count();
    let before = wallet_count();

    let ended = supervisor
        .begin_exclusive_operation(&manager, CloudBackupExclusiveOperation::RecreateManifest)
        .unwrap();
    supervisor.complete_exclusive_operation(ended).await.unwrap();
    let running = supervisor
        .begin_exclusive_operation(&manager, CloudBackupExclusiveOperation::RepairPasskey)
        .unwrap();

    // the ended operation's upload finished late; the owner refuses it and the write lane
    // reports it cancelled instead of marking wallets uploaded
    let Produces::Value(late) = supervisor
        .record_uploaded_wallets(
            Some(ended),
            9,
            CloudBackupUploadedWalletsStateMode::PreserveVerification,
        )
        .await
        .unwrap()
    else {
        panic!("the supervisor answers the write lane");
    };

    assert!(matches!(late, Err(CloudBackupError::Cancelled)));
    assert_eq!(wallet_count(), before);
    assert_eq!(supervisor.runtime, Some(running));

    let Produces::Value(current) = supervisor
        .record_uploaded_wallets(
            Some(running),
            9,
            CloudBackupUploadedWalletsStateMode::PreserveVerification,
        )
        .await
        .unwrap()
    else {
        panic!("the supervisor answers the write lane");
    };

    current.unwrap();
    assert_eq!(wallet_count(), Some(9));
}

/// The listed rows the screen draws now
async fn drawn_rows(manager: &RustCloudBackupManager) -> Vec<CloudBackupListedRow> {
    let screen = call!(manager.supervisor.projected_screen_for_test()).await.unwrap();
    let CloudBackupScreenBody::Configured(configured) = screen.body else { return Vec::new() };

    configured
        .sections
        .into_iter()
        .flat_map(|section| match section {
            CloudBackupSection::NotOnDevice(section) => section.rows,
            CloudBackupSection::ExtraCopies(section) => section.rows,
            _ => Vec::new(),
        })
        .collect()
}

/// The consent the Delete on the row for `record_id` carries, or `None` when it offers none
async fn row_delete_consent(
    manager: &RustCloudBackupManager,
    record_id: &str,
) -> Option<CloudBackupRowDeleteConsent> {
    drawn_rows(manager)
        .await
        .into_iter()
        .filter(|row| row.record_id.as_str() == record_id)
        .flat_map(|row| row.actions)
        .find_map(|action| match action.action {
            CloudBackupRowActionKind::Delete { consent, .. } => Some(consent),
            _ => None,
        })
}
