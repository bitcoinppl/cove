use super::*;

#[tokio::test(flavor = "current_thread")]
async fn fetch_cloud_only_wallets_surfaces_unsupported_versions() {
    let _guard = async_test_lock().lock().await;
    cove_tokio::init();
    let globals = test_globals();
    let manager = init_manager();
    configure_enabled_cloud_backup(&manager, globals, 0);

    let metadata = xpub_only_wallet_metadata();
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

    let wallets = manager.do_fetch_cloud_only_wallets().await.unwrap();

    assert_eq!(wallets.len(), 1);
    assert_eq!(wallets[0].record_id, record_id);
    assert_eq!(wallets[0].name, UNSUPPORTED_CLOUD_ONLY_WALLET_NAME);
    assert_eq!(wallets[0].sync_status, CloudBackupWalletStatus::UnsupportedVersion);
    assert_eq!(wallets[0].network, None);
    assert_eq!(wallets[0].wallet_mode, None);
    assert_eq!(wallets[0].wallet_type, None);
    assert_eq!(wallets[0].label_count, None);
    assert_eq!(wallets[0].backup_updated_at, None);
}

#[tokio::test(flavor = "current_thread")]
async fn detail_reports_other_backup_namespaces() {
    let _guard = async_test_lock().lock().await;
    cove_tokio::init();
    let globals = test_globals();
    let manager = init_manager();
    configure_enabled_cloud_backup(&manager, globals, 0);

    let current_namespace = CloudBackupKeychain::global().namespace_id().unwrap();
    globals.cloud.set_master_key_backup(current_namespace.clone(), vec![1, 2, 3]);
    globals.cloud.set_wallet_files(
        current_namespace,
        vec![wallet_filename_from_record_id("current-wallet")],
    );

    let other_master_key = cove_cspp::master_key::MasterKey::generate();
    let other_namespace = other_master_key.namespace_id();
    let encrypted_master = cove_cspp::master_key_crypto::encrypt_master_key_with_provider_hint(
        &other_master_key,
        &[7; 32],
        &[9; 32],
        Some(cove_cspp::backup_data::PasskeyProviderHint {
            aaguid: "ea9b8d66-4d01-1d21-3ce4-b6b48cb575d4".into(),
            registered_platform: cove_cspp::backup_data::PasskeyRegistrationPlatform::Android,
            registered_at: 1_777_661_234,
            name_suffix: "09IX".into(),
        }),
    )
    .unwrap();
    globals.cloud.set_master_key_backup(
        other_namespace.clone(),
        serde_json::to_vec(&encrypted_master).unwrap(),
    );
    globals.cloud.set_wallet_files(
        other_namespace,
        vec![
            wallet_filename_from_record_id("other-wallet-1"),
            wallet_filename_from_record_id("other-wallet-2"),
        ],
    );

    let Some(CloudBackupDetailResult::Success(detail)) =
        manager.refresh_cloud_backup_detail().await
    else {
        panic!("expected cloud backup detail");
    };

    let CloudBackupOtherBackupsState::Loaded { summary } = detail.other_backups else {
        panic!("expected loaded other backups");
    };
    assert_eq!(summary.namespace_count, 1);
    assert_eq!(summary.wallet_count, 2);
    assert_eq!(summary.passkey_hints.len(), 1);
    assert_eq!(summary.passkey_hints[0].name_suffix, "09IX");
    assert_eq!(summary.passkey_hints[0].provider_name.as_deref(), Some("Google Password Manager"));
}

#[tokio::test(flavor = "current_thread")]
async fn other_backup_summary_counts_only_wallets_missing_from_local_wallets() {
    let _guard = async_test_lock().lock().await;
    cove_tokio::init();
    let globals = test_globals();
    let manager = init_manager();
    configure_enabled_cloud_backup(&manager, globals, 0);

    let metadata = xpub_only_wallet_metadata();
    let local_record_id = cove_cspp::backup_data::wallet_record_id(metadata.id.as_ref());
    persist_xpub_wallets(vec![metadata]);

    let current_namespace = CloudBackupKeychain::global().namespace_id().unwrap();
    globals.cloud.set_wallet_files(current_namespace.clone(), Vec::new());

    let other_master_key = cove_cspp::master_key::MasterKey::generate();
    let other_namespace = other_master_key.namespace_id();
    globals.cloud.set_master_key_backup(other_namespace.clone(), vec![1, 2, 3]);
    globals.cloud.set_wallet_files(
        other_namespace,
        vec![
            wallet_filename_from_record_id(&local_record_id),
            wallet_filename_from_record_id("missing-local-wallet"),
        ],
    );

    let summary =
        manager.other_backup_summary(&CloudStorage::global_explicit_client()).await.unwrap();
    assert_eq!(summary.namespace_count, 1);
    assert_eq!(summary.wallet_count, 1);

    globals.cloud.set_wallet_files(
        current_namespace,
        vec![wallet_filename_from_record_id("missing-local-wallet")],
    );

    let summary =
        manager.other_backup_summary(&CloudStorage::global_explicit_client()).await.unwrap();
    assert_eq!(summary.namespace_count, 1);
    assert_eq!(summary.wallet_count, 1);
}

#[tokio::test(flavor = "current_thread")]
async fn other_backup_summary_fails_closed_when_wallet_listing_is_missing() {
    let _guard = async_test_lock().lock().await;
    cove_tokio::init();
    let globals = test_globals();
    let manager = init_manager();
    configure_enabled_cloud_backup(&manager, globals, 0);

    let other_master_key = cove_cspp::master_key::MasterKey::generate();
    let other_namespace = other_master_key.namespace_id();
    globals.cloud.set_master_key_backup(other_namespace.clone(), vec![1, 2, 3]);
    globals.cloud.fail_list_wallet_files_for_namespace(
        other_namespace,
        CloudStorageError::NotFound("wallet files missing".into()),
    );

    let error =
        manager.other_backup_summary(&CloudStorage::global_explicit_client()).await.unwrap_err();

    assert!(error.to_string().contains("wallet files missing"), "{error}");
}

#[tokio::test(flavor = "current_thread")]
async fn detail_refresh_keeps_current_detail_when_other_namespace_inspection_fails() {
    let _guard = async_test_lock().lock().await;
    cove_tokio::init();
    let globals = test_globals();
    let manager = init_manager();
    configure_enabled_cloud_backup(&manager, globals, 0);

    let current_namespace = CloudBackupKeychain::global().namespace_id().unwrap();
    globals.cloud.set_wallet_files(current_namespace, Vec::new());

    let other_master_key = cove_cspp::master_key::MasterKey::generate();
    let other_namespace = other_master_key.namespace_id();
    globals.cloud.set_master_key_backup(other_namespace.clone(), vec![1, 2, 3]);
    globals
        .cloud
        .fail_master_key_download_offline(other_namespace, "offline while inspecting namespace");

    let Some(CloudBackupDetailResult::Success(detail)) =
        manager.refresh_cloud_backup_detail().await
    else {
        panic!("expected usable current cloud backup detail");
    };

    let CloudBackupOtherBackupsState::LoadFailed { error } = detail.other_backups else {
        panic!("expected isolated other-backups failure");
    };

    assert!(error.contains("Reconnect to the internet"), "{error}");
    assert!(detail.up_to_date.is_empty());
    assert!(detail.needs_sync.is_empty());
}

#[tokio::test(flavor = "current_thread")]
async fn recover_other_backups_keeps_current_passkey_metadata() {
    let _guard = async_test_lock().lock().await;
    cove_tokio::init();
    let globals = test_globals();
    let manager = init_manager();
    configure_enabled_cloud_backup(&manager, globals, 0);
    globals.cloud.set_reflect_uploaded_wallets_in_listing(true);

    let current_namespace = CloudBackupKeychain::global().namespace_id().unwrap();
    CloudBackupKeychain::global()
        .save_passkey_and_namespace(&[9, 8, 7], [6; 32], &current_namespace)
        .unwrap();

    let prf_key = [7u8; 32];
    let other_master_key = cove_cspp::master_key::MasterKey::generate();
    let other_namespace = other_master_key.namespace_id();
    let encrypted_master =
        cove_cspp::master_key_crypto::encrypt_master_key(&other_master_key, &prf_key, &[9; 32])
            .unwrap();
    globals.cloud.set_master_key_backup(
        other_namespace.clone(),
        serde_json::to_vec(&encrypted_master).unwrap(),
    );
    globals.passkey.set_discover_result(Ok(DiscoveredPasskeyResult {
        prf_output: prf_key.to_vec(),
        credential_id: vec![1, 2, 3],
    }));

    let wallet = xpub_only_wallet_metadata();
    Keychain::global().save_wallet_xpub(&wallet.id, sample_xpub(&wallet).parse().unwrap()).unwrap();
    let record_id = cove_cspp::backup_data::wallet_record_id(wallet.id.as_ref());
    globals.cloud.set_wallet_backup(
        other_namespace.clone(),
        record_id.clone(),
        encrypted_wallet_backup_bytes(&wallet, &other_master_key, "other-revision", 1).await,
    );
    globals.cloud.set_wallet_files(
        other_namespace.clone(),
        vec![wallet_filename_from_record_id(&record_id)],
    );

    let current_namespace_list_attempt_count =
        globals.cloud.list_wallet_files_attempt_count_for_namespace(&current_namespace);
    let report = manager.do_recover_other_backups().await.unwrap();

    assert_eq!(report.wallets_restored, 1);
    assert_eq!(report.wallets_failed, 0);
    assert_eq!(
        globals.cloud.list_wallet_files_attempt_count_for_namespace(&current_namespace),
        current_namespace_list_attempt_count + 3,
    );
    assert_eq!(globals.cloud.uploaded_wallet_backup_count(), 1);
    assert!(!globals.cloud.has_namespace(&other_namespace));
    assert_eq!(CloudBackupKeychain::global().namespace_id(), Some(current_namespace.clone()));
    assert_eq!(
        globals.keychain.get_entry(CSPP_NAMESPACE_ID_KEY).as_deref(),
        Some(current_namespace.as_str())
    );
    assert_eq!(CloudBackupKeychain::global().load_credential_id(), Some(vec![9, 8, 7]));
    assert!(globals.keychain.get_entry(CSPP_PRF_SALT_KEY).is_some());

    let summary =
        manager.other_backup_summary(&CloudStorage::global_explicit_client()).await.unwrap();
    assert_eq!(summary.namespace_count, 0);
    assert_eq!(summary.wallet_count, 0);
}

#[tokio::test(flavor = "current_thread")]
async fn recover_other_backups_current_namespace_not_found_fails_closed_without_source_delete() {
    let _guard = async_test_lock().lock().await;
    cove_tokio::init();
    let globals = test_globals();
    let manager = init_manager();
    configure_enabled_cloud_backup(&manager, globals, 0);

    let current_namespace = CloudBackupKeychain::global().namespace_id().unwrap();
    globals.cloud.fail_list_wallet_files_for_namespace(
        current_namespace,
        CloudStorageError::NotFound("current wallet files missing".into()),
    );

    let prf_key = [7u8; 32];
    let other_master_key = cove_cspp::master_key::MasterKey::generate();
    let other_namespace = other_master_key.namespace_id();
    let encrypted_master =
        cove_cspp::master_key_crypto::encrypt_master_key(&other_master_key, &prf_key, &[9; 32])
            .unwrap();
    globals.cloud.set_master_key_backup(
        other_namespace.clone(),
        serde_json::to_vec(&encrypted_master).unwrap(),
    );
    globals.passkey.set_discover_result(Ok(DiscoveredPasskeyResult {
        prf_output: prf_key.to_vec(),
        credential_id: vec![1, 2, 3],
    }));

    let wallet = xpub_only_wallet_metadata();
    Keychain::global().save_wallet_xpub(&wallet.id, sample_xpub(&wallet).parse().unwrap()).unwrap();
    let record_id = cove_cspp::backup_data::wallet_record_id(wallet.id.as_ref());
    globals.cloud.set_wallet_backup(
        other_namespace.clone(),
        record_id.clone(),
        encrypted_wallet_backup_bytes(&wallet, &other_master_key, "other-revision", 1).await,
    );
    globals.cloud.set_wallet_files(
        other_namespace.clone(),
        vec![wallet_filename_from_record_id(&record_id)],
    );
    let delete_attempt_count = globals.cloud.delete_namespace_attempt_count();

    let error = manager.do_recover_other_backups().await.unwrap_err();

    assert!(error.to_string().contains("current wallet files missing"), "{error}");
    assert!(globals.cloud.has_namespace(&other_namespace));
    assert_eq!(globals.cloud.delete_namespace_attempt_count(), delete_attempt_count);
}

#[tokio::test(flavor = "current_thread")]
async fn recover_other_backups_keeps_partially_moved_namespace() {
    let _guard = async_test_lock().lock().await;
    cove_tokio::init();
    let globals = test_globals();
    let manager = init_manager();
    configure_enabled_cloud_backup(&manager, globals, 0);
    globals.cloud.set_reflect_uploaded_wallets_in_listing(true);

    let prf_key = [7u8; 32];
    let other_master_key = cove_cspp::master_key::MasterKey::generate();
    let other_namespace = other_master_key.namespace_id();
    let encrypted_master =
        cove_cspp::master_key_crypto::encrypt_master_key(&other_master_key, &prf_key, &[9; 32])
            .unwrap();
    globals.cloud.set_master_key_backup(
        other_namespace.clone(),
        serde_json::to_vec(&encrypted_master).unwrap(),
    );
    globals.passkey.set_discover_result(Ok(DiscoveredPasskeyResult {
        prf_output: prf_key.to_vec(),
        credential_id: vec![1, 2, 3],
    }));

    let restored_wallet = xpub_only_wallet_metadata();
    Keychain::global()
        .save_wallet_xpub(&restored_wallet.id, sample_xpub(&restored_wallet).parse().unwrap())
        .unwrap();
    let restored_record_id = cove_cspp::backup_data::wallet_record_id(restored_wallet.id.as_ref());
    let missing_wallet = xpub_only_wallet_metadata();
    let missing_record_id = cove_cspp::backup_data::wallet_record_id(missing_wallet.id.as_ref());

    globals.cloud.set_wallet_backup(
        other_namespace.clone(),
        restored_record_id.clone(),
        encrypted_wallet_backup_bytes(&restored_wallet, &other_master_key, "other-revision", 1)
            .await,
    );
    globals.cloud.set_wallet_files(
        other_namespace.clone(),
        vec![
            wallet_filename_from_record_id(&restored_record_id),
            wallet_filename_from_record_id(&missing_record_id),
        ],
    );

    let report = manager.do_recover_other_backups().await.unwrap();

    assert_eq!(report.wallets_restored, 1);
    assert_eq!(report.wallets_failed, 1);
    assert!(globals.cloud.has_namespace(&other_namespace));

    let summary =
        manager.other_backup_summary(&CloudStorage::global_explicit_client()).await.unwrap();
    assert_eq!(summary.namespace_count, 1);
    assert_eq!(summary.wallet_count, 1);
}

#[tokio::test(flavor = "current_thread")]
async fn recover_other_backups_keeps_namespace_when_current_upload_fails() {
    let _guard = async_test_lock().lock().await;
    cove_tokio::init();
    let globals = test_globals();
    let manager = init_manager();
    configure_enabled_cloud_backup(&manager, globals, 0);
    globals.cloud.fail_wallet_backup_upload("upload failed");

    let prf_key = [7u8; 32];
    let other_master_key = cove_cspp::master_key::MasterKey::generate();
    let other_namespace = other_master_key.namespace_id();
    let encrypted_master =
        cove_cspp::master_key_crypto::encrypt_master_key(&other_master_key, &prf_key, &[9; 32])
            .unwrap();
    globals.cloud.set_master_key_backup(
        other_namespace.clone(),
        serde_json::to_vec(&encrypted_master).unwrap(),
    );
    globals.passkey.set_discover_result(Ok(DiscoveredPasskeyResult {
        prf_output: prf_key.to_vec(),
        credential_id: vec![1, 2, 3],
    }));

    let wallet = xpub_only_wallet_metadata();
    Keychain::global().save_wallet_xpub(&wallet.id, sample_xpub(&wallet).parse().unwrap()).unwrap();
    let record_id = cove_cspp::backup_data::wallet_record_id(wallet.id.as_ref());
    globals.cloud.set_wallet_backup(
        other_namespace.clone(),
        record_id.clone(),
        encrypted_wallet_backup_bytes(&wallet, &other_master_key, "other-revision", 1).await,
    );
    globals.cloud.set_wallet_files(
        other_namespace.clone(),
        vec![wallet_filename_from_record_id(&record_id)],
    );

    let result = manager.do_recover_other_backups().await;

    assert!(result.is_err());
    assert!(globals.cloud.has_namespace(&other_namespace));
}

#[tokio::test(flavor = "current_thread")]
async fn recover_other_backups_returns_offline_when_wallet_download_is_offline() {
    let _guard = async_test_lock().lock().await;
    cove_tokio::init();
    let globals = test_globals();
    let manager = init_manager();
    configure_enabled_cloud_backup(&manager, globals, 0);

    let prf_key = [7u8; 32];
    let other_master_key = cove_cspp::master_key::MasterKey::generate();
    let other_namespace = other_master_key.namespace_id();
    let encrypted_master =
        cove_cspp::master_key_crypto::encrypt_master_key(&other_master_key, &prf_key, &[9; 32])
            .unwrap();
    globals.cloud.set_master_key_backup(
        other_namespace.clone(),
        serde_json::to_vec(&encrypted_master).unwrap(),
    );
    globals.passkey.set_discover_result(Ok(DiscoveredPasskeyResult {
        prf_output: prf_key.to_vec(),
        credential_id: vec![1, 2, 3],
    }));

    let wallet = xpub_only_wallet_metadata();
    let record_id = cove_cspp::backup_data::wallet_record_id(wallet.id.as_ref());
    globals.cloud.set_wallet_files(
        other_namespace.clone(),
        vec![wallet_filename_from_record_id(&record_id)],
    );
    globals.cloud.fail_wallet_backup_download_offline(
        other_namespace,
        record_id,
        "offline while downloading wallet",
    );

    let result = manager.do_recover_other_backups().await;

    match result {
        Err(CloudBackupError::Offline(message)) => {
            assert_eq!(
                message,
                "Reconnect to the internet, then try recovering the other cloud backups again"
            );
        }
        Ok(report) => panic!(
            "expected offline error, got report with {} failed wallet(s)",
            report.wallets_failed
        ),
        Err(error) => panic!("expected offline error, got {error:?}"),
    }
}

#[tokio::test(flavor = "current_thread")]
async fn recover_other_backups_returns_offline_when_namespace_inspection_is_offline() {
    let _guard = async_test_lock().lock().await;
    cove_tokio::init();
    let globals = test_globals();
    let manager = init_manager();
    configure_enabled_cloud_backup(&manager, globals, 0);

    let other_master_key = cove_cspp::master_key::MasterKey::generate();
    let other_namespace = other_master_key.namespace_id();
    let encrypted_master =
        cove_cspp::master_key_crypto::encrypt_master_key(&other_master_key, &[7; 32], &[9; 32])
            .unwrap();
    globals.cloud.set_master_key_backup(
        other_namespace.clone(),
        serde_json::to_vec(&encrypted_master).unwrap(),
    );
    globals
        .cloud
        .fail_master_key_download_offline(other_namespace, "offline while inspecting namespace");

    let result = manager.do_recover_other_backups().await;

    match result {
        Err(CloudBackupError::Offline(message)) => {
            assert_eq!(
                message,
                "Reconnect to the internet, then try recovering the other cloud backups again"
            );
        }
        Ok(report) => panic!(
            "expected offline error, got report with {} restored wallet(s)",
            report.wallets_restored
        ),
        Err(error) => panic!("expected offline error, got {error:?}"),
    }
}

#[tokio::test(flavor = "current_thread")]
async fn delete_other_backups_removes_only_non_current_namespaces() {
    let _guard = async_test_lock().lock().await;
    cove_tokio::init();
    let globals = test_globals();
    let manager = init_manager();
    configure_enabled_cloud_backup(&manager, globals, 0);

    let current_namespace = CloudBackupKeychain::global().namespace_id().unwrap();
    globals.cloud.set_master_key_backup(current_namespace.clone(), vec![1, 2, 3]);
    globals.cloud.set_wallet_files(
        current_namespace.clone(),
        vec![wallet_filename_from_record_id("current-wallet")],
    );

    let other_master_key = cove_cspp::master_key::MasterKey::generate();
    let other_namespace = other_master_key.namespace_id();
    let encrypted_master =
        cove_cspp::master_key_crypto::encrypt_master_key(&other_master_key, &[7; 32], &[9; 32])
            .unwrap();
    globals.cloud.set_master_key_backup(
        other_namespace.clone(),
        serde_json::to_vec(&encrypted_master).unwrap(),
    );
    globals.cloud.set_wallet_files(
        other_namespace.clone(),
        vec![wallet_filename_from_record_id("other-wallet")],
    );

    manager.do_delete_other_backups().await.unwrap();

    assert!(globals.cloud.has_namespace(&current_namespace));
    assert!(!globals.cloud.has_namespace(&other_namespace));
    assert_eq!(globals.cloud.deleted_namespace_policies(), vec![CloudAccessPolicy::ConsentAllowed]);
}

#[tokio::test(flavor = "current_thread")]
async fn delete_other_backups_returns_offline_when_namespace_inspection_is_offline() {
    let _guard = async_test_lock().lock().await;
    cove_tokio::init();
    let globals = test_globals();
    let manager = init_manager();
    configure_enabled_cloud_backup(&manager, globals, 0);

    let current_namespace = CloudBackupKeychain::global().namespace_id().unwrap();
    globals.cloud.set_master_key_backup(current_namespace.clone(), vec![1, 2, 3]);

    let other_master_key = cove_cspp::master_key::MasterKey::generate();
    let other_namespace = other_master_key.namespace_id();
    let encrypted_master =
        cove_cspp::master_key_crypto::encrypt_master_key(&other_master_key, &[7; 32], &[9; 32])
            .unwrap();
    globals.cloud.set_master_key_backup(
        other_namespace.clone(),
        serde_json::to_vec(&encrypted_master).unwrap(),
    );
    globals.cloud.fail_master_key_download_offline(
        other_namespace.clone(),
        "offline while inspecting namespace",
    );

    let result = manager.do_delete_other_backups().await;

    match result {
        Err(CloudBackupError::Offline(message)) => {
            assert_eq!(
                message,
                "Reconnect to the internet, then try deleting the other cloud backups again"
            );
        }
        Ok(()) => panic!("expected offline error"),
        Err(error) => panic!("expected offline error, got {error:?}"),
    }
    assert!(globals.cloud.has_namespace(&other_namespace));
}
