
use super::*;
use super::enable::{
    EnableRecoveryFinalization, EnableUploadFinalization,
};
use super::verification::DeepVerificationContinuation;
use cove_cspp::{MasterKeyPromotionActiveState, MasterKeyPromotionStatus};
use cove_cspp::CsppStore as _;
use crate::database::cloud_backup::{
    PersistedDisablingCloudBackup, PersistedDriveAccountSwitch,
    PersistedDriveAccountSwitchPhase,
};
use crate::manager::cloud_backup_manager::CloudBackupWalletStatus;
use crate::manager::cloud_backup_manager::model::CloudBackupDetailState;
use crate::manager::cloud_backup_manager::ops::test_support::{
    async_test_lock, complete_cloud_only_fetch, complete_loaded_outcome,
    configure_enabled_cloud_backup, encrypted_wallet_backup_bytes_for_entry, listed_detail,
    persisted_enabled_cloud_backup_state, reset_cloud_backup_test_state, test_globals,
    staged_pending_enable_journal, wait_for_test_condition, wallet_entry_with_labels,
    xpub_only_wallet_metadata,
};
use crate::manager::cloud_backup_manager::reconcile::CloudBackupReconcileMessage;
use crate::manager::cloud_backup_manager::verify::coordinator::CloudBackupVerificationEffect;
use crate::manager::cloud_backup_manager::wallets::{
    StagedPrfKey, UnpersistedPrfKey,
};
use crate::manager::cloud_backup_manager::{
    CloudBackupDetail, CloudBackupInventoryAuthority, CloudBackupLifecycle, CloudBackupListedDetail,
    CloudBackupOtherBackupsSummary,
    CloudBackupPendingEnableCleanupState, CloudBackupPendingEnableRecovery, CloudBackupStore,
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

fn test_supervisor_manager() -> Arc<RustCloudBackupManager> {
    let globals = test_globals();

    // a new manager resumes any persisted transition on init and blocks its writes for it, so a
    // transition an earlier test left behind must be gone first
    let _ = Database::global().cloud_backup_state.delete();
    let manager = RustCloudBackupManager::init();
    reset_cloud_backup_test_state(&manager, globals);
    manager
}

fn persist_enabled_for_supervisor_test() {
    Database::global()
        .cloud_backup_state
        .set(&PersistedCloudBackupState::mark_enabled_reset_verification(0, 0))
        .unwrap();
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
    CloudBackupStore::global()
        .persist_restore_all_marker(VALID_NAMESPACE_ID.into())
        .unwrap();
    manager.sync_persisted_state();
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

fn test_cloud_backup_detail(cloud_only_count: u32) -> CloudBackupDetail {
    CloudBackupDetail {
        last_sync: None,
        up_to_date: Vec::new(),
        needs_sync: Vec::new(),
        cloud_only_count,
    }
}

/// Waits until the loaded rows hold every expected record and none of the replaced ones, which
/// only a fetch that read the provider again can produce
async fn wait_for_cloud_only_rows(
    manager: &RustCloudBackupManager,
    present: &[&str],
    absent: &[&str],
) -> Vec<CloudBackupWalletItem> {
    let loaded_rows = || match manager.state.read().cloud_only() {
        CloudOnlyState::Loaded { wallets, .. } => Some(wallets),
        CloudOnlyState::NotFetched | CloudOnlyState::Loading | CloudOnlyState::Failed { .. } => {
            None
        }
    };

    wait_for_test_condition(Duration::from_secs(5), "cloud-only rows should be refetched", || {
        loaded_rows().is_some_and(|wallets| {
            present.iter().all(|id| wallets.iter().any(|wallet| wallet.record_id == *id))
                && absent.iter().all(|id| wallets.iter().all(|wallet| wallet.record_id != *id))
        })
    })
    .await;

    loaded_rows().expect("cloud-only rows should be loaded")
}

fn cloud_only_row_statuses(rows: &[CloudBackupWalletItem]) -> Vec<(&str, CloudBackupWalletStatus)> {
    rows.iter().map(|row| (row.record_id.as_str(), row.sync_status.clone())).collect()
}

/// The cloud-only state the frontends read, which only exists once the detail has loaded
fn public_cloud_only(manager: &RustCloudBackupManager) -> Option<CloudOnlyState> {
    let CloudBackupLifecycle::Configured(configured) = manager.state().lifecycle else {
        return None;
    };

    match configured.detail {
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
        let CloudOnlyState::Loaded { wallets, .. } = manager.state.read().cloud_only() else {
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

fn list_test_wallet_files(namespace: &str, record_ids: &[&str]) {
    test_globals().cloud.set_wallet_files(
        namespace.into(),
        record_ids
            .iter()
            .map(|record_id| cove_cspp::backup_data::wallet_filename_from_record_id(record_id))
            .collect(),
    );
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
    let local_restore = manager.prepare_active_namespace_wallet_restore().await.unwrap();
    local_restore.restore_record(&local_record_id).await.unwrap();
    assert!(local_wallet_exists(&local_wallet));

    // test xpub-only wallets share one xpub, so this is the same wallet under another record ID
    let duplicate_record_id = upload_test_wallet_backup(&namespace, &xpub_only_wallet_metadata());
    list_test_wallet_files(&namespace, &[duplicate_record_id.as_str()]);

    LocalWalletDuplicateFixture { namespace, local_wallet, local_record_id, duplicate_record_id }
}

fn disabling_for_current_namespace(
    manager: &RustCloudBackupManager,
) -> PersistedDisablingCloudBackup {
    let PersistedCloudBackupState::Configured(previous_configured) =
        Database::global().cloud_backup_state.get().unwrap()
    else {
        panic!("expected configured cloud backup state");
    };

    PersistedDisablingCloudBackup {
        previous_configured,
        namespace_id: manager.current_namespace_id().unwrap(),
        disable_generation: 1,
        started_at: 1,
        delete_started_at: None,
        last_error: None,
        retry_after: None,
    }
}

fn begin_test_detail_refresh(
    supervisor: &mut CloudBackupSupervisor,
    manager: &RustCloudBackupManager,
) -> DetailRefreshClaim {
    supervisor.detail_workflow.open();
    let DetailRefreshPlan::Start(claim) = supervisor.detail_workflow.request_refresh() else {
        panic!("expected detail refresh to start");
    };
    manager.apply_detail_outcome(CloudBackupDetailOutcome::Checking);

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
    manager.apply_detail_outcome(CloudBackupDetailOutcome::Refreshed(listed_detail(
        &record_ids.iter().map(String::as_str).collect::<Vec<_>>(),
    )));
    manager.apply_cloud_only_fetch_outcome(complete_loaded_outcome(manager, items.clone()));
    assert!(matches!(
        manager.projected_restore_all_state(),
        CloudBackupRestoreAllState::StartAvailable { .. }
    ));

    (namespace, items)
}

fn test_enable_upload_finalization() -> (
    EnableUploadFinalization,
    cove_cspp::master_key::MasterKey,
) {
    let master_key = cove_cspp::master_key::MasterKey::generate();
    let namespace_id = master_key.namespace_id();
    let passkey = test_enable_passkey(vec![1, 2, 3]);
    let pending_completion = test_pending_completion(
        namespace_id.clone(),
        vec![PendingVerificationUpload::master_key_wrapper()],
    );

    (
        EnableUploadFinalization {
            master_key: zeroize::Zeroizing::new(cove_cspp::master_key::MasterKey::from_bytes(
                *master_key.as_bytes(),
            )),
            passkey: zeroize::Zeroizing::new(passkey),
            context: CloudBackupEnableContext::settings_manual(),
            namespace_id,
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
    let cloud_keychain = CloudBackupKeychain::global();
    let cspp = cove_cspp::Cspp::new(Keychain::global().clone());
    cspp.save_staged_master_key(master_key).unwrap();

    let mut journal = staged_pending_enable_journal(
        finalization.context,
        finalization.namespace_id.clone(),
        PendingEnableNamespaceOwnership::FreshOwned,
        cloud_keychain.snapshot_passkey_metadata(),
    );
    assert!(journal.register_passkey(PendingEnablePasskeyMetadata {
        credential_id: finalization.passkey.credential_id.clone(),
        prf_salt: finalization.passkey.prf_salt,
        provider_hint: finalization.passkey.provider_hint.clone(),
    }));
    assert!(journal.mark_remote_writes_started());
    cloud_keychain.save_pending_enable_journal(&journal).unwrap();

    manager
        .pending_enable
        .begin_pending_enable_local_promotion(master_key, &finalization.passkey)
        .unwrap();
}

fn test_enable_recovery_finalization(namespace_id: &str) -> EnableRecoveryFinalization {
    EnableRecoveryFinalization {
        context: CloudBackupEnableContext::settings_manual(),
        namespace_id: namespace_id.into(),
        credential_id: vec![1, 2, 3],
        prf_salt: [9; 32],
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
        cloud_keychain.snapshot_passkey_metadata(),
    );
    assert!(journal.register_passkey(PendingEnablePasskeyMetadata {
        credential_id: finalization.credential_id.clone(),
        prf_salt: finalization.prf_salt,
        provider_hint: None,
    }));
    assert!(journal.mark_remote_writes_started());
    cloud_keychain.save_pending_enable_journal(&journal).unwrap();

    manager
        .pending_enable
        .begin_enable_recovery_local_promotion(
            &finalization.namespace_id,
            &finalization.credential_id,
            finalization.prf_salt,
        )
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
        credential_id: vec![1, 2, 3],
        prf_salt: [9; 32],
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
        credential_id: vec![1, 2, 3],
        prf_salt: [9; 32],
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
    let mut supervisor = CloudBackupSupervisor::new(
        Arc::downgrade(&manager),
        spawn_actor(CloudBackupWriteSupervisor::new(Weak::new())),
    );

    let first = supervisor
        .begin_exclusive_operation(&manager, CloudBackupExclusiveOperation::Enable)
        .unwrap();
    let second =
        supervisor.begin_exclusive_operation(&manager, CloudBackupExclusiveOperation::Restore);

    assert!(second.is_none());
    assert_eq!(supervisor.active_operation, Some(first));
    assert_eq!(manager.projected_exclusive_operation(), Some(first));

    supervisor.complete_exclusive_operation(first).await.unwrap();

    assert_eq!(supervisor.active_operation, None);
    assert_eq!(manager.projected_exclusive_operation(), None);
}

#[test]
fn local_reset_cancels_restore_without_releasing_its_claim() {
    let claim = CloudBackupExclusiveOperationClaim::new(
        CloudBackupExclusiveOperation::Restore,
        u64::MAX,
    );
    let cancellation = Arc::new(AtomicBool::new(false));
    let mut active = ActiveOperation::default();
    active.start_restore(RestoreRun::new(claim, Arc::clone(&cancellation)));

    let released_claim = active.prepare_local_reset();

    assert!(released_claim.is_none());
    assert!(cancellation.load(Ordering::Acquire));
    assert_eq!(active, Some(claim));
}

#[tokio::test(flavor = "current_thread")]
async fn drive_account_switch_reports_busy_and_cancellation_releases_claim() {
    let _guard = async_test_lock().lock().await;
    let manager = test_supervisor_manager();
    persist_enabled_for_supervisor_test();
    let mut supervisor = CloudBackupSupervisor::new(
        Arc::downgrade(&manager),
        spawn_actor(CloudBackupWriteSupervisor::new(Arc::downgrade(&manager))),
    );

    let (transition_id, _ready) = supervisor
        .begin_drive_account_switch()
        .await
        .unwrap()
        .await
        .unwrap()
        .unwrap();
    let busy = supervisor
        .begin_drive_account_switch()
        .await
        .unwrap()
        .await
        .unwrap()
        .unwrap_err();

    assert_eq!(busy, CloudBackupDriveAccountSwitchError::Busy);

    let invalid_commit = supervisor
        .confirm_drive_account_switch_committed((transition_id.value() + 1).into())
        .await
        .unwrap()
        .await
        .unwrap()
        .unwrap_err();

    assert_eq!(invalid_commit, CloudBackupDriveAccountSwitchError::InvalidTransition);

    supervisor
        .cancel_drive_account_switch(transition_id)
        .await
        .unwrap()
        .await
        .unwrap()
        .unwrap();
    supervisor
        .confirm_drive_account_switch_rolled_back(transition_id)
        .await
        .unwrap()
        .await
        .unwrap()
        .unwrap();

    assert!(RustCloudBackupManager::load_persisted_state().drive_account_switch().is_none());
    assert_eq!(supervisor.active_operation, None);
    assert_eq!(manager.projected_exclusive_operation(), None);
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
    let mut supervisor = CloudBackupSupervisor::new(
        Arc::downgrade(&manager),
        spawn_actor(CloudBackupWriteSupervisor::new(Arc::downgrade(&manager))),
    );

    let action = supervisor
        .reconcile_drive_account_switch(DriveAccountSwitchPlatformState::NoTransition)
        .await
        .unwrap()
        .await
        .unwrap()
        .unwrap();

    assert_eq!(action, DriveAccountSwitchReconcileAction::Rollback(7));
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
    let mut supervisor = CloudBackupSupervisor::new(
        Arc::downgrade(&manager),
        spawn_actor(CloudBackupWriteSupervisor::new(Arc::downgrade(&manager))),
    );

    let action = supervisor
        .reconcile_drive_account_switch(DriveAccountSwitchPlatformState::Staged(7))
        .await
        .unwrap()
        .await
        .unwrap()
        .unwrap();

    assert_eq!(action, DriveAccountSwitchReconcileAction::None);
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
    assert_eq!(
        RustCloudBackupManager::load_persisted_state().drive_account_switch(),
        Some(&transition)
    );
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
    let mut supervisor = CloudBackupSupervisor::new(
        Arc::downgrade(&manager),
        spawn_actor(CloudBackupWriteSupervisor::new(Arc::downgrade(&manager))),
    );
    let claim = CloudBackupExclusiveOperationClaim::drive_account_switch(transition.transition_id);
    supervisor.active_operation.start_standard(claim);
    manager.project_exclusive_operation_started(claim);

    let action = supervisor
        .reconcile_drive_account_switch(DriveAccountSwitchPlatformState::Staged(7))
        .await
        .unwrap()
        .await
        .unwrap()
        .unwrap();

    assert_eq!(action, DriveAccountSwitchReconcileAction::None);
    assert_eq!(supervisor.active_operation.claim(), Some(claim));
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
    let mut supervisor = CloudBackupSupervisor::new(
        Arc::downgrade(&manager),
        spawn_actor(CloudBackupWriteSupervisor::new(Arc::downgrade(&manager))),
    );
    let claim = CloudBackupExclusiveOperationClaim::drive_account_switch(transition.transition_id);
    supervisor.active_operation.start_standard(claim);
    manager.project_exclusive_operation_started(claim);

    supervisor.finish_enable_operation(manager.clone(), claim);
    supervisor.finish_enable_operation(manager.clone(), claim);

    assert_eq!(supervisor.active_operation.claim(), Some(claim));
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
    let mut supervisor = CloudBackupSupervisor::new(
        Arc::downgrade(&manager),
        spawn_actor(CloudBackupWriteSupervisor::new(Arc::downgrade(&manager))),
    );
    let claim = CloudBackupExclusiveOperationClaim::drive_account_switch(transition.transition_id);
    supervisor.active_operation.start_standard(claim);
    manager.project_exclusive_operation_started(claim);

    assert_eq!(
        supervisor.drive_account_switch_reinitialization_finished(&manager, claim, false),
        DriveAccountSwitchReinitializationCompletion::Handled
    );

    assert_eq!(
        RustCloudBackupManager::load_persisted_state()
            .drive_account_switch()
            .map(|account_switch| account_switch.phase),
        Some(PersistedDriveAccountSwitchPhase::AwaitingReinitializationRetry)
    );
    assert_eq!(supervisor.active_operation, None);
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
    let mut supervisor = CloudBackupSupervisor::new(
        Arc::downgrade(&manager),
        spawn_actor(CloudBackupWriteSupervisor::new(Arc::downgrade(&manager))),
    );

    let error = supervisor
        .begin_drive_account_switch()
        .await
        .unwrap()
        .await
        .unwrap()
        .unwrap_err();

    assert_eq!(error, CloudBackupDriveAccountSwitchError::Busy);
    assert_eq!(supervisor.active_operation, None);
    assert_eq!(manager.projected_exclusive_operation(), None);
}

#[tokio::test(flavor = "current_thread")]
async fn drive_account_switch_restart_finalizes_platform_only_commit() {
    let _guard = async_test_lock().lock().await;
    let manager = test_supervisor_manager();
    persist_enabled_for_supervisor_test();
    let mut supervisor = CloudBackupSupervisor::new(
        Arc::downgrade(&manager),
        spawn_actor(CloudBackupWriteSupervisor::new(Arc::downgrade(&manager))),
    );

    let action = supervisor
        .reconcile_drive_account_switch(DriveAccountSwitchPlatformState::Committed(7))
        .await
        .unwrap()
        .await
        .unwrap()
        .unwrap();

    assert_eq!(action, DriveAccountSwitchReconcileAction::Finalize(7));
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
    let mut supervisor = CloudBackupSupervisor::new(
        Arc::downgrade(&manager),
        spawn_actor(CloudBackupWriteSupervisor::new(Arc::downgrade(&manager))),
    );

    let action = supervisor
        .reconcile_drive_account_switch(DriveAccountSwitchPlatformState::Committed(8))
        .await
        .unwrap()
        .await
        .unwrap()
        .unwrap();

    assert_eq!(action, DriveAccountSwitchReconcileAction::None);
    assert_eq!(
        RustCloudBackupManager::load_persisted_state().drive_account_switch(),
        Some(&transition)
    );
    assert_eq!(
        supervisor.active_operation.claim().and_then(CloudBackupExclusiveOperationClaim::drive_account_switch_id),
        None
    );
    assert!(manager.reconciler.receiver().try_iter().any(|messages| match messages {
        SingleOrMany::Single(message) => matches!(
            message,
            CloudBackupReconcileMessage::DriveAccountSwitchRecoveryRequired {
                transition_id: 7,
                ..
            }
        ),
        SingleOrMany::Many(messages) => messages.into_iter().any(|message| matches!(
            message,
            CloudBackupReconcileMessage::DriveAccountSwitchRecoveryRequired {
                transition_id: 7,
                ..
            }
        )),
    }));
}

fn begin_exclusive(
    operation: CloudBackupExclusiveOperation,
) -> impl FnOnce(&mut CloudBackupSupervisor, &Arc<RustCloudBackupManager>) -> CloudBackupExclusiveOperationClaim
{
    move |supervisor, manager| supervisor.begin_exclusive_operation(manager, operation).unwrap()
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
    let mut supervisor = CloudBackupSupervisor::new(
        Arc::downgrade(&manager),
        spawn_actor(CloudBackupWriteSupervisor::new(Weak::new())),
    );

    let current = begin(&mut supervisor, &manager);
    let stale = CloudBackupExclusiveOperationClaim::new(current.operation(), u64::MAX);

    complete(&mut supervisor, stale).await;

    assert_eq!(supervisor.active_operation, Some(current));
    assert_eq!(manager.projected_exclusive_operation(), Some(current));

    complete(&mut supervisor, current).await;

    assert_eq!(supervisor.active_operation, None);
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
    assert_completion_ignores_stale_claim(begin_exclusive(CloudBackupExclusiveOperation::DeleteCloudWallet), async |supervisor, claim| {
            supervisor
                .complete_delete_cloud_wallet(claim, "wallet-record".into(), Err(CloudBackupError::Internal("completion".into())))
                .await
                .unwrap();
    })
    .await;
}

#[tokio::test(flavor = "current_thread")]
async fn supervisor_ignores_stale_restore_cloud_wallet_completion() {
    assert_completion_ignores_stale_claim(begin_exclusive(CloudBackupExclusiveOperation::RestoreCloudWallet), async |supervisor, claim| {
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
    assert_completion_ignores_stale_claim(begin_exclusive(CloudBackupExclusiveOperation::RepairPasskey), async |supervisor, claim| {
            supervisor.complete_repair_passkey_wrapper(claim, Err(CloudBackupError::Internal("completion".into()))).await.unwrap();
    })
    .await;
}

#[tokio::test(flavor = "current_thread")]
async fn supervisor_ignores_stale_repair_passkey_finalization_completion() {
    assert_completion_ignores_stale_claim(begin_exclusive(CloudBackupExclusiveOperation::RepairPasskey), async |supervisor, claim| {
            let finalization = Err(CloudBackupError::Internal("completion".into()));
            supervisor.complete_repair_passkey_finalization(claim, finalization).await.unwrap();
    })
    .await;
}

#[tokio::test(flavor = "current_thread")]
async fn supervisor_ignores_stale_repair_passkey_wrapper_upload_completion() {
    assert_completion_ignores_stale_claim(begin_exclusive(CloudBackupExclusiveOperation::RepairPasskey), async |supervisor, claim| {
            let upload = Err(CloudBackupError::Internal("completion".into()));
            supervisor.complete_repair_passkey_wrapper_upload(claim, upload).await.unwrap();
    })
    .await;
}

#[tokio::test(flavor = "current_thread")]
async fn supervisor_ignores_stale_recreate_manifest_recovery_completion() {
    assert_completion_ignores_stale_claim(begin_exclusive(CloudBackupExclusiveOperation::RecreateManifest), async |supervisor, claim| {
            supervisor.complete_recreate_manifest_recovery(claim, Err(CloudBackupError::Internal("completion".into()))).await.unwrap();
    })
    .await;
}

#[tokio::test(flavor = "current_thread")]
async fn supervisor_ignores_stale_recreate_manifest_finalization_completion() {
    assert_completion_ignores_stale_claim(begin_exclusive(CloudBackupExclusiveOperation::RecreateManifest), async |supervisor, claim| {
            supervisor.complete_recreate_manifest_finalization(claim, Err(CloudBackupError::Internal("completion".into()))).await.unwrap();
    })
    .await;
}

#[tokio::test(flavor = "current_thread")]
async fn supervisor_ignores_stale_recreate_manifest_verification_completion() {
    assert_completion_ignores_stale_claim(begin_exclusive(CloudBackupExclusiveOperation::RecreateManifest), async |supervisor, claim| {
            supervisor
                .complete_verification(
                    Some(claim),
                    CloudBackupDeepVerificationStep::Complete(DeepVerificationResult::NotEnabled),
                    DeepVerificationContinuation::RecreateManifest {
                        attempt: VerificationAttempt::Initial,
                    },
                )
                .await
                .unwrap();
    })
    .await;
}

#[tokio::test(flavor = "current_thread")]
async fn supervisor_ignores_stale_deep_verification_wrapper_repair_upload_completion() {
    assert_completion_ignores_stale_claim(begin_exclusive(CloudBackupExclusiveOperation::VerificationRepair), async |supervisor, claim| {
            let continuation = DeepVerificationContinuation::Manual {
                force_discoverable: true,
                attempt: VerificationAttempt::Initial,
            };
            supervisor
                .complete_deep_verification_wrapper_repair_upload(claim, continuation, Err(CloudBackupError::Internal("completion".into())))
                .await
                .unwrap();
    })
    .await;
}

#[tokio::test(flavor = "current_thread")]
async fn supervisor_ignores_stale_deep_verification_wrapper_repair_resume_completion() {
    assert_completion_ignores_stale_claim(begin_exclusive(CloudBackupExclusiveOperation::VerificationRepair), async |supervisor, claim| {
            let continuation = DeepVerificationContinuation::Manual {
                force_discoverable: true,
                attempt: VerificationAttempt::Initial,
            };
            supervisor
                .complete_deep_verification_wrapper_repair_resume(
                    claim,
                    continuation,
                    CloudBackupDeepVerificationStep::Complete(DeepVerificationResult::NotEnabled),
                )
                .await
                .unwrap();
    })
    .await;
}

#[tokio::test(flavor = "current_thread")]
async fn supervisor_ignores_stale_deep_verification_auto_sync_upload_completion() {
    assert_completion_ignores_stale_claim(begin_exclusive(CloudBackupExclusiveOperation::VerificationRepair), async |supervisor, claim| {
            let continuation = DeepVerificationContinuation::Manual {
                force_discoverable: true,
                attempt: VerificationAttempt::Initial,
            };
            supervisor
                .complete_deep_verification_auto_sync_upload(
                    claim,
                    continuation,
                    Err(DeepVerificationResult::NotEnabled),
                )
                .await
                .unwrap();
    })
    .await;
}

#[tokio::test(flavor = "current_thread")]
async fn supervisor_ignores_stale_deep_verification_auto_sync_finalization_completion() {
    assert_completion_ignores_stale_claim(begin_exclusive(CloudBackupExclusiveOperation::VerificationRepair), async |supervisor, claim| {
            let continuation = DeepVerificationContinuation::Manual {
                force_discoverable: true,
                attempt: VerificationAttempt::Initial,
            };
            supervisor
                .complete_deep_verification_auto_sync_finalization(
                    claim,
                    continuation,
                    Err(DeepVerificationResult::NotEnabled),
                )
                .await
                .unwrap();
    })
    .await;
}

#[tokio::test(flavor = "current_thread")]
async fn supervisor_ignores_stale_deep_verification_auto_sync_resume_completion() {
    assert_completion_ignores_stale_claim(begin_exclusive(CloudBackupExclusiveOperation::VerificationRepair), async |supervisor, claim| {
            let continuation = DeepVerificationContinuation::Manual {
                force_discoverable: true,
                attempt: VerificationAttempt::Initial,
            };
            supervisor
                .complete_deep_verification_auto_sync_resume(
                    claim,
                    continuation,
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
    assert_completion_ignores_stale_claim(begin_exclusive(CloudBackupExclusiveOperation::ReinitializeBackup), async |supervisor, claim| {
            supervisor.complete_enable_preparation(claim, Err(CloudBackupError::Internal("completion".into()))).await.unwrap();
    })
    .await;
}

#[tokio::test(flavor = "current_thread")]
async fn supervisor_ignores_stale_reinitialize_verification_completion() {
    assert_completion_ignores_stale_claim(begin_exclusive(CloudBackupExclusiveOperation::ReinitializeBackup), async |supervisor, claim| {
            supervisor
                .complete_verification(
                    Some(claim),
                    CloudBackupDeepVerificationStep::Complete(DeepVerificationResult::NotEnabled),
                    DeepVerificationContinuation::ReinitializeBackup {
                        attempt: VerificationAttempt::Initial,
                    },
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
                        credential_id: vec![1, 2, 3],
                        prf_salt: [9; 32],
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
async fn supervisor_ignores_stale_disable_preparation_completion() {
    assert_completion_ignores_stale_claim(begin_exclusive(CloudBackupExclusiveOperation::Disable), async |supervisor, claim| {
            supervisor.complete_disable_preparation(claim, Err(CloudBackupError::Internal("completion".into()))).await.unwrap();
    })
    .await;
}

#[tokio::test(flavor = "current_thread")]
async fn supervisor_ignores_stale_disable_blocker_completion() {
    assert_completion_ignores_stale_claim(begin_exclusive(CloudBackupExclusiveOperation::Disable), async |supervisor, claim| {
            supervisor
                .complete_disable_blocker_check(claim, test_disabling_state(), Err(CloudBackupError::Internal("completion".into())))
                .await
                .unwrap();
    })
    .await;
}

#[tokio::test(flavor = "current_thread")]
async fn supervisor_ignores_stale_disable_delete_completion() {
    assert_completion_ignores_stale_claim(begin_exclusive(CloudBackupExclusiveOperation::Disable), async |supervisor, claim| {
            supervisor
                .complete_disable_namespace_delete(claim, test_disabling_state(), Err(CloudBackupError::Internal("completion".into())))
                .await
                .unwrap();
    })
    .await;
}

#[tokio::test(flavor = "current_thread")]
async fn supervisor_ignores_stale_disable_local_cleanup_completion() {
    assert_completion_ignores_stale_claim(begin_exclusive(CloudBackupExclusiveOperation::Disable), async |supervisor, claim| {
            supervisor
                .complete_disable_local_cleanup(claim, test_disabling_state(), Err(CloudBackupError::Internal("completion".into())))
                .await
                .unwrap();
    })
    .await;
}

#[tokio::test(flavor = "current_thread")]
async fn supervisor_ignores_stale_recover_other_backups_completion() {
    assert_completion_ignores_stale_claim(|supervisor, manager| {
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
async fn supervisor_ignores_stale_delete_cloud_wallet_preparation_completion() {
    assert_completion_ignores_stale_claim(begin_exclusive(CloudBackupExclusiveOperation::DeleteCloudWallet), async |supervisor, claim| {
            supervisor.complete_delete_cloud_wallet_preparation(claim, Err(CloudBackupError::Internal("completion".into()))).await.unwrap();
    })
    .await;
}

#[tokio::test(flavor = "current_thread")]
async fn supervisor_ignores_stale_delete_other_backups_completion() {
    assert_completion_ignores_stale_claim(|supervisor, manager| {
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
    let mut supervisor = CloudBackupSupervisor::new(
        Arc::downgrade(&manager),
        spawn_actor(CloudBackupWriteSupervisor::new(Weak::new())),
    );

    let current = supervisor
        .begin_exclusive_operation(&manager, CloudBackupExclusiveOperation::RepairPasskey)
        .unwrap();
    let stale = CloudBackupExclusiveOperationClaim::new(
        CloudBackupExclusiveOperation::RepairPasskey,
        u64::MAX,
    );

    let detail_claim = supervisor.detail_workflow.start_operation_result();
    supervisor
        .complete_repair_passkey_refresh_detail(stale, detail_claim, None)
        .await
        .unwrap();

    assert_eq!(supervisor.active_operation, Some(current));
    assert_eq!(manager.projected_exclusive_operation(), Some(current));

    supervisor
        .complete_repair_passkey_refresh_detail(current, detail_claim, None)
        .await
        .unwrap();

    assert_eq!(supervisor.active_operation, None);
    assert_eq!(manager.projected_exclusive_operation(), None);
}

#[tokio::test(flavor = "current_thread")]
async fn repair_passkey_refresh_failure_resolves_superseded_detail_refresh() {
    let _guard = async_test_lock().lock().await;
    let manager = test_supervisor_manager();
    configure_enabled_cloud_backup(&manager, test_globals(), 0);
    let mut supervisor = CloudBackupSupervisor::new(
        Arc::downgrade(&manager),
        spawn_actor(CloudBackupWriteSupervisor::new(Weak::new())),
    );

    supervisor.detail_workflow.open();
    let DetailRefreshPlan::Start(superseded_refresh) =
        supervisor.detail_workflow.request_refresh()
    else {
        panic!("expected detail refresh to start");
    };
    manager.apply_detail_outcome(CloudBackupDetailOutcome::Checking);

    let operation_claim = supervisor
        .begin_exclusive_operation(&manager, CloudBackupExclusiveOperation::RepairPasskey)
        .unwrap();
    let repair_detail_claim = supervisor.detail_workflow.start_operation_result();
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

    let CloudBackupLifecycle::Configured(configured) = manager.state.read().public_state().lifecycle
    else {
        panic!("expected configured cloud backup");
    };
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
        )
        .await
        .unwrap();

    let CloudBackupLifecycle::Configured(configured) = manager.state.read().public_state().lifecycle
    else {
        panic!("expected configured cloud backup");
    };
    assert!(matches!(configured.detail, CloudBackupDetailState::Failed { .. }));
}

#[tokio::test(flavor = "current_thread")]
async fn trusted_local_detail_stays_active_until_provider_confirmation() {
    let _guard = async_test_lock().lock().await;
    let manager = test_supervisor_manager();
    configure_enabled_cloud_backup(&manager, test_globals(), 2);
    let mut supervisor = CloudBackupSupervisor::new(
        Arc::downgrade(&manager),
        spawn_actor(CloudBackupWriteSupervisor::new(Weak::new())),
    );
    let listed = listed_detail(&["first", "second"]);
    let (claim, confirmation) =
        start_trusted_local_refresh(&mut supervisor, &manager, listed.clone()).await;

    assert!(supervisor.detail_workflow.is_refresh_active(claim));

    let wallets = vec![test_cloud_only_wallet("first"), test_cloud_only_wallet("second")];
    manager.apply_cloud_only_fetch_outcome(complete_loaded_outcome(&manager, 
        wallets.clone(),
    ));
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
        )
        .await
        .unwrap();

    assert!(!supervisor.detail_workflow.is_refresh_active(claim));
    assert_eq!(
        manager.projected_restore_all_state(),
        CloudBackupRestoreAllState::StartAvailable { wallet_count: 2 },
    );

    let CloudBackupLifecycle::Configured(configured) = manager.state().lifecycle else {
        panic!("expected configured cloud backup");
    };
    let CloudBackupDetailState::Complete { state } = configured.detail else {
        panic!("expected complete cloud backup detail");
    };

    assert_eq!(state.inventory_authority, CloudBackupInventoryAuthority::ProviderConfirmed);
    assert_eq!(state.cloud_only, CloudOnlyState::Loaded { wallets, incomplete: false });
}

#[tokio::test(flavor = "current_thread")]
async fn provider_confirmation_failure_marks_trusted_local_detail_incomplete() {
    let _guard = async_test_lock().lock().await;
    let manager = test_supervisor_manager();
    configure_enabled_cloud_backup(&manager, test_globals(), 2);
    let mut supervisor = CloudBackupSupervisor::new(
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
        )
        .await
        .unwrap();

    assert!(!supervisor.detail_workflow.is_refresh_active(claim));
    assert!(matches!(
        manager.state().lifecycle,
        CloudBackupLifecycle::Configured(ref configured)
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
    let mut supervisor = CloudBackupSupervisor::new(
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
        )
        .await
        .unwrap();

    assert!(!supervisor.detail_workflow.is_refresh_active(claim));
    assert!(matches!(
        manager.state().lifecycle,
        CloudBackupLifecycle::Configured(ref configured)
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
    let mut supervisor = CloudBackupSupervisor::new(
        Arc::downgrade(&manager),
        spawn_actor(CloudBackupWriteSupervisor::new(Weak::new())),
    );
    let (claim, confirmation) =
        start_trusted_local_refresh(&mut supervisor, &manager, listed_detail(&[])).await;

    assert_eq!(supervisor.detail_workflow.request_refresh(), DetailRefreshPlan::Queued);

    supervisor
        .complete_confirm_refresh_detail(
            confirmation,
            Some(CloudBackupDetailResult::SuccessWithAuthority {
                listed: listed_detail(&[]),
                authority: CloudBackupInventoryAuthority::ProviderConfirmed,
            }),
            DetailRefreshAttempt::Initial,
            claim,
        )
        .await
        .unwrap();

    assert!(!supervisor.detail_workflow.is_refresh_active(claim));
    assert_eq!(supervisor.detail_workflow.request_refresh(), DetailRefreshPlan::Queued);
}

#[tokio::test(flavor = "current_thread")]
async fn stale_provider_confirmation_does_not_overwrite_newer_operation_result() {
    let _guard = async_test_lock().lock().await;
    let manager = test_supervisor_manager();
    configure_enabled_cloud_backup(&manager, test_globals(), 2);
    let mut supervisor = CloudBackupSupervisor::new(
        Arc::downgrade(&manager),
        spawn_actor(CloudBackupWriteSupervisor::new(Weak::new())),
    );
    let (claim, confirmation) =
        start_trusted_local_refresh(&mut supervisor, &manager, listed_detail(&[])).await;

    supervisor.detail_workflow.start_operation_result();
    let newer = listed_detail(&["only"]);
    let newer_detail = newer.detail.clone();
    manager.apply_detail_outcome(CloudBackupDetailOutcome::Refreshed(newer));

    supervisor
        .complete_confirm_refresh_detail(
            confirmation,
            Some(CloudBackupDetailResult::SuccessWithAuthority {
                listed: listed_detail(&[]),
                authority: CloudBackupInventoryAuthority::ProviderConfirmed,
            }),
            DetailRefreshAttempt::Initial,
            claim,
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
    let mut supervisor = CloudBackupSupervisor::new(
        Arc::downgrade(&manager),
        spawn_actor(CloudBackupWriteSupervisor::new(Weak::new())),
    );
    supervisor.active_sync_request = Some(7);

    supervisor
        .complete_sync_request(6, Err(CloudBackupError::Internal("stale completion".into())))
        .await
        .unwrap();

    assert_eq!(supervisor.active_sync_request, Some(7));

    supervisor
        .complete_sync_request(7, Err(CloudBackupError::Internal("current completion".into())))
        .await
        .unwrap();

    assert_eq!(supervisor.active_sync_request, None);
}

#[tokio::test(flavor = "current_thread")]
async fn supervisor_ignores_stale_sync_refresh_completion() {
    let _guard = async_test_lock().lock().await;
    let manager = test_supervisor_manager();
    let mut supervisor = CloudBackupSupervisor::new(
        Arc::downgrade(&manager),
        spawn_actor(CloudBackupWriteSupervisor::new(Weak::new())),
    );
    supervisor.active_sync_request = Some(7);
    let detail_claim = supervisor.detail_workflow.start_operation_result();

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
    let mut supervisor = CloudBackupSupervisor::new(
        Arc::downgrade(&manager),
        spawn_actor(CloudBackupWriteSupervisor::new(Weak::new())),
    );
    supervisor.active_cloud_only_fetch_request = Some(7);

    supervisor.complete_cloud_only_fetch_request(6, Ok(complete_cloud_only_fetch(&manager, Vec::new()))).await.unwrap();

    assert_eq!(supervisor.active_cloud_only_fetch_request, Some(7));

    supervisor.complete_cloud_only_fetch_request(7, Ok(complete_cloud_only_fetch(&manager, Vec::new()))).await.unwrap();

    assert_eq!(supervisor.active_cloud_only_fetch_request, None);
}















#[tokio::test(flavor = "current_thread")]
async fn enable_recovery_finalization_requires_durable_pending_completion_before_success() {
    let _guard = async_test_lock().lock().await;
    let manager = test_supervisor_manager();
    let master_key = cove_cspp::master_key::MasterKey::generate();
    let namespace_id = master_key.namespace_id();
    let finalization = test_enable_recovery_finalization(&namespace_id);
    Database::global()
        .cloud_backup_state
        .set(&persisted_enabled_cloud_backup_state(Some(0)))
        .unwrap();
    let mut supervisor = CloudBackupSupervisor::new(
        Arc::downgrade(&manager),
        spawn_actor(CloudBackupWriteSupervisor::new(Weak::new())),
    );
    let context = CloudBackupEnableContext::settings_manual();
    manager.project_enable_context_started(context);
    let claim = supervisor
        .begin_exclusive_operation(&manager, CloudBackupExclusiveOperation::Enable)
        .unwrap();
    prepare_test_enable_recovery_local_promotion(&manager, &finalization, &master_key);

    supervisor
        .complete_enable_recovery_finalization(claim, finalization, Ok(()))
        .await
        .unwrap();

    assert!(matches!(manager.model_snapshot().status, CloudBackupStatus::Error(_)));
    assert!(manager.pending_verification_completion().is_none());
    assert!(!emitted_enable_completed(&manager));
    assert_eq!(supervisor.active_operation, None);
    let journal = CloudBackupKeychain::global().load_pending_enable_journal().unwrap().unwrap();
    assert!(matches!(journal.phase(), PendingEnableJournalPhase::LocalPromotionStarted(_)));
    assert_eq!(journal.namespace_ownership(), PendingEnableNamespaceOwnership::RecoveredExisting);
}

#[tokio::test(flavor = "current_thread")]
async fn enable_recovery_finalization_failure_restores_prior_and_retains_retry_evidence() {
    let _guard = async_test_lock().lock().await;
    let manager = test_supervisor_manager();
    let cspp = cove_cspp::Cspp::new(Keychain::global().clone());
    let prior_master_key = cove_cspp::master_key::MasterKey::generate();
    let prior_master_key_bytes = *prior_master_key.as_bytes();
    cspp.save_master_key(&prior_master_key).unwrap();
    let cloud_keychain = CloudBackupKeychain::global();
    cloud_keychain.save_passkey_and_namespace(&[7, 8, 9], [8; 32], "prior-namespace").unwrap();
    let prior_metadata = cloud_keychain.snapshot_passkey_metadata();

    let recovered_master_key = cove_cspp::master_key::MasterKey::generate();
    let finalization =
        test_enable_recovery_finalization(&recovered_master_key.namespace_id());
    Database::global()
        .cloud_backup_state
        .set(&persisted_enabled_cloud_backup_state(Some(0)))
        .unwrap();
    let mut supervisor = CloudBackupSupervisor::new(
        Arc::downgrade(&manager),
        spawn_actor(CloudBackupWriteSupervisor::new(Weak::new())),
    );
    let context = CloudBackupEnableContext::settings_manual();
    manager.project_enable_context_started(context);
    let claim = supervisor
        .begin_exclusive_operation(&manager, CloudBackupExclusiveOperation::Enable)
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
    assert_eq!(cloud_keychain.snapshot_passkey_metadata(), prior_metadata);
    assert_eq!(
        cspp.master_key_promotion_status().unwrap(),
        MasterKeyPromotionStatus::Pending(MasterKeyPromotionActiveState::Prior)
    );
    let journal = cloud_keychain.load_pending_enable_journal().unwrap().unwrap();
    assert!(matches!(journal.phase(), PendingEnableJournalPhase::RemoteWritesStarted(_)));
    assert_eq!(journal.namespace_ownership(), PendingEnableNamespaceOwnership::RecoveredExisting);
    assert!(!emitted_enable_completed(&manager));
    assert_eq!(supervisor.active_operation, None);
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
    Database::global().cloud_backup_state.set(&persisted_state).unwrap();
    let mut supervisor = CloudBackupSupervisor::new(
        Arc::downgrade(&manager),
        spawn_actor(CloudBackupWriteSupervisor::new(Weak::new())),
    );
    let context = CloudBackupEnableContext::settings_manual();
    manager.project_enable_context_started(context);
    let claim = supervisor
        .begin_exclusive_operation(&manager, CloudBackupExclusiveOperation::Enable)
        .unwrap();
    prepare_test_enable_recovery_local_promotion(&manager, &finalization, &master_key);

    supervisor
        .complete_enable_recovery_finalization(claim, finalization, Ok(()))
        .await
        .unwrap();

    assert_eq!(manager.model_snapshot().status, CloudBackupStatus::Enabled);
    assert!(manager.pending_verification_completion().is_some());
    assert!(emitted_enable_completed(&manager));
    assert_eq!(supervisor.active_operation, None);
    assert!(CloudBackupKeychain::global().load_pending_enable_journal().unwrap().is_none());
    assert_eq!(CloudBackupKeychain::global().namespace_id().as_deref(), Some(namespace_id.as_str()));
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
    let mut supervisor = CloudBackupSupervisor::new(
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

    assert_eq!(supervisor.active_operation, Some(current));
    assert_eq!(manager.projected_exclusive_operation(), Some(current));
    let pending = supervisor.pending_enable_session.take().unwrap();
    let (pending_master_key, pending_passkey) = pending.into_staged_parts().unwrap();
    assert_eq!(pending_master_key.namespace_id(), expected_namespace);
    assert_eq!(pending_passkey.credential_id, expected_credential_id);
}

#[tokio::test(flavor = "current_thread")]
async fn supervisor_consumes_retry_pending_enable_upload() {
    let _guard = async_test_lock().lock().await;
    let manager = test_supervisor_manager();
    let mut supervisor = CloudBackupSupervisor::new(
        Arc::downgrade(&manager),
        spawn_actor(CloudBackupWriteSupervisor::new(Weak::new())),
    );
    let master_key = cove_cspp::master_key::MasterKey::generate();
    let expected_namespace = master_key.namespace_id();
    let expected_credential_id = vec![1, 2, 3];

    supervisor.pending_enable_session = Some(PendingEnableSession::retry_upload(
        master_key,
        test_enable_passkey(expected_credential_id.clone()),
        CloudBackupEnableContext::settings_manual(),
    ));

    let ready = supervisor
        .take_ready_enable_upload()
        .unwrap()
        .unwrap();

    assert_eq!(ready.master_key.namespace_id(), expected_namespace);
    assert_eq!(ready.passkey.credential_id, expected_credential_id);
    assert!(supervisor.pending_enable_session.is_none());
}





#[tokio::test(flavor = "current_thread")]
async fn enable_upload_finalization_requires_durable_pending_completion_before_success() {
    let _guard = async_test_lock().lock().await;
    let manager = test_supervisor_manager();
    Database::global()
        .cloud_backup_state
        .set(&persisted_enabled_cloud_backup_state(Some(0)))
        .unwrap();
    let mut supervisor = CloudBackupSupervisor::new(
        Arc::downgrade(&manager),
        spawn_actor(CloudBackupWriteSupervisor::new(Weak::new())),
    );
    let context = CloudBackupEnableContext::settings_manual();
    manager.project_enable_context_started(context);
    let claim = supervisor
        .begin_exclusive_operation(&manager, CloudBackupExclusiveOperation::Enable)
        .unwrap();

    supervisor
        .complete_enable_upload_finalization(claim, test_enable_upload_finalization().0, Ok(()))
        .await
        .unwrap();

    assert!(matches!(manager.model_snapshot().status, CloudBackupStatus::Error(_)));
    assert!(manager.pending_verification_completion().is_none());
    assert!(!emitted_enable_completed(&manager));
    assert_eq!(supervisor.active_operation, None);
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
    Database::global()
        .cloud_backup_state
        .set(&persisted_state)
        .unwrap();
    let mut supervisor = CloudBackupSupervisor::new(
        Arc::downgrade(&manager),
        spawn_actor(CloudBackupWriteSupervisor::new(Weak::new())),
    );
    let context = CloudBackupEnableContext::settings_manual();
    manager.project_enable_context_started(context);
    let claim = supervisor
        .begin_exclusive_operation(&manager, CloudBackupExclusiveOperation::Enable)
        .unwrap();

    supervisor
        .complete_enable_upload_finalization(claim, finalization, Ok(()))
        .await
        .unwrap();

    assert_eq!(manager.model_snapshot().status, CloudBackupStatus::Enabled);
    assert!(manager.pending_verification_completion().is_some());
    assert!(emitted_enable_completed(&manager));
    assert_eq!(supervisor.active_operation, None);
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
            cloud_keychain.snapshot_passkey_metadata(),
        ))
        .unwrap();
    cspp.save_staged_master_key(&staged_master_key).unwrap();
    let mut supervisor = CloudBackupSupervisor::new(
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
    let previous_metadata = cloud_keychain.snapshot_passkey_metadata();
    cspp.save_staged_master_key(&staged_master_key).unwrap();
    cloud_keychain
        .save_pending_enable_journal(&staged_pending_enable_journal(
            CloudBackupEnableContext::settings_manual(),
            staged_master_key.namespace_id(),
            PendingEnableNamespaceOwnership::FreshOwned,
            previous_metadata.clone(),
        ))
        .unwrap();
    let mut supervisor = CloudBackupSupervisor::new(
        Arc::downgrade(&manager),
        spawn_actor(CloudBackupWriteSupervisor::new(Weak::new())),
    );

    supervisor.recover_pending_enable_after_restart(&manager).unwrap();

    assert_eq!(
        cspp.load_master_key_from_store().unwrap().unwrap().as_bytes(),
        prior_master_key.as_bytes()
    );
    assert_eq!(cspp.master_key_promotion_status().unwrap(), cove_cspp::MasterKeyPromotionStatus::None);
    assert_eq!(cloud_keychain.snapshot_passkey_metadata(), previous_metadata);
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
        cloud_keychain.snapshot_passkey_metadata(),
    );
    assert!(journal.register_passkey(PendingEnablePasskeyMetadata {
        credential_id: passkey.credential_id.clone(),
        prf_salt: passkey.prf_salt,
        provider_hint: passkey.provider_hint.clone(),
    }));
    cloud_keychain.save_pending_enable_journal(&journal).unwrap();
    let mut supervisor = CloudBackupSupervisor::new(
        Arc::downgrade(&manager),
        spawn_actor(CloudBackupWriteSupervisor::new(Weak::new())),
    );

    supervisor.recover_pending_enable_after_restart(&manager).unwrap();

    let pending = supervisor.pending_enable_session.take().unwrap();
    assert!(pending.is_awaiting_saved_passkey_confirmation());
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
    let previous_metadata = cloud_keychain.snapshot_passkey_metadata();
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
    let mut supervisor = CloudBackupSupervisor::new(
        Arc::downgrade(&manager),
        spawn_actor(CloudBackupWriteSupervisor::new(Weak::new())),
    );

    supervisor.recover_pending_enable_after_restart(&manager).unwrap();

    assert!(supervisor.pending_enable_session.is_none());
    assert_eq!(
        cspp.load_master_key_from_store().unwrap().unwrap().as_bytes(),
        prior_master_key.as_bytes()
    );
    assert_eq!(
        cspp.master_key_promotion_status().unwrap(),
        cove_cspp::MasterKeyPromotionStatus::None
    );
    assert_eq!(cloud_keychain.snapshot_passkey_metadata(), previous_metadata);
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

    assert!(supervisor.pending_enable_session.is_none());
    assert_eq!(
        cspp.load_master_key_from_store().unwrap().unwrap().as_bytes(),
        prior_master_key.as_bytes()
    );
    assert_eq!(
        cspp.master_key_promotion_status().unwrap(),
        cove_cspp::MasterKeyPromotionStatus::None
    );
    assert_eq!(cloud_keychain.snapshot_passkey_metadata(), previous_metadata);
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
    let mut supervisor = CloudBackupSupervisor::new(
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
    assert_eq!(cloud_keychain.namespace_id().as_deref(), Some(finalization.namespace_id.as_str()));
    assert_eq!(cloud_keychain.load_credential_id().as_deref(), Some(finalization.passkey.credential_id.as_slice()));
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
    let mut supervisor = CloudBackupSupervisor::new(
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
        CloudBackupKeychain::global().namespace_id().as_deref(),
        Some(finalization.namespace_id.as_str())
    );
    assert!(CloudBackupKeychain::global()
        .load_pending_enable_journal()
        .unwrap()
        .is_none());
    assert!(!matches!(manager.state().lifecycle, CloudBackupLifecycle::PendingEnableRecovery(_)));
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
    let previous_metadata = cloud_keychain.snapshot_passkey_metadata();
    let (finalization, staged_master_key) = test_enable_upload_finalization();
    prepare_test_enable_local_promotion(&manager, &finalization, &staged_master_key);
    let mut supervisor = CloudBackupSupervisor::new(
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
    assert_eq!(cloud_keychain.snapshot_passkey_metadata(), previous_metadata);
    assert!(supervisor
        .pending_enable_session
        .as_ref()
        .is_some_and(PendingEnableSession::is_awaiting_saved_passkey_confirmation));
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
    let mut supervisor = CloudBackupSupervisor::new(
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
        manager.state().lifecycle,
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
    let mut supervisor = CloudBackupSupervisor::new(
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
    let mut supervisor = CloudBackupSupervisor::new(
        Arc::downgrade(&manager),
        spawn_actor(CloudBackupWriteSupervisor::new(Weak::new())),
    );

    supervisor.resume_pending_enable_after_restart().await.unwrap();

    assert!(matches!(
        manager.state().lifecycle,
        CloudBackupLifecycle::PendingEnableRecovery(CloudBackupPendingEnableRecovery {
            support_code,
            cleanup: CloudBackupPendingEnableCleanupState::Available,
        }) if support_code == "CB-PE-001"
    ));
    assert_eq!(manager.state().settings_row_status, CloudBackupSettingsRowStatus::RecoveryRequired);
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
    let previous_metadata = cloud_keychain.snapshot_passkey_metadata();
    Database::global()
        .cloud_backup_state
        .set(&persisted_enabled_cloud_backup_state(Some(0)))
        .unwrap();
    cspp.save_staged_master_key(&staged_master_key).unwrap();
    let remote_deletes_before = globals.cloud.delete_namespace_attempt_count();
    let mut supervisor = CloudBackupSupervisor::new(
        Arc::downgrade(&manager),
        spawn_actor(CloudBackupWriteSupervisor::new(Weak::new())),
    );

    supervisor.resume_pending_enable_after_restart().await.unwrap();
    assert!(matches!(
        manager.state().lifecycle,
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
    assert_eq!(cloud_keychain.snapshot_passkey_metadata(), previous_metadata);
    assert!(cloud_keychain.load_pending_enable_journal().unwrap().is_none());
    assert_eq!(cspp.master_key_promotion_status().unwrap(), MasterKeyPromotionStatus::None);
    assert_eq!(globals.cloud.delete_namespace_attempt_count(), remote_deletes_before);
    assert!(matches!(manager.state().lifecycle, CloudBackupLifecycle::Configured(_)));
}

#[tokio::test(flavor = "current_thread")]
async fn no_journal_staged_cleanup_revalidates_before_discarding() {
    let _guard = async_test_lock().lock().await;
    let manager = test_supervisor_manager();
    let cspp = cove_cspp::Cspp::new(Keychain::global().clone());
    cspp.save_master_key(&cove_cspp::master_key::MasterKey::generate()).unwrap();
    cspp.save_staged_master_key(&cove_cspp::master_key::MasterKey::generate()).unwrap();
    let mut supervisor = CloudBackupSupervisor::new(
        Arc::downgrade(&manager),
        spawn_actor(CloudBackupWriteSupervisor::new(Weak::new())),
    );
    supervisor.resume_pending_enable_after_restart().await.unwrap();
    assert!(matches!(
        manager.state().lifecycle,
        CloudBackupLifecycle::PendingEnableRecovery(CloudBackupPendingEnableRecovery {
            cleanup: CloudBackupPendingEnableCleanupState::Available,
            ..
        })
    ));
    cspp.promote_staged_master_key().unwrap();

    supervisor.confirm_pending_enable_cleanup().await.unwrap();

    assert!(matches!(
        manager.state().lifecycle,
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
        cloud_keychain.snapshot_passkey_metadata(),
    );
    assert!(journal.register_passkey(PendingEnablePasskeyMetadata {
        credential_id: vec![1, 2, 3],
        prf_salt: [9; 32],
        provider_hint: None,
    }));
    cloud_keychain.save_pending_enable_journal(&journal).unwrap();
    cspp.promote_staged_master_key().unwrap();
    let mut supervisor = CloudBackupSupervisor::new(
        Arc::downgrade(&manager),
        spawn_actor(CloudBackupWriteSupervisor::new(Weak::new())),
    );

    supervisor.resume_pending_enable_after_restart().await.unwrap();

    assert!(matches!(
        manager.state().lifecycle,
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
    let mut supervisor = CloudBackupSupervisor::new(
        Arc::downgrade(&manager),
        spawn_actor(CloudBackupWriteSupervisor::new(Weak::new())),
    );

    supervisor.resume_pending_enable_after_restart().await.unwrap();

    let CloudBackupLifecycle::PendingEnableRecovery(recovery) = manager.state().lifecycle else {
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
    let previous_metadata = cloud_keychain.snapshot_passkey_metadata();
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
    let mut supervisor = CloudBackupSupervisor::new(
        Arc::downgrade(&manager),
        spawn_actor(CloudBackupWriteSupervisor::new(Weak::new())),
    );

    supervisor.confirm_pending_enable_cleanup().await.unwrap();

    assert_eq!(
        cspp.load_master_key_from_store().unwrap().unwrap().as_bytes(),
        prior_master_key.as_bytes()
    );
    assert_eq!(cloud_keychain.snapshot_passkey_metadata(), previous_metadata);
    assert_eq!(cspp.master_key_promotion_status().unwrap(), MasterKeyPromotionStatus::None);
    assert!(cloud_keychain.load_pending_enable_journal().unwrap().is_none());
    assert_eq!(globals.cloud.delete_namespace_attempt_count(), remote_deletes_before);
    assert!(matches!(manager.state().lifecycle, CloudBackupLifecycle::Configured(_)));
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
            cloud_keychain.snapshot_passkey_metadata(),
        ))
        .unwrap();
    let mut supervisor = CloudBackupSupervisor::new(
        Arc::downgrade(&manager),
        spawn_actor(CloudBackupWriteSupervisor::new(Weak::new())),
    );

    supervisor.confirm_pending_enable_cleanup().await.unwrap();

    assert_eq!(cspp.master_key_promotion_status().unwrap(), MasterKeyPromotionStatus::Staged);
    assert!(matches!(
        manager.state().lifecycle,
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
            cloud_keychain.snapshot_passkey_metadata(),
        ))
        .unwrap();
    globals.keychain.fail_delete_at(3);
    let mut supervisor = CloudBackupSupervisor::new(
        Arc::downgrade(&manager),
        spawn_actor(CloudBackupWriteSupervisor::new(Weak::new())),
    );

    supervisor.confirm_pending_enable_cleanup().await.unwrap();

    assert!(matches!(
        manager.state().lifecycle,
        CloudBackupLifecycle::PendingEnableRecovery(CloudBackupPendingEnableRecovery {
            support_code,
            cleanup: CloudBackupPendingEnableCleanupState::Available,
        }) if support_code == "CB-PE-006"
    ));
    assert!(cloud_keychain.load_pending_enable_journal().unwrap().is_some());
}



#[tokio::test(flavor = "current_thread")]
async fn supervisor_ignores_disable_blocker_completion_after_keep_enabled_restores_configured() {
    let _guard = async_test_lock().lock().await;
    let globals = test_globals();
    let manager = test_supervisor_manager();
    let mut supervisor = CloudBackupSupervisor::new(
        Arc::downgrade(&manager),
        spawn_actor(CloudBackupWriteSupervisor::new(Weak::new())),
    );

    let claim = supervisor
        .begin_exclusive_operation(&manager, CloudBackupExclusiveOperation::Disable)
        .unwrap();
    let mut disabling = test_disabling_state();
    disabling.delete_started_at = None;
    Database::global()
        .cloud_backup_state
        .set(&PersistedCloudBackupState::disabling_transition(disabling.clone()))
        .unwrap();

    assert!(manager.restore_configured_cloud_backup_after_disable(&disabling).unwrap());

    supervisor.complete_disable_blocker_check(claim, disabling, Ok(())).await.unwrap();

    assert_eq!(supervisor.active_operation, None);
    assert_eq!(manager.projected_exclusive_operation(), None);
    assert_eq!(globals.cloud.delete_namespace_attempt_count(), 0);
    assert!(matches!(
        Database::global().cloud_backup_state.get().unwrap(),
        PersistedCloudBackupState::Configured(_)
    ));
}

#[tokio::test(flavor = "current_thread")]
async fn keep_enabled_finishes_active_disable_operation() {
    let _guard = async_test_lock().lock().await;
    let manager = test_supervisor_manager();
    let mut supervisor = CloudBackupSupervisor::new(
        Arc::downgrade(&manager),
        spawn_actor(CloudBackupWriteSupervisor::new(Weak::new())),
    );

    let claim = supervisor
        .begin_exclusive_operation(&manager, CloudBackupExclusiveOperation::Disable)
        .unwrap();
    let mut disabling = test_disabling_state();
    disabling.delete_started_at = None;
    Database::global()
        .cloud_backup_state
        .set(&PersistedCloudBackupState::disabling_transition(disabling.clone()))
        .unwrap();
    supervisor.pending_disable_write_drain = Some(PendingDisableWriteDrain {
        claim,
        blocker: CloudBackupWriteBlocker::Disabling { operation_id: disabling.disable_generation },
        disabling: disabling.clone(),
    });

    supervisor
        .complete_keep_cloud_backup_enabled(Ok(CloudBackupKeepEnabledPreparation::Ready(Box::new(
            disabling,
        ))))
        .await
        .unwrap();

    assert_eq!(supervisor.active_operation, None);
    assert!(supervisor.pending_disable_write_drain.is_none());
    assert_eq!(manager.projected_exclusive_operation(), None);
    assert!(matches!(
        Database::global().cloud_backup_state.get().unwrap(),
        PersistedCloudBackupState::Configured(_)
    ));
}

#[tokio::test(flavor = "current_thread")]
async fn disable_runtime_drain_failure_remains_pre_delete() {
    let _guard = async_test_lock().lock().await;
    let manager = test_supervisor_manager();
    let mut supervisor = CloudBackupSupervisor::new(
        Arc::downgrade(&manager),
        spawn_actor(CloudBackupWriteSupervisor::new(Weak::new())),
    );
    supervisor.sync_health = Addr::detached();

    let claim = supervisor
        .begin_exclusive_operation(&manager, CloudBackupExclusiveOperation::Disable)
        .unwrap();
    let mut disabling = test_disabling_state();
    disabling.delete_started_at = None;
    Database::global()
        .cloud_backup_state
        .set(&PersistedCloudBackupState::disabling_transition(disabling.clone()))
        .unwrap();
    let blocker =
        CloudBackupWriteBlocker::Disabling { operation_id: disabling.disable_generation };
    supervisor.pending_disable_write_drain =
        Some(PendingDisableWriteDrain { claim, blocker, disabling });

    supervisor.complete_disable_write_drain(claim, blocker).await.unwrap();

    assert_eq!(supervisor.active_operation, None);
    assert_eq!(manager.projected_exclusive_operation(), None);
    let state = Database::global().cloud_backup_state.get().unwrap();
    let disabling = state.disabling().expect("expected persisted disabling state");
    assert!(disabling.delete_started_at.is_none());
    assert!(disabling.last_error.is_some());
    assert!(manager.disable_can_keep_enabled());
}






#[tokio::test(flavor = "current_thread")]
async fn supervisor_routes_write_blocker_commands_to_write_supervisor() {
    let _guard = async_test_lock().lock().await;
    let manager = test_supervisor_manager();
    let writes = spawn_actor(CloudBackupWriteSupervisor::new(Weak::new()));
    let mut supervisor = CloudBackupSupervisor::new(Arc::downgrade(&manager), writes.clone());
    let blocker = CloudBackupWriteBlocker::Disabling { operation_id: 9 };
    let namespace = "0123456789abcdef0123456789abcdef";

    call!(writes.block(blocker)).await.unwrap();

    let blocked_write = call!(writes.upload_wallet_backup(
        CloudStorage::global_explicit_client(),
        namespace.into(),
        "record".into(),
        vec![1, 2, 3]
    ))
    .await
    .unwrap()
    .await
    .unwrap();
    let blocked_write = blocked_write.into_result();
    assert!(matches!(blocked_write, Err(CloudBackupError::Deferred(_))));

    supervisor.unblock_cloud_backup_writes(blocker).await.unwrap();

    let allowed_write = call!(writes.upload_wallet_backup(
        CloudStorage::global_explicit_client(),
        namespace.into(),
        "record".into(),
        vec![1, 2, 3]
    ))
    .await
    .unwrap()
    .await
    .unwrap();
    let allowed_write = allowed_write.into_result();
    assert!(allowed_write.is_ok());
}

#[tokio::test(flavor = "current_thread")]
async fn restore_all_restart_marker_enters_detail_without_passkey_verification() {
    let _guard = async_test_lock().lock().await;
    let manager = test_supervisor_manager();
    prepare_restore_all_marker(&manager);
    let supervisor = CloudBackupSupervisor::new(
        Arc::downgrade(&manager),
        spawn_actor(CloudBackupWriteSupervisor::new(Weak::new())),
    );

    assert!(matches!(
        supervisor.detail_workflow.entry_plan(&manager),
        DetailEntryPlan::RefreshOnly
    ));
}

#[tokio::test(flavor = "current_thread")]
async fn cloud_only_refetch_clears_exhausted_restore_all_marker() {
    let _guard = async_test_lock().lock().await;
    let manager = test_supervisor_manager();
    prepare_restore_all_marker(&manager);
    let mut supervisor = CloudBackupSupervisor::new(
        Arc::downgrade(&manager),
        spawn_actor(CloudBackupWriteSupervisor::new(Weak::new())),
    );
    manager.apply_detail_outcome(CloudBackupDetailOutcome::Refreshed(listed_detail(&[])));
    supervisor.active_cloud_only_fetch_request = Some(7);

    supervisor.complete_cloud_only_fetch_request(7, Ok(complete_cloud_only_fetch(&manager, Vec::new()))).await.unwrap();

    assert!(RustCloudBackupManager::load_persisted_state().pending_restore_all().is_none());
}

#[tokio::test(flavor = "current_thread")]
async fn overlapping_cloud_only_refetch_preserves_active_restore_all_marker_and_progress() {
    let _guard = async_test_lock().lock().await;
    let manager = test_supervisor_manager();
    prepare_restore_all_marker(&manager);
    let mut supervisor = CloudBackupSupervisor::new(
        Arc::downgrade(&manager),
        spawn_actor(CloudBackupWriteSupervisor::new(Weak::new())),
    );
    let claim = supervisor
        .begin_exclusive_operation(
            &manager,
            CloudBackupExclusiveOperation::RestoreAllCloudWallets,
        )
        .unwrap();
    supervisor.active_operation.start_restore_all(RestoreRun::new(claim, Arc::new(AtomicBool::new(false))));
    manager.apply_restore_all_started(claim, 2);
    supervisor.active_cloud_only_fetch_request = Some(10);

    supervisor.complete_cloud_only_fetch_request(10, Ok(complete_cloud_only_fetch(&manager, Vec::new()))).await.unwrap();

    assert!(RustCloudBackupManager::load_persisted_state().pending_restore_all().is_some());
    assert!(matches!(
        manager.projected_restore_all_state(),
        CloudBackupRestoreAllState::Running { total: 2, .. }
    ));
    assert_eq!(supervisor.active_operation, Some(claim));
}

#[tokio::test(flavor = "current_thread")]
async fn cloud_only_refetch_retains_restore_all_marker_when_wallets_remain() {
    let _guard = async_test_lock().lock().await;
    let manager = test_supervisor_manager();
    prepare_restore_all_marker(&manager);
    let mut supervisor = CloudBackupSupervisor::new(
        Arc::downgrade(&manager),
        spawn_actor(CloudBackupWriteSupervisor::new(Weak::new())),
    );
    supervisor.active_cloud_only_fetch_request = Some(8);

    supervisor
        .complete_cloud_only_fetch_request(
            8,
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
    let mut supervisor = CloudBackupSupervisor::new(
        Arc::downgrade(&manager),
        spawn_actor(CloudBackupWriteSupervisor::new(Weak::new())),
    );
    supervisor.active_cloud_only_fetch_request = Some(9);

    supervisor
        .complete_cloud_only_fetch_request(
            9,
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
    manager.apply_detail_outcome(CloudBackupDetailOutcome::Refreshed(listed_detail(&[
        "a", "b",
    ])));
    manager.apply_cloud_only_fetch_outcome(complete_loaded_outcome(&manager, vec![
        test_cloud_only_wallet("a"),
        test_cloud_only_wallet("b"),
    ]));
    let mut supervisor = CloudBackupSupervisor::new(
        Arc::downgrade(&manager),
        spawn_actor(CloudBackupWriteSupervisor::new(Weak::new())),
    );
    let claim = supervisor
        .begin_exclusive_operation(&manager, CloudBackupExclusiveOperation::RestoreAllCloudWallets)
        .unwrap();
    supervisor.active_operation.start_restore_all(RestoreRun::new(claim, Arc::new(AtomicBool::new(false))));
    let detail_claim = supervisor.detail_workflow.start_operation_result();

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
    assert!(!matches!(
        manager.state.read().cloud_only(),
        CloudOnlyState::Loaded { wallets, .. } if wallets.iter().any(|wallet| wallet.record_id == "a")
    ));

    supervisor.complete_restore_all_queue_finished(claim).await.unwrap();

    assert_eq!(supervisor.active_operation, None);
    assert!(RustCloudBackupManager::load_persisted_state().pending_restore_all().is_some());
}

#[tokio::test(flavor = "current_thread")]
async fn cloud_only_operation_refresh_that_resets_the_cache_starts_a_fetch() {
    let _guard = async_test_lock().lock().await;
    let manager = test_supervisor_manager();
    configure_enabled_cloud_backup(&manager, test_globals(), 2);
    let [a, b, c] = upload_distinct_test_wallet_backups(&manager);
    let [a, b, c] = [a.as_str(), b.as_str(), c.as_str()];
    manager.apply_detail_outcome(CloudBackupDetailOutcome::Refreshed(listed_detail(&[a, b])));
    manager.apply_cloud_only_fetch_outcome(complete_loaded_outcome(&manager, vec![
        test_cloud_only_wallet(a),
        test_cloud_only_wallet(b),
    ]));
    list_test_wallet_files(&manager.current_namespace_id().unwrap(), &[b, c]);
    let mut supervisor = CloudBackupSupervisor::new(
        Arc::downgrade(&manager),
        spawn_actor(CloudBackupWriteSupervisor::new(Weak::new())),
    );
    let claim = supervisor
        .begin_exclusive_operation(&manager, CloudBackupExclusiveOperation::RestoreCloudWallet)
        .unwrap();
    let detail_claim = supervisor.detail_workflow.start_operation_result();

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
    manager.apply_detail_outcome(CloudBackupDetailOutcome::Refreshed(listed_detail(&[
        first, second,
    ])));
    manager.apply_cloud_only_fetch_outcome(complete_loaded_outcome(&manager, vec![
        test_cloud_only_wallet(first),
        test_cloud_only_wallet(second),
    ]));
    list_test_wallet_files(&manager.current_namespace_id().unwrap(), &[second, third]);
    let mut supervisor = CloudBackupSupervisor::new(
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
    call!(supervisor.complete_refresh_detail_from_snapshot(
        Some(CloudBackupDetailSnapshotCompletion::TrustedLocal {
            listed: listed_detail(&[second, third]),
            confirmation,
        }),
        DetailRefreshAttempt::Initial,
        claim
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
    manager.apply_detail_outcome(CloudBackupDetailOutcome::Refreshed(listed_detail(&[])));
    manager.apply_cloud_only_fetch_outcome(complete_loaded_outcome(&manager, Vec::new()));
    let mut supervisor = CloudBackupSupervisor::new(
        Arc::downgrade(&manager),
        spawn_actor(CloudBackupWriteSupervisor::new(Weak::new())),
    );
    let claim = supervisor
        .begin_exclusive_operation(
            &manager,
            CloudBackupExclusiveOperation::RestoreAllCloudWallets,
        )
        .unwrap();
    supervisor.active_operation.start_restore_all(RestoreRun::new(claim, Arc::new(AtomicBool::new(false))));

    supervisor.complete_restore_all_queue_finished(claim).await.unwrap();

    assert_eq!(supervisor.active_operation, None);
    assert!(RustCloudBackupManager::load_persisted_state().pending_restore_all().is_none());
}

#[tokio::test(flavor = "current_thread")]
async fn restore_all_cancellation_keeps_claim_until_record_boundary() {
    let _guard = async_test_lock().lock().await;
    let manager = test_supervisor_manager();
    let mut supervisor = CloudBackupSupervisor::new(
        Arc::downgrade(&manager),
        spawn_actor(CloudBackupWriteSupervisor::new(Weak::new())),
    );
    let claim = supervisor
        .begin_exclusive_operation(
            &manager,
            CloudBackupExclusiveOperation::RestoreAllCloudWallets,
        )
        .unwrap();
    let cancellation = Arc::new(AtomicBool::new(false));
    supervisor.active_operation.start_restore_all(RestoreRun::new(claim, cancellation.clone()));

    supervisor.request_restore_all_cancellation();

    assert!(cancellation.load(Ordering::Acquire));
    assert_eq!(supervisor.active_operation, Some(claim));
    assert_eq!(manager.projected_exclusive_operation(), Some(claim));
}

#[tokio::test(flavor = "current_thread")]
async fn supplemental_inventory_does_not_cancel_restore_all() {
    let _guard = async_test_lock().lock().await;
    let manager = test_supervisor_manager();
    let mut supervisor = CloudBackupSupervisor::new(
        Arc::downgrade(&manager),
        spawn_actor(CloudBackupWriteSupervisor::new(Weak::new())),
    );
    let claim = supervisor
        .begin_exclusive_operation(
            &manager,
            CloudBackupExclusiveOperation::RestoreAllCloudWallets,
        )
        .unwrap();
    let cancellation = Arc::new(AtomicBool::new(false));
    supervisor.active_operation.start_restore_all(RestoreRun::new(claim, cancellation.clone()));

    supervisor.ensure_supplemental_inventory_discovery(manager.clone());

    assert!(!cancellation.load(Ordering::Acquire));
    assert_eq!(supervisor.active_operation, Some(claim));
    assert_eq!(manager.projected_exclusive_operation(), Some(claim));
}


#[tokio::test(flavor = "current_thread")]
async fn completed_other_backups_scan_remains_cached() {
    let _guard = async_test_lock().lock().await;
    let manager = test_supervisor_manager();
    let mut supervisor = CloudBackupSupervisor::new(
        Arc::downgrade(&manager),
        spawn_actor(CloudBackupWriteSupervisor::new(Weak::new())),
    );
    let loaded = CloudBackupOtherBackupsState::Loaded {
        summary: CloudBackupOtherBackupsSummary::default(),
    };
    manager.apply_other_backups_state(loaded.clone());

    supervisor.ensure_supplemental_inventory_discovery(manager.clone());

    assert_eq!(manager.state.read().other_backups_state(), loaded);
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

    call!(manager.supervisor.start_restore_all_operation(false)).await.unwrap();
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
    let rows = wait_for_cloud_only_rows(&manager, &[failed_record_id], &[]).await;
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

    call!(manager.supervisor.start_restore_all_operation(false)).await.unwrap();
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

    call!(manager.supervisor.start_restore_all_operation(false)).await.unwrap();
    tokio::time::timeout(Duration::from_secs(2), gate.wait_until_blocked())
        .await
        .expect("first wallet restore should reach atomic download");
    call!(manager.supervisor.cancel_restore_all_operation()).await.unwrap();
    assert!(matches!(
        manager.projected_restore_all_state(),
        CloudBackupRestoreAllState::Running { cancellation_requested: true, .. }
    ));
    gate.release();
    wait_for_test_condition(Duration::from_secs(2), "cancelled Restore All should settle", || {
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
    assert!(matches!(
        manager.projected_restore_all_state(),
        CloudBackupRestoreAllState::RetryAvailable { wallet_count: 1 }
    ));
}

#[tokio::test(flavor = "current_thread")]
async fn restore_all_cancellation_during_preparation_clears_marker_after_completion() {
    let _guard = async_test_lock().lock().await;
    let manager = test_supervisor_manager();
    prepare_restore_all_marker(&manager);
    let mut supervisor = CloudBackupSupervisor::new(
        Arc::downgrade(&manager),
        spawn_actor(CloudBackupWriteSupervisor::new(Weak::new())),
    );
    let claim = supervisor
        .begin_exclusive_operation(
            &manager,
            CloudBackupExclusiveOperation::RestoreAllCloudWallets,
        )
        .unwrap();
    let cancellation = Arc::new(AtomicBool::new(true));
    supervisor.active_operation.start_restore_all(RestoreRun::new(claim, cancellation.clone()));

    supervisor
        .complete_restore_all_preparation(
            claim,
            Err(CloudBackupError::Offline("offline".into())),
        )
        .await
        .unwrap();

    assert_eq!(supervisor.active_operation, None);
    assert!(RustCloudBackupManager::load_persisted_state().pending_restore_all().is_none());
}

#[tokio::test(flavor = "current_thread")]
async fn stale_restore_all_record_dispatch_does_not_mutate_current_claim() {
    let _guard = async_test_lock().lock().await;
    let manager = test_supervisor_manager();
    let mut supervisor = CloudBackupSupervisor::new(
        Arc::downgrade(&manager),
        spawn_actor(CloudBackupWriteSupervisor::new(Weak::new())),
    );
    let current = supervisor
        .begin_exclusive_operation(
            &manager,
            CloudBackupExclusiveOperation::RestoreAllCloudWallets,
        )
        .unwrap();
    let stale = CloudBackupExclusiveOperationClaim::new(
        CloudBackupExclusiveOperation::RestoreAllCloudWallets,
        u64::MAX,
    );

    let applied = supervisor
        .begin_restore_all_record(stale, 0, test_cloud_only_wallet("stale"))
        .await
        .unwrap()
        .await
        .unwrap();

    assert!(!applied);
    assert_eq!(supervisor.active_operation, Some(current));
    assert_eq!(manager.projected_exclusive_operation(), Some(current));
}

#[tokio::test(flavor = "current_thread")]
async fn restore_all_provider_failure_during_success_refresh_stops_with_marker_retained() {
    let _guard = async_test_lock().lock().await;
    let manager = test_supervisor_manager();
    prepare_restore_all_marker(&manager);
    let mut supervisor = CloudBackupSupervisor::new(
        Arc::downgrade(&manager),
        spawn_actor(CloudBackupWriteSupervisor::new(Weak::new())),
    );
    let claim = supervisor
        .begin_exclusive_operation(
            &manager,
            CloudBackupExclusiveOperation::RestoreAllCloudWallets,
        )
        .unwrap();
    supervisor.active_operation.start_restore_all(RestoreRun::new(claim, Arc::new(AtomicBool::new(false))));
    let detail_claim = supervisor.detail_workflow.start_operation_result();

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
    assert_eq!(supervisor.active_operation, None);
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
    manager.apply_cloud_only_fetch_outcome(complete_loaded_outcome(&manager, vec![
        wallet.clone(),
    ]));
    let mut supervisor = CloudBackupSupervisor::new(
        Arc::downgrade(&manager),
        spawn_actor(CloudBackupWriteSupervisor::new(Weak::new())),
    );
    let claim = supervisor
        .begin_exclusive_operation(
            &manager,
            CloudBackupExclusiveOperation::RestoreAllCloudWallets,
        )
        .unwrap();
    supervisor.active_operation.start_restore_all(RestoreRun::new(claim, Arc::new(AtomicBool::new(false))));

    let listing_generation = manager.state.read().cloud_only_listing_generation();
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
    assert_eq!(supervisor.active_operation, Some(claim));
}

#[tokio::test(flavor = "current_thread")]
async fn cloud_only_fetch_started_before_a_listing_change_is_discarded_and_refetched() {
    let _guard = async_test_lock().lock().await;
    let manager = test_supervisor_manager();
    configure_enabled_cloud_backup(&manager, test_globals(), 2);
    let [a, b, c] = upload_distinct_test_wallet_backups(&manager);
    let [a, b, c] = [a.as_str(), b.as_str(), c.as_str()];
    manager.apply_detail_outcome(CloudBackupDetailOutcome::Refreshed(listed_detail(&[a, b])));
    let mut supervisor = CloudBackupSupervisor::new(
        Arc::downgrade(&manager),
        spawn_actor(CloudBackupWriteSupervisor::new(Weak::new())),
    );
    supervisor.active_cloud_only_fetch_request = Some(5);
    let stale_fetch = complete_cloud_only_fetch(
        &manager,
        vec![test_cloud_only_wallet(a), test_cloud_only_wallet(b)],
    );
    manager.apply_cloud_only_fetch_outcome(CloudBackupCloudOnlyFetchOutcome::Started);

    // provider confirmation replaces wallet a with c at the same count while the fetch runs
    list_test_wallet_files(&manager.current_namespace_id().unwrap(), &[b, c]);
    manager.apply_detail_outcome(CloudBackupDetailOutcome::Refreshed(listed_detail(&[b, c])));
    supervisor.complete_cloud_only_fetch_request(5, Ok(stale_fetch)).await.unwrap();

    assert_eq!(supervisor.active_cloud_only_fetch_request, None);
    assert!(!matches!(
        manager.state.read().cloud_only(),
        CloudOnlyState::Loaded { wallets, .. } if wallets.iter().any(|wallet| wallet.record_id == a)
    ));
    wait_for_cloud_only_rows(&manager, &[b, c], &[a]).await;
}

#[tokio::test(flavor = "current_thread")]
async fn restore_all_keeps_its_marker_when_a_listed_record_cannot_be_read() {
    let _guard = async_test_lock().lock().await;
    let globals = test_globals();
    let manager = test_supervisor_manager();
    configure_enabled_cloud_backup(&manager, globals, 0);
    let namespace = manager.current_namespace_id().unwrap();
    CloudBackupStore::global().persist_restore_all_marker(namespace.clone()).unwrap();
    manager.sync_persisted_state();

    // the provider lists the record but its download is not available yet
    let record_id = cove_cspp::backup_data::wallet_record_id("listed-but-unreadable");
    globals.cloud.set_wallet_files(
        namespace,
        vec![cove_cspp::backup_data::wallet_filename_from_record_id(&record_id)],
    );
    manager.apply_detail_outcome(CloudBackupDetailOutcome::Refreshed(listed_detail(&[
        record_id.as_str(),
    ])));
    let mut supervisor = CloudBackupSupervisor::new(
        Arc::downgrade(&manager),
        spawn_actor(CloudBackupWriteSupervisor::new(Weak::new())),
    );
    supervisor.active_cloud_only_fetch_request = Some(3);
    let generation = manager.state.read().cloud_only_listing_generation();
    let fetch = manager
        .do_fetch_cloud_only_wallets_with_client(CloudStorage::global_silent_client(), generation)
        .await
        .unwrap();
    assert_eq!(fetch.unresolved_record_ids, vec![record_id]);

    supervisor.complete_cloud_only_fetch_request(3, Ok(fetch)).await.unwrap();

    // the unread record keeps the list incomplete, so the retry stays reachable without any row
    assert!(RustCloudBackupManager::load_persisted_state().pending_restore_all().is_some());
    assert_eq!(
        public_cloud_only(&manager),
        Some(CloudOnlyState::Loaded { wallets: Vec::new(), incomplete: true })
    );
    assert_eq!(
        manager.projected_restore_all_state(),
        CloudBackupRestoreAllState::RetryAvailable { wallet_count: 0 }
    );
}

#[tokio::test(flavor = "current_thread")]
async fn incomplete_cloud_only_list_keeps_its_retry_and_check_again_reads_it_again() {
    let _guard = async_test_lock().lock().await;
    let manager = test_supervisor_manager();
    configure_enabled_cloud_backup(&manager, test_globals(), 0);
    let namespace = manager.current_namespace_id().unwrap();
    CloudBackupStore::global().persist_restore_all_marker(namespace.clone()).unwrap();
    manager.sync_persisted_state();

    // the provider lists the record before its download is available
    let wallet = xpub_only_wallet_metadata();
    let record_id = cove_cspp::backup_data::wallet_record_id(wallet.id.as_ref());
    let record_id = record_id.as_str();
    list_test_wallet_files(&namespace, &[record_id]);
    manager.apply_detail_outcome(CloudBackupDetailOutcome::Refreshed(listed_detail(&[record_id])));

    call!(manager.supervisor.ensure_cloud_only_fetch()).await.unwrap();
    wait_for_test_condition(Duration::from_secs(5), "the incomplete list should load", || {
        public_cloud_only(&manager)
            == Some(CloudOnlyState::Loaded { wallets: Vec::new(), incomplete: true })
    })
    .await;
    assert_eq!(
        manager.projected_restore_all_state(),
        CloudBackupRestoreAllState::RetryAvailable { wallet_count: 0 }
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
}

#[tokio::test(flavor = "current_thread")]
async fn restore_all_preparation_with_only_unreadable_wallets_keeps_its_marker() {
    let _guard = async_test_lock().lock().await;
    let globals = test_globals();
    let manager = test_supervisor_manager();
    configure_enabled_cloud_backup(&manager, globals, 0);
    let namespace = manager.current_namespace_id().unwrap();
    CloudBackupStore::global().persist_restore_all_marker(namespace.clone()).unwrap();
    manager.sync_persisted_state();

    // the selected wallet is listed but its download fails, so preparation queues nothing
    let record_id = cove_cspp::backup_data::wallet_record_id("selected-but-unreadable");
    globals.cloud.set_wallet_files(
        namespace,
        vec![cove_cspp::backup_data::wallet_filename_from_record_id(&record_id)],
    );
    manager.apply_detail_outcome(CloudBackupDetailOutcome::Refreshed(listed_detail(&[
        record_id.as_str(),
    ])));
    let generation = manager.state.read().cloud_only_listing_generation();
    let prepared = manager
        .prepare_restore_all_cloud_wallets(vec![test_cloud_only_wallet(&record_id)], generation)
        .await;

    let mut supervisor = CloudBackupSupervisor::new(
        Arc::downgrade(&manager),
        spawn_actor(CloudBackupWriteSupervisor::new(Weak::new())),
    );
    let claim = supervisor
        .begin_exclusive_operation(
            &manager,
            CloudBackupExclusiveOperation::RestoreAllCloudWallets,
        )
        .unwrap();
    supervisor.active_operation.start_restore_all(RestoreRun::new(claim, Arc::new(AtomicBool::new(false))));
    supervisor.complete_restore_all_preparation(claim, prepared).await.unwrap();

    assert_eq!(supervisor.active_operation, None);
    assert!(RustCloudBackupManager::load_persisted_state().pending_restore_all().is_some());
}

#[tokio::test(flavor = "current_thread")]
async fn restore_all_clears_its_marker_when_the_only_listed_record_duplicates_a_local_wallet() {
    let _guard = async_test_lock().lock().await;
    let manager = test_supervisor_manager();
    let fixture = seed_local_wallet_with_cloud_duplicate(&manager).await;
    let duplicate_record_id = fixture.duplicate_record_id.as_str();
    CloudBackupStore::global().persist_restore_all_marker(fixture.namespace.clone()).unwrap();
    manager.sync_persisted_state();
    manager.apply_detail_outcome(CloudBackupDetailOutcome::Refreshed(listed_detail(&[
        duplicate_record_id,
    ])));

    let generation = manager.state.read().cloud_only_listing_generation();
    let prepared = manager
        .prepare_restore_all_cloud_wallets(
            vec![test_cloud_only_wallet(duplicate_record_id)],
            generation,
        )
        .await
        .unwrap();

    // the duplicate is read as a row that Restore All never queues
    assert_eq!(
        cloud_only_row_statuses(&prepared.authoritative_fetch().wallets),
        vec![(duplicate_record_id, CloudBackupWalletStatus::AlreadyOnDevice)]
    );
    assert!(prepared.ordered_queue().is_empty());

    let mut supervisor = CloudBackupSupervisor::new(
        Arc::downgrade(&manager),
        spawn_actor(CloudBackupWriteSupervisor::new(Weak::new())),
    );
    let claim = supervisor
        .begin_exclusive_operation(
            &manager,
            CloudBackupExclusiveOperation::RestoreAllCloudWallets,
        )
        .unwrap();
    supervisor.active_operation.start_restore_all(RestoreRun::new(claim, Arc::new(AtomicBool::new(false))));
    supervisor.complete_restore_all_preparation(claim, Ok(prepared)).await.unwrap();

    assert_eq!(supervisor.active_operation, None);
    assert!(RustCloudBackupManager::load_persisted_state().pending_restore_all().is_none());
    let CloudOnlyState::Loaded { wallets, .. } = manager.state.read().cloud_only() else {
        panic!("expected the duplicate row to stay loaded");
    };
    assert_eq!(
        cloud_only_row_statuses(&wallets),
        vec![(duplicate_record_id, CloudBackupWalletStatus::AlreadyOnDevice)]
    );
}

#[tokio::test(flavor = "current_thread")]
async fn identity_duplicate_row_can_be_deleted_and_then_stops_blocking_disable() {
    let _guard = async_test_lock().lock().await;
    let globals = test_globals();
    let manager = test_supervisor_manager();
    let fixture = seed_local_wallet_with_cloud_duplicate(&manager).await;
    let duplicate_record_id = fixture.duplicate_record_id.as_str();
    manager.apply_detail_outcome(CloudBackupDetailOutcome::Refreshed(listed_detail(&[
        duplicate_record_id,
    ])));

    call!(manager.supervisor.ensure_cloud_only_fetch()).await.unwrap();
    let rows = wait_for_cloud_only_rows(&manager, &[duplicate_record_id], &[]).await;

    // the duplicate shows as a row, but Restore All has nothing to restore
    assert_eq!(
        cloud_only_row_statuses(&rows),
        vec![(duplicate_record_id, CloudBackupWalletStatus::AlreadyOnDevice)]
    );
    assert_eq!(manager.restore_all_proven_eligible_wallets(), Some(Vec::new()));

    let cloud = CloudStorage::global_silent_client();
    let disabling = disabling_for_current_namespace(&manager);
    assert!(matches!(
        manager.check_disable_blockers(&cloud, &disabling).await,
        Err(CloudBackupError::RecoveryRequired(_))
    ));

    call!(manager.supervisor.start_delete_cloud_wallet_operation(
        fixture.duplicate_record_id.clone()
    ))
    .await
    .unwrap();
    wait_for_test_condition(Duration::from_secs(5), "the duplicate delete should finish", || {
        !globals.cloud.has_wallet_backup(&fixture.namespace, duplicate_record_id)
            && manager.projected_exclusive_operation().is_none()
    })
    .await;

    // only the cloud record went away, the local wallet and its own backup stay
    assert!(local_wallet_exists(&fixture.local_wallet));
    assert!(globals.cloud.has_wallet_backup(&fixture.namespace, &fixture.local_record_id));
    assert!(manager.check_disable_blockers(&cloud, &disabling).await.is_ok());
}

#[tokio::test(flavor = "current_thread")]
async fn deleting_the_matching_local_wallet_makes_its_cloud_duplicate_restorable() {
    let _guard = async_test_lock().lock().await;
    let manager = test_supervisor_manager();
    let fixture = seed_local_wallet_with_cloud_duplicate(&manager).await;
    let duplicate_record_id = fixture.duplicate_record_id.as_str();
    manager.apply_detail_outcome(CloudBackupDetailOutcome::Refreshed(listed_detail(&[
        duplicate_record_id,
    ])));

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
        matches!(
            manager.state.read().cloud_only(),
            CloudOnlyState::Loaded { wallets, .. }
                if cloud_only_row_statuses(&wallets)
                    == vec![(duplicate_record_id, CloudBackupWalletStatus::DeletedFromDevice)]
        )
    })
    .await;
}

#[tokio::test(flavor = "current_thread")]
async fn background_verification_that_resets_loaded_rows_starts_a_fetch() {
    let _guard = async_test_lock().lock().await;
    let manager = test_supervisor_manager();
    configure_enabled_cloud_backup(&manager, test_globals(), 2);
    let [a, b, c] = upload_distinct_test_wallet_backups(&manager);
    let [a, b, c] = [a.as_str(), b.as_str(), c.as_str()];
    manager.apply_detail_outcome(CloudBackupDetailOutcome::Refreshed(listed_detail(&[a, b])));
    manager.apply_cloud_only_fetch_outcome(complete_loaded_outcome(&manager, vec![
        test_cloud_only_wallet(a),
        test_cloud_only_wallet(b),
    ]));
    list_test_wallet_files(&manager.current_namespace_id().unwrap(), &[b, c]);

    // a background report carries no listing, so the loaded rows cannot be matched and reset
    manager.apply_verification_effect(CloudBackupVerificationEffect {
        detail: Some(test_cloud_backup_detail(2)),
        ..Default::default()
    });
    assert_eq!(manager.state.read().cloud_only(), CloudOnlyState::NotFetched);

    wait_for_cloud_only_rows(&manager, &[b, c], &[a]).await;
}

#[tokio::test(flavor = "current_thread")]
async fn seed_backup_matching_only_a_watch_only_wallet_restores_into_that_wallet() {
    let _guard = async_test_lock().lock().await;
    let manager = test_supervisor_manager();
    configure_enabled_cloud_backup(&manager, test_globals(), 0);
    let namespace = manager.current_namespace_id().unwrap();
    let secret = test_seed_secret();
    let local_wallet = save_local_watch_only_wallet(&secret);
    let (seed_wallet, seed_record_id) = upload_test_seed_backup(&namespace, &secret);
    let seed_record_id = seed_record_id.as_str();
    list_test_wallet_files(&namespace, &[seed_record_id]);
    manager.apply_detail_outcome(CloudBackupDetailOutcome::Refreshed(listed_detail(&[
        seed_record_id,
    ])));

    call!(manager.supervisor.ensure_cloud_only_fetch()).await.unwrap();
    let rows = wait_for_cloud_only_rows(&manager, &[seed_record_id], &[]).await;

    // the watch-only wallet cannot sign, so the seed backup is restorable rather than an extra copy
    assert_eq!(
        cloud_only_row_statuses(&rows),
        vec![(seed_record_id, CloudBackupWalletStatus::DeletedFromDevice)]
    );
    assert_eq!(manager.restore_all_proven_eligible_wallets().map(|rows| rows.len()), Some(1));

    call!(manager.supervisor.start_restore_cloud_wallet_operation(seed_record_id.into()))
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
    let local_restore = manager.prepare_active_namespace_wallet_restore().await.unwrap();
    local_restore.restore_record(&local_record_id).await.unwrap();
    assert!(local_wallet_exists(&local_wallet));

    let (_, copy_record_id) = upload_test_seed_backup(&namespace, &secret);
    let copy_record_id = copy_record_id.as_str();
    list_test_wallet_files(&namespace, &[copy_record_id]);
    manager.apply_detail_outcome(CloudBackupDetailOutcome::Refreshed(listed_detail(&[
        copy_record_id,
    ])));

    call!(manager.supervisor.ensure_cloud_only_fetch()).await.unwrap();
    let rows = wait_for_cloud_only_rows(&manager, &[copy_record_id], &[]).await;

    assert_eq!(
        cloud_only_row_statuses(&rows),
        vec![(copy_record_id, CloudBackupWalletStatus::AlreadyOnDevice)]
    );
    assert_eq!(manager.restore_all_proven_eligible_wallets(), Some(Vec::new()));
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

    call!(manager.supervisor.start_restore_all_operation(false)).await.unwrap();
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
    CloudBackupStore::global().persist_restore_all_marker(namespace.clone()).unwrap();
    manager.sync_persisted_state();

    // the detail lists a record that another device deletes before the fetch lists again
    manager.apply_detail_outcome(CloudBackupDetailOutcome::Refreshed(listed_detail(&[
        "deleted-elsewhere",
    ])));
    list_test_wallet_files(&namespace, &[]);

    call!(manager.supervisor.ensure_cloud_only_fetch()).await.unwrap();
    wait_for_test_condition(Duration::from_secs(5), "the fetch should load", || {
        public_cloud_only(&manager)
            == Some(CloudOnlyState::Loaded { wallets: Vec::new(), incomplete: true })
    })
    .await;

    // the two listings disagree, so the marker stays and so does its retry
    assert!(RustCloudBackupManager::load_persisted_state().pending_restore_all().is_some());
    assert_eq!(
        manager.projected_restore_all_state(),
        CloudBackupRestoreAllState::RetryAvailable { wallet_count: 0 }
    );

    // Check Again lists the provider again, which settles the listing and clears the marker
    call!(manager.supervisor.start_refresh_detail()).await.unwrap();
    wait_for_test_condition(Duration::from_secs(5), "the new listing should settle", || {
        public_cloud_only(&manager)
            == Some(CloudOnlyState::Loaded { wallets: Vec::new(), incomplete: false })
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
    manager.apply_detail_outcome(CloudBackupDetailOutcome::Refreshed(listed_detail(&[
        first, second, third,
    ])));

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

    call!(manager.supervisor.start_restore_cloud_wallet_operation(first.into())).await.unwrap();

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
    assert_eq!(manager.restore_all_eligible_wallets(), Some(Vec::new()));
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

    call!(manager.supervisor.start_restore_all_operation(false)).await.unwrap();
    tokio::time::timeout(Duration::from_secs(2), gate.wait_until_blocked())
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
    assert!(RustCloudBackupManager::load_persisted_state().pending_restore_all().is_none());

    // the deleted wallet's record is cloud-only again, and the second wallet now covers it
    wait_for_cloud_only_statuses(&manager, &[(
        first_record_id,
        CloudBackupWalletStatus::AlreadyOnDevice,
    )])
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
    manager.apply_detail_outcome(CloudBackupDetailOutcome::Refreshed(listed_detail(&[
        seed_record_id,
    ])));

    call!(manager.supervisor.ensure_cloud_only_fetch()).await.unwrap();
    wait_for_cloud_only_statuses(&manager, &[(
        seed_record_id,
        CloudBackupWalletStatus::DeletedFromDevice,
    )])
    .await;

    call!(manager.supervisor.start_restore_cloud_wallet_operation(seed_record_id.into()))
        .await
        .unwrap();
    wait_for_test_condition(Duration::from_secs(5), "the restore should finish", || {
        manager.projected_exclusive_operation().is_none()
    })
    .await;

    // the restore fails without touching the unreadable seed, and the backup is not a copy
    let CloudOnlyState::Loaded { wallets, .. } = manager.state.read().cloud_only() else {
        panic!("expected the seed row to stay loaded");
    };
    assert_eq!(
        cloud_only_row_statuses(&wallets),
        vec![(seed_record_id, CloudBackupWalletStatus::DeletedFromDevice)]
    );
    assert!(wallets[0].restore_failure.is_some());
    assert_eq!(Keychain::global().get(secret_key), Some("garbage-secret".to_string()));
    assert_eq!(Keychain::global().get(cryptor_key), Some("garbage-cryptor".to_string()));
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
    manager.apply_detail_outcome(CloudBackupDetailOutcome::Refreshed(listed_detail(&[
        public_record_id,
    ])));

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

    let prepared = manager.prepare_active_namespace_wallet_restore().await.unwrap();
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
