use super::*;
use crate::manager::cloud_backup_manager::owner::operation::test_support::operation_id;

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
    assert!(error.to_string().contains("has no private key"), "{error}");
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
    let globals = test_globals();
    let manager = init_manager();
    configure_enabled_cloud_backup(&manager, globals, 3);

    let metadata = xpub_only_wallet_metadata();
    let xpub = sample_xpub(&metadata);
    Keychain::global().save_wallet_xpub(&metadata.id, xpub.parse().unwrap()).unwrap();

    let namespace_id = CloudBackupKeychain::global().namespace_id().unwrap().unwrap();
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
    let globals = test_globals();
    globals.reset();

    CloudBackupKeychain::global().save_namespace_id("0123456789abcdef0123456789abcdef").unwrap();

    let manager = init_manager();
    let claim = CloudBackupExclusiveOperationClaim::new(
        CloudBackupExclusiveOperation::RecreateManifest,
        operation_id(1),
    );
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
    let globals = test_globals();
    let manager = init_manager();
    configure_enabled_cloud_backup(&manager, globals, 0);

    let metadata = xpub_only_wallet_metadata();
    persist_xpub_wallets(vec![metadata]);

    let namespace = CloudBackupKeychain::global().namespace_id().unwrap().unwrap();
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
async fn upload_wallet_if_dirty_stays_retryable_when_a_keychain_read_fails() {
    let _guard = async_test_lock().lock().await;
    let globals = test_globals();
    let manager = init_manager();
    configure_enabled_cloud_backup(&manager, globals, 0);

    let namespace = CloudBackupKeychain::global().namespace_id().unwrap().unwrap();
    let metadata = xpub_only_wallet_metadata();
    let xpub = sample_xpub(&metadata);
    Keychain::global().save_wallet_xpub(&metadata.id, xpub.parse().unwrap()).unwrap();
    persist_xpub_wallets(vec![metadata.clone()]);

    let record_id = cove_cspp::backup_data::wallet_record_id(metadata.id.as_ref());
    let xpub_key = format!("{}{}", metadata.id, crate::test_support::WALLET_XPUB_KEY_SUFFIX);

    let unreadable_keys = [
        crate::manager::cloud_backup_manager::keychain::CSPP_NAMESPACE_ID_KEY,
        xpub_key.as_str(),
        "cspp::v1::master_key",
    ];

    for key in unreadable_keys {
        Database::global()
            .cloud_blob_sync_states
            .set(&PersistedCloudBlobSyncState::wallet(
                namespace.clone(),
                metadata.id.clone(),
                record_id.clone(),
                PersistedCloudBlobState::Dirty(CloudBlobDirtyState { changed_at: 1 }),
            ))
            .unwrap();

        globals.keychain.fail_reads_for(key);
        let result = manager.do_upload_wallet_if_dirty(&metadata.id).await;
        globals.keychain.clear_read_failures();

        assert!(result.is_err(), "unreadable {key} still uploaded");
        assert_eq!(globals.cloud.uploaded_wallet_backup_count(), 0, "{key}");
        assert!(
            matches!(
                Database::global().cloud_blob_sync_states.get(&record_id).unwrap(),
                Some(PersistedCloudBlobSyncState {
                    state: PersistedCloudBlobState::Failed(CloudBlobFailedState {
                        retryable: true,
                        ..
                    }),
                    ..
                })
            ),
            "unreadable {key} was recorded as a terminal upload failure"
        );
    }
}

#[tokio::test(flavor = "current_thread")]
async fn wallet_change_is_recorded_while_the_namespace_is_unreadable() {
    let _guard = async_test_lock().lock().await;
    let globals = test_globals();
    let manager = init_manager();
    configure_enabled_cloud_backup(&manager, globals, 0);

    let namespace = CloudBackupKeychain::global().namespace_id().unwrap().unwrap();
    let metadata = xpub_only_wallet_metadata();
    persist_xpub_wallets(vec![metadata.clone()]);

    let record_id = cove_cspp::backup_data::wallet_record_id(metadata.id.as_ref());
    let failed = PersistedCloudBlobState::Failed(CloudBlobFailedState {
        revision_hash: None,
        retryable: false,
        issue: None,
        error: "earlier failure".into(),
        failed_at: 1,
    });
    Database::global()
        .cloud_blob_sync_states
        .set(&PersistedCloudBlobSyncState::wallet(
            namespace.clone(),
            metadata.id.clone(),
            record_id.clone(),
            failed,
        ))
        .unwrap();

    globals
        .keychain
        .fail_reads_for(crate::manager::cloud_backup_manager::keychain::CSPP_NAMESPACE_ID_KEY);
    manager.mark_wallet_blob_dirty(metadata.id);
    globals.keychain.clear_read_failures();

    let sync_state = Database::global().cloud_blob_sync_states.get(&record_id).unwrap().unwrap();
    assert_eq!(sync_state.namespace_id, namespace);
    assert!(matches!(sync_state.state, PersistedCloudBlobState::Dirty(_)), "{sync_state:?}");
}

#[tokio::test(flavor = "current_thread")]
async fn deferred_live_wallet_upload_retries_without_restart() {
    let _guard = async_test_lock().lock().await;
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

    // the wait above is long enough for a failed first retry to upload on the next backoff, so
    // the attempt count is what proves the first retry itself uploaded
    assert_eq!(
        globals.cloud.wallet_backup_upload_attempt_count(),
        2,
        "the first automatic retry should upload the deferred wallet"
    );
    assert!(globals.cloud.uploaded_wallet_backup_count() >= 1);

    clear_wallet_upload_runtime_for_test_async(&manager).await;
}

#[tokio::test(flavor = "current_thread")]
async fn upload_wallet_if_dirty_removes_deleted_wallet_sync_state() {
    let _guard = async_test_lock().lock().await;
    let globals = test_globals();
    let manager = init_manager();
    configure_enabled_cloud_backup(&manager, globals, 0);

    let metadata = WalletMetadata::preview_new();
    let namespace = CloudBackupKeychain::global().namespace_id().unwrap().unwrap();
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
    assert_eq!(CloudBackupKeychain::global().namespace_id().unwrap(), Some(namespace));
}

#[tokio::test(flavor = "current_thread")]
async fn upload_wallet_if_dirty_preserves_newer_dirty_state() {
    let _guard = async_test_lock().lock().await;
    let globals = test_globals();
    let manager = init_manager();
    configure_enabled_cloud_backup(&manager, globals, 0);

    let mut metadata = WalletMetadata::preview_new();
    metadata.wallet_type = WalletType::WatchOnly;
    Database::global()
        .wallets()
        .save_all_wallets(metadata.network, metadata.wallet_mode, vec![metadata.clone()])
        .unwrap();

    let namespace_id = CloudBackupKeychain::global().namespace_id().unwrap().unwrap();
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
    let globals = test_globals();
    let manager = init_manager();
    configure_enabled_cloud_backup(&manager, globals, 0);

    let metadata = xpub_only_wallet_metadata();
    persist_xpub_wallets(vec![metadata.clone()]);
    persist_dirty_blob_state(metadata.id.clone());

    let namespace_id = CloudBackupKeychain::global().namespace_id().unwrap().unwrap();
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
    let globals = test_globals();
    let manager = init_manager();
    configure_enabled_cloud_backup(&manager, globals, 0);

    let metadata = xpub_only_wallet_metadata();
    persist_xpub_wallets(vec![metadata.clone()]);
    let record_id = cove_cspp::backup_data::wallet_record_id(metadata.id.as_ref());
    persist_uploading_blob_state(
        metadata.id.clone(),
        cove_util::time::unix_timestamp_secs_or_zero(),
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

// the load-time origin repair changes a field of the backup revision, so a wallet whose backup is
// already confirmed must be uploaded again with the repaired origin. A hot wallet whose key is
// missing locally keeps its confirmed backup, since opening it downgrades it to watch-only and
// that upload would drop the key from the cloud
#[tokio::test(flavor = "current_thread")]
async fn repairing_a_wallet_origin_on_load_marks_its_confirmed_backup_dirty() {
    let _guard = async_test_lock().lock().await;
    let globals = test_globals();
    let manager = init_manager();
    configure_enabled_cloud_backup(&manager, globals, 1);
    crate::app::reconcile::test_support::init_noop_updater();

    // offline keeps the dirty mark in place instead of starting an upload
    CONNECTIVITY_MANAGER.set_connection_state(false);

    let database = Database::global();
    database.global_config.set_selected_network(crate::network::Network::Bitcoin).unwrap();
    database.global_config.set_main_mode().unwrap();

    let with_key = load_origin_less_wallet_with_confirmed_backup(true);
    let without_key = load_origin_less_wallet_with_confirmed_backup(false);
    CONNECTIVITY_MANAGER.set_connection_state(true);

    let (loaded_origin, expected_origin, sync_state) = with_key;
    assert_eq!(loaded_origin, expected_origin);
    assert!(
        matches!(
            sync_state,
            Some(PersistedCloudBlobSyncState { state: PersistedCloudBlobState::Dirty(_), .. })
        ),
        "{sync_state:?}"
    );

    let (loaded_origin, expected_origin, sync_state) = without_key;
    assert_eq!(loaded_origin, expected_origin);
    assert!(
        matches!(
            sync_state,
            Some(PersistedCloudBlobSyncState { state: PersistedCloudBlobState::Confirmed(_), .. })
        ),
        "{sync_state:?}"
    );
}

/// Loads a hot wallet whose stored row lacks an origin and whose backup is confirmed, returning
/// the loaded origin, the origin its build derived, and the backup's sync state after the load
fn load_origin_less_wallet_with_confirmed_backup(
    keep_secret: bool,
) -> (Option<String>, Option<String>, Option<PersistedCloudBlobSyncState>) {
    use crate::database::cloud_backup::CloudBlobConfirmedState;
    use crate::wallet::{Wallet, metadata::WalletMetadata};

    let database = Database::global();
    let mnemonic = Mnemonic::parse_normalized(
        "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about",
    )
    .unwrap();
    let wallet = Wallet::try_new_persisted_from_mnemonic_segwit(
        WalletMetadata::preview_new(),
        mnemonic.clone(),
        None,
    )
    .unwrap();
    let id = wallet.id.clone();
    let expected_origin = wallet.metadata.origin.clone();
    assert!(expected_origin.is_some(), "a mnemonic wallet build derives an origin");

    if keep_secret {
        Keychain::global()
            .save_wallet_secret(&id, cove_device::keychain::WalletSecret::Mnemonic(mnemonic))
            .unwrap();
    } else {
        Keychain::global().delete_wallet_secret(&id);
    }

    let mut origin_less = wallet.metadata.clone();
    origin_less.origin = None;
    database.wallets.save_new_wallet_metadata(origin_less).unwrap();
    drop(wallet);

    let namespace_id = CloudBackupKeychain::global().namespace_id().unwrap().unwrap();
    let record_id = wallet_record_id(id.as_ref());
    let confirmed = PersistedCloudBlobSyncState::wallet(
        namespace_id,
        id.clone(),
        record_id.clone(),
        PersistedCloudBlobState::Confirmed(CloudBlobConfirmedState {
            revision_hash: "uploaded-without-origin".into(),
            confirmed_at: 1,
        }),
    );
    database.cloud_blob_sync_states.set(&confirmed).unwrap();

    let loaded = Wallet::try_load_persisted(id.clone());
    let loaded_origin = loaded.as_ref().ok().and_then(|wallet| wallet.metadata.origin.clone());
    let sync_state = database.cloud_blob_sync_states.get(&record_id).unwrap();

    drop(loaded);
    crate::wallet::test_support::remove_wallet(&id);

    (loaded_origin, expected_origin, sync_state)
}

// an address-type switch interrupted before its metadata commit is healed on the next load, and
// the healed address type and origin must reach a backup that was already confirmed. A hot
// wallet whose key is missing locally keeps its confirmed backup, since opening it downgrades it
// to watch-only and that upload would drop the key from the cloud
#[tokio::test(flavor = "current_thread")]
async fn healing_an_interrupted_wrapped_segwit_switch_marks_its_confirmed_backup_dirty() {
    use crate::wallet::WalletAddressType;

    let _guard = async_test_lock().lock().await;
    let globals = test_globals();
    let manager = init_manager();
    configure_enabled_cloud_backup(&manager, globals, 1);
    crate::app::reconcile::test_support::init_noop_updater();

    // offline keeps the dirty mark in place instead of starting an upload
    CONNECTIVITY_MANAGER.set_connection_state(false);

    let database = Database::global();
    database.global_config.set_selected_network(crate::network::Network::Bitcoin).unwrap();
    database.global_config.set_main_mode().unwrap();

    let with_key = heal_interrupted_wrapped_segwit_switch_with_confirmed_backup(true);
    let without_key = heal_interrupted_wrapped_segwit_switch_with_confirmed_backup(false);
    CONNECTIVITY_MANAGER.set_connection_state(true);

    for (stored, _) in [&with_key, &without_key] {
        assert_eq!(stored.address_type, WalletAddressType::WrappedSegwit);
        assert_eq!(stored.origin.as_deref(), Some("sh(wpkh([73c5da0a/49'/0'/0']))"));
    }

    let (_, sync_state) = with_key;
    assert!(
        matches!(
            sync_state,
            Some(PersistedCloudBlobSyncState { state: PersistedCloudBlobState::Dirty(_), .. })
        ),
        "{sync_state:?}"
    );

    let (_, sync_state) = without_key;
    assert!(
        matches!(
            sync_state,
            Some(PersistedCloudBlobSyncState { state: PersistedCloudBlobState::Confirmed(_), .. })
        ),
        "{sync_state:?}"
    );
}

/// Leaves a native segwit wallet with a confirmed backup mid-switch to wrapped segwit, then loads
/// it, returning the healed row and the backup's sync state after the load
fn heal_interrupted_wrapped_segwit_switch_with_confirmed_backup(
    keep_secret: bool,
) -> (crate::wallet::metadata::WalletMetadata, Option<PersistedCloudBlobSyncState>) {
    use crate::database::cloud_backup::CloudBlobConfirmedState;
    use crate::wallet::{Wallet, WalletAddressType, metadata::WalletMetadata};

    let database = Database::global();
    let mnemonic = Mnemonic::parse_normalized(
        "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about",
    )
    .unwrap();
    let mut wallet = Wallet::try_new_persisted_from_mnemonic_segwit(
        WalletMetadata::preview_new(),
        mnemonic.clone(),
        None,
    )
    .unwrap();
    let id = wallet.id.clone();
    Keychain::global()
        .save_wallet_secret(&id, cove_device::keychain::WalletSecret::Mnemonic(mnemonic))
        .unwrap();
    database.wallets.save_new_wallet_metadata(wallet.metadata.clone()).unwrap();

    // publishing the replacement store without the metadata commit is what an interruption leaves
    wallet.switch_private_wallet_to_new_address_type(WalletAddressType::WrappedSegwit).unwrap();
    drop(wallet);

    if !keep_secret {
        Keychain::global().delete_wallet_secret(&id);
    }

    let namespace_id = CloudBackupKeychain::global().namespace_id().unwrap().unwrap();
    let record_id = wallet_record_id(id.as_ref());
    let confirmed = PersistedCloudBlobSyncState::wallet(
        namespace_id,
        id.clone(),
        record_id.clone(),
        PersistedCloudBlobState::Confirmed(CloudBlobConfirmedState {
            revision_hash: "uploaded-as-native-segwit".into(),
            confirmed_at: 1,
        }),
    );
    database.cloud_blob_sync_states.set(&confirmed).unwrap();

    let loaded = Wallet::try_load_persisted(id.clone()).map(|_| ()).map_err(|e| e.to_string());
    let stored = database
        .wallets
        .get(&id, crate::network::Network::Bitcoin, crate::wallet::metadata::WalletMode::Main)
        .unwrap();
    let sync_state = database.cloud_blob_sync_states.get(&record_id).unwrap();

    crate::wallet::test_support::remove_wallet(&id);

    loaded.expect("the wallet reloads from the replacement store");
    (stored.expect("the wallet row exists"), sync_state)
}
