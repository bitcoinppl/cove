use super::*;
use crate::manager::cloud_backup_manager::GENERIC_CLOUD_BACKUP_ERROR_MESSAGE;
use crate::manager::cloud_backup_manager::ops::{CloudBackupDisablePreparation, DisableAttempt};
use crate::manager::cloud_backup_manager::owner::operation::test_support::operation_id;

/// A Turn Off stopped before its commit point leaves the backup configured, with nothing to
/// keep enabled, and returns the failure message
fn assert_blocked_disable_left_backup_configured(manager: &RustCloudBackupManager) -> String {
    assert_eq!(
        Database::global().cloud_backup_state.get().unwrap().status(),
        PersistedCloudBackupStatus::Enabled
    );
    assert!(current_disable_generation().is_none());

    let CloudBackupLifecycle::Configured(configured) = manager.model_state().lifecycle else {
        panic!("expected configured cloud backup lifecycle");
    };
    let CloudBackupDestructiveOperationState::DisableFailed { message, can_keep_enabled } =
        configured.destructive_operation
    else {
        panic!("expected disable failure");
    };

    assert!(!can_keep_enabled);
    message
}

#[tokio::test(flavor = "current_thread")]
async fn disable_cloud_backup_deletes_active_namespace_and_clears_local_cloud_state() {
    let _guard = async_test_lock().lock().await;
    let globals = test_globals();
    let manager = init_manager();
    configure_enabled_cloud_backup(&manager, globals, 0);

    let namespace = CloudBackupKeychain::global().namespace_id().unwrap().unwrap();
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
    assert!(CloudBackupKeychain::global().namespace_id().unwrap().is_none());
    assert!(
        globals.cloud.deleted_namespace_policies().contains(&CloudAccessPolicy::ConsentAllowed)
    );
}

#[tokio::test(flavor = "current_thread")]
async fn disable_cloud_backup_keeps_disabling_state_when_local_cleanup_fails() {
    let _guard = async_test_lock().lock().await;
    let globals = test_globals();
    let manager = init_manager();
    configure_enabled_cloud_backup(&manager, globals, 0);

    let namespace = CloudBackupKeychain::global().namespace_id().unwrap().unwrap();
    globals.cloud.set_master_key_backup(namespace.clone(), vec![1, 2, 3]);
    globals.keychain.fail_delete_at(1);

    run_disable_cloud_backup(&manager).await;
    let error = disable_failure_message(&manager);

    assert_eq!(error, GENERIC_CLOUD_BACKUP_ERROR_MESSAGE);
    assert!(!globals.cloud.has_namespace(&namespace));
    let disabling = persisted_disabling();
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

/// Turn Off's local cleanup reaches Disabled from any partial state: whichever cleanup delete
/// fails, and whichever of the namespace, credential and master key are already gone when it
/// retries, the retry finishes and leaves none of them
#[tokio::test(flavor = "current_thread")]
async fn turn_off_cleanup_finishes_from_every_partial_local_state() {
    use cove_device::keychain::KeychainAccess as _;

    use crate::manager::cloud_backup_manager::keychain::{
        CSPP_CREDENTIAL_ID_KEY, CSPP_NAMESPACE_ID_KEY,
    };

    let _guard = async_test_lock().lock().await;
    let globals = test_globals();
    let manager = init_manager();
    let mut failed_deletes = 0;

    for failing_delete in 1.. {
        let mut reached_cleanup = false;
        for missing in 0..8u8 {
            configure_enabled_cloud_backup(&manager, globals, 0);
            let namespace = CloudBackupKeychain::global().namespace_id().unwrap().unwrap();
            globals.cloud.set_master_key_backup(namespace.clone(), vec![1, 2, 3]);

            globals.keychain.fail_delete_at(failing_delete);
            run_disable_cloud_backup(&manager).await;
            globals.keychain.fail_delete_at(usize::MAX);

            // past the cleanup's last delete, nothing fails and Turn Off finishes at once
            let status = Database::global().cloud_backup_state.get().unwrap().status();
            if status == PersistedCloudBackupStatus::Disabled {
                break;
            }

            assert!(persisted_disabling().last_error.is_some(), "delete {failing_delete}");

            reached_cleanup = true;
            failed_deletes += 1;
            let keychain = CloudBackupKeychain::global();
            if missing & 1 != 0 {
                globals.keychain.delete(CSPP_NAMESPACE_ID_KEY.to_string());
            }
            if missing & 2 != 0 {
                globals.keychain.delete(CSPP_CREDENTIAL_ID_KEY.to_string());
            }
            if missing & 4 != 0 {
                keychain.delete_master_key().unwrap();
            }

            run_disable_cloud_backup(&manager).await;
            let case = format!("delete {failing_delete} failed, missing set {missing:03b}");
            assert_eq!(
                Database::global().cloud_backup_state.get().unwrap().status(),
                PersistedCloudBackupStatus::Disabled,
                "{case}"
            );
            assert_eq!(keychain.namespace_id().unwrap(), None, "{case}");
            assert_eq!(keychain.load_credential_id().unwrap(), None, "{case}");
            assert!(!globals.cloud.has_namespace(&namespace), "{case}");
        }

        if !reached_cleanup {
            break;
        }
    }

    // the configured backup holds a master key, namespace and credential, so the cleanup makes
    // three deletes; each failed once, under every one of the 8 combinations
    assert_eq!(failed_deletes, 3 * 8);
}

#[tokio::test(flavor = "current_thread")]
async fn disable_cloud_backup_blocks_cloud_only_wallets_without_deleting_namespace() {
    let _guard = async_test_lock().lock().await;
    let globals = test_globals();
    let manager = init_manager();
    configure_enabled_cloud_backup(&manager, globals, 0);

    let namespace = CloudBackupKeychain::global().namespace_id().unwrap().unwrap();
    globals.cloud.set_master_key_backup(namespace.clone(), vec![1, 2, 3]);
    globals
        .cloud
        .set_wallet_files(namespace.clone(), vec![wallet_filename_from_record_id("cloud-only")]);

    run_disable_cloud_backup(&manager).await;
    let error = assert_blocked_disable_left_backup_configured(&manager);

    assert!(error.contains("cloud-only wallets"), "{error}");
    assert!(globals.cloud.has_namespace(&namespace));
}

#[tokio::test(flavor = "current_thread")]
async fn blocked_disable_can_be_retried_once_the_blocker_is_gone() {
    let _guard = async_test_lock().lock().await;
    let globals = test_globals();
    let manager = init_manager();
    configure_enabled_cloud_backup(&manager, globals, 0);

    let namespace = CloudBackupKeychain::global().namespace_id().unwrap().unwrap();
    globals.cloud.set_master_key_backup(namespace.clone(), vec![1, 2, 3]);
    globals
        .cloud
        .set_wallet_files(namespace.clone(), vec![wallet_filename_from_record_id("cloud-only")]);

    run_disable_cloud_backup(&manager).await;
    assert_blocked_disable_left_backup_configured(&manager);

    // the blocked attempt released its claim and write fence, so a retry runs to the end
    globals.cloud.set_wallet_files(namespace.clone(), Vec::new());
    run_disable_cloud_backup(&manager).await;

    assert!(!globals.cloud.has_namespace(&namespace));
    assert_eq!(
        Database::global().cloud_backup_state.get().unwrap().status(),
        PersistedCloudBackupStatus::Disabled
    );
}

#[tokio::test(flavor = "current_thread")]
async fn disable_cloud_backup_blocks_other_namespaces_without_deleting_them() {
    let _guard = async_test_lock().lock().await;
    let globals = test_globals();
    let manager = init_manager();
    configure_enabled_cloud_backup(&manager, globals, 0);

    let namespace = CloudBackupKeychain::global().namespace_id().unwrap().unwrap();
    let other_namespace = cove_cspp::master_key::MasterKey::generate().namespace_id();
    globals.cloud.set_master_key_backup(namespace.clone(), vec![1, 2, 3]);
    globals.cloud.set_master_key_backup(other_namespace.clone(), vec![4, 5, 6]);
    let delete_attempt_count = globals.cloud.delete_namespace_attempt_count();

    run_disable_cloud_backup(&manager).await;
    let error = assert_blocked_disable_left_backup_configured(&manager);

    assert!(error.contains("other cloud backups"), "{error}");
    assert!(globals.cloud.has_namespace(&namespace));
    assert!(globals.cloud.has_namespace(&other_namespace));
    assert_eq!(globals.cloud.delete_namespace_attempt_count(), delete_attempt_count);
}

#[tokio::test(flavor = "current_thread")]
async fn disable_cloud_backup_blocks_active_exclusive_operation_without_persisting_disabling() {
    let _guard = async_test_lock().lock().await;
    let globals = test_globals();
    let manager = init_manager();
    configure_enabled_cloud_backup(&manager, globals, 0);

    let namespace = CloudBackupKeychain::global().namespace_id().unwrap().unwrap();
    globals.cloud.set_master_key_backup(namespace.clone(), vec![1, 2, 3]);
    let claim = CloudBackupExclusiveOperationClaim::new(
        CloudBackupExclusiveOperation::RecreateManifest,
        operation_id(1),
    );
    manager.project_exclusive_operation_started(claim);

    let error = manager.prepare_disable_cloud_backup().await.unwrap_err();

    assert!(error.to_string().contains("another cloud backup operation"), "{error}");
    assert!(globals.cloud.has_namespace(&namespace));
    assert_eq!(
        Database::global().cloud_backup_state.get().unwrap().status(),
        PersistedCloudBackupStatus::Enabled
    );
    assert_eq!(
        manager.projected_exclusive_operation().map(CloudBackupExclusiveOperationClaim::operation),
        Some(CloudBackupExclusiveOperation::RecreateManifest)
    );
    manager.project_exclusive_operation_finished(claim);
}

/// Each Turn Off attempt gets its own id, so a result from an earlier attempt can never pass as
/// the current one
#[tokio::test(flavor = "current_thread")]
async fn each_turn_off_attempt_gets_its_own_id_and_persists_nothing() {
    let _guard = async_test_lock().lock().await;
    let globals = test_globals();
    let manager = init_manager();
    configure_enabled_cloud_backup(&manager, globals, 0);

    let mut attempt_ids = Vec::new();
    for _ in 0..2 {
        let preparation = manager.prepare_disable_cloud_backup().await.unwrap();
        let CloudBackupDisablePreparation::Ready(attempt) = preparation else {
            panic!("expected a Turn Off attempt, got {preparation:?}");
        };
        let DisableAttempt::Uncommitted { attempt_id, .. } = *attempt else {
            panic!("expected an uncommitted attempt, got {attempt:?}");
        };
        attempt_ids.push(attempt_id);
    }

    assert_ne!(attempt_ids[0], attempt_ids[1]);
    assert_eq!(
        Database::global().cloud_backup_state.get().unwrap().status(),
        PersistedCloudBackupStatus::Enabled
    );
}

#[tokio::test(flavor = "current_thread")]
async fn disable_corrupted_persisted_state_fails_closed() {
    let _guard = async_test_lock().lock().await;
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
    let globals = test_globals();
    let manager = init_manager();
    configure_enabled_cloud_backup(&manager, globals, 0);

    let namespace = CloudBackupKeychain::global().namespace_id().unwrap().unwrap();
    globals.cloud.set_master_key_backup(namespace.clone(), vec![1, 2, 3]);
    globals.cloud.fail_delete_namespace("delete failed");

    run_disable_cloud_backup(&manager).await;
    let error = disable_failure_message(&manager);

    assert_eq!(error, GENERIC_CLOUD_BACKUP_ERROR_MESSAGE);
    let disabling = persisted_disabling();
    assert!(disabling.delete_started_at.is_some());
    assert_eq!(disabling.last_error.as_deref(), Some(GENERIC_CLOUD_BACKUP_ERROR_MESSAGE));
    assert_eq!(
        CloudBackupKeychain::global().namespace_id().unwrap().as_deref(),
        Some(namespace.as_str())
    );
    assert_eq!(current_disable_generation(), Some(disabling.disable_generation));
}

/// A committed Turn Off whose delete fails and whose failure can't be saved still shows the
/// failure and keeps its commit, so Try Again finishes it once the faults clear
#[tokio::test(flavor = "current_thread")]
async fn turn_off_whose_failure_cannot_be_saved_finishes_on_try_again() {
    let _guard = async_test_lock().lock().await;
    let globals = test_globals();
    let manager = init_manager();
    configure_enabled_cloud_backup(&manager, globals, 0);

    let namespace = CloudBackupKeychain::global().namespace_id().unwrap().unwrap();
    globals.cloud.set_master_key_backup(namespace.clone(), vec![1, 2, 3]);
    globals.cloud.fail_delete_namespace("delete failed");
    Database::global().cloud_backup_state.fail_next_write_where(|state| {
        state.disabling().is_some_and(|disabling| disabling.last_error.is_some())
    });

    run_disable_cloud_backup(&manager).await;

    assert_eq!(disable_failure_message(&manager), GENERIC_CLOUD_BACKUP_ERROR_MESSAGE);
    let disabling = persisted_disabling();
    assert!(disabling.delete_started_at.is_some());
    assert_eq!(disabling.last_error, None, "the failure was saved after all");
    assert!(globals.cloud.has_namespace(&namespace));

    globals.cloud.clear_delete_namespace_failure();
    run_disable_cloud_backup(&manager).await;

    assert_eq!(
        Database::global().cloud_backup_state.get().unwrap().status(),
        PersistedCloudBackupStatus::Disabled
    );
    assert!(!globals.cloud.has_namespace(&namespace));
}

#[tokio::test(flavor = "current_thread")]
async fn disable_cloud_backup_not_found_listing_retries_then_fails_closed() {
    let _guard = async_test_lock().lock().await;
    let globals = test_globals();
    let manager = init_manager();
    configure_enabled_cloud_backup(&manager, globals, 0);

    let namespace = CloudBackupKeychain::global().namespace_id().unwrap().unwrap();
    globals.cloud.set_master_key_backup(namespace.clone(), vec![1, 2, 3]);
    globals.cloud.fail_list_wallet_files_for_namespace(
        namespace.clone(),
        CloudStorageError::NotFound("missing".into()),
    );

    run_disable_cloud_backup(&manager).await;
    let error = assert_blocked_disable_left_backup_configured(&manager);

    assert_eq!(
        error,
        CloudBackupError::from(CloudStorageError::NotFound(String::new())).reader_message()
    );
    assert_eq!(globals.cloud.list_wallet_files_attempt_count_for_namespace(&namespace), 4);
    assert_eq!(globals.cloud.delete_namespace_attempt_count(), 0);
    assert!(globals.cloud.has_namespace(&namespace));
    assert_eq!(
        CloudBackupKeychain::global().namespace_id().unwrap().as_deref(),
        Some(namespace.as_str())
    );
}

#[tokio::test(flavor = "current_thread")]
async fn disable_cloud_backup_delete_not_found_finishes_cleanup() {
    let _guard = async_test_lock().lock().await;
    let globals = test_globals();
    let manager = init_manager();
    configure_enabled_cloud_backup(&manager, globals, 0);

    let namespace = CloudBackupKeychain::global().namespace_id().unwrap().unwrap();
    globals.cloud.set_master_key_backup(namespace.clone(), vec![1, 2, 3]);
    globals.cloud.fail_delete_namespace_not_found("already deleted");

    run_disable_cloud_backup(&manager).await;

    assert_eq!(
        Database::global().cloud_backup_state.get().unwrap().status(),
        PersistedCloudBackupStatus::Disabled
    );
    assert!(CloudBackupKeychain::global().namespace_id().unwrap().is_none());
}

#[tokio::test(flavor = "current_thread")]
async fn persisted_disabling_on_restart_resumes_to_disabled() {
    let _guard = async_test_lock().lock().await;
    let globals = test_globals();
    let manager = init_manager();
    configure_enabled_cloud_backup(&manager, globals, 0);

    let namespace = CloudBackupKeychain::global().namespace_id().unwrap().unwrap();
    globals.cloud.set_master_key_backup(namespace.clone(), vec![1, 2, 3]);
    let PersistedCloudBackupState::Configured(previous_configured) =
        Database::global().cloud_backup_state.get().unwrap()
    else {
        panic!("expected configured cloud backup state");
    };

    Database::global()
        .cloud_backup_state
        .set(&PersistedCloudBackupState::disabling_transition(PersistedDisablingCloudBackup {
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
            && CloudBackupKeychain::global().namespace_id().unwrap().is_none()
            && restarted_manager.model_snapshot().status == CloudBackupStatus::Disabled
    })
    .await;

    assert!(!globals.cloud.has_namespace(&namespace));
    assert_eq!(
        Database::global().cloud_backup_state.get().unwrap().status(),
        PersistedCloudBackupStatus::Disabled
    );
    assert!(CloudBackupKeychain::global().namespace_id().unwrap().is_none());
    assert_eq!(restarted_manager.model_snapshot().status, CloudBackupStatus::Disabled);
}

/// Turn Off's drain and namespace delete are lane commands: while an upload is the write in
/// flight, the disable neither commits nor deletes, and it finishes once the upload returns
#[tokio::test(flavor = "current_thread")]
async fn turn_off_waits_for_the_write_in_flight_before_it_commits_and_deletes() {
    let _guard = async_test_lock().lock().await;
    let globals = test_globals();
    let manager = init_manager();
    configure_enabled_cloud_backup(&manager, globals, 0);

    let metadata = xpub_only_wallet_metadata();
    persist_xpub_wallets(vec![metadata.clone()]);
    let namespace = CloudBackupKeychain::global().namespace_id().unwrap().unwrap();
    globals.cloud.set_master_key_backup(namespace.clone(), vec![1, 2, 3]);
    let (upload_gate, upload) =
        hold_wallet_upload_in_flight(&manager, globals, metadata.id.clone()).await;

    call!(manager.supervisor.start_disable_operation()).await.unwrap();
    wait_for_queued_lane_write(&manager).await;

    assert!(matches!(
        Database::global().cloud_backup_state.get().unwrap(),
        PersistedCloudBackupState::Configured(_)
    ));
    assert_eq!(globals.cloud.delete_namespace_attempt_count(), 0);

    upload_gate.release();
    upload.await.unwrap().unwrap();
    wait_for_test_condition(
        Duration::from_secs(5),
        "the disable finishes after the upload",
        || manager.projected_exclusive_operation().is_none(),
    )
    .await;

    assert!(!globals.cloud.has_namespace(&namespace));
    assert_eq!(
        Database::global().cloud_backup_state.get().unwrap().status(),
        PersistedCloudBackupStatus::Disabled
    );
}
