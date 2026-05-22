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
    CloudBackupRecordKey, CloudBlobDirtyState, CloudBlobFailedState, CloudBlobFailureIssue,
    CloudBlobUploadedPendingConfirmationState, CloudBlobUploadingState, PersistedCloudBackupState,
    PersistedCloudBackupStatus, PersistedCloudBlobState, PersistedCloudBlobSyncState,
    PersistedDisablingCloudBackup,
};
use crate::label_manager::LabelManager;
use crate::manager::cloud_backup_manager::actors::{
    CloudBackupOperation, CloudBackupWriteClient,
    cleanup::{CleanupExpectedWalletRecord, CleanupSourceNamespace, CloudBackupCleanupJob},
    supervisor::VerificationAttempt,
};
use crate::manager::cloud_backup_manager::model::{
    CloudBackupDestructiveOperationState, CloudBackupExclusiveOperation,
    CloudBackupExclusiveOperationClaim,
};
use crate::manager::cloud_backup_manager::wallets::{NamespaceMatch, WalletRestoreSession};
use crate::manager::cloud_backup_manager::wallets::{
    NamespaceMatchOutcome, NamespacePasskeyMatcher, PasskeyMaterialAcquirer, StagedPrfKey,
};
use crate::manager::cloud_backup_manager::{
    CLOUD_BACKUP_MANAGER, CloudBackupDetailResult, CloudBackupDisableOutcome,
    CloudBackupEnableContext, CloudBackupEnableOutcome, CloudBackupEnableState,
    CloudBackupKeychain, CloudBackupLifecycle, CloudBackupManagerAction,
    CloudBackupOtherBackupsState, CloudBackupPasskeyChoiceIntent, CloudBackupRootPrompt,
    CloudBackupVerificationOutcome, CloudBackupVerificationPresentation,
    CloudBackupVerificationReason, CloudBackupVerificationSource, CloudBackupWalletStatus,
    DeepVerificationFailure, DeepVerificationReport, DeepVerificationResult, PendingEnableSession,
    PendingUploadVerificationState, PendingVerificationCompletion, PendingVerificationUpload,
    RecoveryAction, SavedPasskeyConfirmationMode, VerificationState,
};
use crate::manager::cloud_backup_manager::{
    CloudBackupStatus, PendingEnableSessionMaterial, UnpersistedPrfKey,
};
use crate::manager::cloud_backup_manager::{
    SYNC_HEALTH_MISSING_MASTER_KEY_MESSAGE, cspp_master_key_record_id,
    keychain::{
        CSPP_CREDENTIAL_ID_KEY, CSPP_NAMESPACE_ID_KEY, CSPP_PRF_SALT_KEY, CloudBackupKeychainError,
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
        PersistedCloudBackupState::Configured(_) | PersistedCloudBackupState::Disabled => None,
    }
}

async fn deep_verify_for_test(
    manager: &RustCloudBackupManager,
    force_discoverable: bool,
) -> DeepVerificationResult {
    let step = manager.prepare_deep_verify_cloud_backup(force_discoverable).await;
    call!(manager.supervisor.complete_verification(
        step,
        force_discoverable,
        VerificationAttempt::Initial
    ))
    .await
    .unwrap();
    wait_for_test_condition(Duration::from_secs(8), "deep verification completes", || {
        let snapshot = manager.model_snapshot();
        manager.pending_verification_completion().is_some()
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
        let mut report = completion.report().clone();
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
        CloudBackupOperation::Enable(CloudBackupEnableContext::settings_manual()),
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
    run_enable_operation(manager, CloudBackupOperation::EnableForceNew(context)).await
}

async fn enable_cloud_backup_no_discovery_with_context(
    manager: &RustCloudBackupManager,
    context: CloudBackupEnableContext,
) -> Result<(), CloudBackupError> {
    run_enable_operation(manager, CloudBackupOperation::EnableNoDiscovery(context)).await
}

async fn run_enable_operation(
    manager: &RustCloudBackupManager,
    operation: CloudBackupOperation,
) -> Result<(), CloudBackupError> {
    let saved_passkey_confirmation = match &operation {
        CloudBackupOperation::Enable(context)
        | CloudBackupOperation::EnableForceNew(context)
        | CloudBackupOperation::EnableNoDiscovery(context) => context.saved_passkey_confirmation,
        _ => SavedPasskeyConfirmationMode::Manual,
    };

    call!(manager.supervisor.start_operation(operation, None))
        .await
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
        CloudBackupStatus::Error(message) => Err(CloudBackupError::Internal(message)),
        CloudBackupStatus::UnsupportedPasskeyProvider => {
            Err(CloudBackupError::UnsupportedPasskeyProvider)
        }
        _ => Ok(()),
    }
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
        .map_err_prefix("save namespace_id", CloudBackupError::Internal)?;
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
    call!(manager.supervisor.start_operation(CloudBackupOperation::Disable, None))
        .await
        .expect("start disable operation");
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
    call!(
        manager.supervisor.start_operation(
            CloudBackupOperation::Recovery(RecoveryAction::RecreateManifest),
            None
        )
    )
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
    call!(
        manager
            .supervisor
            .start_operation(CloudBackupOperation::RepairPasskey { no_discovery }, None)
    )
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
                    uploaded_at: jiff::Timestamp::now().as_second().try_into().unwrap_or(0),
                    attempt_count: 0,
                    last_checked_at: None,
                },
            ),
        ))
        .unwrap();
}

#[tokio::test(flavor = "current_thread")]
async fn passkey_match_treats_missing_credential_as_no_match() {
    let _guard = async_test_lock().lock().await;
    cove_tokio::init();
    let globals = test_globals();
    globals.reset();

    let master_key = cove_cspp::master_key::MasterKey::generate();
    let namespace = master_key.namespace_id();
    let encrypted_master =
        cove_cspp::master_key_crypto::encrypt_master_key(&master_key, &[7; 32], &[9; 32]).unwrap();
    globals
        .cloud
        .set_master_key_backup(namespace.clone(), serde_json::to_vec(&encrypted_master).unwrap());
    globals.passkey.set_discover_result(Err(PasskeyError::NoCredentialFound));

    let outcome = NamespacePasskeyMatcher::new(
        &CloudStorage::global_explicit_client(),
        PasskeyAccess::global(),
    )
    .match_namespaces(&[namespace])
    .await
    .unwrap();

    assert!(matches!(outcome, NamespaceMatchOutcome::NoMatch));
}

#[tokio::test(flavor = "current_thread")]
async fn passkey_match_treats_user_cancel_as_user_declined() {
    let _guard = async_test_lock().lock().await;
    cove_tokio::init();
    let globals = test_globals();
    globals.reset();

    let master_key = cove_cspp::master_key::MasterKey::generate();
    let namespace = master_key.namespace_id();
    let encrypted_master =
        cove_cspp::master_key_crypto::encrypt_master_key(&master_key, &[7; 32], &[9; 32]).unwrap();
    globals
        .cloud
        .set_master_key_backup(namespace.clone(), serde_json::to_vec(&encrypted_master).unwrap());
    globals.passkey.set_discover_result(Err(PasskeyError::UserCancelled));

    let outcome = NamespacePasskeyMatcher::new(
        &CloudStorage::global_explicit_client(),
        PasskeyAccess::global(),
    )
    .match_namespaces(&[namespace])
    .await
    .unwrap();

    assert!(matches!(outcome, NamespaceMatchOutcome::UserDeclined));
}

#[tokio::test(flavor = "current_thread")]
async fn passkey_match_mixed_supported_and_unsupported_versions_returns_no_match() {
    let _guard = async_test_lock().lock().await;
    cove_tokio::init();
    let globals = test_globals();
    globals.reset();

    let master_key = cove_cspp::master_key::MasterKey::generate();
    let supported_namespace = format!("{}-supported", master_key.namespace_id());
    let unsupported_namespace = format!("{}-unsupported", master_key.namespace_id());
    let encrypted_master =
        cove_cspp::master_key_crypto::encrypt_master_key(&master_key, &[7; 32], &[9; 32]).unwrap();
    let mut unsupported_master =
        cove_cspp::master_key_crypto::encrypt_master_key(&master_key, &[7; 32], &[9; 32]).unwrap();
    unsupported_master.version = 2;

    globals.cloud.set_master_key_backup(
        supported_namespace.clone(),
        serde_json::to_vec(&encrypted_master).unwrap(),
    );
    globals.cloud.set_master_key_backup(
        unsupported_namespace.clone(),
        serde_json::to_vec(&unsupported_master).unwrap(),
    );
    globals.passkey.set_discover_result(Ok(DiscoveredPasskeyResult {
        prf_output: vec![8; 32],
        credential_id: vec![1, 2, 3],
    }));

    let outcome = NamespacePasskeyMatcher::new(
        &CloudStorage::global_explicit_client(),
        PasskeyAccess::global(),
    )
    .match_namespaces(&[supported_namespace, unsupported_namespace])
    .await
    .unwrap();

    assert!(matches!(outcome, NamespaceMatchOutcome::NoMatch));
}

#[tokio::test(flavor = "current_thread")]
async fn passkey_match_discovery_propagates_unsupported_provider() {
    let _guard = async_test_lock().lock().await;
    cove_tokio::init();
    let globals = test_globals();
    globals.reset();

    let master_key = cove_cspp::master_key::MasterKey::generate();
    let namespace = master_key.namespace_id();
    let encrypted_master =
        cove_cspp::master_key_crypto::encrypt_master_key(&master_key, &[7; 32], &[9; 32]).unwrap();
    globals
        .cloud
        .set_master_key_backup(namespace.clone(), serde_json::to_vec(&encrypted_master).unwrap());
    globals.passkey.set_discover_result(Err(PasskeyError::PrfUnsupportedProvider));

    let result = NamespacePasskeyMatcher::new(
        &CloudStorage::global_explicit_client(),
        PasskeyAccess::global(),
    )
    .match_namespaces(&[namespace])
    .await;
    let error = match result {
        Ok(_) => panic!("expected unsupported passkey provider error"),
        Err(error) => error,
    };

    assert!(matches!(error, CloudBackupError::UnsupportedPasskeyProvider));
}

#[tokio::test(flavor = "current_thread")]
async fn passkey_match_targeted_auth_propagates_unsupported_provider() {
    let _guard = async_test_lock().lock().await;
    cove_tokio::init();
    let globals = test_globals();
    globals.reset();

    let master_key = cove_cspp::master_key::MasterKey::generate();
    let first_namespace = format!("{}-first", master_key.namespace_id());
    let second_namespace = format!("{}-second", master_key.namespace_id());
    let first_encrypted =
        cove_cspp::master_key_crypto::encrypt_master_key(&master_key, &[7; 32], &[9; 32]).unwrap();
    let second_encrypted =
        cove_cspp::master_key_crypto::encrypt_master_key(&master_key, &[8; 32], &[9; 32]).unwrap();
    globals.cloud.set_master_key_backup(
        first_namespace.clone(),
        serde_json::to_vec(&first_encrypted).unwrap(),
    );
    globals.cloud.set_master_key_backup(
        second_namespace.clone(),
        serde_json::to_vec(&second_encrypted).unwrap(),
    );
    globals.passkey.set_discover_result(Ok(DiscoveredPasskeyResult {
        prf_output: vec![1; 32],
        credential_id: vec![1, 2, 3],
    }));
    globals.passkey.set_authenticate_result(Err(PasskeyError::PrfUnsupportedProvider));

    let result = NamespacePasskeyMatcher::new(
        &CloudStorage::global_explicit_client(),
        PasskeyAccess::global(),
    )
    .match_namespaces(&[first_namespace, second_namespace])
    .await;
    let error = match result {
        Ok(_) => panic!("expected unsupported passkey provider error"),
        Err(error) => error,
    };

    assert!(matches!(error, CloudBackupError::UnsupportedPasskeyProvider));
}

#[tokio::test(flavor = "current_thread")]
async fn passkey_match_allows_one_credential_to_match_multiple_namespaces() {
    let _guard = async_test_lock().lock().await;
    cove_tokio::init();
    let globals = test_globals();
    globals.reset();

    let prf_key = [7u8; 32];
    let first_master_key = cove_cspp::master_key::MasterKey::generate();
    let second_master_key = cove_cspp::master_key::MasterKey::generate();
    let first_namespace = first_master_key.namespace_id();
    let second_namespace = second_master_key.namespace_id();
    let first_encrypted =
        cove_cspp::master_key_crypto::encrypt_master_key(&first_master_key, &prf_key, &[9; 32])
            .unwrap();
    let second_encrypted =
        cove_cspp::master_key_crypto::encrypt_master_key(&second_master_key, &prf_key, &[8; 32])
            .unwrap();

    globals.cloud.set_master_key_backup(
        first_namespace.clone(),
        serde_json::to_vec(&first_encrypted).unwrap(),
    );
    globals.cloud.set_master_key_backup(
        second_namespace.clone(),
        serde_json::to_vec(&second_encrypted).unwrap(),
    );
    globals.passkey.set_discover_result(Ok(DiscoveredPasskeyResult {
        prf_output: prf_key.to_vec(),
        credential_id: vec![1, 2, 3],
    }));
    globals.passkey.set_authenticate_result(Ok(prf_key.to_vec()));

    let outcome = NamespacePasskeyMatcher::new(
        &CloudStorage::global_explicit_client(),
        PasskeyAccess::global(),
    )
    .match_namespaces(&[first_namespace.clone(), second_namespace.clone()])
    .await
    .unwrap();

    let NamespaceMatchOutcome::Matched(matches) = outcome else {
        panic!("expected multiple namespace matches");
    };
    let matched_namespaces =
        matches.into_iter().map(|matched| matched.namespace_id).collect::<Vec<_>>();

    assert_eq!(matched_namespaces, vec![first_namespace, second_namespace]);
}

#[test]
fn persist_xpub_wallets_saves_each_wallet_in_its_own_scope() {
    let _guard = test_lock().lock();
    let globals = test_globals();
    globals.reset();

    let first_wallet = xpub_only_wallet_metadata();
    let mut second_wallet = xpub_only_wallet_metadata();
    second_wallet.wallet_mode = WalletMode::Decoy;

    Database::global()
        .wallets()
        .save_all_wallets(first_wallet.network, first_wallet.wallet_mode, Vec::new())
        .unwrap();
    Database::global()
        .wallets()
        .save_all_wallets(second_wallet.network, second_wallet.wallet_mode, Vec::new())
        .unwrap();

    persist_xpub_wallets(vec![first_wallet.clone(), second_wallet.clone()]);

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
            .is_some()
    );
}

#[tokio::test(flavor = "current_thread")]
async fn wrapper_repair_discovery_propagates_unsupported_provider() {
    let _guard = async_test_lock().lock().await;
    cove_tokio::init();
    let globals = test_globals();
    globals.reset();
    globals.passkey.set_discover_result(Err(PasskeyError::PrfUnsupportedProvider));

    let acquirer = PasskeyMaterialAcquirer::new(PasskeyAccess::global());
    let discovery_result = acquirer.discover_or_create_for_wrapper_repair().await;
    let error = match discovery_result {
        Ok(_) => panic!("expected unsupported passkey provider error"),
        Err(error) => error,
    };

    assert!(matches!(error, CloudBackupError::UnsupportedPasskeyProvider));
}

#[tokio::test(flavor = "current_thread")]
async fn backup_wallets_uploads_when_cloud_backup_is_enabled() {
    let _guard = async_test_lock().lock().await;
    cove_tokio::init();
    let globals = test_globals();
    let manager = init_manager();
    configure_enabled_cloud_backup(&manager, globals, 3);

    let metadata = xpub_only_wallet_metadata();
    let xpub = sample_xpub(&metadata);
    Keychain::global().save_wallet_xpub(&metadata.id, xpub.parse().unwrap()).unwrap();

    manager.do_backup_wallets(&[metadata]).await.unwrap();

    assert_eq!(globals.cloud.uploaded_wallet_backup_count(), 1);
    assert_eq!(Database::global().cloud_backup_state.get().unwrap().wallet_count(), Some(4));
    assert!(Database::global().cloud_blob_sync_states.list().unwrap().into_iter().any(
        |state| matches!(state.state, PersistedCloudBlobState::UploadedPendingConfirmation(_))
    ));
}

#[tokio::test(flavor = "current_thread")]
async fn backup_wallets_persists_partial_uploads_when_later_wallet_fails() {
    let _guard = async_test_lock().lock().await;
    cove_tokio::init();
    let globals = test_globals();
    let manager = init_manager();
    configure_enabled_cloud_backup(&manager, globals, 3);

    let first_wallet = xpub_only_wallet_metadata();
    let mut second_wallet = WalletMetadata::preview_new();
    second_wallet.wallet_type = WalletType::Hot;
    Keychain::global()
        .save_wallet_xpub(&first_wallet.id, sample_xpub(&first_wallet).parse().unwrap())
        .unwrap();

    let error =
        manager.do_backup_wallets(&[first_wallet.clone(), second_wallet]).await.unwrap_err();

    let record_id = wallet_record_id(first_wallet.id.as_ref());
    assert!(error.to_string().contains("has no mnemonic"), "{error}");
    assert_eq!(globals.cloud.uploaded_wallet_backup_count(), 1);
    assert_eq!(Database::global().cloud_backup_state.get().unwrap().wallet_count(), Some(4));
    assert!(matches!(
        Database::global().cloud_blob_sync_states.get(&record_id).unwrap(),
        Some(PersistedCloudBlobSyncState {
            state: PersistedCloudBlobState::UploadedPendingConfirmation(_)
                | PersistedCloudBlobState::Confirmed(_),
            ..
        })
    ));
}

#[tokio::test(flavor = "current_thread")]
async fn backup_wallets_defers_local_completion_when_disable_starts_after_upload() {
    let _guard = async_test_lock().lock().await;
    cove_tokio::init();
    let globals = test_globals();
    let manager = init_manager();
    configure_enabled_cloud_backup(&manager, globals, 3);

    let metadata = xpub_only_wallet_metadata();
    let xpub = sample_xpub(&metadata);
    Keychain::global().save_wallet_xpub(&metadata.id, xpub.parse().unwrap()).unwrap();

    let namespace_id = CloudBackupKeychain::global().namespace_id().unwrap();
    let PersistedCloudBackupState::Configured(previous_configured) =
        Database::global().cloud_backup_state.get().unwrap()
    else {
        panic!("expected configured cloud backup state");
    };
    globals.cloud.persist_disabling_on_next_upload(PersistedDisablingCloudBackup {
        previous_configured,
        namespace_id,
        disable_generation: 42,
        started_at: 100,
        delete_started_at: None,
        last_error: None,
        retry_after: None,
    });

    let error = manager.do_backup_wallets(&[metadata]).await.unwrap_err();

    assert!(matches!(error, CloudBackupError::Deferred(_)));
    assert_eq!(globals.cloud.uploaded_wallet_backup_count(), 1);
    assert!(matches!(
        Database::global().cloud_backup_state.get().unwrap(),
        PersistedCloudBackupState::Disabling(_)
    ));
    assert_eq!(Database::global().cloud_backup_state.get().unwrap().wallet_count(), Some(3));
    assert!(Database::global().cloud_blob_sync_states.list().unwrap().is_empty());
    manager.debug_reset_cloud_backup_state();
}

#[tokio::test(flavor = "current_thread")]
async fn backup_new_wallet_marks_verification_required() {
    let _guard = async_test_lock().lock().await;
    cove_tokio::init();
    let globals = test_globals();
    let manager = init_manager();
    configure_enabled_cloud_backup(&manager, globals, 3);

    manager.backup_new_wallet(xpub_only_wallet_metadata());

    let state = Database::global().cloud_backup_state.get().unwrap();
    assert_eq!(state.status(), PersistedCloudBackupStatus::Unverified);
    assert!(state.last_verification_requested_at().is_some());

    clear_wallet_upload_runtime_for_test_async(&manager).await;
}

#[tokio::test(flavor = "current_thread")]
async fn pending_upload_state_changes_do_not_dismiss_verification_prompt() {
    let _guard = async_test_lock().lock().await;
    cove_tokio::init();
    let globals = test_globals();
    let manager = init_manager();
    configure_enabled_cloud_backup(&manager, globals, 3);

    manager.backup_new_wallet(xpub_only_wallet_metadata());
    assert_eq!(manager.model_snapshot().root_prompt, CloudBackupRootPrompt::Verification);

    manager.reconcile_pending_upload_verification(PendingUploadVerificationState::Confirming);
    assert_eq!(manager.model_snapshot().root_prompt, CloudBackupRootPrompt::Verification);

    manager.reconcile_pending_upload_verification(
        PendingUploadVerificationState::BlockedOnAuthorization,
    );
    assert_eq!(manager.model_snapshot().root_prompt, CloudBackupRootPrompt::Verification);

    manager.reconcile_pending_upload_verification(PendingUploadVerificationState::Idle);
    let state = manager.model_snapshot();
    assert_eq!(state.root_prompt, CloudBackupRootPrompt::Verification);
    assert!(matches!(
        state.verification_presentation,
        CloudBackupVerificationPresentation::NeedsDecision {
            reason: CloudBackupVerificationReason::BackupChanged,
            ..
        }
    ));

    clear_wallet_upload_runtime_for_test_async(&manager).await;
}

#[tokio::test(flavor = "current_thread")]
async fn verification_prompt_restored_when_pending_upload_idle_hides_persisted_decision() {
    let _guard = async_test_lock().lock().await;
    cove_tokio::init();
    let globals = test_globals();
    let manager = init_manager();
    configure_enabled_cloud_backup(&manager, globals, 3);

    manager.backup_new_wallet(xpub_only_wallet_metadata());
    manager.reconcile_verification_presentation(CloudBackupVerificationPresentation::Hidden {
        source: Some(CloudBackupVerificationSource::Settings),
    });
    assert_eq!(manager.state.read().snapshot().root_prompt, CloudBackupRootPrompt::None);

    manager.reconcile_pending_upload_verification(PendingUploadVerificationState::Idle);

    let state = manager.model_snapshot();
    assert_eq!(state.root_prompt, CloudBackupRootPrompt::Verification);
    assert!(matches!(
        state.verification_presentation,
        CloudBackupVerificationPresentation::NeedsDecision {
            reason: CloudBackupVerificationReason::BackupChanged,
            ..
        }
    ));

    clear_wallet_upload_runtime_for_test_async(&manager).await;
}

#[tokio::test(flavor = "current_thread")]
async fn dismiss_verification_prompt_hides_pending_decision() {
    let _guard = async_test_lock().lock().await;
    cove_tokio::init();
    let globals = test_globals();
    let manager = init_manager();
    configure_enabled_cloud_backup(&manager, globals, 3);

    manager.backup_new_wallet(xpub_only_wallet_metadata());
    assert_eq!(manager.model_snapshot().root_prompt, CloudBackupRootPrompt::Verification);

    manager.dispatch(CloudBackupManagerAction::DismissVerificationPrompt);

    let state = manager.model_snapshot();
    assert_eq!(state.root_prompt, CloudBackupRootPrompt::None);
    assert!(matches!(
        state.verification_presentation,
        CloudBackupVerificationPresentation::Hidden { .. }
    ));

    clear_wallet_upload_runtime_for_test_async(&manager).await;
}

#[tokio::test(flavor = "current_thread")]
async fn start_verification_with_pending_upload_consumes_prompt_decision() {
    let _guard = async_test_lock().lock().await;
    cove_tokio::init();
    let globals = test_globals();
    let manager = init_manager();
    configure_enabled_cloud_backup(&manager, globals, 3);

    let metadata = xpub_only_wallet_metadata();
    let namespace_id = CloudBackupKeychain::global().namespace_id().unwrap();
    let record_id = wallet_record_id(metadata.id.as_ref());
    manager.backup_new_wallet(metadata.clone());
    manager
        .mark_blob_uploaded_pending_confirmation(
            &namespace_id,
            CloudBackupRecordKey::Wallet(metadata.id, record_id),
            "pending-revision".into(),
            jiff::Timestamp::now().as_second().try_into().unwrap_or(0),
        )
        .unwrap();
    assert_eq!(manager.model_snapshot().root_prompt, CloudBackupRootPrompt::Verification);

    manager.dispatch(CloudBackupManagerAction::StartVerification(
        CloudBackupVerificationSource::RootPrompt,
    ));

    let state = manager.model_snapshot();
    assert_eq!(state.root_prompt, CloudBackupRootPrompt::None);
    assert!(matches!(
        state.verification_presentation,
        CloudBackupVerificationPresentation::BackgroundConfirming(
            CloudBackupVerificationSource::RootPrompt
        )
    ));
    assert_eq!(state.pending_upload_verification, PendingUploadVerificationState::Confirming);

    clear_wallet_upload_runtime_for_test_async(&manager).await;
}

#[tokio::test(flavor = "current_thread")]
async fn backup_new_wallet_still_tracks_when_runtime_status_is_error() {
    let _guard = async_test_lock().lock().await;
    cove_tokio::init();
    let globals = test_globals();
    let manager = init_manager();
    globals.reset();

    let namespace = "test-namespace".to_string();
    CloudBackupKeychain::global().save_namespace_id(&namespace).unwrap();
    Database::global().cloud_blob_sync_states.delete_all().unwrap();
    manager
        .persist_cloud_backup_state(
            &persisted_enabled_cloud_backup_state(Some(3)),
            "set cloud backup enabled for test",
        )
        .unwrap();
    manager.sync_persisted_state();

    let metadata = xpub_only_wallet_metadata();
    let record_id = wallet_record_id(metadata.id.as_ref());
    manager.reconcile_runtime_status(CloudBackupStatus::Error("offline".into()));

    manager.backup_new_wallet(metadata);

    let state = Database::global().cloud_backup_state.get().unwrap();
    assert_eq!(state.status(), PersistedCloudBackupStatus::Unverified);
    assert!(state.last_verification_requested_at().is_some());
    assert!(matches!(
        Database::global().cloud_blob_sync_states.get(&record_id).unwrap(),
        Some(PersistedCloudBlobSyncState { state: PersistedCloudBlobState::Dirty(_), .. })
    ));

    clear_wallet_upload_runtime_for_test_async(&manager).await;
}

#[tokio::test(flavor = "current_thread")]
async fn restore_downloaded_wallet_does_not_reupload_wallet_or_mutate_backup_counts() {
    let _guard = async_test_lock().lock().await;
    cove_tokio::init();
    let globals = test_globals();
    let manager = init_manager();
    configure_enabled_cloud_backup(&manager, globals, 5);

    let metadata = xpub_only_wallet_metadata();
    let wallet = DownloadedWalletBackup {
        metadata: metadata.clone(),
        entry: WalletEntry {
            wallet_id: metadata.id.to_string(),
            secret: WalletSecret::WatchOnly,
            metadata: serde_json::to_value(&metadata).unwrap(),
            descriptors: None,
            xpub: Some(sample_xpub(&metadata)),
            wallet_mode: CloudWalletMode::Main,
            labels_zstd_jsonl: None,
            labels_count: 0,
            labels_hash: None,
            labels_uncompressed_size: None,
            content_revision_hash: "test-content-hash".to_string(),
            updated_at: 42,
        },
    };

    WalletRestoreSession::new(crate::backup::import::ExistingWalletIdentitySet::default())
        .restore_downloaded(&wallet)
        .unwrap();

    assert_eq!(globals.cloud.uploaded_wallet_backup_count(), 0);
    assert_eq!(Database::global().cloud_backup_state.get().unwrap().wallet_count(), Some(5));
    assert!(Database::global().cloud_blob_sync_states.list().unwrap().is_empty());
    assert!(
        Database::global()
            .wallets()
            .get(&metadata.id, metadata.network, WalletMode::Main)
            .unwrap()
            .is_some()
    );
}

#[tokio::test(flavor = "current_thread")]
async fn restore_downloaded_wallet_restores_labels_without_marking_cloud_backup_dirty() {
    let _guard = async_test_lock().lock().await;
    cove_tokio::init();
    let globals = test_globals();
    let manager = init_manager();
    configure_enabled_cloud_backup(&manager, globals, 5);

    let metadata = xpub_only_wallet_metadata();
    let wallet = DownloadedWalletBackup {
        metadata: metadata.clone(),
        entry: wallet_entry_with_labels(&metadata, Some(sample_labels_jsonl())),
    };

    let outcome =
        WalletRestoreSession::new(crate::backup::import::ExistingWalletIdentitySet::default())
            .restore_downloaded(&wallet)
            .unwrap();

    assert!(outcome.labels_warning.is_none());
    assert_eq!(globals.cloud.uploaded_wallet_backup_count(), 0);
    assert_eq!(Database::global().cloud_backup_state.get().unwrap().wallet_count(), Some(5));
    assert!(Database::global().cloud_blob_sync_states.list().unwrap().is_empty());

    let exported = LabelManager::new(metadata.id.clone()).export().await.unwrap();
    assert!(exported.contains("\"label\":\"last txn received\""));
}

#[tokio::test(flavor = "current_thread")]
async fn cloud_action_uses_existing_master_key_without_recovery() {
    cove_tokio::init();
    let store = Arc::new(MockStore::default());
    let cspp = cove_cspp::Cspp::new(MockStoreHandle(store));
    let expected = cove_cspp::master_key::MasterKey::generate();
    let namespace = expected.namespace_id();
    cspp.save_master_key(&expected).unwrap();

    let recovered = load_master_key_for_cloud_action(&cspp, &namespace, || async {
        Err(CloudBackupError::RecoveryRequired("unexpected".into()))
    })
    .await
    .unwrap();

    assert_eq!(recovered.as_bytes(), expected.as_bytes());
}

#[tokio::test(flavor = "current_thread")]
async fn cloud_action_does_not_create_master_key_when_missing() {
    cove_tokio::init();
    let store = Arc::new(MockStore::default());
    let cspp = cove_cspp::Cspp::new(MockStoreHandle(store.clone()));
    let namespace = cove_cspp::master_key::MasterKey::generate().namespace_id();

    let result = load_master_key_for_cloud_action(&cspp, &namespace, || async {
        Err(CloudBackupError::RecoveryRequired("needs recovery".into()))
    })
    .await;

    assert!(matches!(
        result,
        Err(CloudBackupError::RecoveryRequired(message)) if message == "needs recovery"
    ));
    assert!(cspp.load_master_key_from_store().unwrap().is_none());
    assert_eq!(*store.save_count.lock(), 0);
}

#[tokio::test(flavor = "current_thread")]
async fn cloud_action_recovers_when_local_master_key_namespace_mismatches() {
    cove_tokio::init();
    let store = Arc::new(MockStore::default());
    let cspp = cove_cspp::Cspp::new(MockStoreHandle(store));
    let stale = cove_cspp::master_key::MasterKey::generate();
    cspp.save_master_key(&stale).unwrap();

    let expected = cove_cspp::master_key::MasterKey::generate();
    let expected_bytes = *expected.as_bytes();
    let namespace = expected.namespace_id();

    let recovered = load_master_key_for_cloud_action(&cspp, &namespace, || async move {
        Ok(cove_cspp::master_key::MasterKey::from_bytes(expected_bytes))
    })
    .await
    .unwrap();

    assert_eq!(recovered.as_bytes(), expected.as_bytes());
}

#[tokio::test(flavor = "current_thread")]
async fn local_master_key_fallback_persists_namespace_id() {
    let _guard = async_test_lock().lock().await;
    cove_tokio::init();
    let globals = test_globals();
    globals.reset();

    let store = Arc::new(MockStore::default());
    let store_handle = MockStoreHandle(store.clone());
    let cspp = cove_cspp::Cspp::new(store_handle.clone());
    let expected = cove_cspp::master_key::MasterKey::generate();
    let namespace_id = expected.namespace_id();
    cspp.save_master_key(&expected).unwrap();
    globals.cloud.set_wallet_files(namespace_id.clone(), vec!["wallet-test.json".into()]);

    let (restored, restored_namespace) = restore_from_local_master_key_fallback(
        &CloudStorage::global_explicit_client(),
        &store_handle,
        &cspp,
    )
    .await
    .unwrap();

    assert_eq!(restored.as_bytes(), expected.as_bytes());
    assert_eq!(restored_namespace, namespace_id.clone());
    assert_eq!(
        store_handle.get(CSPP_NAMESPACE_ID_KEY.into()).as_deref(),
        Some(namespace_id.as_str())
    );
}

#[test]
fn save_passkey_rolls_back_on_second_save_failure() {
    let _guard = test_lock().lock();
    let globals = test_globals();
    globals.reset();
    globals.keychain.set_entries(vec![
        (CSPP_CREDENTIAL_ID_KEY, "old_credential"),
        (CSPP_PRF_SALT_KEY, "old_salt"),
    ]);
    globals.keychain.fail_save_at(2);

    let error = CloudBackupKeychain::global().save_passkey(&[1, 2, 3], [7; 32]).unwrap_err();

    assert!(matches!(
        error,
        CloudBackupKeychainError::Keychain(cove_device::keychain::KeychainError::Save)
    ));
    assert_eq!(
        globals.keychain.get_entry(CSPP_CREDENTIAL_ID_KEY).as_deref(),
        Some("old_credential")
    );
    assert_eq!(globals.keychain.get_entry(CSPP_PRF_SALT_KEY).as_deref(), Some("old_salt"));
}

#[test]
fn save_passkey_and_namespace_rolls_back_on_third_save_failure() {
    let _guard = test_lock().lock();
    let globals = test_globals();
    globals.reset();
    globals.keychain.set_entries(vec![
        (CSPP_CREDENTIAL_ID_KEY, "old_credential"),
        (CSPP_PRF_SALT_KEY, "old_salt"),
        (CSPP_NAMESPACE_ID_KEY, "old_namespace"),
    ]);
    globals.keychain.fail_save_at(3);

    let error = CloudBackupKeychain::global()
        .save_passkey_and_namespace(&[1, 2, 3], [9; 32], "new_namespace")
        .unwrap_err();

    assert!(matches!(
        error,
        CloudBackupKeychainError::Keychain(cove_device::keychain::KeychainError::Save)
    ));
    assert_eq!(
        globals.keychain.get_entry(CSPP_CREDENTIAL_ID_KEY).as_deref(),
        Some("old_credential")
    );
    assert_eq!(globals.keychain.get_entry(CSPP_PRF_SALT_KEY).as_deref(), Some("old_salt"));
    assert_eq!(globals.keychain.get_entry(CSPP_NAMESPACE_ID_KEY).as_deref(), Some("old_namespace"));
}

#[test]
fn load_credential_id_returns_none_for_invalid_hex_and_decodes_valid_hex() {
    let _guard = test_lock().lock();
    let globals = test_globals();
    globals.reset();
    globals.keychain.set_entries(vec![(CSPP_CREDENTIAL_ID_KEY, "not-hex")]);

    assert!(CloudBackupKeychain::global().load_credential_id().is_none());

    let credential_id = vec![1, 2, 3, 254, 255];
    let credential_hex = hex::encode(&credential_id);
    globals.keychain.set_entries(vec![(CSPP_CREDENTIAL_ID_KEY, &credential_hex)]);

    assert_eq!(CloudBackupKeychain::global().load_credential_id(), Some(credential_id));
}

#[test]
fn clear_passkey_removes_credential_and_salt_only() {
    let _guard = test_lock().lock();
    let globals = test_globals();
    globals.reset();
    globals.keychain.set_entries(vec![
        (CSPP_CREDENTIAL_ID_KEY, "credential"),
        (CSPP_PRF_SALT_KEY, "salt"),
        (CSPP_NAMESPACE_ID_KEY, "namespace"),
    ]);

    CloudBackupKeychain::global().clear_passkey();

    assert!(globals.keychain.get_entry(CSPP_CREDENTIAL_ID_KEY).is_none());
    assert!(globals.keychain.get_entry(CSPP_PRF_SALT_KEY).is_none());
    assert_eq!(globals.keychain.get_entry(CSPP_NAMESPACE_ID_KEY).as_deref(), Some("namespace"));
}

#[test]
fn clear_local_state_treats_empty_keychain_as_success() {
    let _guard = test_lock().lock();
    let globals = test_globals();
    globals.reset();

    CloudBackupKeychain::global().clear_local_state().unwrap();
    assert!(CloudBackupKeychain::global().namespace_id().is_none());
}

#[test]
fn clear_local_state_removes_master_key_and_passkey_metadata() {
    let _guard = test_lock().lock();
    let globals = test_globals();
    globals.reset();

    let keychain = Keychain::global();
    let cloud_keychain = CloudBackupKeychain::global();
    let cspp = cove_cspp::Cspp::new(keychain.clone());
    let master_key = cove_cspp::master_key::MasterKey::generate();
    cspp.save_master_key(&master_key).unwrap();
    cloud_keychain.save_passkey_and_namespace(&[1, 2, 3], [4; 32], "test-namespace").unwrap();

    assert!(cspp.load_master_key_from_store().unwrap().is_some());
    assert!(keychain.get(CSPP_CREDENTIAL_ID_KEY.into()).is_some());
    assert!(keychain.get(CSPP_PRF_SALT_KEY.into()).is_some());
    assert!(keychain.get(CSPP_NAMESPACE_ID_KEY.into()).is_some());

    cloud_keychain.clear_local_state().unwrap();

    assert!(cspp.load_master_key_from_store().unwrap().is_none());
    assert!(keychain.get(CSPP_CREDENTIAL_ID_KEY.into()).is_none());
    assert!(keychain.get(CSPP_PRF_SALT_KEY.into()).is_none());
    assert!(keychain.get(CSPP_NAMESPACE_ID_KEY.into()).is_none());
}

#[test]
fn clear_local_state_attempts_passkey_metadata_after_master_key_delete_failure() {
    let _guard = test_lock().lock();
    let globals = test_globals();
    globals.reset();

    let keychain = Keychain::global();
    let cloud_keychain = CloudBackupKeychain::global();
    let cspp = cove_cspp::Cspp::new(keychain.clone());
    let master_key = cove_cspp::master_key::MasterKey::generate();
    cspp.save_master_key(&master_key).unwrap();
    cloud_keychain.save_passkey_and_namespace(&[1, 2, 3], [4; 32], "test-namespace").unwrap();

    globals.keychain.fail_delete_at(1);

    let error = cloud_keychain.clear_local_state().unwrap_err();

    assert!(matches!(
        error,
        CloudBackupKeychainError::Keychain(cove_device::keychain::KeychainError::Delete)
    ));
    assert!(keychain.get(CSPP_CREDENTIAL_ID_KEY.into()).is_none());
    assert!(keychain.get(CSPP_PRF_SALT_KEY.into()).is_none());
    assert!(keychain.get(CSPP_NAMESPACE_ID_KEY.into()).is_none());
}

#[tokio::test(flavor = "current_thread")]
async fn local_master_key_fallback_is_unavailable_after_local_cloud_state_clear() {
    let _guard = async_test_lock().lock().await;
    cove_tokio::init();
    let globals = test_globals();
    globals.reset();

    let keychain = Keychain::global();
    let cspp = cove_cspp::Cspp::new(keychain.clone());
    let master_key = cove_cspp::master_key::MasterKey::generate();
    let namespace_id = master_key.namespace_id();
    cspp.save_master_key(&master_key).unwrap();
    globals.cloud.set_wallet_files(namespace_id, vec!["wallet-test.json".into()]);

    CloudBackupKeychain::global().clear_local_state().unwrap();

    let fallback =
        try_restore_from_local_master_key(&CloudStorage::global_explicit_client(), &cspp)
            .await
            .unwrap();

    assert!(fallback.is_none());
}

#[tokio::test(flavor = "current_thread")]
async fn enable_recovery_rolls_back_local_master_key_when_wallet_upload_fails() {
    let _guard = async_test_lock().lock().await;
    cove_tokio::init();
    let globals = test_globals();
    let manager = init_manager();

    reset_cloud_backup_test_state(&manager, globals);
    persist_xpub_wallets(vec![xpub_only_wallet_metadata()]);
    globals.cloud.fail_wallet_backup_upload("upload failed");

    let cspp = cove_cspp::Cspp::new(Keychain::global().clone());
    let matched = NamespaceMatch {
        namespace_id: "matched-namespace".into(),
        master_key: cove_cspp::master_key::MasterKey::generate(),
        prf_salt: [9; 32],
        credential_id: vec![1, 2, 3],
    };

    let preparation = manager.prepare_enable_recovery(vec![matched]).await.unwrap();
    manager.save_enable_recovery_master_key(&preparation).unwrap();
    let claim = CloudBackupExclusiveOperationClaim::new(CloudBackupExclusiveOperation::Enable, 42);
    manager.project_exclusive_operation_started(claim);
    let writes = operation_write_client_for_test(&manager, claim);
    let error = manager.prepare_enable_recovery_completion(preparation, writes).await.unwrap_err();
    manager.project_exclusive_operation_finished(claim);
    manager.rollback_enable_recovery_master_key();

    assert!(matches!(error, CloudBackupError::CloudStorage(_)));
    assert!(cspp.load_master_key_from_store().unwrap().is_none());
}

#[tokio::test(flavor = "current_thread")]
async fn enable_recovery_rolls_back_local_master_key_when_keychain_save_fails() {
    let _guard = async_test_lock().lock().await;
    cove_tokio::init();
    let globals = test_globals();
    let manager = init_manager();

    reset_cloud_backup_test_state(&manager, globals);
    globals.keychain.fail_save_at(3);

    let cspp = cove_cspp::Cspp::new(Keychain::global().clone());
    let matched = NamespaceMatch {
        namespace_id: "matched-namespace".into(),
        master_key: cove_cspp::master_key::MasterKey::generate(),
        prf_salt: [9; 32],
        credential_id: vec![1, 2, 3],
    };

    let preparation = manager.prepare_enable_recovery(vec![matched]).await.unwrap();
    manager.save_enable_recovery_master_key(&preparation).unwrap();
    let claim = CloudBackupExclusiveOperationClaim::new(CloudBackupExclusiveOperation::Enable, 42);
    manager.project_exclusive_operation_started(claim);
    let writes = operation_write_client_for_test(&manager, claim);
    let completion = manager.prepare_enable_recovery_completion(preparation, writes).await.unwrap();
    manager.project_exclusive_operation_finished(claim);
    let error = CloudBackupKeychain::global()
        .save_passkey_and_namespace(
            &completion.credential_id,
            completion.prf_salt,
            &completion.namespace_id,
        )
        .unwrap_err();
    manager.rollback_enable_recovery_master_key();

    assert!(matches!(error, CloudBackupKeychainError::Keychain(_)));
    assert!(cspp.load_master_key_from_store().unwrap().is_none());
}

#[tokio::test(flavor = "current_thread")]
async fn restore_from_local_master_key_propagates_store_read_errors() {
    let _guard = async_test_lock().lock().await;
    cove_tokio::init();
    let globals = test_globals();
    globals.reset();

    let store = Arc::new(MockStore::default());
    let store_handle = MockStoreHandle(store.clone());
    let cspp = cove_cspp::Cspp::new(store_handle);
    let expected = cove_cspp::master_key::MasterKey::generate();
    cspp.save_master_key(&expected).unwrap();
    let key_to_corrupt =
        store.entries.lock().keys().next().cloned().expect("saved master key entry");
    store.entries.lock().insert(key_to_corrupt, "not-a-valid-master-key".into());

    let error =
        match try_restore_from_local_master_key(&CloudStorage::global_explicit_client(), &cspp)
            .await
        {
            Ok(_) => panic!("expected local master key read failure"),
            Err(error) => error,
        };

    assert!(matches!(
        error,
        CloudBackupError::Internal(message)
            if message.starts_with("loading master key from store:")
    ));
}

#[test]
fn blocking_cloud_error_rewrites_unavailable_storage_errors_to_offline() {
    let error = blocking_cloud_error(
        BlockingCloudStep::Enable,
        CloudBackupError::CloudStorage(CloudStorageError::NotAvailable(
            "iCloud Drive is not available".into(),
        )),
    );

    assert!(matches!(error, CloudBackupError::Offline(_)));
}

#[tokio::test(flavor = "current_thread")]
async fn failed_create_new_enable_does_not_persist_passkey_metadata() {
    let _guard = async_test_lock().lock().await;
    cove_tokio::init();
    let globals = test_globals();
    globals.reset();
    globals.cloud.fail_master_key_upload("boom");
    globals.passkey.set_discover_result(Ok(DiscoveredPasskeyResult {
        prf_output: vec![7; 32],
        credential_id: vec![1, 2, 3],
    }));

    let manager = init_manager();
    let error = enable_cloud_backup_create_new(&manager).await.unwrap_err();
    assert!(matches!(
        error,
        CloudBackupError::Internal(message)
            if message.contains("upload master key backup") && message.contains("boom")
    ));

    let keychain = Keychain::global();
    assert!(keychain.get(CSPP_CREDENTIAL_ID_KEY.into()).is_none());
    assert!(keychain.get(CSPP_PRF_SALT_KEY.into()).is_none());
    assert!(keychain.get(CSPP_NAMESPACE_ID_KEY.into()).is_none());
}

#[tokio::test(flavor = "current_thread")]
async fn failed_no_discovery_enable_does_not_persist_passkey_metadata() {
    let _guard = async_test_lock().lock().await;
    cove_tokio::init();
    let globals = test_globals();
    globals.reset();
    globals.passkey.set_create_result(Ok(vec![1, 2, 3]));
    globals.passkey.set_authenticate_result(Err(PasskeyError::RequestFailed {
        operation: PasskeyOperation::AuthenticateAssertion,
        reason: PasskeyFailureReason::Unknown { diagnostic_message: "boom".into() },
    }));

    let manager = init_manager();
    enable_cloud_backup_no_discovery(&manager).await.unwrap();
    confirm_saved_passkey_session(&manager).await;

    let keychain = Keychain::global();
    let cspp = cove_cspp::Cspp::new(keychain.clone());
    assert!(cspp.load_master_key_from_store().unwrap().is_some());
    assert!(take_pending_enable_session_for_test(&manager).await.is_some());
    assert!(keychain.get(CSPP_CREDENTIAL_ID_KEY.into()).is_none());
    assert!(keychain.get(CSPP_PRF_SALT_KEY.into()).is_none());
    assert!(keychain.get(CSPP_NAMESPACE_ID_KEY.into()).is_none());
}

#[tokio::test(flavor = "current_thread")]
async fn enable_create_new_succeeds_with_new_passkey_auth() {
    let _guard = async_test_lock().lock().await;
    cove_tokio::init();
    let globals = test_globals();
    let manager = init_manager();

    reset_cloud_backup_test_state(&manager, globals);
    CONNECTIVITY_MANAGER.set_connection_state(true);
    globals.passkey.set_create_result(Ok(vec![1, 2, 3]));
    globals.passkey.set_authenticate_result(Ok(vec![7; 32]));

    enable_cloud_backup_create_new(&manager).await.unwrap();
    confirm_saved_passkey_session(&manager).await;

    assert_eq!(manager.current_status(), CloudBackupStatus::Enabled);
    let state = manager.model_snapshot();
    assert!(matches!(state.verification, VerificationState::Idle));
    assert_eq!(state.pending_upload_verification, PendingUploadVerificationState::Confirming);
    assert!(matches!(state.root_prompt, CloudBackupRootPrompt::None));
    assert!(globals.keychain.get_entry(CSPP_CREDENTIAL_ID_KEY).is_some());
    assert!(globals.keychain.get_entry(CSPP_PRF_SALT_KEY).is_some());
    assert!(globals.keychain.get_entry(CSPP_NAMESPACE_ID_KEY).is_some());

    let discover_count = globals.passkey.discover_count();
    let authenticate_count = globals.passkey.authenticate_count();

    call!(manager.supervisor.start_enter_detail()).await.unwrap();
    tokio::time::sleep(Duration::from_millis(50)).await;

    assert_eq!(globals.passkey.discover_count(), discover_count);
    assert_eq!(globals.passkey.authenticate_count(), authenticate_count);
}

#[tokio::test(flavor = "current_thread")]
async fn detail_entry_starts_discoverable_verification_without_runtime_authorization() {
    let _guard = async_test_lock().lock().await;
    cove_tokio::init();
    let globals = test_globals();
    let manager = init_manager();

    reset_cloud_backup_test_state(&manager, globals);
    CONNECTIVITY_MANAGER.set_connection_state(true);
    globals.passkey.set_create_result(Ok(vec![1, 2, 3]));
    globals.passkey.set_authenticate_result(Ok(vec![7; 32]));

    enable_cloud_backup_no_discovery(&manager).await.unwrap();
    confirm_saved_passkey_session(&manager).await;

    manager.clear_runtime_passkey_authorization();
    manager.clear_pending_verification_completion();
    manager.reconcile_pending_upload_verification(PendingUploadVerificationState::Idle);
    manager.apply_verification_outcome(CloudBackupVerificationOutcome::Idle);
    Database::global().cloud_blob_sync_states.delete_all().unwrap();
    globals.passkey.set_discover_result(Err(PasskeyError::UserCancelled));
    globals.cloud.fail_list_wallet_files("list should not run before passkey auth");

    let discover_count = globals.passkey.discover_count();
    let list_count = globals.cloud.list_wallet_files_attempt_count();

    call!(manager.supervisor.start_enter_detail()).await.unwrap();
    wait_for_discover_count(globals, discover_count + 1).await;

    assert_eq!(globals.passkey.discover_count(), discover_count + 1);
    assert_eq!(globals.cloud.list_wallet_files_attempt_count(), list_count);
}

#[tokio::test(flavor = "current_thread")]
async fn detail_entry_does_not_restart_rust_owned_verification_states() {
    let _guard = async_test_lock().lock().await;
    cove_tokio::init();
    let globals = test_globals();
    let manager = init_manager();

    let states = [
        VerificationState::Verifying,
        VerificationState::Verified(DeepVerificationReport {
            master_key_wrapper_repaired: false,
            local_master_key_repaired: false,
            credential_recovered: false,
            wallets_verified: 0,
            wallets_failed: 0,
            wallets_unsupported: 0,
            detail: None,
        }),
        VerificationState::PasskeyConfirmed,
    ];

    for verification in states {
        configure_enabled_cloud_backup(&manager, globals, 0);
        manager.clear_runtime_passkey_authorization();
        manager.clear_pending_verification_completion();
        manager.reconcile_pending_upload_verification(PendingUploadVerificationState::Idle);
        manager
            .apply_verification_outcome(CloudBackupVerificationOutcome::from_state(verification));
        globals.passkey.set_discover_result(Err(PasskeyError::UserCancelled));

        let discover_count = globals.passkey.discover_count();
        let authenticate_count = globals.passkey.authenticate_count();

        call!(manager.supervisor.start_enter_detail()).await.unwrap();
        tokio::time::sleep(Duration::from_millis(50)).await;

        assert_eq!(globals.passkey.discover_count(), discover_count);
        assert_eq!(globals.passkey.authenticate_count(), authenticate_count);
    }
}

#[tokio::test(flavor = "current_thread")]
async fn deep_verify_authenticates_before_loading_wallet_inventory() {
    let _guard = async_test_lock().lock().await;
    cove_tokio::init();
    let globals = test_globals();
    let manager = init_manager();

    reset_cloud_backup_test_state(&manager, globals);
    configure_enabled_cloud_backup(&manager, globals, 0);
    CONNECTIVITY_MANAGER.set_connection_state(true);
    seed_verifiable_cloud_master_key(globals);
    globals.passkey.set_discover_result(Err(PasskeyError::UserCancelled));
    globals.cloud.fail_list_wallet_files("list should not run before passkey auth");

    let discover_count = globals.passkey.discover_count();
    let list_count = globals.cloud.list_wallet_files_attempt_count();

    call!(manager.supervisor.start_verification(true)).await.unwrap();
    wait_for_discover_count(globals, discover_count + 1).await;

    assert_eq!(globals.passkey.discover_count(), discover_count + 1);
    assert_eq!(globals.cloud.list_wallet_files_attempt_count(), list_count);
}

#[tokio::test(flavor = "current_thread")]
async fn enable_no_discovery_succeeds_with_new_passkey_auth() {
    let _guard = async_test_lock().lock().await;
    cove_tokio::init();
    let globals = test_globals();
    let manager = init_manager();

    reset_cloud_backup_test_state(&manager, globals);
    CONNECTIVITY_MANAGER.set_connection_state(true);
    globals.passkey.set_create_result(Ok(vec![1, 2, 3]));
    globals.passkey.set_authenticate_result(Ok(vec![7; 32]));

    enable_cloud_backup_no_discovery(&manager).await.unwrap();
    confirm_saved_passkey_session(&manager).await;

    assert_eq!(manager.current_status(), CloudBackupStatus::Enabled);
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn registered_passkey_stages_confirmation_without_automatic_auth() {
    let _guard = async_test_lock().lock().await;
    cove_tokio::init();
    let globals = test_globals();
    let manager = init_manager();

    reset_cloud_backup_test_state(&manager, globals);
    CONNECTIVITY_MANAGER.set_connection_state(true);
    globals.passkey.set_create_result(Ok(vec![1, 2, 3]));
    globals.passkey.set_authenticate_result(Ok(vec![7; 32]));

    enable_cloud_backup_no_discovery(&manager).await.unwrap();

    assert_eq!(globals.passkey.create_count(), 1);
    assert_eq!(globals.passkey.authenticate_count(), 0);
    assert_eq!(CloudBackupKeychain::global().load_credential_id(), None);
    assert_eq!(
        manager.model_snapshot().enable_state,
        CloudBackupEnableState::AwaitingSavedPasskeyConfirmation(
            SavedPasskeyConfirmationMode::Manual,
        )
    );

    let pending = take_pending_enable_session_for_test(&manager).await.unwrap();
    assert!(matches!(pending, PendingEnableSession::AwaitingSavedPasskeyConfirmation(_)));
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn registered_passkey_confirmation_session_prevents_duplicate_create() {
    let _guard = async_test_lock().lock().await;
    cove_tokio::init();
    let globals = test_globals();
    let manager = init_manager();

    reset_cloud_backup_test_state(&manager, globals);
    CONNECTIVITY_MANAGER.set_connection_state(true);
    globals.passkey.set_create_result(Ok(vec![1, 2, 3]));

    enable_cloud_backup_no_discovery(&manager).await.unwrap();
    enable_cloud_backup_no_discovery(&manager).await.unwrap();

    assert_eq!(globals.passkey.create_count(), 1);
    assert_eq!(globals.passkey.authenticate_count(), 0);
    assert_eq!(
        manager.model_snapshot().enable_state,
        CloudBackupEnableState::AwaitingSavedPasskeyConfirmation(
            SavedPasskeyConfirmationMode::Manual,
        )
    );

    let pending = take_pending_enable_session_for_test(&manager).await.unwrap();
    assert!(matches!(pending, PendingEnableSession::AwaitingSavedPasskeyConfirmation(_)));
    let (_, pending_passkey) = pending.into_staged_parts().unwrap();
    assert_eq!(pending_passkey.credential_id, vec![1, 2, 3]);
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn normal_enable_preserves_registered_passkey_confirmation_session() {
    let _guard = async_test_lock().lock().await;
    cove_tokio::init();
    let globals = test_globals();
    let manager = init_manager();

    reset_cloud_backup_test_state(&manager, globals);
    CONNECTIVITY_MANAGER.set_connection_state(true);
    globals.passkey.set_create_result(Ok(vec![1, 2, 3]));

    enable_cloud_backup_no_discovery(&manager).await.unwrap();

    globals.passkey.set_create_result(Ok(vec![4, 5, 6]));
    enable_cloud_backup_create_new(&manager).await.unwrap();

    assert_eq!(globals.passkey.create_count(), 1);
    assert_eq!(globals.passkey.authenticate_count(), 0);
    assert_eq!(
        manager.model_snapshot().enable_state,
        CloudBackupEnableState::AwaitingSavedPasskeyConfirmation(
            SavedPasskeyConfirmationMode::Manual,
        )
    );

    let pending = take_pending_enable_session_for_test(&manager).await.unwrap();
    assert!(matches!(pending, PendingEnableSession::AwaitingSavedPasskeyConfirmation(_)));
    let (_, pending_passkey) = pending.into_staged_parts().unwrap();
    assert_eq!(pending_passkey.credential_id, vec![1, 2, 3]);
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn reinitialize_preserves_registered_passkey_confirmation_session() {
    let _guard = async_test_lock().lock().await;
    cove_tokio::init();
    let globals = test_globals();
    let manager = init_manager();

    reset_cloud_backup_test_state(&manager, globals);
    CONNECTIVITY_MANAGER.set_connection_state(true);
    globals.passkey.set_create_result(Ok(vec![1, 2, 3]));

    enable_cloud_backup_no_discovery(&manager).await.unwrap();

    globals.passkey.set_create_result(Ok(vec![4, 5, 6]));
    run_enable_operation(
        &manager,
        CloudBackupOperation::Recovery(RecoveryAction::ReinitializeBackup),
    )
    .await
    .unwrap();

    assert_eq!(globals.passkey.create_count(), 1);
    assert_eq!(globals.passkey.authenticate_count(), 0);
    assert_eq!(
        manager.model_snapshot().enable_state,
        CloudBackupEnableState::AwaitingSavedPasskeyConfirmation(
            SavedPasskeyConfirmationMode::Manual,
        )
    );

    let pending = take_pending_enable_session_for_test(&manager).await.unwrap();
    assert!(matches!(pending, PendingEnableSession::AwaitingSavedPasskeyConfirmation(_)));
    let (_, pending_passkey) = pending.into_staged_parts().unwrap();
    assert_eq!(pending_passkey.credential_id, vec![1, 2, 3]);
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn force_new_preserves_registered_passkey_confirmation_session() {
    let _guard = async_test_lock().lock().await;
    cove_tokio::init();
    let globals = test_globals();
    let manager = init_manager();

    reset_cloud_backup_test_state(&manager, globals);
    CONNECTIVITY_MANAGER.set_connection_state(true);
    globals.passkey.set_create_result(Ok(vec![1, 2, 3]));

    enable_cloud_backup_no_discovery(&manager).await.unwrap();

    globals.passkey.set_create_result(Ok(vec![4, 5, 6]));
    enable_cloud_backup_force_new(&manager).await.unwrap();

    assert_eq!(globals.passkey.create_count(), 1);
    assert_eq!(globals.passkey.authenticate_count(), 0);
    assert_eq!(
        manager.model_snapshot().enable_state,
        CloudBackupEnableState::AwaitingSavedPasskeyConfirmation(
            SavedPasskeyConfirmationMode::Manual,
        )
    );

    let pending = take_pending_enable_session_for_test(&manager).await.unwrap();
    assert!(matches!(pending, PendingEnableSession::AwaitingSavedPasskeyConfirmation(_)));
    let (_, pending_passkey) = pending.into_staged_parts().unwrap();
    assert_eq!(pending_passkey.credential_id, vec![1, 2, 3]);
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn registered_passkey_stages_confirmation_without_duplicate_create() {
    let _guard = async_test_lock().lock().await;
    cove_tokio::init();
    let globals = test_globals();
    let manager = init_manager();

    reset_cloud_backup_test_state(&manager, globals);
    CONNECTIVITY_MANAGER.set_connection_state(true);
    globals.passkey.set_create_result(Ok(vec![1, 2, 3]));

    enable_cloud_backup_no_discovery(&manager).await.unwrap();

    assert_eq!(globals.passkey.create_count(), 1);
    assert_eq!(globals.passkey.authenticate_count(), 0);
    assert_eq!(
        manager.model_snapshot().enable_state,
        CloudBackupEnableState::AwaitingSavedPasskeyConfirmation(
            SavedPasskeyConfirmationMode::Manual,
        )
    );

    let pending = take_pending_enable_session_for_test(&manager).await.unwrap();
    assert!(matches!(pending, PendingEnableSession::AwaitingSavedPasskeyConfirmation(_)));
    let (_, pending_passkey) = pending.into_staged_parts().unwrap();
    assert_eq!(pending_passkey.credential_id, vec![1, 2, 3]);
}

#[tokio::test(flavor = "current_thread")]
async fn confirm_saved_passkey_reuses_original_credential_id() {
    let _guard = async_test_lock().lock().await;
    cove_tokio::init();
    let globals = test_globals();
    let manager = init_manager();

    reset_cloud_backup_test_state(&manager, globals);
    CONNECTIVITY_MANAGER.set_connection_state(true);
    globals.passkey.set_create_result(Ok(vec![1, 2, 3]));

    enable_cloud_backup_no_discovery(&manager).await.unwrap();

    globals.passkey.set_authenticate_result(Ok(vec![7; 32]));
    confirm_saved_passkey_session(&manager).await;

    assert_eq!(globals.passkey.create_count(), 1);
    assert_eq!(globals.passkey.authenticate_count(), 1);
    assert_eq!(globals.passkey.authenticated_credential_ids(), vec![vec![1, 2, 3]]);
    assert_eq!(manager.current_status(), CloudBackupStatus::Enabled);
    assert!(take_pending_enable_session_for_test(&manager).await.is_none());
}

#[tokio::test(flavor = "current_thread")]
async fn duplicate_confirm_saved_passkey_dispatches_are_ignored_while_confirming() {
    let _guard = async_test_lock().lock().await;
    cove_tokio::init();
    let globals = test_globals();
    let manager = init_manager();

    reset_cloud_backup_test_state(&manager, globals);
    CONNECTIVITY_MANAGER.set_connection_state(true);
    let master_key = cove_cspp::master_key::MasterKey::generate();
    cove_cspp::Cspp::new(Keychain::global().clone()).save_master_key(&master_key).unwrap();
    replace_pending_enable_session_for_test(
        &manager,
        PendingEnableSession::awaiting_saved_passkey_confirmation(
            zeroize::Zeroizing::new(master_key),
            zeroize::Zeroizing::new(StagedPrfKey {
                prf_salt: [9; 32],
                credential_id: vec![1, 2, 3],
                provider_hint: None,
            }),
            CloudBackupEnableContext::settings_manual(),
        ),
    )
    .await;

    globals.passkey.set_authenticate_result(Ok(vec![7; 32]));
    manager.dispatch(CloudBackupManagerAction::ConfirmSavedPasskey);
    manager.dispatch(CloudBackupManagerAction::ConfirmSavedPasskey);

    wait_for_test_condition(
        Duration::from_secs(1),
        "saved passkey confirmation should enable cloud backup",
        || manager.current_status() == CloudBackupStatus::Enabled,
    )
    .await;

    assert_eq!(globals.passkey.authenticate_count(), 1);
    assert!(take_pending_enable_session_for_test(&manager).await.is_none());

    let authenticate_count = globals.passkey.authenticate_count();
    let has_more_pending = verify_pending_uploads_once_for_test_async(&manager).await;

    assert!(!has_more_pending);
    assert_eq!(globals.passkey.authenticate_count(), authenticate_count);
    assert_eq!(
        manager.model_snapshot().pending_upload_verification,
        PendingUploadVerificationState::Idle
    );
    assert!(matches!(manager.model_snapshot().verification, VerificationState::Verified(_)));
}

#[tokio::test(flavor = "current_thread")]
async fn cancelled_saved_passkey_confirmation_preserves_pending_session() {
    let _guard = async_test_lock().lock().await;
    cove_tokio::init();
    let globals = test_globals();
    let manager = init_manager();

    reset_cloud_backup_test_state(&manager, globals);
    CONNECTIVITY_MANAGER.set_connection_state(true);
    globals.passkey.set_create_result(Ok(vec![1, 2, 3]));

    enable_cloud_backup_no_discovery(&manager).await.unwrap();

    globals.passkey.set_authenticate_result(Err(PasskeyError::UserCancelled));
    confirm_saved_passkey_session(&manager).await;

    assert_eq!(
        manager.model_snapshot().enable_state,
        CloudBackupEnableState::AwaitingSavedPasskeyConfirmation(
            SavedPasskeyConfirmationMode::Manual,
        )
    );
    let pending = take_pending_enable_session_for_test(&manager).await.unwrap();
    assert!(matches!(pending, PendingEnableSession::AwaitingSavedPasskeyConfirmation(_)));
}

#[tokio::test(flavor = "current_thread")]
async fn unsupported_provider_saved_passkey_confirmation_fails_without_retry() {
    let _guard = async_test_lock().lock().await;
    cove_tokio::init();
    let globals = test_globals();
    let manager = init_manager();

    reset_cloud_backup_test_state(&manager, globals);
    CONNECTIVITY_MANAGER.set_connection_state(true);
    let master_key = cove_cspp::master_key::MasterKey::generate();
    replace_pending_enable_session_for_test(
        &manager,
        PendingEnableSession::awaiting_saved_passkey_confirmation(
            zeroize::Zeroizing::new(master_key),
            zeroize::Zeroizing::new(StagedPrfKey {
                prf_salt: [9; 32],
                credential_id: vec![1, 2, 3],
                provider_hint: None,
            }),
            CloudBackupEnableContext::settings_manual(),
        ),
    )
    .await;

    globals.passkey.set_authenticate_result(Err(PasskeyError::PrfUnsupportedProvider));
    confirm_saved_passkey_session(&manager).await;

    assert_eq!(globals.passkey.authenticate_count(), 1);
    assert_eq!(manager.current_status(), CloudBackupStatus::UnsupportedPasskeyProvider);
    assert!(take_pending_enable_session_for_test(&manager).await.is_none());
}

#[tokio::test(flavor = "current_thread")]
async fn enable_with_multiple_matching_namespaces_merges_into_largest_namespace() {
    let _guard = async_test_lock().lock().await;
    cove_tokio::init();
    let globals = test_globals();
    let manager = init_manager();

    reset_cloud_backup_test_state(&manager, globals);
    CONNECTIVITY_MANAGER.set_connection_state(true);
    globals.cloud.set_reflect_uploaded_wallets_in_listing(true);

    let prf_key = [7u8; 32];
    let first_master_key = cove_cspp::master_key::MasterKey::generate();
    let second_master_key = cove_cspp::master_key::MasterKey::generate();
    let first_namespace = first_master_key.namespace_id();
    let second_namespace = second_master_key.namespace_id();
    let first_encrypted =
        cove_cspp::master_key_crypto::encrypt_master_key(&first_master_key, &prf_key, &[9; 32])
            .unwrap();
    let second_encrypted =
        cove_cspp::master_key_crypto::encrypt_master_key(&second_master_key, &prf_key, &[8; 32])
            .unwrap();

    globals.cloud.set_master_key_backup(
        first_namespace.clone(),
        serde_json::to_vec(&first_encrypted).unwrap(),
    );
    globals.cloud.set_master_key_backup(
        second_namespace.clone(),
        serde_json::to_vec(&second_encrypted).unwrap(),
    );
    globals.passkey.set_discover_result(Ok(DiscoveredPasskeyResult {
        prf_output: prf_key.to_vec(),
        credential_id: vec![1, 2, 3],
    }));
    globals.passkey.set_authenticate_result(Ok(prf_key.to_vec()));

    let first_wallet = xpub_only_wallet_metadata();
    let second_wallet = xpub_only_wallet_metadata();
    let third_wallet = xpub_only_wallet_metadata();
    let first_wallet = WalletMetadata { master_fingerprint: None, ..first_wallet };
    let second_wallet = WalletMetadata { master_fingerprint: None, ..second_wallet };
    let third_wallet = WalletMetadata { master_fingerprint: None, ..third_wallet };
    Keychain::global()
        .save_wallet_xpub(&first_wallet.id, sample_xpub(&first_wallet).parse().unwrap())
        .unwrap();
    Keychain::global()
        .save_wallet_xpub(&second_wallet.id, sample_xpub(&second_wallet).parse().unwrap())
        .unwrap();
    Keychain::global()
        .save_wallet_xpub(&third_wallet.id, sample_xpub(&third_wallet).parse().unwrap())
        .unwrap();

    let first_record_id = cove_cspp::backup_data::wallet_record_id(first_wallet.id.as_ref());
    let second_record_id = cove_cspp::backup_data::wallet_record_id(second_wallet.id.as_ref());
    let third_record_id = cove_cspp::backup_data::wallet_record_id(third_wallet.id.as_ref());
    let first_revision = crate::manager::cloud_backup_manager::wallets::prepare_wallet_backup(
        &first_wallet,
        first_wallet.wallet_mode,
    )
    .await
    .unwrap()
    .revision_hash;
    let second_revision = crate::manager::cloud_backup_manager::wallets::prepare_wallet_backup(
        &second_wallet,
        second_wallet.wallet_mode,
    )
    .await
    .unwrap()
    .revision_hash;
    let third_revision = crate::manager::cloud_backup_manager::wallets::prepare_wallet_backup(
        &third_wallet,
        third_wallet.wallet_mode,
    )
    .await
    .unwrap()
    .revision_hash;
    globals.cloud.set_wallet_backup(
        first_namespace.clone(),
        first_record_id.clone(),
        encrypted_wallet_backup_bytes(&first_wallet, &first_master_key, &first_revision, 1).await,
    );
    globals.cloud.set_wallet_backup(
        second_namespace.clone(),
        second_record_id.clone(),
        encrypted_wallet_backup_bytes(&second_wallet, &second_master_key, &second_revision, 1)
            .await,
    );
    globals.cloud.set_wallet_backup(
        second_namespace.clone(),
        third_record_id.clone(),
        encrypted_wallet_backup_bytes(&third_wallet, &second_master_key, &third_revision, 1).await,
    );
    globals.cloud.set_wallet_files(
        first_namespace.clone(),
        vec![wallet_filename_from_record_id(&first_record_id)],
    );
    globals.cloud.set_wallet_files(
        second_namespace.clone(),
        vec![
            wallet_filename_from_record_id(&second_record_id),
            wallet_filename_from_record_id(&third_record_id),
        ],
    );

    enable_cloud_backup_create_new(&manager).await.unwrap();

    assert_eq!(manager.current_status(), CloudBackupStatus::Enabled);
    assert_eq!(CloudBackupKeychain::global().namespace_id(), Some(second_namespace.clone()));
    assert_eq!(Database::global().cloud_backup_state.get().unwrap().wallet_count(), Some(3));
    assert!(globals.cloud.has_namespace(&second_namespace));
    wait_for_test_condition(
        Duration::from_secs(1),
        "merged source namespace should be deleted after proof",
        || !globals.cloud.has_namespace(&first_namespace),
    )
    .await;

    let active_records =
        CloudStorage::global_explicit_client().list_wallet_backups(second_namespace).await.unwrap();
    assert!(active_records.contains(&first_record_id));
    assert!(active_records.contains(&second_record_id));
    assert!(active_records.contains(&third_record_id));
}

#[tokio::test(flavor = "current_thread")]
async fn enable_treats_missing_wallet_listing_as_empty_during_recovery() {
    let _guard = async_test_lock().lock().await;
    cove_tokio::init();
    let globals = test_globals();
    let manager = init_manager();

    reset_cloud_backup_test_state(&manager, globals);
    CONNECTIVITY_MANAGER.set_connection_state(true);
    globals.cloud.set_reflect_uploaded_wallets_in_listing(true);

    let prf_key = [7u8; 32];
    let empty_master_key = cove_cspp::master_key::MasterKey::generate();
    let wallet_master_key = cove_cspp::master_key::MasterKey::generate();
    let empty_namespace = empty_master_key.namespace_id();
    let wallet_namespace = wallet_master_key.namespace_id();
    let empty_encrypted =
        cove_cspp::master_key_crypto::encrypt_master_key(&empty_master_key, &prf_key, &[9; 32])
            .unwrap();
    let wallet_encrypted =
        cove_cspp::master_key_crypto::encrypt_master_key(&wallet_master_key, &prf_key, &[8; 32])
            .unwrap();

    globals.cloud.set_master_key_backup(
        empty_namespace.clone(),
        serde_json::to_vec(&empty_encrypted).unwrap(),
    );
    globals.cloud.set_master_key_backup(
        wallet_namespace.clone(),
        serde_json::to_vec(&wallet_encrypted).unwrap(),
    );
    globals.cloud.fail_list_wallet_files_for_namespace(
        empty_namespace.clone(),
        CloudStorageError::NotFound("wallet files missing".into()),
    );
    globals.passkey.set_discover_result(Ok(DiscoveredPasskeyResult {
        prf_output: prf_key.to_vec(),
        credential_id: vec![1, 2, 3],
    }));
    globals.passkey.set_authenticate_result(Ok(prf_key.to_vec()));

    let wallet = xpub_only_wallet_metadata();
    let wallet = WalletMetadata { master_fingerprint: None, ..wallet };
    Keychain::global().save_wallet_xpub(&wallet.id, sample_xpub(&wallet).parse().unwrap()).unwrap();
    let record_id = cove_cspp::backup_data::wallet_record_id(wallet.id.as_ref());
    let revision = crate::manager::cloud_backup_manager::wallets::prepare_wallet_backup(
        &wallet,
        wallet.wallet_mode,
    )
    .await
    .unwrap()
    .revision_hash;
    globals.cloud.set_wallet_backup(
        wallet_namespace.clone(),
        record_id.clone(),
        encrypted_wallet_backup_bytes(&wallet, &wallet_master_key, &revision, 1).await,
    );
    globals.cloud.set_wallet_files(
        wallet_namespace.clone(),
        vec![wallet_filename_from_record_id(&record_id)],
    );

    enable_cloud_backup_create_new(&manager).await.unwrap();

    assert_eq!(manager.current_status(), CloudBackupStatus::Enabled);
    assert_eq!(CloudBackupKeychain::global().namespace_id(), Some(wallet_namespace.clone()));
    assert_eq!(Database::global().cloud_backup_state.get().unwrap().wallet_count(), Some(1));
    assert!(globals.cloud.has_namespace(&wallet_namespace));
}

struct CleanupTestSource {
    namespace: String,
    record_id: String,
    revision_hash: Option<String>,
}

async fn enqueue_cleanup_for_test(
    manager: &RustCloudBackupManager,
    active_namespace: &str,
    active_master_key: &cove_cspp::master_key::MasterKey,
    source: CleanupTestSource,
    wait_message: &str,
    mut wait_condition: impl FnMut() -> bool,
) {
    call!(manager.supervisor.enqueue_cleanup_for_test(CloudBackupCleanupJob {
        cloud: CloudStorage::global_explicit_client(),
        active_namespace_id: active_namespace.to_owned(),
        active_critical_key: active_master_key.critical_data_key(),
        sources: vec![CleanupSourceNamespace {
            namespace_id: source.namespace,
            expected_wallets: vec![CleanupExpectedWalletRecord {
                record_id: source.record_id,
                content_revision_hash: source.revision_hash,
            }],
        }],
    }))
    .await
    .expect("enqueue cleanup");

    wait_for_test_condition(Duration::from_secs(1), wait_message, &mut wait_condition).await;
}

#[tokio::test(flavor = "current_thread")]
async fn cleanup_deletes_source_namespace_after_active_record_proof() {
    let _guard = async_test_lock().lock().await;
    cove_tokio::init();
    let globals = test_globals();
    let manager = init_manager();

    reset_cloud_backup_test_state(&manager, globals);
    let active_master_key = cove_cspp::master_key::MasterKey::generate();
    let active_namespace = active_master_key.namespace_id();
    let source_namespace = "source-namespace".to_string();
    let wallet = xpub_only_wallet_metadata();
    Keychain::global().save_wallet_xpub(&wallet.id, sample_xpub(&wallet).parse().unwrap()).unwrap();
    let record_id = cove_cspp::backup_data::wallet_record_id(wallet.id.as_ref());

    globals.cloud.set_wallet_backup(
        active_namespace.clone(),
        record_id.clone(),
        encrypted_wallet_backup_bytes(&wallet, &active_master_key, "matching-revision", 1).await,
    );
    globals.cloud.set_wallet_files(
        active_namespace.clone(),
        vec![wallet_filename_from_record_id(&record_id)],
    );
    globals.cloud.set_wallet_files(
        source_namespace.clone(),
        vec![wallet_filename_from_record_id(&record_id)],
    );

    enqueue_cleanup_for_test(
        &manager,
        &active_namespace,
        &active_master_key,
        CleanupTestSource {
            namespace: source_namespace.clone(),
            record_id,
            revision_hash: Some("matching-revision".into()),
        },
        "cleanup should delete source namespace",
        || !globals.cloud.has_namespace(&source_namespace),
    )
    .await;

    assert!(!globals.cloud.has_namespace(&source_namespace));
}

#[tokio::test(flavor = "current_thread")]
async fn cleanup_keeps_source_namespace_when_active_record_is_missing() {
    let _guard = async_test_lock().lock().await;
    cove_tokio::init();
    let globals = test_globals();
    let manager = init_manager();

    reset_cloud_backup_test_state(&manager, globals);
    let active_master_key = cove_cspp::master_key::MasterKey::generate();
    let active_namespace = active_master_key.namespace_id();
    let source_namespace = "source-namespace".to_string();
    let record_id = "missing-record".to_string();
    let active_namespace_list_attempt_count =
        globals.cloud.list_wallet_files_attempt_count_for_namespace(&active_namespace);
    globals.cloud.set_wallet_files(
        source_namespace.clone(),
        vec![wallet_filename_from_record_id(&record_id)],
    );

    enqueue_cleanup_for_test(
        &manager,
        &active_namespace,
        &active_master_key,
        CleanupTestSource {
            namespace: source_namespace.clone(),
            record_id,
            revision_hash: Some("expected-revision".into()),
        },
        "cleanup should inspect active namespace",
        || {
            globals.cloud.list_wallet_files_attempt_count_for_namespace(&active_namespace)
                > active_namespace_list_attempt_count
        },
    )
    .await;

    assert!(globals.cloud.has_namespace(&source_namespace));
}

#[tokio::test(flavor = "current_thread")]
async fn cleanup_keeps_source_namespace_when_active_record_is_undecryptable() {
    let _guard = async_test_lock().lock().await;
    cove_tokio::init();
    let globals = test_globals();
    let manager = init_manager();

    reset_cloud_backup_test_state(&manager, globals);
    let active_master_key = cove_cspp::master_key::MasterKey::generate();
    let wrong_master_key = cove_cspp::master_key::MasterKey::generate();
    let active_namespace = active_master_key.namespace_id();
    let source_namespace = "source-namespace".to_string();
    let wallet = xpub_only_wallet_metadata();
    Keychain::global().save_wallet_xpub(&wallet.id, sample_xpub(&wallet).parse().unwrap()).unwrap();
    let record_id = cove_cspp::backup_data::wallet_record_id(wallet.id.as_ref());

    globals.cloud.set_wallet_backup(
        active_namespace.clone(),
        record_id.clone(),
        encrypted_wallet_backup_bytes(&wallet, &wrong_master_key, "expected-revision", 1).await,
    );
    globals.cloud.set_wallet_files(
        active_namespace.clone(),
        vec![wallet_filename_from_record_id(&record_id)],
    );
    globals.cloud.set_wallet_files(
        source_namespace.clone(),
        vec![wallet_filename_from_record_id(&record_id)],
    );
    let active_namespace_list_attempt_count =
        globals.cloud.list_wallet_files_attempt_count_for_namespace(&active_namespace);

    enqueue_cleanup_for_test(
        &manager,
        &active_namespace,
        &active_master_key,
        CleanupTestSource {
            namespace: source_namespace.clone(),
            record_id,
            revision_hash: Some("expected-revision".into()),
        },
        "cleanup should inspect active namespace",
        || {
            globals.cloud.list_wallet_files_attempt_count_for_namespace(&active_namespace)
                > active_namespace_list_attempt_count
        },
    )
    .await;

    assert!(globals.cloud.has_namespace(&source_namespace));
}

#[tokio::test(flavor = "current_thread")]
async fn cleanup_keeps_source_namespace_when_active_record_is_unsupported() {
    let _guard = async_test_lock().lock().await;
    cove_tokio::init();
    let globals = test_globals();
    let manager = init_manager();

    reset_cloud_backup_test_state(&manager, globals);
    let active_master_key = cove_cspp::master_key::MasterKey::generate();
    let active_namespace = active_master_key.namespace_id();
    let source_namespace = "source-namespace".to_string();
    let wallet = xpub_only_wallet_metadata();
    Keychain::global().save_wallet_xpub(&wallet.id, sample_xpub(&wallet).parse().unwrap()).unwrap();
    let record_id = cove_cspp::backup_data::wallet_record_id(wallet.id.as_ref());

    globals.cloud.set_wallet_backup(
        active_namespace.clone(),
        record_id.clone(),
        encrypted_wallet_backup_bytes(&wallet, &active_master_key, "expected-revision", 2).await,
    );
    globals.cloud.set_wallet_files(
        active_namespace.clone(),
        vec![wallet_filename_from_record_id(&record_id)],
    );
    globals.cloud.set_wallet_files(
        source_namespace.clone(),
        vec![wallet_filename_from_record_id(&record_id)],
    );
    let active_namespace_list_attempt_count =
        globals.cloud.list_wallet_files_attempt_count_for_namespace(&active_namespace);

    enqueue_cleanup_for_test(
        &manager,
        &active_namespace,
        &active_master_key,
        CleanupTestSource {
            namespace: source_namespace.clone(),
            record_id,
            revision_hash: Some("expected-revision".into()),
        },
        "cleanup should inspect active namespace",
        || {
            globals.cloud.list_wallet_files_attempt_count_for_namespace(&active_namespace)
                > active_namespace_list_attempt_count
        },
    )
    .await;

    assert!(globals.cloud.has_namespace(&source_namespace));
}

#[tokio::test(flavor = "current_thread")]
async fn cleanup_keeps_source_namespace_when_active_revision_mismatches() {
    let _guard = async_test_lock().lock().await;
    cove_tokio::init();
    let globals = test_globals();
    let manager = init_manager();

    reset_cloud_backup_test_state(&manager, globals);
    let active_master_key = cove_cspp::master_key::MasterKey::generate();
    let active_namespace = active_master_key.namespace_id();
    let source_namespace = "source-namespace".to_string();
    let wallet = xpub_only_wallet_metadata();
    Keychain::global().save_wallet_xpub(&wallet.id, sample_xpub(&wallet).parse().unwrap()).unwrap();
    let record_id = cove_cspp::backup_data::wallet_record_id(wallet.id.as_ref());

    globals.cloud.set_wallet_backup(
        active_namespace.clone(),
        record_id.clone(),
        encrypted_wallet_backup_bytes(&wallet, &active_master_key, "actual-revision", 1).await,
    );
    globals.cloud.set_wallet_files(
        active_namespace.clone(),
        vec![wallet_filename_from_record_id(&record_id)],
    );
    globals.cloud.set_wallet_files(
        source_namespace.clone(),
        vec![wallet_filename_from_record_id(&record_id)],
    );
    let active_namespace_list_attempt_count =
        globals.cloud.list_wallet_files_attempt_count_for_namespace(&active_namespace);

    enqueue_cleanup_for_test(
        &manager,
        &active_namespace,
        &active_master_key,
        CleanupTestSource {
            namespace: source_namespace.clone(),
            record_id,
            revision_hash: Some("expected-revision".into()),
        },
        "cleanup should inspect active namespace",
        || {
            globals.cloud.list_wallet_files_attempt_count_for_namespace(&active_namespace)
                > active_namespace_list_attempt_count
        },
    )
    .await;

    assert!(globals.cloud.has_namespace(&source_namespace));
}

#[tokio::test(flavor = "current_thread")]
async fn cleanup_keeps_source_namespace_when_delete_fails() {
    let _guard = async_test_lock().lock().await;
    cove_tokio::init();
    let globals = test_globals();
    let manager = init_manager();

    reset_cloud_backup_test_state(&manager, globals);
    let active_master_key = cove_cspp::master_key::MasterKey::generate();
    let active_namespace = active_master_key.namespace_id();
    let source_namespace = "source-namespace".to_string();
    let wallet = xpub_only_wallet_metadata();
    Keychain::global().save_wallet_xpub(&wallet.id, sample_xpub(&wallet).parse().unwrap()).unwrap();
    let record_id = cove_cspp::backup_data::wallet_record_id(wallet.id.as_ref());

    globals.cloud.set_wallet_backup(
        active_namespace.clone(),
        record_id.clone(),
        encrypted_wallet_backup_bytes(&wallet, &active_master_key, "expected-revision", 1).await,
    );
    globals.cloud.set_wallet_files(
        active_namespace.clone(),
        vec![wallet_filename_from_record_id(&record_id)],
    );
    globals.cloud.set_wallet_files(
        source_namespace.clone(),
        vec![wallet_filename_from_record_id(&record_id)],
    );
    globals.cloud.fail_delete_namespace("delete failed");
    let delete_attempt_count = globals.cloud.delete_namespace_attempt_count();

    enqueue_cleanup_for_test(
        &manager,
        &active_namespace,
        &active_master_key,
        CleanupTestSource {
            namespace: source_namespace.clone(),
            record_id,
            revision_hash: Some("expected-revision".into()),
        },
        "cleanup should attempt source namespace delete",
        || globals.cloud.delete_namespace_attempt_count() > delete_attempt_count,
    )
    .await;

    assert!(globals.cloud.has_namespace(&source_namespace));
}

#[tokio::test(flavor = "current_thread")]
async fn passkey_repair_finalization_keeps_existing_count_when_wallet_refresh_fails() {
    let _guard = async_test_lock().lock().await;
    cove_tokio::init();
    let globals = test_globals();
    let manager = init_manager();
    configure_enabled_cloud_backup(&manager, globals, 2);

    Database::global()
        .cloud_backup_state
        .set(&persisted_passkey_missing_cloud_backup_state(Some(7)))
        .unwrap();
    manager.sync_persisted_state();
    globals.cloud.fail_list_wallet_files("timed out");

    let finalization = manager.prepare_passkey_repair_finalization().await.unwrap();
    manager.apply_passkey_repair_finalization(finalization).unwrap();

    let state = Database::global().cloud_backup_state.get().unwrap();
    assert_eq!(state.status(), PersistedCloudBackupStatus::Enabled);
    assert_eq!(state.wallet_count(), Some(7));
    assert_eq!(manager.model_snapshot().status, CloudBackupStatus::Enabled);
}

#[tokio::test(flavor = "current_thread")]
async fn wrapper_repair_refreshes_missing_master_key_sync_health_to_uploading() {
    let _guard = async_test_lock().lock().await;
    cove_tokio::init();
    let globals = test_globals();
    let manager = init_manager();
    configure_enabled_cloud_backup(&manager, globals, 1);

    let metadata = xpub_only_wallet_metadata();
    persist_xpub_wallets(vec![metadata]);
    globals.passkey.set_create_result(Ok(vec![1, 2, 3]));
    globals.passkey.set_authenticate_result(Ok(vec![7; 32]));
    manager.observe_sync_health(CloudSyncHealth::Failed(
        SYNC_HEALTH_MISSING_MASTER_KEY_MESSAGE.into(),
    ));

    run_repair_passkey_operation(&manager, true).await;

    for _ in 0..20 {
        if manager.model_snapshot().sync_health == CloudSyncHealth::Uploading {
            break;
        }

        tokio::time::sleep(Duration::from_millis(10)).await;
    }

    assert_eq!(manager.model_snapshot().sync_health, CloudSyncHealth::Uploading);

    clear_wallet_upload_runtime_for_test_async(&manager).await;
}

#[tokio::test(flavor = "current_thread")]
async fn wrapper_repair_reports_failure_after_upload_when_passkey_persistence_fails() {
    let _guard = async_test_lock().lock().await;
    cove_tokio::init();
    let globals = test_globals();
    let manager = init_manager();
    configure_enabled_cloud_backup(&manager, globals, 0);

    let namespace = CloudBackupKeychain::global().namespace_id().unwrap();
    globals.passkey.set_create_result(Ok(vec![1, 2, 3]));
    globals.passkey.set_authenticate_result(Ok(vec![7; 32]));
    globals.keychain.fail_save_at(1);

    run_repair_passkey_operation(&manager, true).await;

    assert!(globals.cloud.has_master_key_backup(&namespace));
    assert_eq!(CloudBackupKeychain::global().load_credential_id(), None);
    assert_eq!(CloudBackupKeychain::global().load_prf_salt(), None);
}

#[tokio::test(flavor = "current_thread")]
async fn disable_cloud_backup_deletes_active_namespace_and_clears_local_cloud_state() {
    let _guard = async_test_lock().lock().await;
    cove_tokio::init();
    let globals = test_globals();
    let manager = init_manager();
    configure_enabled_cloud_backup(&manager, globals, 0);

    let namespace = CloudBackupKeychain::global().namespace_id().unwrap();
    globals.cloud.set_master_key_backup(namespace.clone(), vec![1, 2, 3]);
    Database::global()
        .cloud_blob_sync_states
        .set(&PersistedCloudBlobSyncState::master_key_wrapper(
            namespace.clone(),
            PersistedCloudBlobState::Dirty(CloudBlobDirtyState { changed_at: 1 }),
        ))
        .unwrap();

    run_disable_cloud_backup(&manager).await;

    assert!(!globals.cloud.has_namespace(&namespace));
    assert_eq!(
        Database::global().cloud_backup_state.get().unwrap().status(),
        PersistedCloudBackupStatus::Disabled
    );
    assert!(Database::global().cloud_blob_sync_states.list().unwrap().is_empty());
    assert!(CloudBackupKeychain::global().namespace_id().is_none());
    assert!(
        globals.cloud.deleted_namespace_policies().contains(&CloudAccessPolicy::ConsentAllowed)
    );
}

#[tokio::test(flavor = "current_thread")]
async fn disable_cloud_backup_keeps_disabling_state_when_local_cleanup_fails() {
    let _guard = async_test_lock().lock().await;
    cove_tokio::init();
    let globals = test_globals();
    let manager = init_manager();
    configure_enabled_cloud_backup(&manager, globals, 0);

    let namespace = CloudBackupKeychain::global().namespace_id().unwrap();
    globals.cloud.set_master_key_backup(namespace.clone(), vec![1, 2, 3]);
    globals.keychain.fail_delete_at(1);

    run_disable_cloud_backup(&manager).await;
    let error = disable_failure_message(&manager);

    assert!(error.contains("clear cloud backup local keychain state"), "{error}");
    assert!(!globals.cloud.has_namespace(&namespace));
    let PersistedCloudBackupState::Disabling(disabling) =
        Database::global().cloud_backup_state.get().unwrap()
    else {
        panic!("expected persisted disabling state");
    };
    assert!(disabling.delete_started_at.is_some());
    assert!(
        disabling
            .last_error
            .as_deref()
            .is_some_and(|message| message.contains("clear cloud backup local keychain state"))
    );
    assert_eq!(current_disable_generation(), Some(disabling.disable_generation));

    run_disable_cloud_backup(&manager).await;

    assert_eq!(
        Database::global().cloud_backup_state.get().unwrap().status(),
        PersistedCloudBackupStatus::Disabled
    );
    assert!(current_disable_generation().is_none());
}

#[tokio::test(flavor = "current_thread")]
async fn disable_cloud_backup_blocks_cloud_only_wallets_without_deleting_namespace() {
    let _guard = async_test_lock().lock().await;
    cove_tokio::init();
    let globals = test_globals();
    let manager = init_manager();
    configure_enabled_cloud_backup(&manager, globals, 0);

    let namespace = CloudBackupKeychain::global().namespace_id().unwrap();
    globals.cloud.set_master_key_backup(namespace.clone(), vec![1, 2, 3]);
    globals
        .cloud
        .set_wallet_files(namespace.clone(), vec![wallet_filename_from_record_id("cloud-only")]);

    run_disable_cloud_backup(&manager).await;
    let error = disable_failure_message(&manager);

    assert!(error.contains("cloud-only wallets"), "{error}");
    assert!(globals.cloud.has_namespace(&namespace));
    let PersistedCloudBackupState::Disabling(disabling) =
        Database::global().cloud_backup_state.get().unwrap()
    else {
        panic!("expected persisted disabling state");
    };
    assert!(disabling.delete_started_at.is_none());
    assert!(current_disable_generation().is_some());
}

#[tokio::test(flavor = "current_thread")]
async fn disable_cloud_backup_uses_unique_generation_for_each_attempt() {
    let _guard = async_test_lock().lock().await;
    cove_tokio::init();
    let globals = test_globals();
    let manager = init_manager();
    configure_enabled_cloud_backup(&manager, globals, 0);

    let namespace = CloudBackupKeychain::global().namespace_id().unwrap();
    globals.cloud.set_master_key_backup(namespace.clone(), vec![1, 2, 3]);
    globals
        .cloud
        .set_wallet_files(namespace.clone(), vec![wallet_filename_from_record_id("cloud-only")]);

    run_disable_cloud_backup(&manager).await;
    let first_generation = current_disable_generation().unwrap();
    run_keep_cloud_backup_enabled(&manager).await;

    run_disable_cloud_backup(&manager).await;
    let second_generation = current_disable_generation().unwrap();

    assert_ne!(first_generation, second_generation);
}

#[tokio::test(flavor = "current_thread")]
async fn keep_cloud_backup_enabled_clears_rolled_back_disable_failure() {
    let _guard = async_test_lock().lock().await;
    cove_tokio::init();
    let globals = test_globals();
    let manager = init_manager();
    configure_enabled_cloud_backup(&manager, globals, 0);

    let namespace = CloudBackupKeychain::global().namespace_id().unwrap();
    globals.cloud.set_master_key_backup(namespace.clone(), vec![1, 2, 3]);
    globals
        .cloud
        .set_wallet_files(namespace.clone(), vec![wallet_filename_from_record_id("cloud-only")]);

    run_disable_cloud_backup(&manager).await;

    assert!(globals.cloud.has_namespace(&namespace));
    let PersistedCloudBackupState::Disabling(disabling) =
        Database::global().cloud_backup_state.get().unwrap()
    else {
        panic!("expected persisted disabling state");
    };
    assert!(disabling.delete_started_at.is_none());
    assert_eq!(current_disable_generation(), Some(disabling.disable_generation));
    let CloudBackupLifecycle::Configured(configured) = manager.state().lifecycle else {
        panic!("expected configured cloud backup lifecycle");
    };
    assert!(matches!(
        configured.destructive_operation,
        CloudBackupDestructiveOperationState::DisableFailed { can_keep_enabled: true, .. }
    ));

    run_keep_cloud_backup_enabled(&manager).await;

    let CloudBackupLifecycle::Configured(configured) = manager.state().lifecycle else {
        panic!("expected configured cloud backup lifecycle");
    };
    assert_eq!(configured.destructive_operation, CloudBackupDestructiveOperationState::Idle);
    assert!(globals.cloud.has_namespace(&namespace));
    assert!(current_disable_generation().is_none());
}

#[tokio::test(flavor = "current_thread")]
async fn keep_cloud_backup_enabled_preserves_configured_runtime_status() {
    let _guard = async_test_lock().lock().await;
    cove_tokio::init();
    let globals = test_globals();
    let manager = init_manager();
    configure_enabled_cloud_backup(&manager, globals, 3);

    Database::global()
        .cloud_backup_state
        .set(&persisted_passkey_missing_cloud_backup_state(Some(3)))
        .unwrap();
    manager.reconcile_runtime_status(CloudBackupStatus::Enabled);
    manager.apply_disable_outcome(CloudBackupDisableOutcome::Failed {
        message: "disable failed".into(),
        can_keep_enabled: true,
    });

    manager.finish_keep_cloud_backup_enabled();

    assert_eq!(manager.current_status(), CloudBackupStatus::PasskeyMissing);
    let CloudBackupLifecycle::Configured(configured) = manager.state().lifecycle else {
        panic!("expected configured cloud backup lifecycle");
    };
    assert_eq!(configured.destructive_operation, CloudBackupDestructiveOperationState::Idle);
}

#[tokio::test(flavor = "current_thread")]
async fn keep_cloud_backup_enabled_ignores_stale_disabling_generation() {
    let _guard = async_test_lock().lock().await;
    cove_tokio::init();
    let globals = test_globals();
    let manager = init_manager();
    configure_enabled_cloud_backup(&manager, globals, 0);

    let namespace_id = CloudBackupKeychain::global().namespace_id().unwrap();
    let PersistedCloudBackupState::Configured(previous_configured) =
        Database::global().cloud_backup_state.get().unwrap()
    else {
        panic!("expected configured state");
    };

    let stale = PersistedDisablingCloudBackup {
        previous_configured: previous_configured.clone(),
        namespace_id: namespace_id.clone(),
        disable_generation: 10,
        started_at: 100,
        delete_started_at: None,
        last_error: None,
        retry_after: None,
    };
    let current = PersistedDisablingCloudBackup { disable_generation: 11, ..stale.clone() };
    Database::global()
        .cloud_backup_state
        .set(&PersistedCloudBackupState::Disabling(current))
        .unwrap();

    let restored = manager.restore_configured_cloud_backup_after_disable(&stale).unwrap();

    assert!(!restored);
    let PersistedCloudBackupState::Disabling(disabling) =
        Database::global().cloud_backup_state.get().unwrap()
    else {
        panic!("expected disabling state");
    };
    assert_eq!(disabling.disable_generation, 11);
}

#[tokio::test(flavor = "current_thread")]
async fn disable_cloud_backup_blocks_other_namespaces_without_deleting_them() {
    let _guard = async_test_lock().lock().await;
    cove_tokio::init();
    let globals = test_globals();
    let manager = init_manager();
    configure_enabled_cloud_backup(&manager, globals, 0);

    let namespace = CloudBackupKeychain::global().namespace_id().unwrap();
    let other_namespace = cove_cspp::master_key::MasterKey::generate().namespace_id();
    globals.cloud.set_master_key_backup(namespace.clone(), vec![1, 2, 3]);
    globals.cloud.set_master_key_backup(other_namespace.clone(), vec![4, 5, 6]);
    let delete_attempt_count = globals.cloud.delete_namespace_attempt_count();

    run_disable_cloud_backup(&manager).await;
    let error = disable_failure_message(&manager);

    assert!(error.contains("other cloud backups"), "{error}");
    assert!(globals.cloud.has_namespace(&namespace));
    assert!(globals.cloud.has_namespace(&other_namespace));
    assert_eq!(globals.cloud.delete_namespace_attempt_count(), delete_attempt_count);
    let PersistedCloudBackupState::Disabling(disabling) =
        Database::global().cloud_backup_state.get().unwrap()
    else {
        panic!("expected persisted disabling state");
    };
    assert!(disabling.delete_started_at.is_none());
}

#[tokio::test(flavor = "current_thread")]
async fn disable_cloud_backup_blocks_active_exclusive_operation_without_persisting_disabling() {
    let _guard = async_test_lock().lock().await;
    cove_tokio::init();
    let globals = test_globals();
    let manager = init_manager();
    configure_enabled_cloud_backup(&manager, globals, 0);

    let namespace = CloudBackupKeychain::global().namespace_id().unwrap();
    globals.cloud.set_master_key_backup(namespace.clone(), vec![1, 2, 3]);
    let claim =
        CloudBackupExclusiveOperationClaim::new(CloudBackupExclusiveOperation::RecreateManifest, 1);
    manager.project_exclusive_operation_started(claim);

    let error = manager.prepare_disable_cloud_backup().await.unwrap_err();

    assert!(error.to_string().contains("another cloud backup operation"), "{error}");
    assert!(globals.cloud.has_namespace(&namespace));
    assert_eq!(
        Database::global().cloud_backup_state.get().unwrap().status(),
        PersistedCloudBackupStatus::Enabled
    );
    assert_eq!(
        manager.projected_exclusive_operation().map(|claim| claim.operation()),
        Some(CloudBackupExclusiveOperation::RecreateManifest)
    );
    manager.project_exclusive_operation_finished(claim);
}

#[tokio::test(flavor = "current_thread")]
async fn disable_cloud_backup_delete_failure_keeps_disabling_state_and_keychain() {
    let _guard = async_test_lock().lock().await;
    cove_tokio::init();
    let globals = test_globals();
    let manager = init_manager();
    configure_enabled_cloud_backup(&manager, globals, 0);

    let namespace = CloudBackupKeychain::global().namespace_id().unwrap();
    globals.cloud.set_master_key_backup(namespace.clone(), vec![1, 2, 3]);
    globals.cloud.fail_delete_namespace("delete failed");

    run_disable_cloud_backup(&manager).await;
    let error = disable_failure_message(&manager);

    assert!(error.contains("delete failed"), "{error}");
    let PersistedCloudBackupState::Disabling(disabling) =
        Database::global().cloud_backup_state.get().unwrap()
    else {
        panic!("expected persisted disabling state");
    };
    assert!(disabling.delete_started_at.is_some());
    assert!(
        disabling.last_error.as_deref().is_some_and(|message| message.contains("delete failed"))
    );
    assert_eq!(CloudBackupKeychain::global().namespace_id().as_deref(), Some(namespace.as_str()));
    assert_eq!(current_disable_generation(), Some(disabling.disable_generation));
}

#[tokio::test(flavor = "current_thread")]
async fn disable_cloud_backup_not_found_listing_retries_then_finishes_cleanup() {
    let _guard = async_test_lock().lock().await;
    cove_tokio::init();
    let globals = test_globals();
    let manager = init_manager();
    configure_enabled_cloud_backup(&manager, globals, 0);

    let namespace = CloudBackupKeychain::global().namespace_id().unwrap();
    globals.cloud.set_master_key_backup(namespace.clone(), vec![1, 2, 3]);
    globals.cloud.fail_list_wallet_files_for_namespace(
        namespace.clone(),
        CloudStorageError::NotFound("missing".into()),
    );

    run_disable_cloud_backup(&manager).await;

    assert_eq!(globals.cloud.list_wallet_files_attempt_count_for_namespace(&namespace), 4);
    assert_eq!(
        Database::global().cloud_backup_state.get().unwrap().status(),
        PersistedCloudBackupStatus::Disabled
    );
    assert!(CloudBackupKeychain::global().namespace_id().is_none());
}

#[tokio::test(flavor = "current_thread")]
async fn disable_cloud_backup_delete_not_found_finishes_cleanup() {
    let _guard = async_test_lock().lock().await;
    cove_tokio::init();
    let globals = test_globals();
    let manager = init_manager();
    configure_enabled_cloud_backup(&manager, globals, 0);

    let namespace = CloudBackupKeychain::global().namespace_id().unwrap();
    globals.cloud.set_master_key_backup(namespace.clone(), vec![1, 2, 3]);
    globals.cloud.fail_delete_namespace_not_found("already deleted");

    run_disable_cloud_backup(&manager).await;

    assert_eq!(
        Database::global().cloud_backup_state.get().unwrap().status(),
        PersistedCloudBackupStatus::Disabled
    );
    assert!(CloudBackupKeychain::global().namespace_id().is_none());
}

#[tokio::test(flavor = "current_thread")]
async fn persisted_disabling_on_restart_resumes_to_disabled() {
    let _guard = async_test_lock().lock().await;
    cove_tokio::init();
    let globals = test_globals();
    let manager = init_manager();
    configure_enabled_cloud_backup(&manager, globals, 0);

    let namespace = CloudBackupKeychain::global().namespace_id().unwrap();
    globals.cloud.set_master_key_backup(namespace.clone(), vec![1, 2, 3]);
    let PersistedCloudBackupState::Configured(previous_configured) =
        Database::global().cloud_backup_state.get().unwrap()
    else {
        panic!("expected configured cloud backup state");
    };

    Database::global()
        .cloud_backup_state
        .set(&PersistedCloudBackupState::Disabling(PersistedDisablingCloudBackup {
            previous_configured,
            namespace_id: namespace.clone(),
            disable_generation: 42,
            started_at: 41,
            delete_started_at: Some(43),
            last_error: Some("interrupted".into()),
            retry_after: Some(44),
        }))
        .unwrap();

    let restarted_manager = init_manager();
    for _ in 0..20 {
        if Database::global().cloud_backup_state.get().unwrap().status()
            == PersistedCloudBackupStatus::Disabled
        {
            break;
        }

        tokio::time::sleep(Duration::from_millis(10)).await;
    }

    assert!(!globals.cloud.has_namespace(&namespace));
    assert_eq!(
        Database::global().cloud_backup_state.get().unwrap().status(),
        PersistedCloudBackupStatus::Disabled
    );
    assert!(CloudBackupKeychain::global().namespace_id().is_none());
    assert_eq!(restarted_manager.model_snapshot().status, CloudBackupStatus::Disabled);
}

#[tokio::test(flavor = "current_thread")]
async fn keep_cloud_backup_enabled_after_delete_failure_requires_existing_namespace() {
    let _guard = async_test_lock().lock().await;
    cove_tokio::init();
    let globals = test_globals();
    let manager = init_manager();
    configure_enabled_cloud_backup(&manager, globals, 0);

    let namespace = CloudBackupKeychain::global().namespace_id().unwrap();
    globals.cloud.set_master_key_backup(namespace.clone(), vec![1, 2, 3]);
    globals.cloud.fail_delete_namespace("delete failed");

    run_disable_cloud_backup(&manager).await;
    let error = disable_failure_message(&manager);
    assert!(error.contains("delete failed"), "{error}");

    run_keep_cloud_backup_enabled(&manager).await;

    assert_eq!(
        Database::global().cloud_backup_state.get().unwrap().status(),
        PersistedCloudBackupStatus::Enabled
    );
    assert_eq!(CloudBackupKeychain::global().namespace_id().as_deref(), Some(namespace.as_str()));
    assert!(current_disable_generation().is_none());
}

#[tokio::test(flavor = "current_thread")]
async fn reupload_all_wallets_does_not_create_master_key_for_existing_namespace() {
    let _guard = async_test_lock().lock().await;
    cove_tokio::init();
    let globals = test_globals();
    globals.reset();

    CloudBackupKeychain::global().save_namespace_id("existing-namespace").unwrap();

    let manager = init_manager();
    let claim =
        CloudBackupExclusiveOperationClaim::new(CloudBackupExclusiveOperation::RecreateManifest, 1);
    let writes = operation_write_client_for_test(&manager, claim);
    let error = manager.prepare_reupload_all_wallets(writes).await.unwrap_err();

    assert!(matches!(
        error,
        CloudBackupError::RecoveryRequired(message)
            if message == RECREATE_MANIFEST_RECOVERY_MESSAGE
    ));

    let cspp = cove_cspp::Cspp::new(Keychain::global().clone());
    assert!(cspp.load_master_key_from_store().unwrap().is_none());
}

#[tokio::test(flavor = "current_thread")]
async fn reupload_all_wallets_persists_full_cloud_wallet_count() {
    let _guard = async_test_lock().lock().await;
    cove_tokio::init();
    let globals = test_globals();
    let manager = init_manager();
    configure_enabled_cloud_backup(&manager, globals, 0);

    let metadata = xpub_only_wallet_metadata();
    persist_xpub_wallets(vec![metadata]);

    let namespace = CloudBackupKeychain::global().namespace_id().unwrap();
    globals.cloud.set_reflect_uploaded_wallets_in_listing(true);
    globals
        .cloud
        .set_wallet_files(namespace, vec![wallet_filename_from_record_id("cloud-only-record")]);

    run_recreate_manifest(&manager).await;

    assert_eq!(Database::global().cloud_backup_state.get().unwrap().wallet_count(), Some(2));
    clear_wallet_upload_runtime_for_test_async(&manager).await;
}

#[tokio::test(flavor = "current_thread")]
async fn fetch_cloud_only_wallets_surfaces_unsupported_versions() {
    let _guard = async_test_lock().lock().await;
    cove_tokio::init();
    let globals = test_globals();
    let manager = init_manager();
    configure_enabled_cloud_backup(&manager, globals, 0);

    let metadata = xpub_only_wallet_metadata();
    let keychain = Keychain::global();
    let namespace = CloudBackupKeychain::new(keychain.clone()).namespace_id().unwrap();
    let master_key =
        cove_cspp::Cspp::new(keychain.clone()).load_master_key_from_store().unwrap().unwrap();
    let record_id = cove_cspp::backup_data::wallet_record_id(metadata.id.as_ref());
    globals.cloud.set_wallet_backup(
        namespace.clone(),
        record_id.clone(),
        encrypted_wallet_backup_bytes(&metadata, &master_key, "unsupported-revision", 2).await,
    );
    globals.cloud.set_wallet_files(namespace, vec![wallet_filename_from_record_id(&record_id)]);

    let wallets = manager.do_fetch_cloud_only_wallets().await.unwrap();

    assert_eq!(wallets.len(), 1);
    assert_eq!(wallets[0].record_id, record_id);
    assert_eq!(wallets[0].name, UNSUPPORTED_CLOUD_ONLY_WALLET_NAME);
    assert_eq!(wallets[0].sync_status, CloudBackupWalletStatus::UnsupportedVersion);
    assert_eq!(wallets[0].network, None);
    assert_eq!(wallets[0].wallet_mode, None);
    assert_eq!(wallets[0].wallet_type, None);
    assert_eq!(wallets[0].label_count, None);
    assert_eq!(wallets[0].backup_updated_at, None);
}

#[tokio::test(flavor = "current_thread")]
async fn detail_reports_other_backup_namespaces() {
    let _guard = async_test_lock().lock().await;
    cove_tokio::init();
    let globals = test_globals();
    let manager = init_manager();
    configure_enabled_cloud_backup(&manager, globals, 0);

    let current_namespace = CloudBackupKeychain::global().namespace_id().unwrap();
    globals.cloud.set_master_key_backup(current_namespace.clone(), vec![1, 2, 3]);
    globals.cloud.set_wallet_files(
        current_namespace,
        vec![wallet_filename_from_record_id("current-wallet")],
    );

    let other_master_key = cove_cspp::master_key::MasterKey::generate();
    let other_namespace = other_master_key.namespace_id();
    let encrypted_master = cove_cspp::master_key_crypto::encrypt_master_key_with_provider_hint(
        &other_master_key,
        &[7; 32],
        &[9; 32],
        Some(cove_cspp::backup_data::PasskeyProviderHint {
            aaguid: "ea9b8d66-4d01-1d21-3ce4-b6b48cb575d4".into(),
            registered_platform: cove_cspp::backup_data::PasskeyRegistrationPlatform::Android,
            registered_at: 1_777_661_234,
            name_suffix: "09IX".into(),
        }),
    )
    .unwrap();
    globals.cloud.set_master_key_backup(
        other_namespace.clone(),
        serde_json::to_vec(&encrypted_master).unwrap(),
    );
    globals.cloud.set_wallet_files(
        other_namespace,
        vec![
            wallet_filename_from_record_id("other-wallet-1"),
            wallet_filename_from_record_id("other-wallet-2"),
        ],
    );

    let Some(CloudBackupDetailResult::Success(detail)) =
        manager.refresh_cloud_backup_detail().await
    else {
        panic!("expected cloud backup detail");
    };

    let CloudBackupOtherBackupsState::Loaded { summary } = detail.other_backups else {
        panic!("expected loaded other backups");
    };
    assert_eq!(summary.namespace_count, 1);
    assert_eq!(summary.wallet_count, 2);
    assert_eq!(summary.passkey_hints.len(), 1);
    assert_eq!(summary.passkey_hints[0].name_suffix, "09IX");
    assert_eq!(summary.passkey_hints[0].provider_name.as_deref(), Some("Google Password Manager"));
}

#[tokio::test(flavor = "current_thread")]
async fn other_backup_summary_counts_only_wallets_missing_from_local_wallets() {
    let _guard = async_test_lock().lock().await;
    cove_tokio::init();
    let globals = test_globals();
    let manager = init_manager();
    configure_enabled_cloud_backup(&manager, globals, 0);

    let metadata = xpub_only_wallet_metadata();
    let local_record_id = cove_cspp::backup_data::wallet_record_id(metadata.id.as_ref());
    persist_xpub_wallets(vec![metadata]);

    let current_namespace = CloudBackupKeychain::global().namespace_id().unwrap();
    globals.cloud.set_wallet_files(current_namespace.clone(), Vec::new());

    let other_master_key = cove_cspp::master_key::MasterKey::generate();
    let other_namespace = other_master_key.namespace_id();
    globals.cloud.set_master_key_backup(other_namespace.clone(), vec![1, 2, 3]);
    globals.cloud.set_wallet_files(
        other_namespace,
        vec![
            wallet_filename_from_record_id(&local_record_id),
            wallet_filename_from_record_id("missing-local-wallet"),
        ],
    );

    let summary =
        manager.other_backup_summary(&CloudStorage::global_explicit_client()).await.unwrap();
    assert_eq!(summary.namespace_count, 1);
    assert_eq!(summary.wallet_count, 1);

    globals.cloud.set_wallet_files(
        current_namespace,
        vec![wallet_filename_from_record_id("missing-local-wallet")],
    );

    let summary =
        manager.other_backup_summary(&CloudStorage::global_explicit_client()).await.unwrap();
    assert_eq!(summary.namespace_count, 1);
    assert_eq!(summary.wallet_count, 1);
}

#[tokio::test(flavor = "current_thread")]
async fn other_backup_summary_counts_empty_namespace_when_wallet_listing_is_missing() {
    let _guard = async_test_lock().lock().await;
    cove_tokio::init();
    let globals = test_globals();
    let manager = init_manager();
    configure_enabled_cloud_backup(&manager, globals, 0);

    let other_master_key = cove_cspp::master_key::MasterKey::generate();
    let other_namespace = other_master_key.namespace_id();
    globals.cloud.set_master_key_backup(other_namespace.clone(), vec![1, 2, 3]);
    globals.cloud.fail_list_wallet_files_for_namespace(
        other_namespace,
        CloudStorageError::NotFound("wallet files missing".into()),
    );

    let summary =
        manager.other_backup_summary(&CloudStorage::global_explicit_client()).await.unwrap();
    assert_eq!(summary.namespace_count, 1);
    assert_eq!(summary.wallet_count, 0);
}

#[tokio::test(flavor = "current_thread")]
async fn detail_refresh_marks_other_backups_failed_when_namespace_inspection_fails() {
    let _guard = async_test_lock().lock().await;
    cove_tokio::init();
    let globals = test_globals();
    let manager = init_manager();
    configure_enabled_cloud_backup(&manager, globals, 0);

    let current_namespace = CloudBackupKeychain::global().namespace_id().unwrap();
    globals.cloud.set_wallet_files(current_namespace, Vec::new());

    let other_master_key = cove_cspp::master_key::MasterKey::generate();
    let other_namespace = other_master_key.namespace_id();
    globals.cloud.set_master_key_backup(other_namespace.clone(), vec![1, 2, 3]);
    globals
        .cloud
        .fail_master_key_download_offline(other_namespace, "offline while inspecting namespace");

    let Some(CloudBackupDetailResult::Success(detail)) =
        manager.refresh_cloud_backup_detail().await
    else {
        panic!("expected cloud backup detail");
    };

    let CloudBackupOtherBackupsState::LoadFailed { error } = detail.other_backups else {
        panic!("expected failed other backups state");
    };
    assert_eq!(
        error,
        "offline: Reconnect to the internet, then try refreshing cloud backup details again"
    );
}

#[tokio::test(flavor = "current_thread")]
async fn recover_other_backups_keeps_current_passkey_metadata() {
    let _guard = async_test_lock().lock().await;
    cove_tokio::init();
    let globals = test_globals();
    let manager = init_manager();
    configure_enabled_cloud_backup(&manager, globals, 0);
    globals.cloud.set_reflect_uploaded_wallets_in_listing(true);

    let current_namespace = CloudBackupKeychain::global().namespace_id().unwrap();
    CloudBackupKeychain::global()
        .save_passkey_and_namespace(&[9, 8, 7], [6; 32], &current_namespace)
        .unwrap();

    let prf_key = [7u8; 32];
    let other_master_key = cove_cspp::master_key::MasterKey::generate();
    let other_namespace = other_master_key.namespace_id();
    let encrypted_master =
        cove_cspp::master_key_crypto::encrypt_master_key(&other_master_key, &prf_key, &[9; 32])
            .unwrap();
    globals.cloud.set_master_key_backup(
        other_namespace.clone(),
        serde_json::to_vec(&encrypted_master).unwrap(),
    );
    globals.passkey.set_discover_result(Ok(DiscoveredPasskeyResult {
        prf_output: prf_key.to_vec(),
        credential_id: vec![1, 2, 3],
    }));

    let wallet = xpub_only_wallet_metadata();
    Keychain::global().save_wallet_xpub(&wallet.id, sample_xpub(&wallet).parse().unwrap()).unwrap();
    let record_id = cove_cspp::backup_data::wallet_record_id(wallet.id.as_ref());
    globals.cloud.set_wallet_backup(
        other_namespace.clone(),
        record_id.clone(),
        encrypted_wallet_backup_bytes(&wallet, &other_master_key, "other-revision", 1).await,
    );
    globals.cloud.set_wallet_files(
        other_namespace.clone(),
        vec![wallet_filename_from_record_id(&record_id)],
    );

    let current_namespace_list_attempt_count =
        globals.cloud.list_wallet_files_attempt_count_for_namespace(&current_namespace);
    let report = manager.do_recover_other_backups().await.unwrap();

    assert_eq!(report.wallets_restored, 1);
    assert_eq!(report.wallets_failed, 0);
    assert_eq!(
        globals.cloud.list_wallet_files_attempt_count_for_namespace(&current_namespace),
        current_namespace_list_attempt_count + 3,
    );
    assert_eq!(globals.cloud.uploaded_wallet_backup_count(), 1);
    assert!(!globals.cloud.has_namespace(&other_namespace));
    assert_eq!(CloudBackupKeychain::global().namespace_id(), Some(current_namespace.clone()));
    assert_eq!(
        globals.keychain.get_entry(CSPP_NAMESPACE_ID_KEY).as_deref(),
        Some(current_namespace.as_str())
    );
    assert_eq!(CloudBackupKeychain::global().load_credential_id(), Some(vec![9, 8, 7]));
    assert!(globals.keychain.get_entry(CSPP_PRF_SALT_KEY).is_some());

    let summary =
        manager.other_backup_summary(&CloudStorage::global_explicit_client()).await.unwrap();
    assert_eq!(summary.namespace_count, 0);
    assert_eq!(summary.wallet_count, 0);
}

#[tokio::test(flavor = "current_thread")]
async fn recover_other_backups_keeps_partially_moved_namespace() {
    let _guard = async_test_lock().lock().await;
    cove_tokio::init();
    let globals = test_globals();
    let manager = init_manager();
    configure_enabled_cloud_backup(&manager, globals, 0);
    globals.cloud.set_reflect_uploaded_wallets_in_listing(true);

    let prf_key = [7u8; 32];
    let other_master_key = cove_cspp::master_key::MasterKey::generate();
    let other_namespace = other_master_key.namespace_id();
    let encrypted_master =
        cove_cspp::master_key_crypto::encrypt_master_key(&other_master_key, &prf_key, &[9; 32])
            .unwrap();
    globals.cloud.set_master_key_backup(
        other_namespace.clone(),
        serde_json::to_vec(&encrypted_master).unwrap(),
    );
    globals.passkey.set_discover_result(Ok(DiscoveredPasskeyResult {
        prf_output: prf_key.to_vec(),
        credential_id: vec![1, 2, 3],
    }));

    let restored_wallet = xpub_only_wallet_metadata();
    Keychain::global()
        .save_wallet_xpub(&restored_wallet.id, sample_xpub(&restored_wallet).parse().unwrap())
        .unwrap();
    let restored_record_id = cove_cspp::backup_data::wallet_record_id(restored_wallet.id.as_ref());
    let missing_wallet = xpub_only_wallet_metadata();
    let missing_record_id = cove_cspp::backup_data::wallet_record_id(missing_wallet.id.as_ref());

    globals.cloud.set_wallet_backup(
        other_namespace.clone(),
        restored_record_id.clone(),
        encrypted_wallet_backup_bytes(&restored_wallet, &other_master_key, "other-revision", 1)
            .await,
    );
    globals.cloud.set_wallet_files(
        other_namespace.clone(),
        vec![
            wallet_filename_from_record_id(&restored_record_id),
            wallet_filename_from_record_id(&missing_record_id),
        ],
    );

    let report = manager.do_recover_other_backups().await.unwrap();

    assert_eq!(report.wallets_restored, 1);
    assert_eq!(report.wallets_failed, 1);
    assert!(globals.cloud.has_namespace(&other_namespace));

    let summary =
        manager.other_backup_summary(&CloudStorage::global_explicit_client()).await.unwrap();
    assert_eq!(summary.namespace_count, 1);
    assert_eq!(summary.wallet_count, 1);
}

#[tokio::test(flavor = "current_thread")]
async fn recover_other_backups_keeps_namespace_when_current_upload_fails() {
    let _guard = async_test_lock().lock().await;
    cove_tokio::init();
    let globals = test_globals();
    let manager = init_manager();
    configure_enabled_cloud_backup(&manager, globals, 0);
    globals.cloud.fail_wallet_backup_upload("upload failed");

    let prf_key = [7u8; 32];
    let other_master_key = cove_cspp::master_key::MasterKey::generate();
    let other_namespace = other_master_key.namespace_id();
    let encrypted_master =
        cove_cspp::master_key_crypto::encrypt_master_key(&other_master_key, &prf_key, &[9; 32])
            .unwrap();
    globals.cloud.set_master_key_backup(
        other_namespace.clone(),
        serde_json::to_vec(&encrypted_master).unwrap(),
    );
    globals.passkey.set_discover_result(Ok(DiscoveredPasskeyResult {
        prf_output: prf_key.to_vec(),
        credential_id: vec![1, 2, 3],
    }));

    let wallet = xpub_only_wallet_metadata();
    Keychain::global().save_wallet_xpub(&wallet.id, sample_xpub(&wallet).parse().unwrap()).unwrap();
    let record_id = cove_cspp::backup_data::wallet_record_id(wallet.id.as_ref());
    globals.cloud.set_wallet_backup(
        other_namespace.clone(),
        record_id.clone(),
        encrypted_wallet_backup_bytes(&wallet, &other_master_key, "other-revision", 1).await,
    );
    globals.cloud.set_wallet_files(
        other_namespace.clone(),
        vec![wallet_filename_from_record_id(&record_id)],
    );

    let result = manager.do_recover_other_backups().await;

    assert!(result.is_err());
    assert!(globals.cloud.has_namespace(&other_namespace));
}

#[tokio::test(flavor = "current_thread")]
async fn recover_other_backups_returns_offline_when_wallet_download_is_offline() {
    let _guard = async_test_lock().lock().await;
    cove_tokio::init();
    let globals = test_globals();
    let manager = init_manager();
    configure_enabled_cloud_backup(&manager, globals, 0);

    let prf_key = [7u8; 32];
    let other_master_key = cove_cspp::master_key::MasterKey::generate();
    let other_namespace = other_master_key.namespace_id();
    let encrypted_master =
        cove_cspp::master_key_crypto::encrypt_master_key(&other_master_key, &prf_key, &[9; 32])
            .unwrap();
    globals.cloud.set_master_key_backup(
        other_namespace.clone(),
        serde_json::to_vec(&encrypted_master).unwrap(),
    );
    globals.passkey.set_discover_result(Ok(DiscoveredPasskeyResult {
        prf_output: prf_key.to_vec(),
        credential_id: vec![1, 2, 3],
    }));

    let wallet = xpub_only_wallet_metadata();
    let record_id = cove_cspp::backup_data::wallet_record_id(wallet.id.as_ref());
    globals.cloud.set_wallet_files(
        other_namespace.clone(),
        vec![wallet_filename_from_record_id(&record_id)],
    );
    globals.cloud.fail_wallet_backup_download_offline(
        other_namespace,
        record_id,
        "offline while downloading wallet",
    );

    let result = manager.do_recover_other_backups().await;

    match result {
        Err(CloudBackupError::Offline(message)) => {
            assert_eq!(
                message,
                "Reconnect to the internet, then try recovering the other cloud backups again"
            );
        }
        Ok(report) => panic!(
            "expected offline error, got report with {} failed wallet(s)",
            report.wallets_failed
        ),
        Err(error) => panic!("expected offline error, got {error:?}"),
    }
}

#[tokio::test(flavor = "current_thread")]
async fn recover_other_backups_returns_offline_when_namespace_inspection_is_offline() {
    let _guard = async_test_lock().lock().await;
    cove_tokio::init();
    let globals = test_globals();
    let manager = init_manager();
    configure_enabled_cloud_backup(&manager, globals, 0);

    let other_master_key = cove_cspp::master_key::MasterKey::generate();
    let other_namespace = other_master_key.namespace_id();
    let encrypted_master =
        cove_cspp::master_key_crypto::encrypt_master_key(&other_master_key, &[7; 32], &[9; 32])
            .unwrap();
    globals.cloud.set_master_key_backup(
        other_namespace.clone(),
        serde_json::to_vec(&encrypted_master).unwrap(),
    );
    globals
        .cloud
        .fail_master_key_download_offline(other_namespace, "offline while inspecting namespace");

    let result = manager.do_recover_other_backups().await;

    match result {
        Err(CloudBackupError::Offline(message)) => {
            assert_eq!(
                message,
                "Reconnect to the internet, then try recovering the other cloud backups again"
            );
        }
        Ok(report) => panic!(
            "expected offline error, got report with {} restored wallet(s)",
            report.wallets_restored
        ),
        Err(error) => panic!("expected offline error, got {error:?}"),
    }
}

#[tokio::test(flavor = "current_thread")]
async fn delete_other_backups_removes_only_non_current_namespaces() {
    let _guard = async_test_lock().lock().await;
    cove_tokio::init();
    let globals = test_globals();
    let manager = init_manager();
    configure_enabled_cloud_backup(&manager, globals, 0);

    let current_namespace = CloudBackupKeychain::global().namespace_id().unwrap();
    globals.cloud.set_master_key_backup(current_namespace.clone(), vec![1, 2, 3]);
    globals.cloud.set_wallet_files(
        current_namespace.clone(),
        vec![wallet_filename_from_record_id("current-wallet")],
    );

    let other_master_key = cove_cspp::master_key::MasterKey::generate();
    let other_namespace = other_master_key.namespace_id();
    let encrypted_master =
        cove_cspp::master_key_crypto::encrypt_master_key(&other_master_key, &[7; 32], &[9; 32])
            .unwrap();
    globals.cloud.set_master_key_backup(
        other_namespace.clone(),
        serde_json::to_vec(&encrypted_master).unwrap(),
    );
    globals.cloud.set_wallet_files(
        other_namespace.clone(),
        vec![wallet_filename_from_record_id("other-wallet")],
    );

    manager.do_delete_other_backups().await.unwrap();

    assert!(globals.cloud.has_namespace(&current_namespace));
    assert!(!globals.cloud.has_namespace(&other_namespace));
    assert_eq!(globals.cloud.deleted_namespace_policies(), vec![CloudAccessPolicy::ConsentAllowed]);
}

#[tokio::test(flavor = "current_thread")]
async fn delete_other_backups_returns_offline_when_namespace_inspection_is_offline() {
    let _guard = async_test_lock().lock().await;
    cove_tokio::init();
    let globals = test_globals();
    let manager = init_manager();
    configure_enabled_cloud_backup(&manager, globals, 0);

    let current_namespace = CloudBackupKeychain::global().namespace_id().unwrap();
    globals.cloud.set_master_key_backup(current_namespace.clone(), vec![1, 2, 3]);

    let other_master_key = cove_cspp::master_key::MasterKey::generate();
    let other_namespace = other_master_key.namespace_id();
    let encrypted_master =
        cove_cspp::master_key_crypto::encrypt_master_key(&other_master_key, &[7; 32], &[9; 32])
            .unwrap();
    globals.cloud.set_master_key_backup(
        other_namespace.clone(),
        serde_json::to_vec(&encrypted_master).unwrap(),
    );
    globals.cloud.fail_master_key_download_offline(
        other_namespace.clone(),
        "offline while inspecting namespace",
    );

    let result = manager.do_delete_other_backups().await;

    match result {
        Err(CloudBackupError::Offline(message)) => {
            assert_eq!(
                message,
                "Reconnect to the internet, then try deleting the other cloud backups again"
            );
        }
        Ok(()) => panic!("expected offline error"),
        Err(error) => panic!("expected offline error, got {error:?}"),
    }
    assert!(globals.cloud.has_namespace(&other_namespace));
}

#[tokio::test(flavor = "current_thread")]
async fn backup_wallets_does_not_create_master_key_or_upload_when_missing() {
    let _guard = async_test_lock().lock().await;
    cove_tokio::init();
    let globals = test_globals();
    globals.reset();

    let namespace = "existing-namespace";
    CloudBackupKeychain::global().save_namespace_id(namespace).unwrap();

    let manager = init_manager();
    let mut metadata = WalletMetadata::preview_new();
    metadata.wallet_type = crate::wallet::metadata::WalletType::WatchOnly;

    let error = manager.do_backup_wallets(&[metadata]).await.unwrap_err();

    match error {
        CloudBackupError::RecoveryRequired(message) => {
            assert_eq!(message, "Cloud backup needs verification before wallets can be uploaded");
        }
        error => panic!("expected recovery-required upload error, got {error:?}"),
    }

    let cspp = cove_cspp::Cspp::new(Keychain::global().clone());
    assert!(cspp.load_master_key_from_store().unwrap().is_none());
    assert_eq!(globals.cloud.uploaded_wallet_backup_count(), 0);
}

#[tokio::test(flavor = "current_thread")]
async fn upload_wallet_if_dirty_does_not_create_master_key_for_existing_namespace() {
    let _guard = async_test_lock().lock().await;
    cove_tokio::init();
    let globals = test_globals();
    globals.reset();

    let namespace = "existing-namespace";
    CloudBackupKeychain::global().save_namespace_id(namespace).unwrap();

    let manager = init_manager();
    let metadata = xpub_only_wallet_metadata();
    let xpub = sample_xpub(&metadata);
    Keychain::global().save_wallet_xpub(&metadata.id, xpub.parse().unwrap()).unwrap();
    Database::global()
        .wallets()
        .save_all_wallets(metadata.network, metadata.wallet_mode, vec![metadata.clone()])
        .unwrap();

    let record_id = cove_cspp::backup_data::wallet_record_id(metadata.id.as_ref());
    Database::global()
        .cloud_blob_sync_states
        .set(&PersistedCloudBlobSyncState::wallet(
            namespace.into(),
            metadata.id.clone(),
            record_id.clone(),
            PersistedCloudBlobState::Dirty(CloudBlobDirtyState { changed_at: 1 }),
        ))
        .unwrap();

    let error = manager.do_upload_wallet_if_dirty(&metadata.id).await.unwrap_err();

    match error {
        CloudBackupError::RecoveryRequired(message) => {
            assert_eq!(message, "Cloud backup needs verification before wallets can be uploaded");
        }
        error => panic!("expected recovery-required upload error, got {error:?}"),
    }

    let cspp = cove_cspp::Cspp::new(Keychain::global().clone());
    assert!(cspp.load_master_key_from_store().unwrap().is_none());
    assert_eq!(globals.cloud.uploaded_wallet_backup_count(), 0);
    assert!(matches!(
        Database::global().cloud_blob_sync_states.get(&record_id).unwrap(),
        Some(PersistedCloudBlobSyncState {
            state: PersistedCloudBlobState::Failed(CloudBlobFailedState { retryable: false, .. }),
            ..
        })
    ));
}

#[tokio::test(flavor = "current_thread")]
async fn deferred_live_wallet_upload_retries_without_restart() {
    let _guard = async_test_lock().lock().await;
    cove_tokio::init();
    let globals = test_globals();
    let manager = global_manager();
    clear_wallet_upload_runtime_for_test_async(&manager).await;
    configure_enabled_cloud_backup(&manager, globals, 0);

    let metadata = xpub_only_wallet_metadata();
    let record_id = cove_cspp::backup_data::wallet_record_id(metadata.id.as_ref());
    persist_xpub_wallets(vec![metadata.clone()]);
    persist_dirty_blob_state(metadata.id.clone());
    globals.cloud.fail_next_wallet_backup_upload_offline("offline");

    run_wallet_upload_for_test_async(&manager, metadata.id.clone()).await;

    assert_eq!(globals.cloud.uploaded_wallet_backup_count(), 0);
    assert_eq!(globals.cloud.wallet_backup_upload_attempt_count(), 1);
    assert!(matches!(
        Database::global().cloud_blob_sync_states.get(&record_id).unwrap(),
        Some(PersistedCloudBlobSyncState { state: PersistedCloudBlobState::Dirty(_), .. })
    ));

    wait_for_test_condition(
        Duration::from_secs(7),
        "deferred live upload should retry automatically after the backoff",
        || globals.cloud.wallet_backup_upload_attempt_count() >= 2,
    )
    .await;

    wait_for_test_condition(
        Duration::from_secs(1),
        "deferred live upload should eventually reach an uploaded state",
        || {
            matches!(
                Database::global().cloud_blob_sync_states.get(&record_id).unwrap(),
                Some(PersistedCloudBlobSyncState {
                    state: PersistedCloudBlobState::UploadedPendingConfirmation(_)
                        | PersistedCloudBlobState::Confirmed(_),
                    ..
                })
            )
        },
    )
    .await;
    assert!(globals.cloud.uploaded_wallet_backup_count() >= 1);

    clear_wallet_upload_runtime_for_test_async(&manager).await;
}

#[tokio::test(flavor = "current_thread")]
async fn failed_blob_states_recover_only_after_last_failed_wallet_upload_recovers() {
    let _guard = async_test_lock().lock().await;
    cove_tokio::init();
    let globals = test_globals();
    let manager = global_manager();
    configure_enabled_cloud_backup(&manager, globals, 0);

    let first_wallet = xpub_only_wallet_metadata();
    let second_wallet = xpub_only_wallet_metadata();
    let first_record_id = cove_cspp::backup_data::wallet_record_id(first_wallet.id.as_ref());
    let second_record_id = cove_cspp::backup_data::wallet_record_id(second_wallet.id.as_ref());

    persist_xpub_wallets(vec![first_wallet.clone(), second_wallet.clone()]);
    persist_dirty_blob_state(first_wallet.id.clone());
    persist_dirty_blob_state(second_wallet.id.clone());
    globals.cloud.fail_wallet_backup_upload("upload failed");

    run_wallet_upload_for_test_async(&manager, first_wallet.id.clone()).await;
    run_wallet_upload_for_test_async(&manager, second_wallet.id.clone()).await;

    assert!(matches!(
        manager.compute_sync_health().await,
        CloudSyncHealth::Failed(message) if message.contains("upload failed")
    ));
    assert!(matches!(
        Database::global().cloud_blob_sync_states.get(&first_record_id).unwrap(),
        Some(PersistedCloudBlobSyncState { state: PersistedCloudBlobState::Failed(_), .. })
    ));
    assert!(matches!(
        Database::global().cloud_blob_sync_states.get(&second_record_id).unwrap(),
        Some(PersistedCloudBlobSyncState { state: PersistedCloudBlobState::Failed(_), .. })
    ));

    globals.cloud.clear_wallet_backup_upload_failure();

    run_wallet_upload_for_test_async(&manager, first_wallet.id.clone()).await;

    assert_eq!(globals.cloud.uploaded_wallet_backup_count(), 1);
    assert!(matches!(
        manager.compute_sync_health().await,
        CloudSyncHealth::Failed(message) if message.contains("upload failed")
    ));
    assert!(matches!(
        Database::global().cloud_blob_sync_states.get(&first_record_id).unwrap(),
        Some(PersistedCloudBlobSyncState {
            state: PersistedCloudBlobState::UploadedPendingConfirmation(_)
                | PersistedCloudBlobState::Confirmed(_),
            ..
        })
    ));
    assert!(matches!(
        Database::global().cloud_blob_sync_states.get(&second_record_id).unwrap(),
        Some(PersistedCloudBlobSyncState { state: PersistedCloudBlobState::Failed(_), .. })
    ));

    run_wallet_upload_for_test_async(&manager, second_wallet.id.clone()).await;

    assert_eq!(globals.cloud.uploaded_wallet_backup_count(), 2);
    assert!(matches!(
        Database::global().cloud_blob_sync_states.get(&second_record_id).unwrap(),
        Some(PersistedCloudBlobSyncState {
            state: PersistedCloudBlobState::UploadedPendingConfirmation(_)
                | PersistedCloudBlobState::Confirmed(_),
            ..
        })
    ));
    assert!(
        Database::global()
            .cloud_blob_sync_states
            .list()
            .unwrap()
            .into_iter()
            .all(|state| !matches!(state.state, PersistedCloudBlobState::Failed(_)))
    );

    clear_wallet_upload_runtime_for_test_async(&manager).await;
}

#[tokio::test(flavor = "current_thread")]
async fn connectivity_reconnect_preserves_failed_wallet_upload_health() {
    let _guard = async_test_lock().lock().await;
    cove_tokio::init();
    let globals = test_globals();
    let manager = init_manager();
    configure_enabled_cloud_backup(&manager, globals, 0);

    let metadata = xpub_only_wallet_metadata();
    let record_id = cove_cspp::backup_data::wallet_record_id(metadata.id.as_ref());
    persist_xpub_wallets(vec![metadata.clone()]);
    Database::global()
        .cloud_blob_sync_states
        .set(&PersistedCloudBlobSyncState::wallet(
            CloudBackupKeychain::global().namespace_id().unwrap(),
            metadata.id,
            record_id,
            PersistedCloudBlobState::Failed(CloudBlobFailedState {
                revision_hash: None,
                error: "upload failed".into(),
                retryable: false,
                issue: None,
                failed_at: 1,
            }),
        ))
        .unwrap();

    manager.handle_connectivity_change(ConnectivityStatus::Connected);

    assert!(matches!(
        manager.compute_sync_health().await,
        CloudSyncHealth::Failed(message) if message.contains("upload failed")
    ));
}

#[tokio::test(flavor = "current_thread")]
async fn connectivity_reconnect_reports_clean_health_when_failed_wallet_uploads_are_gone() {
    let _guard = async_test_lock().lock().await;
    cove_tokio::init();
    let globals = test_globals();
    let manager = init_manager();
    configure_enabled_cloud_backup(&manager, globals, 0);

    manager.handle_connectivity_change(ConnectivityStatus::Connected);

    assert_eq!(manager.compute_sync_health().await, CloudSyncHealth::NoFiles);
}

#[tokio::test(flavor = "current_thread")]
async fn reconnect_retries_verification_after_offline_failure() {
    let _guard = async_test_lock().lock().await;
    cove_tokio::init();
    let globals = test_globals();
    let manager = init_manager();
    configure_enabled_cloud_backup(&manager, globals, 0);
    seed_verifiable_cloud_master_key(globals);
    CONNECTIVITY_MANAGER.set_connection_state(false);

    call!(manager.supervisor.start_verification(false)).await.unwrap();
    wait_for_test_condition(
        Duration::from_secs(1),
        "expected offline verification failure",
        || matches!(manager.model_snapshot().verification, VerificationState::Failed(_)),
    )
    .await;

    CONNECTIVITY_MANAGER.set_connection_state(true);

    wait_for_test_condition(
        Duration::from_secs(1),
        "expected reconnect to retry and verify backup",
        || matches!(manager.model_snapshot().verification, VerificationState::Verified(_)),
    )
    .await;
}

#[tokio::test(flavor = "current_thread")]
async fn connected_connectivity_failure_retries_detail_refresh_once() {
    let _guard = async_test_lock().lock().await;
    cove_tokio::init();
    let globals = test_globals();
    let manager = init_manager();
    configure_enabled_cloud_backup(&manager, globals, 0);
    CONNECTIVITY_MANAGER.set_connection_state(true);
    globals.cloud.fail_next_list_wallet_files_offline("offline");

    call!(manager.supervisor.start_refresh_detail()).await.unwrap();

    wait_for_test_condition(
        Duration::from_secs(1),
        "expected connectivity retry to refresh detail",
        || manager.model_snapshot().detail.is_some(),
    )
    .await;
}

#[tokio::test(flavor = "current_thread")]
async fn connected_connectivity_failure_retries_verification_once() {
    let _guard = async_test_lock().lock().await;
    cove_tokio::init();
    let globals = test_globals();
    let manager = init_manager();
    configure_enabled_cloud_backup(&manager, globals, 0);
    CONNECTIVITY_MANAGER.set_connection_state(true);
    seed_verifiable_cloud_master_key(globals);
    globals.cloud.fail_next_list_wallet_files_offline("offline");

    call!(manager.supervisor.start_verification(false)).await.unwrap();

    wait_for_test_condition(
        Duration::from_secs(1),
        "expected connected offline failure to retry verification",
        || matches!(manager.model_snapshot().verification, VerificationState::Verified(_)),
    )
    .await;
}

#[tokio::test(flavor = "current_thread")]
async fn unknown_connectivity_does_not_block_verification() {
    let _guard = async_test_lock().lock().await;
    cove_tokio::init();
    let globals = test_globals();
    let manager = init_manager();
    configure_enabled_cloud_backup(&manager, globals, 0);
    seed_verifiable_cloud_master_key(globals);
    CONNECTIVITY_MANAGER.set_connection_status(ConnectivityStatus::Unknown);

    call!(manager.supervisor.start_verification(false)).await.unwrap();

    wait_for_test_condition(
        Duration::from_secs(1),
        "expected unknown connectivity to attempt verification",
        || matches!(manager.model_snapshot().verification, VerificationState::Verified(_)),
    )
    .await;
}

#[tokio::test(flavor = "current_thread")]
async fn non_connectivity_verification_failure_does_not_retry_on_reconnect() {
    let _guard = async_test_lock().lock().await;
    cove_tokio::init();
    let globals = test_globals();
    let manager = init_manager();
    configure_enabled_cloud_backup(&manager, globals, 0);
    CONNECTIVITY_MANAGER.set_connection_state(true);
    globals.cloud.fail_list_wallet_files("list failed");

    call!(manager.supervisor.start_verification(false)).await.unwrap();
    wait_for_test_condition(
        Duration::from_secs(1),
        "expected non-connectivity verification failure",
        || matches!(manager.model_snapshot().verification, VerificationState::Failed(_)),
    )
    .await;

    CONNECTIVITY_MANAGER.set_connection_state(false);
    manager.handle_connectivity_change(ConnectivityStatus::Disconnected);
    CONNECTIVITY_MANAGER.set_connection_state(true);
    manager.handle_connectivity_change(ConnectivityStatus::Connected);

    assert_test_condition_stays_true(
        Duration::from_millis(150),
        "non-connectivity failure should stay failed after reconnect",
        || matches!(manager.model_snapshot().verification, VerificationState::Failed(_)),
    )
    .await;
}

#[test]
fn reset_cloud_backup_test_state_clears_state_before_reconnect() {
    let _guard = test_lock().lock();
    cove_tokio::init();
    let globals = test_globals();
    let manager = init_manager();
    configure_enabled_cloud_backup(&manager, globals, 0);

    let wallet_id = xpub_only_wallet_metadata().id;
    let record_id = cove_cspp::backup_data::wallet_record_id(wallet_id.as_ref());
    Database::global()
        .cloud_blob_sync_states
        .set(&PersistedCloudBlobSyncState::wallet(
            CloudBackupKeychain::global().namespace_id().unwrap(),
            wallet_id,
            record_id,
            PersistedCloudBlobState::Failed(CloudBlobFailedState {
                revision_hash: None,
                error: "upload failed".into(),
                retryable: false,
                issue: None,
                failed_at: 1,
            }),
        ))
        .unwrap();
    manager.apply_enable_outcome(CloudBackupEnableOutcome::AwaitingSavedPasskeyConfirmation(
        SavedPasskeyConfirmationMode::Manual,
    ));
    CONNECTIVITY_MANAGER.set_connection_state(false);

    reset_cloud_backup_test_state_with_hook(&manager, globals, || {
        assert!(Database::global().cloud_blob_sync_states.list().unwrap().is_empty());
        assert_eq!(manager.model_snapshot().enable_state, CloudBackupEnableState::Idle);
    });

    assert!(CONNECTIVITY_MANAGER.is_connected());
}

#[tokio::test(flavor = "current_thread")]
async fn startup_resume_skips_non_retryable_failed_wallet_uploads() {
    let _guard = async_test_lock().lock().await;
    cove_tokio::init();
    let globals = test_globals();
    let manager = global_manager();
    clear_wallet_upload_runtime_for_test_async(&manager).await;
    configure_enabled_cloud_backup(&manager, globals, 0);

    let metadata = xpub_only_wallet_metadata();
    let record_id = cove_cspp::backup_data::wallet_record_id(metadata.id.as_ref());
    persist_xpub_wallets(vec![metadata.clone()]);
    persist_failed_blob_state(metadata.id.clone(), false);
    globals.cloud.fail_wallet_backup_upload_quota_exceeded();
    let initial_attempt_count = globals.cloud.wallet_backup_upload_attempt_count();

    resume_wallet_uploads_from_persisted_state_for_test_async(&manager).await;

    assert_test_condition_stays_true(
        Duration::from_millis(250),
        "startup resume should not retry non-retryable failed uploads",
        || globals.cloud.wallet_backup_upload_attempt_count() == initial_attempt_count,
    )
    .await;

    assert_eq!(globals.cloud.wallet_backup_upload_attempt_count(), initial_attempt_count);
    assert!(matches!(
        Database::global().cloud_blob_sync_states.get(&record_id).unwrap(),
        Some(PersistedCloudBlobSyncState {
            state: PersistedCloudBlobState::Failed(CloudBlobFailedState { retryable: false, .. }),
            ..
        })
    ));

    clear_wallet_upload_runtime_for_test_async(&manager).await;
    globals.cloud.clear_wallet_backup_upload_failure();
}

#[tokio::test(flavor = "current_thread")]
async fn startup_resume_retries_authorization_failed_wallet_uploads() {
    let _guard = async_test_lock().lock().await;
    cove_tokio::init();
    let globals = test_globals();
    let manager = global_manager();
    clear_wallet_upload_runtime_for_test_async(&manager).await;
    configure_enabled_cloud_backup(&manager, globals, 0);

    let metadata = xpub_only_wallet_metadata();
    persist_xpub_wallets(vec![metadata.clone()]);
    persist_failed_blob_state_with_issue(
        metadata.id,
        false,
        Some(CloudBlobFailureIssue::AuthorizationRequired),
    );
    let initial_attempt_count = globals.cloud.wallet_backup_upload_attempt_count();

    resume_wallet_uploads_from_persisted_state_for_test_async(&manager).await;

    assert_eq!(
        manager.compute_sync_health().await,
        CloudSyncHealth::AuthorizationRequired("failed".into()),
    );
    wait_for_test_condition(
        Duration::from_secs(1),
        "startup resume should retry authorization failures",
        || globals.cloud.wallet_backup_upload_attempt_count() > initial_attempt_count,
    )
    .await;

    clear_wallet_upload_runtime_for_test_async(&manager).await;
}

#[tokio::test(flavor = "current_thread")]
async fn cloud_storage_change_retries_authorization_failed_wallet_uploads() {
    let _guard = async_test_lock().lock().await;
    cove_tokio::init();
    let globals = test_globals();
    let manager = global_manager();
    clear_wallet_upload_runtime_for_test_async(&manager).await;
    configure_enabled_cloud_backup(&manager, globals, 0);

    let metadata = xpub_only_wallet_metadata();
    persist_xpub_wallets(vec![metadata.clone()]);
    persist_failed_blob_state_with_issue(
        metadata.id,
        false,
        Some(CloudBlobFailureIssue::AuthorizationRequired),
    );
    let initial_attempt_count = globals.cloud.wallet_backup_upload_attempt_count();

    manager.cloud_storage_did_change();

    wait_for_test_condition(
        Duration::from_secs(3),
        "cloud storage change should retry authorization failures",
        || globals.cloud.wallet_backup_upload_attempt_count() > initial_attempt_count,
    )
    .await;

    clear_wallet_upload_runtime_for_test_async(&manager).await;
}

#[tokio::test(flavor = "current_thread")]
async fn sync_health_reports_authorization_required_for_persisted_auth_failures() {
    let _guard = async_test_lock().lock().await;
    cove_tokio::init();
    let globals = test_globals();
    let manager = global_manager();
    clear_wallet_upload_runtime_for_test_async(&manager).await;
    configure_enabled_cloud_backup(&manager, globals, 0);

    let metadata = xpub_only_wallet_metadata();
    persist_xpub_wallets(vec![metadata.clone()]);
    persist_failed_blob_state_with_issue(
        metadata.id,
        true,
        Some(CloudBlobFailureIssue::AuthorizationRequired),
    );

    assert_eq!(
        manager.compute_sync_health().await,
        CloudSyncHealth::AuthorizationRequired("failed".into()),
    );

    clear_wallet_upload_runtime_for_test_async(&manager).await;
}

#[tokio::test(flavor = "current_thread")]
async fn sync_health_reports_uploading_for_fresh_pending_master_key_confirmation() {
    let _guard = async_test_lock().lock().await;
    cove_tokio::init();
    let globals = test_globals();
    let manager = global_manager();
    clear_wallet_upload_runtime_for_test_async(&manager).await;
    configure_enabled_cloud_backup(&manager, globals, 0);

    let namespace_id = CloudBackupKeychain::global().namespace_id().unwrap();
    let master_json = vec![1, 2, 3];
    let master_revision = master_key_wrapper_revision_hash(&master_json);
    persist_pending_master_key_confirmation(namespace_id.clone(), master_revision);
    assert_eq!(manager.compute_sync_health().await, CloudSyncHealth::Uploading);

    globals.cloud.set_master_key_backup(namespace_id, master_json);

    let has_more_pending = verify_pending_uploads_once_for_test_async(&manager).await;

    assert!(!has_more_pending);
    assert_eq!(manager.compute_sync_health().await, CloudSyncHealth::AllUploaded);
    assert_eq!(
        manager.model_snapshot().pending_upload_verification,
        PendingUploadVerificationState::Idle
    );
    assert!(matches!(manager.model_snapshot().verification, VerificationState::Idle));

    clear_wallet_upload_runtime_for_test_async(&manager).await;
}

#[tokio::test(flavor = "current_thread")]
async fn pending_upload_verification_blocks_on_cloud_authorization() {
    let _guard = async_test_lock().lock().await;
    cove_tokio::init();
    let globals = test_globals();
    let manager = global_manager();
    clear_wallet_upload_runtime_for_test_async(&manager).await;
    configure_enabled_cloud_backup(&manager, globals, 0);

    let namespace_id = CloudBackupKeychain::global().namespace_id().unwrap();
    persist_pending_master_key_confirmation(namespace_id.clone(), "pending");
    globals
        .cloud
        .fail_master_key_download_authorization_required(namespace_id, "authorization required");

    let has_more_pending = verify_pending_uploads_once_for_test_async(&manager).await;

    assert!(has_more_pending);
    assert_eq!(
        manager.model_snapshot().pending_upload_verification,
        PendingUploadVerificationState::BlockedOnAuthorization
    );

    clear_wallet_upload_runtime_for_test_async(&manager).await;
}

#[tokio::test(flavor = "current_thread")]
async fn sync_health_reports_uploading_for_fresh_pending_master_key_with_local_wallets() {
    let _guard = async_test_lock().lock().await;
    cove_tokio::init();
    let globals = test_globals();
    let manager = global_manager();
    clear_wallet_upload_runtime_for_test_async(&manager).await;
    configure_enabled_cloud_backup(&manager, globals, 1);

    let metadata = xpub_only_wallet_metadata();
    persist_xpub_wallets(vec![metadata]);
    let namespace_id = CloudBackupKeychain::global().namespace_id().unwrap();
    persist_pending_master_key_confirmation(namespace_id, "pending");

    assert_eq!(manager.compute_sync_health().await, CloudSyncHealth::Uploading,);

    clear_wallet_upload_runtime_for_test_async(&manager).await;
}

#[tokio::test(flavor = "current_thread")]
async fn force_new_reports_uploading_while_master_key_confirmation_is_pending() {
    let _guard = async_test_lock().lock().await;
    cove_tokio::init();
    let globals = test_globals();
    let manager = init_manager();

    reset_cloud_backup_test_state(&manager, globals);
    CONNECTIVITY_MANAGER.set_connection_state(true);
    globals.passkey.set_create_result(Ok(vec![1, 2, 3]));
    globals.passkey.set_authenticate_result(Ok(vec![7; 32]));
    globals.cloud.set_master_key_backup("existing-namespace".into(), vec![1, 2, 3]);
    globals.cloud.set_wallet_files("existing-namespace".into(), vec!["wallet-1.json".into()]);

    let metadata = xpub_only_wallet_metadata();
    persist_xpub_wallets(vec![metadata]);

    enable_cloud_backup_no_discovery(&manager).await.unwrap();
    enable_cloud_backup_force_new(&manager).await.unwrap();
    confirm_saved_passkey_session(&manager).await;

    let namespace_id = CloudBackupKeychain::global().namespace_id().unwrap();
    CloudStorage::global_silent_client()
        .delete_wallet_backup(namespace_id.clone(), cspp_master_key_record_id())
        .await
        .unwrap();
    persist_pending_master_key_confirmation(namespace_id, "pending");

    assert_eq!(manager.compute_sync_health().await, CloudSyncHealth::Uploading,);

    clear_wallet_upload_runtime_for_test_async(&manager).await;
}

#[tokio::test(flavor = "current_thread")]
async fn sync_health_reports_missing_master_key_without_pending_confirmation() {
    let _guard = async_test_lock().lock().await;
    cove_tokio::init();
    let globals = test_globals();
    let manager = global_manager();
    clear_wallet_upload_runtime_for_test_async(&manager).await;
    configure_enabled_cloud_backup(&manager, globals, 1);

    let metadata = xpub_only_wallet_metadata();
    persist_xpub_wallets(vec![metadata]);

    assert_eq!(
        manager.compute_sync_health().await,
        CloudSyncHealth::Failed(SYNC_HEALTH_MISSING_MASTER_KEY_MESSAGE.into()),
    );

    clear_wallet_upload_runtime_for_test_async(&manager).await;
}

#[tokio::test(flavor = "current_thread")]
async fn sync_health_reports_missing_master_key_before_pending_wallet_uploads() {
    let _guard = async_test_lock().lock().await;
    cove_tokio::init();
    let globals = test_globals();
    let manager = global_manager();
    clear_wallet_upload_runtime_for_test_async(&manager).await;
    configure_enabled_cloud_backup(&manager, globals, 1);

    let metadata = xpub_only_wallet_metadata();
    persist_xpub_wallets(vec![metadata.clone()]);
    persist_dirty_blob_state(metadata.id);

    assert_eq!(
        manager.compute_sync_health().await,
        CloudSyncHealth::Failed(SYNC_HEALTH_MISSING_MASTER_KEY_MESSAGE.into()),
    );

    clear_wallet_upload_runtime_for_test_async(&manager).await;
}

#[tokio::test(flavor = "current_thread")]
async fn sync_health_respects_master_key_upload_confirmation_grace() {
    let _guard = async_test_lock().lock().await;
    cove_tokio::init();
    let globals = test_globals();
    let manager = global_manager();
    clear_wallet_upload_runtime_for_test_async(&manager).await;
    configure_enabled_cloud_backup(&manager, globals, 1);

    let metadata = xpub_only_wallet_metadata();
    persist_xpub_wallets(vec![metadata]);
    let namespace_id = CloudBackupKeychain::global().namespace_id().unwrap();

    assert_eq!(
        manager.compute_sync_health_with_master_key_grace(Some(&namespace_id)).await,
        CloudSyncHealth::Uploading,
    );

    clear_wallet_upload_runtime_for_test_async(&manager).await;
}

#[tokio::test(flavor = "current_thread")]
async fn startup_resume_retries_interrupted_uploading_wallets() {
    let _guard = async_test_lock().lock().await;
    cove_tokio::init();
    let globals = test_globals();
    let manager = global_manager();
    clear_wallet_upload_runtime_for_test_async(&manager).await;
    configure_enabled_cloud_backup(&manager, globals, 0);

    let metadata = xpub_only_wallet_metadata();
    let record_id = cove_cspp::backup_data::wallet_record_id(metadata.id.as_ref());
    persist_xpub_wallets(vec![metadata.clone()]);
    persist_uploading_blob_state(metadata.id, 1);

    resume_wallet_uploads_from_persisted_state_for_test_async(&manager).await;

    wait_for_test_condition(
        Duration::from_secs(5),
        "startup resume should retry interrupted uploads",
        || {
            let upload_state_is_pending_or_confirmed = matches!(
                Database::global().cloud_blob_sync_states.get(&record_id).unwrap(),
                Some(PersistedCloudBlobSyncState {
                    state: PersistedCloudBlobState::UploadedPendingConfirmation(_)
                        | PersistedCloudBlobState::Confirmed(_),
                    ..
                })
            );

            globals.cloud.wallet_backup_upload_attempt_count() >= 1
                && upload_state_is_pending_or_confirmed
        },
    )
    .await;

    clear_wallet_upload_runtime_for_test_async(&manager).await;
}

#[tokio::test(flavor = "current_thread")]
async fn validate_metadata_marks_generated_wallet_names_dirty() {
    let _guard = async_test_lock().lock().await;
    cove_tokio::init();
    let globals = test_globals();
    let manager = global_manager();
    reset_cloud_backup_test_state(&manager, globals);

    let mut metadata = WalletMetadata::preview_new();
    metadata.name.clear();

    let wallet = Wallet::try_new_persisted_and_selected(
            metadata,
            Mnemonic::parse(
                "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about",
            )
            .unwrap(),
            None,
        )
        .unwrap();
    let wallet_id = wallet.id();
    let stored_metadata = Database::global()
        .wallets()
        .get(&wallet_id, wallet.network, wallet.metadata.wallet_mode)
        .unwrap()
        .unwrap();
    let expected_name = stored_metadata
        .master_fingerprint
        .as_deref()
        .map_or_else(|| "Unnamed Wallet".to_string(), |fingerprint| fingerprint.as_uppercase());

    enable_cloud_backup_without_reset(&manager, 1);

    let wallet_manager = RustWalletManager::try_new(wallet_id.clone()).unwrap();
    wallet_manager.validate_metadata();

    let updated_metadata = Database::global()
        .wallets()
        .get(&wallet_id, wallet.network, wallet.metadata.wallet_mode)
        .unwrap()
        .unwrap();
    let record_id = cove_cspp::backup_data::wallet_record_id(wallet_id.as_ref());

    assert_eq!(updated_metadata.name, expected_name);
    assert!(matches!(
        Database::global().cloud_blob_sync_states.get(&record_id).unwrap(),
        Some(PersistedCloudBlobSyncState { state: PersistedCloudBlobState::Dirty(_), .. })
    ));

    clear_wallet_upload_runtime_for_test_async(&manager).await;
}

#[tokio::test(flavor = "current_thread")]
async fn upload_wallet_if_dirty_removes_deleted_wallet_sync_state() {
    let _guard = async_test_lock().lock().await;
    cove_tokio::init();
    let globals = test_globals();
    let manager = init_manager();
    configure_enabled_cloud_backup(&manager, globals, 0);

    let metadata = WalletMetadata::preview_new();
    let namespace = CloudBackupKeychain::global().namespace_id().unwrap();
    let record_id = cove_cspp::backup_data::wallet_record_id(metadata.id.as_ref());
    Database::global()
        .cloud_blob_sync_states
        .set(&PersistedCloudBlobSyncState::wallet(
            namespace.clone(),
            metadata.id.clone(),
            record_id.clone(),
            PersistedCloudBlobState::Dirty(CloudBlobDirtyState { changed_at: 1 }),
        ))
        .unwrap();

    manager.do_upload_wallet_if_dirty(&metadata.id).await.unwrap();

    assert!(Database::global().cloud_blob_sync_states.get(&record_id).unwrap().is_none());
    assert_eq!(globals.cloud.uploaded_wallet_backup_count(), 0);
    assert_eq!(CloudBackupKeychain::global().namespace_id(), Some(namespace));
}

#[tokio::test(flavor = "current_thread")]
async fn sync_and_integrity_skip_pending_upload_candidates() {
    let _guard = async_test_lock().lock().await;
    cove_tokio::init();
    let globals = test_globals();
    let manager = init_manager();
    configure_enabled_cloud_backup(&manager, globals, 1);

    let mut metadata = WalletMetadata::preview_new();
    metadata.wallet_type = WalletType::WatchOnly;
    Database::global()
        .wallets()
        .save_all_wallets(metadata.network, metadata.wallet_mode, vec![metadata.clone()])
        .unwrap();

    let namespace_id = CloudBackupKeychain::global().namespace_id().unwrap();
    let record_id = cove_cspp::backup_data::wallet_record_id(metadata.id.as_ref());
    Database::global()
        .cloud_blob_sync_states
        .set(&PersistedCloudBlobSyncState::wallet(
            namespace_id,
            metadata.id.clone(),
            record_id,
            PersistedCloudBlobState::UploadedPendingConfirmation(
                CloudBlobUploadedPendingConfirmationState {
                    revision_hash: "rev-1".into(),
                    uploaded_at: 10,
                    attempt_count: 0,
                    last_checked_at: None,
                },
            ),
        ))
        .unwrap();

    manager.do_sync_unsynced_wallets().await.unwrap();

    assert_eq!(globals.cloud.uploaded_wallet_backup_count(), 0);
    assert_eq!(Database::global().cloud_backup_state.get().unwrap().wallet_count(), Some(1));

    let warning = manager.verify_backup_integrity_impl().await.expect("expected passkey warning");

    assert!(!warning.contains("some wallets are not backed up"));
    assert_eq!(globals.cloud.uploaded_wallet_backup_count(), 0);
    assert_eq!(Database::global().cloud_backup_state.get().unwrap().wallet_count(), Some(1));
}

#[tokio::test(flavor = "current_thread")]
async fn integrity_does_not_retry_sync_after_auto_backup_failure() {
    let _guard = async_test_lock().lock().await;
    cove_tokio::init();
    let globals = test_globals();
    let manager = init_manager();
    configure_enabled_cloud_backup(&manager, globals, 1);

    let namespace = CloudBackupKeychain::global().namespace_id().unwrap();
    CloudBackupKeychain::global()
        .save_passkey_and_namespace(&[1, 2, 3, 4], [9; 32], &namespace)
        .unwrap();

    let mut metadata = WalletMetadata::preview_new();
    metadata.wallet_type = WalletType::WatchOnly;
    Database::global()
        .wallets()
        .save_all_wallets(metadata.network, metadata.wallet_mode, vec![metadata])
        .unwrap();
    globals.cloud.fail_wallet_backup_upload("offline");

    let warning = manager.verify_backup_integrity_impl().await.expect("expected integrity warning");

    assert!(warning.contains("some wallets are not backed up"));
    assert_eq!(globals.cloud.wallet_backup_upload_attempt_count(), 1);
}

#[tokio::test(flavor = "current_thread")]
async fn integrity_warns_when_background_wallet_list_fails() {
    let _guard = async_test_lock().lock().await;
    cove_tokio::init();
    let globals = test_globals();
    let manager = init_manager();
    configure_enabled_cloud_backup(&manager, globals, 0);

    let namespace = CloudBackupKeychain::global().namespace_id().unwrap();
    CloudBackupKeychain::global()
        .save_passkey_and_namespace(&[1, 2, 3, 4], [9; 32], &namespace)
        .unwrap();
    globals.cloud.fail_list_wallet_files_non_interactive("offline");

    let warning = manager.verify_backup_integrity_impl().await.expect("expected integrity warning");

    assert!(warning.contains("wallet backups could not be listed"));
    globals.cloud.clear_list_wallet_files_non_interactive_failure();
}

#[tokio::test(flavor = "current_thread")]
async fn refresh_cloud_backup_detail_uses_interactive_wallet_listing() {
    let _guard = async_test_lock().lock().await;
    cove_tokio::init();
    let globals = test_globals();
    let manager = init_manager();
    configure_enabled_cloud_backup(&manager, globals, 1);

    let metadata = xpub_only_wallet_metadata();
    persist_xpub_wallets(vec![metadata.clone()]);

    let keychain = Keychain::global();
    let namespace = CloudBackupKeychain::new(keychain.clone()).namespace_id().unwrap();
    CloudBackupKeychain::new(keychain.clone())
        .save_passkey_and_namespace(&[1, 2, 3, 4], [9; 32], &namespace)
        .unwrap();
    let master_key =
        cove_cspp::Cspp::new(keychain.clone()).load_master_key_from_store().unwrap().unwrap();
    let record_id = cove_cspp::backup_data::wallet_record_id(metadata.id.as_ref());
    globals.cloud.set_wallet_backup(
        namespace.clone(),
        record_id.clone(),
        encrypted_wallet_backup_bytes(&metadata, &master_key, "interactive-revision", 1).await,
    );
    globals.cloud.set_wallet_files(namespace, vec![wallet_filename_from_record_id(&record_id)]);
    globals.cloud.fail_list_wallet_files_non_interactive("offline");

    let Some(CloudBackupDetailResult::Success(detail)) =
        manager.refresh_cloud_backup_detail().await
    else {
        panic!("expected cloud backup detail");
    };

    assert_eq!(1, detail.up_to_date.len() + detail.needs_sync.len());
    let listed_record_id = detail
        .up_to_date
        .first()
        .map(|wallet| wallet.record_id.clone())
        .or_else(|| detail.needs_sync.first().map(|wallet| wallet.record_id.clone()))
        .expect("expected listed wallet");
    assert_eq!(listed_record_id, record_id);
    globals.cloud.clear_list_wallet_files_non_interactive_failure();
}

#[tokio::test(flavor = "current_thread")]
async fn integrity_preserves_unsupported_remote_wallet_backups() {
    let _guard = async_test_lock().lock().await;
    cove_tokio::init();
    let globals = test_globals();
    let manager = init_manager();
    configure_enabled_cloud_backup(&manager, globals, 1);

    let metadata = xpub_only_wallet_metadata();
    persist_xpub_wallets(vec![metadata.clone()]);

    let keychain = Keychain::global();
    let namespace = CloudBackupKeychain::new(keychain.clone()).namespace_id().unwrap();
    CloudBackupKeychain::new(keychain.clone())
        .save_passkey_and_namespace(&[1, 2, 3, 4], [9; 32], &namespace)
        .unwrap();
    let master_key =
        cove_cspp::Cspp::new(keychain.clone()).load_master_key_from_store().unwrap().unwrap();
    let record_id = cove_cspp::backup_data::wallet_record_id(metadata.id.as_ref());
    globals.cloud.set_wallet_backup(
        namespace.clone(),
        record_id.clone(),
        encrypted_wallet_backup_bytes(&metadata, &master_key, "unsupported-revision", 2).await,
    );
    globals.cloud.set_wallet_files(namespace, vec![wallet_filename_from_record_id(&record_id)]);

    let warning = manager.verify_backup_integrity_impl().await;

    assert!(warning.is_none());
    assert_eq!(globals.cloud.uploaded_wallet_backup_count(), 0);

    let detail = manager.model_snapshot().detail.expect("expected cloud backup detail");
    assert_eq!(detail.needs_sync.len(), 1);
    assert_eq!(detail.needs_sync[0].record_id, record_id);
    assert_eq!(detail.needs_sync[0].sync_status, CloudBackupWalletStatus::UnsupportedVersion);
}

#[tokio::test(flavor = "current_thread")]
async fn refresh_cloud_backup_detail_marks_listed_wallet_unknown_when_master_key_is_missing() {
    let _guard = async_test_lock().lock().await;
    cove_tokio::init();
    let globals = test_globals();
    let manager = init_manager();
    reset_cloud_backup_test_state(&manager, globals);

    let metadata = xpub_only_wallet_metadata();

    let master_key = cove_cspp::master_key::MasterKey::generate();
    let namespace = master_key.namespace_id();
    let keychain = Keychain::global();
    CloudBackupKeychain::new(keychain.clone()).save_namespace_id(&namespace).unwrap();
    cove_cspp::Cspp::new(keychain.clone()).save_master_key(&master_key).unwrap();
    manager
        .persist_cloud_backup_state(
            &persisted_enabled_cloud_backup_state(Some(1)),
            "set cloud backup enabled for test",
        )
        .unwrap();
    manager.sync_persisted_state();

    persist_xpub_wallets(vec![metadata.clone()]);

    let record_id = wallet_record_id(metadata.id.as_ref());
    globals.cloud.set_wallet_backup(
        namespace.clone(),
        record_id.clone(),
        encrypted_wallet_backup_bytes(&metadata, &master_key, "rev-1", 1).await,
    );
    globals.cloud.set_wallet_files(namespace, vec![wallet_filename_from_record_id(&record_id)]);

    let cspp = cove_cspp::Cspp::new(keychain.clone());
    cspp.delete_master_key();
    cove_cspp::Cspp::<Keychain>::clear_cached_master_key();

    let Some(CloudBackupDetailResult::Success(detail)) =
        manager.refresh_cloud_backup_detail().await
    else {
        panic!("expected cloud backup detail");
    };

    assert_eq!(detail.needs_sync.len(), 1);
    assert_eq!(detail.needs_sync[0].record_id, record_id);
    assert_eq!(detail.needs_sync[0].sync_status, CloudBackupWalletStatus::RemoteStateUnknown);
}

#[tokio::test(flavor = "current_thread")]
async fn sync_skips_wallets_with_unknown_remote_truth() {
    let _guard = async_test_lock().lock().await;
    cove_tokio::init();
    let globals = test_globals();
    let manager = init_manager();
    configure_enabled_cloud_backup(&manager, globals, 1);

    let metadata = xpub_only_wallet_metadata();
    let record_id = cove_cspp::backup_data::wallet_record_id(metadata.id.as_ref());
    persist_xpub_wallets(vec![metadata]);

    let namespace = CloudBackupKeychain::global().namespace_id().unwrap();
    globals
        .cloud
        .set_wallet_files(namespace.clone(), vec![wallet_filename_from_record_id(&record_id)]);
    globals.cloud.set_wallet_backup(namespace, record_id, b"{".to_vec());

    manager.do_sync_unsynced_wallets().await.unwrap();

    assert_eq!(globals.cloud.uploaded_wallet_backup_count(), 0);
}

#[tokio::test(flavor = "current_thread")]
async fn integrity_refreshes_detail_after_auto_backup_success() {
    let _guard = async_test_lock().lock().await;
    cove_tokio::init();
    let globals = test_globals();
    let manager = init_manager();
    configure_enabled_cloud_backup(&manager, globals, 0);

    let metadata = xpub_only_wallet_metadata();
    let record_id = cove_cspp::backup_data::wallet_record_id(metadata.id.as_ref());
    persist_xpub_wallets(vec![metadata]);

    let namespace = CloudBackupKeychain::global().namespace_id().unwrap();
    CloudBackupKeychain::global()
        .save_passkey_and_namespace(&[1, 2, 3, 4], [9; 32], &namespace)
        .unwrap();
    globals.cloud.set_reflect_uploaded_wallets_in_listing(true);

    let warning = manager.verify_backup_integrity_impl().await;

    assert!(warning.is_none());
    let detail = manager.model_snapshot().detail.expect("expected cloud backup detail");
    assert_eq!(detail.up_to_date.len(), 1);
    assert!(detail.needs_sync.is_empty());
    assert_eq!(detail.up_to_date[0].record_id, record_id);
    assert_eq!(detail.up_to_date[0].sync_status, CloudBackupWalletStatus::Confirmed);
    clear_wallet_upload_runtime_for_test_async(&manager).await;
}

#[tokio::test(flavor = "current_thread")]
async fn integrity_auto_backup_continues_when_other_backup_summary_fails() {
    let _guard = async_test_lock().lock().await;
    cove_tokio::init();
    let globals = test_globals();
    let manager = init_manager();
    configure_enabled_cloud_backup(&manager, globals, 0);

    let metadata = xpub_only_wallet_metadata();
    persist_xpub_wallets(vec![metadata]);

    let namespace = CloudBackupKeychain::global().namespace_id().unwrap();
    CloudBackupKeychain::global()
        .save_passkey_and_namespace(&[1, 2, 3, 4], [9; 32], &namespace)
        .unwrap();
    globals.cloud.fail_list_namespaces("offline while listing namespaces");

    let warning = manager.verify_backup_integrity_impl().await;

    assert!(warning.is_none());
    assert_eq!(globals.cloud.wallet_backup_upload_attempt_count(), 1);
    let detail = manager.model_snapshot().detail.expect("expected cloud backup detail");
    assert!(matches!(detail.other_backups, CloudBackupOtherBackupsState::LoadFailed { .. }));
    clear_wallet_upload_runtime_for_test_async(&manager).await;
}

#[tokio::test(flavor = "current_thread")]
async fn integrity_does_not_retry_sync_after_auto_backup_success_when_listing_stays_empty() {
    let _guard = async_test_lock().lock().await;
    cove_tokio::init();
    let globals = test_globals();
    let manager = init_manager();
    configure_enabled_cloud_backup(&manager, globals, 0);

    let metadata = xpub_only_wallet_metadata();
    persist_xpub_wallets(vec![metadata]);

    let namespace = CloudBackupKeychain::global().namespace_id().unwrap();
    CloudBackupKeychain::global()
        .save_passkey_and_namespace(&[1, 2, 3, 4], [9; 32], &namespace)
        .unwrap();

    let warning = manager.verify_backup_integrity_impl().await;

    assert!(warning.is_none());
    assert_eq!(globals.cloud.wallet_backup_upload_attempt_count(), 1);
}

#[tokio::test(flavor = "current_thread")]
async fn integrity_refreshes_detail_after_auto_backup_failure() {
    let _guard = async_test_lock().lock().await;
    cove_tokio::init();
    let globals = test_globals();
    let manager = init_manager();
    configure_enabled_cloud_backup(&manager, globals, 0);

    let metadata = xpub_only_wallet_metadata();
    let record_id = cove_cspp::backup_data::wallet_record_id(metadata.id.as_ref());
    persist_xpub_wallets(vec![metadata]);

    let namespace = CloudBackupKeychain::global().namespace_id().unwrap();
    CloudBackupKeychain::global()
        .save_passkey_and_namespace(&[1, 2, 3, 4], [9; 32], &namespace)
        .unwrap();
    globals.cloud.fail_wallet_backup_upload("offline");

    let warning = manager.verify_backup_integrity_impl().await.expect("expected integrity warning");

    assert!(warning.contains("some wallets are not backed up"));
    let detail = manager.model_snapshot().detail.expect("expected cloud backup detail");
    assert_eq!(detail.needs_sync.len(), 1);
    assert_eq!(detail.needs_sync[0].record_id, record_id);
    assert_eq!(detail.needs_sync[0].sync_status, CloudBackupWalletStatus::Dirty);
}

#[tokio::test(flavor = "current_thread")]
async fn upload_wallet_if_dirty_preserves_newer_dirty_state() {
    let _guard = async_test_lock().lock().await;
    cove_tokio::init();
    let globals = test_globals();
    let manager = init_manager();
    configure_enabled_cloud_backup(&manager, globals, 0);

    let mut metadata = WalletMetadata::preview_new();
    metadata.wallet_type = WalletType::WatchOnly;
    Database::global()
        .wallets()
        .save_all_wallets(metadata.network, metadata.wallet_mode, vec![metadata.clone()])
        .unwrap();

    let namespace_id = CloudBackupKeychain::global().namespace_id().unwrap();
    let record_id = cove_cspp::backup_data::wallet_record_id(metadata.id.as_ref());
    Database::global()
        .cloud_blob_sync_states
        .set(&PersistedCloudBlobSyncState::wallet(
            namespace_id,
            metadata.id.clone(),
            record_id.clone(),
            PersistedCloudBlobState::Dirty(CloudBlobDirtyState { changed_at: 1 }),
        ))
        .unwrap();
    globals.cloud.dirty_wallet_on_next_upload(metadata.id.clone());

    manager.do_upload_wallet_if_dirty(&metadata.id).await.unwrap();

    assert_eq!(globals.cloud.uploaded_wallet_backup_count(), 1);
    assert!(matches!(
        Database::global().cloud_blob_sync_states.get(&record_id).unwrap(),
        Some(PersistedCloudBlobSyncState { state: PersistedCloudBlobState::Dirty(_), .. })
    ));
}

#[tokio::test(flavor = "current_thread")]
async fn upload_wallet_if_dirty_recovers_deferred_write_to_dirty() {
    let _guard = async_test_lock().lock().await;
    cove_tokio::init();
    let globals = test_globals();
    let manager = init_manager();
    configure_enabled_cloud_backup(&manager, globals, 0);

    let metadata = xpub_only_wallet_metadata();
    persist_xpub_wallets(vec![metadata.clone()]);
    persist_dirty_blob_state(metadata.id.clone());

    let namespace_id = CloudBackupKeychain::global().namespace_id().unwrap();
    let record_id = wallet_record_id(metadata.id.as_ref());
    let PersistedCloudBackupState::Configured(previous_configured) =
        Database::global().cloud_backup_state.get().unwrap()
    else {
        panic!("expected configured cloud backup state");
    };
    globals.cloud.persist_disabling_on_next_upload(PersistedDisablingCloudBackup {
        previous_configured,
        namespace_id,
        disable_generation: 42,
        started_at: 100,
        delete_started_at: None,
        last_error: None,
        retry_after: None,
    });

    let error = manager.do_upload_wallet_if_dirty(&metadata.id).await.unwrap_err();

    assert!(matches!(error, CloudBackupError::Deferred(_)));
    assert_eq!(globals.cloud.uploaded_wallet_backup_count(), 1);
    assert!(matches!(
        Database::global().cloud_backup_state.get().unwrap(),
        PersistedCloudBackupState::Disabling(_)
    ));
    assert!(matches!(
        Database::global().cloud_blob_sync_states.get(&record_id).unwrap(),
        Some(PersistedCloudBlobSyncState { state: PersistedCloudBlobState::Dirty(_), .. })
    ));
    manager.debug_reset_cloud_backup_state();
}

#[tokio::test(flavor = "current_thread")]
async fn upload_wallet_if_dirty_retries_stale_uploading_state() {
    let _guard = async_test_lock().lock().await;
    cove_tokio::init();
    let globals = test_globals();
    let manager = init_manager();
    configure_enabled_cloud_backup(&manager, globals, 0);

    let metadata = xpub_only_wallet_metadata();
    persist_xpub_wallets(vec![metadata.clone()]);
    let record_id = cove_cspp::backup_data::wallet_record_id(metadata.id.as_ref());
    persist_uploading_blob_state(metadata.id.clone(), 1);

    manager.do_upload_wallet_if_dirty(&metadata.id).await.unwrap();

    assert_eq!(globals.cloud.uploaded_wallet_backup_count(), 1);
    assert!(matches!(
        Database::global().cloud_blob_sync_states.get(&record_id).unwrap(),
        Some(PersistedCloudBlobSyncState {
            state: PersistedCloudBlobState::UploadedPendingConfirmation(_)
                | PersistedCloudBlobState::Confirmed(_),
            ..
        })
    ));
    clear_wallet_upload_runtime_for_test_async(&manager).await;
}

#[tokio::test(flavor = "current_thread")]
async fn upload_wallet_if_dirty_recovers_stale_uploading_state_while_offline() {
    let _guard = async_test_lock().lock().await;
    cove_tokio::init();
    let globals = test_globals();
    let manager = init_manager();
    configure_enabled_cloud_backup(&manager, globals, 0);

    let metadata = xpub_only_wallet_metadata();
    persist_xpub_wallets(vec![metadata.clone()]);
    let record_id = cove_cspp::backup_data::wallet_record_id(metadata.id.as_ref());
    persist_uploading_blob_state(metadata.id.clone(), 1);
    CONNECTIVITY_MANAGER.set_connection_state(false);

    let error = manager.do_upload_wallet_if_dirty(&metadata.id).await.unwrap_err();

    assert!(matches!(error, CloudBackupError::Deferred(_)));
    assert_eq!(globals.cloud.uploaded_wallet_backup_count(), 0);
    assert!(matches!(
        Database::global().cloud_blob_sync_states.get(&record_id).unwrap(),
        Some(PersistedCloudBlobSyncState { state: PersistedCloudBlobState::Dirty(_), .. })
    ));
}

#[tokio::test(flavor = "current_thread")]
async fn upload_wallet_if_dirty_skips_fresh_uploading_state() {
    let _guard = async_test_lock().lock().await;
    cove_tokio::init();
    let globals = test_globals();
    let manager = init_manager();
    configure_enabled_cloud_backup(&manager, globals, 0);

    let metadata = xpub_only_wallet_metadata();
    persist_xpub_wallets(vec![metadata.clone()]);
    let record_id = cove_cspp::backup_data::wallet_record_id(metadata.id.as_ref());
    persist_uploading_blob_state(
        metadata.id.clone(),
        jiff::Timestamp::now().as_second().try_into().unwrap_or(0),
    );

    manager.do_upload_wallet_if_dirty(&metadata.id).await.unwrap();

    assert_eq!(globals.cloud.uploaded_wallet_backup_count(), 0);
    assert!(matches!(
        Database::global().cloud_blob_sync_states.get(&record_id).unwrap(),
        Some(PersistedCloudBlobSyncState {
            state: PersistedCloudBlobState::Uploading(CloudBlobUploadingState { .. }),
            ..
        })
    ));
}

#[tokio::test(flavor = "current_thread")]
async fn backup_wallets_preserves_newer_dirty_state() {
    let _guard = async_test_lock().lock().await;
    cove_tokio::init();
    let globals = test_globals();
    let manager = init_manager();
    configure_enabled_cloud_backup(&manager, globals, 0);

    let mut metadata = WalletMetadata::preview_new();
    metadata.wallet_type = WalletType::WatchOnly;
    Database::global()
        .wallets()
        .save_all_wallets(metadata.network, metadata.wallet_mode, vec![metadata.clone()])
        .unwrap();
    globals.cloud.change_wallet_on_next_upload(metadata.id.clone());

    manager.do_backup_wallets(&[metadata.clone()]).await.unwrap();

    let record_id = cove_cspp::backup_data::wallet_record_id(metadata.id.as_ref());
    assert_eq!(globals.cloud.uploaded_wallet_backup_count(), 1);
    assert!(matches!(
        Database::global().cloud_blob_sync_states.get(&record_id).unwrap(),
        Some(PersistedCloudBlobSyncState { state: PersistedCloudBlobState::Dirty(_), .. })
    ));
}

#[tokio::test(flavor = "current_thread")]
async fn pending_upload_verification_preserves_newer_dirty_state() {
    let _guard = async_test_lock().lock().await;
    cove_tokio::init();
    let globals = test_globals();
    let manager = init_manager();
    configure_enabled_cloud_backup(&manager, globals, 0);

    let metadata = WalletMetadata::preview_new();
    let namespace_id = CloudBackupKeychain::global().namespace_id().unwrap();
    let record_id = cove_cspp::backup_data::wallet_record_id(metadata.id.as_ref());
    Database::global()
        .cloud_blob_sync_states
        .set(&PersistedCloudBlobSyncState::wallet(
            namespace_id,
            metadata.id.clone(),
            record_id.clone(),
            PersistedCloudBlobState::UploadedPendingConfirmation(
                CloudBlobUploadedPendingConfirmationState {
                    revision_hash: "rev-1".into(),
                    uploaded_at: 10,
                    attempt_count: 0,
                    last_checked_at: None,
                },
            ),
        ))
        .unwrap();
    globals.cloud.dirty_wallet_on_next_backup_check(metadata.id.clone());

    let has_more_pending = verify_pending_uploads_once_for_test_async(&manager).await;

    assert!(!has_more_pending);
    assert!(matches!(
        Database::global().cloud_blob_sync_states.get(&record_id).unwrap(),
        Some(PersistedCloudBlobSyncState { state: PersistedCloudBlobState::Dirty(_), .. })
    ));
}

#[tokio::test(flavor = "current_thread")]
async fn deep_verify_fails_when_auto_sync_upload_fails() {
    let _guard = async_test_lock().lock().await;
    cove_tokio::init();
    let globals = test_globals();
    let manager = init_manager();
    let metadata = prepare_deep_verify_with_unsynced_wallet(&manager, globals);
    let record_id = cove_cspp::backup_data::wallet_record_id(metadata.id.as_ref());
    globals.cloud.fail_wallet_backup_upload("upload failed");

    let result = deep_verify_for_test(&manager, true).await;

    match result {
        DeepVerificationResult::Failed(DeepVerificationFailure::Retry {
            message, detail, ..
        }) => {
            assert_eq!(
                message,
                "failed to auto-sync missing wallet backups: cloud storage error: upload failed: upload failed"
            );
            let detail = detail.expect("expected detail on retry failure");
            assert_eq!(detail.needs_sync.len(), 1);
            assert_eq!(detail.needs_sync[0].record_id, record_id);
        }
        other => panic!("expected retry failure, got {other:?}"),
    }
}

#[tokio::test(flavor = "current_thread")]
async fn deep_verify_persists_partial_auto_sync_upload_before_later_wallet_fails() {
    let _guard = async_test_lock().lock().await;
    cove_tokio::init();
    let globals = test_globals();
    let manager = init_manager();
    let first_wallet = prepare_deep_verify_with_unsynced_wallet(&manager, globals);
    globals.cloud.set_reflect_uploaded_wallets_in_listing(true);
    globals.cloud.fail_wallet_backup_upload_after_successes(1, "upload failed");

    let mut second_wallet = WalletMetadata::preview_new();
    second_wallet.wallet_type = WalletType::WatchOnly;
    Database::global()
        .wallets()
        .save_all_wallets(
            first_wallet.network,
            first_wallet.wallet_mode,
            vec![first_wallet.clone(), second_wallet],
        )
        .unwrap();

    let result = deep_verify_for_test(&manager, true).await;

    let record_id = wallet_record_id(first_wallet.id.as_ref());
    match result {
        DeepVerificationResult::Failed(DeepVerificationFailure::Retry { message, .. }) => {
            assert!(message.contains("upload failed"), "{message}");
        }
        other => panic!("expected retry failure, got {other:?}"),
    }
    assert_eq!(globals.cloud.uploaded_wallet_backup_count(), 1);
    assert_eq!(Database::global().cloud_backup_state.get().unwrap().wallet_count(), Some(1));
    assert!(matches!(
        Database::global().cloud_blob_sync_states.get(&record_id).unwrap(),
        Some(PersistedCloudBlobSyncState {
            state: PersistedCloudBlobState::UploadedPendingConfirmation(_)
                | PersistedCloudBlobState::Confirmed(_),
            ..
        })
    ));
}

#[tokio::test(flavor = "current_thread")]
async fn deep_verify_awaits_upload_confirmation_when_relist_still_misses_uploaded_wallet() {
    let _guard = async_test_lock().lock().await;
    cove_tokio::init();
    let globals = test_globals();
    let manager = init_manager();
    let metadata = prepare_deep_verify_with_unsynced_wallet(&manager, globals);
    let record_id = cove_cspp::backup_data::wallet_record_id(metadata.id.as_ref());

    let result = deep_verify_for_test(&manager, true).await;

    match result {
        DeepVerificationResult::AwaitingUploadConfirmation(report) => {
            let detail = report.detail.expect("expected verification detail");
            assert_eq!(detail.up_to_date.len(), 1);
            assert!(detail.needs_sync.is_empty());
            assert_eq!(detail.up_to_date[0].record_id, record_id);
        }
        other => panic!("expected awaiting upload confirmation, got {other:?}"),
    }

    assert_eq!(globals.cloud.uploaded_wallet_backup_count(), 1);
    assert!(manager.pending_verification_completion().is_some());
    assert!(manager.has_pending_cloud_upload_verification());
}

#[tokio::test(flavor = "current_thread")]
async fn manual_verification_clears_interactive_state_when_awaiting_upload_confirmation() {
    let _guard = async_test_lock().lock().await;
    cove_tokio::init();
    let globals = test_globals();
    let manager = init_manager();
    let metadata = prepare_deep_verify_with_unsynced_wallet(&manager, globals);
    let record_id = cove_cspp::backup_data::wallet_record_id(metadata.id.as_ref());

    call!(manager.supervisor.start_verification(true)).await.unwrap();
    wait_for_test_condition(
        Duration::from_secs(2),
        "verification awaits upload confirmation",
        || manager.pending_verification_completion().is_some(),
    )
    .await;

    let state = manager.model_snapshot();
    assert!(matches!(state.verification, VerificationState::Idle));
    assert_eq!(state.pending_upload_verification, PendingUploadVerificationState::Confirming);
    assert!(manager.pending_verification_completion().is_some());

    let detail = state.detail.expect("expected verification detail");
    assert_eq!(detail.up_to_date.len(), 1);
    assert!(detail.needs_sync.is_empty());
    assert_eq!(detail.up_to_date[0].record_id, record_id);
}

#[tokio::test(flavor = "current_thread")]
async fn manual_verification_repairs_missing_master_key_wrapper_through_supervisor() {
    let _guard = async_test_lock().lock().await;
    cove_tokio::init();
    let globals = test_globals();
    let manager = init_manager();
    configure_enabled_cloud_backup(&manager, globals, 0);

    let namespace = CloudBackupKeychain::global().namespace_id().unwrap();
    globals.passkey.set_create_result(Ok(vec![1, 2, 3]));
    globals.passkey.set_authenticate_result(Ok(vec![7; 32]));

    call!(manager.supervisor.start_verification(true)).await.unwrap();

    wait_for_test_condition(Duration::from_secs(8), "verification wrapper repair finishes", || {
        manager.projected_exclusive_operation().is_none()
            && matches!(
                manager.model_snapshot().verification,
                VerificationState::Verified(DeepVerificationReport {
                    master_key_wrapper_repaired: true,
                    ..
                })
            )
    })
    .await;

    assert!(globals.cloud.has_master_key_backup(&namespace));
    assert!(matches!(
        Database::global().cloud_blob_sync_states.get(&cspp_master_key_record_id()).unwrap(),
        Some(PersistedCloudBlobSyncState {
            state: PersistedCloudBlobState::UploadedPendingConfirmation(_)
                | PersistedCloudBlobState::Confirmed(_),
            ..
        })
    ));
}

#[tokio::test(flavor = "current_thread")]
async fn manual_verification_loads_wallet_inventory_before_wrapper_repair() {
    let _guard = async_test_lock().lock().await;
    cove_tokio::init();
    let globals = test_globals();
    let manager = init_manager();
    configure_enabled_cloud_backup(&manager, globals, 0);

    let namespace = CloudBackupKeychain::global().namespace_id().unwrap();
    globals.cloud.fail_list_wallet_files("list failed");
    globals.passkey.set_create_result(Ok(vec![1, 2, 3]));

    call!(manager.supervisor.start_verification(true)).await.unwrap();

    wait_for_test_condition(
        Duration::from_secs(8),
        "verification fails before wrapper repair",
        || {
            matches!(
                manager.model_snapshot().verification,
                VerificationState::Failed(DeepVerificationFailure::Retry { .. })
            )
        },
    )
    .await;

    match manager.model_snapshot().verification {
        VerificationState::Failed(DeepVerificationFailure::Retry { message, .. }) => {
            assert!(message.contains("failed to list wallet backups"), "{message}");
        }
        other => panic!("expected retry failure, got {other:?}"),
    }
    assert_eq!(globals.passkey.create_count(), 0);
    assert!(!globals.cloud.has_master_key_backup(&namespace));
}

#[tokio::test(flavor = "current_thread")]
async fn deep_verify_repairs_stale_local_master_key_before_recreate_manifest() {
    let _guard = async_test_lock().lock().await;
    cove_tokio::init();
    let globals = test_globals();
    let manager = init_manager();

    reset_cloud_backup_test_state(&manager, globals);
    CONNECTIVITY_MANAGER.set_connection_state(true);

    let stale_master_key = cove_cspp::master_key::MasterKey::generate();
    let remote_master_key = cove_cspp::master_key::MasterKey::generate();
    let namespace = remote_master_key.namespace_id();
    let prf_key = [7u8; 32];
    let prf_salt = [9u8; 32];
    let encrypted_master =
        cove_cspp::master_key_crypto::encrypt_master_key(&remote_master_key, &prf_key, &prf_salt)
            .unwrap();

    globals
        .cloud
        .set_master_key_backup(namespace.clone(), serde_json::to_vec(&encrypted_master).unwrap());
    globals.cloud.fail_list_wallet_files_for_namespace(
        namespace.clone(),
        CloudStorageError::NotFound(namespace.clone()),
    );
    globals.passkey.set_discover_result(Ok(DiscoveredPasskeyResult {
        prf_output: prf_key.to_vec(),
        credential_id: vec![1, 2, 3],
    }));

    let keychain = Keychain::global().clone();
    CloudBackupKeychain::new(keychain.clone()).save_namespace_id(&namespace).unwrap();
    let cspp = cove_cspp::Cspp::new(keychain);
    cspp.save_master_key(&stale_master_key).unwrap();
    manager
        .persist_cloud_backup_state(
            &persisted_enabled_cloud_backup_state(Some(0)),
            "set cloud backup enabled for test",
        )
        .unwrap();
    manager.sync_persisted_state();

    let result = deep_verify_for_test(&manager, true).await;

    assert!(matches!(
        result,
        DeepVerificationResult::Failed(DeepVerificationFailure::RecreateManifest { .. })
    ));
    let repaired = cspp.load_master_key_from_store().unwrap().unwrap();
    assert_eq!(repaired.as_bytes(), remote_master_key.as_bytes());
}

#[tokio::test(flavor = "current_thread")]
async fn start_verification_dispatch_resumes_pending_upload_verification() {
    let _guard = async_test_lock().lock().await;
    cove_tokio::init();
    let globals = test_globals();
    let manager = init_manager();
    configure_enabled_cloud_backup(&manager, globals, 0);

    let namespace_id = CloudBackupKeychain::global().namespace_id().unwrap();
    persist_pending_master_key_confirmation(namespace_id.clone(), "pending");
    manager.replace_pending_verification_completion(PendingVerificationCompletion::new(
        DeepVerificationReport {
            master_key_wrapper_repaired: false,
            local_master_key_repaired: false,
            credential_recovered: false,
            wallets_verified: 0,
            wallets_failed: 0,
            wallets_unsupported: 0,
            detail: None,
        },
        namespace_id.clone(),
        vec![PendingVerificationUpload::master_key_wrapper()],
    ));
    globals
        .cloud
        .fail_master_key_download_authorization_required(namespace_id, "authorization required");

    manager.dispatch(CloudBackupManagerAction::StartVerification(
        CloudBackupVerificationSource::Settings,
    ));

    wait_for_test_condition(
        Duration::from_secs(1),
        "expected pending upload verification to pause on authorization",
        || {
            matches!(
                manager.model_snapshot().pending_upload_verification,
                PendingUploadVerificationState::BlockedOnAuthorization
            )
        },
    )
    .await;

    assert!(manager.pending_verification_completion().is_some());
    assert!(!matches!(manager.model_snapshot().verification, VerificationState::Verifying));
}

#[tokio::test(flavor = "current_thread")]
async fn pending_upload_verification_finalizes_awaiting_deep_verify() {
    let _guard = async_test_lock().lock().await;
    cove_tokio::init();
    let globals = test_globals();
    let manager = init_manager();
    let metadata = prepare_deep_verify_with_unsynced_wallet(&manager, globals);
    let record_id = cove_cspp::backup_data::wallet_record_id(metadata.id.as_ref());

    let result = deep_verify_for_test(&manager, true).await;

    assert!(matches!(result, DeepVerificationResult::AwaitingUploadConfirmation(_)));
    assert!(manager.pending_verification_completion().is_some());
    assert!(manager.has_pending_cloud_upload_verification());

    let has_more_pending = verify_pending_uploads_once_for_test_async(&manager).await;

    assert!(!has_more_pending);
    assert!(manager.pending_verification_completion().is_none());
    assert_eq!(
        manager.model_snapshot().pending_upload_verification,
        PendingUploadVerificationState::Idle
    );

    match manager.model_snapshot().verification {
        VerificationState::Verified(report) => {
            assert_eq!(report.wallets_verified, 1);
            assert_eq!(report.wallets_failed, 0);
            assert_eq!(report.wallets_unsupported, 0);

            let detail = report.detail.expect("expected verification detail");
            assert_eq!(detail.up_to_date.len(), 1);
            assert!(detail.needs_sync.is_empty());
            assert_eq!(detail.up_to_date[0].record_id, record_id);
        }
        other => {
            panic!("expected verified result after pending upload verification, got {other:?}")
        }
    }

    assert!(!manager.has_pending_cloud_upload_verification());
}

#[tokio::test(flavor = "current_thread")]
async fn pending_upload_verification_keeps_master_key_wrapper_hash_mismatch_pending() {
    let _guard = async_test_lock().lock().await;
    cove_tokio::init();
    let globals = test_globals();
    let manager = init_manager();
    configure_enabled_cloud_backup(&manager, globals, 0);

    let namespace_id = CloudBackupKeychain::global().namespace_id().unwrap();
    let expected_master_json = vec![1, 2, 3];
    let expected_revision = master_key_wrapper_revision_hash(&expected_master_json);
    persist_pending_master_key_confirmation(namespace_id.clone(), expected_revision);
    globals.cloud.set_master_key_backup(namespace_id.clone(), vec![4, 5, 6]);
    manager.replace_pending_verification_completion(PendingVerificationCompletion::new(
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
        vec![PendingVerificationUpload::master_key_wrapper()],
    ));

    let has_more_pending = verify_pending_uploads_once_for_test_async(&manager).await;

    assert!(has_more_pending);
    assert!(manager.pending_verification_completion().is_some());
    assert_eq!(
        manager.model_snapshot().pending_upload_verification,
        PendingUploadVerificationState::Confirming
    );
    assert!(matches!(manager.model_snapshot().verification, VerificationState::Idle));
}

#[tokio::test(flavor = "current_thread")]
async fn pending_upload_verification_refreshes_sync_health_to_all_uploaded() {
    let _guard = async_test_lock().lock().await;
    cove_tokio::init();
    let globals = test_globals();
    let manager = init_manager();
    configure_enabled_cloud_backup(&manager, globals, 1);
    let master_revision = seed_verifiable_cloud_master_key(globals);

    let namespace_id = CloudBackupKeychain::global().namespace_id().unwrap();
    let metadata = xpub_only_wallet_metadata();
    let record_id = wallet_record_id(metadata.id.as_ref());
    persist_xpub_wallets(vec![metadata.clone()]);
    let master_key = cove_cspp::Cspp::new(Keychain::global().clone())
        .load_master_key_from_store()
        .unwrap()
        .unwrap();
    let prepared = crate::manager::cloud_backup_manager::wallets::prepare_wallet_backup(
        &metadata,
        metadata.wallet_mode,
    )
    .await
    .unwrap();

    globals.cloud.set_wallet_backup(
        namespace_id.clone(),
        record_id.clone(),
        encrypted_wallet_backup_bytes(&metadata, &master_key, &prepared.revision_hash, 1).await,
    );
    globals
        .cloud
        .set_wallet_files(namespace_id.clone(), vec![wallet_filename_from_record_id(&record_id)]);
    persist_pending_master_key_confirmation(namespace_id.clone(), master_revision);
    manager
        .mark_blob_uploaded_pending_confirmation(
            &namespace_id,
            CloudBackupRecordKey::Wallet(metadata.id, record_id.clone()),
            prepared.revision_hash.clone(),
            jiff::Timestamp::now().as_second().try_into().unwrap_or(0),
        )
        .unwrap();
    manager.replace_pending_verification_completion(PendingVerificationCompletion::new(
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
        vec![
            PendingVerificationUpload::master_key_wrapper(),
            PendingVerificationUpload::new(record_id, prepared.revision_hash),
        ],
    ));

    assert_eq!(manager.compute_sync_health().await, CloudSyncHealth::Uploading,);

    let has_more_pending = verify_pending_uploads_once_for_test_async(&manager).await;

    assert!(!has_more_pending);
    assert!(manager.pending_verification_completion().is_none());
    assert_eq!(manager.compute_sync_health().await, CloudSyncHealth::AllUploaded,);

    for _ in 0..20 {
        if manager.model_snapshot().sync_health == CloudSyncHealth::AllUploaded {
            break;
        }

        tokio::time::sleep(Duration::from_millis(10)).await;
    }

    assert_eq!(manager.model_snapshot().sync_health, CloudSyncHealth::AllUploaded);
}

#[tokio::test(flavor = "current_thread")]
async fn pending_upload_verification_survives_restart() {
    let _guard = async_test_lock().lock().await;
    cove_tokio::init();
    let globals = test_globals();
    let manager = init_manager();
    let metadata = prepare_deep_verify_with_unsynced_wallet(&manager, globals);
    let record_id = cove_cspp::backup_data::wallet_record_id(metadata.id.as_ref());

    let result = deep_verify_for_test(&manager, true).await;

    assert!(matches!(result, DeepVerificationResult::AwaitingUploadConfirmation(_)));
    assert!(manager.pending_verification_completion().is_some());

    let restarted_manager = init_manager();

    assert!(restarted_manager.pending_verification_completion().is_some());
    restarted_manager.sync_persisted_state();
    let has_more_pending = verify_pending_uploads_once_for_test_async(&restarted_manager).await;

    assert!(!has_more_pending);
    assert!(restarted_manager.pending_verification_completion().is_none());
    match restarted_manager.model_snapshot().verification {
        VerificationState::Verified(report) => {
            assert_eq!(report.wallets_verified, 1);
            assert_eq!(report.wallets_failed, 0);
            assert_eq!(report.wallets_unsupported, 0);

            let detail = report.detail.expect("expected verification detail");
            assert_eq!(detail.up_to_date.len(), 1);
            assert!(detail.needs_sync.is_empty());
            assert_eq!(detail.up_to_date[0].record_id, record_id);
        }
        other => {
            panic!("expected verified result after restart, got {other:?}")
        }
    }
    assert!(!restarted_manager.has_pending_cloud_upload_verification());
}

#[tokio::test(flavor = "current_thread")]
async fn pending_upload_verification_retries_until_expected_revision_is_readable() {
    let _guard = async_test_lock().lock().await;
    cove_tokio::init();
    let globals = test_globals();
    let manager = init_manager();
    let metadata = prepare_deep_verify_with_unsynced_wallet(&manager, globals);
    let namespace = CloudBackupKeychain::global().namespace_id().unwrap();
    let record_id = cove_cspp::backup_data::wallet_record_id(metadata.id.as_ref());
    let master_key = cove_cspp::Cspp::new(Keychain::global().clone())
        .load_master_key_from_store()
        .unwrap()
        .unwrap();
    let current_revision = crate::manager::cloud_backup_manager::wallets::prepare_wallet_backup(
        &metadata,
        metadata.wallet_mode,
    )
    .await
    .unwrap()
    .revision_hash;

    let result = deep_verify_for_test(&manager, true).await;

    assert!(matches!(result, DeepVerificationResult::AwaitingUploadConfirmation(_)));
    globals.cloud.set_wallet_backup_download_override(
        namespace.clone(),
        record_id.clone(),
        encrypted_wallet_backup_bytes(&metadata, &master_key, "stale-revision", 1).await,
    );

    let has_more_pending = verify_pending_uploads_once_for_test_async(&manager).await;

    assert!(has_more_pending);
    assert!(manager.pending_verification_completion().is_some());
    assert!(manager.has_pending_cloud_upload_verification());
    assert!(matches!(
        Database::global().cloud_blob_sync_states.get(&record_id).unwrap(),
        Some(PersistedCloudBlobSyncState {
            state: PersistedCloudBlobState::UploadedPendingConfirmation(_)
                | PersistedCloudBlobState::Confirmed(_),
            ..
        })
    ));
    assert!(!matches!(
        manager.model_snapshot().verification,
        VerificationState::Verified(_) | VerificationState::Failed(_)
    ));

    globals.cloud.set_wallet_backup_download_override(
        namespace,
        record_id,
        encrypted_wallet_backup_bytes(&metadata, &master_key, &current_revision, 1).await,
    );

    let has_more_pending = verify_pending_uploads_once_for_test_async(&manager).await;

    assert!(!has_more_pending);
    assert!(manager.pending_verification_completion().is_none());
    match manager.model_snapshot().verification {
        VerificationState::Verified(report) => {
            assert_eq!(report.wallets_verified, 1);
            assert_eq!(report.wallets_failed, 0);
            assert_eq!(report.wallets_unsupported, 0);
        }
        other => panic!("expected verified result after retry, got {other:?}"),
    }
    assert!(!manager.has_pending_cloud_upload_verification());
}

#[tokio::test(flavor = "current_thread")]
async fn pending_upload_verification_accepts_newer_revision_after_wallet_changes() {
    let _guard = async_test_lock().lock().await;
    cove_tokio::init();
    let globals = test_globals();
    let manager = init_manager();
    let metadata = prepare_deep_verify_with_unsynced_wallet(&manager, globals);
    let record_id = cove_cspp::backup_data::wallet_record_id(metadata.id.as_ref());
    globals.cloud.change_wallet_on_next_upload(metadata.id.clone());

    let result = deep_verify_for_test(&manager, true).await;

    assert!(matches!(result, DeepVerificationResult::AwaitingUploadConfirmation(_)));
    assert!(manager.pending_verification_completion().is_some());
    assert!(manager.has_pending_cloud_upload_verification());

    manager.do_upload_wallet_if_dirty(&metadata.id).await.unwrap();

    assert!(matches!(
        Database::global().cloud_blob_sync_states.get(&record_id).unwrap(),
        Some(PersistedCloudBlobSyncState {
            state: PersistedCloudBlobState::UploadedPendingConfirmation(_)
                | PersistedCloudBlobState::Confirmed(_),
            ..
        })
    ));

    let has_more_pending = verify_pending_uploads_once_for_test_async(&manager).await;

    assert!(!has_more_pending);
    assert!(manager.pending_verification_completion().is_none());
    assert!(matches!(
        Database::global().cloud_blob_sync_states.get(&record_id).unwrap(),
        Some(PersistedCloudBlobSyncState { state: PersistedCloudBlobState::Confirmed(_), .. })
    ));

    match manager.model_snapshot().verification {
        VerificationState::Verified(report) => {
            assert_eq!(report.wallets_verified, 1);
            assert_eq!(report.wallets_failed, 0);
            assert_eq!(report.wallets_unsupported, 0);

            let detail = report.detail.expect("expected verification detail");
            assert_eq!(detail.up_to_date.len(), 1);
            assert!(detail.needs_sync.is_empty());
            assert_eq!(detail.up_to_date[0].record_id, record_id);
        }
        other => panic!("expected verified result after newer upload, got {other:?}"),
    }

    assert!(!manager.has_pending_cloud_upload_verification());
}

#[tokio::test(flavor = "current_thread")]
async fn pending_upload_verification_marks_invalid_wallet_json_failed() {
    let _guard = async_test_lock().lock().await;
    cove_tokio::init();
    let globals = test_globals();
    let manager = init_manager();
    let metadata = prepare_deep_verify_with_unsynced_wallet(&manager, globals);
    let namespace = CloudBackupKeychain::global().namespace_id().unwrap();
    let record_id = cove_cspp::backup_data::wallet_record_id(metadata.id.as_ref());

    let result = deep_verify_for_test(&manager, true).await;

    assert!(matches!(result, DeepVerificationResult::AwaitingUploadConfirmation(_)));
    globals.cloud.set_wallet_backup(namespace, record_id.clone(), b"{".to_vec());

    let has_more_pending = verify_pending_uploads_once_for_test_async(&manager).await;

    assert!(!has_more_pending);
    assert!(manager.pending_verification_completion().is_none());

    match manager.model_snapshot().verification {
        VerificationState::Verified(report) => {
            assert_eq!(report.wallets_verified, 0);
            assert_eq!(report.wallets_failed, 1);
            assert_eq!(report.wallets_unsupported, 0);
        }
        other => {
            panic!("expected verified result after pending upload verification, got {other:?}")
        }
    }

    assert!(!manager.has_pending_cloud_upload_verification());
}

#[tokio::test(flavor = "current_thread")]
async fn pending_upload_verification_marks_terminal_live_upload_failures_failed() {
    let _guard = async_test_lock().lock().await;
    cove_tokio::init();
    let globals = test_globals();
    let manager = init_manager();
    configure_enabled_cloud_backup(&manager, globals, 0);

    let metadata = xpub_only_wallet_metadata();
    let namespace_id = CloudBackupKeychain::global().namespace_id().unwrap();
    let record_id = cove_cspp::backup_data::wallet_record_id(metadata.id.as_ref());
    persist_xpub_wallets(vec![metadata.clone()]);
    Database::global()
        .cloud_blob_sync_states
        .set(&PersistedCloudBlobSyncState::wallet(
            namespace_id.clone(),
            metadata.id,
            record_id.clone(),
            PersistedCloudBlobState::UploadedPendingConfirmation(
                CloudBlobUploadedPendingConfirmationState {
                    revision_hash: "rev-1".into(),
                    uploaded_at: 10,
                    attempt_count: 0,
                    last_checked_at: None,
                },
            ),
        ))
        .unwrap();
    globals.cloud.set_wallet_backup(namespace_id, record_id.clone(), b"{".to_vec());

    let has_more_pending = verify_pending_uploads_once_for_test_async(&manager).await;

    assert!(!has_more_pending);
    assert!(!manager.has_pending_cloud_upload_verification());
    assert!(matches!(
        Database::global().cloud_blob_sync_states.get(&record_id).unwrap(),
        Some(PersistedCloudBlobSyncState {
            state: PersistedCloudBlobState::Failed(CloudBlobFailedState { retryable: false, .. }),
            ..
        })
    ));
}

#[tokio::test(flavor = "current_thread")]
async fn failed_pending_upload_without_remote_backup_remains_pending() {
    let _guard = async_test_lock().lock().await;
    cove_tokio::init();
    let globals = test_globals();
    let manager = init_manager();
    let metadata = prepare_deep_verify_with_unsynced_wallet(&manager, globals);
    let namespace_id = CloudBackupKeychain::global().namespace_id().unwrap();
    let record_id = cove_cspp::backup_data::wallet_record_id(metadata.id.as_ref());

    let result = deep_verify_for_test(&manager, true).await;

    assert!(matches!(result, DeepVerificationResult::AwaitingUploadConfirmation(_)));
    assert!(manager.pending_verification_completion().is_some());

    CloudStorage::global_silent_client()
        .delete_wallet_backup(namespace_id.clone(), record_id.clone())
        .await
        .unwrap();
    Database::global()
        .cloud_blob_sync_states
        .set(&PersistedCloudBlobSyncState::wallet(
            namespace_id,
            metadata.id,
            record_id.clone(),
            PersistedCloudBlobState::Failed(CloudBlobFailedState {
                revision_hash: Some("rev-1".into()),
                retryable: false,
                error: "terminal upload failure".into(),
                issue: None,
                failed_at: 10,
            }),
        ))
        .unwrap();

    let has_more_pending = verify_pending_uploads_once_for_test_async(&manager).await;

    assert!(has_more_pending);
    assert!(manager.pending_verification_completion().is_some());
    assert!(manager.has_pending_cloud_upload_verification());
}

#[tokio::test(flavor = "current_thread")]
async fn deep_verify_preserves_unsupported_remote_wallet_backups() {
    let _guard = async_test_lock().lock().await;
    cove_tokio::init();
    let globals = test_globals();
    let manager = init_manager();
    let metadata = prepare_deep_verify_with_unsynced_wallet(&manager, globals);
    let keychain = Keychain::global();
    let namespace = CloudBackupKeychain::new(keychain.clone()).namespace_id().unwrap();
    let master_key =
        cove_cspp::Cspp::new(keychain.clone()).load_master_key_from_store().unwrap().unwrap();
    let record_id = cove_cspp::backup_data::wallet_record_id(metadata.id.as_ref());
    globals.cloud.set_wallet_backup(
        namespace.clone(),
        record_id.clone(),
        encrypted_wallet_backup_bytes(&metadata, &master_key, "unsupported-revision", 2).await,
    );
    globals.cloud.set_wallet_files(namespace, vec![wallet_filename_from_record_id(&record_id)]);

    let result = deep_verify_for_test(&manager, true).await;

    match result {
        DeepVerificationResult::Verified(report) => {
            assert_eq!(report.wallets_verified, 0);
            assert_eq!(report.wallets_failed, 0);
            assert_eq!(report.wallets_unsupported, 1);
        }
        other => panic!("expected verified result, got {other:?}"),
    }

    assert_eq!(globals.cloud.uploaded_wallet_backup_count(), 0);
    assert!(manager.pending_verification_completion().is_none());
}

#[tokio::test(flavor = "current_thread")]
async fn deep_verify_retries_when_remote_wallet_truth_is_unknown() {
    let _guard = async_test_lock().lock().await;
    cove_tokio::init();
    let globals = test_globals();
    let manager = init_manager();
    let metadata = prepare_deep_verify_with_unsynced_wallet(&manager, globals);
    let namespace = CloudBackupKeychain::global().namespace_id().unwrap();
    let record_id = cove_cspp::backup_data::wallet_record_id(metadata.id.as_ref());
    globals
        .cloud
        .set_wallet_files(namespace.clone(), vec![wallet_filename_from_record_id(&record_id)]);
    globals.cloud.set_wallet_backup(namespace, record_id.clone(), b"{".to_vec());

    let result = deep_verify_for_test(&manager, true).await;

    match result {
        DeepVerificationResult::Failed(DeepVerificationFailure::Retry {
            message, detail, ..
        }) => {
            assert_eq!(message, "failed to refresh remote wallet truth for some wallets");

            let detail = detail.expect("expected verification detail");
            assert_eq!(detail.needs_sync.len(), 1);
            assert_eq!(detail.needs_sync[0].record_id, record_id);
            assert_eq!(
                detail.needs_sync[0].sync_status,
                CloudBackupWalletStatus::RemoteStateUnknown
            );
        }
        other => panic!("expected retry failure, got {other:?}"),
    }

    assert_eq!(globals.cloud.uploaded_wallet_backup_count(), 0);
}

#[tokio::test(flavor = "current_thread")]
async fn deep_verify_succeeds_after_auto_sync_relist_confirms_wallet() {
    let _guard = async_test_lock().lock().await;
    cove_tokio::init();
    let globals = test_globals();
    let manager = init_manager();
    let metadata = prepare_deep_verify_with_unsynced_wallet(&manager, globals);
    globals.cloud.set_reflect_uploaded_wallets_in_listing(true);

    let result = deep_verify_for_test(&manager, true).await;

    match result {
        DeepVerificationResult::Verified(report) => {
            let detail = report.detail.expect("expected verification detail");
            assert_eq!(detail.up_to_date.len(), 1);
            assert!(detail.needs_sync.is_empty());
            assert_eq!(
                detail.up_to_date[0].record_id,
                cove_cspp::backup_data::wallet_record_id(metadata.id.as_ref())
            );
        }
        other => panic!("expected verified result, got {other:?}"),
    }

    assert_eq!(globals.cloud.uploaded_wallet_backup_count(), 1);
    let record_id = cove_cspp::backup_data::wallet_record_id(metadata.id.as_ref());
    assert!(matches!(
        Database::global().cloud_blob_sync_states.get(&record_id).unwrap(),
        Some(PersistedCloudBlobSyncState { state: PersistedCloudBlobState::Confirmed(_), .. })
    ));
    assert!(!manager.has_pending_cloud_upload_verification());
}

#[tokio::test(flavor = "current_thread")]
async fn deep_verify_awaits_upload_confirmation_when_remote_revision_is_stale() {
    let _guard = async_test_lock().lock().await;
    cove_tokio::init();
    let globals = test_globals();
    let manager = init_manager();
    let metadata = prepare_deep_verify_with_unsynced_wallet(&manager, globals);
    let namespace = CloudBackupKeychain::global().namespace_id().unwrap();
    let record_id = cove_cspp::backup_data::wallet_record_id(metadata.id.as_ref());
    let master_key = cove_cspp::Cspp::new(Keychain::global().clone())
        .load_master_key_from_store()
        .unwrap()
        .unwrap();
    globals.cloud.set_reflect_uploaded_wallets_in_listing(true);
    globals.cloud.set_wallet_backup_download_override(
        namespace,
        record_id.clone(),
        encrypted_wallet_backup_bytes(&metadata, &master_key, "stale-revision", 1).await,
    );

    let result = deep_verify_for_test(&manager, true).await;

    match result {
        DeepVerificationResult::AwaitingUploadConfirmation(report) => {
            let detail = report.detail.expect("expected verification detail");
            assert!(detail.up_to_date.is_empty());
            assert_eq!(detail.needs_sync.len(), 1);
            assert_eq!(detail.needs_sync[0].record_id, record_id);
        }
        other => panic!("expected awaiting upload confirmation, got {other:?}"),
    }

    assert!(manager.pending_verification_completion().is_some());
    assert!(matches!(
        Database::global().cloud_blob_sync_states.get(&record_id).unwrap(),
        Some(PersistedCloudBlobSyncState {
            state: PersistedCloudBlobState::UploadedPendingConfirmation(_),
            ..
        })
    ));
    assert!(manager.has_pending_cloud_upload_verification());
}

#[tokio::test(flavor = "current_thread")]
async fn discard_pending_enable_clears_pending_session_and_local_master_key() {
    let _guard = async_test_lock().lock().await;
    cove_tokio::init();
    let globals = test_globals();
    let manager = init_manager();

    reset_cloud_backup_test_state(&manager, globals);

    let master_key = cove_cspp::master_key::MasterKey::generate();
    let cspp = cove_cspp::Cspp::new(Keychain::global().clone());
    cspp.save_master_key(&master_key).unwrap();
    replace_pending_enable_session_for_test(
        &manager,
        pending_enable_awaiting_confirmation(
            master_key,
            UnpersistedPrfKey {
                prf_key: [7; 32],
                prf_salt: [9; 32],
                credential_id: vec![1, 2, 3],
                provider_hint: None,
            },
            CloudBackupEnableContext::settings_manual(),
        ),
    )
    .await;

    manager.discard_pending_enable_cloud_backup();

    assert!(take_pending_enable_session_for_test(&manager).await.is_none());
    assert!(cspp.load_master_key_from_store().unwrap().is_none());
}

#[tokio::test(flavor = "current_thread")]
async fn discard_pending_enable_retry_upload_deletes_remote_master_key() {
    let _guard = async_test_lock().lock().await;
    cove_tokio::init();
    let globals = test_globals();
    let manager = init_manager();

    reset_cloud_backup_test_state(&manager, globals);

    let master_key = cove_cspp::master_key::MasterKey::generate();
    let namespace = master_key.namespace_id();
    let cspp = cove_cspp::Cspp::new(Keychain::global().clone());
    cspp.save_master_key(&master_key).unwrap();
    globals.cloud.set_master_key_backup(namespace.clone(), vec![1, 2, 3]);
    replace_pending_enable_session_for_test(
        &manager,
        PendingEnableSession::retry_upload(
            master_key,
            UnpersistedPrfKey {
                prf_key: [7; 32],
                prf_salt: [9; 32],
                credential_id: vec![1, 2, 3],
                provider_hint: None,
            },
            CloudBackupEnableContext::settings_manual(),
        ),
    )
    .await;

    manager.discard_pending_enable_cloud_backup();

    wait_for_test_condition(
        Duration::from_secs(1),
        "remote master key backup should be deleted",
        || !globals.cloud.has_master_key_backup(&namespace),
    )
    .await;

    assert!(cspp.load_master_key_from_store().unwrap().is_none());
}

#[test]
fn clear_in_process_state_for_local_reset_clears_enable_state() {
    let _guard = test_lock().lock();
    cove_tokio::init();
    let globals = test_globals();
    let manager = init_manager();

    reset_cloud_backup_test_state(&manager, globals);
    manager.apply_enable_outcome(CloudBackupEnableOutcome::AwaitingSavedPasskeyConfirmation(
        SavedPasskeyConfirmationMode::Manual,
    ));

    manager.clear_in_process_state_for_local_reset();

    assert_eq!(manager.model_snapshot().enable_state, CloudBackupEnableState::Idle);
}

#[tokio::test(flavor = "current_thread")]
async fn enable_preserves_awaiting_force_new_session() {
    let _guard = async_test_lock().lock().await;
    cove_tokio::init();
    let globals = test_globals();
    let manager = init_manager();

    reset_cloud_backup_test_state(&manager, globals);
    CONNECTIVITY_MANAGER.set_connection_state(true);
    let existing_master_key = cove_cspp::master_key::MasterKey::generate();
    let existing_namespace = existing_master_key.namespace_id();
    let encrypted_master =
        cove_cspp::master_key_crypto::encrypt_master_key(&existing_master_key, &[7; 32], &[9; 32])
            .unwrap();
    globals.cloud.set_wallet_files(existing_namespace.clone(), vec!["wallet-1.json".into()]);
    globals
        .cloud
        .set_master_key_backup(existing_namespace, serde_json::to_vec(&encrypted_master).unwrap());
    globals.passkey.set_discover_result(Err(PasskeyError::UserCancelled));

    let master_key = cove_cspp::master_key::MasterKey::generate();
    let expected_namespace = master_key.namespace_id();
    let expected_credential_id = vec![1, 2, 3];
    replace_pending_enable_session_for_test(
        &manager,
        pending_enable_awaiting_confirmation(
            master_key,
            UnpersistedPrfKey {
                prf_key: [7; 32],
                prf_salt: [9; 32],
                credential_id: expected_credential_id.clone(),
                provider_hint: None,
            },
            CloudBackupEnableContext::settings_manual(),
        ),
    )
    .await;

    enable_cloud_backup_create_new(&manager).await.unwrap();

    let pending = take_pending_enable_session_for_test(&manager).await.unwrap();
    let (pending_master_key, pending_passkey) = pending.into_ready_parts().unwrap();
    assert_eq!(pending_master_key.namespace_id(), expected_namespace);
    assert_eq!(pending_passkey.credential_id, expected_credential_id);
}

#[tokio::test(flavor = "current_thread")]
async fn enable_create_new_preserves_awaiting_force_new_session() {
    let _guard = async_test_lock().lock().await;
    cove_tokio::init();
    let globals = test_globals();
    let manager = init_manager();

    reset_cloud_backup_test_state(&manager, globals);
    CONNECTIVITY_MANAGER.set_connection_state(true);
    globals.passkey.set_discover_result(Err(PasskeyError::UserCancelled));

    let master_key = cove_cspp::master_key::MasterKey::generate();
    let expected_namespace = master_key.namespace_id();
    let expected_credential_id = vec![1, 2, 3];
    replace_pending_enable_session_for_test(
        &manager,
        pending_enable_awaiting_confirmation(
            master_key,
            UnpersistedPrfKey {
                prf_key: [7; 32],
                prf_salt: [9; 32],
                credential_id: expected_credential_id.clone(),
                provider_hint: None,
            },
            CloudBackupEnableContext::settings_manual(),
        ),
    )
    .await;

    enable_cloud_backup_create_new(&manager).await.unwrap();

    let pending = take_pending_enable_session_for_test(&manager).await.unwrap();
    let (pending_master_key, pending_passkey) = pending.into_ready_parts().unwrap();
    assert_eq!(pending_master_key.namespace_id(), expected_namespace);
    assert_eq!(pending_passkey.credential_id, expected_credential_id);
}

#[tokio::test(flavor = "current_thread")]
async fn enable_no_discovery_preserves_awaiting_force_new_session() {
    let _guard = async_test_lock().lock().await;
    cove_tokio::init();
    let globals = test_globals();
    let manager = init_manager();

    reset_cloud_backup_test_state(&manager, globals);
    CONNECTIVITY_MANAGER.set_connection_state(true);
    globals.passkey.set_create_result(Err(PasskeyError::UserCancelled));

    let master_key = cove_cspp::master_key::MasterKey::generate();
    let expected_namespace = master_key.namespace_id();
    let expected_credential_id = vec![1, 2, 3];
    replace_pending_enable_session_for_test(
        &manager,
        pending_enable_awaiting_confirmation(
            master_key,
            UnpersistedPrfKey {
                prf_key: [7; 32],
                prf_salt: [9; 32],
                credential_id: expected_credential_id.clone(),
                provider_hint: None,
            },
            CloudBackupEnableContext::settings_manual(),
        ),
    )
    .await;

    let create_count = globals.passkey.create_count();

    enable_cloud_backup_no_discovery(&manager).await.unwrap();

    assert_eq!(globals.passkey.create_count(), create_count);
    assert_eq!(manager.current_status(), CloudBackupStatus::Enabling);
    match manager.model_snapshot().root_prompt {
        CloudBackupRootPrompt::ExistingBackupFound(context, _) => {
            assert_eq!(context, CloudBackupEnableContext::settings_manual());
        }
        other => panic!("expected existing backup prompt, got {other:?}"),
    }

    let pending = take_pending_enable_session_for_test(&manager).await.unwrap();
    let (pending_master_key, pending_passkey) = pending.into_ready_parts().unwrap();
    assert_eq!(pending_master_key.namespace_id(), expected_namespace);
    assert_eq!(pending_passkey.credential_id, expected_credential_id);
}

#[tokio::test(flavor = "current_thread")]
async fn force_new_after_other_namespace_enter_detail_reuses_runtime_authorization() {
    let _guard = async_test_lock().lock().await;
    cove_tokio::init();
    let globals = test_globals();
    let manager = init_manager();

    reset_cloud_backup_test_state(&manager, globals);
    CONNECTIVITY_MANAGER.set_connection_state(true);

    let existing_master_key = cove_cspp::master_key::MasterKey::generate();
    let existing_namespace = existing_master_key.namespace_id();
    let encrypted_existing_master =
        cove_cspp::master_key_crypto::encrypt_master_key(&existing_master_key, &[7; 32], &[9; 32])
            .unwrap();
    globals.cloud.set_wallet_files(existing_namespace.clone(), vec!["wallet-1.json".into()]);
    globals.cloud.set_master_key_backup(
        existing_namespace,
        serde_json::to_vec(&encrypted_existing_master).unwrap(),
    );
    globals.passkey.set_create_result(Ok(vec![1, 2, 3]));
    globals.passkey.set_authenticate_result(Ok(vec![7; 32]));

    enable_cloud_backup_no_discovery(&manager).await.unwrap();

    assert_eq!(globals.passkey.create_count(), 0);
    assert_eq!(globals.passkey.authenticate_count(), 0);
    assert_eq!(globals.passkey.discover_count(), 0);
    assert_eq!(manager.current_status(), CloudBackupStatus::Enabling);
    match manager.model_snapshot().root_prompt {
        CloudBackupRootPrompt::ExistingBackupFound(context, _) => {
            assert_eq!(context, CloudBackupEnableContext::settings_manual());
        }
        other => panic!("expected existing backup prompt, got {other:?}"),
    }
    assert!(take_pending_enable_session_for_test(&manager).await.is_none());

    enable_cloud_backup_no_discovery(&manager).await.unwrap();

    assert_eq!(globals.passkey.create_count(), 0);
    assert_eq!(globals.passkey.authenticate_count(), 0);
    assert_eq!(globals.passkey.discover_count(), 0);
    assert!(take_pending_enable_session_for_test(&manager).await.is_none());

    enable_cloud_backup_force_new(&manager).await.unwrap();
    confirm_saved_passkey_session(&manager).await;

    assert!(take_pending_enable_session_for_test(&manager).await.is_none());
    assert_eq!(manager.current_status(), CloudBackupStatus::Enabled);

    globals.passkey.set_discover_result(Err(PasskeyError::UserCancelled));
    let create_count = globals.passkey.create_count();
    let authenticate_count = globals.passkey.authenticate_count();
    let discover_count = globals.passkey.discover_count();

    call!(manager.supervisor.start_enter_detail()).await.unwrap();
    tokio::time::sleep(Duration::from_millis(50)).await;

    assert_eq!(globals.passkey.create_count(), create_count);
    assert_eq!(globals.passkey.authenticate_count(), authenticate_count);
    assert_eq!(globals.passkey.discover_count(), discover_count);
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn force_new_after_existing_backup_prompt_registers_without_discovery() {
    let _guard = async_test_lock().lock().await;
    cove_tokio::init();
    let globals = test_globals();
    let manager = init_manager();

    reset_cloud_backup_test_state(&manager, globals);
    CONNECTIVITY_MANAGER.set_connection_state(true);

    let existing_master_key = cove_cspp::master_key::MasterKey::generate();
    let existing_namespace = existing_master_key.namespace_id();
    let encrypted_existing_master =
        cove_cspp::master_key_crypto::encrypt_master_key(&existing_master_key, &[7; 32], &[9; 32])
            .unwrap();
    globals.cloud.set_wallet_files(existing_namespace.clone(), vec!["wallet-1.json".into()]);
    globals.cloud.set_master_key_backup(
        existing_namespace,
        serde_json::to_vec(&encrypted_existing_master).unwrap(),
    );
    globals.passkey.set_create_result(Ok(vec![1, 2, 3]));
    globals.passkey.set_discover_result(Err(PasskeyError::UserCancelled));

    enable_cloud_backup_no_discovery(&manager).await.unwrap();

    assert_eq!(globals.passkey.create_count(), 0);
    assert_eq!(globals.passkey.discover_count(), 0);
    assert!(matches!(
        manager.model_snapshot().root_prompt,
        CloudBackupRootPrompt::ExistingBackupFound(_, _)
    ));

    enable_cloud_backup_force_new(&manager).await.unwrap();

    assert_eq!(globals.passkey.create_count(), 1);
    assert_eq!(globals.passkey.discover_count(), 0);
    assert_eq!(
        manager.model_snapshot().enable_state,
        CloudBackupEnableState::AwaitingSavedPasskeyConfirmation(
            SavedPasskeyConfirmationMode::Manual,
        )
    );
}

#[tokio::test(flavor = "current_thread")]
async fn existing_backup_prompt_preserves_onboarding_enable_context() {
    let _guard = async_test_lock().lock().await;
    cove_tokio::init();
    let globals = test_globals();
    let manager = init_manager();

    reset_cloud_backup_test_state(&manager, globals);
    CONNECTIVITY_MANAGER.set_connection_state(true);

    let existing_master_key = cove_cspp::master_key::MasterKey::generate();
    let existing_namespace = existing_master_key.namespace_id();
    let encrypted_existing_master =
        cove_cspp::master_key_crypto::encrypt_master_key(&existing_master_key, &[7; 32], &[9; 32])
            .unwrap();
    globals.cloud.set_wallet_files(existing_namespace.clone(), vec!["wallet-1.json".into()]);
    globals.cloud.set_master_key_backup(
        existing_namespace,
        serde_json::to_vec(&encrypted_existing_master).unwrap(),
    );
    globals.passkey.set_create_result(Ok(vec![1, 2, 3]));

    let context = CloudBackupEnableContext {
        saved_passkey_confirmation: SavedPasskeyConfirmationMode::Automatic,
        verification_source: CloudBackupVerificationSource::Onboarding,
    };
    enable_cloud_backup_no_discovery_with_context(&manager, context).await.unwrap();

    assert_eq!(globals.passkey.create_count(), 0);
    assert_eq!(globals.passkey.authenticate_count(), 0);
    assert_eq!(globals.passkey.discover_count(), 0);
    assert!(take_pending_enable_session_for_test(&manager).await.is_none());

    match manager.model_snapshot().root_prompt {
        CloudBackupRootPrompt::ExistingBackupFound(prompt_context, _) => {
            assert_eq!(prompt_context, context);
        }
        other => panic!("expected existing backup prompt, got {other:?}"),
    }

    enable_cloud_backup_force_new_with_context(&manager, context).await.unwrap();

    assert_eq!(
        manager.model_snapshot().enable_state,
        CloudBackupEnableState::AwaitingSavedPasskeyConfirmation(
            SavedPasskeyConfirmationMode::Automatic,
        )
    );
}

#[tokio::test(flavor = "current_thread")]
async fn detail_entry_after_restart_without_active_authorization_prompts_normally() {
    let _guard = async_test_lock().lock().await;
    cove_tokio::init();
    let globals = test_globals();
    let manager = init_manager();

    reset_cloud_backup_test_state(&manager, globals);
    CONNECTIVITY_MANAGER.set_connection_state(true);
    globals.passkey.set_create_result(Ok(vec![1, 2, 3]));
    globals.passkey.set_authenticate_result(Ok(vec![7; 32]));

    enable_cloud_backup_no_discovery(&manager).await.unwrap();
    confirm_saved_passkey_session(&manager).await;

    assert_eq!(manager.current_status(), CloudBackupStatus::Enabled);

    let restarted_manager = init_manager();
    restarted_manager.sync_persisted_state();
    restarted_manager.clear_pending_verification_completion();
    restarted_manager.reconcile_pending_upload_verification(PendingUploadVerificationState::Idle);
    restarted_manager.apply_verification_outcome(CloudBackupVerificationOutcome::Idle);
    Database::global().cloud_blob_sync_states.delete_all().unwrap();
    globals.passkey.set_discover_result(Err(PasskeyError::UserCancelled));

    let discover_count = globals.passkey.discover_count();

    call!(restarted_manager.supervisor.start_enter_detail()).await.unwrap();
    wait_for_discover_count(globals, discover_count + 1).await;

    assert_eq!(globals.passkey.discover_count(), discover_count + 1);
}

#[tokio::test(flavor = "current_thread")]
async fn enable_force_new_consumes_staged_session() {
    let _guard = async_test_lock().lock().await;
    cove_tokio::init();
    let globals = test_globals();
    let manager = init_manager();

    reset_cloud_backup_test_state(&manager, globals);
    CONNECTIVITY_MANAGER.set_connection_state(true);

    replace_pending_enable_session_for_test(
        &manager,
        pending_enable_awaiting_confirmation(
            cove_cspp::master_key::MasterKey::generate(),
            UnpersistedPrfKey {
                prf_key: [7; 32],
                prf_salt: [9; 32],
                credential_id: vec![1, 2, 3],
                provider_hint: None,
            },
            CloudBackupEnableContext::settings_manual(),
        ),
    )
    .await;

    enable_cloud_backup_force_new(&manager).await.unwrap();

    assert!(take_pending_enable_session_for_test(&manager).await.is_none());
    assert_eq!(manager.current_status(), CloudBackupStatus::Enabled);
}

#[tokio::test(flavor = "current_thread")]
async fn cancelled_enable_create_new_rolls_back_new_local_master_key() {
    let _guard = async_test_lock().lock().await;
    cove_tokio::init();
    let globals = test_globals();
    let manager = init_manager();

    reset_cloud_backup_test_state(&manager, globals);
    globals.passkey.set_discover_result(Err(PasskeyError::UserCancelled));

    enable_cloud_backup_create_new(&manager).await.unwrap();

    let cspp = cove_cspp::Cspp::new(Keychain::global().clone());
    assert!(cspp.load_master_key_from_store().unwrap().is_none());
    assert_eq!(manager.current_status(), CloudBackupStatus::Enabling);
    assert!(matches!(
        manager.model_snapshot().root_prompt,
        CloudBackupRootPrompt::PasskeyChoice(CloudBackupPasskeyChoiceIntent::Enable(_, _))
    ));
}

#[tokio::test(flavor = "current_thread")]
async fn cancelled_enable_no_discovery_rolls_back_new_local_master_key() {
    let _guard = async_test_lock().lock().await;
    cove_tokio::init();
    let globals = test_globals();
    let manager = init_manager();

    reset_cloud_backup_test_state(&manager, globals);
    globals.passkey.set_create_result(Err(PasskeyError::UserCancelled));

    enable_cloud_backup_no_discovery(&manager).await.unwrap();

    let cspp = cove_cspp::Cspp::new(Keychain::global().clone());
    assert!(cspp.load_master_key_from_store().unwrap().is_none());
    assert_eq!(manager.current_status(), CloudBackupStatus::Enabling);
    assert!(matches!(
        manager.model_snapshot().root_prompt,
        CloudBackupRootPrompt::PasskeyChoice(CloudBackupPasskeyChoiceIntent::Enable(_, _))
    ));
}

#[tokio::test(flavor = "current_thread")]
async fn cancelled_passkey_restore_does_not_fall_back_to_local_master_key() {
    let _guard = async_test_lock().lock().await;
    cove_tokio::init();
    let globals = test_globals();
    let manager = init_manager();

    reset_cloud_backup_test_state(&manager, globals);

    let local_master_key = cove_cspp::master_key::MasterKey::generate();
    let local_namespace_id = local_master_key.namespace_id();
    cove_cspp::Cspp::new(Keychain::global().clone()).save_master_key(&local_master_key).unwrap();
    globals.cloud.set_wallet_files(local_namespace_id.clone(), vec!["wallet-test.json".into()]);

    let remote_master_key = cove_cspp::master_key::MasterKey::generate();
    let remote_namespace_id = remote_master_key.namespace_id();
    let remote_prf_key = [7u8; 32];
    let remote_prf_salt = [9u8; 32];
    let encrypted_master = cove_cspp::master_key_crypto::encrypt_master_key(
        &remote_master_key,
        &remote_prf_key,
        &remote_prf_salt,
    )
    .unwrap();
    globals.cloud.set_master_key_backup(
        remote_namespace_id.clone(),
        serde_json::to_vec(&encrypted_master).unwrap(),
    );
    globals.cloud.set_wallet_files(remote_namespace_id, vec!["wallet-remote.json".into()]);
    globals.passkey.set_discover_result(Err(PasskeyError::UserCancelled));

    let operation = new_restore_operation_for_test(&manager).await;
    let error = operation.restore_from_cloud_backup(&manager).await.unwrap_err();

    assert!(matches!(error, CloudBackupError::PasskeyDiscoveryCancelled));
    assert_eq!(CloudBackupKeychain::global().namespace_id(), None);
}

#[tokio::test(flavor = "current_thread")]
async fn restore_counts_unsupported_wallet_versions_as_failures() {
    let _guard = async_test_lock().lock().await;
    cove_tokio::init();
    let globals = test_globals();
    let manager = init_manager();

    reset_cloud_backup_test_state(&manager, globals);

    let master_key = cove_cspp::master_key::MasterKey::generate();
    let namespace = master_key.namespace_id();
    let encrypted_master =
        cove_cspp::master_key_crypto::encrypt_master_key(&master_key, &[7; 32], &[9; 32]).unwrap();
    globals
        .cloud
        .set_master_key_backup(namespace.clone(), serde_json::to_vec(&encrypted_master).unwrap());
    cove_cspp::Cspp::new(Keychain::global().clone()).save_master_key(&master_key).unwrap();

    let supported_wallet = xpub_only_wallet_metadata();
    let unsupported_wallet = xpub_only_wallet_metadata();
    Keychain::global()
        .save_wallet_xpub(&supported_wallet.id, sample_xpub(&supported_wallet).parse().unwrap())
        .unwrap();
    Keychain::global()
        .save_wallet_xpub(&unsupported_wallet.id, sample_xpub(&unsupported_wallet).parse().unwrap())
        .unwrap();

    let supported_record_id =
        cove_cspp::backup_data::wallet_record_id(supported_wallet.id.as_ref());
    let unsupported_record_id =
        cove_cspp::backup_data::wallet_record_id(unsupported_wallet.id.as_ref());
    globals.cloud.set_wallet_backup(
        namespace.clone(),
        supported_record_id.clone(),
        encrypted_wallet_backup_bytes(&supported_wallet, &master_key, "supported-revision", 1)
            .await,
    );
    globals.cloud.set_wallet_backup(
        namespace.clone(),
        unsupported_record_id.clone(),
        encrypted_wallet_backup_bytes(&unsupported_wallet, &master_key, "unsupported-revision", 2)
            .await,
    );
    globals.cloud.set_wallet_files(
        namespace,
        vec![
            wallet_filename_from_record_id(&supported_record_id),
            wallet_filename_from_record_id(&unsupported_record_id),
        ],
    );

    let operation = new_restore_operation_for_test(&manager).await;
    let report = operation.restore_from_cloud_backup(&manager).await.unwrap();
    assert_eq!(report.wallets_restored, 1);
    assert_eq!(report.wallets_failed, 1);
    assert_eq!(report.failed_wallet_errors.len(), 1);
    assert!(report.failed_wallet_errors[0].contains("unsupported wallet backup version 2"));
    assert_eq!(Database::global().cloud_backup_state.get().unwrap().wallet_count(), Some(1));
    assert!(
        Database::global()
            .wallets()
            .get(&supported_wallet.id, supported_wallet.network, supported_wallet.wallet_mode,)
            .unwrap()
            .is_some()
    );
    assert!(
            Database::global()
                .wallets()
                .get(
                    &unsupported_wallet.id,
                    unsupported_wallet.network,
                    unsupported_wallet.wallet_mode,
                )
                .unwrap()
                .is_none()
        );
}

#[tokio::test(flavor = "current_thread")]
async fn restore_with_one_passkey_restores_wallets_from_all_matching_namespaces() {
    let _guard = async_test_lock().lock().await;
    cove_tokio::init();
    let globals = test_globals();
    let manager = init_manager();

    reset_cloud_backup_test_state(&manager, globals);

    let prf_key = [7u8; 32];
    let first_master_key = cove_cspp::master_key::MasterKey::generate();
    let second_master_key = cove_cspp::master_key::MasterKey::generate();
    let first_namespace = first_master_key.namespace_id();
    let second_namespace = second_master_key.namespace_id();
    let first_encrypted =
        cove_cspp::master_key_crypto::encrypt_master_key(&first_master_key, &prf_key, &[9; 32])
            .unwrap();
    let second_encrypted =
        cove_cspp::master_key_crypto::encrypt_master_key(&second_master_key, &prf_key, &[8; 32])
            .unwrap();

    globals.cloud.set_master_key_backup(
        first_namespace.clone(),
        serde_json::to_vec(&first_encrypted).unwrap(),
    );
    globals.cloud.set_master_key_backup(
        second_namespace.clone(),
        serde_json::to_vec(&second_encrypted).unwrap(),
    );
    globals.passkey.set_discover_result(Ok(DiscoveredPasskeyResult {
        prf_output: prf_key.to_vec(),
        credential_id: vec![1, 2, 3],
    }));
    globals.passkey.set_authenticate_result(Ok(prf_key.to_vec()));

    let first_wallet = xpub_only_wallet_metadata();
    let second_wallet = xpub_only_wallet_metadata();
    Keychain::global()
        .save_wallet_xpub(&first_wallet.id, sample_xpub(&first_wallet).parse().unwrap())
        .unwrap();
    Keychain::global()
        .save_wallet_xpub(&second_wallet.id, sample_xpub(&second_wallet).parse().unwrap())
        .unwrap();

    let first_record_id = cove_cspp::backup_data::wallet_record_id(first_wallet.id.as_ref());
    let second_record_id = cove_cspp::backup_data::wallet_record_id(second_wallet.id.as_ref());
    globals.cloud.set_wallet_backup(
        first_namespace.clone(),
        first_record_id.clone(),
        encrypted_wallet_backup_bytes(&first_wallet, &first_master_key, "first-revision", 1).await,
    );
    globals.cloud.set_wallet_backup(
        second_namespace.clone(),
        second_record_id.clone(),
        encrypted_wallet_backup_bytes(&second_wallet, &second_master_key, "second-revision", 1)
            .await,
    );
    globals
        .cloud
        .set_wallet_files(first_namespace, vec![wallet_filename_from_record_id(&first_record_id)]);
    globals.cloud.set_wallet_files(
        second_namespace,
        vec![wallet_filename_from_record_id(&second_record_id)],
    );

    let operation = new_restore_operation_for_test(&manager).await;
    let report = operation.restore_from_cloud_backup(&manager).await.unwrap();
    assert_eq!(report.wallets_restored, 2);
    assert_eq!(report.wallets_failed, 0);
    assert!(report.failed_wallet_errors.is_empty(), "{:?}", report.failed_wallet_errors);
    assert_eq!(Database::global().cloud_backup_state.get().unwrap().wallet_count(), Some(2));
}

#[tokio::test(flavor = "current_thread")]
async fn restore_treats_missing_wallet_listing_as_empty_without_enabling() {
    let _guard = async_test_lock().lock().await;
    cove_tokio::init();
    let globals = test_globals();
    let manager = init_manager();

    reset_cloud_backup_test_state(&manager, globals);

    let prf_key = [7u8; 32];
    let master_key = cove_cspp::master_key::MasterKey::generate();
    let namespace = master_key.namespace_id();
    let encrypted =
        cove_cspp::master_key_crypto::encrypt_master_key(&master_key, &prf_key, &[9; 32]).unwrap();

    globals.cloud.set_master_key_backup(namespace.clone(), serde_json::to_vec(&encrypted).unwrap());
    globals.cloud.fail_list_wallet_files_for_namespace(
        namespace,
        CloudStorageError::NotFound("wallet files missing".into()),
    );
    globals.passkey.set_discover_result(Ok(DiscoveredPasskeyResult {
        prf_output: prf_key.to_vec(),
        credential_id: vec![1, 2, 3],
    }));
    globals.passkey.set_authenticate_result(Ok(prf_key.to_vec()));

    let operation = new_restore_operation_for_test(&manager).await;
    let report = operation.restore_from_cloud_backup(&manager).await.unwrap();

    assert_eq!(report.wallets_restored, 0);
    assert_eq!(report.wallets_failed, 0);
    assert_eq!(
        Database::global().cloud_backup_state.get().unwrap().status(),
        PersistedCloudBackupStatus::Disabled
    );
    assert_eq!(CloudBackupKeychain::global().namespace_id(), None);
}

#[tokio::test(flavor = "current_thread")]
async fn restore_retries_platform_authorization_discover_failures() {
    let _guard = async_test_lock().lock().await;
    cove_tokio::init();
    let globals = test_globals();
    let manager = init_manager();

    reset_cloud_backup_test_state(&manager, globals);

    let prf_key = [7u8; 32];
    let master_key = cove_cspp::master_key::MasterKey::generate();
    let namespace = master_key.namespace_id();
    let encrypted =
        cove_cspp::master_key_crypto::encrypt_master_key(&master_key, &prf_key, &[9; 32]).unwrap();

    globals.cloud.set_master_key_backup(namespace.clone(), serde_json::to_vec(&encrypted).unwrap());
    globals.passkey.push_discover_result(Err(platform_authorization_failed()));
    globals.passkey.push_discover_result(Err(platform_authorization_failed()));
    globals.passkey.push_discover_result(Err(platform_authorization_failed()));
    globals.passkey.push_discover_result(Ok(DiscoveredPasskeyResult {
        prf_output: prf_key.to_vec(),
        credential_id: vec![1, 2, 3],
    }));

    let wallet = xpub_only_wallet_metadata();
    Keychain::global().save_wallet_xpub(&wallet.id, sample_xpub(&wallet).parse().unwrap()).unwrap();
    let record_id = cove_cspp::backup_data::wallet_record_id(wallet.id.as_ref());
    globals.cloud.set_wallet_backup(
        namespace.clone(),
        record_id.clone(),
        encrypted_wallet_backup_bytes(&wallet, &master_key, "revision", 1).await,
    );
    globals.cloud.set_wallet_files(namespace, vec![wallet_filename_from_record_id(&record_id)]);

    let operation = new_restore_operation_for_test(&manager).await;
    let report = operation.restore_from_cloud_backup(&manager).await.unwrap();
    assert_eq!(report.wallets_restored, 1);
    assert_eq!(report.wallets_failed, 0);
}

#[tokio::test(flavor = "current_thread")]
async fn restore_does_not_persist_first_passkey_match_before_restore_work_succeeds() {
    let _guard = async_test_lock().lock().await;
    cove_tokio::init();
    let globals = test_globals();
    let manager = init_manager();

    reset_cloud_backup_test_state(&manager, globals);

    let prf_key = [7u8; 32];
    let first_master_key = cove_cspp::master_key::MasterKey::generate();
    let second_master_key = cove_cspp::master_key::MasterKey::generate();
    let first_namespace = first_master_key.namespace_id();
    let second_namespace = second_master_key.namespace_id();
    let first_encrypted =
        cove_cspp::master_key_crypto::encrypt_master_key(&first_master_key, &prf_key, &[9; 32])
            .unwrap();
    let second_encrypted =
        cove_cspp::master_key_crypto::encrypt_master_key(&second_master_key, &prf_key, &[8; 32])
            .unwrap();

    globals.cloud.set_master_key_backup(
        first_namespace.clone(),
        serde_json::to_vec(&first_encrypted).unwrap(),
    );
    globals.cloud.set_master_key_backup(
        second_namespace.clone(),
        serde_json::to_vec(&second_encrypted).unwrap(),
    );
    globals.cloud.set_wallet_files(first_namespace, vec!["wallet-1.json".into()]);
    globals.cloud.set_wallet_files(second_namespace, vec!["wallet-2.json".into()]);
    globals.cloud.fail_list_wallet_files("list failed");
    globals.passkey.set_discover_result(Ok(DiscoveredPasskeyResult {
        prf_output: prf_key.to_vec(),
        credential_id: vec![1, 2, 3],
    }));
    globals.passkey.set_authenticate_result(Ok(prf_key.to_vec()));

    let operation = new_restore_operation_for_test(&manager).await;
    let error = operation.restore_from_cloud_backup(&manager).await.unwrap_err();

    assert!(error.to_string().contains("list failed"), "{error}");
    assert_eq!(CloudBackupKeychain::global().namespace_id(), None);
}

#[tokio::test(flavor = "current_thread")]
async fn restore_counts_listed_missing_wallet_backups_as_failures() {
    let _guard = async_test_lock().lock().await;
    cove_tokio::init();
    let globals = test_globals();
    let manager = init_manager();

    reset_cloud_backup_test_state(&manager, globals);

    let master_key = cove_cspp::master_key::MasterKey::generate();
    let namespace = master_key.namespace_id();
    let encrypted_master =
        cove_cspp::master_key_crypto::encrypt_master_key(&master_key, &[7; 32], &[9; 32]).unwrap();
    globals
        .cloud
        .set_master_key_backup(namespace.clone(), serde_json::to_vec(&encrypted_master).unwrap());
    cove_cspp::Cspp::new(Keychain::global().clone()).save_master_key(&master_key).unwrap();

    let supported_wallet = xpub_only_wallet_metadata();
    let missing_wallet = xpub_only_wallet_metadata();
    Keychain::global()
        .save_wallet_xpub(&supported_wallet.id, sample_xpub(&supported_wallet).parse().unwrap())
        .unwrap();
    let supported_record_id =
        cove_cspp::backup_data::wallet_record_id(supported_wallet.id.as_ref());
    let missing_record_id = cove_cspp::backup_data::wallet_record_id(missing_wallet.id.as_ref());
    globals.cloud.set_wallet_backup(
        namespace.clone(),
        supported_record_id.clone(),
        encrypted_wallet_backup_bytes(&supported_wallet, &master_key, "supported-revision", 1)
            .await,
    );
    globals.cloud.set_wallet_files(
        namespace,
        vec![
            wallet_filename_from_record_id(&supported_record_id),
            wallet_filename_from_record_id(&missing_record_id),
        ],
    );

    let operation = new_restore_operation_for_test(&manager).await;
    let report = operation.restore_from_cloud_backup(&manager).await.unwrap();
    assert_eq!(report.wallets_restored, 1);
    assert_eq!(report.wallets_failed, 1);
    assert!(report.failed_wallet_errors[0].contains("was listed but missing from cloud backup"));
    assert!(report.labels_failed_wallet_names.is_empty());
    assert_eq!(Database::global().cloud_backup_state.get().unwrap().wallet_count(), Some(1));
}

#[tokio::test(flavor = "current_thread")]
async fn restore_reports_label_warning_without_failing_wallet_restore() {
    let _guard = async_test_lock().lock().await;
    cove_tokio::init();
    let globals = test_globals();
    let manager = init_manager();

    reset_cloud_backup_test_state(&manager, globals);

    let master_key = cove_cspp::master_key::MasterKey::generate();
    let namespace = master_key.namespace_id();
    let encrypted_master =
        cove_cspp::master_key_crypto::encrypt_master_key(&master_key, &[7; 32], &[9; 32]).unwrap();
    globals
        .cloud
        .set_master_key_backup(namespace.clone(), serde_json::to_vec(&encrypted_master).unwrap());
    cove_cspp::Cspp::new(Keychain::global().clone()).save_master_key(&master_key).unwrap();

    let wallet = xpub_only_wallet_metadata();
    let record_id = cove_cspp::backup_data::wallet_record_id(wallet.id.as_ref());
    let entry = wallet_entry_with_labels(&wallet, Some("{"));
    globals.cloud.set_wallet_backup(
        namespace.clone(),
        record_id.clone(),
        encrypted_wallet_backup_bytes_for_entry(&entry, &master_key, 1),
    );
    globals.cloud.set_wallet_files(namespace, vec![wallet_filename_from_record_id(&record_id)]);

    let operation = new_restore_operation_for_test(&manager).await;
    let report = operation.restore_from_cloud_backup(&manager).await.unwrap();
    assert_eq!(report.wallets_restored, 1);
    assert_eq!(report.wallets_failed, 0);
    assert_eq!(report.labels_failed_wallet_names, vec![wallet.name.clone()]);
    assert_eq!(report.labels_failed_errors.len(), 1);
    assert!(
        report.labels_failed_errors[0].contains("Failed to parse labels")
            || report.labels_failed_errors[0].contains("failed to parse")
    );
    assert!(
        Database::global()
            .wallets()
            .get(&wallet.id, wallet.network, wallet.wallet_mode)
            .unwrap()
            .is_some()
    );
}

#[tokio::test(flavor = "current_thread")]
async fn restore_cloud_wallet_returns_label_warning_without_failing_restore() {
    let _guard = async_test_lock().lock().await;
    cove_tokio::init();
    let globals = test_globals();
    let manager = init_manager();

    reset_cloud_backup_test_state(&manager, globals);

    let master_key = cove_cspp::master_key::MasterKey::generate();
    let namespace = master_key.namespace_id();
    CloudBackupKeychain::global().save_namespace_id(&namespace).unwrap();
    cove_cspp::Cspp::new(Keychain::global().clone()).save_master_key(&master_key).unwrap();
    manager
        .persist_cloud_backup_state(
            &persisted_enabled_cloud_backup_state(None),
            "enable cloud backup for restore cloud wallet test",
        )
        .unwrap();

    let wallet = xpub_only_wallet_metadata();
    let record_id = cove_cspp::backup_data::wallet_record_id(wallet.id.as_ref());
    let entry = wallet_entry_with_labels(&wallet, Some("{"));
    globals.cloud.set_wallet_backup(
        namespace,
        record_id.clone(),
        encrypted_wallet_backup_bytes_for_entry(&entry, &master_key, 1),
    );

    let outcome = manager.do_restore_cloud_wallet(&record_id).await.unwrap();

    let warning = outcome.labels_warning.expect("expected label warning");
    assert_eq!(warning.wallet_name, wallet.name);
    assert!(
        warning.error.contains("Failed to parse labels")
            || warning.error.contains("failed to parse")
    );
    assert!(
        Database::global()
            .wallets()
            .get(&wallet.id, wallet.network, wallet.wallet_mode)
            .unwrap()
            .is_some()
    );
}

#[tokio::test(flavor = "current_thread")]
async fn restore_fails_when_all_wallet_backups_are_unsupported() {
    let _guard = async_test_lock().lock().await;
    cove_tokio::init();
    let globals = test_globals();
    let manager = init_manager();

    reset_cloud_backup_test_state(&manager, globals);

    let master_key = cove_cspp::master_key::MasterKey::generate();
    let namespace = master_key.namespace_id();
    let encrypted_master =
        cove_cspp::master_key_crypto::encrypt_master_key(&master_key, &[7; 32], &[9; 32]).unwrap();
    globals
        .cloud
        .set_master_key_backup(namespace.clone(), serde_json::to_vec(&encrypted_master).unwrap());
    cove_cspp::Cspp::new(Keychain::global().clone()).save_master_key(&master_key).unwrap();

    let wallet = xpub_only_wallet_metadata();
    Keychain::global().save_wallet_xpub(&wallet.id, sample_xpub(&wallet).parse().unwrap()).unwrap();

    let record_id = cove_cspp::backup_data::wallet_record_id(wallet.id.as_ref());
    globals.cloud.set_wallet_backup(
        namespace.clone(),
        record_id.clone(),
        encrypted_wallet_backup_bytes(&wallet, &master_key, "unsupported-revision", 2).await,
    );
    globals.cloud.set_wallet_files(namespace, vec![wallet_filename_from_record_id(&record_id)]);

    let operation = new_restore_operation_for_test(&manager).await;
    let error = operation.restore_from_cloud_backup(&manager).await.unwrap_err();

    assert!(matches!(
        error,
        CloudBackupError::Internal(message) if message == "all wallets failed to restore"
    ));

    assert_eq!(
        Database::global().cloud_backup_state.get().unwrap().status(),
        PersistedCloudBackupStatus::Disabled
    );
}

#[tokio::test(flavor = "current_thread")]
async fn restore_fails_when_all_listed_wallet_backups_are_missing() {
    let _guard = async_test_lock().lock().await;
    cove_tokio::init();
    let globals = test_globals();
    let manager = init_manager();

    reset_cloud_backup_test_state(&manager, globals);

    let master_key = cove_cspp::master_key::MasterKey::generate();
    let namespace = master_key.namespace_id();
    let encrypted_master =
        cove_cspp::master_key_crypto::encrypt_master_key(&master_key, &[7; 32], &[9; 32]).unwrap();
    globals
        .cloud
        .set_master_key_backup(namespace.clone(), serde_json::to_vec(&encrypted_master).unwrap());
    cove_cspp::Cspp::new(Keychain::global().clone()).save_master_key(&master_key).unwrap();

    let missing_wallet = xpub_only_wallet_metadata();
    let missing_record_id = cove_cspp::backup_data::wallet_record_id(missing_wallet.id.as_ref());
    globals
        .cloud
        .set_wallet_files(namespace, vec![wallet_filename_from_record_id(&missing_record_id)]);

    let operation = new_restore_operation_for_test(&manager).await;
    let error = operation.restore_from_cloud_backup(&manager).await.unwrap_err();

    assert!(matches!(
        error,
        CloudBackupError::Internal(message) if message == "all wallets failed to restore"
    ));

    assert_eq!(
        Database::global().cloud_backup_state.get().unwrap().status(),
        PersistedCloudBackupStatus::Disabled
    );
}
