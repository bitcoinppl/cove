use super::*;

#[tokio::test(flavor = "current_thread")]
async fn cloud_action_uses_existing_master_key_without_recovery() {
    ensure_cloud_backup_test_tokio_runtime();
    let store = Arc::new(MockStore::default());
    let cspp = cove_cspp::Cspp::new(MockStoreHandle(store));
    let expected = cove_cspp::master_key::MasterKey::generate();
    let namespace = expected.namespace_id();
    cspp.save_master_key(&expected).unwrap();

    let recovered = load_master_key_for_cloud_action(&cspp, &namespace, || async {
        Err(CloudBackupError::RecoveryRequired("unexpected".into()))
    })
    .await
    .unwrap();

    assert_eq!(recovered.as_bytes(), expected.as_bytes());
}

#[tokio::test(flavor = "current_thread")]
async fn cloud_action_does_not_create_master_key_when_missing() {
    ensure_cloud_backup_test_tokio_runtime();
    let store = Arc::new(MockStore::default());
    let cspp = cove_cspp::Cspp::new(MockStoreHandle(store.clone()));
    let namespace = cove_cspp::master_key::MasterKey::generate().namespace_id();

    let result = load_master_key_for_cloud_action(&cspp, &namespace, || async {
        Err(CloudBackupError::RecoveryRequired("needs recovery".into()))
    })
    .await;

    assert!(matches!(
        result,
        Err(CloudBackupError::RecoveryRequired(message)) if message == "needs recovery"
    ));
    assert!(cspp.load_master_key_from_store().unwrap().is_none());
    assert_eq!(*store.save_count.lock(), 0);
}

#[tokio::test(flavor = "current_thread")]
async fn cloud_action_recovers_when_local_master_key_namespace_mismatches() {
    ensure_cloud_backup_test_tokio_runtime();
    let store = Arc::new(MockStore::default());
    let cspp = cove_cspp::Cspp::new(MockStoreHandle(store));
    let stale = cove_cspp::master_key::MasterKey::generate();
    cspp.save_master_key(&stale).unwrap();

    let expected = cove_cspp::master_key::MasterKey::generate();
    let expected_bytes = *expected.as_bytes();
    let namespace = expected.namespace_id();

    let recovered = load_master_key_for_cloud_action(&cspp, &namespace, || async move {
        Ok(cove_cspp::master_key::MasterKey::from_bytes(expected_bytes))
    })
    .await
    .unwrap();

    assert_eq!(recovered.as_bytes(), expected.as_bytes());
}

#[tokio::test(flavor = "current_thread")]
async fn local_master_key_fallback_is_unavailable_after_local_cloud_state_clear() {
    let _guard = async_test_lock().lock().await;
    ensure_cloud_backup_test_tokio_runtime();
    let globals = test_globals();
    globals.reset();

    let keychain = Keychain::global();
    let cspp = cove_cspp::Cspp::new(keychain.clone());
    let master_key = cove_cspp::master_key::MasterKey::generate();
    let namespace_id = master_key.namespace_id();
    cspp.save_master_key(&master_key).unwrap();
    globals.cloud.set_wallet_files(namespace_id, vec!["wallet-test.json".into()]);

    CloudBackupKeychain::global().clear_local_state().unwrap();

    let fallback =
        try_restore_from_local_master_key(&CloudStorage::global_explicit_client(), &cspp)
            .await
            .unwrap();

    assert!(fallback.is_none());
}

#[test]
fn blocking_cloud_error_keeps_unavailable_storage_errors_visible() {
    let error = blocking_cloud_error(
        BlockingCloudStep::Enable,
        CloudBackupError::CloudStorage(CloudStorageError::NotAvailable(
            "iCloud Drive is not available".into(),
        )),
    );

    assert!(matches!(
        error,
        CloudBackupError::CloudStorage(CloudStorageError::NotAvailable(message))
            if message == "iCloud Drive is not available"
    ));
}

#[test]
fn blocking_cloud_error_rewrites_offline_storage_errors_to_step_message() {
    let error = blocking_cloud_error(
        BlockingCloudStep::Enable,
        CloudBackupError::CloudStorage(CloudStorageError::Offline("offline".into())),
    );

    assert!(matches!(
        error,
        CloudBackupError::Offline(message)
            if message == "Reconnect to the internet, then try enabling cloud backup again"
    ));
}
