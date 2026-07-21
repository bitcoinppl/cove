use std::sync::Arc;
use std::time::Duration;

use act_zero::call;
use cove_cspp::CsppStore;
use cove_cspp::backup_data::{
    WalletEntry, WalletMode as CloudWalletMode, WalletSecret, wallet_filename_from_record_id,
    wallet_record_id,
};
use cove_device::cloud_storage::{
    CloudAccessPolicy, CloudStorage, CloudStorageError, CloudSyncHealth,
};
use cove_device::keychain::Keychain;
use cove_device::passkey::{
    DiscoveredPasskeyResult, PasskeyAccess, PasskeyError, PasskeyFailureReason, PasskeyOperation,
};

use super::test_support::*;
use super::*;
use crate::database::Database;
use crate::database::cloud_backup::{
    CloudBackupRecordKey, CloudBlobDirtyState, CloudBlobFailedState,
    CloudBlobUploadedPendingConfirmationState, CloudBlobUploadingState, CloudStorageIssue,
    PersistedCloudBackupState, PersistedCloudBackupStatus, PersistedCloudBlobState,
    PersistedCloudBlobSyncState, PersistedDisablingCloudBackup, PersistedDriveAccountSwitch,
    PersistedDriveAccountSwitchPhase,
};
use crate::label_manager::LabelManager;
use crate::manager::cloud_backup_manager::actors::{
    CloudBackupWriteClient,
    cleanup::{CleanupExpectedWalletRecord, CleanupSourceNamespace, CloudBackupCleanupJob},
    supervisor::{DeepVerificationContinuation, VerificationAttempt},
};
use crate::manager::cloud_backup_manager::model::CloudBackupDetailState;
use crate::manager::cloud_backup_manager::model::{
    CloudBackupDestructiveOperationState, CloudBackupExclusiveOperation,
    CloudBackupExclusiveOperationClaim,
};
use crate::manager::cloud_backup_manager::verify::CloudBackupDeepVerificationStep;
use crate::manager::cloud_backup_manager::wallets::{
    NamespaceMatch, WalletRestoreOutcome, WalletRestoreSession,
};
use crate::manager::cloud_backup_manager::wallets::{
    NamespaceMatchOutcome, NamespaceMatchSnapshotOutcome, NamespacePasskeyMatcher,
    PasskeyMaterialAcquirer, StagedPrfKey,
};
use crate::manager::cloud_backup_manager::{
    CLOUD_BACKUP_MANAGER, CORRUPTED_CLOUD_BACKUP_STATE_MESSAGE,
    CloudBackupDetailInventorySnapshotResult, CloudBackupDetailOutcome, CloudBackupDetailResult,
    CloudBackupDisableOutcome, CloudBackupEnableContext, CloudBackupEnablePromptChoice,
    CloudBackupEnableState, CloudBackupKeychain, CloudBackupLifecycle, CloudBackupManagerAction,
    CloudBackupOtherBackupsState, CloudBackupPasskeyChoiceIntent, CloudBackupRestoreEvent,
    CloudBackupRootPrompt, CloudBackupVerificationPresentation, CloudBackupVerificationReason,
    CloudBackupVerificationSource, CloudBackupWalletStatus, DeepVerificationFailure,
    DeepVerificationReport, DeepVerificationResult, GENERIC_CLOUD_BACKUP_ERROR_MESSAGE,
    PendingEnableNamespaceOwnership, PendingEnablePasskeyMetadata, PendingEnableSession,
    PendingUploadVerificationState, PendingVerificationCompletion, PendingVerificationUpload,
    RecoveryAction, SavedPasskeyConfirmationMode, VerificationState,
};
use crate::manager::cloud_backup_manager::{
    CloudBackupStatus, PendingEnableSessionMaterial, UnpersistedPrfKey,
};
use crate::manager::cloud_backup_manager::{
    SYNC_HEALTH_MISSING_MASTER_KEY_MESSAGE,
    cspp_exports::cspp_master_key_record_id,
    keychain::{
        CSPP_CREDENTIAL_ID_KEY, CSPP_NAMESPACE_ID_KEY, CSPP_PENDING_ENABLE_JOURNAL_KEY,
        CSPP_PRF_SALT_KEY, CloudBackupKeychainError,
    },
    master_key_wrapper_revision_hash,
};
use crate::manager::connectivity_manager::{CONNECTIVITY_MANAGER, ConnectivityStatus};
use crate::manager::wallet_manager::RustWalletManager;
use crate::wallet::{
    Wallet,
    metadata::{WalletMetadata, WalletMode, WalletType},
};
use bip39::Mnemonic;

fn pending_enable_awaiting_confirmation(
    master_key: cove_cspp::master_key::MasterKey,
    passkey: UnpersistedPrfKey,
    context: CloudBackupEnableContext,
) -> PendingEnableSession {
    PendingEnableSession::AwaitingForceNewConfirmation(PendingEnableSessionMaterial::new(
        master_key, passkey, context,
    ))
}

fn current_disable_generation() -> Option<u64> {
    match RustCloudBackupManager::load_persisted_state() {
        PersistedCloudBackupState::Disabling(disabling) => Some(disabling.disable_generation),
        PersistedCloudBackupState::Configured(_)
        | PersistedCloudBackupState::Disabled
        | PersistedCloudBackupState::Corrupted { .. } => None,
    }
}

async fn deep_verify_for_test(
    manager: &RustCloudBackupManager,
    force_discoverable: bool,
) -> DeepVerificationResult {
    let step = manager.prepare_deep_verify_cloud_backup(force_discoverable).await;
    call!(manager.supervisor.complete_verification(
        None,
        step,
        DeepVerificationContinuation::Manual {
            force_discoverable,
            attempt: VerificationAttempt::Initial,
        }
    ))
    .await
    .unwrap();
    wait_for_test_condition(Duration::from_secs(8), "deep verification completes", || {
        let snapshot = manager.model_snapshot();
        let awaits_upload_confirmation = manager.pending_verification_completion().is_some()
            && snapshot.detail.is_some()
            && manager.projected_exclusive_operation().is_none();

        awaits_upload_confirmation
            || matches!(
                snapshot.verification,
                VerificationState::Verified(_)
                    | VerificationState::PasskeyConfirmed
                    | VerificationState::Failed(_)
                    | VerificationState::Cancelled
            )
    })
    .await;

    if let Some(completion) = manager.pending_verification_completion() {
        let mut report = completion.report();
        if report.detail.is_none() {
            report.detail = manager.model_snapshot().detail;
        }

        return DeepVerificationResult::AwaitingUploadConfirmation(report);
    }

    match manager.model_snapshot().verification {
        VerificationState::Verified(report) => DeepVerificationResult::Verified(report),
        VerificationState::PasskeyConfirmed => {
            DeepVerificationResult::PasskeyConfirmed(manager.model_snapshot().detail)
        }
        VerificationState::Failed(failure) => DeepVerificationResult::Failed(failure),
        VerificationState::Cancelled => {
            DeepVerificationResult::UserCancelled(manager.model_snapshot().detail)
        }
        VerificationState::Idle | VerificationState::Verifying => {
            panic!("expected supervisor-owned deep verification result")
        }
    }
}

async fn enable_cloud_backup_create_new(
    manager: &RustCloudBackupManager,
) -> Result<(), CloudBackupError> {
    run_enable_operation(
        manager,
        TestEnableOperation::Enable(CloudBackupEnableContext::settings_manual()),
    )
    .await
}

async fn enable_cloud_backup_force_new(
    manager: &RustCloudBackupManager,
) -> Result<(), CloudBackupError> {
    enable_cloud_backup_force_new_with_context(manager, CloudBackupEnableContext::settings_manual())
        .await
}

async fn enable_cloud_backup_no_discovery(
    manager: &RustCloudBackupManager,
) -> Result<(), CloudBackupError> {
    enable_cloud_backup_no_discovery_with_context(
        manager,
        CloudBackupEnableContext::settings_manual(),
    )
    .await
}

async fn enable_cloud_backup_force_new_with_context(
    manager: &RustCloudBackupManager,
    context: CloudBackupEnableContext,
) -> Result<(), CloudBackupError> {
    run_enable_operation(manager, TestEnableOperation::ForceNew(context)).await
}

async fn enable_cloud_backup_no_discovery_with_context(
    manager: &RustCloudBackupManager,
    context: CloudBackupEnableContext,
) -> Result<(), CloudBackupError> {
    run_enable_operation(manager, TestEnableOperation::NoDiscovery(context)).await
}

enum TestEnableOperation {
    Enable(CloudBackupEnableContext),
    ForceNew(CloudBackupEnableContext),
    NoDiscovery(CloudBackupEnableContext),
    ReinitializeBackup,
}

async fn run_enable_operation(
    manager: &RustCloudBackupManager,
    operation: TestEnableOperation,
) -> Result<(), CloudBackupError> {
    let saved_passkey_confirmation = match &operation {
        TestEnableOperation::Enable(context)
        | TestEnableOperation::ForceNew(context)
        | TestEnableOperation::NoDiscovery(context) => context.saved_passkey_confirmation,
        TestEnableOperation::ReinitializeBackup => SavedPasskeyConfirmationMode::Manual,
    };

    match operation {
        TestEnableOperation::Enable(context) => {
            call!(manager.supervisor.start_enable_operation(context)).await
        }
        TestEnableOperation::ForceNew(context) => {
            call!(manager.supervisor.start_enable_force_new_operation(context)).await
        }
        TestEnableOperation::NoDiscovery(context) => {
            call!(manager.supervisor.start_enable_no_discovery_operation(context)).await
        }
        TestEnableOperation::ReinitializeBackup => {
            call!(manager.supervisor.start_recovery_operation(RecoveryAction::ReinitializeBackup))
                .await
        }
    }
    .expect("start enable operation");

    for _ in 0..200 {
        let Some(claim) = manager.projected_exclusive_operation() else { break };

        if has_awaiting_saved_passkey_confirmation_for_test(manager).await {
            call!(
                manager
                    .supervisor
                    .complete_enable_saved_passkey_wait(claim, saved_passkey_confirmation)
            )
            .await
            .expect("complete enable saved-passkey wait");
        }

        tokio::time::sleep(Duration::from_millis(10)).await;
        tokio::task::yield_now().await;
    }

    assert!(manager.projected_exclusive_operation().is_none(), "enable operation finishes");

    match manager.current_status() {
        CloudBackupStatus::Error(message) => Err(CloudBackupError::Internal(message.into())),
        CloudBackupStatus::UnsupportedPasskeyProvider => {
            Err(CloudBackupError::UnsupportedPasskeyProvider)
        }
        _ => Ok(()),
    }
}

async fn run_reinitialize_backup_operation(
    manager: &RustCloudBackupManager,
) -> Result<(), CloudBackupError> {
    run_enable_operation(manager, TestEnableOperation::ReinitializeBackup).await
}

async fn restore_from_local_master_key_fallback<S>(
    cloud: &CloudStorageClient,
    store: &S,
    cspp: &cove_cspp::Cspp<S>,
) -> Result<(cove_cspp::master_key::MasterKey, String), CloudBackupError>
where
    S: cove_cspp::CsppStore,
    S::Error: std::fmt::Display,
{
    let (master_key, namespace_id) = try_restore_from_local_master_key(cloud, cspp)
        .await?
        .ok_or(CloudBackupError::PasskeyMismatch)?;
    store
        .save(
            crate::manager::cloud_backup_manager::keychain::CSPP_NAMESPACE_ID_KEY.into(),
            namespace_id.to_owned(),
        )
        .map_err(|source| {
            CloudBackupError::Internal(format!("save namespace_id: {source}").into())
        })?;
    Ok((master_key, namespace_id))
}

fn platform_authorization_failed() -> PasskeyError {
    PasskeyError::RequestFailed {
        operation: PasskeyOperation::DiscoverAssertion,
        reason: PasskeyFailureReason::PlatformAuthorizationFailed,
    }
}

async fn wait_for_discover_count(globals: &TestGlobals, expected_count: usize) {
    for _ in 0..20 {
        if globals.passkey.discover_count() == expected_count {
            return;
        }

        tokio::time::sleep(Duration::from_millis(10)).await;
    }

    assert_eq!(globals.passkey.discover_count(), expected_count);
}

async fn run_disable_cloud_backup(manager: &Arc<RustCloudBackupManager>) {
    call!(manager.supervisor.start_disable_operation()).await.expect("start disable operation");
    wait_for_test_condition(Duration::from_secs(2), "disable operation finishes", || {
        !matches!(
            manager.projected_exclusive_operation().map(|claim| claim.operation()),
            Some(CloudBackupExclusiveOperation::Disable)
        )
    })
    .await;
}

async fn run_keep_cloud_backup_enabled(manager: &Arc<RustCloudBackupManager>) {
    call!(manager.supervisor.keep_cloud_backup_enabled()).await.expect("keep cloud backup enabled");
    wait_for_test_condition(Duration::from_secs(2), "keep-enabled recovery finishes", || {
        current_disable_generation().is_none()
    })
    .await;
}

async fn run_recreate_manifest(manager: &Arc<RustCloudBackupManager>) {
    call!(manager.supervisor.start_recovery_operation(RecoveryAction::RecreateManifest))
        .await
        .expect("start recreate-manifest operation");
    wait_for_test_condition(Duration::from_secs(8), "recreate-manifest operation finishes", || {
        !matches!(
            manager.projected_exclusive_operation().map(|claim| claim.operation()),
            Some(CloudBackupExclusiveOperation::RecreateManifest)
        )
    })
    .await;
}

async fn run_repair_passkey_operation(manager: &Arc<RustCloudBackupManager>, no_discovery: bool) {
    call!(manager.supervisor.start_repair_passkey_operation(no_discovery))
        .await
        .expect("start repair-passkey operation");
    wait_for_test_condition(Duration::from_secs(8), "repair-passkey operation finishes", || {
        !matches!(
            manager.projected_exclusive_operation().map(|claim| claim.operation()),
            Some(CloudBackupExclusiveOperation::RepairPasskey)
        )
    })
    .await;
}

async fn confirm_saved_passkey_session(manager: &Arc<RustCloudBackupManager>) {
    call!(manager.supervisor.confirm_saved_passkey()).await.expect("confirm saved passkey");
    for _ in 0..500 {
        if !matches!(
            manager.projected_exclusive_operation().map(|claim| claim.operation()),
            Some(CloudBackupExclusiveOperation::Enable)
        ) {
            return;
        }

        tokio::time::sleep(Duration::from_millis(10)).await;
        tokio::task::yield_now().await;
    }

    panic!("saved passkey confirmation finishes");
}

fn disable_failure_message(manager: &RustCloudBackupManager) -> String {
    let CloudBackupLifecycle::Configured(configured) = manager.state().lifecycle else {
        panic!("expected configured cloud backup lifecycle");
    };
    let CloudBackupDestructiveOperationState::DisableFailed { message, .. } =
        configured.destructive_operation
    else {
        panic!("expected disable failure");
    };

    message
}

mod cove_tokio {
    pub(crate) fn init() {
        super::ensure_cloud_backup_test_tokio_runtime();
    }
}

fn init_manager() -> Arc<RustCloudBackupManager> {
    ensure_cloud_backup_test_tokio_runtime();
    RustCloudBackupManager::init()
}

fn operation_write_client_for_test(
    manager: &RustCloudBackupManager,
    claim: CloudBackupExclusiveOperationClaim,
) -> CloudBackupWriteClient {
    CloudBackupWriteClient::for_operation(manager.cloud_writes.clone(), claim)
}

fn seed_verifiable_cloud_master_key(globals: &TestGlobals) -> String {
    let prf_key = [7u8; 32];
    let prf_salt = [9u8; 32];
    let namespace = CloudBackupKeychain::global().namespace_id().unwrap();
    let master_key = cove_cspp::Cspp::new(Keychain::global().clone())
        .load_master_key_from_store()
        .unwrap()
        .unwrap();
    let encrypted =
        cove_cspp::master_key_crypto::encrypt_master_key(&master_key, &prf_key, &prf_salt).unwrap();

    let master_json = serde_json::to_vec(&encrypted).unwrap();
    let revision_hash = master_key_wrapper_revision_hash(&master_json);
    globals.cloud.set_master_key_backup(namespace, master_json);
    CloudBackupKeychain::global().save_passkey(&[1, 2, 3], prf_salt).unwrap();
    globals.passkey.set_discover_result(Ok(DiscoveredPasskeyResult {
        prf_output: prf_key.to_vec(),
        credential_id: vec![1, 2, 3],
    }));
    globals.passkey.set_authenticate_result(Ok(prf_key.to_vec()));
    revision_hash
}

fn global_manager() -> Arc<RustCloudBackupManager> {
    ensure_cloud_backup_test_tokio_runtime();
    CLOUD_BACKUP_MANAGER.clone()
}

fn persist_pending_master_key_confirmation(namespace_id: String, revision_hash: impl Into<String>) {
    Database::global()
        .cloud_blob_sync_states
        .set(&PersistedCloudBlobSyncState::master_key_wrapper(
            namespace_id,
            PersistedCloudBlobState::UploadedPendingConfirmation(
                CloudBlobUploadedPendingConfirmationState {
                    revision_hash: revision_hash.into(),
                    uploaded_at: crate::manager::cloud_backup_manager::current_timestamp(),
                    attempt_count: 0,
                    last_checked_at: None,
                },
            ),
        ))
        .unwrap();
}

mod account_switch;
mod cleanup;
mod connectivity;
mod deep_verify;
mod disable;
mod enable;
mod enable_session;
mod master_key;
mod other_backups;
mod passkey;
mod pending_upload;
mod restore;
mod sync_health;
mod upload;
