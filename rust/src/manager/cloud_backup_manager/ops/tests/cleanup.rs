use super::*;
use crate::manager::cloud_backup_manager::actors::write::ConfigGeneration;

struct CleanupTestSource {
    namespace: String,
    record_id: String,
    revision_hash: Option<String>,
}

/// What the cleanup job must have done before a test inspects the source namespace
enum CleanupWait {
    ActiveNamespaceInspected,
    SourceNamespaceDeleted,
    SourceDeleteAttempted,
    /// The cleanup worker finished the job, whatever it did
    Idle,
}

async fn enqueue_cleanup_for_test(
    manager: &RustCloudBackupManager,
    globals: &TestGlobals,
    active_namespace: &str,
    active_master_key: &cove_cspp::master_key::MasterKey,
    source: CleanupTestSource,
    wait: CleanupWait,
) {
    let generation = manager.write_ticket().await.expect("write ticket").generation;
    enqueue_cleanup_in_generation_for_test(
        manager,
        globals,
        active_namespace,
        active_master_key,
        source,
        generation,
        wait,
    )
    .await;
}

async fn enqueue_cleanup_in_generation_for_test(
    manager: &RustCloudBackupManager,
    globals: &TestGlobals,
    active_namespace: &str,
    active_master_key: &cove_cspp::master_key::MasterKey,
    source: CleanupTestSource,
    generation: ConfigGeneration,
    wait: CleanupWait,
) {
    let source_namespace = source.namespace.clone();
    let list_attempts_before =
        globals.cloud.list_wallet_files_attempt_count_for_namespace(active_namespace);
    let delete_attempts_before = globals.cloud.delete_namespace_attempt_count();
    call!(manager.supervisor.enqueue_cleanup_for_test(CloudBackupCleanupJob {
        cloud: CloudStorage::global_explicit_client(),
        generation,
        active_namespace_id: active_namespace.to_owned(),
        active_critical_key: active_master_key.critical_data_key(),
        sources: vec![CleanupSourceNamespace {
            namespace_id: source.namespace,
            expected_wallets: vec![CleanupExpectedWalletRecord {
                record_id: source.record_id,
                content_revision_hash: source.revision_hash,
            }],
        }],
    }))
    .await
    .expect("enqueue cleanup");

    match wait {
        CleanupWait::ActiveNamespaceInspected => {
            wait_for_test_condition(
                Duration::from_secs(1),
                "cleanup should inspect active namespace",
                &mut || {
                    globals.cloud.list_wallet_files_attempt_count_for_namespace(active_namespace)
                        > list_attempts_before
                },
            )
            .await;
        }
        CleanupWait::SourceNamespaceDeleted => {
            wait_for_test_condition(
                Duration::from_secs(1),
                "cleanup should delete source namespace",
                &mut || !globals.cloud.has_namespace(&source_namespace),
            )
            .await;
        }
        CleanupWait::SourceDeleteAttempted => {
            wait_for_test_condition(
                Duration::from_secs(1),
                "cleanup should attempt source namespace delete",
                &mut || globals.cloud.delete_namespace_attempt_count() > delete_attempts_before,
            )
            .await;
        }
        CleanupWait::Idle => {
            let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
            while !call!(manager.supervisor.cleanup_idle_for_test()).await.expect("cleanup idle") {
                assert!(tokio::time::Instant::now() < deadline, "cleanup should finish");
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        }
    }
}

/// A merge cleanup carries the configuration it was proven in; after Turn Off and a later
/// Enable the lane drops it, so the old proof never deletes a source namespace
#[tokio::test(flavor = "current_thread")]
async fn cleanup_from_an_older_configuration_never_deletes_its_source() {
    let _guard = async_test_lock().lock().await;
    let globals = test_globals();
    let manager = init_manager();
    configure_enabled_cloud_backup(&manager, globals, 0);
    let old_generation = manager.write_ticket().await.unwrap().generation;

    // the backup is turned off and enabled again before the cleanup reaches the lane
    Database::global().cloud_backup_state.set(&PersistedCloudBackupState::Disabled).unwrap();
    manager.sync_persisted_state();
    supervisor_checkpoint(&manager).await;
    enable_cloud_backup_without_reset(&manager, 0);
    assert_ne!(manager.write_ticket().await.unwrap().generation, old_generation);

    let active_master_key = cove_cspp::master_key::MasterKey::generate();
    let active_namespace = active_master_key.namespace_id();
    let source_namespace = cove_cspp::master_key::MasterKey::generate().namespace_id();
    let wallet = xpub_only_wallet_metadata();
    Keychain::global().save_wallet_xpub(&wallet.id, sample_xpub(&wallet).parse().unwrap()).unwrap();
    let record_id = cove_cspp::backup_data::wallet_record_id(wallet.id.as_ref());
    globals.cloud.set_wallet_backup(
        active_namespace.clone(),
        record_id.clone(),
        encrypted_wallet_backup_bytes(&wallet, &active_master_key, "matching-revision", 1).await,
    );
    globals.cloud.set_wallet_files(
        active_namespace.clone(),
        vec![wallet_filename_from_record_id(&record_id)],
    );
    globals.cloud.set_wallet_files(
        source_namespace.clone(),
        vec![wallet_filename_from_record_id(&record_id)],
    );

    enqueue_cleanup_in_generation_for_test(
        &manager,
        globals,
        &active_namespace,
        &active_master_key,
        CleanupTestSource {
            namespace: source_namespace.clone(),
            record_id,
            revision_hash: Some("matching-revision".into()),
        },
        old_generation,
        CleanupWait::Idle,
    )
    .await;

    assert!(globals.cloud.has_namespace(&source_namespace));
    assert_eq!(globals.cloud.delete_namespace_attempt_count(), 0);
}

/// A merge cleanup held in its proof while the backup is turned off and on again never deletes
/// its source once the proof finishes: its delete carries the replaced configuration
///
/// `cleanup_deletes_source_namespace_after_active_record_proof` is the control: the same proof
/// with no configuration change deletes the source
#[tokio::test(flavor = "current_thread")]
async fn cleanup_held_in_its_proof_across_turn_off_and_enable_never_deletes_its_source() {
    let _guard = async_test_lock().lock().await;
    let globals = test_globals();
    let manager = init_manager();
    configure_enabled_cloud_backup(&manager, globals, 0);

    let active_master_key = cove_cspp::master_key::MasterKey::generate();
    let active_namespace = active_master_key.namespace_id();
    let source_namespace = cove_cspp::master_key::MasterKey::generate().namespace_id();
    let wallet = xpub_only_wallet_metadata();
    Keychain::global().save_wallet_xpub(&wallet.id, sample_xpub(&wallet).parse().unwrap()).unwrap();
    let record_id = cove_cspp::backup_data::wallet_record_id(wallet.id.as_ref());
    globals.cloud.set_wallet_backup(
        active_namespace.clone(),
        record_id.clone(),
        encrypted_wallet_backup_bytes(&wallet, &active_master_key, "matching-revision", 1).await,
    );
    globals.cloud.set_wallet_files(
        active_namespace.clone(),
        vec![wallet_filename_from_record_id(&record_id)],
    );
    globals.cloud.set_wallet_files(
        source_namespace.clone(),
        vec![wallet_filename_from_record_id(&record_id)],
    );

    // the proof's last read, the active record, waits until the configuration was replaced
    let proof = globals.cloud.gate_wallet_backup_download_after_successes(
        active_namespace.clone(),
        record_id.clone(),
        0,
    );
    let generation = manager.write_ticket().await.expect("write ticket").generation;
    let enqueue = enqueue_cleanup_in_generation_for_test(
        &manager,
        globals,
        &active_namespace,
        &active_master_key,
        CleanupTestSource {
            namespace: source_namespace.clone(),
            record_id,
            revision_hash: Some("matching-revision".into()),
        },
        generation,
        CleanupWait::Idle,
    );
    let replace_configuration = async {
        proof.wait_until_blocked().await;

        Database::global().cloud_backup_state.set(&PersistedCloudBackupState::Disabled).unwrap();
        manager.sync_persisted_state();
        supervisor_checkpoint(&manager).await;
        enable_cloud_backup_without_reset(&manager, 0);
        assert_ne!(manager.write_ticket().await.unwrap().generation, generation);
        proof.release();
    };
    tokio::join!(enqueue, replace_configuration);

    assert!(globals.cloud.has_namespace(&source_namespace), "the late proof deleted its source");
    assert_eq!(globals.cloud.delete_namespace_attempt_count(), 0);
}

#[tokio::test(flavor = "current_thread")]
async fn cleanup_deletes_source_namespace_after_active_record_proof() {
    let _guard = async_test_lock().lock().await;
    let globals = test_globals();
    let manager = init_manager();

    // the cleanup runs after Enable committed, as a configured backup's background write
    configure_enabled_cloud_backup(&manager, globals, 0);
    let active_master_key = cove_cspp::master_key::MasterKey::generate();
    let active_namespace = active_master_key.namespace_id();
    let source_namespace = cove_cspp::master_key::MasterKey::generate().namespace_id();
    let wallet = xpub_only_wallet_metadata();
    Keychain::global().save_wallet_xpub(&wallet.id, sample_xpub(&wallet).parse().unwrap()).unwrap();
    let record_id = cove_cspp::backup_data::wallet_record_id(wallet.id.as_ref());

    globals.cloud.set_wallet_backup(
        active_namespace.clone(),
        record_id.clone(),
        encrypted_wallet_backup_bytes(&wallet, &active_master_key, "matching-revision", 1).await,
    );
    globals.cloud.set_wallet_files(
        active_namespace.clone(),
        vec![wallet_filename_from_record_id(&record_id)],
    );
    globals.cloud.set_wallet_files(
        source_namespace.clone(),
        vec![wallet_filename_from_record_id(&record_id)],
    );

    enqueue_cleanup_for_test(
        &manager,
        globals,
        &active_namespace,
        &active_master_key,
        CleanupTestSource {
            namespace: source_namespace.clone(),
            record_id,
            revision_hash: Some("matching-revision".into()),
        },
        CleanupWait::SourceNamespaceDeleted,
    )
    .await;

    assert!(!globals.cloud.has_namespace(&source_namespace));
}

#[tokio::test(flavor = "current_thread")]
async fn cleanup_keeps_source_namespace_when_active_record_is_missing() {
    let _guard = async_test_lock().lock().await;
    let globals = test_globals();
    let manager = init_manager();

    // the cleanup runs after Enable committed, as a configured backup's background write
    configure_enabled_cloud_backup(&manager, globals, 0);
    let active_master_key = cove_cspp::master_key::MasterKey::generate();
    let active_namespace = active_master_key.namespace_id();
    let source_namespace = cove_cspp::master_key::MasterKey::generate().namespace_id();
    let record_id = "missing-record".to_string();
    globals.cloud.set_wallet_files(
        source_namespace.clone(),
        vec![wallet_filename_from_record_id(&record_id)],
    );

    enqueue_cleanup_for_test(
        &manager,
        globals,
        &active_namespace,
        &active_master_key,
        CleanupTestSource {
            namespace: source_namespace.clone(),
            record_id,
            revision_hash: Some("expected-revision".into()),
        },
        CleanupWait::ActiveNamespaceInspected,
    )
    .await;

    assert!(globals.cloud.has_namespace(&source_namespace));
}

#[tokio::test(flavor = "current_thread")]
async fn cleanup_keeps_source_namespace_when_active_record_is_undecryptable() {
    let _guard = async_test_lock().lock().await;
    let globals = test_globals();
    let manager = init_manager();

    // the cleanup runs after Enable committed, as a configured backup's background write
    configure_enabled_cloud_backup(&manager, globals, 0);
    let active_master_key = cove_cspp::master_key::MasterKey::generate();
    let wrong_master_key = cove_cspp::master_key::MasterKey::generate();
    let active_namespace = active_master_key.namespace_id();
    let source_namespace = cove_cspp::master_key::MasterKey::generate().namespace_id();
    let wallet = xpub_only_wallet_metadata();
    Keychain::global().save_wallet_xpub(&wallet.id, sample_xpub(&wallet).parse().unwrap()).unwrap();
    let record_id = cove_cspp::backup_data::wallet_record_id(wallet.id.as_ref());

    globals.cloud.set_wallet_backup(
        active_namespace.clone(),
        record_id.clone(),
        encrypted_wallet_backup_bytes(&wallet, &wrong_master_key, "expected-revision", 1).await,
    );
    globals.cloud.set_wallet_files(
        active_namespace.clone(),
        vec![wallet_filename_from_record_id(&record_id)],
    );
    globals.cloud.set_wallet_files(
        source_namespace.clone(),
        vec![wallet_filename_from_record_id(&record_id)],
    );

    enqueue_cleanup_for_test(
        &manager,
        globals,
        &active_namespace,
        &active_master_key,
        CleanupTestSource {
            namespace: source_namespace.clone(),
            record_id,
            revision_hash: Some("expected-revision".into()),
        },
        CleanupWait::ActiveNamespaceInspected,
    )
    .await;

    assert!(globals.cloud.has_namespace(&source_namespace));
}

#[tokio::test(flavor = "current_thread")]
async fn cleanup_keeps_source_namespace_when_active_record_is_unsupported() {
    let _guard = async_test_lock().lock().await;
    let globals = test_globals();
    let manager = init_manager();

    // the cleanup runs after Enable committed, as a configured backup's background write
    configure_enabled_cloud_backup(&manager, globals, 0);
    let active_master_key = cove_cspp::master_key::MasterKey::generate();
    let active_namespace = active_master_key.namespace_id();
    let source_namespace = cove_cspp::master_key::MasterKey::generate().namespace_id();
    let wallet = xpub_only_wallet_metadata();
    Keychain::global().save_wallet_xpub(&wallet.id, sample_xpub(&wallet).parse().unwrap()).unwrap();
    let record_id = cove_cspp::backup_data::wallet_record_id(wallet.id.as_ref());

    globals.cloud.set_wallet_backup(
        active_namespace.clone(),
        record_id.clone(),
        encrypted_wallet_backup_bytes(&wallet, &active_master_key, "expected-revision", 3).await,
    );
    globals.cloud.set_wallet_files(
        active_namespace.clone(),
        vec![wallet_filename_from_record_id(&record_id)],
    );
    globals.cloud.set_wallet_files(
        source_namespace.clone(),
        vec![wallet_filename_from_record_id(&record_id)],
    );

    enqueue_cleanup_for_test(
        &manager,
        globals,
        &active_namespace,
        &active_master_key,
        CleanupTestSource {
            namespace: source_namespace.clone(),
            record_id,
            revision_hash: Some("expected-revision".into()),
        },
        CleanupWait::ActiveNamespaceInspected,
    )
    .await;

    assert!(globals.cloud.has_namespace(&source_namespace));
}

#[tokio::test(flavor = "current_thread")]
async fn cleanup_keeps_source_namespace_when_active_revision_mismatches() {
    let _guard = async_test_lock().lock().await;
    let globals = test_globals();
    let manager = init_manager();

    // the cleanup runs after Enable committed, as a configured backup's background write
    configure_enabled_cloud_backup(&manager, globals, 0);
    let active_master_key = cove_cspp::master_key::MasterKey::generate();
    let active_namespace = active_master_key.namespace_id();
    let source_namespace = cove_cspp::master_key::MasterKey::generate().namespace_id();
    let wallet = xpub_only_wallet_metadata();
    Keychain::global().save_wallet_xpub(&wallet.id, sample_xpub(&wallet).parse().unwrap()).unwrap();
    let record_id = cove_cspp::backup_data::wallet_record_id(wallet.id.as_ref());

    globals.cloud.set_wallet_backup(
        active_namespace.clone(),
        record_id.clone(),
        encrypted_wallet_backup_bytes(&wallet, &active_master_key, "actual-revision", 1).await,
    );
    globals.cloud.set_wallet_files(
        active_namespace.clone(),
        vec![wallet_filename_from_record_id(&record_id)],
    );
    globals.cloud.set_wallet_files(
        source_namespace.clone(),
        vec![wallet_filename_from_record_id(&record_id)],
    );

    enqueue_cleanup_for_test(
        &manager,
        globals,
        &active_namespace,
        &active_master_key,
        CleanupTestSource {
            namespace: source_namespace.clone(),
            record_id,
            revision_hash: Some("expected-revision".into()),
        },
        CleanupWait::ActiveNamespaceInspected,
    )
    .await;

    assert!(globals.cloud.has_namespace(&source_namespace));
}

#[tokio::test(flavor = "current_thread")]
async fn cleanup_keeps_source_namespace_when_delete_fails() {
    let _guard = async_test_lock().lock().await;
    let globals = test_globals();
    let manager = init_manager();

    // the cleanup runs after Enable committed, as a configured backup's background write
    configure_enabled_cloud_backup(&manager, globals, 0);
    let active_master_key = cove_cspp::master_key::MasterKey::generate();
    let active_namespace = active_master_key.namespace_id();
    let source_namespace = cove_cspp::master_key::MasterKey::generate().namespace_id();
    let wallet = xpub_only_wallet_metadata();
    Keychain::global().save_wallet_xpub(&wallet.id, sample_xpub(&wallet).parse().unwrap()).unwrap();
    let record_id = cove_cspp::backup_data::wallet_record_id(wallet.id.as_ref());

    globals.cloud.set_wallet_backup(
        active_namespace.clone(),
        record_id.clone(),
        encrypted_wallet_backup_bytes(&wallet, &active_master_key, "expected-revision", 1).await,
    );
    globals.cloud.set_wallet_files(
        active_namespace.clone(),
        vec![wallet_filename_from_record_id(&record_id)],
    );
    globals.cloud.set_wallet_files(
        source_namespace.clone(),
        vec![wallet_filename_from_record_id(&record_id)],
    );
    globals.cloud.fail_delete_namespace("delete failed");

    enqueue_cleanup_for_test(
        &manager,
        globals,
        &active_namespace,
        &active_master_key,
        CleanupTestSource {
            namespace: source_namespace.clone(),
            record_id,
            revision_hash: Some("expected-revision".into()),
        },
        CleanupWait::SourceDeleteAttempted,
    )
    .await;

    assert!(globals.cloud.has_namespace(&source_namespace));
}
