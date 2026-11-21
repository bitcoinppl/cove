use super::*;
use crate::manager::cloud_backup_manager::actors::write::ConfigGeneration;
use crate::manager::cloud_backup_manager::pending::test_support::hold_verifier_pass;

struct CleanupTestSource {
    namespace: String,
    record_id: String,
    revision_hash: Option<String>,
}

/// Runs a merge cleanup job under the current configuration and waits for the worker to finish
/// it
///
/// The job only records a proven source; the verifier pass a test runs next deletes it
async fn enqueue_cleanup_for_test(
    manager: &RustCloudBackupManager,
    active_namespace: &str,
    active_master_key: &cove_cspp::master_key::MasterKey,
    source: CleanupTestSource,
) {
    let generation = manager.write_ticket().await.expect("write ticket").generation;
    enqueue_cleanup_in_generation_for_test(
        manager,
        active_namespace,
        active_master_key,
        source,
        generation,
    )
    .await;
}

async fn enqueue_cleanup_in_generation_for_test(
    manager: &RustCloudBackupManager,
    active_namespace: &str,
    active_master_key: &cove_cspp::master_key::MasterKey,
    source: CleanupTestSource,
    generation: ConfigGeneration,
) {
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

    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    while !call!(manager.supervisor.cleanup_idle_for_test()).await.expect("cleanup idle") {
        assert!(tokio::time::Instant::now() < deadline, "cleanup should finish");
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

/// The configured backup's master key, whose namespace a merge's sources moved into
fn configured_master_key() -> cove_cspp::master_key::MasterKey {
    cove_cspp::Cspp::new(Keychain::global().clone()).load_master_key_from_store().unwrap().unwrap()
}

/// Makes `master_key` the configured backup's key again, as a new configuration that keeps the
/// namespace does
fn reuse_namespace(master_key: &cove_cspp::master_key::MasterKey) {
    CloudBackupKeychain::global().save_namespace_id(&master_key.namespace_id()).unwrap();
    cove_cspp::Cspp::new(Keychain::global().clone()).save_master_key(master_key).unwrap();
}

/// The sources the configured record still waits to delete
fn pending_source_namespaces() -> Vec<String> {
    Database::global()
        .cloud_backup_state
        .get()
        .unwrap()
        .pending_source_deletes()
        .iter()
        .map(|pending| pending.source_namespace_id.clone())
        .collect()
}

/// Drives a reinitialize that is already running to its end, confirming its saved passkey
async fn finish_running_reinitialize(manager: &RustCloudBackupManager) {
    let deadline = LoadedHostDeadline::after(Duration::from_secs(2));
    while !deadline.has_passed() {
        let Some(claim) = manager.projected_exclusive_operation() else { break };

        if has_awaiting_saved_passkey_confirmation_for_test(manager).await {
            call!(
                manager.supervisor.complete_enable_saved_passkey_wait(
                    claim,
                    SavedPasskeyConfirmationMode::Manual
                )
            )
            .await
            .expect("complete the saved-passkey wait");
        }

        tokio::time::sleep(Duration::from_millis(10)).await;
        tokio::task::yield_now().await;
    }

    assert!(manager.projected_exclusive_operation().is_none(), "the reinitialize finishes");
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
    let old_master_key = configured_master_key();
    enable_cloud_backup_without_reset(&manager, 0);
    assert_ne!(manager.write_ticket().await.unwrap().generation, old_generation);

    // the new configuration keeps the namespace, so only the configuration the proof was read
    // under keeps it from deleting
    let active_master_key = old_master_key;
    reuse_namespace(&active_master_key);
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
        &active_namespace,
        &active_master_key,
        CleanupTestSource {
            namespace: source_namespace.clone(),
            record_id,
            revision_hash: Some("matching-revision".into()),
        },
        old_generation,
    )
    .await;
    verify_pending_uploads_once_for_test_async(&manager).await;

    assert!(globals.cloud.has_namespace(&source_namespace));
    assert_eq!(globals.cloud.delete_namespace_attempt_count(), 0);
    assert!(pending_source_namespaces().is_empty());
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

    let active_master_key = configured_master_key();
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
        &active_namespace,
        &active_master_key,
        CleanupTestSource {
            namespace: source_namespace.clone(),
            record_id,
            revision_hash: Some("matching-revision".into()),
        },
        generation,
    );
    let replace_configuration = async {
        proof.wait_until_blocked().await;

        Database::global().cloud_backup_state.set(&PersistedCloudBackupState::Disabled).unwrap();
        manager.sync_persisted_state();
        supervisor_checkpoint(&manager).await;
        enable_cloud_backup_without_reset(&manager, 0);
        assert_ne!(manager.write_ticket().await.unwrap().generation, generation);

        // the new configuration keeps the namespace, so only the configuration the proof was
        // read under keeps it from deleting
        reuse_namespace(&active_master_key);
        proof.release();
    };
    tokio::join!(enqueue, replace_configuration);
    verify_pending_uploads_once_for_test_async(&manager).await;

    assert!(globals.cloud.has_namespace(&source_namespace), "the late proof deleted its source");
    assert_eq!(globals.cloud.delete_namespace_attempt_count(), 0);
    assert!(pending_source_namespaces().is_empty());
}

/// A merge cleanup proven before a reinitialize never deletes the namespace that reinitialize
/// promotes: the old proof's record arrives while the reinitialize is recovering its source, the
/// running reinitialize pauses source cleanup, and once it ends another namespace is active, so
/// the record is dropped
///
/// `cleanup_deletes_source_namespace_after_active_record_proof` is the control: the same proof
/// with no reinitialize deletes the source
#[tokio::test(flavor = "current_thread")]
async fn cleanup_proven_before_a_reinitialize_never_deletes_the_namespace_it_promotes() {
    let _guard = async_test_lock().lock().await;
    let globals = test_globals();
    let manager = init_manager();
    configure_enabled_cloud_backup(&manager, globals, 0);
    CONNECTIVITY_MANAGER.set_connection_state(true);

    // the merge's source holds a wrapper the user's passkey opens, so a reinitialize with no
    // local master key recovers it as the new active namespace
    let prf_key = [7u8; 32];
    let source_namespace = seed_master_key_wrapper(globals, &prf_key, &[9; 32], 1);
    globals.passkey.set_discover_result(Ok(DiscoveredPasskeyResult {
        prf_output: prf_key.to_vec(),
        credential_id: vec![1, 2, 3],
    }));
    globals.passkey.set_authenticate_result(Ok(prf_key.to_vec()));

    let active_master_key = cove_cspp::master_key::MasterKey::generate();
    let active_namespace = active_master_key.namespace_id();
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
    CloudBackupKeychain::global().delete_master_key().unwrap();

    // the proof's last read, the active record, waits until the reinitialize is running
    let proof = globals.cloud.gate_wallet_backup_download_after_successes(
        active_namespace.clone(),
        record_id.clone(),
        0,
    );
    let generation = manager.write_ticket().await.expect("write ticket").generation;
    let enqueue = enqueue_cleanup_in_generation_for_test(
        &manager,
        &active_namespace,
        &active_master_key,
        CleanupTestSource {
            namespace: source_namespace.clone(),
            record_id,
            revision_hash: Some("matching-revision".into()),
        },
        generation,
    );
    let reinitialize = async {
        proof.wait_until_blocked().await;

        // the reinitialize waits at its namespace listing while the old proof finishes and
        // records its source
        let listing = globals.cloud.gate_next_list_namespaces();
        call!(manager.supervisor.start_recovery_operation(
            RecoveryAction::ReinitializeBackup,
            Some(tap(PermitIntent::Reinitialize))
        ))
        .await
        .expect("start reinitialize");
        listing.wait_until_blocked().await;
        proof.release();
        let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
        while !call!(manager.supervisor.cleanup_idle_for_test()).await.expect("cleanup idle") {
            assert!(tokio::time::Instant::now() < deadline, "the old cleanup should finish");
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        listing.release();
    };
    tokio::join!(enqueue, reinitialize);
    finish_running_reinitialize(&manager).await;
    verify_pending_uploads_once_for_test_async(&manager).await;

    assert!(globals.cloud.has_namespace(&source_namespace), "the old proof deleted its source");
    assert_eq!(globals.cloud.delete_namespace_attempt_count(), 0);
    assert!(pending_source_namespaces().is_empty());
    assert_eq!(
        CloudBackupKeychain::global().namespace_id().unwrap(),
        Some(source_namespace),
        "the reinitialize did not promote the cleanup's source"
    );
}

#[tokio::test(flavor = "current_thread")]
async fn cleanup_deletes_source_namespace_after_active_record_proof() {
    let _guard = async_test_lock().lock().await;
    let globals = test_globals();
    let manager = init_manager();

    // the cleanup runs after Enable committed, as a configured backup's background write
    configure_enabled_cloud_backup(&manager, globals, 0);
    let active_master_key = configured_master_key();
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

    // recording the source starts the manager's own verifier loop, so the test holds the pass and
    // runs it itself
    let verifier_pass = hold_verifier_pass().await;
    enqueue_cleanup_for_test(
        &manager,
        &active_namespace,
        &active_master_key,
        CleanupTestSource {
            namespace: source_namespace.clone(),
            record_id,
            revision_hash: Some("matching-revision".into()),
        },
    )
    .await;

    // the cleanup only records the source; the verifier pass that finds its replacement
    // uploaded deletes it
    assert!(globals.cloud.has_namespace(&source_namespace));
    assert_eq!(pending_source_namespaces(), vec![source_namespace.clone()]);
    verifier_pass.verify_once(&manager).await;

    assert!(!globals.cloud.has_namespace(&source_namespace));
    assert!(pending_source_namespaces().is_empty());

    drop(verifier_pass);
    wait_for_background_refreshes(&manager).await;
}

#[tokio::test(flavor = "current_thread")]
async fn cleanup_keeps_source_namespace_when_active_record_is_missing() {
    let _guard = async_test_lock().lock().await;
    let globals = test_globals();
    let manager = init_manager();

    // the cleanup runs after Enable committed, as a configured backup's background write
    configure_enabled_cloud_backup(&manager, globals, 0);
    let active_master_key = configured_master_key();
    let active_namespace = active_master_key.namespace_id();
    let source_namespace = cove_cspp::master_key::MasterKey::generate().namespace_id();
    let record_id = "missing-record".to_string();
    globals.cloud.set_wallet_files(
        source_namespace.clone(),
        vec![wallet_filename_from_record_id(&record_id)],
    );

    enqueue_cleanup_for_test(
        &manager,
        &active_namespace,
        &active_master_key,
        CleanupTestSource {
            namespace: source_namespace.clone(),
            record_id,
            revision_hash: Some("expected-revision".into()),
        },
    )
    .await;
    verify_pending_uploads_once_for_test_async(&manager).await;

    assert!(globals.cloud.has_namespace(&source_namespace));
    assert!(pending_source_namespaces().is_empty());
}

#[tokio::test(flavor = "current_thread")]
async fn cleanup_keeps_source_namespace_when_active_record_is_undecryptable() {
    let _guard = async_test_lock().lock().await;
    let globals = test_globals();
    let manager = init_manager();

    // the cleanup runs after Enable committed, as a configured backup's background write
    configure_enabled_cloud_backup(&manager, globals, 0);
    let active_master_key = configured_master_key();
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
        &active_namespace,
        &active_master_key,
        CleanupTestSource {
            namespace: source_namespace.clone(),
            record_id,
            revision_hash: Some("expected-revision".into()),
        },
    )
    .await;
    verify_pending_uploads_once_for_test_async(&manager).await;

    assert!(globals.cloud.has_namespace(&source_namespace));
    assert!(pending_source_namespaces().is_empty());
}

#[tokio::test(flavor = "current_thread")]
async fn cleanup_keeps_source_namespace_when_active_record_is_unsupported() {
    let _guard = async_test_lock().lock().await;
    let globals = test_globals();
    let manager = init_manager();

    // the cleanup runs after Enable committed, as a configured backup's background write
    configure_enabled_cloud_backup(&manager, globals, 0);
    let active_master_key = configured_master_key();
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
        &active_namespace,
        &active_master_key,
        CleanupTestSource {
            namespace: source_namespace.clone(),
            record_id,
            revision_hash: Some("expected-revision".into()),
        },
    )
    .await;
    verify_pending_uploads_once_for_test_async(&manager).await;

    assert!(globals.cloud.has_namespace(&source_namespace));
    assert!(pending_source_namespaces().is_empty());
}

#[tokio::test(flavor = "current_thread")]
async fn cleanup_keeps_source_namespace_when_active_revision_mismatches() {
    let _guard = async_test_lock().lock().await;
    let globals = test_globals();
    let manager = init_manager();

    // the cleanup runs after Enable committed, as a configured backup's background write
    configure_enabled_cloud_backup(&manager, globals, 0);
    let active_master_key = configured_master_key();
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
        &active_namespace,
        &active_master_key,
        CleanupTestSource {
            namespace: source_namespace.clone(),
            record_id,
            revision_hash: Some("expected-revision".into()),
        },
    )
    .await;
    verify_pending_uploads_once_for_test_async(&manager).await;

    assert!(globals.cloud.has_namespace(&source_namespace));
    assert!(pending_source_namespaces().is_empty());
}

#[tokio::test(flavor = "current_thread")]
async fn cleanup_keeps_source_namespace_when_delete_fails() {
    let _guard = async_test_lock().lock().await;
    let globals = test_globals();
    let manager = init_manager();

    // the cleanup runs after Enable committed, as a configured backup's background write
    configure_enabled_cloud_backup(&manager, globals, 0);
    let active_master_key = configured_master_key();
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
        &active_namespace,
        &active_master_key,
        CleanupTestSource {
            namespace: source_namespace.clone(),
            record_id,
            revision_hash: Some("expected-revision".into()),
        },
    )
    .await;
    verify_pending_uploads_once_for_test_async(&manager).await;

    // a failed delete keeps the source pending, so a later pass tries again
    assert!(globals.cloud.delete_namespace_attempt_count() > 0);
    assert!(globals.cloud.has_namespace(&source_namespace));
    assert_eq!(pending_source_namespaces(), vec![source_namespace.clone()]);

    // a source another device already deleted is what the delete wanted
    globals.cloud.fail_delete_namespace_not_found("source already deleted");
    verify_pending_uploads_once_for_test_async(&manager).await;

    assert!(pending_source_namespaces().is_empty());
}
