use super::*;
use crate::manager::cloud_backup_manager::admission::ContinuationOwner;

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn registered_passkey_stages_confirmation_without_automatic_auth() {
    let _guard = async_test_lock().lock().await;
    let globals = test_globals();
    let manager = init_manager();

    reset_cloud_backup_test_state(&manager, globals);
    CONNECTIVITY_MANAGER.set_connection_state(true);
    globals.passkey.set_create_result(Ok(vec![1, 2, 3]));
    globals.passkey.set_authenticate_result(Ok(vec![7; 32]));

    enable_cloud_backup_no_discovery(&manager).await.unwrap();

    assert_eq!(globals.passkey.create_count(), 1);
    assert_eq!(globals.passkey.authenticate_count(), 0);
    assert_eq!(CloudBackupKeychain::global().load_credential_id().unwrap(), None);
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
    let (_, pending_passkey) = pending.into_staged_parts();
    assert_eq!(pending_passkey.credential_id, vec![1, 2, 3]);
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn normal_enable_preserves_registered_passkey_confirmation_session() {
    let _guard = async_test_lock().lock().await;
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
    let (_, pending_passkey) = pending.into_staged_parts();
    assert_eq!(pending_passkey.credential_id, vec![1, 2, 3]);
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn reinitialize_preserves_registered_passkey_confirmation_session() {
    let _guard = async_test_lock().lock().await;
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
    let (_, pending_passkey) = pending.into_staged_parts();
    assert_eq!(pending_passkey.credential_id, vec![1, 2, 3]);
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn force_new_preserves_registered_passkey_confirmation_session() {
    let _guard = async_test_lock().lock().await;
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
    let (_, pending_passkey) = pending.into_staged_parts();
    assert_eq!(pending_passkey.credential_id, vec![1, 2, 3]);
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn registered_passkey_stages_confirmation_without_duplicate_create() {
    let _guard = async_test_lock().lock().await;
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
    let (_, pending_passkey) = pending.into_staged_parts();
    assert_eq!(pending_passkey.credential_id, vec![1, 2, 3]);
}

#[tokio::test(flavor = "current_thread")]
async fn confirm_saved_passkey_reuses_original_credential_id() {
    let _guard = async_test_lock().lock().await;
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
        CloudBackupKeychain::global().snapshot_passkey_metadata().unwrap(),
    );
    assert!(journal.register_passkey(PendingEnablePasskeyMetadata {
        credential_id: vec![1, 2, 3],
        prf_salt: [9; 32],
        provider_hint: None,
    }));
    CloudBackupKeychain::global().save_pending_enable_journal(&journal).unwrap();
    replace_enable_continuation_for_test(
        &manager,
        ContinuationOwner::Enable,
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
    let globals = test_globals();
    let manager = init_manager();

    reset_cloud_backup_test_state(&manager, globals);
    CONNECTIVITY_MANAGER.set_connection_state(true);
    globals.passkey.set_create_result(Ok(vec![1, 2, 3]));

    enable_cloud_backup_no_discovery(&manager).await.unwrap();

    globals.passkey.set_authenticate_result(Err(PasskeyError::NotCompleted));
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
    let globals = test_globals();
    let manager = init_manager();

    reset_cloud_backup_test_state(&manager, globals);
    CONNECTIVITY_MANAGER.set_connection_state(true);
    let master_key = cove_cspp::master_key::MasterKey::generate();
    replace_enable_continuation_for_test(
        &manager,
        ContinuationOwner::Enable,
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
    let previous_metadata = cloud_keychain.snapshot_passkey_metadata().unwrap();

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
    replace_enable_continuation_for_test(manager, ContinuationOwner::Enable, fixture.session())
        .await;
}

impl PendingEnableDiscardFixture {
    /// The registered passkey's session, as it waits for the user's confirmation
    fn session(&self) -> PendingEnableSession {
        PendingEnableSession::awaiting_saved_passkey_confirmation(
            zeroize::Zeroizing::new(cove_cspp::master_key::MasterKey::from_bytes(
                self.staged_master_key,
            )),
            zeroize::Zeroizing::new(StagedPrfKey {
                prf_salt: self.passkey.prf_salt,
                credential_id: self.passkey.credential_id.clone(),
                provider_hint: self.passkey.provider_hint.clone(),
            }),
            CloudBackupEnableContext::settings_manual(),
        )
    }
}

fn assert_active_master_key(expected: &[u8; 32]) {
    let cspp = cove_cspp::Cspp::new(Keychain::global().clone());
    assert_eq!(cspp.load_master_key_from_store().unwrap().unwrap().as_bytes(), expected);
}

#[tokio::test(flavor = "current_thread")]
async fn discard_pending_enable_staged_preserves_prior_master_and_metadata() {
    let _guard = async_test_lock().lock().await;
    let globals = test_globals();
    let manager = init_manager();

    reset_cloud_backup_test_state(&manager, globals);
    let fixture =
        prepare_pending_enable_discard_fixture(PendingEnableNamespaceOwnership::FreshOwned, false);
    install_pending_enable_discard_session(&manager, &fixture).await;
    let cspp = cove_cspp::Cspp::new(Keychain::global().clone());

    discard_held_continuation_for_test(&manager).await;

    assert!(take_pending_enable_session_for_test(&manager).await.is_none());
    assert_active_master_key(&fixture.prior_master_key);
    assert!(cspp.load_staged_master_key().unwrap().is_none());
    assert_eq!(
        CloudBackupKeychain::global().snapshot_passkey_metadata().unwrap(),
        fixture.previous_metadata
    );
    assert!(CloudBackupKeychain::global().load_pending_enable_journal().unwrap().is_none());
    assert_eq!(globals.cloud.delete_namespace_attempt_count(), 0);
}

#[tokio::test(flavor = "current_thread")]
async fn discard_pending_enable_fresh_owned_remote_writes_delete_entire_namespace() {
    let _guard = async_test_lock().lock().await;
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

    discard_pending_enable_and_wait_for_remote_delete(&manager).await;

    assert!(take_pending_enable_session_for_test(&manager).await.is_none());
    assert!(!globals.cloud.has_namespace(&fixture.namespace));
    assert_eq!(globals.cloud.delete_namespace_attempt_count(), 1);
    assert_active_master_key(&fixture.prior_master_key);
    assert!(cspp.load_staged_master_key().unwrap().is_none());
    assert_eq!(
        CloudBackupKeychain::global().snapshot_passkey_metadata().unwrap(),
        fixture.previous_metadata
    );
    assert!(CloudBackupKeychain::global().load_pending_enable_journal().unwrap().is_none());
}

/// Discarding a stopped reinitialize deletes the namespace it wrote, while the configured
/// backup stays on the namespace it already had
#[tokio::test(flavor = "current_thread")]
async fn discard_stopped_reinitialize_deletes_only_its_namespace() {
    let _guard = async_test_lock().lock().await;
    let globals = test_globals();
    let manager = init_manager();

    reset_cloud_backup_test_state(&manager, globals);
    let fixture =
        prepare_pending_enable_discard_fixture(PendingEnableNamespaceOwnership::FreshOwned, true);
    let active_namespace =
        cove_cspp::master_key::MasterKey::from_bytes(fixture.prior_master_key).namespace_id();
    Database::global()
        .cloud_backup_state
        .set(&persisted_enabled_cloud_backup_state(Some(0)))
        .unwrap();
    globals.cloud.set_master_key_backup(active_namespace.clone(), vec![1]);
    globals.cloud.set_master_key_backup(fixture.namespace.clone(), vec![1, 2, 3]);
    replace_enable_continuation_for_test(
        &manager,
        ContinuationOwner::Reinitialize,
        fixture.session(),
    )
    .await;

    discard_pending_enable_and_wait_for_remote_delete(&manager).await;

    assert!(take_pending_enable_session_for_test(&manager).await.is_none());
    assert!(!globals.cloud.has_namespace(&fixture.namespace));
    assert!(globals.cloud.has_namespace(&active_namespace));
    assert_eq!(globals.cloud.delete_namespace_attempt_count(), 1);
    assert_active_master_key(&fixture.prior_master_key);
    assert_eq!(
        CloudBackupKeychain::global().snapshot_passkey_metadata().unwrap(),
        fixture.previous_metadata
    );
    assert!(matches!(
        Database::global().cloud_backup_state.get().unwrap(),
        PersistedCloudBackupState::Configured(_)
    ));
    assert_eq!(manager.current_status(), CloudBackupStatus::Enabled);
}

/// The last dispatched intent's admission, read on the supervisor
async fn last_admission(
    manager: &RustCloudBackupManager,
) -> crate::manager::cloud_backup_manager::admission::Admission {
    call!(manager.supervisor.harness_last_outcome())
        .await
        .expect("supervisor should answer its last outcome")
        .expect("an intent should have been dispatched")
        .admission
}

#[tokio::test(flavor = "current_thread")]
async fn enable_during_a_pending_enable_discard_is_rejected_and_the_discard_finishes_alone() {
    use crate::manager::cloud_backup_manager::admission::{
        Admission, CoalescibleIntent, Rejection,
    };

    let _guard = async_test_lock().lock().await;
    let globals = test_globals();
    let manager = init_manager();

    reset_cloud_backup_test_state(&manager, globals);
    let fixture =
        prepare_pending_enable_discard_fixture(PendingEnableNamespaceOwnership::FreshOwned, true);
    globals.cloud.set_master_key_backup(fixture.namespace.clone(), vec![1, 2, 3]);
    install_pending_enable_discard_session(&manager, &fixture).await;
    let delete = globals.cloud.gate_next_delete_namespace();

    manager.dispatch(CloudBackupManagerAction::DiscardPendingEnableCloudBackup);
    tokio::time::timeout(Duration::from_secs(10), delete.wait_until_blocked())
        .await
        .expect("checkpoint: the discard's remote delete should reach the cloud");

    // the discard is the running operation, so Enable meets it and is rejected as busy
    dispatch_and_wait(
        &manager,
        CloudBackupManagerAction::EnableCloudBackup(CloudBackupEnableContext::settings_manual()),
    )
    .await;
    assert_eq!(
        last_admission(&manager).await,
        Admission::Rejected(Rejection::OperationRunning(
            CloudBackupExclusiveOperation::DiscardPendingEnable
        ))
    );

    // a second Discard is the same request, not a second run that settles the state early
    dispatch_and_wait(&manager, CloudBackupManagerAction::DiscardPendingEnableCloudBackup).await;
    assert_eq!(
        last_admission(&manager).await,
        Admission::Coalesced(CoalescibleIntent::RepeatOfRunning(
            CloudBackupExclusiveOperation::DiscardPendingEnable
        ))
    );
    assert_eq!(manager.model_state().lifecycle, CloudBackupLifecycle::Disabled);

    delete.release();
    wait_for_test_condition(
        Duration::from_secs(10),
        "checkpoint: the discard should clear the journal after its delete",
        || CloudBackupKeychain::global().load_pending_enable_journal().unwrap().is_none(),
    )
    .await;
    supervisor_checkpoint(&manager).await;

    // the completion was current, so it rolled back the discarded Enable's state exactly once
    // and nothing else had started that it could roll back over
    assert!(!globals.cloud.has_namespace(&fixture.namespace));
    assert_eq!(globals.cloud.delete_namespace_attempt_count(), 1);
    assert_active_master_key(&fixture.prior_master_key);
    assert!(take_pending_enable_session_for_test(&manager).await.is_none());
    assert_eq!(manager.projected_exclusive_operation(), None);

    // the slot is free again: a new Enable is admitted
    dispatch_and_wait(
        &manager,
        CloudBackupManagerAction::EnableCloudBackup(CloudBackupEnableContext::settings_manual()),
    )
    .await;
    assert_eq!(last_admission(&manager).await, Admission::Accepted);
}

#[tokio::test(flavor = "current_thread")]
async fn discard_pending_enable_fresh_owned_treats_missing_namespace_as_deleted() {
    let _guard = async_test_lock().lock().await;
    let globals = test_globals();
    let manager = init_manager();

    reset_cloud_backup_test_state(&manager, globals);
    let fixture =
        prepare_pending_enable_discard_fixture(PendingEnableNamespaceOwnership::FreshOwned, true);
    globals.cloud.fail_delete_namespace_not_found("already deleted");
    install_pending_enable_discard_session(&manager, &fixture).await;

    discard_pending_enable_and_wait_for_remote_delete(&manager).await;

    assert!(take_pending_enable_session_for_test(&manager).await.is_none());
    assert_eq!(globals.cloud.delete_namespace_attempt_count(), 1);
    assert_active_master_key(&fixture.prior_master_key);
    assert!(CloudBackupKeychain::global().load_pending_enable_journal().unwrap().is_none());
}

#[tokio::test(flavor = "current_thread")]
async fn discard_pending_enable_remote_cleanup_failure_retains_session_and_journal() {
    let _guard = async_test_lock().lock().await;
    let globals = test_globals();
    let manager = init_manager();

    reset_cloud_backup_test_state(&manager, globals);
    let fixture =
        prepare_pending_enable_discard_fixture(PendingEnableNamespaceOwnership::FreshOwned, true);
    globals.cloud.set_master_key_backup(fixture.namespace.clone(), vec![1, 2, 3]);
    globals.cloud.fail_delete_namespace("delete failed");
    install_pending_enable_discard_session(&manager, &fixture).await;
    let cspp = cove_cspp::Cspp::new(Keychain::global().clone());

    manager.dispatch(CloudBackupManagerAction::DiscardPendingEnableCloudBackup);

    wait_for_test_condition(
        Duration::from_secs(1),
        "discard pending enable cleanup failure should surface",
        || matches!(manager.model_state().lifecycle, CloudBackupLifecycle::Failed(_)),
    )
    .await;

    let CloudBackupLifecycle::Failed(failure) = manager.model_state().lifecycle else {
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

    discard_held_continuation_for_test(&manager).await;

    assert!(take_pending_enable_session_for_test(&manager).await.is_none());
    assert!(globals.cloud.has_namespace(&fixture.namespace));
    assert_eq!(globals.cloud.delete_namespace_attempt_count(), 0);
    assert_active_master_key(&fixture.prior_master_key);
    assert!(cspp.load_staged_master_key().unwrap().is_none());
    assert_eq!(
        CloudBackupKeychain::global().snapshot_passkey_metadata().unwrap(),
        fixture.previous_metadata
    );
    assert!(CloudBackupKeychain::global().load_pending_enable_journal().unwrap().is_none());
}

#[tokio::test(flavor = "current_thread")]
async fn discard_pending_enable_unjournaled_session_never_deletes_or_clears_material() {
    let _guard = async_test_lock().lock().await;
    let globals = test_globals();
    let manager = init_manager();

    reset_cloud_backup_test_state(&manager, globals);
    let fixture =
        prepare_pending_enable_discard_fixture(PendingEnableNamespaceOwnership::FreshOwned, true);
    CloudBackupKeychain::global().delete_pending_enable_journal().unwrap();
    globals.cloud.set_master_key_backup(fixture.namespace.clone(), vec![1, 2, 3]);
    install_pending_enable_discard_session(&manager, &fixture).await;
    let cspp = cove_cspp::Cspp::new(Keychain::global().clone());

    discard_held_continuation_for_test(&manager).await;

    assert!(take_pending_enable_session_for_test(&manager).await.is_none());
    assert!(globals.cloud.has_namespace(&fixture.namespace));
    assert_eq!(globals.cloud.delete_namespace_attempt_count(), 0);
    assert_active_master_key(&fixture.prior_master_key);
    assert_eq!(
        cspp.load_staged_master_key().unwrap().unwrap().as_bytes(),
        &fixture.staged_master_key
    );
    assert_eq!(
        CloudBackupKeychain::global().snapshot_passkey_metadata().unwrap(),
        fixture.previous_metadata
    );
}

#[tokio::test(flavor = "current_thread")]
async fn discard_pending_enable_started_promotion_rolls_back_prior_state() {
    let _guard = async_test_lock().lock().await;
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

    discard_held_continuation_for_test(&manager).await;
    wait_for_test_condition(
        Duration::from_secs(1),
        "pending enable discard should finish its remote delete",
        || CloudBackupKeychain::global().load_pending_enable_journal().unwrap().is_none(),
    )
    .await;
    supervisor_checkpoint(&manager).await;

    assert!(take_pending_enable_session_for_test(&manager).await.is_none());
    assert!(!globals.cloud.has_namespace(&fixture.namespace));
    assert_active_master_key(&fixture.prior_master_key);
    assert_eq!(
        cspp.master_key_promotion_status().unwrap(),
        cove_cspp::MasterKeyPromotionStatus::None
    );
    assert_eq!(
        CloudBackupKeychain::global().snapshot_passkey_metadata().unwrap(),
        fixture.previous_metadata
    );
    assert!(CloudBackupKeychain::global().load_pending_enable_journal().unwrap().is_none());
}

/// A stopped reinitialize whose record already switched to the new namespace finishes on Discard
/// instead of deleting what the record now names
#[tokio::test(flavor = "current_thread")]
async fn discard_pending_enable_durable_completion_commits_without_remote_delete() {
    let _guard = async_test_lock().lock().await;
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
                wallet_issues: Default::default(),
                detail: None,
            },
            fixture.namespace.clone(),
            vec![PendingVerificationUpload::master_key_wrapper()],
        )
    ));
    Database::global().cloud_backup_state.set(&persisted_state).unwrap();
    globals.cloud.set_master_key_backup(fixture.namespace.clone(), vec![1, 2, 3]);

    // the record is configured, so only a reinitialize can own work stopped in it
    replace_enable_continuation_for_test(
        &manager,
        ContinuationOwner::Reinitialize,
        fixture.session(),
    )
    .await;
    let cspp = cove_cspp::Cspp::new(Keychain::global().clone());

    discard_held_continuation_for_test(&manager).await;

    assert!(take_pending_enable_session_for_test(&manager).await.is_none());
    assert!(globals.cloud.has_namespace(&fixture.namespace));
    assert_eq!(globals.cloud.delete_namespace_attempt_count(), 0);
    assert_active_master_key(&fixture.staged_master_key);
    assert_eq!(
        cspp.master_key_promotion_status().unwrap(),
        cove_cspp::MasterKeyPromotionStatus::None
    );
    assert_eq!(
        CloudBackupKeychain::global().namespace_id().unwrap().as_deref(),
        Some(fixture.namespace.as_str())
    );
    assert!(CloudBackupKeychain::global().load_pending_enable_journal().unwrap().is_none());
}

#[tokio::test(flavor = "current_thread")]
async fn committed_pending_enable_journal_cleanup_retries_without_restart() {
    let _guard = async_test_lock().lock().await;
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
    let cspp = cove_cspp::Cspp::new(Keychain::global().clone());
    cspp.commit_master_key_promotion().unwrap();

    assert_eq!(
        cspp.master_key_promotion_status().unwrap(),
        cove_cspp::MasterKeyPromotionStatus::None
    );
    assert!(CloudBackupKeychain::global().load_pending_enable_journal().unwrap().is_some());

    manager.pending_enable.commit_pending_enable_local_promotion().unwrap();

    assert_active_master_key(&fixture.staged_master_key);
    assert!(CloudBackupKeychain::global().load_pending_enable_journal().unwrap().is_none());
}

#[tokio::test(flavor = "current_thread")]
async fn discard_pending_enable_local_cleanup_failure_retains_session_and_journal() {
    let _guard = async_test_lock().lock().await;
    let globals = test_globals();
    let manager = init_manager();

    reset_cloud_backup_test_state(&manager, globals);
    let fixture =
        prepare_pending_enable_discard_fixture(PendingEnableNamespaceOwnership::FreshOwned, false);
    install_pending_enable_discard_session(&manager, &fixture).await;
    globals.keychain.fail_delete_at(1);

    discard_held_continuation_for_test(&manager).await;

    wait_for_test_condition(
        Duration::from_secs(1),
        "discard pending enable local cleanup failure should surface",
        || matches!(manager.model_state().lifecycle, CloudBackupLifecycle::Failed(_)),
    )
    .await;

    let CloudBackupLifecycle::Failed(failure) = manager.model_state().lifecycle else {
        panic!("expected failed lifecycle");
    };
    assert_eq!(failure.message, GENERIC_CLOUD_BACKUP_ERROR_MESSAGE);

    assert_active_master_key(&fixture.prior_master_key);
    assert!(CloudBackupKeychain::global().load_pending_enable_journal().unwrap().is_some());
    assert!(take_pending_enable_session_for_test(&manager).await.is_some());
}
