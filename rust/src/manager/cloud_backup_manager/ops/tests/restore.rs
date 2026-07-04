use super::*;

#[tokio::test(flavor = "current_thread")]
async fn restore_downloaded_wallet_does_not_reupload_wallet_or_mutate_backup_counts() {
    let _guard = async_test_lock().lock().await;
    cove_tokio::init();
    let globals = test_globals();
    let manager = init_manager();
    configure_enabled_cloud_backup(&manager, globals, 5);

    let metadata = xpub_only_wallet_metadata();
    let wallet = DownloadedWalletBackup {
        metadata: metadata.clone(),
        entry: WalletEntry {
            wallet_id: metadata.id.to_string(),
            secret: WalletSecret::WatchOnly,
            metadata: serde_json::to_value(&metadata).unwrap(),
            descriptors: None,
            xpub: Some(sample_xpub(&metadata)),
            wallet_mode: CloudWalletMode::Main,
            labels_zstd_jsonl: None,
            labels_count: 0,
            labels_hash: None,
            labels_uncompressed_size: None,
            content_revision_hash: "test-content-hash".to_string(),
            updated_at: 42,
        },
    };

    WalletRestoreSession::new(crate::wallet_identity::ExistingWalletIdentitySet::default())
        .restore_downloaded(&wallet)
        .unwrap();

    assert_eq!(globals.cloud.uploaded_wallet_backup_count(), 0);
    assert_eq!(Database::global().cloud_backup_state.get().unwrap().wallet_count(), Some(5));
    assert!(Database::global().cloud_blob_sync_states.list().unwrap().is_empty());
    assert!(
        Database::global()
            .wallets()
            .get(&metadata.id, metadata.network, WalletMode::Main)
            .unwrap()
            .is_some()
    );
}

#[tokio::test(flavor = "current_thread")]
async fn restore_downloaded_wallet_restores_labels_without_marking_cloud_backup_dirty() {
    let _guard = async_test_lock().lock().await;
    cove_tokio::init();
    let globals = test_globals();
    let manager = init_manager();
    configure_enabled_cloud_backup(&manager, globals, 5);

    let locked_output_ref = "d97bf8892657980426c879e4ab2001f09342f1ab61cfa602741a7715a3d60290:0";
    let labels_jsonl = format!(
        "{}\n{{\"type\":\"output\",\"ref\":\"{locked_output_ref}\",\"spendable\":false}}",
        sample_labels_jsonl()
    );
    let metadata = xpub_only_wallet_metadata();
    let wallet = DownloadedWalletBackup {
        metadata: metadata.clone(),
        entry: wallet_entry_with_labels(&metadata, Some(&labels_jsonl)),
    };

    let outcome =
        WalletRestoreSession::new(crate::wallet_identity::ExistingWalletIdentitySet::default())
            .restore_downloaded(&wallet)
            .unwrap();

    let WalletRestoreOutcome::Restored { labels_warning } = outcome else {
        panic!("expected restored wallet");
    };
    assert!(labels_warning.is_none());
    assert_eq!(globals.cloud.uploaded_wallet_backup_count(), 0);
    assert_eq!(Database::global().cloud_backup_state.get().unwrap().wallet_count(), Some(5));
    assert!(Database::global().cloud_blob_sync_states.list().unwrap().is_empty());

    let exported = LabelManager::new(metadata.id.clone()).export().await.unwrap();
    assert!(exported.contains("\"label\":\"last txn received\""));
    assert!(exported.contains(&format!("\"ref\":\"{locked_output_ref}\"")));
    assert!(exported.contains("\"spendable\":false"));
}

#[tokio::test(flavor = "current_thread")]
async fn restore_from_local_master_key_propagates_store_read_errors() {
    let _guard = async_test_lock().lock().await;
    cove_tokio::init();
    let globals = test_globals();
    globals.reset();

    let store = Arc::new(MockStore::default());
    let store_handle = MockStoreHandle(store.clone());
    let cspp = cove_cspp::Cspp::new(store_handle);
    let expected = cove_cspp::master_key::MasterKey::generate();
    cspp.save_master_key(&expected).unwrap();
    let key_to_corrupt =
        store.entries.lock().keys().next().cloned().expect("saved master key entry");
    store.entries.lock().insert(key_to_corrupt, "not-a-valid-master-key".into());

    let error =
        match try_restore_from_local_master_key(&CloudStorage::global_explicit_client(), &cspp)
            .await
        {
            Ok(_) => panic!("expected local master key read failure"),
            Err(error) => error,
        };

    assert!(matches!(
        error,
        CloudBackupError::Internal(message)
            if message.starts_with("loading master key from store:")
    ));
}

#[tokio::test(flavor = "current_thread")]
async fn restore_counts_unsupported_wallet_versions_as_failures() {
    let _guard = async_test_lock().lock().await;
    cove_tokio::init();
    let globals = test_globals();
    let manager = init_manager();

    reset_cloud_backup_test_state(&manager, globals);

    let master_key = cove_cspp::master_key::MasterKey::generate();
    let namespace = master_key.namespace_id();
    let encrypted_master =
        cove_cspp::master_key_crypto::encrypt_master_key(&master_key, &[7; 32], &[9; 32]).unwrap();
    globals
        .cloud
        .set_master_key_backup(namespace.clone(), serde_json::to_vec(&encrypted_master).unwrap());
    cove_cspp::Cspp::new(Keychain::global().clone()).save_master_key(&master_key).unwrap();

    let supported_wallet = xpub_only_wallet_metadata();
    let unsupported_wallet = xpub_only_wallet_metadata();
    Keychain::global()
        .save_wallet_xpub(&supported_wallet.id, sample_xpub(&supported_wallet).parse().unwrap())
        .unwrap();
    Keychain::global()
        .save_wallet_xpub(&unsupported_wallet.id, sample_xpub(&unsupported_wallet).parse().unwrap())
        .unwrap();

    let supported_record_id =
        cove_cspp::backup_data::wallet_record_id(supported_wallet.id.as_ref());
    let unsupported_record_id =
        cove_cspp::backup_data::wallet_record_id(unsupported_wallet.id.as_ref());
    globals.cloud.set_wallet_backup(
        namespace.clone(),
        supported_record_id.clone(),
        encrypted_wallet_backup_bytes(&supported_wallet, &master_key, "supported-revision", 1)
            .await,
    );
    globals.cloud.set_wallet_backup(
        namespace.clone(),
        unsupported_record_id.clone(),
        encrypted_wallet_backup_bytes(&unsupported_wallet, &master_key, "unsupported-revision", 2)
            .await,
    );
    globals.cloud.set_wallet_files(
        namespace,
        vec![
            wallet_filename_from_record_id(&supported_record_id),
            wallet_filename_from_record_id(&unsupported_record_id),
        ],
    );

    let operation = new_restore_operation_for_test(&manager).await;
    let report = operation.restore_from_cloud_backup(&manager).await.unwrap();
    assert_eq!(report.wallets_restored, 1);
    assert_eq!(report.wallets_failed, 1);
    assert_eq!(report.failed_wallet_errors.len(), 1);
    assert!(report.failed_wallet_errors[0].contains("unsupported wallet backup version 2"));
    assert_eq!(Database::global().cloud_backup_state.get().unwrap().wallet_count(), Some(1));
    assert!(
        Database::global()
            .wallets()
            .get(&supported_wallet.id, supported_wallet.network, supported_wallet.wallet_mode,)
            .unwrap()
            .is_some()
    );
    assert!(
            Database::global()
                .wallets()
                .get(
                    &unsupported_wallet.id,
                    unsupported_wallet.network,
                    unsupported_wallet.wallet_mode,
                )
                .unwrap()
                .is_none()
        );
}

#[tokio::test(flavor = "current_thread")]
async fn restore_queues_reupload_when_cloud_upload_confirmation_lags() {
    let _guard = async_test_lock().lock().await;
    cove_tokio::init();
    let globals = test_globals();
    let manager = init_manager();

    reset_cloud_backup_test_state(&manager, globals);

    let master_key = cove_cspp::master_key::MasterKey::generate();
    let namespace = master_key.namespace_id();
    let encrypted_master =
        cove_cspp::master_key_crypto::encrypt_master_key(&master_key, &[7; 32], &[9; 32]).unwrap();
    globals
        .cloud
        .set_master_key_backup(namespace.clone(), serde_json::to_vec(&encrypted_master).unwrap());
    cove_cspp::Cspp::new(Keychain::global().clone()).save_master_key(&master_key).unwrap();

    let wallet = xpub_only_wallet_metadata();
    Keychain::global().save_wallet_xpub(&wallet.id, sample_xpub(&wallet).parse().unwrap()).unwrap();

    let record_id = wallet_record_id(wallet.id.as_ref());
    globals.cloud.set_wallet_backup(
        namespace.clone(),
        record_id.clone(),
        encrypted_wallet_backup_bytes(&wallet, &master_key, "restored-revision", 1).await,
    );
    globals.cloud.set_wallet_files(namespace, vec![wallet_filename_from_record_id(&record_id)]);
    globals.cloud.set_uploaded_wallets_pending_confirmation(true);

    let operation = new_restore_operation_for_test(&manager).await;
    let report = operation.restore_from_cloud_backup(&manager).await.unwrap();

    assert_eq!(report.wallets_restored, 1);
    assert_eq!(
        Database::global().cloud_backup_state.get().unwrap().status(),
        PersistedCloudBackupStatus::Enabled
    );
    let sync_state = Database::global().cloud_blob_sync_states.get(&record_id).unwrap();
    assert!(
        matches!(
            sync_state,
            Some(PersistedCloudBlobSyncState { state: PersistedCloudBlobState::Dirty(_), .. })
        ),
        "unexpected sync state: {sync_state:?}"
    );
    assert_eq!(globals.cloud.wallet_backup_upload_attempt_count(), 0);
}

#[tokio::test(flavor = "current_thread")]
async fn restore_with_one_passkey_restores_wallets_from_all_matching_namespaces() {
    let _guard = async_test_lock().lock().await;
    cove_tokio::init();
    let globals = test_globals();
    let manager = init_manager();

    reset_cloud_backup_test_state(&manager, globals);

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

    let first_record_id = cove_cspp::backup_data::wallet_record_id(first_wallet.id.as_ref());
    let second_record_id = cove_cspp::backup_data::wallet_record_id(second_wallet.id.as_ref());
    globals.cloud.set_wallet_backup(
        first_namespace.clone(),
        first_record_id.clone(),
        encrypted_wallet_backup_bytes(&first_wallet, &first_master_key, "first-revision", 1).await,
    );
    globals.cloud.set_wallet_backup(
        second_namespace.clone(),
        second_record_id.clone(),
        encrypted_wallet_backup_bytes(&second_wallet, &second_master_key, "second-revision", 1)
            .await,
    );
    globals
        .cloud
        .set_wallet_files(first_namespace, vec![wallet_filename_from_record_id(&first_record_id)]);
    globals.cloud.set_wallet_files(
        second_namespace,
        vec![wallet_filename_from_record_id(&second_record_id)],
    );

    let operation = new_restore_operation_for_test(&manager).await;
    let report = operation.restore_from_cloud_backup(&manager).await.unwrap();
    assert_eq!(report.wallets_restored, 2);
    assert_eq!(report.wallets_failed, 0);
    assert!(report.failed_wallet_errors.is_empty(), "{:?}", report.failed_wallet_errors);
    assert_eq!(Database::global().cloud_backup_state.get().unwrap().wallet_count(), Some(2));
    let active_namespace = CloudBackupKeychain::global().namespace_id().unwrap();
    for record_id in [&first_record_id, &second_record_id] {
        let sync_state = Database::global().cloud_blob_sync_states.get(record_id).unwrap().unwrap();
        assert_eq!(sync_state.namespace_id, active_namespace);
        assert!(
            matches!(sync_state.state, PersistedCloudBlobState::Dirty(_)),
            "unexpected sync state: {sync_state:?}"
        );
    }
}

#[tokio::test(flavor = "current_thread")]
async fn restore_missing_wallet_listing_fails_closed_without_finalizing_empty_state() {
    let _guard = async_test_lock().lock().await;
    cove_tokio::init();
    let globals = test_globals();
    let manager = init_manager();

    configure_enabled_cloud_backup(&manager, globals, 7);
    let existing_namespace = CloudBackupKeychain::global().namespace_id().unwrap();

    let prf_key = [7u8; 32];
    let master_key = cove_cspp::master_key::MasterKey::generate();
    let namespace = master_key.namespace_id();
    let encrypted =
        cove_cspp::master_key_crypto::encrypt_master_key(&master_key, &prf_key, &[9; 32]).unwrap();

    globals.cloud.set_master_key_backup(namespace.clone(), serde_json::to_vec(&encrypted).unwrap());
    globals.cloud.fail_list_wallet_files_for_namespace(
        namespace,
        CloudStorageError::NotFound("wallet files missing".into()),
    );
    globals.passkey.set_discover_result(Ok(DiscoveredPasskeyResult {
        prf_output: prf_key.to_vec(),
        credential_id: vec![1, 2, 3],
    }));
    globals.passkey.set_authenticate_result(Ok(prf_key.to_vec()));

    let operation = new_restore_operation_for_test(&manager).await;
    let error = operation.restore_from_cloud_backup(&manager).await.unwrap_err();

    assert!(error.to_string().contains("wallet files missing"), "{error}");
    assert_eq!(Database::global().cloud_backup_state.get().unwrap().wallet_count(), Some(7));
    assert_eq!(CloudBackupKeychain::global().namespace_id(), Some(existing_namespace));
}

#[tokio::test(flavor = "current_thread")]
async fn restore_activation_upload_failure_keeps_restore_successful_and_queues_upload() {
    let _guard = async_test_lock().lock().await;
    cove_tokio::init();
    let globals = test_globals();
    let manager = init_manager();

    reset_cloud_backup_test_state(&manager, globals);

    let wallet = xpub_only_wallet_metadata();
    let record_id = cove_cspp::backup_data::wallet_record_id(wallet.id.as_ref());
    let prf_key = [7u8; 32];
    let master_key = cove_cspp::master_key::MasterKey::generate();
    let namespace = master_key.namespace_id();
    let encrypted =
        cove_cspp::master_key_crypto::encrypt_master_key(&master_key, &prf_key, &[9; 32]).unwrap();

    cove_cspp::Cspp::new(Keychain::global().clone()).save_master_key(&master_key).unwrap();
    Keychain::global().save_wallet_xpub(&wallet.id, sample_xpub(&wallet).parse().unwrap()).unwrap();
    globals.cloud.set_master_key_backup(namespace.clone(), serde_json::to_vec(&encrypted).unwrap());
    globals.cloud.set_wallet_backup(
        namespace.clone(),
        record_id.clone(),
        encrypted_wallet_backup_bytes(&wallet, &master_key, "restore-revision", 1).await,
    );
    globals
        .cloud
        .set_wallet_files(namespace.clone(), vec![wallet_filename_from_record_id(&record_id)]);
    globals.cloud.fail_wallet_backup_upload("activation upload failed");
    globals.passkey.set_discover_result(Ok(DiscoveredPasskeyResult {
        prf_output: prf_key.to_vec(),
        credential_id: vec![1, 2, 3],
    }));
    globals.passkey.set_authenticate_result(Ok(prf_key.to_vec()));

    let (sender, receiver) = flume::bounded(1);
    call!(manager.supervisor.start_restore_from_cloud_backup_with_events(sender)).await.unwrap();

    let report = tokio::time::timeout(Duration::from_secs(1), async {
        loop {
            match receiver.recv_async().await.expect("receive restore event") {
                CloudBackupRestoreEvent::Progress(_) => {}
                CloudBackupRestoreEvent::Complete(report) => break report,
                CloudBackupRestoreEvent::Failed(message) => {
                    panic!("restore should complete before background upload failure: {message}");
                }
                other => panic!("expected restore completion event, got {other:?}"),
            }
        }
    })
    .await
    .expect("restore completion event");

    assert_eq!(report.wallets_restored, 1);
    wait_for_test_condition(
        Duration::from_secs(5),
        "background reupload should fail after restore and remain queued for retry",
        || {
            let upload_attempted = globals.cloud.wallet_backup_upload_attempt_count() > 0;
            let retryable_failure = matches!(
                Database::global().cloud_blob_sync_states.get(&record_id).unwrap(),
                Some(PersistedCloudBlobSyncState {
                    state: PersistedCloudBlobState::Failed(CloudBlobFailedState {
                        retryable: true,
                        ..
                    }),
                    ..
                })
            );

            upload_attempted && retryable_failure
        },
    )
    .await;
    assert_eq!(
        Database::global().cloud_backup_state.get().unwrap().status(),
        PersistedCloudBackupStatus::Enabled
    );
    assert_eq!(CloudBackupKeychain::global().namespace_id(), Some(namespace.clone()));
    let sync_state = Database::global().cloud_blob_sync_states.get(&record_id).unwrap().unwrap();
    assert_eq!(sync_state.namespace_id, namespace);
    assert!(
        matches!(
            sync_state.state,
            PersistedCloudBlobState::Failed(CloudBlobFailedState { retryable: true, .. })
        ),
        "unexpected sync state: {sync_state:?}"
    );
}

#[tokio::test(flavor = "current_thread")]
async fn restore_all_duplicate_wallets_preserves_existing_configured_state() {
    let _guard = async_test_lock().lock().await;
    cove_tokio::init();
    let globals = test_globals();
    let manager = init_manager();

    configure_enabled_cloud_backup(&manager, globals, 7);
    let existing_namespace = CloudBackupKeychain::global().namespace_id().unwrap();
    let duplicate_wallet = xpub_only_wallet_metadata();
    persist_xpub_wallets(vec![duplicate_wallet.clone()]);

    let prf_key = [7u8; 32];
    let master_key = cove_cspp::master_key::MasterKey::generate();
    let namespace = master_key.namespace_id();
    let encrypted =
        cove_cspp::master_key_crypto::encrypt_master_key(&master_key, &prf_key, &[9; 32]).unwrap();
    let record_id = cove_cspp::backup_data::wallet_record_id(duplicate_wallet.id.as_ref());
    globals.cloud.set_master_key_backup(namespace.clone(), serde_json::to_vec(&encrypted).unwrap());
    globals.cloud.set_wallet_backup(
        namespace.clone(),
        record_id.clone(),
        encrypted_wallet_backup_bytes(&duplicate_wallet, &master_key, "duplicate-revision", 1)
            .await,
    );
    globals.cloud.set_wallet_files(namespace, vec![wallet_filename_from_record_id(&record_id)]);
    globals.passkey.set_discover_result(Ok(DiscoveredPasskeyResult {
        prf_output: prf_key.to_vec(),
        credential_id: vec![1, 2, 3],
    }));
    globals.passkey.set_authenticate_result(Ok(prf_key.to_vec()));

    let operation = new_restore_operation_for_test(&manager).await;
    let report = operation.restore_from_cloud_backup(&manager).await.unwrap();

    assert_eq!(report.wallets_restored, 0);
    assert_eq!(report.wallets_failed, 0);
    assert_eq!(
        Database::global().cloud_backup_state.get().unwrap().status(),
        PersistedCloudBackupStatus::Enabled
    );
    assert_eq!(Database::global().cloud_backup_state.get().unwrap().wallet_count(), Some(7));
    assert_eq!(CloudBackupKeychain::global().namespace_id(), Some(existing_namespace));
}

#[tokio::test(flavor = "current_thread")]
async fn restore_all_duplicate_wallets_activates_namespace_when_persisted_state_is_empty() {
    let _guard = async_test_lock().lock().await;
    cove_tokio::init();
    let globals = test_globals();
    let manager = init_manager();

    reset_cloud_backup_test_state(&manager, globals);
    let duplicate_wallet = xpub_only_wallet_metadata();
    persist_xpub_wallets(vec![duplicate_wallet.clone()]);

    let prf_key = [7u8; 32];
    let master_key = cove_cspp::master_key::MasterKey::generate();
    let namespace = master_key.namespace_id();
    let encrypted =
        cove_cspp::master_key_crypto::encrypt_master_key(&master_key, &prf_key, &[9; 32]).unwrap();
    let record_id = cove_cspp::backup_data::wallet_record_id(duplicate_wallet.id.as_ref());
    globals.cloud.set_master_key_backup(namespace.clone(), serde_json::to_vec(&encrypted).unwrap());
    globals.cloud.set_wallet_backup(
        namespace.clone(),
        record_id.clone(),
        encrypted_wallet_backup_bytes(&duplicate_wallet, &master_key, "duplicate-revision", 1)
            .await,
    );
    globals
        .cloud
        .set_wallet_files(namespace.clone(), vec![wallet_filename_from_record_id(&record_id)]);
    globals.passkey.set_discover_result(Ok(DiscoveredPasskeyResult {
        prf_output: prf_key.to_vec(),
        credential_id: vec![1, 2, 3],
    }));
    globals.passkey.set_authenticate_result(Ok(prf_key.to_vec()));

    let operation = new_restore_operation_for_test(&manager).await;
    let report = operation.restore_from_cloud_backup(&manager).await.unwrap();

    assert_eq!(report.wallets_restored, 0);
    assert_eq!(report.wallets_failed, 0);
    assert_eq!(
        Database::global().cloud_backup_state.get().unwrap().status(),
        PersistedCloudBackupStatus::Enabled
    );
    assert_eq!(Database::global().cloud_backup_state.get().unwrap().wallet_count(), Some(1));
    assert_eq!(CloudBackupKeychain::global().namespace_id(), Some(namespace.clone()));
    let sync_state = Database::global().cloud_blob_sync_states.get(&record_id).unwrap().unwrap();
    assert_eq!(sync_state.namespace_id, namespace);
    assert!(
        matches!(sync_state.state, PersistedCloudBlobState::Dirty(_)),
        "unexpected sync state: {sync_state:?}"
    );
}

#[tokio::test(flavor = "current_thread")]
async fn restore_duplicate_wallets_with_failures_preserves_existing_configured_state() {
    let _guard = async_test_lock().lock().await;
    cove_tokio::init();
    let globals = test_globals();
    let manager = init_manager();

    configure_enabled_cloud_backup(&manager, globals, 7);
    let existing_namespace = CloudBackupKeychain::global().namespace_id().unwrap();
    let duplicate_wallet = xpub_only_wallet_metadata();
    let missing_wallet = xpub_only_wallet_metadata();
    persist_xpub_wallets(vec![duplicate_wallet.clone()]);

    let prf_key = [7u8; 32];
    let master_key = cove_cspp::master_key::MasterKey::generate();
    let namespace = master_key.namespace_id();
    let encrypted =
        cove_cspp::master_key_crypto::encrypt_master_key(&master_key, &prf_key, &[9; 32]).unwrap();
    let duplicate_record_id =
        cove_cspp::backup_data::wallet_record_id(duplicate_wallet.id.as_ref());
    let missing_record_id = cove_cspp::backup_data::wallet_record_id(missing_wallet.id.as_ref());
    globals.cloud.set_master_key_backup(namespace.clone(), serde_json::to_vec(&encrypted).unwrap());
    globals.cloud.set_wallet_backup(
        namespace.clone(),
        duplicate_record_id.clone(),
        encrypted_wallet_backup_bytes(&duplicate_wallet, &master_key, "duplicate-revision", 1)
            .await,
    );
    globals.cloud.set_wallet_files(
        namespace,
        vec![
            wallet_filename_from_record_id(&duplicate_record_id),
            wallet_filename_from_record_id(&missing_record_id),
        ],
    );
    globals.passkey.set_discover_result(Ok(DiscoveredPasskeyResult {
        prf_output: prf_key.to_vec(),
        credential_id: vec![1, 2, 3],
    }));
    globals.passkey.set_authenticate_result(Ok(prf_key.to_vec()));

    let operation = new_restore_operation_for_test(&manager).await;
    let report = operation.restore_from_cloud_backup(&manager).await.unwrap();

    assert_eq!(report.wallets_restored, 0);
    assert_eq!(report.wallets_failed, 1);
    assert!(report.failed_wallet_errors[0].contains("was listed but missing from cloud backup"));
    assert_eq!(
        Database::global().cloud_backup_state.get().unwrap().status(),
        PersistedCloudBackupStatus::Enabled
    );
    assert_eq!(Database::global().cloud_backup_state.get().unwrap().wallet_count(), Some(7));
    assert_eq!(CloudBackupKeychain::global().namespace_id(), Some(existing_namespace));
}

#[tokio::test(flavor = "current_thread")]
async fn restore_empty_namespace_list_returns_no_backup_found() {
    let _guard = async_test_lock().lock().await;
    cove_tokio::init();
    let globals = test_globals();
    let manager = init_manager();

    reset_cloud_backup_test_state(&manager, globals);

    let operation = new_restore_operation_for_test(&manager).await;
    let error = operation.restore_from_cloud_backup(&manager).await.unwrap_err();

    assert!(matches!(error, CloudBackupError::NoBackupFound));
    assert_eq!(CloudBackupKeychain::global().namespace_id(), None);
}

#[tokio::test(flavor = "current_thread")]
async fn restore_retries_platform_authorization_discover_failures() {
    let _guard = async_test_lock().lock().await;
    cove_tokio::init();
    let globals = test_globals();
    let manager = init_manager();

    reset_cloud_backup_test_state(&manager, globals);

    let prf_key = [7u8; 32];
    let master_key = cove_cspp::master_key::MasterKey::generate();
    let namespace = master_key.namespace_id();
    let encrypted =
        cove_cspp::master_key_crypto::encrypt_master_key(&master_key, &prf_key, &[9; 32]).unwrap();

    globals.cloud.set_master_key_backup(namespace.clone(), serde_json::to_vec(&encrypted).unwrap());
    globals.passkey.push_discover_result(Err(platform_authorization_failed()));
    globals.passkey.push_discover_result(Err(platform_authorization_failed()));
    globals.passkey.push_discover_result(Err(platform_authorization_failed()));
    globals.passkey.push_discover_result(Ok(DiscoveredPasskeyResult {
        prf_output: prf_key.to_vec(),
        credential_id: vec![1, 2, 3],
    }));

    let wallet = xpub_only_wallet_metadata();
    Keychain::global().save_wallet_xpub(&wallet.id, sample_xpub(&wallet).parse().unwrap()).unwrap();
    let record_id = cove_cspp::backup_data::wallet_record_id(wallet.id.as_ref());
    globals.cloud.set_wallet_backup(
        namespace.clone(),
        record_id.clone(),
        encrypted_wallet_backup_bytes(&wallet, &master_key, "revision", 1).await,
    );
    globals.cloud.set_wallet_files(namespace, vec![wallet_filename_from_record_id(&record_id)]);

    let operation = new_restore_operation_for_test(&manager).await;
    let report = operation.restore_from_cloud_backup(&manager).await.unwrap();
    assert_eq!(report.wallets_restored, 1);
    assert_eq!(report.wallets_failed, 0);
}

#[tokio::test(flavor = "current_thread")]
async fn restore_does_not_persist_first_passkey_match_before_restore_work_succeeds() {
    let _guard = async_test_lock().lock().await;
    cove_tokio::init();
    let globals = test_globals();
    let manager = init_manager();

    reset_cloud_backup_test_state(&manager, globals);

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
    globals.cloud.set_wallet_files(first_namespace, vec!["wallet-1.json".into()]);
    globals.cloud.set_wallet_files(second_namespace, vec!["wallet-2.json".into()]);
    globals.cloud.fail_list_wallet_files("list failed");
    globals.passkey.set_discover_result(Ok(DiscoveredPasskeyResult {
        prf_output: prf_key.to_vec(),
        credential_id: vec![1, 2, 3],
    }));
    globals.passkey.set_authenticate_result(Ok(prf_key.to_vec()));

    let operation = new_restore_operation_for_test(&manager).await;
    let error = operation.restore_from_cloud_backup(&manager).await.unwrap_err();

    assert!(error.to_string().contains("list failed"), "{error}");
    assert_eq!(CloudBackupKeychain::global().namespace_id(), None);
}

#[tokio::test(flavor = "current_thread")]
async fn restore_counts_listed_missing_wallet_backups_as_failures() {
    let _guard = async_test_lock().lock().await;
    cove_tokio::init();
    let globals = test_globals();
    let manager = init_manager();

    reset_cloud_backup_test_state(&manager, globals);

    let master_key = cove_cspp::master_key::MasterKey::generate();
    let namespace = master_key.namespace_id();
    let encrypted_master =
        cove_cspp::master_key_crypto::encrypt_master_key(&master_key, &[7; 32], &[9; 32]).unwrap();
    globals
        .cloud
        .set_master_key_backup(namespace.clone(), serde_json::to_vec(&encrypted_master).unwrap());
    cove_cspp::Cspp::new(Keychain::global().clone()).save_master_key(&master_key).unwrap();

    let supported_wallet = xpub_only_wallet_metadata();
    let missing_wallet = xpub_only_wallet_metadata();
    Keychain::global()
        .save_wallet_xpub(&supported_wallet.id, sample_xpub(&supported_wallet).parse().unwrap())
        .unwrap();
    let supported_record_id =
        cove_cspp::backup_data::wallet_record_id(supported_wallet.id.as_ref());
    let missing_record_id = cove_cspp::backup_data::wallet_record_id(missing_wallet.id.as_ref());
    globals.cloud.set_wallet_backup(
        namespace.clone(),
        supported_record_id.clone(),
        encrypted_wallet_backup_bytes(&supported_wallet, &master_key, "supported-revision", 1)
            .await,
    );
    globals.cloud.set_wallet_files(
        namespace,
        vec![
            wallet_filename_from_record_id(&supported_record_id),
            wallet_filename_from_record_id(&missing_record_id),
        ],
    );

    let operation = new_restore_operation_for_test(&manager).await;
    let report = operation.restore_from_cloud_backup(&manager).await.unwrap();
    assert_eq!(report.wallets_restored, 1);
    assert_eq!(report.wallets_failed, 1);
    assert!(report.failed_wallet_errors[0].contains("was listed but missing from cloud backup"));
    assert!(report.labels_failed_wallet_names.is_empty());
    assert_eq!(Database::global().cloud_backup_state.get().unwrap().wallet_count(), Some(1));
}

#[tokio::test(flavor = "current_thread")]
async fn restore_sanitizes_non_connectivity_wallet_download_errors() {
    let _guard = async_test_lock().lock().await;
    cove_tokio::init();
    let globals = test_globals();
    let manager = init_manager();

    reset_cloud_backup_test_state(&manager, globals);

    let master_key = cove_cspp::master_key::MasterKey::generate();
    let namespace = master_key.namespace_id();
    let encrypted_master =
        cove_cspp::master_key_crypto::encrypt_master_key(&master_key, &[7; 32], &[9; 32]).unwrap();
    globals
        .cloud
        .set_master_key_backup(namespace.clone(), serde_json::to_vec(&encrypted_master).unwrap());
    cove_cspp::Cspp::new(Keychain::global().clone()).save_master_key(&master_key).unwrap();

    let supported_wallet = xpub_only_wallet_metadata();
    let failed_wallet = xpub_only_wallet_metadata();
    Keychain::global()
        .save_wallet_xpub(&supported_wallet.id, sample_xpub(&supported_wallet).parse().unwrap())
        .unwrap();

    let supported_record_id =
        cove_cspp::backup_data::wallet_record_id(supported_wallet.id.as_ref());
    let failed_record_id = cove_cspp::backup_data::wallet_record_id(failed_wallet.id.as_ref());
    globals.cloud.set_wallet_backup(
        namespace.clone(),
        supported_record_id.clone(),
        encrypted_wallet_backup_bytes(&supported_wallet, &master_key, "supported-revision", 1)
            .await,
    );
    globals.cloud.fail_wallet_backup_download(
        namespace.clone(),
        failed_record_id.clone(),
        CloudStorageError::DownloadFailed(format!("raw record id {failed_record_id}")),
    );
    globals.cloud.set_wallet_files(
        namespace,
        vec![
            wallet_filename_from_record_id(&supported_record_id),
            wallet_filename_from_record_id(&failed_record_id),
        ],
    );

    let operation = new_restore_operation_for_test(&manager).await;
    let report = operation.restore_from_cloud_backup(&manager).await.unwrap();

    assert_eq!(report.wallets_restored, 1);
    assert_eq!(report.wallets_failed, 1);
    assert_eq!(report.failed_wallet_errors, vec!["wallet backup could not be read"]);
    assert!(!report.failed_wallet_errors[0].contains(&failed_record_id));
}

#[tokio::test(flavor = "current_thread")]
async fn restore_reports_label_warning_without_failing_wallet_restore() {
    let _guard = async_test_lock().lock().await;
    cove_tokio::init();
    let globals = test_globals();
    let manager = init_manager();

    reset_cloud_backup_test_state(&manager, globals);

    let master_key = cove_cspp::master_key::MasterKey::generate();
    let namespace = master_key.namespace_id();
    let encrypted_master =
        cove_cspp::master_key_crypto::encrypt_master_key(&master_key, &[7; 32], &[9; 32]).unwrap();
    globals
        .cloud
        .set_master_key_backup(namespace.clone(), serde_json::to_vec(&encrypted_master).unwrap());
    cove_cspp::Cspp::new(Keychain::global().clone()).save_master_key(&master_key).unwrap();

    let wallet = xpub_only_wallet_metadata();
    let record_id = cove_cspp::backup_data::wallet_record_id(wallet.id.as_ref());
    let entry = wallet_entry_with_labels(&wallet, Some("{"));
    globals.cloud.set_wallet_backup(
        namespace.clone(),
        record_id.clone(),
        encrypted_wallet_backup_bytes_for_entry(&entry, &master_key, 1),
    );
    globals.cloud.set_wallet_files(namespace, vec![wallet_filename_from_record_id(&record_id)]);

    let operation = new_restore_operation_for_test(&manager).await;
    let report = operation.restore_from_cloud_backup(&manager).await.unwrap();
    assert_eq!(report.wallets_restored, 1);
    assert_eq!(report.wallets_failed, 0);
    assert_eq!(report.labels_failed_wallet_names, vec![wallet.name.clone()]);
    assert_eq!(report.labels_failed_errors.len(), 1);
    assert!(
        report.labels_failed_errors[0].contains("Failed to parse labels")
            || report.labels_failed_errors[0].contains("failed to parse")
    );
    assert!(
        Database::global()
            .wallets()
            .get(&wallet.id, wallet.network, wallet.wallet_mode)
            .unwrap()
            .is_some()
    );
}

#[tokio::test(flavor = "current_thread")]
async fn restore_cloud_wallet_returns_label_warning_without_failing_restore() {
    let _guard = async_test_lock().lock().await;
    cove_tokio::init();
    let globals = test_globals();
    let manager = init_manager();

    reset_cloud_backup_test_state(&manager, globals);

    let master_key = cove_cspp::master_key::MasterKey::generate();
    let namespace = master_key.namespace_id();
    CloudBackupKeychain::global().save_namespace_id(&namespace).unwrap();
    cove_cspp::Cspp::new(Keychain::global().clone()).save_master_key(&master_key).unwrap();
    manager
        .persist_cloud_backup_state(
            &persisted_enabled_cloud_backup_state(None),
            "enable cloud backup for restore cloud wallet test",
        )
        .unwrap();

    let wallet = xpub_only_wallet_metadata();
    let record_id = cove_cspp::backup_data::wallet_record_id(wallet.id.as_ref());
    let entry = wallet_entry_with_labels(&wallet, Some("{"));
    globals.cloud.set_wallet_backup(
        namespace,
        record_id.clone(),
        encrypted_wallet_backup_bytes_for_entry(&entry, &master_key, 1),
    );

    let outcome = manager.do_restore_cloud_wallet(&record_id).await.unwrap();

    let WalletRestoreOutcome::Restored { labels_warning } = outcome else {
        panic!("expected restored wallet");
    };
    let warning = labels_warning.expect("expected label warning");
    assert_eq!(warning.wallet_name, wallet.name);
    assert!(
        warning.error.contains("Failed to parse labels")
            || warning.error.contains("failed to parse")
    );
    assert!(
        Database::global()
            .wallets()
            .get(&wallet.id, wallet.network, wallet.wallet_mode)
            .unwrap()
            .is_some()
    );
}

#[tokio::test(flavor = "current_thread")]
async fn restore_fails_when_all_wallet_backups_are_unsupported() {
    let _guard = async_test_lock().lock().await;
    cove_tokio::init();
    let globals = test_globals();
    let manager = init_manager();

    reset_cloud_backup_test_state(&manager, globals);

    let master_key = cove_cspp::master_key::MasterKey::generate();
    let namespace = master_key.namespace_id();
    let encrypted_master =
        cove_cspp::master_key_crypto::encrypt_master_key(&master_key, &[7; 32], &[9; 32]).unwrap();
    globals
        .cloud
        .set_master_key_backup(namespace.clone(), serde_json::to_vec(&encrypted_master).unwrap());
    cove_cspp::Cspp::new(Keychain::global().clone()).save_master_key(&master_key).unwrap();

    let wallet = xpub_only_wallet_metadata();
    Keychain::global().save_wallet_xpub(&wallet.id, sample_xpub(&wallet).parse().unwrap()).unwrap();

    let record_id = cove_cspp::backup_data::wallet_record_id(wallet.id.as_ref());
    globals.cloud.set_wallet_backup(
        namespace.clone(),
        record_id.clone(),
        encrypted_wallet_backup_bytes(&wallet, &master_key, "unsupported-revision", 2).await,
    );
    globals.cloud.set_wallet_files(namespace, vec![wallet_filename_from_record_id(&record_id)]);

    let operation = new_restore_operation_for_test(&manager).await;
    let error = operation.restore_from_cloud_backup(&manager).await.unwrap_err();

    assert!(matches!(
        error,
        CloudBackupError::Internal(message) if message == "all wallets failed to restore"
    ));

    assert_eq!(
        Database::global().cloud_backup_state.get().unwrap().status(),
        PersistedCloudBackupStatus::Disabled
    );
}

#[tokio::test(flavor = "current_thread")]
async fn restore_fails_when_all_listed_wallet_backups_are_missing() {
    let _guard = async_test_lock().lock().await;
    cove_tokio::init();
    let globals = test_globals();
    let manager = init_manager();

    reset_cloud_backup_test_state(&manager, globals);

    let master_key = cove_cspp::master_key::MasterKey::generate();
    let namespace = master_key.namespace_id();
    let encrypted_master =
        cove_cspp::master_key_crypto::encrypt_master_key(&master_key, &[7; 32], &[9; 32]).unwrap();
    globals
        .cloud
        .set_master_key_backup(namespace.clone(), serde_json::to_vec(&encrypted_master).unwrap());
    cove_cspp::Cspp::new(Keychain::global().clone()).save_master_key(&master_key).unwrap();

    let missing_wallet = xpub_only_wallet_metadata();
    let missing_record_id = cove_cspp::backup_data::wallet_record_id(missing_wallet.id.as_ref());
    globals
        .cloud
        .set_wallet_files(namespace, vec![wallet_filename_from_record_id(&missing_record_id)]);

    let operation = new_restore_operation_for_test(&manager).await;
    let error = operation.restore_from_cloud_backup(&manager).await.unwrap_err();

    assert!(matches!(
        error,
        CloudBackupError::Internal(message) if message == "all wallets failed to restore"
    ));

    assert_eq!(
        Database::global().cloud_backup_state.get().unwrap().status(),
        PersistedCloudBackupStatus::Disabled
    );
}
