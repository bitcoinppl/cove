use super::*;

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
async fn corrupt_blob_sync_state_projects_failed_sync_health() {
    let _guard = async_test_lock().lock().await;
    cove_tokio::init();
    let globals = test_globals();
    let manager = init_manager();
    configure_enabled_cloud_backup(&manager, globals, 0);

    Database::global()
        .cloud_blob_sync_states
        .set(&PersistedCloudBlobSyncState::corrupted(
            "failed to decode persisted cloud backup blob sync state".into(),
        ))
        .unwrap();

    assert!(matches!(
        manager.compute_sync_health().await,
        CloudSyncHealth::Failed(message)
            if message.contains("failed to decode persisted cloud backup blob sync state")
    ));
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
    let existing_namespace = "fedcba9876543210fedcba9876543210";
    globals.cloud.set_master_key_backup(existing_namespace.into(), vec![1, 2, 3]);
    globals.cloud.set_wallet_files(existing_namespace.into(), vec!["wallet-1.json".into()]);

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
async fn refresh_cloud_backup_detail_returns_access_error_when_wallet_listing_is_missing() {
    let _guard = async_test_lock().lock().await;
    cove_tokio::init();
    let globals = test_globals();
    let manager = init_manager();
    configure_enabled_cloud_backup(&manager, globals, 1);

    let namespace = CloudBackupKeychain::global().namespace_id().unwrap();
    globals.cloud.fail_list_wallet_files_for_namespace(
        namespace,
        CloudStorageError::NotFound("wallet files missing".into()),
    );

    let Some(CloudBackupDetailResult::AccessError(error)) =
        manager.refresh_cloud_backup_detail().await
    else {
        panic!("expected cloud backup detail access error");
    };

    assert!(error.to_string().contains("wallet files missing"), "{error}");
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
