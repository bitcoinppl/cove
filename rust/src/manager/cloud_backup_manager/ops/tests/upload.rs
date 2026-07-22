use super::*;

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
async fn backup_wallets_defers_remaining_writes_when_disable_starts_after_upload() {
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
    assert!(Database::global().cloud_backup_state.get().unwrap().is_disabling());
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
async fn reupload_all_wallets_does_not_create_master_key_for_existing_namespace() {
    let _guard = async_test_lock().lock().await;
    cove_tokio::init();
    let globals = test_globals();
    globals.reset();

    CloudBackupKeychain::global().save_namespace_id("0123456789abcdef0123456789abcdef").unwrap();

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
async fn backup_wallets_does_not_create_master_key_or_upload_when_missing() {
    let _guard = async_test_lock().lock().await;
    cove_tokio::init();
    let globals = test_globals();
    globals.reset();

    let namespace = "0123456789abcdef0123456789abcdef";
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

    let namespace = "0123456789abcdef0123456789abcdef";
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
async fn upload_wallet_if_dirty_completes_inside_disable_transition_when_already_started() {
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

    manager.do_upload_wallet_if_dirty(&metadata.id).await.unwrap();

    assert_eq!(globals.cloud.uploaded_wallet_backup_count(), 1);
    assert!(Database::global().cloud_backup_state.get().unwrap().is_disabling());
    assert!(matches!(
        Database::global().cloud_blob_sync_states.get(&record_id).unwrap(),
        Some(PersistedCloudBlobSyncState {
            state: PersistedCloudBlobState::UploadedPendingConfirmation(_)
                | PersistedCloudBlobState::Confirmed(_),
            ..
        })
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
        crate::manager::cloud_backup_manager::current_timestamp(),
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
