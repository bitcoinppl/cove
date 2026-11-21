//! Recover Other Backups and enable merge delete a source namespace only once the provider
//! reports every record that replaced it uploaded
//!
//! iOS answers an upload once the file is staged on the device, so a source deleted then could
//! leave iCloud with no copy of a wallet when the upload never reaches the cloud, such as while
//! iCloud storage is full. The mock models that with uploads pending confirmation

use super::*;
use crate::manager::cloud_backup_manager::pending::test_support::hold_verifier_pass;

/// Another backup namespace holding one watch-only wallet that Recover Other Backups restores
/// under its own id, returning the namespace and the wallet's record id
async fn other_backup_with_one_wallet(globals: &TestGlobals) -> (String, String) {
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
    let record_id = wallet_record_id(wallet.id.as_ref());
    globals.cloud.set_wallet_backup(
        other_namespace.clone(),
        record_id.clone(),
        encrypted_remote_wallet_backup_bytes(&wallet, &other_master_key, "other-revision", 1).await,
    );
    globals.cloud.set_wallet_files(
        other_namespace.clone(),
        vec![wallet_filename_from_record_id(&record_id)],
    );

    (other_namespace, record_id)
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

#[tokio::test(flavor = "current_thread")]
async fn recover_other_backups_keeps_the_source_until_the_cloud_confirms_its_uploads() {
    let _guard = async_test_lock().lock().await;
    let globals = test_globals();
    let manager = init_manager();
    configure_enabled_cloud_backup(&manager, globals, 0);
    globals.cloud.set_reflect_uploaded_wallets_in_listing(true);
    globals.cloud.set_uploaded_wallets_pending_confirmation(true);
    let (other_namespace, _) = other_backup_with_one_wallet(globals).await;

    // the recovery's upload starts the manager's own verifier loop, so the test holds the pass
    // and runs each one itself
    let verifier_pass = hold_verifier_pass().await;
    let report = recover_other_backups_for_test(&manager).await.unwrap();

    assert_eq!((report.wallets_restored, report.wallets_failed), (1, 0));
    assert!(
        globals.cloud.has_namespace(&other_namespace),
        "the source was deleted before its replacement reached the cloud"
    );

    verifier_pass.verify_once(&manager).await;

    assert!(globals.cloud.has_namespace(&other_namespace));
    assert_eq!(pending_source_namespaces(), vec![other_namespace.clone()]);

    globals.cloud.set_uploaded_wallets_pending_confirmation(false);
    verifier_pass.verify_once(&manager).await;

    assert!(!globals.cloud.has_namespace(&other_namespace));
    assert!(pending_source_namespaces().is_empty());

    drop(verifier_pass);
    wait_for_background_refreshes(&manager).await;
}

/// The record of a pending source delete is all that remembers it, so a source recorded before
/// the app exits is deleted by a pass after the next launch
#[tokio::test(flavor = "current_thread")]
async fn recover_other_backups_deletes_the_source_after_a_restart_once_uploads_confirm() {
    let _guard = async_test_lock().lock().await;
    let globals = test_globals();
    let manager = init_manager();
    configure_enabled_cloud_backup(&manager, globals, 0);
    globals.cloud.set_reflect_uploaded_wallets_in_listing(true);
    globals.cloud.set_uploaded_wallets_pending_confirmation(true);
    let (other_namespace, _) = other_backup_with_one_wallet(globals).await;

    let verifier_pass = hold_verifier_pass().await;
    let report = recover_other_backups_for_test(&manager).await.unwrap();

    assert_eq!((report.wallets_restored, report.wallets_failed), (1, 0));
    assert!(globals.cloud.has_namespace(&other_namespace));

    // the old manager's loop waits for the held pass, and it holds the manager until it runs
    drop(verifier_pass);
    shut_down_manager_for_restart(manager).await;
    let restarted_manager = init_manager();

    assert_eq!(pending_source_namespaces(), vec![other_namespace.clone()]);
    assert!(globals.cloud.has_namespace(&other_namespace));

    globals.cloud.set_uploaded_wallets_pending_confirmation(false);
    verify_pending_uploads_once_for_test_async(&restarted_manager).await;

    assert!(!globals.cloud.has_namespace(&other_namespace));
    assert!(pending_source_namespaces().is_empty());
    wait_for_background_refreshes(&restarted_manager).await;
}

/// A source record under an id the current backup already holds is proven by reading the
/// current copy back, which on iOS can read a file only staged on this device, so that source
/// also waits for the provider to report the current record uploaded
#[tokio::test(flavor = "current_thread")]
async fn recover_other_backups_keeps_a_source_the_current_backup_holds_until_its_upload_confirms() {
    let _guard = async_test_lock().lock().await;
    let globals = test_globals();
    let manager = init_manager();
    configure_enabled_cloud_backup(&manager, globals, 0);
    globals.cloud.set_reflect_uploaded_wallets_in_listing(true);
    globals.cloud.set_uploaded_wallets_pending_confirmation(true);
    let verifier_pass = hold_verifier_pass().await;

    // this device backed the wallet up to the current backup, and that upload hasn't reached
    // the cloud yet
    let wallet = xpub_only_wallet_metadata();
    let record_id = wallet_record_id(wallet.id.as_ref());
    persist_xpub_wallets(vec![wallet.clone()]);
    backup_wallets_for_test(&manager, std::slice::from_ref(&wallet)).await.unwrap();

    // another backup holds the same wallet under the same record id
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
    globals.cloud.set_wallet_backup(
        other_namespace.clone(),
        record_id.clone(),
        encrypted_wallet_backup_bytes(&wallet, &other_master_key, "other-revision", 1).await,
    );
    globals.cloud.set_wallet_files(
        other_namespace.clone(),
        vec![wallet_filename_from_record_id(&record_id)],
    );

    let report = recover_other_backups_for_test(&manager).await.unwrap();

    assert_eq!((report.wallets_restored, report.wallets_failed), (0, 0));
    assert!(globals.cloud.has_namespace(&other_namespace));
    let state = Database::global().cloud_backup_state.get().unwrap();
    let [pending] = state.pending_source_deletes() else {
        panic!("expected the source pending deletion, found {state:?}");
    };
    assert_eq!(pending.record_ids, vec![record_id]);

    verifier_pass.verify_once(&manager).await;

    assert!(globals.cloud.has_namespace(&other_namespace));

    globals.cloud.set_uploaded_wallets_pending_confirmation(false);
    verifier_pass.verify_once(&manager).await;

    assert!(!globals.cloud.has_namespace(&other_namespace));
    assert!(pending_source_namespaces().is_empty());

    drop(verifier_pass);
    wait_for_background_refreshes(&manager).await;
}

/// An upload that failed for good never holds the source's wallet, so the source stays and its
/// pending delete is dropped instead of waiting forever
#[tokio::test(flavor = "current_thread")]
async fn recover_other_backups_keeps_the_source_when_its_replacement_upload_fails_for_good() {
    let _guard = async_test_lock().lock().await;
    let globals = test_globals();
    let manager = init_manager();
    configure_enabled_cloud_backup(&manager, globals, 0);
    globals.cloud.set_reflect_uploaded_wallets_in_listing(true);
    let (other_namespace, record_id) = other_backup_with_one_wallet(globals).await;

    let verifier_pass = hold_verifier_pass().await;
    recover_other_backups_for_test(&manager).await.unwrap();
    assert_eq!(pending_source_namespaces(), vec![other_namespace.clone()]);

    // the provider holds an unreadable file under the replacement's name, a terminal failure
    let current_namespace = CloudBackupKeychain::global().namespace_id().unwrap().unwrap();
    globals.cloud.set_wallet_backup(current_namespace, record_id.clone(), b"{".to_vec());
    let delete_attempts = globals.cloud.delete_namespace_attempt_count();
    verifier_pass.verify_once(&manager).await;

    assert!(matches!(
        Database::global().cloud_blob_sync_states.get(&record_id).unwrap(),
        Some(PersistedCloudBlobSyncState {
            state: PersistedCloudBlobState::Failed(CloudBlobFailedState { retryable: false, .. }),
            ..
        })
    ));
    assert!(globals.cloud.has_namespace(&other_namespace));
    assert_eq!(globals.cloud.delete_namespace_attempt_count(), delete_attempts);
    assert!(pending_source_namespaces().is_empty());

    drop(verifier_pass);
    wait_for_background_refreshes(&manager).await;
}
