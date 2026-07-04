use super::*;

#[tokio::test(flavor = "current_thread")]
async fn enable_recovery_rolls_back_local_master_key_when_wallet_upload_fails() {
    let _guard = async_test_lock().lock().await;
    cove_tokio::init();
    let globals = test_globals();
    let manager = init_manager();

    reset_cloud_backup_test_state(&manager, globals);
    persist_xpub_wallets(vec![xpub_only_wallet_metadata()]);
    globals.cloud.fail_wallet_backup_upload("upload failed");

    let cspp = cove_cspp::Cspp::new(Keychain::global().clone());
    let master_key = cove_cspp::master_key::MasterKey::generate();
    let matched = NamespaceMatch {
        namespace_id: master_key.namespace_id(),
        master_key,
        prf_salt: [9; 32],
        credential_id: vec![1, 2, 3],
    };

    let preparation = manager
        .prepare_enable_recovery(CloudBackupEnableContext::settings_manual(), vec![matched])
        .await
        .unwrap();
    manager.save_enable_recovery_master_key(&preparation).unwrap();
    let claim = CloudBackupExclusiveOperationClaim::new(CloudBackupExclusiveOperation::Enable, 42);
    manager.project_exclusive_operation_started(claim);
    let writes = operation_write_client_for_test(&manager, claim);
    let error = manager.prepare_enable_recovery_completion(preparation, writes).await.unwrap_err();
    manager.project_exclusive_operation_finished(claim);
    manager.rollback_enable_recovery_master_key();

    assert!(matches!(error, CloudBackupError::CloudStorage(_)));
    assert!(cspp.load_master_key_from_store().unwrap().is_none());
}

#[tokio::test(flavor = "current_thread")]
async fn enable_recovery_rolls_back_local_master_key_when_keychain_save_fails() {
    let _guard = async_test_lock().lock().await;
    cove_tokio::init();
    let globals = test_globals();
    let manager = init_manager();

    reset_cloud_backup_test_state(&manager, globals);
    globals.keychain.fail_save_at(3);

    let cspp = cove_cspp::Cspp::new(Keychain::global().clone());
    let master_key = cove_cspp::master_key::MasterKey::generate();
    let matched = NamespaceMatch {
        namespace_id: master_key.namespace_id(),
        master_key,
        prf_salt: [9; 32],
        credential_id: vec![1, 2, 3],
    };

    let preparation = manager
        .prepare_enable_recovery(CloudBackupEnableContext::settings_manual(), vec![matched])
        .await
        .unwrap();
    manager.save_enable_recovery_master_key(&preparation).unwrap();
    let claim = CloudBackupExclusiveOperationClaim::new(CloudBackupExclusiveOperation::Enable, 42);
    manager.project_exclusive_operation_started(claim);
    let writes = operation_write_client_for_test(&manager, claim);
    let completion = manager.prepare_enable_recovery_completion(preparation, writes).await.unwrap();
    manager.project_exclusive_operation_finished(claim);
    let error = CloudBackupKeychain::global()
        .save_passkey_and_namespace(
            &completion.credential_id,
            completion.prf_salt,
            &completion.namespace_id,
        )
        .unwrap_err();
    manager.rollback_enable_recovery_master_key();

    assert!(matches!(error, CloudBackupKeychainError::Keychain(_)));
    assert!(cspp.load_master_key_from_store().unwrap().is_none());
}

#[tokio::test(flavor = "current_thread")]
async fn failed_create_new_enable_does_not_persist_passkey_metadata() {
    let _guard = async_test_lock().lock().await;
    cove_tokio::init();
    let globals = test_globals();
    globals.reset();
    globals.cloud.fail_master_key_upload("boom");
    globals.passkey.set_discover_result(Ok(DiscoveredPasskeyResult {
        prf_output: vec![7; 32],
        credential_id: vec![1, 2, 3],
    }));

    let manager = init_manager();
    let error = enable_cloud_backup_create_new(&manager).await.unwrap_err();
    assert!(matches!(
        error,
        CloudBackupError::Internal(message)
            if message.contains("upload master key backup") && message.contains("boom")
    ));

    let keychain = Keychain::global();
    assert!(keychain.get(CSPP_CREDENTIAL_ID_KEY.into()).is_none());
    assert!(keychain.get(CSPP_PRF_SALT_KEY.into()).is_none());
    assert!(keychain.get(CSPP_NAMESPACE_ID_KEY.into()).is_none());
}

#[tokio::test(flavor = "current_thread")]
async fn failed_no_discovery_enable_does_not_persist_passkey_metadata() {
    let _guard = async_test_lock().lock().await;
    cove_tokio::init();
    let globals = test_globals();
    globals.reset();
    globals.passkey.set_create_result(Ok(vec![1, 2, 3]));
    globals.passkey.set_authenticate_result(Err(PasskeyError::RequestFailed {
        operation: PasskeyOperation::AuthenticateAssertion,
        reason: PasskeyFailureReason::Unknown { diagnostic_message: "boom".into() },
    }));

    let manager = init_manager();
    enable_cloud_backup_no_discovery(&manager).await.unwrap();
    confirm_saved_passkey_session(&manager).await;

    let keychain = Keychain::global();
    let cspp = cove_cspp::Cspp::new(keychain.clone());
    assert!(cspp.load_master_key_from_store().unwrap().is_some());
    assert!(take_pending_enable_session_for_test(&manager).await.is_some());
    assert!(keychain.get(CSPP_CREDENTIAL_ID_KEY.into()).is_none());
    assert!(keychain.get(CSPP_PRF_SALT_KEY.into()).is_none());
    assert!(keychain.get(CSPP_NAMESPACE_ID_KEY.into()).is_none());
}

#[tokio::test(flavor = "current_thread")]
async fn enable_create_new_succeeds_with_new_passkey_auth() {
    let _guard = async_test_lock().lock().await;
    cove_tokio::init();
    let globals = test_globals();
    let manager = init_manager();

    reset_cloud_backup_test_state(&manager, globals);
    CONNECTIVITY_MANAGER.set_connection_state(true);
    globals.passkey.set_create_result(Ok(vec![1, 2, 3]));
    globals.passkey.set_authenticate_result(Ok(vec![7; 32]));

    enable_cloud_backup_create_new(&manager).await.unwrap();
    confirm_saved_passkey_session(&manager).await;

    assert_eq!(manager.current_status(), CloudBackupStatus::Enabled);
    let state = manager.model_snapshot();
    assert!(matches!(state.verification, VerificationState::Idle));
    assert_eq!(state.pending_upload_verification, PendingUploadVerificationState::Confirming);
    assert!(matches!(state.root_prompt, CloudBackupRootPrompt::None));
    assert!(globals.keychain.get_entry(CSPP_CREDENTIAL_ID_KEY).is_some());
    assert!(globals.keychain.get_entry(CSPP_PRF_SALT_KEY).is_some());
    assert!(globals.keychain.get_entry(CSPP_NAMESPACE_ID_KEY).is_some());

    let discover_count = globals.passkey.discover_count();
    let authenticate_count = globals.passkey.authenticate_count();

    call!(manager.supervisor.start_enter_detail()).await.unwrap();
    tokio::time::sleep(Duration::from_millis(50)).await;

    assert_eq!(globals.passkey.discover_count(), discover_count);
    assert_eq!(globals.passkey.authenticate_count(), authenticate_count);
}

#[tokio::test(flavor = "current_thread")]
async fn enable_create_new_recovers_from_corrupted_persisted_state() {
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
    CONNECTIVITY_MANAGER.set_connection_state(true);
    globals.passkey.set_create_result(Ok(vec![1, 2, 3]));
    globals.passkey.set_authenticate_result(Ok(vec![7; 32]));

    enable_cloud_backup_create_new(&manager).await.unwrap();
    confirm_saved_passkey_session(&manager).await;

    assert_eq!(manager.current_status(), CloudBackupStatus::Enabled);
    assert_eq!(
        Database::global().cloud_backup_state.get().unwrap().status(),
        PersistedCloudBackupStatus::Unverified
    );
}

#[tokio::test(flavor = "current_thread")]
async fn detail_entry_starts_discoverable_verification_without_runtime_authorization() {
    let _guard = async_test_lock().lock().await;
    cove_tokio::init();
    let globals = test_globals();
    let manager = init_manager();

    reset_cloud_backup_test_state(&manager, globals);
    CONNECTIVITY_MANAGER.set_connection_state(true);
    globals.passkey.set_create_result(Ok(vec![1, 2, 3]));
    globals.passkey.set_authenticate_result(Ok(vec![7; 32]));

    enable_cloud_backup_no_discovery(&manager).await.unwrap();
    confirm_saved_passkey_session(&manager).await;

    manager.clear_runtime_passkey_authorization();
    manager.clear_pending_verification_completion();
    manager.reconcile_pending_upload_verification(PendingUploadVerificationState::Idle);
    manager.apply_verification_outcome(CloudBackupVerificationOutcome::Idle);
    Database::global().cloud_blob_sync_states.delete_all().unwrap();
    globals.passkey.set_discover_result(Err(PasskeyError::UserCancelled));
    globals.cloud.fail_list_wallet_files("list should not run before passkey auth");

    let discover_count = globals.passkey.discover_count();
    let list_count = globals.cloud.list_wallet_files_attempt_count();

    call!(manager.supervisor.start_enter_detail()).await.unwrap();
    wait_for_discover_count(globals, discover_count + 1).await;

    assert_eq!(globals.passkey.discover_count(), discover_count + 1);
    assert_eq!(globals.cloud.list_wallet_files_attempt_count(), list_count);
}

#[tokio::test(flavor = "current_thread")]
async fn detail_entry_does_not_restart_rust_owned_verification_states() {
    let _guard = async_test_lock().lock().await;
    cove_tokio::init();
    let globals = test_globals();
    let manager = init_manager();

    let states = [
        VerificationState::Verifying,
        VerificationState::Verified(DeepVerificationReport {
            master_key_wrapper_repaired: false,
            local_master_key_repaired: false,
            credential_recovered: false,
            wallets_verified: 0,
            wallets_failed: 0,
            wallets_unsupported: 0,
            detail: None,
        }),
        VerificationState::PasskeyConfirmed,
    ];

    for verification in states {
        configure_enabled_cloud_backup(&manager, globals, 0);
        manager.clear_runtime_passkey_authorization();
        manager.clear_pending_verification_completion();
        manager.reconcile_pending_upload_verification(PendingUploadVerificationState::Idle);
        manager
            .apply_verification_outcome(CloudBackupVerificationOutcome::from_state(verification));
        globals.passkey.set_discover_result(Err(PasskeyError::UserCancelled));

        let discover_count = globals.passkey.discover_count();
        let authenticate_count = globals.passkey.authenticate_count();

        call!(manager.supervisor.start_enter_detail()).await.unwrap();
        tokio::time::sleep(Duration::from_millis(50)).await;

        assert_eq!(globals.passkey.discover_count(), discover_count);
        assert_eq!(globals.passkey.authenticate_count(), authenticate_count);
    }
}

#[tokio::test(flavor = "current_thread")]
async fn enable_no_discovery_succeeds_with_new_passkey_auth() {
    let _guard = async_test_lock().lock().await;
    cove_tokio::init();
    let globals = test_globals();
    let manager = init_manager();

    reset_cloud_backup_test_state(&manager, globals);
    CONNECTIVITY_MANAGER.set_connection_state(true);
    globals.passkey.set_create_result(Ok(vec![1, 2, 3]));
    globals.passkey.set_authenticate_result(Ok(vec![7; 32]));

    enable_cloud_backup_no_discovery(&manager).await.unwrap();
    confirm_saved_passkey_session(&manager).await;

    assert_eq!(manager.current_status(), CloudBackupStatus::Enabled);
}

#[tokio::test(flavor = "current_thread")]
async fn enable_with_multiple_matching_namespaces_merges_into_largest_namespace() {
    let _guard = async_test_lock().lock().await;
    cove_tokio::init();
    let globals = test_globals();
    let manager = init_manager();

    reset_cloud_backup_test_state(&manager, globals);
    CONNECTIVITY_MANAGER.set_connection_state(true);
    globals.cloud.set_reflect_uploaded_wallets_in_listing(true);

    let prf_key = [7u8; 32];
    let first_master_key = cove_cspp::master_key::MasterKey::generate();
    let second_master_key = cove_cspp::master_key::MasterKey::generate();
    let first_namespace = first_master_key.namespace_id();
    let second_namespace = second_master_key.namespace_id();
    let first_encrypted =
        cove_cspp::master_key_crypto::encrypt_master_key(&first_master_key, &prf_key, &[9; 32])
            .unwrap();
    let second_encrypted =
        cove_cspp::master_key_crypto::encrypt_master_key(&second_master_key, &prf_key, &[8; 32])
            .unwrap();

    globals.cloud.set_master_key_backup(
        first_namespace.clone(),
        serde_json::to_vec(&first_encrypted).unwrap(),
    );
    globals.cloud.set_master_key_backup(
        second_namespace.clone(),
        serde_json::to_vec(&second_encrypted).unwrap(),
    );
    globals.passkey.set_discover_result(Ok(DiscoveredPasskeyResult {
        prf_output: prf_key.to_vec(),
        credential_id: vec![1, 2, 3],
    }));
    globals.passkey.set_authenticate_result(Ok(prf_key.to_vec()));

    let first_wallet = xpub_only_wallet_metadata();
    let second_wallet = xpub_only_wallet_metadata();
    let third_wallet = xpub_only_wallet_metadata();
    let first_wallet = WalletMetadata { master_fingerprint: None, ..first_wallet };
    let second_wallet = WalletMetadata { master_fingerprint: None, ..second_wallet };
    let third_wallet = WalletMetadata { master_fingerprint: None, ..third_wallet };
    let sample_xpub_from_entropy = |metadata: &WalletMetadata, byte| {
        let entropy = [byte; 16];
        let mnemonic = Mnemonic::from_entropy(&entropy).unwrap();

        crate::mnemonic::MnemonicExt::xpub(&mnemonic, metadata.network.into()).to_string()
    };
    Keychain::global()
        .save_wallet_xpub(
            &first_wallet.id,
            sample_xpub_from_entropy(&first_wallet, 1).parse().unwrap(),
        )
        .unwrap();
    Keychain::global()
        .save_wallet_xpub(
            &second_wallet.id,
            sample_xpub_from_entropy(&second_wallet, 2).parse().unwrap(),
        )
        .unwrap();
    Keychain::global()
        .save_wallet_xpub(
            &third_wallet.id,
            sample_xpub_from_entropy(&third_wallet, 3).parse().unwrap(),
        )
        .unwrap();

    let first_record_id = cove_cspp::backup_data::wallet_record_id(first_wallet.id.as_ref());
    let second_record_id = cove_cspp::backup_data::wallet_record_id(second_wallet.id.as_ref());
    let third_record_id = cove_cspp::backup_data::wallet_record_id(third_wallet.id.as_ref());
    let first_revision = crate::manager::cloud_backup_manager::wallets::prepare_wallet_backup(
        &first_wallet,
        first_wallet.wallet_mode,
    )
    .await
    .unwrap()
    .revision_hash;
    let second_revision = crate::manager::cloud_backup_manager::wallets::prepare_wallet_backup(
        &second_wallet,
        second_wallet.wallet_mode,
    )
    .await
    .unwrap()
    .revision_hash;
    let third_revision = crate::manager::cloud_backup_manager::wallets::prepare_wallet_backup(
        &third_wallet,
        third_wallet.wallet_mode,
    )
    .await
    .unwrap()
    .revision_hash;
    globals.cloud.set_wallet_backup(
        first_namespace.clone(),
        first_record_id.clone(),
        encrypted_wallet_backup_bytes(&first_wallet, &first_master_key, &first_revision, 1).await,
    );
    globals.cloud.set_wallet_backup(
        second_namespace.clone(),
        second_record_id.clone(),
        encrypted_wallet_backup_bytes(&second_wallet, &second_master_key, &second_revision, 1)
            .await,
    );
    globals.cloud.set_wallet_backup(
        second_namespace.clone(),
        third_record_id.clone(),
        encrypted_wallet_backup_bytes(&third_wallet, &second_master_key, &third_revision, 1).await,
    );
    globals.cloud.set_wallet_files(
        first_namespace.clone(),
        vec![wallet_filename_from_record_id(&first_record_id)],
    );
    globals.cloud.set_wallet_files(
        second_namespace.clone(),
        vec![
            wallet_filename_from_record_id(&second_record_id),
            wallet_filename_from_record_id(&third_record_id),
        ],
    );

    enable_cloud_backup_create_new(&manager).await.unwrap();

    assert_eq!(manager.current_status(), CloudBackupStatus::Enabled);
    assert_eq!(CloudBackupKeychain::global().namespace_id(), Some(second_namespace.clone()));
    assert_eq!(Database::global().cloud_backup_state.get().unwrap().wallet_count(), Some(3));
    assert!(globals.cloud.has_namespace(&second_namespace));
    wait_for_test_condition(
        Duration::from_secs(1),
        "merged source namespace should be deleted after proof",
        || !globals.cloud.has_namespace(&first_namespace),
    )
    .await;

    let active_records =
        CloudStorage::global_explicit_client().list_wallet_backups(second_namespace).await.unwrap();
    assert!(active_records.contains(&first_record_id));
    assert!(active_records.contains(&second_record_id));
    assert!(active_records.contains(&third_record_id));
}

#[tokio::test(flavor = "current_thread")]
async fn enable_recovery_fails_closed_when_matched_wallet_listing_is_missing() {
    let _guard = async_test_lock().lock().await;
    cove_tokio::init();
    let globals = test_globals();
    let manager = init_manager();

    reset_cloud_backup_test_state(&manager, globals);
    CONNECTIVITY_MANAGER.set_connection_state(true);
    globals.cloud.set_reflect_uploaded_wallets_in_listing(true);

    let prf_key = [7u8; 32];
    let empty_master_key = cove_cspp::master_key::MasterKey::generate();
    let wallet_master_key = cove_cspp::master_key::MasterKey::generate();
    let empty_namespace = empty_master_key.namespace_id();
    let wallet_namespace = wallet_master_key.namespace_id();
    let empty_encrypted =
        cove_cspp::master_key_crypto::encrypt_master_key(&empty_master_key, &prf_key, &[9; 32])
            .unwrap();
    let wallet_encrypted =
        cove_cspp::master_key_crypto::encrypt_master_key(&wallet_master_key, &prf_key, &[8; 32])
            .unwrap();

    globals.cloud.set_master_key_backup(
        empty_namespace.clone(),
        serde_json::to_vec(&empty_encrypted).unwrap(),
    );
    globals.cloud.set_master_key_backup(
        wallet_namespace.clone(),
        serde_json::to_vec(&wallet_encrypted).unwrap(),
    );
    globals.cloud.fail_list_wallet_files_for_namespace(
        empty_namespace.clone(),
        CloudStorageError::NotFound("wallet files missing".into()),
    );
    globals.passkey.set_discover_result(Ok(DiscoveredPasskeyResult {
        prf_output: prf_key.to_vec(),
        credential_id: vec![1, 2, 3],
    }));
    globals.passkey.set_authenticate_result(Ok(prf_key.to_vec()));

    let wallet = xpub_only_wallet_metadata();
    let wallet = WalletMetadata { master_fingerprint: None, ..wallet };
    Keychain::global().save_wallet_xpub(&wallet.id, sample_xpub(&wallet).parse().unwrap()).unwrap();
    let record_id = cove_cspp::backup_data::wallet_record_id(wallet.id.as_ref());
    let revision = crate::manager::cloud_backup_manager::wallets::prepare_wallet_backup(
        &wallet,
        wallet.wallet_mode,
    )
    .await
    .unwrap()
    .revision_hash;
    globals.cloud.set_wallet_backup(
        wallet_namespace.clone(),
        record_id.clone(),
        encrypted_wallet_backup_bytes(&wallet, &wallet_master_key, &revision, 1).await,
    );
    globals.cloud.set_wallet_files(
        wallet_namespace.clone(),
        vec![wallet_filename_from_record_id(&record_id)],
    );

    let error = enable_cloud_backup_create_new(&manager).await.unwrap_err();

    assert!(error.to_string().contains("wallet files missing"), "{error}");
    assert_eq!(CloudBackupKeychain::global().namespace_id(), None);
    assert_eq!(
        Database::global().cloud_backup_state.get().unwrap().status(),
        PersistedCloudBackupStatus::Disabled
    );
    assert!(globals.cloud.has_namespace(&empty_namespace));
    assert!(globals.cloud.has_namespace(&wallet_namespace));
}

#[test]
fn clear_in_process_state_for_local_reset_clears_enable_state() {
    let _guard = test_lock().lock();
    cove_tokio::init();
    let globals = test_globals();
    let manager = init_manager();

    reset_cloud_backup_test_state(&manager, globals);
    manager.apply_enable_outcome(CloudBackupEnableOutcome::AwaitingSavedPasskeyConfirmation(
        SavedPasskeyConfirmationMode::Manual,
    ));

    manager.clear_in_process_state_for_local_reset();

    assert_eq!(manager.model_snapshot().enable_state, CloudBackupEnableState::Idle);
}

#[tokio::test(flavor = "current_thread")]
async fn enable_preserves_awaiting_force_new_session() {
    let _guard = async_test_lock().lock().await;
    cove_tokio::init();
    let globals = test_globals();
    let manager = init_manager();

    reset_cloud_backup_test_state(&manager, globals);
    CONNECTIVITY_MANAGER.set_connection_state(true);
    let existing_master_key = cove_cspp::master_key::MasterKey::generate();
    let existing_namespace = existing_master_key.namespace_id();
    let encrypted_master =
        cove_cspp::master_key_crypto::encrypt_master_key(&existing_master_key, &[7; 32], &[9; 32])
            .unwrap();
    globals.cloud.set_wallet_files(existing_namespace.clone(), vec!["wallet-1.json".into()]);
    globals
        .cloud
        .set_master_key_backup(existing_namespace, serde_json::to_vec(&encrypted_master).unwrap());
    globals.passkey.set_discover_result(Err(PasskeyError::UserCancelled));

    let master_key = cove_cspp::master_key::MasterKey::generate();
    let expected_namespace = master_key.namespace_id();
    let expected_credential_id = vec![1, 2, 3];
    replace_pending_enable_session_for_test(
        &manager,
        pending_enable_awaiting_confirmation(
            master_key,
            UnpersistedPrfKey {
                prf_key: [7; 32],
                prf_salt: [9; 32],
                credential_id: expected_credential_id.clone(),
                provider_hint: None,
            },
            CloudBackupEnableContext::settings_manual(),
        ),
    )
    .await;

    enable_cloud_backup_create_new(&manager).await.unwrap();

    let pending = take_pending_enable_session_for_test(&manager).await.unwrap();
    let (pending_master_key, pending_passkey) = pending.into_ready_parts().unwrap();
    assert_eq!(pending_master_key.namespace_id(), expected_namespace);
    assert_eq!(pending_passkey.credential_id, expected_credential_id);
}

#[tokio::test(flavor = "current_thread")]
async fn enable_create_new_preserves_awaiting_force_new_session() {
    let _guard = async_test_lock().lock().await;
    cove_tokio::init();
    let globals = test_globals();
    let manager = init_manager();

    reset_cloud_backup_test_state(&manager, globals);
    CONNECTIVITY_MANAGER.set_connection_state(true);
    globals.passkey.set_discover_result(Err(PasskeyError::UserCancelled));

    let master_key = cove_cspp::master_key::MasterKey::generate();
    let expected_namespace = master_key.namespace_id();
    let expected_credential_id = vec![1, 2, 3];
    replace_pending_enable_session_for_test(
        &manager,
        pending_enable_awaiting_confirmation(
            master_key,
            UnpersistedPrfKey {
                prf_key: [7; 32],
                prf_salt: [9; 32],
                credential_id: expected_credential_id.clone(),
                provider_hint: None,
            },
            CloudBackupEnableContext::settings_manual(),
        ),
    )
    .await;

    enable_cloud_backup_create_new(&manager).await.unwrap();

    let pending = take_pending_enable_session_for_test(&manager).await.unwrap();
    let (pending_master_key, pending_passkey) = pending.into_ready_parts().unwrap();
    assert_eq!(pending_master_key.namespace_id(), expected_namespace);
    assert_eq!(pending_passkey.credential_id, expected_credential_id);
}

#[tokio::test(flavor = "current_thread")]
async fn enable_no_discovery_preserves_awaiting_force_new_session() {
    let _guard = async_test_lock().lock().await;
    cove_tokio::init();
    let globals = test_globals();
    let manager = init_manager();

    reset_cloud_backup_test_state(&manager, globals);
    CONNECTIVITY_MANAGER.set_connection_state(true);
    globals.passkey.set_create_result(Err(PasskeyError::UserCancelled));

    let master_key = cove_cspp::master_key::MasterKey::generate();
    let expected_namespace = master_key.namespace_id();
    let expected_credential_id = vec![1, 2, 3];
    replace_pending_enable_session_for_test(
        &manager,
        pending_enable_awaiting_confirmation(
            master_key,
            UnpersistedPrfKey {
                prf_key: [7; 32],
                prf_salt: [9; 32],
                credential_id: expected_credential_id.clone(),
                provider_hint: None,
            },
            CloudBackupEnableContext::settings_manual(),
        ),
    )
    .await;

    let create_count = globals.passkey.create_count();

    enable_cloud_backup_no_discovery(&manager).await.unwrap();

    assert_eq!(globals.passkey.create_count(), create_count);
    assert_eq!(manager.current_status(), CloudBackupStatus::Enabling);
    match manager.model_snapshot().root_prompt {
        CloudBackupRootPrompt::ExistingBackupFound(context, _) => {
            assert_eq!(context, CloudBackupEnableContext::settings_manual());
        }
        other => panic!("expected existing backup prompt, got {other:?}"),
    }

    let pending = take_pending_enable_session_for_test(&manager).await.unwrap();
    let (pending_master_key, pending_passkey) = pending.into_ready_parts().unwrap();
    assert_eq!(pending_master_key.namespace_id(), expected_namespace);
    assert_eq!(pending_passkey.credential_id, expected_credential_id);
}

#[tokio::test(flavor = "current_thread")]
async fn force_new_after_other_namespace_enter_detail_reuses_runtime_authorization() {
    let _guard = async_test_lock().lock().await;
    cove_tokio::init();
    let globals = test_globals();
    let manager = init_manager();

    reset_cloud_backup_test_state(&manager, globals);
    CONNECTIVITY_MANAGER.set_connection_state(true);

    let existing_master_key = cove_cspp::master_key::MasterKey::generate();
    let existing_namespace = existing_master_key.namespace_id();
    let encrypted_existing_master =
        cove_cspp::master_key_crypto::encrypt_master_key(&existing_master_key, &[7; 32], &[9; 32])
            .unwrap();
    globals.cloud.set_wallet_files(existing_namespace.clone(), vec!["wallet-1.json".into()]);
    globals.cloud.set_master_key_backup(
        existing_namespace,
        serde_json::to_vec(&encrypted_existing_master).unwrap(),
    );
    globals.passkey.set_create_result(Ok(vec![1, 2, 3]));
    globals.passkey.set_authenticate_result(Ok(vec![7; 32]));

    enable_cloud_backup_no_discovery(&manager).await.unwrap();

    assert_eq!(globals.passkey.create_count(), 0);
    assert_eq!(globals.passkey.authenticate_count(), 0);
    assert_eq!(globals.passkey.discover_count(), 0);
    assert_eq!(manager.current_status(), CloudBackupStatus::Enabling);
    match manager.model_snapshot().root_prompt {
        CloudBackupRootPrompt::ExistingBackupFound(context, _) => {
            assert_eq!(context, CloudBackupEnableContext::settings_manual());
        }
        other => panic!("expected existing backup prompt, got {other:?}"),
    }
    assert!(take_pending_enable_session_for_test(&manager).await.is_none());

    enable_cloud_backup_no_discovery(&manager).await.unwrap();

    assert_eq!(globals.passkey.create_count(), 0);
    assert_eq!(globals.passkey.authenticate_count(), 0);
    assert_eq!(globals.passkey.discover_count(), 0);
    assert!(take_pending_enable_session_for_test(&manager).await.is_none());

    enable_cloud_backup_force_new(&manager).await.unwrap();
    confirm_saved_passkey_session(&manager).await;

    assert!(take_pending_enable_session_for_test(&manager).await.is_none());
    assert_eq!(manager.current_status(), CloudBackupStatus::Enabled);

    globals.passkey.set_discover_result(Err(PasskeyError::UserCancelled));
    let create_count = globals.passkey.create_count();
    let authenticate_count = globals.passkey.authenticate_count();
    let discover_count = globals.passkey.discover_count();

    call!(manager.supervisor.start_enter_detail()).await.unwrap();
    tokio::time::sleep(Duration::from_millis(50)).await;

    assert_eq!(globals.passkey.create_count(), create_count);
    assert_eq!(globals.passkey.authenticate_count(), authenticate_count);
    assert_eq!(globals.passkey.discover_count(), discover_count);
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn force_new_after_existing_backup_prompt_registers_without_discovery() {
    let _guard = async_test_lock().lock().await;
    cove_tokio::init();
    let globals = test_globals();
    let manager = init_manager();

    reset_cloud_backup_test_state(&manager, globals);
    CONNECTIVITY_MANAGER.set_connection_state(true);

    let existing_master_key = cove_cspp::master_key::MasterKey::generate();
    let existing_namespace = existing_master_key.namespace_id();
    let encrypted_existing_master =
        cove_cspp::master_key_crypto::encrypt_master_key(&existing_master_key, &[7; 32], &[9; 32])
            .unwrap();
    globals.cloud.set_wallet_files(existing_namespace.clone(), vec!["wallet-1.json".into()]);
    globals.cloud.set_master_key_backup(
        existing_namespace,
        serde_json::to_vec(&encrypted_existing_master).unwrap(),
    );
    globals.passkey.set_create_result(Ok(vec![1, 2, 3]));
    globals.passkey.set_discover_result(Err(PasskeyError::UserCancelled));

    enable_cloud_backup_no_discovery(&manager).await.unwrap();

    assert_eq!(globals.passkey.create_count(), 0);
    assert_eq!(globals.passkey.discover_count(), 0);
    assert!(matches!(
        manager.model_snapshot().root_prompt,
        CloudBackupRootPrompt::ExistingBackupFound(_, _)
    ));

    enable_cloud_backup_force_new(&manager).await.unwrap();

    assert_eq!(globals.passkey.create_count(), 1);
    assert_eq!(globals.passkey.discover_count(), 0);
    assert_eq!(
        manager.model_snapshot().enable_state,
        CloudBackupEnableState::AwaitingSavedPasskeyConfirmation(
            SavedPasskeyConfirmationMode::Manual,
        )
    );
}

#[tokio::test(flavor = "current_thread")]
async fn existing_backup_prompt_preserves_onboarding_enable_context() {
    let _guard = async_test_lock().lock().await;
    cove_tokio::init();
    let globals = test_globals();
    let manager = init_manager();

    reset_cloud_backup_test_state(&manager, globals);
    CONNECTIVITY_MANAGER.set_connection_state(true);

    let existing_master_key = cove_cspp::master_key::MasterKey::generate();
    let existing_namespace = existing_master_key.namespace_id();
    let encrypted_existing_master =
        cove_cspp::master_key_crypto::encrypt_master_key(&existing_master_key, &[7; 32], &[9; 32])
            .unwrap();
    globals.cloud.set_wallet_files(existing_namespace.clone(), vec!["wallet-1.json".into()]);
    globals.cloud.set_master_key_backup(
        existing_namespace,
        serde_json::to_vec(&encrypted_existing_master).unwrap(),
    );
    globals.passkey.set_create_result(Ok(vec![1, 2, 3]));

    let context = CloudBackupEnableContext {
        saved_passkey_confirmation: SavedPasskeyConfirmationMode::Automatic,
        verification_source: CloudBackupVerificationSource::Onboarding,
    };
    enable_cloud_backup_no_discovery_with_context(&manager, context).await.unwrap();

    assert_eq!(globals.passkey.create_count(), 0);
    assert_eq!(globals.passkey.authenticate_count(), 0);
    assert_eq!(globals.passkey.discover_count(), 0);
    assert!(take_pending_enable_session_for_test(&manager).await.is_none());

    match manager.model_snapshot().root_prompt {
        CloudBackupRootPrompt::ExistingBackupFound(prompt_context, _) => {
            assert_eq!(prompt_context, context);
        }
        other => panic!("expected existing backup prompt, got {other:?}"),
    }

    globals.passkey.set_authenticate_result(Ok(vec![7; 32]));
    enable_cloud_backup_force_new_with_context(&manager, context).await.unwrap();

    assert_eq!(manager.current_status(), CloudBackupStatus::Enabled);
    assert_eq!(globals.passkey.authenticate_count(), 1);
}

#[tokio::test(flavor = "current_thread")]
async fn existing_passkey_onboarding_recovery_completes_verification() {
    let _guard = async_test_lock().lock().await;
    cove_tokio::init();
    let globals = test_globals();
    let manager = init_manager();

    reset_cloud_backup_test_state(&manager, globals);
    CONNECTIVITY_MANAGER.set_connection_state(true);

    let prf_key = [7u8; 32];
    let existing_master_key = cove_cspp::master_key::MasterKey::generate();
    let existing_namespace = existing_master_key.namespace_id();
    let encrypted_existing_master =
        cove_cspp::master_key_crypto::encrypt_master_key(&existing_master_key, &prf_key, &[9; 32])
            .unwrap();
    globals.cloud.set_master_key_backup(
        existing_namespace,
        serde_json::to_vec(&encrypted_existing_master).unwrap(),
    );
    globals.passkey.set_discover_result(Ok(DiscoveredPasskeyResult {
        prf_output: prf_key.to_vec(),
        credential_id: vec![1, 2, 3],
    }));

    let context = CloudBackupEnableContext {
        saved_passkey_confirmation: SavedPasskeyConfirmationMode::Automatic,
        verification_source: CloudBackupVerificationSource::Onboarding,
    };
    enable_cloud_backup_no_discovery_with_context(&manager, context).await.unwrap();

    assert!(matches!(
        manager.model_snapshot().root_prompt,
        CloudBackupRootPrompt::ExistingBackupFound(_, _)
    ));

    manager.dispatch(CloudBackupManagerAction::AcceptEnablePrompt(
        CloudBackupEnablePromptChoice::UseExisting,
    ));

    wait_for_test_condition(
        Duration::from_secs(2),
        "existing passkey onboarding recovery should verify",
        || {
            let snapshot = manager.model_snapshot();
            manager.current_status() == CloudBackupStatus::Enabled
                && matches!(snapshot.verification, VerificationState::Verified(_))
        },
    )
    .await;

    assert_eq!(
        manager.model_snapshot().pending_upload_verification,
        PendingUploadVerificationState::Idle
    );
}

#[tokio::test(flavor = "current_thread")]
async fn detail_entry_after_restart_without_active_authorization_prompts_normally() {
    let _guard = async_test_lock().lock().await;
    cove_tokio::init();
    let globals = test_globals();
    let manager = init_manager();

    reset_cloud_backup_test_state(&manager, globals);
    CONNECTIVITY_MANAGER.set_connection_state(true);
    globals.passkey.set_create_result(Ok(vec![1, 2, 3]));
    globals.passkey.set_authenticate_result(Ok(vec![7; 32]));

    enable_cloud_backup_no_discovery(&manager).await.unwrap();
    confirm_saved_passkey_session(&manager).await;

    assert_eq!(manager.current_status(), CloudBackupStatus::Enabled);

    let restarted_manager = init_manager();
    restarted_manager.sync_persisted_state();
    restarted_manager.clear_pending_verification_completion();
    restarted_manager.reconcile_pending_upload_verification(PendingUploadVerificationState::Idle);
    restarted_manager.apply_verification_outcome(CloudBackupVerificationOutcome::Idle);
    Database::global().cloud_blob_sync_states.delete_all().unwrap();
    globals.passkey.set_discover_result(Err(PasskeyError::UserCancelled));

    let discover_count = globals.passkey.discover_count();

    call!(restarted_manager.supervisor.start_enter_detail()).await.unwrap();
    wait_for_discover_count(globals, discover_count + 1).await;

    assert_eq!(globals.passkey.discover_count(), discover_count + 1);
}

#[tokio::test(flavor = "current_thread")]
async fn enable_force_new_consumes_staged_session() {
    let _guard = async_test_lock().lock().await;
    cove_tokio::init();
    let globals = test_globals();
    let manager = init_manager();

    reset_cloud_backup_test_state(&manager, globals);
    CONNECTIVITY_MANAGER.set_connection_state(true);

    replace_pending_enable_session_for_test(
        &manager,
        pending_enable_awaiting_confirmation(
            cove_cspp::master_key::MasterKey::generate(),
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

    enable_cloud_backup_force_new(&manager).await.unwrap();

    assert!(take_pending_enable_session_for_test(&manager).await.is_none());
    assert_eq!(manager.current_status(), CloudBackupStatus::Enabled);
}

#[tokio::test(flavor = "current_thread")]
async fn cancelled_enable_create_new_rolls_back_new_local_master_key() {
    let _guard = async_test_lock().lock().await;
    cove_tokio::init();
    let globals = test_globals();
    let manager = init_manager();

    reset_cloud_backup_test_state(&manager, globals);
    globals.passkey.set_discover_result(Err(PasskeyError::UserCancelled));

    enable_cloud_backup_create_new(&manager).await.unwrap();

    let cspp = cove_cspp::Cspp::new(Keychain::global().clone());
    assert!(cspp.load_master_key_from_store().unwrap().is_none());
    assert_eq!(manager.current_status(), CloudBackupStatus::Enabling);
    assert!(matches!(
        manager.model_snapshot().root_prompt,
        CloudBackupRootPrompt::PasskeyChoice(CloudBackupPasskeyChoiceIntent::Enable(_, _))
    ));
}

#[tokio::test(flavor = "current_thread")]
async fn cancelled_enable_no_discovery_rolls_back_new_local_master_key() {
    let _guard = async_test_lock().lock().await;
    cove_tokio::init();
    let globals = test_globals();
    let manager = init_manager();

    reset_cloud_backup_test_state(&manager, globals);
    globals.passkey.set_create_result(Err(PasskeyError::UserCancelled));

    enable_cloud_backup_no_discovery(&manager).await.unwrap();

    let cspp = cove_cspp::Cspp::new(Keychain::global().clone());
    assert!(cspp.load_master_key_from_store().unwrap().is_none());
    assert_eq!(manager.current_status(), CloudBackupStatus::Enabling);
    assert!(matches!(
        manager.model_snapshot().root_prompt,
        CloudBackupRootPrompt::PasskeyChoice(CloudBackupPasskeyChoiceIntent::Enable(_, _))
    ));
}

#[tokio::test(flavor = "current_thread")]
async fn cancelled_passkey_restore_does_not_fall_back_to_local_master_key() {
    let _guard = async_test_lock().lock().await;
    cove_tokio::init();
    let globals = test_globals();
    let manager = init_manager();

    reset_cloud_backup_test_state(&manager, globals);

    let local_master_key = cove_cspp::master_key::MasterKey::generate();
    let local_namespace_id = local_master_key.namespace_id();
    cove_cspp::Cspp::new(Keychain::global().clone()).save_master_key(&local_master_key).unwrap();
    globals.cloud.set_wallet_files(local_namespace_id.clone(), vec!["wallet-test.json".into()]);

    let remote_master_key = cove_cspp::master_key::MasterKey::generate();
    let remote_namespace_id = remote_master_key.namespace_id();
    let remote_prf_key = [7u8; 32];
    let remote_prf_salt = [9u8; 32];
    let encrypted_master = cove_cspp::master_key_crypto::encrypt_master_key(
        &remote_master_key,
        &remote_prf_key,
        &remote_prf_salt,
    )
    .unwrap();
    globals.cloud.set_master_key_backup(
        remote_namespace_id.clone(),
        serde_json::to_vec(&encrypted_master).unwrap(),
    );
    globals.cloud.set_wallet_files(remote_namespace_id, vec!["wallet-remote.json".into()]);
    globals.passkey.set_discover_result(Err(PasskeyError::UserCancelled));

    let operation = new_restore_operation_for_test(&manager).await;
    let error = operation.restore_from_cloud_backup(&manager).await.unwrap_err();

    assert!(matches!(error, CloudBackupError::PasskeyDiscoveryCancelled));
    assert_eq!(CloudBackupKeychain::global().namespace_id(), None);
}
