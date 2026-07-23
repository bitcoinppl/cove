use super::*;
use crate::database::cloud_backup::CloudBlobConfirmedState;

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
            crate::manager::cloud_backup_manager::current_timestamp(),
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
async fn pending_upload_verification_blocks_on_cloud_authorization() {
    let _guard = async_test_lock().lock().await;
    cove_tokio::init();
    let globals = test_globals();
    let manager = global_manager();
    clear_wallet_upload_runtime_for_test_async(&manager).await;
    configure_enabled_cloud_backup(&manager, globals, 0);

    let namespace_id = CloudBackupKeychain::global().namespace_id().unwrap();
    persist_pending_master_key_confirmation(namespace_id.clone(), "pending");
    globals.cloud.set_master_key_backup(namespace_id.clone(), vec![1, 2, 3]);
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
            namespace_id.clone(),
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
    globals.cloud.set_wallet_backup(namespace_id, record_id.clone(), vec![1, 2, 3]);
    globals.cloud.dirty_wallet_on_next_backup_check(metadata.id.clone());

    let has_more_pending = verify_pending_uploads_once_for_test_async(&manager).await;

    assert!(!has_more_pending);
    assert!(matches!(
        Database::global().cloud_blob_sync_states.get(&record_id).unwrap(),
        Some(PersistedCloudBlobSyncState { state: PersistedCloudBlobState::Dirty(_), .. })
    ));
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
    manager
        .replace_pending_verification_completion(PendingVerificationCompletion::new(
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
        ))
        .unwrap();
    globals.cloud.set_master_key_backup(namespace_id.clone(), vec![1, 2, 3]);
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
    manager
        .replace_pending_verification_completion(PendingVerificationCompletion::new(
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
        ))
        .unwrap();

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
async fn pending_upload_verification_does_not_read_locally_staged_master_key_as_remote() {
    let _guard = async_test_lock().lock().await;
    cove_tokio::init();
    let globals = test_globals();
    let manager = init_manager();
    configure_enabled_cloud_backup(&manager, globals, 0);

    let namespace_id = CloudBackupKeychain::global().namespace_id().unwrap();
    let expected_master_json = vec![1, 2, 3];
    let expected_revision = master_key_wrapper_revision_hash(&expected_master_json);
    persist_pending_master_key_confirmation(namespace_id.clone(), expected_revision.clone());
    globals.cloud.set_master_key_backup(namespace_id, expected_master_json);
    globals.cloud.set_uploaded_master_key_pending_confirmation(true);
    let downloads_before = globals.cloud.master_key_download_attempt_count();

    let has_more_pending = verify_pending_uploads_once_for_test_async(&manager).await;

    assert!(has_more_pending);
    assert_eq!(globals.cloud.master_key_download_attempt_count(), downloads_before);

    globals.cloud.set_uploaded_master_key_pending_confirmation(false);
    let has_more_pending = verify_pending_uploads_once_for_test_async(&manager).await;

    assert!(!has_more_pending);
    assert!(globals.cloud.master_key_download_attempt_count() > downloads_before);
    let confirmed = Database::global()
        .cloud_blob_sync_states
        .get(cove_cspp::backup_data::MASTER_KEY_RECORD_ID)
        .unwrap()
        .expect("confirmed master key sync state");
    assert!(matches!(
        confirmed.state,
        PersistedCloudBlobState::Confirmed(CloudBlobConfirmedState { revision_hash, .. })
            if revision_hash == expected_revision
    ));
}

#[tokio::test(flavor = "current_thread")]
async fn pending_upload_verification_expires_stale_completion() {
    let _guard = async_test_lock().lock().await;
    cove_tokio::init();
    let globals = test_globals();
    let manager = init_manager();
    configure_enabled_cloud_backup(&manager, globals, 0);

    let namespace_id = CloudBackupKeychain::global().namespace_id().unwrap();
    manager
        .replace_pending_verification_completion(PendingVerificationCompletion::new(
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
        ))
        .unwrap();

    let mut state = RustCloudBackupManager::load_persisted_state();
    match &mut state {
        PersistedCloudBackupState::Configured(configured) => {
            configured
                .pending_verification_completion
                .as_mut()
                .expect("pending verification completion")
                .created_at = Some(0);
        }
        other => panic!("expected configured cloud backup state, got {other:?}"),
    }
    Database::global().cloud_backup_state.set(&state).unwrap();

    manager.finalize_pending_verification_if_ready().await;

    assert!(manager.pending_verification_completion().is_none());
    assert_eq!(
        manager.model_snapshot().pending_upload_verification,
        PendingUploadVerificationState::Idle
    );
    match manager.model_snapshot().verification {
        VerificationState::Failed(DeepVerificationFailure::Retry { message, .. }) => {
            assert_eq!(
                message,
                "cloud backup upload confirmation expired; start verification again"
            );
        }
        other => panic!("expected retry failure for stale completion, got {other:?}"),
    }
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
            crate::manager::cloud_backup_manager::current_timestamp(),
        )
        .unwrap();
    manager
        .replace_pending_verification_completion(PendingVerificationCompletion::new(
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
        ))
        .unwrap();

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
async fn terminally_failed_pending_upload_finishes_verification_without_waiting_for_ttl() {
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

    assert!(!has_more_pending);
    assert!(manager.pending_verification_completion().is_none());
    assert!(!manager.has_pending_cloud_upload_verification());
    match manager.model_snapshot().verification {
        VerificationState::Failed(DeepVerificationFailure::Retry { message, .. }) => {
            assert!(message.contains("terminal upload failure"), "{message}");
        }
        other => panic!("expected terminal upload verification failure, got {other:?}"),
    }
}
