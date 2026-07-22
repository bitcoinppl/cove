use super::*;
use crate::manager::cloud_backup_manager::GENERIC_CLOUD_BACKUP_ERROR_MESSAGE;

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
async fn deep_verify_corrupted_persisted_state_short_circuits() {
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
    globals.cloud.fail_list_namespaces("verify should not read cloud storage");
    globals.passkey.set_discover_result(Err(PasskeyError::UserCancelled));

    let step = manager.prepare_deep_verify_cloud_backup(true).await;

    assert!(matches!(
        step,
        CloudBackupDeepVerificationStep::Complete(DeepVerificationResult::NotEnabled)
    ));
    assert_eq!(globals.passkey.discover_count(), 0);
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
                crate::manager::cloud_backup_manager::error::GENERIC_CLOUD_BACKUP_ERROR_MESSAGE
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
            assert_eq!(
                message,
                crate::manager::cloud_backup_manager::error::GENERIC_CLOUD_BACKUP_ERROR_MESSAGE
            );
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
        || {
            let state = manager.model_snapshot();
            manager.pending_verification_completion().is_some()
                && state.pending_upload_verification == PendingUploadVerificationState::Confirming
                && matches!(state.verification, VerificationState::Idle)
                && state.detail.is_some()
        },
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
            assert_eq!(
                message,
                crate::manager::cloud_backup_manager::error::GENERIC_CLOUD_BACKUP_ERROR_MESSAGE
            );
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
async fn deep_verify_reads_each_wallet_backup_once() {
    let _guard = async_test_lock().lock().await;
    cove_tokio::init();
    let globals = test_globals();
    let manager = init_manager();

    configure_enabled_cloud_backup(&manager, globals, 1);
    seed_verifiable_cloud_master_key(globals);

    let namespace = CloudBackupKeychain::global().namespace_id().unwrap();
    let master_key = cove_cspp::Cspp::new(Keychain::global().clone())
        .load_master_key_from_store()
        .unwrap()
        .unwrap();
    let metadata = xpub_only_wallet_metadata();
    let record_id = wallet_record_id(metadata.id.as_ref());
    persist_xpub_wallets(vec![metadata.clone()]);

    let prepared = crate::manager::cloud_backup_manager::wallets::prepare_wallet_backup(
        &metadata,
        metadata.wallet_mode,
    )
    .await
    .unwrap();

    globals.cloud.set_wallet_backup(
        namespace.clone(),
        record_id.clone(),
        encrypted_wallet_backup_bytes(&metadata, &master_key, &prepared.revision_hash, 1).await,
    );
    globals.cloud.set_wallet_files(namespace, vec![wallet_filename_from_record_id(&record_id)]);

    let downloads_before = globals.cloud.wallet_backup_download_attempt_count();

    let step = manager.prepare_deep_verify_cloud_backup(true).await;

    assert_eq!(globals.cloud.wallet_backup_download_attempt_count() - downloads_before, 1);

    call!(manager.supervisor.complete_verification(
        None,
        step,
        DeepVerificationContinuation::Manual {
            force_discoverable: true,
            attempt: VerificationAttempt::Initial,
        }
    ))
    .await
    .unwrap();
    wait_for_test_condition(Duration::from_secs(8), "deep verification completes", || {
        matches!(manager.model_snapshot().verification, VerificationState::Verified(_))
    })
    .await;

    let result = match manager.model_snapshot().verification {
        VerificationState::Verified(report) => DeepVerificationResult::Verified(report),
        other => panic!("expected verified result, got {other:?}"),
    };

    match result {
        DeepVerificationResult::Verified(report) => {
            assert_eq!(report.wallets_verified, 1);
            assert_eq!(report.wallets_failed, 0);
            assert_eq!(report.wallets_unsupported, 0);

            let detail = report.detail.expect("expected verification detail");
            assert_eq!(detail.up_to_date.len(), 1);
            assert!(detail.needs_sync.is_empty());
            assert_eq!(detail.up_to_date[0].record_id, record_id);
        }
        other => panic!("expected verified result, got {other:?}"),
    }

    assert_eq!(globals.cloud.wallet_backup_download_attempt_count() - downloads_before, 1);
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
            assert_eq!(message, GENERIC_CLOUD_BACKUP_ERROR_MESSAGE);

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
