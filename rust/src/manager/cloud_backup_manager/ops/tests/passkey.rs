use super::*;

#[tokio::test(flavor = "current_thread")]
async fn non_missing_discovery_failure_never_registers_enable_passkey() {
    let _guard = async_test_lock().lock().await;
    cove_tokio::init();
    let globals = test_globals();
    globals.reset();
    globals.passkey.set_discover_result(Err(PasskeyError::RequestFailed {
        operation: PasskeyOperation::DiscoverAssertion,
        reason: PasskeyFailureReason::InvalidResponse,
    }));
    globals.passkey.set_create_result(Ok(vec![1, 2, 3]));

    let result = PasskeyMaterialAcquirer::new(PasskeyAccess::global())
        .discover_or_register_for_enable()
        .await;

    assert!(matches!(result, Err(CloudBackupError::Passkey(_))));
    assert_eq!(globals.passkey.discover_count(), 1);
    assert_eq!(globals.passkey.create_count(), 0);
}

#[tokio::test(flavor = "current_thread")]
async fn passkey_match_treats_missing_credential_as_no_match() {
    let _guard = async_test_lock().lock().await;
    cove_tokio::init();
    let globals = test_globals();
    globals.reset();

    let master_key = cove_cspp::master_key::MasterKey::generate();
    let namespace = master_key.namespace_id();
    let encrypted_master =
        cove_cspp::master_key_crypto::encrypt_master_key(&master_key, &[7; 32], &[9; 32]).unwrap();
    globals
        .cloud
        .set_master_key_backup(namespace.clone(), serde_json::to_vec(&encrypted_master).unwrap());
    globals.passkey.set_discover_result(Err(PasskeyError::NoCredentialFound));

    let outcome = NamespacePasskeyMatcher::new(
        &CloudStorage::global_explicit_client(),
        PasskeyAccess::global(),
    )
    .match_namespaces(&[namespace])
    .await
    .unwrap();

    assert!(matches!(outcome, NamespaceMatchOutcome::NoMatch));
}

#[tokio::test(flavor = "current_thread")]
async fn passkey_match_treats_user_cancel_as_user_declined() {
    let _guard = async_test_lock().lock().await;
    cove_tokio::init();
    let globals = test_globals();
    globals.reset();

    let master_key = cove_cspp::master_key::MasterKey::generate();
    let namespace = master_key.namespace_id();
    let encrypted_master =
        cove_cspp::master_key_crypto::encrypt_master_key(&master_key, &[7; 32], &[9; 32]).unwrap();
    globals
        .cloud
        .set_master_key_backup(namespace.clone(), serde_json::to_vec(&encrypted_master).unwrap());
    globals.passkey.set_discover_result(Err(PasskeyError::UserCancelled));

    let outcome = NamespacePasskeyMatcher::new(
        &CloudStorage::global_explicit_client(),
        PasskeyAccess::global(),
    )
    .match_namespaces(&[namespace])
    .await
    .unwrap();

    assert!(matches!(outcome, NamespaceMatchOutcome::UserDeclined));
}

#[tokio::test(flavor = "current_thread")]
async fn passkey_match_mixed_supported_and_unsupported_versions_returns_no_match() {
    let _guard = async_test_lock().lock().await;
    cove_tokio::init();
    let globals = test_globals();
    globals.reset();

    let supported_master_key = cove_cspp::master_key::MasterKey::generate();
    let supported_namespace = supported_master_key.namespace_id();
    let unsupported_master_key = cove_cspp::master_key::MasterKey::generate();
    let unsupported_namespace = unsupported_master_key.namespace_id();
    let encrypted_master =
        cove_cspp::master_key_crypto::encrypt_master_key(&supported_master_key, &[7; 32], &[9; 32])
            .unwrap();
    let mut unsupported_master = cove_cspp::master_key_crypto::encrypt_master_key(
        &unsupported_master_key,
        &[7; 32],
        &[9; 32],
    )
    .unwrap();
    unsupported_master.version = 2;

    globals.cloud.set_master_key_backup(
        supported_namespace.clone(),
        serde_json::to_vec(&encrypted_master).unwrap(),
    );
    globals.cloud.set_master_key_backup(
        unsupported_namespace.clone(),
        serde_json::to_vec(&unsupported_master).unwrap(),
    );
    globals.passkey.set_discover_result(Ok(DiscoveredPasskeyResult {
        prf_output: vec![8; 32],
        credential_id: vec![1, 2, 3],
    }));

    let outcome = NamespacePasskeyMatcher::new(
        &CloudStorage::global_explicit_client(),
        PasskeyAccess::global(),
    )
    .match_namespaces(&[supported_namespace, unsupported_namespace])
    .await
    .unwrap();

    assert!(matches!(outcome, NamespaceMatchOutcome::NoMatch));
}

#[tokio::test(flavor = "current_thread")]
async fn passkey_match_discovery_propagates_unsupported_provider() {
    let _guard = async_test_lock().lock().await;
    cove_tokio::init();
    let globals = test_globals();
    globals.reset();

    let master_key = cove_cspp::master_key::MasterKey::generate();
    let namespace = master_key.namespace_id();
    let encrypted_master =
        cove_cspp::master_key_crypto::encrypt_master_key(&master_key, &[7; 32], &[9; 32]).unwrap();
    globals
        .cloud
        .set_master_key_backup(namespace.clone(), serde_json::to_vec(&encrypted_master).unwrap());
    globals.passkey.set_discover_result(Err(PasskeyError::PrfUnsupportedProvider));

    let result = NamespacePasskeyMatcher::new(
        &CloudStorage::global_explicit_client(),
        PasskeyAccess::global(),
    )
    .match_namespaces(&[namespace])
    .await;
    let error = match result {
        Ok(_) => panic!("expected unsupported passkey provider error"),
        Err(error) => error,
    };

    assert!(matches!(error, CloudBackupError::UnsupportedPasskeyProvider));
}

#[tokio::test(flavor = "current_thread")]
async fn passkey_match_targeted_auth_propagates_unsupported_provider() {
    let _guard = async_test_lock().lock().await;
    cove_tokio::init();
    let globals = test_globals();
    globals.reset();

    let first_master_key = cove_cspp::master_key::MasterKey::generate();
    let first_namespace = first_master_key.namespace_id();
    let second_master_key = cove_cspp::master_key::MasterKey::generate();
    let second_namespace = second_master_key.namespace_id();
    let first_encrypted =
        cove_cspp::master_key_crypto::encrypt_master_key(&first_master_key, &[7; 32], &[9; 32])
            .unwrap();
    let second_encrypted =
        cove_cspp::master_key_crypto::encrypt_master_key(&second_master_key, &[8; 32], &[9; 32])
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
        prf_output: vec![1; 32],
        credential_id: vec![1, 2, 3],
    }));
    globals.passkey.set_authenticate_result(Err(PasskeyError::PrfUnsupportedProvider));

    let result = NamespacePasskeyMatcher::new(
        &CloudStorage::global_explicit_client(),
        PasskeyAccess::global(),
    )
    .match_namespaces(&[first_namespace, second_namespace])
    .await;
    let error = match result {
        Ok(_) => panic!("expected unsupported passkey provider error"),
        Err(error) => error,
    };

    assert!(matches!(error, CloudBackupError::UnsupportedPasskeyProvider));
}

#[tokio::test(flavor = "current_thread")]
async fn passkey_match_allows_one_credential_to_match_multiple_namespaces() {
    let _guard = async_test_lock().lock().await;
    cove_tokio::init();
    let globals = test_globals();
    globals.reset();

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

    let outcome = NamespacePasskeyMatcher::new(
        &CloudStorage::global_explicit_client(),
        PasskeyAccess::global(),
    )
    .match_namespaces(&[first_namespace.clone(), second_namespace.clone()])
    .await
    .unwrap();

    let NamespaceMatchOutcome::Matched(matches) = outcome else {
        panic!("expected multiple namespace matches");
    };
    let matched_namespaces =
        matches.into_iter().map(|matched| matched.namespace_id).collect::<Vec<_>>();

    assert_eq!(matched_namespaces, vec![first_namespace, second_namespace]);
}

#[tokio::test(flavor = "current_thread")]
async fn wrapper_repair_discovery_propagates_unsupported_provider() {
    let _guard = async_test_lock().lock().await;
    cove_tokio::init();
    let globals = test_globals();
    globals.reset();
    globals.passkey.set_discover_result(Err(PasskeyError::PrfUnsupportedProvider));

    let acquirer = PasskeyMaterialAcquirer::new(PasskeyAccess::global());
    let discovery_result = acquirer.discover_or_create_for_wrapper_repair().await;
    let error = match discovery_result {
        Ok(_) => panic!("expected unsupported passkey provider error"),
        Err(error) => error,
    };

    assert!(matches!(error, CloudBackupError::UnsupportedPasskeyProvider));
}

#[test]
fn save_passkey_rolls_back_on_second_save_failure() {
    let _guard = test_lock().lock();
    let globals = test_globals();
    globals.reset();
    globals.keychain.set_entries(vec![
        (CSPP_CREDENTIAL_ID_KEY, "old_credential"),
        (CSPP_PRF_SALT_KEY, "old_salt"),
    ]);
    globals.keychain.fail_save_at(2);

    let error = CloudBackupKeychain::global().save_passkey(&[1, 2, 3], [7; 32]).unwrap_err();

    assert!(matches!(
        error,
        CloudBackupKeychainError::Keychain(cove_device::keychain::KeychainError::Save)
    ));
    assert_eq!(
        globals.keychain.get_entry(CSPP_CREDENTIAL_ID_KEY).as_deref(),
        Some("old_credential")
    );
    assert_eq!(globals.keychain.get_entry(CSPP_PRF_SALT_KEY).as_deref(), Some("old_salt"));
}

#[test]
fn save_passkey_and_namespace_rolls_back_on_third_save_failure() {
    let _guard = test_lock().lock();
    let globals = test_globals();
    globals.reset();
    globals.keychain.set_entries(vec![
        (CSPP_CREDENTIAL_ID_KEY, "old_credential"),
        (CSPP_PRF_SALT_KEY, "old_salt"),
        (CSPP_NAMESPACE_ID_KEY, "old_namespace"),
    ]);
    globals.keychain.fail_save_at(3);

    let error = CloudBackupKeychain::global()
        .save_passkey_and_namespace(&[1, 2, 3], [9; 32], "new_namespace")
        .unwrap_err();

    assert!(matches!(
        error,
        CloudBackupKeychainError::Keychain(cove_device::keychain::KeychainError::Save)
    ));
    assert_eq!(
        globals.keychain.get_entry(CSPP_CREDENTIAL_ID_KEY).as_deref(),
        Some("old_credential")
    );
    assert_eq!(globals.keychain.get_entry(CSPP_PRF_SALT_KEY).as_deref(), Some("old_salt"));
    assert_eq!(globals.keychain.get_entry(CSPP_NAMESPACE_ID_KEY).as_deref(), Some("old_namespace"));
}

#[test]
fn load_credential_id_returns_none_for_invalid_hex_and_decodes_valid_hex() {
    let _guard = test_lock().lock();
    let globals = test_globals();
    globals.reset();
    globals.keychain.set_entries(vec![(CSPP_CREDENTIAL_ID_KEY, "not-hex")]);

    assert!(CloudBackupKeychain::global().load_credential_id().is_none());

    let credential_id = vec![1, 2, 3, 254, 255];
    let credential_hex = hex::encode(&credential_id);
    globals.keychain.set_entries(vec![(CSPP_CREDENTIAL_ID_KEY, &credential_hex)]);

    assert_eq!(CloudBackupKeychain::global().load_credential_id(), Some(credential_id));
}

#[test]
fn clear_passkey_removes_credential_and_salt_only() {
    let _guard = test_lock().lock();
    let globals = test_globals();
    globals.reset();
    globals.keychain.set_entries(vec![
        (CSPP_CREDENTIAL_ID_KEY, "credential"),
        (CSPP_PRF_SALT_KEY, "salt"),
        (CSPP_NAMESPACE_ID_KEY, "namespace"),
    ]);

    CloudBackupKeychain::global().clear_passkey();

    assert!(globals.keychain.get_entry(CSPP_CREDENTIAL_ID_KEY).is_none());
    assert!(globals.keychain.get_entry(CSPP_PRF_SALT_KEY).is_none());
    assert_eq!(globals.keychain.get_entry(CSPP_NAMESPACE_ID_KEY).as_deref(), Some("namespace"));
}

#[test]
fn pending_enable_journal_roundtrips_without_prf_output() {
    let _guard = test_lock().lock();
    let globals = test_globals();
    globals.reset();
    globals.keychain.set_entries(vec![
        (CSPP_CREDENTIAL_ID_KEY, "prior-credential"),
        (CSPP_PRF_SALT_KEY, "prior-salt"),
        (CSPP_NAMESPACE_ID_KEY, "prior-namespace"),
    ]);

    let cloud_keychain = CloudBackupKeychain::global();
    let mut journal = staged_pending_enable_journal(
        CloudBackupEnableContext::settings_manual(),
        "fresh-namespace".into(),
        PendingEnableNamespaceOwnership::FreshOwned,
        cloud_keychain.snapshot_passkey_metadata(),
    );
    assert!(journal.register_passkey(PendingEnablePasskeyMetadata {
        credential_id: vec![1, 2, 3],
        prf_salt: [9; 32],
        provider_hint: None,
    }));
    assert!(journal.mark_remote_writes_started());

    cloud_keychain.save_pending_enable_journal(&journal).unwrap();

    let encoded = globals.keychain.get_entry(CSPP_PENDING_ENABLE_JOURNAL_KEY).unwrap();
    assert!(!encoded.contains("prf_key"));
    assert_eq!(cloud_keychain.load_pending_enable_journal().unwrap(), Some(journal));
}

#[test]
fn pending_enable_journal_rejects_unknown_version() {
    let _guard = test_lock().lock();
    let globals = test_globals();
    globals.reset();

    let cloud_keychain = CloudBackupKeychain::global();
    let journal = staged_pending_enable_journal(
        CloudBackupEnableContext::settings_manual(),
        "fresh-namespace".into(),
        PendingEnableNamespaceOwnership::FreshOwned,
        cloud_keychain.snapshot_passkey_metadata(),
    );
    let mut value = serde_json::to_value(journal).unwrap();
    value["version"] = serde_json::json!(99);
    globals.keychain.set_entries(vec![(
        CSPP_PENDING_ENABLE_JOURNAL_KEY,
        &serde_json::to_string(&value).unwrap(),
    )]);

    assert!(matches!(
        cloud_keychain.load_pending_enable_journal(),
        Err(CloudBackupKeychainError::UnsupportedPendingEnableVersion(99))
    ));
}

#[test]
fn pending_enable_journal_rejects_out_of_order_or_conflicting_transitions() {
    let _guard = test_lock().lock();
    let globals = test_globals();
    globals.reset();

    let mut journal = staged_pending_enable_journal(
        CloudBackupEnableContext::settings_manual(),
        "fresh-namespace".into(),
        PendingEnableNamespaceOwnership::FreshOwned,
        CloudBackupKeychain::global().snapshot_passkey_metadata(),
    );
    let passkey = PendingEnablePasskeyMetadata {
        credential_id: vec![1, 2, 3],
        prf_salt: [9; 32],
        provider_hint: None,
    };

    assert!(!journal.mark_remote_writes_started());
    assert!(!journal.mark_local_promotion_started());
    assert!(journal.register_passkey(passkey.clone()));
    assert!(!journal.register_passkey(PendingEnablePasskeyMetadata {
        credential_id: vec![4, 5, 6],
        ..passkey.clone()
    }));
    assert!(journal.mark_remote_writes_started());
    assert!(journal.mark_remote_writes_started());
    assert!(journal.mark_local_promotion_started());
    assert!(journal.mark_local_promotion_started());
    assert!(journal.roll_back_local_promotion());
    assert!(journal.roll_back_local_promotion());
}

#[test]
fn pending_enable_metadata_snapshot_restores_exact_prior_values() {
    let _guard = test_lock().lock();
    let globals = test_globals();
    globals.reset();
    globals.keychain.set_entries(vec![
        (CSPP_CREDENTIAL_ID_KEY, "prior-credential"),
        (CSPP_NAMESPACE_ID_KEY, "prior-namespace"),
    ]);

    let cloud_keychain = CloudBackupKeychain::global();
    let snapshot = cloud_keychain.snapshot_passkey_metadata();
    cloud_keychain.save_passkey_and_namespace(&[1, 2, 3], [9; 32], "fresh-namespace").unwrap();

    cloud_keychain.restore_passkey_metadata(&snapshot).unwrap();

    assert_eq!(
        globals.keychain.get_entry(CSPP_CREDENTIAL_ID_KEY).as_deref(),
        Some("prior-credential")
    );
    assert!(globals.keychain.get_entry(CSPP_PRF_SALT_KEY).is_none());
    assert_eq!(
        globals.keychain.get_entry(CSPP_NAMESPACE_ID_KEY).as_deref(),
        Some("prior-namespace")
    );
}

#[test]
fn clear_local_state_treats_empty_keychain_as_success() {
    let _guard = test_lock().lock();
    let globals = test_globals();
    globals.reset();

    CloudBackupKeychain::global().clear_local_state().unwrap();
    assert!(CloudBackupKeychain::global().namespace_id().is_none());
}

#[test]
fn clear_local_state_removes_master_key_and_passkey_metadata() {
    let _guard = test_lock().lock();
    let globals = test_globals();
    globals.reset();

    let keychain = Keychain::global();
    let cloud_keychain = CloudBackupKeychain::global();
    let cspp = cove_cspp::Cspp::new(keychain.clone());
    let master_key = cove_cspp::master_key::MasterKey::generate();
    cspp.save_master_key(&master_key).unwrap();
    cloud_keychain.save_passkey_and_namespace(&[1, 2, 3], [4; 32], "test-namespace").unwrap();
    cloud_keychain
        .save_pending_enable_journal(&staged_pending_enable_journal(
            CloudBackupEnableContext::settings_manual(),
            "pending-namespace".into(),
            PendingEnableNamespaceOwnership::FreshOwned,
            cloud_keychain.snapshot_passkey_metadata(),
        ))
        .unwrap();

    assert!(cspp.load_master_key_from_store().unwrap().is_some());
    assert!(keychain.get(CSPP_CREDENTIAL_ID_KEY.into()).is_some());
    assert!(keychain.get(CSPP_PRF_SALT_KEY.into()).is_some());
    assert!(keychain.get(CSPP_NAMESPACE_ID_KEY.into()).is_some());
    assert!(keychain.get(CSPP_PENDING_ENABLE_JOURNAL_KEY.into()).is_some());

    cloud_keychain.clear_local_state().unwrap();

    assert!(cspp.load_master_key_from_store().unwrap().is_none());
    assert!(keychain.get(CSPP_CREDENTIAL_ID_KEY.into()).is_none());
    assert!(keychain.get(CSPP_PRF_SALT_KEY.into()).is_none());
    assert!(keychain.get(CSPP_NAMESPACE_ID_KEY.into()).is_none());
    assert!(keychain.get(CSPP_PENDING_ENABLE_JOURNAL_KEY.into()).is_none());
}

#[test]
fn clear_local_state_attempts_passkey_metadata_after_master_key_delete_failure() {
    let _guard = test_lock().lock();
    let globals = test_globals();
    globals.reset();

    let keychain = Keychain::global();
    let cloud_keychain = CloudBackupKeychain::global();
    let cspp = cove_cspp::Cspp::new(keychain.clone());
    let master_key = cove_cspp::master_key::MasterKey::generate();
    cspp.save_master_key(&master_key).unwrap();
    cloud_keychain.save_passkey_and_namespace(&[1, 2, 3], [4; 32], "test-namespace").unwrap();

    globals.keychain.fail_delete_at(1);

    let error = cloud_keychain.clear_local_state().unwrap_err();

    assert!(matches!(
        error,
        CloudBackupKeychainError::Keychain(cove_device::keychain::KeychainError::Delete)
    ));
    assert!(keychain.get(CSPP_CREDENTIAL_ID_KEY.into()).is_none());
    assert!(keychain.get(CSPP_PRF_SALT_KEY.into()).is_none());
    assert!(keychain.get(CSPP_NAMESPACE_ID_KEY.into()).is_none());
}

#[tokio::test(flavor = "current_thread")]
async fn passkey_repair_finalization_keeps_existing_count_when_wallet_refresh_fails() {
    let _guard = async_test_lock().lock().await;
    cove_tokio::init();
    let globals = test_globals();
    let manager = init_manager();
    configure_enabled_cloud_backup(&manager, globals, 2);

    Database::global()
        .cloud_backup_state
        .set(&persisted_passkey_missing_cloud_backup_state(Some(7)))
        .unwrap();
    manager.sync_persisted_state();
    globals.cloud.fail_list_wallet_files("timed out");

    let finalization = manager.prepare_passkey_repair_finalization().await.unwrap();
    manager.apply_passkey_repair_finalization(finalization).unwrap();

    let state = Database::global().cloud_backup_state.get().unwrap();
    assert_eq!(state.status(), PersistedCloudBackupStatus::Enabled);
    assert_eq!(state.wallet_count(), Some(7));
    assert_eq!(manager.model_snapshot().status, CloudBackupStatus::Enabled);
}

#[tokio::test(flavor = "current_thread")]
async fn passkey_repair_finalization_keeps_existing_count_when_wallet_listing_is_missing() {
    let _guard = async_test_lock().lock().await;
    cove_tokio::init();
    let globals = test_globals();
    let manager = init_manager();
    configure_enabled_cloud_backup(&manager, globals, 2);

    Database::global()
        .cloud_backup_state
        .set(&persisted_passkey_missing_cloud_backup_state(Some(7)))
        .unwrap();
    manager.sync_persisted_state();
    let namespace = CloudBackupKeychain::global().namespace_id().unwrap();
    globals.cloud.fail_list_wallet_files_for_namespace(
        namespace,
        CloudStorageError::NotFound("wallet files missing".into()),
    );

    let finalization = manager.prepare_passkey_repair_finalization().await.unwrap();
    manager.apply_passkey_repair_finalization(finalization).unwrap();

    let state = Database::global().cloud_backup_state.get().unwrap();
    assert_eq!(state.status(), PersistedCloudBackupStatus::Enabled);
    assert_eq!(state.wallet_count(), Some(7));
    assert_eq!(manager.model_snapshot().status, CloudBackupStatus::Enabled);
}

#[tokio::test(flavor = "current_thread")]
async fn wrapper_repair_refreshes_missing_master_key_sync_health_to_uploading() {
    let _guard = async_test_lock().lock().await;
    cove_tokio::init();
    let globals = test_globals();
    let manager = init_manager();
    configure_enabled_cloud_backup(&manager, globals, 1);

    let metadata = xpub_only_wallet_metadata();
    persist_xpub_wallets(vec![metadata]);
    globals.passkey.set_create_result(Ok(vec![1, 2, 3]));
    globals.passkey.set_authenticate_result(Ok(vec![7; 32]));
    manager.observe_sync_health(CloudSyncHealth::Failed(
        SYNC_HEALTH_MISSING_MASTER_KEY_MESSAGE.into(),
    ));

    run_repair_passkey_operation(&manager, true).await;

    for _ in 0..20 {
        if manager.model_snapshot().sync_health == CloudSyncHealth::Uploading {
            break;
        }

        tokio::time::sleep(Duration::from_millis(10)).await;
    }

    assert_eq!(manager.model_snapshot().sync_health, CloudSyncHealth::Uploading);

    clear_wallet_upload_runtime_for_test_async(&manager).await;
}

#[tokio::test(flavor = "current_thread")]
async fn wrapper_repair_fails_closed_when_wallet_listing_is_missing() {
    let _guard = async_test_lock().lock().await;
    cove_tokio::init();
    let globals = test_globals();
    let manager = init_manager();
    configure_enabled_cloud_backup(&manager, globals, 1);

    let namespace = CloudBackupKeychain::global().namespace_id().unwrap();
    globals.cloud.fail_list_wallet_files_for_namespace(
        namespace,
        CloudStorageError::NotFound("wallet files missing".into()),
    );

    let error = manager.prepare_passkey_wrapper_repair_no_discovery().await.unwrap_err();

    assert!(error.to_string().contains("wallet files missing"), "{error}");
}

#[tokio::test(flavor = "current_thread")]
async fn wrapper_repair_reports_failure_after_upload_when_passkey_persistence_fails() {
    let _guard = async_test_lock().lock().await;
    cove_tokio::init();
    let globals = test_globals();
    let manager = init_manager();
    configure_enabled_cloud_backup(&manager, globals, 0);

    let namespace = CloudBackupKeychain::global().namespace_id().unwrap();
    globals.passkey.set_create_result(Ok(vec![1, 2, 3]));
    globals.passkey.set_authenticate_result(Ok(vec![7; 32]));
    globals.keychain.fail_save_at(1);

    run_repair_passkey_operation(&manager, true).await;

    assert!(globals.cloud.has_master_key_backup(&namespace));
    assert_eq!(CloudBackupKeychain::global().load_credential_id(), None);
    assert_eq!(CloudBackupKeychain::global().load_prf_salt(), None);
}
