use std::collections::HashSet;

use super::*;
use crate::manager::cloud_backup_manager::CloudBackupWalletVerificationIssues;

fn mark_undecryptable_backups_reported(manager: &RustCloudBackupManager, decryption_failed: u32) {
    manager.apply_verification_state(VerificationState::NeedsAttention(DeepVerificationReport {
        master_key_wrapper_repaired: false,
        local_master_key_repaired: false,
        credential_recovered: false,
        wallets_verified: 0,
        wallet_issues: CloudBackupWalletVerificationIssues {
            decryption_failed,
            ..CloudBackupWalletVerificationIssues::default()
        },
        detail: None,
    }));
}

#[tokio::test(flavor = "current_thread")]
async fn undecryptable_deletion_rechecks_and_deletes_only_cloud_only_crypto_failures() {
    let _guard = async_test_lock().lock().await;
    cove_tokio::init();
    let globals = test_globals();
    let manager = init_manager();
    configure_enabled_cloud_backup(&manager, globals, 0);

    let namespace = CloudBackupKeychain::global().namespace_id().unwrap();
    let current_master_key = cove_cspp::Cspp::new(Keychain::global().clone())
        .load_master_key_from_store()
        .unwrap()
        .unwrap();
    let other_master_key = cove_cspp::master_key::MasterKey::generate();
    let good_wallet = xpub_only_wallet_metadata();
    let mut bad_wallet = xpub_only_wallet_metadata();
    bad_wallet.network = crate::network::Network::Testnet;
    let mut recovered_before_delete = xpub_only_wallet_metadata();
    recovered_before_delete.network = crate::network::Network::Signet;
    let mut local_bad_wallet = xpub_only_wallet_metadata();
    local_bad_wallet.network = crate::network::Network::Testnet4;
    persist_xpub_wallets(vec![local_bad_wallet.clone()]);

    let good_record_id = wallet_record_id(good_wallet.id.as_ref());
    let bad_record_id = wallet_record_id(bad_wallet.id.as_ref());
    let recovered_record_id = wallet_record_id(recovered_before_delete.id.as_ref());
    let local_bad_record_id = wallet_record_id(local_bad_wallet.id.as_ref());
    globals.cloud.set_wallet_backup(
        namespace.clone(),
        good_record_id.clone(),
        encrypted_wallet_backup_bytes(&good_wallet, &current_master_key, "good", 1).await,
    );
    globals.cloud.set_wallet_backup(
        namespace.clone(),
        bad_record_id.clone(),
        encrypted_wallet_backup_bytes(&bad_wallet, &other_master_key, "bad", 1).await,
    );
    globals.cloud.set_wallet_backup(
        namespace.clone(),
        recovered_record_id.clone(),
        encrypted_wallet_backup_bytes(
            &recovered_before_delete,
            &other_master_key,
            "recovered-later",
            1,
        )
        .await,
    );
    globals.cloud.set_wallet_backup(
        namespace.clone(),
        local_bad_record_id.clone(),
        encrypted_wallet_backup_bytes(&local_bad_wallet, &other_master_key, "local-bad", 1).await,
    );
    globals.cloud.set_wallet_files(
        namespace.clone(),
        vec![
            wallet_filename_from_record_id(&good_record_id),
            wallet_filename_from_record_id(&bad_record_id),
            wallet_filename_from_record_id(&recovered_record_id),
            wallet_filename_from_record_id(&local_bad_record_id),
        ],
    );
    mark_undecryptable_backups_reported(&manager, 3);

    let prepared = manager.prepare_delete_undecryptable_wallet_backups().await.unwrap();
    let candidates: HashSet<_> = prepared.record_ids().iter().cloned().collect();
    assert_eq!(candidates, HashSet::from([bad_record_id.clone(), recovered_record_id.clone()]));

    globals.cloud.set_wallet_backup(
        namespace.clone(),
        recovered_record_id.clone(),
        encrypted_wallet_backup_bytes(
            &recovered_before_delete,
            &current_master_key,
            "recovered",
            1,
        )
        .await,
    );
    let claim = CloudBackupExclusiveOperationClaim::new(
        CloudBackupExclusiveOperation::DeleteUndecryptableWalletBackups,
        1,
    );
    manager.project_exclusive_operation_started(claim);
    let writes = operation_write_client_for_test(&manager, claim);

    let deleted =
        manager.delete_prepared_undecryptable_wallet_backups(prepared, writes).await.unwrap();

    assert_eq!(deleted, 1);
    assert!(!globals.cloud.has_wallet_backup(&namespace, &bad_record_id));
    assert!(globals.cloud.has_wallet_backup(&namespace, &recovered_record_id));
    assert!(globals.cloud.has_wallet_backup(&namespace, &good_record_id));
    assert!(globals.cloud.has_wallet_backup(&namespace, &local_bad_record_id));
}
