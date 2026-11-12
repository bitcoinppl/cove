use std::collections::BTreeSet;

use super::*;
use crate::manager::cloud_backup_manager::CloudBackupWalletVerificationIssues;
use crate::manager::cloud_backup_manager::owner::inventory::RecordRead;

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

/// The preparation reads only the records the user confirmed and not this device's own, and
/// reads as undecryptable only those the configured key fails to decrypt
#[tokio::test(flavor = "current_thread")]
async fn undecryptable_preparation_reads_only_confirmed_cloud_only_crypto_failures() {
    let _guard = async_test_lock().lock().await;
    let globals = test_globals();
    let manager = init_manager();
    configure_enabled_cloud_backup(&manager, globals, 0);

    let namespace = CloudBackupKeychain::global().namespace_id().unwrap().unwrap();
    let current_master_key = cove_cspp::Cspp::new(Keychain::global().clone())
        .load_master_key_from_store()
        .unwrap()
        .unwrap();
    let other_master_key = cove_cspp::master_key::MasterKey::generate();
    let good_wallet = xpub_only_wallet_metadata();
    let mut bad_wallet = xpub_only_wallet_metadata();
    bad_wallet.network = crate::network::Network::Testnet;
    let mut second_bad_wallet = xpub_only_wallet_metadata();
    second_bad_wallet.network = crate::network::Network::Signet;
    let mut local_bad_wallet = xpub_only_wallet_metadata();
    local_bad_wallet.network = crate::network::Network::Testnet4;
    let listed_after_confirmation = xpub_only_wallet_metadata();
    persist_xpub_wallets(vec![local_bad_wallet.clone()]);

    let good_record_id = wallet_record_id(good_wallet.id.as_ref());
    let bad_record_id = wallet_record_id(bad_wallet.id.as_ref());
    let second_bad_record_id = wallet_record_id(second_bad_wallet.id.as_ref());
    let local_bad_record_id = wallet_record_id(local_bad_wallet.id.as_ref());
    let listed_after_record_id = wallet_record_id(listed_after_confirmation.id.as_ref());
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
        second_bad_record_id.clone(),
        encrypted_wallet_backup_bytes(&second_bad_wallet, &other_master_key, "second-bad", 1).await,
    );
    globals.cloud.set_wallet_backup(
        namespace.clone(),
        local_bad_record_id.clone(),
        encrypted_wallet_backup_bytes(&local_bad_wallet, &other_master_key, "local-bad", 1).await,
    );
    globals.cloud.set_wallet_backup(
        namespace.clone(),
        listed_after_record_id.clone(),
        encrypted_wallet_backup_bytes(&listed_after_confirmation, &other_master_key, "after", 1)
            .await,
    );
    globals.cloud.set_wallet_files(
        namespace.clone(),
        vec![
            wallet_filename_from_record_id(&good_record_id),
            wallet_filename_from_record_id(&bad_record_id),
            wallet_filename_from_record_id(&second_bad_record_id),
            wallet_filename_from_record_id(&local_bad_record_id),
            wallet_filename_from_record_id(&listed_after_record_id),
        ],
    );
    mark_undecryptable_backups_reported(&manager, 4);

    // the confirmed batch: every record but the one listed after the user confirmed
    let confirmed = vec![
        good_record_id.clone(),
        bad_record_id.clone(),
        second_bad_record_id.clone(),
        local_bad_record_id.clone(),
    ];
    let read = call!(manager.supervisor.start_unreadable_read_for_test()).await.unwrap();
    let prepared = manager
        .prepare_delete_undecryptable_wallet_backups(
            &unowned_writes(
                &manager,
                CloudBackupExclusiveOperation::DeleteUndecryptableWalletBackups,
            ),
            confirmed,
            permit(PermitIntent::DeleteUndecryptableWalletBackups),
            read,
        )
        .await
        .unwrap();

    // every confirmed record not on this device is read, and only the crypto failures read as
    // undecryptable; the rest are evidence the owner applies to its rows
    let reads = &prepared.reads().reads;
    assert_eq!(
        reads.keys().collect::<BTreeSet<_>>(),
        BTreeSet::from([&good_record_id, &bad_record_id, &second_bad_record_id])
    );
    assert!(matches!(reads[&good_record_id], RecordRead::Readable(_)));
    assert_eq!(reads[&bad_record_id], RecordRead::Undecryptable);
    assert_eq!(reads[&second_bad_record_id], RecordRead::Undecryptable);
}
