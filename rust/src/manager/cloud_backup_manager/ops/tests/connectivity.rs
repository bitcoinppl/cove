use super::*;

async fn configure_trusted_local_snapshot_with_two_cloud_only_wallets(
    manager: &RustCloudBackupManager,
    globals: &TestGlobals,
) {
    configure_enabled_cloud_backup(manager, globals, 2);

    let namespace = CloudBackupKeychain::global().namespace_id().unwrap();
    let master_key = cove_cspp::Cspp::new(Keychain::global().clone())
        .load_master_key_from_store()
        .unwrap()
        .unwrap();
    let first_wallet = xpub_only_wallet_metadata();
    let mut second_wallet = xpub_only_wallet_metadata();
    second_wallet.network = crate::network::Network::Testnet;
    let first_record_id = wallet_record_id(first_wallet.id.as_ref());
    let second_record_id = wallet_record_id(second_wallet.id.as_ref());

    globals.cloud.set_wallet_backup(
        namespace.clone(),
        first_record_id.clone(),
        encrypted_wallet_backup_bytes_for_entry(
            &wallet_entry_with_labels(&first_wallet, None),
            &master_key,
            1,
        ),
    );
    globals.cloud.set_wallet_backup(
        namespace.clone(),
        second_record_id.clone(),
        encrypted_wallet_backup_bytes_for_entry(
            &wallet_entry_with_labels(&second_wallet, None),
            &master_key,
            1,
        ),
    );

    let wallet_files = vec![
        wallet_filename_from_record_id(&first_record_id),
        wallet_filename_from_record_id(&second_record_id),
    ];
    globals.cloud.set_wallet_files(namespace.clone(), wallet_files.clone());
    globals.cloud.set_wallet_files_snapshot(namespace, wallet_files, false);
}

#[tokio::test(flavor = "current_thread")]
async fn trusted_local_snapshot_is_confirmed_by_provider_in_background() {
    let _guard = async_test_lock().lock().await;
    let globals = test_globals();
    let manager = init_manager();
    configure_trusted_local_snapshot_with_two_cloud_only_wallets(&manager, globals).await;
    let initial_list_attempts = globals.cloud.list_wallet_files_attempt_count();

    call!(manager.supervisor.start_refresh_detail()).await.unwrap();

    wait_for_test_condition(Duration::from_secs(2), "expected provider-confirmed detail", || {
        matches!(
            manager.state().lifecycle,
            CloudBackupLifecycle::Configured(ref configured)
                if matches!(
                    configured.detail,
                    CloudBackupDetailState::Complete { ref state }
                        if state.inventory_authority
                            == CloudBackupInventoryAuthority::ProviderConfirmed
                )
                && matches!(
                    configured.restore_all,
                    CloudBackupRestoreAllState::StartAvailable { wallet_count: 2 }
                )
        )
    })
    .await;

    assert!(globals.cloud.list_wallet_files_attempt_count() > initial_list_attempts);
}

#[tokio::test(flavor = "current_thread")]
async fn provider_confirmation_failure_keeps_trusted_local_detail_complete() {
    let _guard = async_test_lock().lock().await;
    let globals = test_globals();
    let manager = init_manager();
    configure_trusted_local_snapshot_with_two_cloud_only_wallets(&manager, globals).await;
    globals.cloud.fail_list_wallet_files("metadata timed out");
    let initial_list_attempts = globals.cloud.list_wallet_files_attempt_count();

    call!(manager.supervisor.start_refresh_detail()).await.unwrap();

    wait_for_test_condition(
        Duration::from_secs(2),
        "expected retained trusted local detail",
        || {
            globals.cloud.list_wallet_files_attempt_count() > initial_list_attempts
                && matches!(
                    manager.state().lifecycle,
                    CloudBackupLifecycle::Configured(ref configured)
                        if matches!(
                            configured.detail,
                            CloudBackupDetailState::Complete { ref state }
                                if state.inventory_authority
                                    == CloudBackupInventoryAuthority::LocalSnapshotMatchesKnownCount
                        )
                        && matches!(
                            configured.restore_all,
                            CloudBackupRestoreAllState::StartDisabled { wallet_count: 2 }
                        )
                )
        },
    )
    .await;

    assert_test_condition_stays_true(
        Duration::from_millis(150),
        "provider confirmation failure should keep trusted local detail",
        || {
            matches!(
                manager.state().lifecycle,
                CloudBackupLifecycle::Configured(ref configured)
                    if matches!(configured.detail, CloudBackupDetailState::Complete { .. })
            )
        },
    )
    .await;
}

#[tokio::test(flavor = "current_thread")]
async fn connectivity_reconnect_preserves_failed_wallet_upload_health() {
    let _guard = async_test_lock().lock().await;
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
        CloudSyncHealth::Failed(message) if message == GENERIC_CLOUD_BACKUP_ERROR_MESSAGE
    ));
}

#[tokio::test(flavor = "current_thread")]
async fn connectivity_reconnect_reports_no_files_when_nothing_was_uploaded() {
    let _guard = async_test_lock().lock().await;
    let globals = test_globals();
    let manager = init_manager();
    configure_enabled_cloud_backup(&manager, globals, 0);

    manager.handle_connectivity_change(ConnectivityStatus::Connected);

    assert_eq!(manager.compute_sync_health().await, CloudSyncHealth::NoFiles);
}

#[tokio::test(flavor = "current_thread")]
async fn reconnect_retries_verification_after_offline_failure() {
    let _guard = async_test_lock().lock().await;
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
async fn manual_detail_refresh_recovers_after_automatic_retry_fails() {
    let _guard = async_test_lock().lock().await;
    let globals = test_globals();
    let manager = init_manager();
    configure_enabled_cloud_backup(&manager, globals, 0);
    CONNECTIVITY_MANAGER.set_connection_state(true);

    let namespace = CloudBackupKeychain::global().namespace_id().unwrap();
    globals.cloud.set_wallet_files_snapshot(namespace, Vec::new(), false);
    globals.cloud.fail_list_wallet_files("metadata timed out");

    call!(manager.supervisor.start_refresh_detail()).await.unwrap();
    wait_for_test_condition(Duration::from_secs(1), "expected incomplete detail inventory", || {
        matches!(
            manager.state().lifecycle,
            CloudBackupLifecycle::Configured(ref configured)
                if matches!(&configured.detail, CloudBackupDetailState::Failed { .. })
        )
    })
    .await;

    globals.cloud.clear_list_wallet_files_error();
    let snapshot_attempts = globals.cloud.list_wallet_files_snapshot_attempt_count();

    call!(manager.supervisor.start_refresh_detail()).await.unwrap();
    wait_for_test_condition(Duration::from_secs(6), "expected manual detail refresh", || {
        globals.cloud.list_wallet_files_snapshot_attempt_count() > snapshot_attempts
    })
    .await;
}

#[tokio::test(flavor = "current_thread")]
async fn provider_signal_reopens_only_incomplete_ready_detail_inventory() {
    let _guard = async_test_lock().lock().await;
    let globals = test_globals();
    let manager = init_manager();
    configure_enabled_cloud_backup(&manager, globals, 1);

    let namespace = CloudBackupKeychain::global().namespace_id().unwrap();
    globals.cloud.set_wallet_files_snapshot(namespace, Vec::new(), true);

    call!(manager.supervisor.start_refresh_detail()).await.unwrap();
    wait_for_test_condition(Duration::from_secs(1), "expected complete detail inventory", || {
        matches!(
            manager.state().lifecycle,
            CloudBackupLifecycle::Configured(ref configured)
                if matches!(&configured.detail, CloudBackupDetailState::Complete { .. })
        )
    })
    .await;
    let complete_detail = manager.model_snapshot().detail.expect("expected loaded detail");
    let active_snapshot_attempts = globals.cloud.list_wallet_files_snapshot_attempt_count();

    manager.cloud_storage_did_change();
    tokio::time::sleep(Duration::from_millis(100)).await;

    assert!(matches!(
        manager.state().lifecycle,
        CloudBackupLifecycle::Configured(ref configured)
            if matches!(&configured.detail, CloudBackupDetailState::Complete { .. })
    ));
    assert_eq!(globals.cloud.list_wallet_files_snapshot_attempt_count(), active_snapshot_attempts);

    manager.apply_detail_outcome(CloudBackupDetailOutcome::RefreshedWithAuthority {
        detail: complete_detail,
        authority: CloudBackupInventoryAuthority::LocalSnapshotMatchesKnownCount,
    });
    let snapshot_attempts = globals.cloud.list_wallet_files_snapshot_attempt_count();

    manager.cloud_storage_did_change();
    wait_for_test_condition(Duration::from_secs(6), "expected provider signal refresh", || {
        globals.cloud.list_wallet_files_snapshot_attempt_count() > snapshot_attempts
    })
    .await;

    assert!(matches!(
        manager.state().lifecycle,
        CloudBackupLifecycle::Configured(ref configured)
            if matches!(&configured.detail, CloudBackupDetailState::Complete { .. })
    ));
    assert!(globals.cloud.list_wallet_files_snapshot_attempt_count() > snapshot_attempts);
}

#[tokio::test(flavor = "current_thread")]
async fn connected_connectivity_failure_retries_verification_once() {
    let _guard = async_test_lock().lock().await;
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
    manager.apply_enable_state(CloudBackupEnableState::AwaitingSavedPasskeyConfirmation(
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
    let globals = test_globals();
    let manager = global_manager();
    clear_wallet_upload_runtime_for_test_async(&manager).await;
    configure_enabled_cloud_backup(&manager, globals, 0);

    let metadata = xpub_only_wallet_metadata();
    persist_xpub_wallets(vec![metadata.clone()]);
    persist_failed_blob_state_with_issue(
        metadata.id,
        false,
        Some(CloudStorageIssue::AuthorizationRequired),
    );
    let initial_attempt_count = globals.cloud.wallet_backup_upload_attempt_count();

    assert_eq!(
        manager.compute_sync_health().await,
        CloudSyncHealth::AuthorizationRequired(
            "Cove couldn't access your cloud backup. Reconnect your cloud account, then try again."
                .into(),
        ),
    );

    resume_wallet_uploads_from_persisted_state_for_test_async(&manager).await;

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
    let globals = test_globals();
    let manager = global_manager();
    clear_wallet_upload_runtime_for_test_async(&manager).await;
    configure_enabled_cloud_backup(&manager, globals, 0);

    let metadata = xpub_only_wallet_metadata();
    persist_xpub_wallets(vec![metadata.clone()]);
    persist_failed_blob_state_with_issue(
        metadata.id,
        false,
        Some(CloudStorageIssue::AuthorizationRequired),
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
async fn startup_resume_retries_interrupted_uploading_wallets() {
    let _guard = async_test_lock().lock().await;
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
