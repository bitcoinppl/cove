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
    run_reinitialize_backup_operation(&manager).await.unwrap();

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
async fn automatic_saved_passkey_confirmation_exhaustion_preserves_staged_session() {
    let _guard = async_test_lock().lock().await;
    cove_tokio::init();
    let globals = test_globals();
    let manager = init_manager();

    reset_cloud_backup_test_state(&manager, globals);
    CONNECTIVITY_MANAGER.set_connection_state(true);
    globals.passkey.set_create_result(Ok(vec![1, 2, 3]));
    globals.passkey.set_authenticate_result(Err(PasskeyError::NoCredentialFound));

    let context = CloudBackupEnableContext {
        saved_passkey_confirmation: SavedPasskeyConfirmationMode::Automatic,
        verification_source: CloudBackupVerificationSource::Onboarding,
    };
    enable_cloud_backup_no_discovery_with_context(&manager, context).await.unwrap();

    assert_eq!(globals.passkey.create_count(), 1);
    assert_eq!(globals.passkey.authenticate_count(), 1);
    assert_eq!(globals.passkey.authenticated_credential_ids(), vec![vec![1, 2, 3]]);
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
async fn post_presentation_saved_passkey_failure_attempts_confirmation_once() {
    let _guard = async_test_lock().lock().await;
    cove_tokio::init();
    let globals = test_globals();
    let manager = init_manager();

    reset_cloud_backup_test_state(&manager, globals);
    CONNECTIVITY_MANAGER.set_connection_state(true);
    globals.passkey.set_create_result(Ok(vec![1, 2, 3]));
    globals.passkey.set_authenticate_result(Err(PasskeyError::RequestFailed {
        operation: PasskeyOperation::AuthenticateAssertion,
        reason: PasskeyFailureReason::PlatformAuthorizationFailedAfterPresentation,
    }));

    let context = CloudBackupEnableContext {
        saved_passkey_confirmation: SavedPasskeyConfirmationMode::Automatic,
        verification_source: CloudBackupVerificationSource::Onboarding,
    };
    enable_cloud_backup_no_discovery_with_context(&manager, context).await.unwrap();

    assert_eq!(globals.passkey.authenticate_count(), 1);
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
async fn duplicate_confirm_saved_passkey_dispatches_are_ignored_while_confirming() {
    let _guard = async_test_lock().lock().await;
    cove_tokio::init();
    let globals = test_globals();
    let manager = init_manager();

    reset_cloud_backup_test_state(&manager, globals);
    CONNECTIVITY_MANAGER.set_connection_state(true);
    let master_key = cove_cspp::master_key::MasterKey::generate();
    let cspp = cove_cspp::Cspp::new(Keychain::global().clone());
    cspp.save_staged_master_key(&master_key).unwrap();
    let mut journal = staged_pending_enable_journal(
        CloudBackupEnableContext::settings_manual(),
        master_key.namespace_id(),
        PendingEnableNamespaceOwnership::FreshOwned,
        CloudBackupKeychain::global().snapshot_passkey_metadata(),
    );
    assert!(journal.register_passkey(PendingEnablePasskeyMetadata {
        credential_id: vec![1, 2, 3],
        prf_salt: [9; 32],
        provider_hint: None,
    }));
    CloudBackupKeychain::global().save_pending_enable_journal(&journal).unwrap();
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
    let snapshot = manager.model_snapshot();
    assert!(
        matches!(snapshot.verification, VerificationState::Verified(_)),
        "unexpected final model snapshot: {snapshot:#?}; pending completion: {:#?}; pending journal: {:#?}",
        manager.pending_verification_completion(),
        CloudBackupKeychain::global().load_pending_enable_journal()
    );
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

struct PendingEnableDiscardFixture {
    prior_master_key: [u8; 32],
    staged_master_key: [u8; 32],
    namespace: String,
    passkey: UnpersistedPrfKey,
    previous_metadata: crate::manager::cloud_backup_manager::PendingEnableLocalMetadataSnapshot,
}

fn prepare_pending_enable_discard_fixture(
    ownership: PendingEnableNamespaceOwnership,
    remote_writes_started: bool,
) -> PendingEnableDiscardFixture {
    let cspp = cove_cspp::Cspp::new(Keychain::global().clone());
    let cloud_keychain = CloudBackupKeychain::global();
    let prior_master_key = cove_cspp::master_key::MasterKey::generate();
    let prior_master_key_bytes = *prior_master_key.as_bytes();
    cspp.save_master_key(&prior_master_key).unwrap();
    cloud_keychain
        .save_passkey_and_namespace(&[4, 5, 6], [8; 32], &prior_master_key.namespace_id())
        .unwrap();
    let previous_metadata = cloud_keychain.snapshot_passkey_metadata();

    let staged_master_key = cove_cspp::master_key::MasterKey::generate();
    let staged_master_key_bytes = *staged_master_key.as_bytes();
    let namespace = staged_master_key.namespace_id();
    cspp.save_staged_master_key(&staged_master_key).unwrap();
    let passkey = UnpersistedPrfKey {
        prf_key: [7; 32],
        prf_salt: [9; 32],
        credential_id: vec![1, 2, 3],
        provider_hint: None,
    };
    let mut journal = staged_pending_enable_journal(
        CloudBackupEnableContext::settings_manual(),
        namespace.clone(),
        ownership,
        previous_metadata.clone(),
    );
    if remote_writes_started {
        assert!(journal.register_passkey(PendingEnablePasskeyMetadata {
            credential_id: passkey.credential_id.clone(),
            prf_salt: passkey.prf_salt,
            provider_hint: passkey.provider_hint.clone(),
        }));
        assert!(journal.mark_remote_writes_started());
    }
    cloud_keychain.save_pending_enable_journal(&journal).unwrap();

    PendingEnableDiscardFixture {
        prior_master_key: prior_master_key_bytes,
        staged_master_key: staged_master_key_bytes,
        namespace,
        passkey,
        previous_metadata,
    }
}

async fn install_pending_enable_discard_session(
    manager: &RustCloudBackupManager,
    fixture: &PendingEnableDiscardFixture,
) {
    replace_pending_enable_session_for_test(
        manager,
        PendingEnableSession::retry_upload(
            cove_cspp::master_key::MasterKey::from_bytes(fixture.staged_master_key),
            fixture.passkey.copy_for_retry(),
            CloudBackupEnableContext::settings_manual(),
        ),
    )
    .await;
}

fn assert_active_master_key(expected: &[u8; 32]) {
    let cspp = cove_cspp::Cspp::new(Keychain::global().clone());
    assert_eq!(cspp.load_master_key_from_store().unwrap().unwrap().as_bytes(), expected);
}

#[tokio::test(flavor = "current_thread")]
async fn discard_pending_enable_staged_preserves_prior_master_and_metadata() {
    let _guard = async_test_lock().lock().await;
    cove_tokio::init();
    let globals = test_globals();
    let manager = init_manager();

    reset_cloud_backup_test_state(&manager, globals);
    let fixture =
        prepare_pending_enable_discard_fixture(PendingEnableNamespaceOwnership::FreshOwned, false);
    install_pending_enable_discard_session(&manager, &fixture).await;
    let cspp = cove_cspp::Cspp::new(Keychain::global().clone());

    manager.discard_pending_enable_cloud_backup();

    assert!(take_pending_enable_session_for_test(&manager).await.is_none());
    assert_active_master_key(&fixture.prior_master_key);
    assert!(cspp.load_staged_master_key().unwrap().is_none());
    assert_eq!(
        CloudBackupKeychain::global().snapshot_passkey_metadata(),
        fixture.previous_metadata
    );
    assert!(CloudBackupKeychain::global().load_pending_enable_journal().unwrap().is_none());
    assert_eq!(globals.cloud.delete_namespace_attempt_count(), 0);
}

#[tokio::test(flavor = "current_thread")]
async fn discard_pending_enable_fresh_owned_remote_writes_delete_entire_namespace() {
    let _guard = async_test_lock().lock().await;
    cove_tokio::init();
    let globals = test_globals();
    let manager = init_manager();

    reset_cloud_backup_test_state(&manager, globals);
    let fixture =
        prepare_pending_enable_discard_fixture(PendingEnableNamespaceOwnership::FreshOwned, true);
    globals.cloud.set_master_key_backup(fixture.namespace.clone(), vec![1, 2, 3]);
    globals.cloud.set_wallet_backup(
        fixture.namespace.clone(),
        "wallet-record".into(),
        vec![4, 5, 6],
    );
    install_pending_enable_discard_session(&manager, &fixture).await;
    let cspp = cove_cspp::Cspp::new(Keychain::global().clone());

    manager.discard_pending_enable_cloud_backup();

    assert!(take_pending_enable_session_for_test(&manager).await.is_none());
    assert!(!globals.cloud.has_namespace(&fixture.namespace));
    assert_eq!(globals.cloud.delete_namespace_attempt_count(), 1);
    assert_active_master_key(&fixture.prior_master_key);
    assert!(cspp.load_staged_master_key().unwrap().is_none());
    assert_eq!(
        CloudBackupKeychain::global().snapshot_passkey_metadata(),
        fixture.previous_metadata
    );
    assert!(CloudBackupKeychain::global().load_pending_enable_journal().unwrap().is_none());
}

#[tokio::test(flavor = "current_thread")]
async fn discard_pending_enable_fresh_owned_treats_missing_namespace_as_deleted() {
    let _guard = async_test_lock().lock().await;
    cove_tokio::init();
    let globals = test_globals();
    let manager = init_manager();

    reset_cloud_backup_test_state(&manager, globals);
    let fixture =
        prepare_pending_enable_discard_fixture(PendingEnableNamespaceOwnership::FreshOwned, true);
    globals.cloud.fail_delete_namespace_not_found("already deleted");
    install_pending_enable_discard_session(&manager, &fixture).await;

    manager.discard_pending_enable_cloud_backup();

    assert!(take_pending_enable_session_for_test(&manager).await.is_none());
    assert_eq!(globals.cloud.delete_namespace_attempt_count(), 1);
    assert_active_master_key(&fixture.prior_master_key);
    assert!(CloudBackupKeychain::global().load_pending_enable_journal().unwrap().is_none());
}

#[tokio::test(flavor = "current_thread")]
async fn discard_pending_enable_remote_cleanup_failure_retains_session_and_journal() {
    let _guard = async_test_lock().lock().await;
    cove_tokio::init();
    let globals = test_globals();
    let manager = init_manager();

    reset_cloud_backup_test_state(&manager, globals);
    let fixture =
        prepare_pending_enable_discard_fixture(PendingEnableNamespaceOwnership::FreshOwned, true);
    globals.cloud.set_master_key_backup(fixture.namespace.clone(), vec![1, 2, 3]);
    globals.cloud.fail_delete_namespace("delete failed");
    install_pending_enable_discard_session(&manager, &fixture).await;
    let cspp = cove_cspp::Cspp::new(Keychain::global().clone());

    manager.discard_pending_enable_cloud_backup();

    wait_for_test_condition(
        Duration::from_secs(1),
        "discard pending enable cleanup failure should surface",
        || matches!(manager.state().lifecycle, CloudBackupLifecycle::Failed(_)),
    )
    .await;

    let CloudBackupLifecycle::Failed(failure) = manager.state().lifecycle else {
        panic!("expected failed lifecycle");
    };
    assert_eq!(failure.message, GENERIC_CLOUD_BACKUP_ERROR_MESSAGE);

    assert!(globals.cloud.has_namespace(&fixture.namespace));
    assert_active_master_key(&fixture.prior_master_key);
    assert_eq!(
        cspp.load_staged_master_key().unwrap().unwrap().as_bytes(),
        &fixture.staged_master_key
    );
    assert!(CloudBackupKeychain::global().load_pending_enable_journal().unwrap().is_some());
    assert!(take_pending_enable_session_for_test(&manager).await.is_some());
}

#[tokio::test(flavor = "current_thread")]
async fn discard_pending_enable_recovered_existing_never_deletes_remote_namespace() {
    let _guard = async_test_lock().lock().await;
    cove_tokio::init();
    let globals = test_globals();
    let manager = init_manager();

    reset_cloud_backup_test_state(&manager, globals);
    let fixture = prepare_pending_enable_discard_fixture(
        PendingEnableNamespaceOwnership::RecoveredExisting,
        true,
    );
    globals.cloud.set_master_key_backup(fixture.namespace.clone(), vec![1, 2, 3]);
    install_pending_enable_discard_session(&manager, &fixture).await;
    let cspp = cove_cspp::Cspp::new(Keychain::global().clone());

    manager.discard_pending_enable_cloud_backup();

    assert!(take_pending_enable_session_for_test(&manager).await.is_none());
    assert!(globals.cloud.has_namespace(&fixture.namespace));
    assert_eq!(globals.cloud.delete_namespace_attempt_count(), 0);
    assert_active_master_key(&fixture.prior_master_key);
    assert!(cspp.load_staged_master_key().unwrap().is_none());
    assert_eq!(
        CloudBackupKeychain::global().snapshot_passkey_metadata(),
        fixture.previous_metadata
    );
    assert!(CloudBackupKeychain::global().load_pending_enable_journal().unwrap().is_none());
}

#[tokio::test(flavor = "current_thread")]
async fn discard_pending_enable_unjournaled_session_never_deletes_or_clears_material() {
    let _guard = async_test_lock().lock().await;
    cove_tokio::init();
    let globals = test_globals();
    let manager = init_manager();

    reset_cloud_backup_test_state(&manager, globals);
    let fixture =
        prepare_pending_enable_discard_fixture(PendingEnableNamespaceOwnership::FreshOwned, true);
    CloudBackupKeychain::global().delete_pending_enable_journal().unwrap();
    globals.cloud.set_master_key_backup(fixture.namespace.clone(), vec![1, 2, 3]);
    install_pending_enable_discard_session(&manager, &fixture).await;
    let cspp = cove_cspp::Cspp::new(Keychain::global().clone());

    manager.discard_pending_enable_cloud_backup();

    assert!(take_pending_enable_session_for_test(&manager).await.is_none());
    assert!(globals.cloud.has_namespace(&fixture.namespace));
    assert_eq!(globals.cloud.delete_namespace_attempt_count(), 0);
    assert_active_master_key(&fixture.prior_master_key);
    assert_eq!(
        cspp.load_staged_master_key().unwrap().unwrap().as_bytes(),
        &fixture.staged_master_key
    );
    assert_eq!(
        CloudBackupKeychain::global().snapshot_passkey_metadata(),
        fixture.previous_metadata
    );
}

#[tokio::test(flavor = "current_thread")]
async fn discard_pending_enable_started_promotion_rolls_back_prior_state() {
    let _guard = async_test_lock().lock().await;
    cove_tokio::init();
    let globals = test_globals();
    let manager = init_manager();

    reset_cloud_backup_test_state(&manager, globals);
    let fixture =
        prepare_pending_enable_discard_fixture(PendingEnableNamespaceOwnership::FreshOwned, true);
    manager
        .pending_enable
        .begin_pending_enable_local_promotion(
            &cove_cspp::master_key::MasterKey::from_bytes(fixture.staged_master_key),
            &fixture.passkey,
        )
        .unwrap();
    globals.cloud.set_master_key_backup(fixture.namespace.clone(), vec![1, 2, 3]);
    install_pending_enable_discard_session(&manager, &fixture).await;
    let cspp = cove_cspp::Cspp::new(Keychain::global().clone());

    manager.discard_pending_enable_cloud_backup();

    assert!(take_pending_enable_session_for_test(&manager).await.is_none());
    assert!(!globals.cloud.has_namespace(&fixture.namespace));
    assert_active_master_key(&fixture.prior_master_key);
    assert_eq!(
        cspp.master_key_promotion_status().unwrap(),
        cove_cspp::MasterKeyPromotionStatus::None
    );
    assert_eq!(
        CloudBackupKeychain::global().snapshot_passkey_metadata(),
        fixture.previous_metadata
    );
    assert!(CloudBackupKeychain::global().load_pending_enable_journal().unwrap().is_none());
}

#[tokio::test(flavor = "current_thread")]
async fn discard_pending_enable_durable_completion_commits_without_remote_delete() {
    let _guard = async_test_lock().lock().await;
    cove_tokio::init();
    let globals = test_globals();
    let manager = init_manager();

    reset_cloud_backup_test_state(&manager, globals);
    let fixture =
        prepare_pending_enable_discard_fixture(PendingEnableNamespaceOwnership::FreshOwned, true);
    manager
        .pending_enable
        .begin_pending_enable_local_promotion(
            &cove_cspp::master_key::MasterKey::from_bytes(fixture.staged_master_key),
            &fixture.passkey,
        )
        .unwrap();
    let mut persisted_state = persisted_enabled_cloud_backup_state(Some(0));
    assert!(persisted_state.replace_pending_verification_completion(
        PendingVerificationCompletion::new(
            DeepVerificationReport {
                master_key_wrapper_repaired: false,
                local_master_key_repaired: false,
                credential_recovered: false,
                wallets_verified: 0,
                wallets_failed: 0,
                wallets_unsupported: 0,
                detail: None,
            },
            fixture.namespace.clone(),
            vec![PendingVerificationUpload::master_key_wrapper()],
        )
    ));
    Database::global().cloud_backup_state.set(&persisted_state).unwrap();
    globals.cloud.set_master_key_backup(fixture.namespace.clone(), vec![1, 2, 3]);
    install_pending_enable_discard_session(&manager, &fixture).await;
    let cspp = cove_cspp::Cspp::new(Keychain::global().clone());

    manager.discard_pending_enable_cloud_backup();

    assert!(take_pending_enable_session_for_test(&manager).await.is_none());
    assert!(globals.cloud.has_namespace(&fixture.namespace));
    assert_eq!(globals.cloud.delete_namespace_attempt_count(), 0);
    assert_active_master_key(&fixture.staged_master_key);
    assert_eq!(
        cspp.master_key_promotion_status().unwrap(),
        cove_cspp::MasterKeyPromotionStatus::None
    );
    assert_eq!(
        CloudBackupKeychain::global().namespace_id().as_deref(),
        Some(fixture.namespace.as_str())
    );
    assert!(CloudBackupKeychain::global().load_pending_enable_journal().unwrap().is_none());
}

#[tokio::test(flavor = "current_thread")]
async fn discard_pending_enable_local_cleanup_failure_retains_session_and_journal() {
    let _guard = async_test_lock().lock().await;
    cove_tokio::init();
    let globals = test_globals();
    let manager = init_manager();

    reset_cloud_backup_test_state(&manager, globals);
    let fixture =
        prepare_pending_enable_discard_fixture(PendingEnableNamespaceOwnership::FreshOwned, false);
    install_pending_enable_discard_session(&manager, &fixture).await;
    globals.keychain.fail_delete_at(1);

    manager.discard_pending_enable_cloud_backup();

    wait_for_test_condition(
        Duration::from_secs(1),
        "discard pending enable local cleanup failure should surface",
        || matches!(manager.state().lifecycle, CloudBackupLifecycle::Failed(_)),
    )
    .await;

    let CloudBackupLifecycle::Failed(failure) = manager.state().lifecycle else {
        panic!("expected failed lifecycle");
    };
    assert_eq!(failure.message, GENERIC_CLOUD_BACKUP_ERROR_MESSAGE);

    assert_active_master_key(&fixture.prior_master_key);
    assert!(CloudBackupKeychain::global().load_pending_enable_journal().unwrap().is_some());
    assert!(take_pending_enable_session_for_test(&manager).await.is_some());
}
