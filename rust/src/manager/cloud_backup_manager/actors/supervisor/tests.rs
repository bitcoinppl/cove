use super::*;
use super::enable::{
    EnableRecoveryFinalization, EnableUploadFinalization, PendingEnableUploadSelection,
};
use super::verification::DeepVerificationContinuation;
use cove_cspp::{MasterKeyPromotionActiveState, MasterKeyPromotionStatus};
use crate::database::cloud_backup::{
    PersistedDisablingCloudBackup, PersistedDriveAccountSwitch,
    PersistedDriveAccountSwitchPhase,
};
use crate::manager::cloud_backup_manager::model::CloudBackupDetailState;
use crate::manager::cloud_backup_manager::ops::test_support::{
    async_test_lock, configure_enabled_cloud_backup, encrypted_wallet_backup_bytes_for_entry,
    persisted_enabled_cloud_backup_state, reset_cloud_backup_test_state, test_globals,
    staged_pending_enable_journal, wait_for_test_condition, wallet_entry_with_labels,
    xpub_only_wallet_metadata,
};
use crate::manager::cloud_backup_manager::reconcile::CloudBackupReconcileMessage;
use crate::manager::cloud_backup_manager::wallets::{
    StagedPrfKey, UnpersistedPrfKey, WalletRestoreOutcome,
};
use crate::manager::cloud_backup_manager::{
    CloudBackupDetail, CloudBackupLifecycle, CloudBackupOtherBackupsState,
    CloudBackupPendingEnableCleanupState, CloudBackupPendingEnableRecovery, CloudBackupStore,
    CloudBackupSettingsRowStatus, CloudOnlyState, PendingEnableJournal,
    PendingEnableNamespaceOwnership, PendingEnableJournalPhase, PendingEnablePasskeyMetadata,
    PendingEnableSessionMaterial,
};
use crate::manager::deferred_sender::SingleOrMany;
use crate::network::Network;
use crate::wallet::metadata::WalletMetadata;

const VALID_NAMESPACE_ID: &str = "0123456789abcdef0123456789abcdef";

fn test_supervisor_manager() -> Arc<RustCloudBackupManager> {
    let globals = test_globals();
    let manager = RustCloudBackupManager::init();
    reset_cloud_backup_test_state(&manager, globals);
    manager
}

fn test_disabling_state() -> PersistedDisablingCloudBackup {
    CloudBackupStore::global().persist_enabled(0).unwrap();
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

fn test_runtime_passkey_authorization() -> RuntimePasskeyAuthorization {
    RuntimePasskeyAuthorization {
        namespace_id: VALID_NAMESPACE_ID.into(),
        credential_id: vec![1, 2, 3],
        prf_salt: [9; 32],
    }
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
    manager.apply_detail_outcome(CloudBackupDetailOutcome::Refreshed(CloudBackupDetail {
        last_sync: None,
        up_to_date: Vec::new(),
        needs_sync: Vec::new(),
        cloud_only_count: items.len().try_into().unwrap(),
        other_backups: CloudBackupOtherBackupsState::Loaded { summary: Default::default() },
    }));
    manager.apply_cloud_only_fetch_outcome(CloudBackupCloudOnlyFetchOutcome::Loaded(items.clone()));
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
            wallets_failed: 0,
            wallets_unsupported: 0,
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

fn awaiting_force_new_session(
    master_key: cove_cspp::master_key::MasterKey,
    passkey: UnpersistedPrfKey,
) -> PendingEnableSession {
    PendingEnableSession::AwaitingForceNewConfirmation(PendingEnableSessionMaterial::new(
        master_key,
        passkey,
        CloudBackupEnableContext::settings_manual(),
    ))
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

#[tokio::test(flavor = "current_thread")]
async fn drive_account_switch_reports_busy_and_cancellation_releases_claim() {
    let _guard = async_test_lock().lock().await;
    let manager = test_supervisor_manager();
    CloudBackupStore::global().persist_enabled(0).unwrap();
    let mut supervisor = CloudBackupSupervisor::new(
        Arc::downgrade(&manager),
        spawn_actor(CloudBackupWriteSupervisor::new(Arc::downgrade(&manager))),
    );

    let transition_id = supervisor
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
    CloudBackupStore::global().persist_enabled(0).unwrap();
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

    supervisor
        .reconcile_drive_account_switch(DriveAccountSwitchPlatformState::NoTransition)
        .await
        .unwrap()
        .await
        .unwrap()
        .unwrap();

    assert_eq!(
        RustCloudBackupManager::load_persisted_state()
            .drive_account_switch()
            .map(|account_switch| account_switch.phase),
        Some(PersistedDriveAccountSwitchPhase::AwaitingAccountRollback)
    );
    assert!(manager.reconciler.receiver().try_iter().any(|messages| match messages {
        SingleOrMany::Single(message) => matches!(
            message,
            CloudBackupReconcileMessage::DriveAccountSwitchRollbackRequired(7)
        ),
        SingleOrMany::Many(messages) => messages.into_iter().any(|message| matches!(
            message,
            CloudBackupReconcileMessage::DriveAccountSwitchRollbackRequired(7)
        )),
    }));
}

#[tokio::test(flavor = "current_thread")]
async fn drive_account_switch_restart_replays_required_commit() {
    let _guard = async_test_lock().lock().await;
    let manager = test_supervisor_manager();
    CloudBackupStore::global().persist_enabled(0).unwrap();
    let transition = PersistedDriveAccountSwitch {
        transition_id: 7.into(),
        phase: PersistedDriveAccountSwitchPhase::AwaitingAccountCommitFailed,
    };
    let mut persisted = RustCloudBackupManager::load_persisted_state();
    assert!(persisted.set_drive_account_switch(transition));
    Database::global().cloud_backup_state.set(&persisted).unwrap();
    let mut supervisor = CloudBackupSupervisor::new(
        Arc::downgrade(&manager),
        spawn_actor(CloudBackupWriteSupervisor::new(Arc::downgrade(&manager))),
    );

    supervisor
        .reconcile_drive_account_switch(DriveAccountSwitchPlatformState::Staged(7))
        .await
        .unwrap()
        .await
        .unwrap()
        .unwrap();

    assert!(manager.reconciler.receiver().try_iter().any(|messages| match messages {
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
async fn drive_account_switch_restart_finalizes_platform_only_commit() {
    let _guard = async_test_lock().lock().await;
    let manager = test_supervisor_manager();
    CloudBackupStore::global().persist_enabled(0).unwrap();
    let mut supervisor = CloudBackupSupervisor::new(
        Arc::downgrade(&manager),
        spawn_actor(CloudBackupWriteSupervisor::new(Arc::downgrade(&manager))),
    );

    supervisor
        .reconcile_drive_account_switch(DriveAccountSwitchPlatformState::Committed(7))
        .await
        .unwrap()
        .await
        .unwrap()
        .unwrap();

    assert!(manager.reconciler.receiver().try_iter().any(|messages| match messages {
        SingleOrMany::Single(message) => matches!(
            message,
            CloudBackupReconcileMessage::DriveAccountSwitchFinalizeRequired(7)
        ),
        SingleOrMany::Many(messages) => messages.into_iter().any(|message| matches!(
            message,
            CloudBackupReconcileMessage::DriveAccountSwitchFinalizeRequired(7)
        )),
    }));
}

#[tokio::test(flavor = "current_thread")]
async fn drive_account_switch_restart_keeps_fence_for_mismatched_platform_transition() {
    let _guard = async_test_lock().lock().await;
    let manager = test_supervisor_manager();
    CloudBackupStore::global().persist_enabled(0).unwrap();
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

    supervisor
        .reconcile_drive_account_switch(DriveAccountSwitchPlatformState::Committed(8))
        .await
        .unwrap()
        .await
        .unwrap()
        .unwrap();

    assert_eq!(
        RustCloudBackupManager::load_persisted_state().drive_account_switch(),
        Some(&transition)
    );
    assert_eq!(
        supervisor.active_operation.claim().and_then(|claim| claim.drive_account_switch_id()),
        Some(transition.transition_id)
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

#[tokio::test(flavor = "current_thread")]
async fn supervisor_ignores_stale_exclusive_operation_completion() {
    let _guard = async_test_lock().lock().await;
    let manager = test_supervisor_manager();
    let mut supervisor = CloudBackupSupervisor::new(
        Arc::downgrade(&manager),
        spawn_actor(CloudBackupWriteSupervisor::new(Weak::new())),
    );

    let current = supervisor
        .begin_exclusive_operation(&manager, CloudBackupExclusiveOperation::Enable)
        .unwrap();
    let stale =
        CloudBackupExclusiveOperationClaim::new(CloudBackupExclusiveOperation::Enable, u64::MAX);

    supervisor.complete_exclusive_operation(stale).await.unwrap();

    assert_eq!(supervisor.active_operation, Some(current));
    assert_eq!(manager.projected_exclusive_operation(), Some(current));

    supervisor.complete_exclusive_operation(current).await.unwrap();

    assert_eq!(supervisor.active_operation, None);
    assert_eq!(manager.projected_exclusive_operation(), None);
}

#[tokio::test(flavor = "current_thread")]
async fn supervisor_ignores_stale_delete_cloud_wallet_completion() {
    let _guard = async_test_lock().lock().await;
    let manager = test_supervisor_manager();
    let mut supervisor = CloudBackupSupervisor::new(
        Arc::downgrade(&manager),
        spawn_actor(CloudBackupWriteSupervisor::new(Weak::new())),
    );

    let current = supervisor
        .begin_exclusive_operation(&manager, CloudBackupExclusiveOperation::DeleteCloudWallet)
        .unwrap();
    let stale = CloudBackupExclusiveOperationClaim::new(
        CloudBackupExclusiveOperation::DeleteCloudWallet,
        u64::MAX,
    );

    supervisor
        .complete_delete_cloud_wallet(
            stale,
            "wallet-record".into(),
            Err(CloudBackupError::Internal("stale completion".into())),
        )
        .await
        .unwrap();

    assert_eq!(supervisor.active_operation, Some(current));
    assert_eq!(manager.projected_exclusive_operation(), Some(current));

    supervisor
        .complete_delete_cloud_wallet(
            current,
            "wallet-record".into(),
            Err(CloudBackupError::Internal("current completion".into())),
        )
        .await
        .unwrap();

    assert_eq!(supervisor.active_operation, None);
    assert_eq!(manager.projected_exclusive_operation(), None);
}

#[tokio::test(flavor = "current_thread")]
async fn supervisor_ignores_stale_restore_cloud_wallet_completion() {
    let _guard = async_test_lock().lock().await;
    let manager = test_supervisor_manager();
    let mut supervisor = CloudBackupSupervisor::new(
        Arc::downgrade(&manager),
        spawn_actor(CloudBackupWriteSupervisor::new(Weak::new())),
    );

    let current = supervisor
        .begin_exclusive_operation(&manager, CloudBackupExclusiveOperation::RestoreCloudWallet)
        .unwrap();
    let stale = CloudBackupExclusiveOperationClaim::new(
        CloudBackupExclusiveOperation::RestoreCloudWallet,
        u64::MAX,
    );

    supervisor
        .complete_restore_cloud_wallet(
            stale,
            "wallet-record".into(),
            Ok(WalletRestoreOutcome::Restored { labels_warning: None }),
        )
        .await
        .unwrap();

    assert_eq!(supervisor.active_operation, Some(current));
    assert_eq!(manager.projected_exclusive_operation(), Some(current));

    supervisor
        .complete_restore_cloud_wallet(
            current,
            "wallet-record".into(),
            Err(CloudBackupError::Internal("current completion".into())),
        )
        .await
        .unwrap();

    assert_eq!(supervisor.active_operation, None);
    assert_eq!(manager.projected_exclusive_operation(), None);
}

#[tokio::test(flavor = "current_thread")]
async fn supervisor_ignores_stale_repair_passkey_wrapper_completion() {
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

    supervisor
        .complete_repair_passkey_wrapper(
            stale,
            Err(CloudBackupError::Internal("stale completion".into())),
        )
        .await
        .unwrap();

    assert_eq!(supervisor.active_operation, Some(current));
    assert_eq!(manager.projected_exclusive_operation(), Some(current));

    supervisor
        .complete_repair_passkey_wrapper(
            current,
            Err(CloudBackupError::Internal("current completion".into())),
        )
        .await
        .unwrap();

    assert_eq!(supervisor.active_operation, None);
    assert_eq!(manager.projected_exclusive_operation(), None);
}

#[tokio::test(flavor = "current_thread")]
async fn supervisor_ignores_stale_repair_passkey_finalization_completion() {
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
    let finalization = CloudBackupPasskeyRepairFinalization { wallet_count: 2 };

    supervisor.complete_repair_passkey_finalization(stale, Ok(finalization)).await.unwrap();

    assert_eq!(supervisor.active_operation, Some(current));
    assert_eq!(manager.projected_exclusive_operation(), Some(current));

    supervisor
        .complete_repair_passkey_finalization(
            current,
            Err(CloudBackupError::Internal("current completion".into())),
        )
        .await
        .unwrap();

    assert_eq!(supervisor.active_operation, None);
    assert_eq!(manager.projected_exclusive_operation(), None);
}

#[tokio::test(flavor = "current_thread")]
async fn supervisor_ignores_stale_repair_passkey_wrapper_upload_completion() {
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

    supervisor
        .complete_repair_passkey_wrapper_upload(
            stale,
            Ok((
                CloudBackupUploadedPasskeyWrapperRepair { namespace_id: "stale".into() },
                test_runtime_passkey_authorization(),
            )),
        )
        .await
        .unwrap();

    assert_eq!(supervisor.active_operation, Some(current));
    assert_eq!(manager.projected_exclusive_operation(), Some(current));

    supervisor
        .complete_repair_passkey_wrapper_upload(
            current,
            Err(CloudBackupError::Internal("current completion".into())),
        )
        .await
        .unwrap();

    assert_eq!(supervisor.active_operation, None);
    assert_eq!(manager.projected_exclusive_operation(), None);
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
            Some(CloudBackupDetailResult::Success(CloudBackupDetail {
                last_sync: None,
                up_to_date: Vec::new(),
                needs_sync: Vec::new(),
                cloud_only_count: 0,
                other_backups: CloudBackupOtherBackupsState::Loaded {
                    summary: Default::default(),
                },
            })),
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
async fn supervisor_ignores_stale_recreate_manifest_recovery_completion() {
    let _guard = async_test_lock().lock().await;
    let manager = test_supervisor_manager();
    let mut supervisor = CloudBackupSupervisor::new(
        Arc::downgrade(&manager),
        spawn_actor(CloudBackupWriteSupervisor::new(Weak::new())),
    );

    let current = supervisor
        .begin_exclusive_operation(&manager, CloudBackupExclusiveOperation::RecreateManifest)
        .unwrap();
    let stale = CloudBackupExclusiveOperationClaim::new(
        CloudBackupExclusiveOperation::RecreateManifest,
        u64::MAX,
    );

    supervisor
        .complete_recreate_manifest_recovery(
            stale,
            Err(CloudBackupError::Internal("stale completion".into())),
        )
        .await
        .unwrap();

    assert_eq!(supervisor.active_operation, Some(current));
    assert_eq!(manager.projected_exclusive_operation(), Some(current));

    supervisor
        .complete_recreate_manifest_recovery(
            current,
            Err(CloudBackupError::Internal("current completion".into())),
        )
        .await
        .unwrap();

    assert_eq!(supervisor.active_operation, None);
    assert_eq!(manager.projected_exclusive_operation(), None);
}

#[tokio::test(flavor = "current_thread")]
async fn supervisor_ignores_stale_recreate_manifest_finalization_completion() {
    let _guard = async_test_lock().lock().await;
    let manager = test_supervisor_manager();
    let mut supervisor = CloudBackupSupervisor::new(
        Arc::downgrade(&manager),
        spawn_actor(CloudBackupWriteSupervisor::new(Weak::new())),
    );

    let current = supervisor
        .begin_exclusive_operation(&manager, CloudBackupExclusiveOperation::RecreateManifest)
        .unwrap();
    let stale = CloudBackupExclusiveOperationClaim::new(
        CloudBackupExclusiveOperation::RecreateManifest,
        u64::MAX,
    );

    supervisor
        .complete_recreate_manifest_finalization(
            stale,
            Err(CloudBackupError::Internal("stale completion".into())),
        )
        .await
        .unwrap();

    assert_eq!(supervisor.active_operation, Some(current));
    assert_eq!(manager.projected_exclusive_operation(), Some(current));

    supervisor
        .complete_recreate_manifest_finalization(
            current,
            Err(CloudBackupError::Internal("current completion".into())),
        )
        .await
        .unwrap();

    assert_eq!(supervisor.active_operation, None);
    assert_eq!(manager.projected_exclusive_operation(), None);
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

    supervisor.complete_cloud_only_fetch_request(6, Ok(Vec::new())).await.unwrap();

    assert_eq!(supervisor.active_cloud_only_fetch_request, Some(7));

    supervisor.complete_cloud_only_fetch_request(7, Ok(Vec::new())).await.unwrap();

    assert_eq!(supervisor.active_cloud_only_fetch_request, None);
}

#[tokio::test(flavor = "current_thread")]
async fn supervisor_ignores_stale_recreate_manifest_verification_completion() {
    let _guard = async_test_lock().lock().await;
    let manager = test_supervisor_manager();
    let mut supervisor = CloudBackupSupervisor::new(
        Arc::downgrade(&manager),
        spawn_actor(CloudBackupWriteSupervisor::new(Weak::new())),
    );

    let current = supervisor
        .begin_exclusive_operation(&manager, CloudBackupExclusiveOperation::RecreateManifest)
        .unwrap();
    let stale = CloudBackupExclusiveOperationClaim::new(
        CloudBackupExclusiveOperation::RecreateManifest,
        u64::MAX,
    );

    supervisor
        .complete_verification(
            Some(stale),
            CloudBackupDeepVerificationStep::Complete(DeepVerificationResult::NotEnabled),
            DeepVerificationContinuation::RecreateManifest {
                attempt: VerificationAttempt::Initial,
            },
        )
        .await
        .unwrap();

    assert_eq!(supervisor.active_operation, Some(current));
    assert_eq!(manager.projected_exclusive_operation(), Some(current));

    supervisor
        .complete_verification(
            Some(current),
            CloudBackupDeepVerificationStep::Complete(DeepVerificationResult::NotEnabled),
            DeepVerificationContinuation::RecreateManifest {
                attempt: VerificationAttempt::Initial,
            },
        )
        .await
        .unwrap();

    assert_eq!(supervisor.active_operation, None);
    assert_eq!(manager.projected_exclusive_operation(), None);
}

#[tokio::test(flavor = "current_thread")]
async fn supervisor_ignores_stale_deep_verification_wrapper_repair_upload_completion() {
    let _guard = async_test_lock().lock().await;
    let manager = test_supervisor_manager();
    let mut supervisor = CloudBackupSupervisor::new(
        Arc::downgrade(&manager),
        spawn_actor(CloudBackupWriteSupervisor::new(Weak::new())),
    );

    let current = supervisor
        .begin_exclusive_operation(&manager, CloudBackupExclusiveOperation::VerificationRepair)
        .unwrap();
    let stale = CloudBackupExclusiveOperationClaim::new(
        CloudBackupExclusiveOperation::VerificationRepair,
        u64::MAX,
    );
    let continuation = DeepVerificationContinuation::Manual {
        force_discoverable: true,
        attempt: VerificationAttempt::Initial,
    };

    supervisor
        .complete_deep_verification_wrapper_repair_upload(
            stale,
            continuation,
            Err(CloudBackupError::Internal("stale completion".into())),
        )
        .await
        .unwrap();

    assert_eq!(supervisor.active_operation, Some(current));
    assert_eq!(manager.projected_exclusive_operation(), Some(current));

    supervisor
        .complete_deep_verification_wrapper_repair_upload(
            current,
            continuation,
            Err(CloudBackupError::Internal("current completion".into())),
        )
        .await
        .unwrap();

    assert_eq!(supervisor.active_operation, None);
    assert_eq!(manager.projected_exclusive_operation(), None);
}

#[tokio::test(flavor = "current_thread")]
async fn supervisor_ignores_stale_deep_verification_wrapper_repair_resume_completion() {
    let _guard = async_test_lock().lock().await;
    let manager = test_supervisor_manager();
    let mut supervisor = CloudBackupSupervisor::new(
        Arc::downgrade(&manager),
        spawn_actor(CloudBackupWriteSupervisor::new(Weak::new())),
    );

    let current = supervisor
        .begin_exclusive_operation(&manager, CloudBackupExclusiveOperation::VerificationRepair)
        .unwrap();
    let stale = CloudBackupExclusiveOperationClaim::new(
        CloudBackupExclusiveOperation::VerificationRepair,
        u64::MAX,
    );
    let continuation = DeepVerificationContinuation::Manual {
        force_discoverable: true,
        attempt: VerificationAttempt::Initial,
    };

    supervisor
        .complete_deep_verification_wrapper_repair_resume(
            stale,
            continuation,
            CloudBackupDeepVerificationStep::Complete(DeepVerificationResult::NotEnabled),
        )
        .await
        .unwrap();

    assert_eq!(supervisor.active_operation, Some(current));
    assert_eq!(manager.projected_exclusive_operation(), Some(current));

    supervisor
        .complete_deep_verification_wrapper_repair_resume(
            current,
            continuation,
            CloudBackupDeepVerificationStep::Complete(DeepVerificationResult::NotEnabled),
        )
        .await
        .unwrap();

    assert_eq!(supervisor.active_operation, None);
    assert_eq!(manager.projected_exclusive_operation(), None);
}

#[tokio::test(flavor = "current_thread")]
async fn supervisor_ignores_stale_deep_verification_auto_sync_upload_completion() {
    let _guard = async_test_lock().lock().await;
    let manager = test_supervisor_manager();
    let mut supervisor = CloudBackupSupervisor::new(
        Arc::downgrade(&manager),
        spawn_actor(CloudBackupWriteSupervisor::new(Weak::new())),
    );

    let current = supervisor
        .begin_exclusive_operation(&manager, CloudBackupExclusiveOperation::VerificationRepair)
        .unwrap();
    let stale = CloudBackupExclusiveOperationClaim::new(
        CloudBackupExclusiveOperation::VerificationRepair,
        u64::MAX,
    );
    let continuation = DeepVerificationContinuation::Manual {
        force_discoverable: true,
        attempt: VerificationAttempt::Initial,
    };

    supervisor
        .complete_deep_verification_auto_sync_upload(
            stale,
            continuation,
            Err(DeepVerificationResult::NotEnabled),
        )
        .await
        .unwrap();

    assert_eq!(supervisor.active_operation, Some(current));
    assert_eq!(manager.projected_exclusive_operation(), Some(current));

    supervisor
        .complete_deep_verification_auto_sync_upload(
            current,
            continuation,
            Err(DeepVerificationResult::NotEnabled),
        )
        .await
        .unwrap();

    assert_eq!(supervisor.active_operation, None);
    assert_eq!(manager.projected_exclusive_operation(), None);
}

#[tokio::test(flavor = "current_thread")]
async fn supervisor_ignores_stale_deep_verification_auto_sync_finalization_completion() {
    let _guard = async_test_lock().lock().await;
    let manager = test_supervisor_manager();
    let mut supervisor = CloudBackupSupervisor::new(
        Arc::downgrade(&manager),
        spawn_actor(CloudBackupWriteSupervisor::new(Weak::new())),
    );

    let current = supervisor
        .begin_exclusive_operation(&manager, CloudBackupExclusiveOperation::VerificationRepair)
        .unwrap();
    let stale = CloudBackupExclusiveOperationClaim::new(
        CloudBackupExclusiveOperation::VerificationRepair,
        u64::MAX,
    );
    let continuation = DeepVerificationContinuation::Manual {
        force_discoverable: true,
        attempt: VerificationAttempt::Initial,
    };

    supervisor
        .complete_deep_verification_auto_sync_finalization(
            stale,
            continuation,
            Err(DeepVerificationResult::NotEnabled),
        )
        .await
        .unwrap();

    assert_eq!(supervisor.active_operation, Some(current));
    assert_eq!(manager.projected_exclusive_operation(), Some(current));

    supervisor
        .complete_deep_verification_auto_sync_finalization(
            current,
            continuation,
            Err(DeepVerificationResult::NotEnabled),
        )
        .await
        .unwrap();

    assert_eq!(supervisor.active_operation, None);
    assert_eq!(manager.projected_exclusive_operation(), None);
}

#[tokio::test(flavor = "current_thread")]
async fn supervisor_ignores_stale_deep_verification_auto_sync_resume_completion() {
    let _guard = async_test_lock().lock().await;
    let manager = test_supervisor_manager();
    let mut supervisor = CloudBackupSupervisor::new(
        Arc::downgrade(&manager),
        spawn_actor(CloudBackupWriteSupervisor::new(Weak::new())),
    );

    let current = supervisor
        .begin_exclusive_operation(&manager, CloudBackupExclusiveOperation::VerificationRepair)
        .unwrap();
    let stale = CloudBackupExclusiveOperationClaim::new(
        CloudBackupExclusiveOperation::VerificationRepair,
        u64::MAX,
    );
    let continuation = DeepVerificationContinuation::Manual {
        force_discoverable: true,
        attempt: VerificationAttempt::Initial,
    };

    supervisor
        .complete_deep_verification_auto_sync_resume(
            stale,
            continuation,
            CloudBackupDeepVerificationAutoSyncCompletion::complete(
                DeepVerificationResult::NotEnabled,
            ),
        )
        .await
        .unwrap();

    assert_eq!(supervisor.active_operation, Some(current));
    assert_eq!(manager.projected_exclusive_operation(), Some(current));

    supervisor
        .complete_deep_verification_auto_sync_resume(
            current,
            continuation,
            CloudBackupDeepVerificationAutoSyncCompletion::complete(
                DeepVerificationResult::NotEnabled,
            ),
        )
        .await
        .unwrap();

    assert_eq!(supervisor.active_operation, None);
    assert_eq!(manager.projected_exclusive_operation(), None);
}

#[tokio::test(flavor = "current_thread")]
async fn supervisor_ignores_stale_reinitialize_backup_completion() {
    let _guard = async_test_lock().lock().await;
    let manager = test_supervisor_manager();
    let mut supervisor = CloudBackupSupervisor::new(
        Arc::downgrade(&manager),
        spawn_actor(CloudBackupWriteSupervisor::new(Weak::new())),
    );

    let current = supervisor
        .begin_exclusive_operation(&manager, CloudBackupExclusiveOperation::ReinitializeBackup)
        .unwrap();
    let stale = CloudBackupExclusiveOperationClaim::new(
        CloudBackupExclusiveOperation::ReinitializeBackup,
        u64::MAX,
    );

    supervisor
        .complete_enable_preparation(
            stale,
            Err(CloudBackupError::Internal("stale completion".into())),
        )
        .await
        .unwrap();

    assert_eq!(supervisor.active_operation, Some(current));
    assert_eq!(manager.projected_exclusive_operation(), Some(current));

    supervisor
        .complete_enable_preparation(
            current,
            Err(CloudBackupError::Internal("current completion".into())),
        )
        .await
        .unwrap();

    assert_eq!(supervisor.active_operation, None);
    assert_eq!(manager.projected_exclusive_operation(), None);
}

#[tokio::test(flavor = "current_thread")]
async fn supervisor_ignores_stale_reinitialize_verification_completion() {
    let _guard = async_test_lock().lock().await;
    let manager = test_supervisor_manager();
    let mut supervisor = CloudBackupSupervisor::new(
        Arc::downgrade(&manager),
        spawn_actor(CloudBackupWriteSupervisor::new(Weak::new())),
    );

    let current = supervisor
        .begin_exclusive_operation(&manager, CloudBackupExclusiveOperation::ReinitializeBackup)
        .unwrap();
    let stale = CloudBackupExclusiveOperationClaim::new(
        CloudBackupExclusiveOperation::ReinitializeBackup,
        u64::MAX,
    );

    supervisor
        .complete_verification(
            Some(stale),
            CloudBackupDeepVerificationStep::Complete(DeepVerificationResult::NotEnabled),
            DeepVerificationContinuation::ReinitializeBackup {
                attempt: VerificationAttempt::Initial,
            },
        )
        .await
        .unwrap();

    assert_eq!(supervisor.active_operation, Some(current));
    assert_eq!(manager.projected_exclusive_operation(), Some(current));

    supervisor
        .complete_verification(
            Some(current),
            CloudBackupDeepVerificationStep::Complete(DeepVerificationResult::NotEnabled),
            DeepVerificationContinuation::ReinitializeBackup {
                attempt: VerificationAttempt::Initial,
            },
        )
        .await
        .unwrap();

    assert_eq!(supervisor.active_operation, None);
    assert_eq!(manager.projected_exclusive_operation(), None);
}

#[tokio::test(flavor = "current_thread")]
async fn supervisor_ignores_stale_saved_passkey_confirmation_completion() {
    let _guard = async_test_lock().lock().await;
    let manager = test_supervisor_manager();
    let mut supervisor = CloudBackupSupervisor::new(
        Arc::downgrade(&manager),
        spawn_actor(CloudBackupWriteSupervisor::new(Weak::new())),
    );

    let current = supervisor
        .begin_exclusive_operation(&manager, CloudBackupExclusiveOperation::Enable)
        .unwrap();
    let stale =
        CloudBackupExclusiveOperationClaim::new(CloudBackupExclusiveOperation::Enable, u64::MAX);

    supervisor
        .complete_saved_passkey_confirmation(
            stale,
            CloudBackupSavedPasskeyConfirmation::Failed(CloudBackupError::Internal(
                "stale completion".into(),
            )),
        )
        .await
        .unwrap();

    assert_eq!(supervisor.active_operation, Some(current));
    assert_eq!(manager.projected_exclusive_operation(), Some(current));

    supervisor
        .complete_saved_passkey_confirmation(
            current,
            CloudBackupSavedPasskeyConfirmation::Failed(CloudBackupError::Internal(
                "current completion".into(),
            )),
        )
        .await
        .unwrap();

    assert_eq!(supervisor.active_operation, None);
    assert_eq!(manager.projected_exclusive_operation(), None);
}

#[tokio::test(flavor = "current_thread")]
async fn supervisor_ignores_stale_enable_passkey_registration_completion() {
    let _guard = async_test_lock().lock().await;
    let manager = test_supervisor_manager();
    let mut supervisor = CloudBackupSupervisor::new(
        Arc::downgrade(&manager),
        spawn_actor(CloudBackupWriteSupervisor::new(Weak::new())),
    );

    let current = supervisor
        .begin_exclusive_operation(&manager, CloudBackupExclusiveOperation::EnableForceNew)
        .unwrap();
    let stale = CloudBackupExclusiveOperationClaim::new(
        CloudBackupExclusiveOperation::EnableForceNew,
        u64::MAX,
    );

    supervisor
        .complete_enable_passkey_registration(
            stale,
            Ok(CloudBackupEnablePasskeyRegistration::Cancelled {
                context: CloudBackupEnableContext::settings_manual(),
            }),
        )
        .await
        .unwrap();

    assert_eq!(supervisor.active_operation, Some(current));
    assert_eq!(manager.projected_exclusive_operation(), Some(current));

    supervisor
        .complete_enable_passkey_registration(
            current,
            Ok(CloudBackupEnablePasskeyRegistration::Cancelled {
                context: CloudBackupEnableContext::settings_manual(),
            }),
        )
        .await
        .unwrap();

    assert_eq!(supervisor.active_operation, None);
    assert_eq!(manager.projected_exclusive_operation(), None);
}

#[tokio::test(flavor = "current_thread")]
async fn supervisor_ignores_stale_enable_preparation_completion() {
    let _guard = async_test_lock().lock().await;
    let manager = test_supervisor_manager();
    let mut supervisor = CloudBackupSupervisor::new(
        Arc::downgrade(&manager),
        spawn_actor(CloudBackupWriteSupervisor::new(Weak::new())),
    );

    let current = supervisor
        .begin_exclusive_operation(&manager, CloudBackupExclusiveOperation::Enable)
        .unwrap();
    let stale =
        CloudBackupExclusiveOperationClaim::new(CloudBackupExclusiveOperation::Enable, u64::MAX);

    supervisor
        .complete_enable_preparation(
            stale,
            Ok(CloudBackupEnablePreparation::ExistingBackupFound {
                context: CloudBackupEnableContext::settings_manual(),
                passkey_hint: None,
            }),
        )
        .await
        .unwrap();

    assert_eq!(supervisor.active_operation, Some(current));
    assert_eq!(manager.projected_exclusive_operation(), Some(current));

    supervisor
        .complete_enable_preparation(
            current,
            Ok(CloudBackupEnablePreparation::ExistingBackupFound {
                context: CloudBackupEnableContext::settings_manual(),
                passkey_hint: None,
            }),
        )
        .await
        .unwrap();

    assert_eq!(supervisor.active_operation, None);
    assert_eq!(manager.projected_exclusive_operation(), None);
}

#[tokio::test(flavor = "current_thread")]
async fn supervisor_ignores_stale_create_new_enable_passkey_completion() {
    let _guard = async_test_lock().lock().await;
    let manager = test_supervisor_manager();
    let mut supervisor = CloudBackupSupervisor::new(
        Arc::downgrade(&manager),
        spawn_actor(CloudBackupWriteSupervisor::new(Weak::new())),
    );

    let current = supervisor
        .begin_exclusive_operation(&manager, CloudBackupExclusiveOperation::Enable)
        .unwrap();
    let stale =
        CloudBackupExclusiveOperationClaim::new(CloudBackupExclusiveOperation::Enable, u64::MAX);

    supervisor
        .complete_create_new_enable_passkey(
            stale,
            Ok(CloudBackupEnablePasskeyPreparation::Cancelled {
                context: CloudBackupEnableContext::settings_manual(),
            }),
        )
        .await
        .unwrap();

    assert_eq!(supervisor.active_operation, Some(current));
    assert_eq!(manager.projected_exclusive_operation(), Some(current));

    supervisor
        .complete_create_new_enable_passkey(
            current,
            Ok(CloudBackupEnablePasskeyPreparation::Cancelled {
                context: CloudBackupEnableContext::settings_manual(),
            }),
        )
        .await
        .unwrap();

    assert_eq!(supervisor.active_operation, None);
    assert_eq!(manager.projected_exclusive_operation(), None);
}

#[tokio::test(flavor = "current_thread")]
async fn supervisor_ignores_stale_enable_recovery_completion() {
    let _guard = async_test_lock().lock().await;
    let manager = test_supervisor_manager();
    let mut supervisor = CloudBackupSupervisor::new(
        Arc::downgrade(&manager),
        spawn_actor(CloudBackupWriteSupervisor::new(Weak::new())),
    );

    let current = supervisor
        .begin_exclusive_operation(&manager, CloudBackupExclusiveOperation::Enable)
        .unwrap();
    let stale =
        CloudBackupExclusiveOperationClaim::new(CloudBackupExclusiveOperation::Enable, u64::MAX);

    supervisor
        .complete_enable_recovery(stale, Err(CloudBackupError::Internal("stale completion".into())))
        .await
        .unwrap();

    assert_eq!(supervisor.active_operation, Some(current));
    assert_eq!(manager.projected_exclusive_operation(), Some(current));

    supervisor
        .complete_enable_recovery(
            current,
            Err(CloudBackupError::Internal("current completion".into())),
        )
        .await
        .unwrap();

    assert_eq!(supervisor.active_operation, None);
    assert_eq!(manager.projected_exclusive_operation(), None);
}

#[tokio::test(flavor = "current_thread")]
async fn supervisor_ignores_stale_enable_recovery_finalization_completion() {
    let _guard = async_test_lock().lock().await;
    let manager = test_supervisor_manager();
    let mut supervisor = CloudBackupSupervisor::new(
        Arc::downgrade(&manager),
        spawn_actor(CloudBackupWriteSupervisor::new(Weak::new())),
    );

    let current = supervisor
        .begin_exclusive_operation(&manager, CloudBackupExclusiveOperation::Enable)
        .unwrap();
    let stale =
        CloudBackupExclusiveOperationClaim::new(CloudBackupExclusiveOperation::Enable, u64::MAX);

    supervisor
        .complete_enable_recovery_finalization(
            stale,
            EnableRecoveryFinalization {
                context: CloudBackupEnableContext::settings_manual(),
                namespace_id: "stale-namespace".into(),
                credential_id: vec![1, 2, 3],
                prf_salt: [9; 32],
                active_critical_key: zeroize::Zeroizing::new([0; 32]),
                pending_completion: test_pending_completion(
                    "stale-namespace".into(),
                    Vec::new(),
                ),
                cleanup_sources: Vec::new(),
            },
            Err(CloudBackupError::Internal("stale completion".into())),
        )
        .await
        .unwrap();

    assert_eq!(supervisor.active_operation, Some(current));
    assert_eq!(manager.projected_exclusive_operation(), Some(current));

    supervisor
        .complete_enable_recovery_finalization(
            current,
            EnableRecoveryFinalization {
                context: CloudBackupEnableContext::settings_manual(),
                namespace_id: "current-namespace".into(),
                credential_id: vec![1, 2, 3],
                prf_salt: [9; 32],
                active_critical_key: zeroize::Zeroizing::new([0; 32]),
                pending_completion: test_pending_completion(
                    "current-namespace".into(),
                    Vec::new(),
                ),
                cleanup_sources: Vec::new(),
            },
            Err(CloudBackupError::Internal("current completion".into())),
        )
        .await
        .unwrap();

    assert_eq!(supervisor.active_operation, None);
    assert_eq!(manager.projected_exclusive_operation(), None);
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
async fn supervisor_ignores_stale_enable_recovery_preparation_completion() {
    let _guard = async_test_lock().lock().await;
    let manager = test_supervisor_manager();
    let mut supervisor = CloudBackupSupervisor::new(
        Arc::downgrade(&manager),
        spawn_actor(CloudBackupWriteSupervisor::new(Weak::new())),
    );

    let current = supervisor
        .begin_exclusive_operation(&manager, CloudBackupExclusiveOperation::Enable)
        .unwrap();
    let stale =
        CloudBackupExclusiveOperationClaim::new(CloudBackupExclusiveOperation::Enable, u64::MAX);

    supervisor
        .complete_enable_recovery_preparation(
            stale,
            Err(CloudBackupError::Internal("stale completion".into())),
        )
        .await
        .unwrap();

    assert_eq!(supervisor.active_operation, Some(current));
    assert_eq!(manager.projected_exclusive_operation(), Some(current));

    supervisor
        .complete_enable_recovery_preparation(
            current,
            Err(CloudBackupError::Internal("current completion".into())),
        )
        .await
        .unwrap();

    assert_eq!(supervisor.active_operation, None);
    assert_eq!(manager.projected_exclusive_operation(), None);
}

#[tokio::test(flavor = "current_thread")]
async fn supervisor_ignores_stale_no_discovery_enable_preparation_completion() {
    let _guard = async_test_lock().lock().await;
    let manager = test_supervisor_manager();
    let mut supervisor = CloudBackupSupervisor::new(
        Arc::downgrade(&manager),
        spawn_actor(CloudBackupWriteSupervisor::new(Weak::new())),
    );

    let current = supervisor
        .begin_exclusive_operation(&manager, CloudBackupExclusiveOperation::EnableNoDiscovery)
        .unwrap();
    let stale = CloudBackupExclusiveOperationClaim::new(
        CloudBackupExclusiveOperation::EnableNoDiscovery,
        u64::MAX,
    );

    supervisor
        .complete_no_discovery_enable_preparation(
            stale,
            Ok(CloudBackupNoDiscoveryEnablePreparation::ExistingBackupFound {
                context: CloudBackupEnableContext::settings_manual(),
                passkey_hint: None,
            }),
        )
        .await
        .unwrap();

    assert_eq!(supervisor.active_operation, Some(current));
    assert_eq!(manager.projected_exclusive_operation(), Some(current));

    supervisor
        .complete_no_discovery_enable_preparation(
            current,
            Ok(CloudBackupNoDiscoveryEnablePreparation::ExistingBackupFound {
                context: CloudBackupEnableContext::settings_manual(),
                passkey_hint: None,
            }),
        )
        .await
        .unwrap();

    assert_eq!(supervisor.active_operation, None);
    assert_eq!(manager.projected_exclusive_operation(), None);
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
        .take_ready_enable_upload(PendingEnableUploadSelection::RetryOnly)
        .unwrap()
        .unwrap();

    assert_eq!(ready.master_key.namespace_id(), expected_namespace);
    assert_eq!(ready.passkey.credential_id, expected_credential_id);
    assert!(supervisor.pending_enable_session.is_none());
}

#[tokio::test(flavor = "current_thread")]
async fn supervisor_preserves_force_new_confirmation_for_plain_enable_retry() {
    let _guard = async_test_lock().lock().await;
    let manager = test_supervisor_manager();
    let mut supervisor = CloudBackupSupervisor::new(
        Arc::downgrade(&manager),
        spawn_actor(CloudBackupWriteSupervisor::new(Weak::new())),
    );
    let master_key = cove_cspp::master_key::MasterKey::generate();

    supervisor.pending_enable_session =
        Some(awaiting_force_new_session(master_key, test_enable_passkey(vec![1, 2, 3])));

    let ready =
        supervisor.take_ready_enable_upload(PendingEnableUploadSelection::RetryOnly).unwrap();

    assert!(ready.is_none());
    assert!(supervisor.pending_enable_session.is_some());
}

#[tokio::test(flavor = "current_thread")]
async fn supervisor_consumes_force_new_confirmation_upload_for_force_new() {
    let _guard = async_test_lock().lock().await;
    let manager = test_supervisor_manager();
    let mut supervisor = CloudBackupSupervisor::new(
        Arc::downgrade(&manager),
        spawn_actor(CloudBackupWriteSupervisor::new(Weak::new())),
    );
    let master_key = cove_cspp::master_key::MasterKey::generate();
    let expected_namespace = master_key.namespace_id();
    let expected_credential_id = vec![1, 2, 3];

    supervisor.pending_enable_session = Some(awaiting_force_new_session(
        master_key,
        test_enable_passkey(expected_credential_id.clone()),
    ));

    let ready = supervisor
        .take_ready_enable_upload(PendingEnableUploadSelection::RetryOrForceNewConfirmation)
        .unwrap()
        .unwrap();

    assert_eq!(ready.master_key.namespace_id(), expected_namespace);
    assert_eq!(ready.passkey.credential_id, expected_credential_id);
    assert!(supervisor.pending_enable_session.is_none());
}

#[tokio::test(flavor = "current_thread")]
async fn supervisor_ignores_stale_enable_upload_completion() {
    let _guard = async_test_lock().lock().await;
    let manager = test_supervisor_manager();
    let mut supervisor = CloudBackupSupervisor::new(
        Arc::downgrade(&manager),
        spawn_actor(CloudBackupWriteSupervisor::new(Weak::new())),
    );

    let current = supervisor
        .begin_exclusive_operation(&manager, CloudBackupExclusiveOperation::Enable)
        .unwrap();
    let stale =
        CloudBackupExclusiveOperationClaim::new(CloudBackupExclusiveOperation::Enable, u64::MAX);

    supervisor
        .complete_enable_upload(stale, Err(CloudBackupError::Internal("stale completion".into())))
        .await
        .unwrap();

    assert_eq!(supervisor.active_operation, Some(current));
    assert_eq!(manager.projected_exclusive_operation(), Some(current));

    supervisor
        .complete_enable_upload(
            current,
            Err(CloudBackupError::Internal("current completion".into())),
        )
        .await
        .unwrap();

    assert_eq!(supervisor.active_operation, None);
    assert_eq!(manager.projected_exclusive_operation(), None);
}

#[tokio::test(flavor = "current_thread")]
async fn supervisor_ignores_stale_enable_upload_finalization_completion() {
    let _guard = async_test_lock().lock().await;
    let manager = test_supervisor_manager();
    let mut supervisor = CloudBackupSupervisor::new(
        Arc::downgrade(&manager),
        spawn_actor(CloudBackupWriteSupervisor::new(Weak::new())),
    );

    let current = supervisor
        .begin_exclusive_operation(&manager, CloudBackupExclusiveOperation::Enable)
        .unwrap();
    let stale =
        CloudBackupExclusiveOperationClaim::new(CloudBackupExclusiveOperation::Enable, u64::MAX);

    supervisor
        .complete_enable_upload_finalization(
            stale,
            test_enable_upload_finalization().0,
            Err(CloudBackupError::Internal("stale completion".into())),
        )
        .await
        .unwrap();

    assert_eq!(supervisor.active_operation, Some(current));
    assert_eq!(manager.projected_exclusive_operation(), Some(current));

    supervisor
        .complete_enable_upload_finalization(
            current,
            test_enable_upload_finalization().0,
            Err(CloudBackupError::Internal("current completion".into())),
        )
        .await
        .unwrap();

    assert_eq!(supervisor.active_operation, None);
    assert_eq!(manager.projected_exclusive_operation(), None);
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
async fn supervisor_ignores_stale_disable_preparation_completion() {
    let _guard = async_test_lock().lock().await;
    let manager = test_supervisor_manager();
    let mut supervisor = CloudBackupSupervisor::new(
        Arc::downgrade(&manager),
        spawn_actor(CloudBackupWriteSupervisor::new(Weak::new())),
    );

    let current = supervisor
        .begin_exclusive_operation(&manager, CloudBackupExclusiveOperation::Disable)
        .unwrap();
    let stale =
        CloudBackupExclusiveOperationClaim::new(CloudBackupExclusiveOperation::Disable, u64::MAX);

    supervisor
        .complete_disable_preparation(
            stale,
            Err(CloudBackupError::Internal("stale completion".into())),
        )
        .await
        .unwrap();

    assert_eq!(supervisor.active_operation, Some(current));
    assert_eq!(manager.projected_exclusive_operation(), Some(current));

    supervisor
        .complete_disable_preparation(
            current,
            Err(CloudBackupError::Internal("current completion".into())),
        )
        .await
        .unwrap();

    assert_eq!(supervisor.active_operation, None);
    assert_eq!(manager.projected_exclusive_operation(), None);
}

#[tokio::test(flavor = "current_thread")]
async fn supervisor_ignores_stale_disable_blocker_completion() {
    let _guard = async_test_lock().lock().await;
    let manager = test_supervisor_manager();
    let mut supervisor = CloudBackupSupervisor::new(
        Arc::downgrade(&manager),
        spawn_actor(CloudBackupWriteSupervisor::new(Weak::new())),
    );

    let current = supervisor
        .begin_exclusive_operation(&manager, CloudBackupExclusiveOperation::Disable)
        .unwrap();
    let stale =
        CloudBackupExclusiveOperationClaim::new(CloudBackupExclusiveOperation::Disable, u64::MAX);
    let disabling = test_disabling_state();

    supervisor
        .complete_disable_blocker_check(
            stale,
            disabling.clone(),
            Err(CloudBackupError::Internal("stale completion".into())),
        )
        .await
        .unwrap();

    assert_eq!(supervisor.active_operation, Some(current));
    assert_eq!(manager.projected_exclusive_operation(), Some(current));

    supervisor
        .complete_disable_blocker_check(
            current,
            disabling,
            Err(CloudBackupError::Internal("current completion".into())),
        )
        .await
        .unwrap();

    assert_eq!(supervisor.active_operation, None);
    assert_eq!(manager.projected_exclusive_operation(), None);
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
        .set(&PersistedCloudBackupState::Disabling(disabling.clone()))
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
        .set(&PersistedCloudBackupState::Disabling(disabling.clone()))
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
        .set(&PersistedCloudBackupState::Disabling(disabling.clone()))
        .unwrap();
    let blocker =
        CloudBackupWriteBlocker::Disabling { operation_id: disabling.disable_generation };
    supervisor.pending_disable_write_drain =
        Some(PendingDisableWriteDrain { claim, blocker, disabling });

    supervisor.complete_disable_write_drain(claim, blocker).await.unwrap();

    assert_eq!(supervisor.active_operation, None);
    assert_eq!(manager.projected_exclusive_operation(), None);
    let PersistedCloudBackupState::Disabling(disabling) =
        Database::global().cloud_backup_state.get().unwrap()
    else {
        panic!("expected persisted disabling state");
    };
    assert!(disabling.delete_started_at.is_none());
    assert!(disabling.last_error.is_some());
    assert!(manager.disable_can_keep_enabled());
}

#[tokio::test(flavor = "current_thread")]
async fn supervisor_ignores_stale_disable_delete_completion() {
    let _guard = async_test_lock().lock().await;
    let manager = test_supervisor_manager();
    let mut supervisor = CloudBackupSupervisor::new(
        Arc::downgrade(&manager),
        spawn_actor(CloudBackupWriteSupervisor::new(Weak::new())),
    );

    let current = supervisor
        .begin_exclusive_operation(&manager, CloudBackupExclusiveOperation::Disable)
        .unwrap();
    let stale =
        CloudBackupExclusiveOperationClaim::new(CloudBackupExclusiveOperation::Disable, u64::MAX);
    let disabling = test_disabling_state();

    supervisor
        .complete_disable_namespace_delete(
            stale,
            disabling.clone(),
            Err(CloudBackupError::Internal("stale completion".into())),
        )
        .await
        .unwrap();

    assert_eq!(supervisor.active_operation, Some(current));
    assert_eq!(manager.projected_exclusive_operation(), Some(current));

    supervisor
        .complete_disable_namespace_delete(
            current,
            disabling,
            Err(CloudBackupError::Internal("current completion".into())),
        )
        .await
        .unwrap();

    assert_eq!(supervisor.active_operation, None);
    assert_eq!(manager.projected_exclusive_operation(), None);
}

#[tokio::test(flavor = "current_thread")]
async fn supervisor_ignores_stale_disable_local_cleanup_completion() {
    let _guard = async_test_lock().lock().await;
    let manager = test_supervisor_manager();
    let mut supervisor = CloudBackupSupervisor::new(
        Arc::downgrade(&manager),
        spawn_actor(CloudBackupWriteSupervisor::new(Weak::new())),
    );

    let current = supervisor
        .begin_exclusive_operation(&manager, CloudBackupExclusiveOperation::Disable)
        .unwrap();
    let stale =
        CloudBackupExclusiveOperationClaim::new(CloudBackupExclusiveOperation::Disable, u64::MAX);
    let disabling = test_disabling_state();

    supervisor
        .complete_disable_local_cleanup(
            stale,
            disabling.clone(),
            Err(CloudBackupError::Internal("stale completion".into())),
        )
        .await
        .unwrap();

    assert_eq!(supervisor.active_operation, Some(current));
    assert_eq!(manager.projected_exclusive_operation(), Some(current));

    supervisor
        .complete_disable_local_cleanup(
            current,
            disabling,
            Err(CloudBackupError::Internal("current completion".into())),
        )
        .await
        .unwrap();

    assert_eq!(supervisor.active_operation, None);
    assert_eq!(manager.projected_exclusive_operation(), None);
}

#[tokio::test(flavor = "current_thread")]
async fn supervisor_ignores_stale_recover_other_backups_completion() {
    let _guard = async_test_lock().lock().await;
    let manager = test_supervisor_manager();
    let mut supervisor = CloudBackupSupervisor::new(
        Arc::downgrade(&manager),
        spawn_actor(CloudBackupWriteSupervisor::new(Weak::new())),
    );

    let current = CloudBackupSupervisor::begin_other_backups_operation(
        &mut supervisor,
        &manager,
        CloudBackupExclusiveOperation::RecoverOtherBackups,
        CloudBackupOtherBackupsOutcome::Recovering,
    )
    .unwrap();
    let stale = CloudBackupExclusiveOperationClaim::new(
        CloudBackupExclusiveOperation::RecoverOtherBackups,
        u64::MAX,
    );
    supervisor
        .complete_recover_other_backups(
            stale,
            Err(CloudBackupError::Internal("stale completion".into())),
        )
        .await
        .unwrap();

    assert_eq!(supervisor.active_operation, Some(current));
    assert_eq!(manager.projected_exclusive_operation(), Some(current));

    supervisor
        .complete_recover_other_backups(
            current,
            Err(CloudBackupError::Internal("current completion".into())),
        )
        .await
        .unwrap();

    assert_eq!(supervisor.active_operation, None);
    assert_eq!(manager.projected_exclusive_operation(), None);
}

#[tokio::test(flavor = "current_thread")]
async fn supervisor_ignores_stale_delete_cloud_wallet_preparation_completion() {
    let _guard = async_test_lock().lock().await;
    let manager = test_supervisor_manager();
    let mut supervisor = CloudBackupSupervisor::new(
        Arc::downgrade(&manager),
        spawn_actor(CloudBackupWriteSupervisor::new(Weak::new())),
    );

    let current = supervisor
        .begin_exclusive_operation(&manager, CloudBackupExclusiveOperation::DeleteCloudWallet)
        .unwrap();
    let stale = CloudBackupExclusiveOperationClaim::new(
        CloudBackupExclusiveOperation::DeleteCloudWallet,
        u64::MAX,
    );

    supervisor
        .complete_delete_cloud_wallet_preparation(
            stale,
            Err(CloudBackupError::Internal("stale completion".into())),
        )
        .await
        .unwrap();

    assert_eq!(supervisor.active_operation, Some(current));
    assert_eq!(manager.projected_exclusive_operation(), Some(current));

    supervisor
        .complete_delete_cloud_wallet_preparation(
            current,
            Err(CloudBackupError::Internal("current completion".into())),
        )
        .await
        .unwrap();

    assert_eq!(supervisor.active_operation, None);
    assert_eq!(manager.projected_exclusive_operation(), None);
}

#[tokio::test(flavor = "current_thread")]
async fn supervisor_ignores_stale_delete_other_backups_completion() {
    let _guard = async_test_lock().lock().await;
    let manager = test_supervisor_manager();
    let mut supervisor = CloudBackupSupervisor::new(
        Arc::downgrade(&manager),
        spawn_actor(CloudBackupWriteSupervisor::new(Weak::new())),
    );

    let current = CloudBackupSupervisor::begin_other_backups_operation(
        &mut supervisor,
        &manager,
        CloudBackupExclusiveOperation::DeleteOtherBackups,
        CloudBackupOtherBackupsOutcome::Deleting,
    )
    .unwrap();
    let stale = CloudBackupExclusiveOperationClaim::new(
        CloudBackupExclusiveOperation::DeleteOtherBackups,
        u64::MAX,
    );

    supervisor
        .complete_delete_other_backups(
            stale,
            Err(CloudBackupError::Internal("stale completion".into())),
        )
        .await
        .unwrap();

    assert_eq!(supervisor.active_operation, Some(current));
    assert_eq!(manager.projected_exclusive_operation(), Some(current));

    supervisor
        .complete_delete_other_backups(
            current,
            Err(CloudBackupError::Internal("current completion".into())),
        )
        .await
        .unwrap();

    assert_eq!(supervisor.active_operation, None);
    assert_eq!(manager.projected_exclusive_operation(), None);
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
    supervisor.active_cloud_only_fetch_request = Some(7);

    supervisor.complete_cloud_only_fetch_request(7, Ok(Vec::new())).await.unwrap();

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
    supervisor.active_operation.start_restore_all(RestoreAllRun {
        claim,
        cancellation: Arc::new(AtomicBool::new(false)),
    });
    manager.apply_restore_all_started(claim, 2);
    supervisor.active_cloud_only_fetch_request = Some(10);

    supervisor.complete_cloud_only_fetch_request(10, Ok(Vec::new())).await.unwrap();

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
        .complete_cloud_only_fetch_request(8, Ok(vec![test_cloud_only_wallet("remaining")]))
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
async fn restore_all_queue_completion_without_remaining_wallets_clears_marker_and_claim() {
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
    supervisor.active_operation.start_restore_all(RestoreAllRun {
        claim,
        cancellation: Arc::new(AtomicBool::new(false)),
    });

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
    supervisor.active_operation.start_restore_all(RestoreAllRun {
        claim,
        cancellation: cancellation.clone(),
    });

    supervisor.request_restore_all_cancellation();

    assert!(cancellation.load(Ordering::Acquire));
    assert_eq!(supervisor.active_operation, Some(claim));
    assert_eq!(manager.projected_exclusive_operation(), Some(claim));
}

#[tokio::test(flavor = "current_thread")]
async fn closing_detail_does_not_cancel_restore_all() {
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
    supervisor.active_operation.start_restore_all(RestoreAllRun {
        claim,
        cancellation: cancellation.clone(),
    });

    supervisor.close_detail().await.unwrap();

    assert!(!cancellation.load(Ordering::Acquire));
    assert_eq!(supervisor.active_operation, Some(claim));
    assert_eq!(manager.projected_exclusive_operation(), Some(claim));
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
    let failed_record_id = items[1].record_id.as_str();
    let CloudOnlyState::Loaded { wallets } = manager.state.read().cloud_only() else {
        panic!("expected retained cloud-only failure row");
    };
    let retained_failure = wallets
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
    supervisor.active_operation.start_restore_all(RestoreAllRun {
        claim,
        cancellation: cancellation.clone(),
    });

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
    supervisor.active_operation.start_restore_all(RestoreAllRun {
        claim,
        cancellation: Arc::new(AtomicBool::new(false)),
    });
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
    manager.apply_cloud_only_fetch_outcome(CloudBackupCloudOnlyFetchOutcome::Loaded(vec![
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
    supervisor.active_operation.start_restore_all(RestoreAllRun {
        claim,
        cancellation: Arc::new(AtomicBool::new(false)),
    });

    let should_continue = supervisor
        .complete_restore_all_record(
            claim,
            1,
            wallet,
            Err(CloudBackupError::Internal("invalid wallet".into())),
        )
        .await
        .unwrap()
        .await
        .unwrap();

    assert!(should_continue);
    assert_eq!(supervisor.active_operation, Some(claim));
}
