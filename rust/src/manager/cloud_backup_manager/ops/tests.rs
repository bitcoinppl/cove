use std::sync::{Arc, Once};
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
    PersistedCloudBlobSyncState, PersistedDisableDisposition, PersistedDisablingCloudBackup,
    PersistedDriveAccountSwitchPhase,
};
use crate::label_manager::LabelManager;
use crate::manager::cloud_backup_manager::CloudBackupDetailState;
use crate::manager::cloud_backup_manager::actors::{
    CloudBackupWriteClient,
    cleanup::{CleanupExpectedWalletRecord, CleanupSourceNamespace, CloudBackupCleanupJob},
};
use crate::manager::cloud_backup_manager::model::{
    CloudBackupDestructiveOperationState, CloudBackupExclusiveOperation,
    CloudBackupExclusiveOperationClaim, CloudBackupVerificationState,
};
use crate::manager::cloud_backup_manager::reconcile::{
    CloudBackupManagerReconciler, CloudBackupReconcileMessage,
};
use crate::manager::cloud_backup_manager::screen::{
    CloudBackupDisableReview, CloudBackupDisableScreen, CloudBackupFinishingDisableStage,
    CloudBackupScreenBody, CloudBackupTurnOffFailure,
};
use crate::manager::cloud_backup_manager::verify::CloudBackupDeepVerificationStep;
use crate::manager::cloud_backup_manager::wallets::{
    CloudOnlyRestoreOutcome, NamespaceMatch, WalletRestoreOutcome, WalletRestoreSession,
};
use crate::manager::cloud_backup_manager::wallets::{
    NamespaceMatchOutcome, NamespaceMatchSnapshotOutcome, NamespacePasskeyMatcher,
    PasskeyMaterialAcquirer, StagedPrfKey,
};
use crate::manager::cloud_backup_manager::{
    CLOUD_BACKUP_MANAGER, CORRUPTED_CLOUD_BACKUP_STATE_MESSAGE,
    CloudBackupDetailInventorySnapshotResult, CloudBackupDetailOutcome,
    CloudBackupDetailProviderConfirmation, CloudBackupDetailResult,
    CloudBackupDetailSnapshotCompletion, CloudBackupEnableContext, CloudBackupEnablePromptChoice,
    CloudBackupEnableState, CloudBackupInventoryAuthority, CloudBackupInventoryIncompleteReason,
    CloudBackupKeychain, CloudBackupLifecycle, CloudBackupManagerAction,
    CloudBackupOtherBackupsState, CloudBackupPasskeyChoiceIntent, CloudBackupRestoreAllState,
    CloudBackupRestoreEvent, CloudBackupRootPrompt, CloudBackupVerificationPresentation,
    CloudBackupVerificationReason, CloudBackupVerificationSource, CloudBackupWalletStatus,
    DeepVerificationFailure, DeepVerificationReport, DeepVerificationResult,
    GENERIC_CLOUD_BACKUP_ERROR_MESSAGE, PendingEnableNamespaceOwnership,
    PendingEnablePasskeyMetadata, PendingEnableSession, PendingUploadVerificationState,
    PendingVerificationCompletion, PendingVerificationUpload, RecoveryAction,
    SavedPasskeyConfirmationMode, VerificationState,
};
use crate::manager::cloud_backup_manager::{CloudBackupStatus, UnpersistedPrfKey};
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
use crate::test_support::LoadedHostDeadline;
use crate::wallet::{
    Wallet,
    metadata::{WalletMetadata, WalletMode, WalletType},
};
use bip39::Mnemonic;

fn current_disable_generation() -> Option<u64> {
    RustCloudBackupManager::load_persisted_state()
        .disabling()
        .map(|disabling| disabling.disable_generation)
}

fn persisted_disabling() -> PersistedDisablingCloudBackup {
    RustCloudBackupManager::load_persisted_state()
        .disabling()
        .cloned()
        .expect("expected persisted disabling state")
}

async fn deep_verify_for_test(
    manager: &RustCloudBackupManager,
    force_discoverable: bool,
) -> DeepVerificationResult {
    let run = call!(manager.supervisor.begin_verification_run_for_test(force_discoverable))
        .await
        .unwrap();
    let step = manager
        .prepare_deep_verify_cloud_backup(
            operation_writes(manager, run.claim),
            run.basis.config(),
            force_discoverable,
        )
        .await;
    call!(manager.supervisor.complete_verification(run, step)).await.unwrap();
    wait_for_test_condition(Duration::from_secs(8), "deep verification completes", || {
        let snapshot = manager.model_snapshot();
        let awaits_upload_confirmation = manager.pending_verification_completion().is_some()
            && snapshot.detail.is_some()
            && manager.projected_exclusive_operation().is_none();

        awaits_upload_confirmation
            || matches!(
                snapshot.verification,
                VerificationState::Verified(_)
                    | VerificationState::NeedsAttention(_)
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
        VerificationState::NeedsAttention(report) => DeepVerificationResult::NeedsAttention(report),
        VerificationState::PasskeyConfirmed => {
            DeepVerificationResult::PasskeyConfirmed(manager.model_snapshot().detail)
        }
        VerificationState::Failed(failure) => DeepVerificationResult::Failed(failure),
        VerificationState::Cancelled => {
            DeepVerificationResult::NotCompleted(manager.model_snapshot().detail)
        }
        VerificationState::Idle | VerificationState::Verifying => {
            panic!("expected supervisor-owned deep verification result")
        }
    }
}

async fn hold_wallet_upload_confirmation(globals: &TestGlobals, metadata: &WalletMetadata) {
    let namespace = CloudBackupKeychain::global().namespace_id().unwrap().unwrap();
    let record_id = wallet_record_id(metadata.id.as_ref());
    let master_key = cove_cspp::Cspp::new(Keychain::global().clone())
        .load_master_key_from_store()
        .unwrap()
        .unwrap();
    let stale_backup =
        encrypted_wallet_backup_bytes(metadata, &master_key, "stale-revision", 1).await;

    globals.cloud.set_wallet_backup_download_override(namespace, record_id, stale_backup);
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

    let deadline = LoadedHostDeadline::after(Duration::from_secs(2));
    while !deadline.has_passed() {
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

fn platform_authorization_failed() -> PasskeyError {
    PasskeyError::RequestFailed {
        operation: PasskeyOperation::DiscoverAssertion,
        reason: PasskeyFailureReason::PlatformAuthorizationFailed,
    }
}

async fn wait_for_discover_count(globals: &TestGlobals, expected_count: usize) {
    wait_for_test_condition(Duration::from_millis(200), "expected passkey discovery count", || {
        globals.passkey.discover_count() == expected_count
    })
    .await;
}

/// The Turn Off flow as the published screen shows it
fn turn_off_screen(manager: &RustCloudBackupManager) -> CloudBackupDisableScreen {
    match manager.published_screen().body {
        CloudBackupScreenBody::Configured(configured) => configured.disable,
        _ => CloudBackupDisableScreen::Idle,
    }
}

/// Dispatches like the platform and waits until the screen it publishes is out
///
/// The handler's publish request queues behind the first checkpoint, so a second one waits it out
async fn dispatch_and_publish(manager: &RustCloudBackupManager, action: CloudBackupManagerAction) {
    dispatch_and_wait(manager, action).await;
    supervisor_checkpoint(manager).await;
}

/// Taps Turn Off and returns the review the dialogs show
async fn review_turn_off(manager: &RustCloudBackupManager) -> CloudBackupDisableReview {
    dispatch_and_publish(manager, CloudBackupManagerAction::DisableCloudBackup).await;
    wait_for_test_condition(Duration::from_secs(2), "the Turn Off review is ready", || {
        !matches!(turn_off_screen(manager), CloudBackupDisableScreen::Preparing)
    })
    .await;

    match turn_off_screen(manager) {
        CloudBackupDisableScreen::Reviewing(review) => review,
        screen => panic!("expected a Turn Off review, got {screen:?}"),
    }
}

/// Confirms the review on screen and returns once the confirmed Turn Off has started
async fn confirm_turn_off(manager: &RustCloudBackupManager) {
    let CloudBackupDisableScreen::Reviewing(review) = turn_off_screen(manager) else {
        panic!("expected a Turn Off review to confirm");
    };

    let review_id = review.review_id;
    dispatch_and_publish(manager, CloudBackupManagerAction::ConfirmDisable { review_id }).await;
}

/// Waits for the running Turn Off, before or after its commit, to end and publish
async fn wait_for_turn_off_run(manager: &RustCloudBackupManager) {
    wait_for_test_condition(Duration::from_secs(5), "the Turn Off run ends", || {
        !matches!(
            manager
                .projected_exclusive_operation()
                .map(CloudBackupExclusiveOperationClaim::operation),
            Some(CloudBackupExclusiveOperation::Disable)
        )
    })
    .await;
    supervisor_checkpoint(manager).await;
}

/// Turns off with consent to whatever the review shows, and waits for the run to end
async fn run_turn_off(manager: &RustCloudBackupManager) {
    review_turn_off(manager).await;
    confirm_turn_off(manager).await;
    wait_for_turn_off_run(manager).await;
}

/// Taps Try Again on a committed Turn Off and waits for the run to end
async fn try_again_turn_off(manager: &RustCloudBackupManager) {
    dispatch_and_publish(manager, CloudBackupManagerAction::ResumeDisable).await;
    wait_for_turn_off_run(manager).await;
}

/// Why the committed Turn Off on screen failed
fn turn_off_failure(manager: &RustCloudBackupManager) -> CloudBackupTurnOffFailure {
    let body = manager.published_screen().body;
    if let CloudBackupScreenBody::FinishingDisable(finishing) = &body
        && let CloudBackupFinishingDisableStage::Failed { reason } = finishing.stage
    {
        return reason;
    }

    panic!("expected a failed committed Turn Off, got {body:?}")
}

async fn run_recreate_manifest(manager: &Arc<RustCloudBackupManager>) {
    call!(manager.supervisor.start_recovery_operation(RecoveryAction::RecreateManifest))
        .await
        .expect("start recreate-manifest operation");
    wait_for_test_condition(Duration::from_secs(8), "recreate-manifest operation finishes", || {
        !matches!(
            manager
                .projected_exclusive_operation()
                .map(CloudBackupExclusiveOperationClaim::operation),
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
            manager
                .projected_exclusive_operation()
                .map(CloudBackupExclusiveOperationClaim::operation),
            Some(CloudBackupExclusiveOperation::RepairPasskey)
        )
    })
    .await;
}

async fn confirm_saved_passkey_session(manager: &Arc<RustCloudBackupManager>) {
    call!(manager.supervisor.confirm_saved_passkey()).await.expect("confirm saved passkey");
    let deadline = LoadedHostDeadline::after(Duration::from_secs(5));
    while !deadline.has_passed() {
        if !matches!(
            manager
                .projected_exclusive_operation()
                .map(CloudBackupExclusiveOperationClaim::operation),
            Some(CloudBackupExclusiveOperation::Enable)
        ) {
            return;
        }

        tokio::time::sleep(Duration::from_millis(10)).await;
        tokio::task::yield_now().await;
    }

    panic!("saved passkey confirmation finishes");
}

fn init_manager() -> Arc<RustCloudBackupManager> {
    ensure_cloud_backup_test_tokio_runtime();
    crate::manager::cloud_backup_manager::initialize_cloud_backup_manager()
}

/// Dispatch runs on the supervisor; its mailbox barrier proves the intent was handled
async fn dispatch_and_wait(manager: &RustCloudBackupManager, action: CloudBackupManagerAction) {
    manager.dispatch(action);
    supervisor_checkpoint(manager).await;
}

/// The discard's remote delete runs off the supervisor mailbox and reports back, so the test
/// waits for the journal it clears and then for the completion message
async fn discard_pending_enable_and_wait_for_remote_delete(manager: &RustCloudBackupManager) {
    manager.dispatch(CloudBackupManagerAction::DiscardPendingEnableCloudBackup);

    wait_for_test_condition(
        Duration::from_secs(1),
        "pending enable discard should finish its remote delete",
        || CloudBackupKeychain::global().load_pending_enable_journal().unwrap().is_none(),
    )
    .await;
    supervisor_checkpoint(manager).await;
}

/// The app-wide manager, listened to as the app's platform listens to it
///
/// It outlives every test, and once a test resets it every later test's seeded record rebases it,
/// so it sends far more than a manager one test builds. No test reads its messages, but its queue
/// is bounded and its supervisor blocks on a full one, so they are consumed as a platform would
fn global_manager() -> Arc<RustCloudBackupManager> {
    static LISTENING: Once = Once::new();

    ensure_cloud_backup_test_tokio_runtime();
    let manager = CLOUD_BACKUP_MANAGER.clone();
    LISTENING.call_once(|| manager.listen_for_updates(Box::new(AbsentPlatform)));
    manager
}

/// The platform side of the app-wide manager in tests, which has no screen to apply messages to
#[derive(Debug)]
struct AbsentPlatform;

impl CloudBackupManagerReconciler for AbsentPlatform {
    fn reconcile(&self, _message: CloudBackupReconcileMessage) {}
}

fn persist_pending_master_key_confirmation(namespace_id: String, revision_hash: impl Into<String>) {
    Database::global()
        .cloud_blob_sync_states
        .set(&PersistedCloudBlobSyncState::master_key_wrapper(
            namespace_id,
            PersistedCloudBlobState::UploadedPendingConfirmation(
                CloudBlobUploadedPendingConfirmationState {
                    revision_hash: revision_hash.into(),
                    uploaded_at: cove_util::time::unix_timestamp_secs_or_zero(),
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
mod golden_v1_3_0_restore;
mod master_key;
mod other_backups;
mod passkey;
mod pending_upload;
mod restore;
mod restored_history;
mod round_trip;
mod sync_health;
mod ui_fixture;
mod undecryptable_wallets;
mod upload;
