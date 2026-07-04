use super::*;

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
async fn automatic_saved_passkey_confirmation_retries_until_available() {
    let _guard = async_test_lock().lock().await;
    cove_tokio::init();
    let globals = test_globals();
    let manager = init_manager();

    reset_cloud_backup_test_state(&manager, globals);
    CONNECTIVITY_MANAGER.set_connection_state(true);
    globals.passkey.set_create_result(Ok(vec![1, 2, 3]));
    globals.passkey.set_authenticate_result(Err(PasskeyError::NoCredentialFound));
    globals.passkey.push_authenticate_result(Err(PasskeyError::NoCredentialFound));
    globals.passkey.push_authenticate_result(Ok(vec![7; 32]));

    let context = CloudBackupEnableContext {
        saved_passkey_confirmation: SavedPasskeyConfirmationMode::Automatic,
        verification_source: CloudBackupVerificationSource::Onboarding,
    };
    enable_cloud_backup_no_discovery_with_context(&manager, context).await.unwrap();

    assert_eq!(globals.passkey.create_count(), 1);
    assert_eq!(globals.passkey.authenticate_count(), 3);
    assert_eq!(
        globals.passkey.authenticated_credential_ids(),
        vec![vec![1, 2, 3], vec![1, 2, 3], vec![1, 2, 3]]
    );
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

#[tokio::test(flavor = "current_thread")]
async fn discard_pending_enable_retry_upload_treats_missing_remote_master_key_as_deleted() {
    let _guard = async_test_lock().lock().await;
    cove_tokio::init();
    let globals = test_globals();
    let manager = init_manager();

    reset_cloud_backup_test_state(&manager, globals);

    let master_key = cove_cspp::master_key::MasterKey::generate();
    let cspp = cove_cspp::Cspp::new(Keychain::global().clone());
    cspp.save_master_key(&master_key).unwrap();
    globals.cloud.fail_delete_wallet_backup_not_found("already deleted");
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
        "pending enable session should be discarded after missing remote cleanup",
        || cspp.load_master_key_from_store().unwrap().is_none(),
    )
    .await;

    assert!(take_pending_enable_session_for_test(&manager).await.is_none());
    assert!(cspp.load_master_key_from_store().unwrap().is_none());
}

#[tokio::test(flavor = "current_thread")]
async fn discard_pending_enable_retry_upload_keeps_pending_state_when_remote_cleanup_fails() {
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
    globals.cloud.fail_delete_wallet_backup("delete failed");
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
        "discard pending enable cleanup failure should surface",
        || matches!(manager.state().lifecycle, CloudBackupLifecycle::Failed(_)),
    )
    .await;

    assert!(globals.cloud.has_master_key_backup(&namespace));
    assert!(cspp.load_master_key_from_store().unwrap().is_some());
    assert!(take_pending_enable_session_for_test(&manager).await.is_some());
}
