use super::*;
use super::enable::{
    EnableRecoveryFinalization, EnableUploadFinalization, PendingEnableUploadSelection,
};
use super::verification::DeepVerificationContinuation;
use crate::database::cloud_backup::PersistedDisablingCloudBackup;
use crate::manager::cloud_backup_manager::ops::test_support::{
    async_test_lock, reset_cloud_backup_test_state, test_globals,
};
use crate::manager::cloud_backup_manager::wallets::{StagedPrfKey, UnpersistedPrfKey};
use crate::manager::cloud_backup_manager::{CloudBackupStore, PendingEnableSessionMaterial};

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
        namespace_id: "namespace".into(),
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
        namespace_id: "namespace".into(),
        credential_id: vec![1, 2, 3],
        prf_salt: [9; 32],
    }
}

fn test_enable_upload_finalization() -> EnableUploadFinalization {
    let master_key = cove_cspp::master_key::MasterKey::generate();
    let namespace_id = master_key.namespace_id();
    let passkey = test_enable_passkey(vec![1, 2, 3]);
    let encrypted_master = master_key_crypto::encrypt_master_key_with_remote_metadata(
        &master_key,
        &passkey.prf_key,
        &passkey.prf_salt,
        passkey.provider_hint.clone(),
        cove_cspp::backup_data::remote_payload::RemotePayloadMetadata::master_key(&namespace_id, 0),
    )
    .unwrap();

    EnableUploadFinalization {
        master_key: zeroize::Zeroizing::new(master_key),
        passkey: zeroize::Zeroizing::new(passkey),
        context: CloudBackupEnableContext::settings_manual(),
        namespace_id,
        encrypted_master,
        pending_uploads: Vec::new(),
    }
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
        .complete_restore_cloud_wallet(stale, "wallet-record".into(), Ok(Default::default()))
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

    supervisor.complete_repair_passkey_refresh_detail(stale, None).await.unwrap();

    assert_eq!(supervisor.active_operation, Some(current));
    assert_eq!(manager.projected_exclusive_operation(), Some(current));

    supervisor.complete_repair_passkey_refresh_detail(current, None).await.unwrap();

    assert_eq!(supervisor.active_operation, None);
    assert_eq!(manager.projected_exclusive_operation(), None);
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

    supervisor.complete_sync_request_refresh_detail(6, None).await.unwrap();

    assert_eq!(supervisor.active_sync_request, Some(7));

    supervisor.complete_sync_request_refresh_detail(7, None).await.unwrap();

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
        .complete_recreate_manifest_verification(
            stale,
            CloudBackupDeepVerificationStep::Complete(DeepVerificationResult::NotEnabled),
            VerificationAttempt::Initial,
        )
        .await
        .unwrap();

    assert_eq!(supervisor.active_operation, Some(current));
    assert_eq!(manager.projected_exclusive_operation(), Some(current));

    supervisor
        .complete_recreate_manifest_verification(
            current,
            CloudBackupDeepVerificationStep::Complete(DeepVerificationResult::NotEnabled),
            VerificationAttempt::Initial,
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
        .complete_reinitialize_verification(
            stale,
            CloudBackupDeepVerificationStep::Complete(DeepVerificationResult::NotEnabled),
            VerificationAttempt::Initial,
        )
        .await
        .unwrap();

    assert_eq!(supervisor.active_operation, Some(current));
    assert_eq!(manager.projected_exclusive_operation(), Some(current));

    supervisor
        .complete_reinitialize_verification(
            current,
            CloudBackupDeepVerificationStep::Complete(DeepVerificationResult::NotEnabled),
            VerificationAttempt::Initial,
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
                namespace_id: "stale-namespace".into(),
                active_critical_key: zeroize::Zeroizing::new([0; 32]),
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
                namespace_id: "current-namespace".into(),
                active_critical_key: zeroize::Zeroizing::new([0; 32]),
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
            test_enable_upload_finalization(),
            Err(CloudBackupError::Internal("stale completion".into())),
        )
        .await
        .unwrap();

    assert_eq!(supervisor.active_operation, Some(current));
    assert_eq!(manager.projected_exclusive_operation(), Some(current));

    supervisor
        .complete_enable_upload_finalization(
            current,
            test_enable_upload_finalization(),
            Err(CloudBackupError::Internal("current completion".into())),
        )
        .await
        .unwrap();

    assert_eq!(supervisor.active_operation, None);
    assert_eq!(manager.projected_exclusive_operation(), None);
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

    call!(writes.block(blocker)).await.unwrap();

    let blocked_write = call!(writes.upload_wallet_backup(
        CloudStorage::global_explicit_client(),
        "namespace".into(),
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
        "namespace".into(),
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
