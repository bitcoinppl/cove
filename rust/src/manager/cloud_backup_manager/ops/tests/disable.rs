use super::*;
use crate::manager::cloud_backup_manager::GENERIC_CLOUD_BACKUP_ERROR_MESSAGE;

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

    assert_eq!(error, GENERIC_CLOUD_BACKUP_ERROR_MESSAGE);
    assert!(!globals.cloud.has_namespace(&namespace));
    let PersistedCloudBackupState::Disabling(disabling) =
        Database::global().cloud_backup_state.get().unwrap()
    else {
        panic!("expected persisted disabling state");
    };
    assert!(disabling.delete_started_at.is_some());
    assert!(disabling.last_error.as_deref() == Some(GENERIC_CLOUD_BACKUP_ERROR_MESSAGE));
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
async fn keep_cloud_backup_enabled_resumes_dirty_wallet_marked_during_disable_fence() {
    let _guard = async_test_lock().lock().await;
    cove_tokio::init();
    let globals = test_globals();
    let manager = init_manager();
    configure_enabled_cloud_backup(&manager, globals, 0);

    let metadata = xpub_only_wallet_metadata();
    persist_xpub_wallets(vec![metadata.clone()]);
    let namespace = CloudBackupKeychain::global().namespace_id().unwrap();
    let record_id = wallet_record_id(metadata.id.as_ref());
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
            started_at: 100,
            delete_started_at: None,
            last_error: None,
            retry_after: None,
        }))
        .unwrap();

    manager.handle_wallet_backup_change(metadata.id.clone());

    assert_eq!(globals.cloud.wallet_backup_upload_attempt_count(), 0);
    assert!(matches!(
        Database::global().cloud_blob_sync_states.get(&record_id).unwrap(),
        Some(PersistedCloudBlobSyncState { state: PersistedCloudBlobState::Dirty(_), .. })
    ));

    run_keep_cloud_backup_enabled(&manager).await;

    wait_for_test_condition(
        Duration::from_secs(5),
        "dirty wallet marked during disable fence uploads after Keep Enabled",
        || {
            let upload_attempted = globals.cloud.wallet_backup_upload_attempt_count() > 0;
            let uploaded_or_confirmed = matches!(
                Database::global().cloud_blob_sync_states.get(&record_id).unwrap(),
                Some(PersistedCloudBlobSyncState {
                    state: PersistedCloudBlobState::UploadedPendingConfirmation(_)
                        | PersistedCloudBlobState::Confirmed(_),
                    ..
                })
            );

            upload_attempted && uploaded_or_confirmed
        },
    )
    .await;
    assert!(matches!(
        Database::global().cloud_backup_state.get().unwrap(),
        PersistedCloudBackupState::Configured(_)
    ));

    clear_wallet_upload_runtime_for_test_async(&manager).await;
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
async fn disable_corrupted_persisted_state_fails_closed() {
    let _guard = async_test_lock().lock().await;
    cove_tokio::init();
    let globals = test_globals();
    let manager = init_manager();

    reset_cloud_backup_test_state(&manager, globals);
    manager
        .persist_cloud_backup_state(
            &PersistedCloudBackupState::corrupted("decode failed"),
            "persist corrupt state for test",
        )
        .unwrap();
    globals.cloud.fail_list_wallet_files("disable should not inspect cloud wallets");
    let list_wallet_files_attempt_count = globals.cloud.list_wallet_files_attempt_count();

    let error = manager.prepare_disable_cloud_backup().await.unwrap_err();

    assert!(matches!(
        error,
        CloudBackupError::Internal(message) if message == CORRUPTED_CLOUD_BACKUP_STATE_MESSAGE
    ));
    assert_eq!(
        Database::global().cloud_backup_state.get().unwrap().status(),
        PersistedCloudBackupStatus::Corrupted
    );
    assert!(matches!(
        manager.current_status(),
        CloudBackupStatus::Error(message) if message == CORRUPTED_CLOUD_BACKUP_STATE_MESSAGE
    ));
    assert_eq!(globals.cloud.list_wallet_files_attempt_count(), list_wallet_files_attempt_count);
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

    assert_eq!(error, GENERIC_CLOUD_BACKUP_ERROR_MESSAGE);
    let PersistedCloudBackupState::Disabling(disabling) =
        Database::global().cloud_backup_state.get().unwrap()
    else {
        panic!("expected persisted disabling state");
    };
    assert!(disabling.delete_started_at.is_some());
    assert_eq!(disabling.last_error.as_deref(), Some(GENERIC_CLOUD_BACKUP_ERROR_MESSAGE));
    assert_eq!(CloudBackupKeychain::global().namespace_id().as_deref(), Some(namespace.as_str()));
    assert_eq!(current_disable_generation(), Some(disabling.disable_generation));
}

#[tokio::test(flavor = "current_thread")]
async fn disable_cloud_backup_not_found_listing_retries_then_fails_closed() {
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
    let error = disable_failure_message(&manager);

    assert_eq!(
        error,
        CloudBackupError::from(CloudStorageError::NotFound(String::new())).reader_message()
    );
    assert_eq!(globals.cloud.list_wallet_files_attempt_count_for_namespace(&namespace), 4);
    assert_eq!(globals.cloud.delete_namespace_attempt_count(), 0);
    assert!(globals.cloud.has_namespace(&namespace));
    assert_eq!(CloudBackupKeychain::global().namespace_id().as_deref(), Some(namespace.as_str()));
    let PersistedCloudBackupState::Disabling(disabling) =
        Database::global().cloud_backup_state.get().unwrap()
    else {
        panic!("expected persisted disabling state");
    };
    assert!(disabling.delete_started_at.is_none());
    assert_eq!(current_disable_generation(), Some(disabling.disable_generation));
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
    wait_for_test_condition(Duration::from_secs(2), "restart disable recovery finishes", || {
        !globals.cloud.has_namespace(&namespace)
            && Database::global().cloud_backup_state.get().unwrap().status()
                == PersistedCloudBackupStatus::Disabled
            && CloudBackupKeychain::global().namespace_id().is_none()
            && restarted_manager.model_snapshot().status == CloudBackupStatus::Disabled
    })
    .await;

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
    assert_eq!(error, GENERIC_CLOUD_BACKUP_ERROR_MESSAGE);

    run_keep_cloud_backup_enabled(&manager).await;

    assert_eq!(
        Database::global().cloud_backup_state.get().unwrap().status(),
        PersistedCloudBackupStatus::Enabled
    );
    assert_eq!(CloudBackupKeychain::global().namespace_id().as_deref(), Some(namespace.as_str()));
    assert!(current_disable_generation().is_none());
}
