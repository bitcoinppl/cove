use std::collections::BTreeMap;

use super::*;
use cove_cspp::backup_data::MASTER_KEY_RECORD_ID;
use cove_cspp::{MasterKeyPromotionActiveState, MasterKeyPromotionStatus};
use cove_types::network::Network;

use crate::manager::cloud_backup_manager::admission::ContinuationOwner;
use crate::manager::cloud_backup_manager::{
    CloudBackupOnboardingCompletionReadiness, PendingEnableJournalPhase,
    PendingEnableLocalMetadataSnapshot, PendingWork, PromotionStartError,
};

fn seed_retained_active_master() -> ([u8; 32], PendingEnableLocalMetadataSnapshot) {
    let master_key = cove_cspp::master_key::MasterKey::generate();
    let master_key_bytes = *master_key.as_bytes();
    cove_cspp::Cspp::new(Keychain::global().clone()).save_master_key(&master_key).unwrap();
    let cloud_keychain = CloudBackupKeychain::global();
    cloud_keychain
        .save_passkey_and_namespace(&[4, 5, 6], [8; 32], &master_key.namespace_id())
        .unwrap();

    (master_key_bytes, cloud_keychain.snapshot_passkey_metadata().unwrap())
}

fn assert_retained_active_master(
    expected_master_key: &[u8; 32],
    expected_metadata: &PendingEnableLocalMetadataSnapshot,
) {
    let active_master_key = cove_cspp::Cspp::new(Keychain::global().clone())
        .load_master_key_from_store()
        .unwrap()
        .unwrap();

    assert_eq!(active_master_key.as_bytes(), expected_master_key);
    assert_eq!(
        CloudBackupKeychain::global().snapshot_passkey_metadata().unwrap(),
        *expected_metadata
    );
}

#[tokio::test(flavor = "current_thread")]
async fn fresh_enable_retains_unowned_staged_master_key() {
    let _guard = async_test_lock().lock().await;
    let globals = test_globals();
    let manager = init_manager();

    reset_cloud_backup_test_state(&manager, globals);
    let cspp = cove_cspp::Cspp::new(Keychain::global().clone());
    let staged = cove_cspp::master_key::MasterKey::generate();
    let staged_bytes = *staged.as_bytes();
    cspp.save_staged_master_key(&staged).unwrap();

    let Err(error) = manager
        .pending_enable
        .stage_fresh_enable_master(CloudBackupEnableContext::settings_manual())
    else {
        panic!("expected unowned staged material to block enable");
    };

    assert!(matches!(error, CloudBackupError::Internal(_)));
    assert_eq!(cspp.master_key_promotion_status().unwrap(), MasterKeyPromotionStatus::Staged);
    assert_eq!(cspp.load_staged_master_key().unwrap().unwrap().as_bytes(), &staged_bytes);
    assert!(CloudBackupKeychain::global().load_pending_enable_journal().unwrap().is_none());
}

#[tokio::test(flavor = "current_thread")]
async fn fresh_enable_keeps_stage_ownership_when_the_stage_cannot_be_discarded() {
    let _guard = async_test_lock().lock().await;
    let globals = test_globals();
    let manager = init_manager();

    reset_cloud_backup_test_state(&manager, globals);

    // the staged key becomes unreadable once written, so neither its read-back nor the
    // rollback discard can finish
    const STAGED_MASTER_KEY: &str = "cspp::v1::staged_master_key";
    globals.keychain.set_after_save(|key| {
        if key == STAGED_MASTER_KEY {
            crate::test_support::shared_mock_keychain().fail_reads_for(STAGED_MASTER_KEY);
        }
    });

    let Err(error) = manager
        .pending_enable
        .stage_fresh_enable_master(CloudBackupEnableContext::settings_manual())
    else {
        panic!("expected the unreadable stage to fail the enable");
    };

    globals.keychain.set_after_save(|_| {});
    globals.keychain.clear_read_failures();

    assert!(error.is_keychain_unreadable(), "{error}");
    assert!(CloudBackupKeychain::global().load_pending_enable_journal().unwrap().is_some());
}

#[tokio::test(flavor = "current_thread")]
async fn enable_recovery_rolls_back_local_master_key_when_wallet_upload_fails() {
    let _guard = async_test_lock().lock().await;
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
    manager
        .pending_enable
        .save_enable_recovery_master_key(
            preparation.context,
            &preparation.active_namespace_id,
            &preparation.active_master_key,
            preparation.recovered_passkey_metadata(),
        )
        .unwrap();
    let (claim, writes) =
        begin_operation_writes_for_test(&manager, CloudBackupExclusiveOperation::Enable).await;
    let error = manager.prepare_enable_recovery_completion(preparation, writes).await.unwrap_err();
    let progress = manager.model_snapshot().progress.unwrap();
    assert_eq!((progress.completed, progress.total), (0, 1));
    finish_operation_for_test(&manager, claim).await;
    let journal = CloudBackupKeychain::global().load_pending_enable_journal().unwrap().unwrap();
    assert_eq!(
        journal.phase(),
        &PendingEnableJournalPhase::RemoteWritesStarted(PendingEnablePasskeyMetadata {
            credential_id: vec![1, 2, 3],
            prf_salt: [9; 32],
            provider_hint: None,
        })
    );
    manager.pending_enable.abort_recovered_existing_enable().unwrap();

    assert!(matches!(error, CloudBackupError::CloudStorage(_)));
    assert!(cspp.load_master_key_from_store().unwrap().is_none());
    assert_eq!(cspp.master_key_promotion_status().unwrap(), MasterKeyPromotionStatus::None);
    assert!(CloudBackupKeychain::global().load_pending_enable_journal().unwrap().is_none());
    assert_eq!(globals.cloud.delete_namespace_attempt_count(), 0);
}

#[tokio::test(flavor = "current_thread")]
async fn fresh_enable_upload_progress_counts_master_and_wallets() {
    let _guard = async_test_lock().lock().await;
    let globals = test_globals();
    let manager = init_manager();

    reset_cloud_backup_test_state(&manager, globals);
    CONNECTIVITY_MANAGER.set_connection_state(true);
    persist_xpub_wallets(vec![xpub_only_wallet_metadata(), xpub_only_wallet_metadata()]);
    globals.passkey.set_discover_result(Ok(DiscoveredPasskeyResult {
        prf_output: vec![7; 32],
        credential_id: vec![1, 2, 3],
    }));

    let preparation = manager
        .prepare_create_new_enable_passkey(
            CloudBackupEnableContext::settings_manual(),
            permit(PermitIntent::Enable),
        )
        .await
        .unwrap();
    let CloudBackupEnablePasskeyPreparation::Ready(ready) = preparation else {
        panic!("expected authenticated passkey material");
    };
    let (claim, writes) =
        begin_operation_writes_for_test(&manager, CloudBackupExclusiveOperation::Enable).await;
    manager.apply_enable_state(CloudBackupEnableState::UploadingBackup);

    let uploaded = manager.upload_ready_enable_backup(ready, writes).await.unwrap();
    let progress = manager.model_snapshot().progress.unwrap();
    finish_operation_for_test(&manager, claim).await;

    assert_eq!(uploaded.uploaded_wallets.len(), 2);
    assert_eq!((progress.completed, progress.total), (3, 3));

    reset_cloud_backup_test_state(&manager, globals);
}

#[tokio::test(flavor = "current_thread")]
async fn enable_recovery_rolls_back_local_master_key_when_keychain_save_fails() {
    let _guard = async_test_lock().lock().await;
    let globals = test_globals();
    let manager = init_manager();

    reset_cloud_backup_test_state(&manager, globals);
    // the configured backup's local key is gone, which is what recovers an existing backup
    let cspp = cove_cspp::Cspp::new(Keychain::global().clone());
    let cloud_keychain = CloudBackupKeychain::global();
    cloud_keychain.save_passkey_and_namespace(&[7, 8, 9], [8; 32], "prior-namespace").unwrap();
    let prior_metadata = cloud_keychain.snapshot_passkey_metadata().unwrap();

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
    manager
        .pending_enable
        .save_enable_recovery_master_key(
            preparation.context,
            &preparation.active_namespace_id,
            &preparation.active_master_key,
            preparation.recovered_passkey_metadata(),
        )
        .unwrap();
    assert!(cspp.load_master_key_from_store().unwrap().is_none());
    assert_eq!(cspp.master_key_promotion_status().unwrap(), MasterKeyPromotionStatus::Staged);
    assert_eq!(cloud_keychain.snapshot_passkey_metadata().unwrap(), prior_metadata);
    let staged_journal = cloud_keychain.load_pending_enable_journal().unwrap().unwrap();
    assert_eq!(
        staged_journal.namespace_ownership(),
        PendingEnableNamespaceOwnership::RecoveredExisting
    );
    assert!(matches!(staged_journal.phase(), PendingEnableJournalPhase::PasskeyRegistered(_)));
    let (claim, writes) =
        begin_operation_writes_for_test(&manager, CloudBackupExclusiveOperation::Enable).await;
    let completion = manager.prepare_enable_recovery_completion(preparation, writes).await.unwrap();
    finish_operation_for_test(&manager, claim).await;
    globals.keychain.fail_save_at(7);
    let error = manager
        .pending_enable
        .begin_enable_recovery_local_promotion(
            &completion.namespace_id,
            &completion.credential_id,
            completion.prf_salt,
        )
        .unwrap_err();

    assert!(matches!(error, PromotionStartError::Failed(CloudBackupError::Internal(_))));
    assert!(cspp.load_master_key_from_store().unwrap().is_none());
    assert_eq!(cloud_keychain.snapshot_passkey_metadata().unwrap(), prior_metadata);
    assert_eq!(
        cspp.master_key_promotion_status().unwrap(),
        MasterKeyPromotionStatus::Pending(MasterKeyPromotionActiveState::Prior)
    );
    let journal = cloud_keychain.load_pending_enable_journal().unwrap().unwrap();
    assert!(matches!(journal.phase(), PendingEnableJournalPhase::RemoteWritesStarted(_)));
    assert_eq!(journal.namespace_ownership(), PendingEnableNamespaceOwnership::RecoveredExisting);
    assert_eq!(globals.cloud.delete_namespace_attempt_count(), 0);

    manager.pending_enable.abort_recovered_existing_enable().unwrap();
    assert!(cspp.load_master_key_from_store().unwrap().is_none());
    assert_eq!(cspp.master_key_promotion_status().unwrap(), MasterKeyPromotionStatus::None);
    assert_eq!(cloud_keychain.snapshot_passkey_metadata().unwrap(), prior_metadata);
    assert_eq!(cloud_keychain.load_pending_enable_journal().unwrap(), None);
}

#[tokio::test(flavor = "current_thread")]
async fn failed_create_new_enable_does_not_persist_passkey_metadata() {
    let _guard = async_test_lock().lock().await;
    let globals = test_globals();
    let manager = init_manager();

    reset_cloud_backup_test_state(&manager, globals);
    globals.cloud.fail_master_key_upload("boom");
    globals.passkey.set_discover_result(Ok(DiscoveredPasskeyResult {
        prf_output: vec![7; 32],
        credential_id: vec![1, 2, 3],
    }));

    let error = enable_cloud_backup_create_new(&manager).await.unwrap_err();
    assert!(matches!(
        error,
        CloudBackupError::Internal(message)
            if message
                == crate::manager::cloud_backup_manager::error::GENERIC_CLOUD_BACKUP_ERROR_MESSAGE
    ));

    let keychain = Keychain::global();
    assert!(keychain.get(CSPP_CREDENTIAL_ID_KEY.into()).unwrap().is_none());
    assert!(keychain.get(CSPP_PRF_SALT_KEY.into()).unwrap().is_none());
    assert!(keychain.get(CSPP_NAMESPACE_ID_KEY.into()).unwrap().is_none());
}

#[tokio::test(flavor = "current_thread")]
async fn failed_no_discovery_confirmation_preserves_only_staged_material() {
    let _guard = async_test_lock().lock().await;
    let globals = test_globals();
    let manager = init_manager();

    reset_cloud_backup_test_state(&manager, globals);
    globals.passkey.set_create_result(Ok(vec![1, 2, 3]));
    globals.passkey.set_authenticate_result(Err(PasskeyError::RequestFailed {
        operation: PasskeyOperation::AuthenticateAssertion,
        reason: PasskeyFailureReason::Unknown { diagnostic_message: "boom".into() },
    }));

    enable_cloud_backup_no_discovery(&manager).await.unwrap();
    confirm_saved_passkey_session(&manager).await;

    let keychain = Keychain::global();
    let cspp = cove_cspp::Cspp::new(keychain.clone());
    assert!(cspp.load_master_key_from_store().unwrap().is_none());
    assert!(cspp.load_staged_master_key().unwrap().is_some());
    assert_eq!(cspp.master_key_promotion_status().unwrap(), MasterKeyPromotionStatus::Staged);
    assert!(take_pending_enable_session_for_test(&manager).await.is_some());
    let journal = CloudBackupKeychain::global().load_pending_enable_journal().unwrap().unwrap();
    assert_eq!(journal.namespace_ownership(), PendingEnableNamespaceOwnership::FreshOwned);
    assert!(matches!(journal.phase(), PendingEnableJournalPhase::PasskeyRegistered(_)));
    assert!(keychain.get(CSPP_CREDENTIAL_ID_KEY.into()).unwrap().is_none());
    assert!(keychain.get(CSPP_PRF_SALT_KEY.into()).unwrap().is_none());
    assert!(keychain.get(CSPP_NAMESPACE_ID_KEY.into()).unwrap().is_none());
}

#[tokio::test(flavor = "current_thread")]
async fn enable_create_new_succeeds_with_new_passkey_auth() {
    let _guard = async_test_lock().lock().await;
    let globals = test_globals();
    let manager = init_manager();

    reset_cloud_backup_test_state(&manager, globals);
    CONNECTIVITY_MANAGER.set_connection_state(true);
    globals.passkey.set_create_result(Ok(vec![1, 2, 3]));
    globals.passkey.set_authenticate_result(Ok(vec![7; 32]));

    enable_cloud_backup_create_new(&manager).await.unwrap();

    // the cloud still reports the new master key as syncing, so the pending upload worker
    // keeps Enable's upload confirmation pending instead of settling it mid-assertions
    globals.cloud.set_uploaded_master_key_pending_confirmation(true);
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

    // once the cloud shows the upload, the worker settles the confirmation and its loop goes
    // idle, so the test ends with no verifier still polling the shared database
    globals.cloud.set_uploaded_master_key_pending_confirmation(false);
    assert!(!verify_pending_uploads_once_for_test_async(&manager).await);
    assert!(manager.pending_verification_completion().is_none());
}

#[tokio::test(flavor = "current_thread")]
async fn detail_entry_loads_the_inventory_without_asking_for_the_passkey() {
    let _guard = async_test_lock().lock().await;
    let globals = test_globals();
    let manager = init_manager();

    configure_enabled_cloud_backup(&manager, globals, 0);
    seed_verifiable_cloud_master_key(globals);
    globals.passkey.set_discover_result(Err(PasskeyError::NotCompleted));

    let discover_count = globals.passkey.discover_count();
    let authenticate_count = globals.passkey.authenticate_count();

    call!(manager.supervisor.start_enter_detail()).await.unwrap();
    wait_for_detail_load_to_settle(&manager).await;

    assert_eq!(globals.passkey.discover_count(), discover_count);
    assert_eq!(globals.passkey.authenticate_count(), authenticate_count);
}

#[tokio::test(flavor = "current_thread")]
async fn check_again_joins_a_detail_load_the_inventory_already_shows() {
    use crate::manager::cloud_backup_manager::admission::{Admission, CoalescibleIntent};

    let _guard = async_test_lock().lock().await;
    let globals = test_globals();
    let manager = init_manager();

    configure_enabled_cloud_backup(&manager, globals, 0);
    manager.apply_detail_outcome(manager.inventory_scope(), CloudBackupDetailOutcome::Checking);

    dispatch_and_wait(&manager, CloudBackupManagerAction::RefreshDetail).await;

    let outcome = call!(manager.supervisor.harness_last_outcome())
        .await
        .unwrap()
        .expect("Check Again should have been dispatched");
    assert_eq!(outcome.admission, Admission::Coalesced(CoalescibleIntent::RefreshDetail));
    assert!(matches!(
        manager.inventory.read().detail_state(),
        CloudBackupDetailState::Checking { .. }
    ));
}

/// Waits until a detail load has finished, whether it completed or failed
async fn wait_for_detail_load_to_settle(manager: &RustCloudBackupManager) {
    wait_for_test_condition(Duration::from_secs(5), "detail load to settle", || {
        matches!(
            manager.inventory.read().detail_state(),
            CloudBackupDetailState::Complete { .. } | CloudBackupDetailState::Failed { .. }
        )
    })
    .await;
}

#[tokio::test(flavor = "current_thread")]
async fn detail_entry_does_not_restart_rust_owned_verification_states() {
    let _guard = async_test_lock().lock().await;
    let globals = test_globals();
    let manager = init_manager();

    let states = [
        VerificationState::Verifying,
        VerificationState::Verified(DeepVerificationReport {
            master_key_wrapper_repaired: false,
            local_master_key_repaired: false,
            credential_recovered: false,
            wallets_verified: 0,
            wallet_issues: Default::default(),
            detail: None,
        }),
        VerificationState::PasskeyConfirmed,
    ];

    for verification in states {
        configure_enabled_cloud_backup(&manager, globals, 0);
        manager.clear_pending_verification_completion();
        manager.reconcile_pending_upload_verification(PendingUploadVerificationState::Idle);
        manager.apply_verification_state(verification);
        globals.passkey.set_discover_result(Err(PasskeyError::NotCompleted));

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
        encrypted_remote_wallet_backup_bytes(&first_wallet, &first_master_key, &first_revision, 1)
            .await,
    );
    globals.cloud.set_wallet_backup(
        second_namespace.clone(),
        second_record_id.clone(),
        encrypted_remote_wallet_backup_bytes(
            &second_wallet,
            &second_master_key,
            &second_revision,
            1,
        )
        .await,
    );
    globals.cloud.set_wallet_backup(
        second_namespace.clone(),
        third_record_id.clone(),
        encrypted_remote_wallet_backup_bytes(&third_wallet, &second_master_key, &third_revision, 1)
            .await,
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
    assert_eq!(
        CloudBackupKeychain::global().namespace_id().unwrap(),
        Some(second_namespace.clone())
    );
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

// regression for #988: a backup written before the fix can carry a Signet wallet's master key as
// an xprv. Merging its namespace restores the key as a tprv, so the copy uploaded to the active
// namespace has a new revision hash, and cleanup must still delete the source namespace
#[tokio::test(flavor = "current_thread")]
async fn enable_merge_cleans_up_a_namespace_holding_a_master_key_in_another_network_form() {
    use bdk_wallet::bitcoin::{NetworkKind, bip32::Xpriv, key::Secp256k1};
    use cove_cspp::backup_data::WalletSecret as CloudWalletSecret;
    use cove_device::keychain::{WalletSecret as KeychainWalletSecret, WalletXprv};

    use crate::manager::cloud_backup_manager::wallets::payload_test_support;

    let _guard = async_test_lock().lock().await;
    let globals = test_globals();
    let manager = init_manager();

    reset_cloud_backup_test_state(&manager, globals);
    CONNECTIVITY_MANAGER.set_connection_state(true);
    globals.cloud.set_reflect_uploaded_wallets_in_listing(true);

    let prf_key = [7u8; 32];
    let source_master_key = cove_cspp::master_key::MasterKey::generate();
    let active_master_key = cove_cspp::master_key::MasterKey::generate();
    let source_namespace = source_master_key.namespace_id();
    let active_namespace = active_master_key.namespace_id();

    for (namespace, master_key, salt) in [
        (&source_namespace, &source_master_key, [9; 32]),
        (&active_namespace, &active_master_key, [8; 32]),
    ] {
        let encrypted =
            cove_cspp::master_key_crypto::encrypt_master_key(master_key, &prf_key, &salt).unwrap();
        globals
            .cloud
            .set_master_key_backup(namespace.clone(), serde_json::to_vec(&encrypted).unwrap());
    }

    globals.passkey.set_discover_result(Ok(DiscoveredPasskeyResult {
        prf_output: prf_key.to_vec(),
        credential_id: vec![1, 2, 3],
    }));
    globals.passkey.set_authenticate_result(Ok(prf_key.to_vec()));

    // the active namespace is the larger one, so it holds two watch-only wallets
    let mut active_record_files = Vec::new();
    for byte in [1, 2] {
        let wallet = WalletMetadata { master_fingerprint: None, ..xpub_only_wallet_metadata() };
        let mnemonic = Mnemonic::from_entropy(&[byte; 16]).unwrap();
        let xpub = crate::mnemonic::MnemonicExt::xpub(&mnemonic, wallet.network.into());
        Keychain::global().save_wallet_xpub(&wallet.id, xpub).unwrap();

        let record_id = cove_cspp::backup_data::wallet_record_id(wallet.id.as_ref());
        let revision = crate::manager::cloud_backup_manager::wallets::prepare_wallet_backup(
            &wallet,
            wallet.wallet_mode,
        )
        .await
        .unwrap()
        .revision_hash;

        globals.cloud.set_wallet_backup(
            active_namespace.clone(),
            record_id.clone(),
            encrypted_remote_wallet_backup_bytes(&wallet, &active_master_key, &revision, 1).await,
        );
        active_record_files.push(wallet_filename_from_record_id(&record_id));
    }

    globals.cloud.set_wallet_files(active_namespace.clone(), active_record_files);

    // the source namespace holds a Signet hot wallet whose key an older build uploaded as an xprv
    let tprv = Xpriv::new_master(NetworkKind::Test, &[93; 32]).unwrap();
    let mut legacy_xprv = tprv;
    legacy_xprv.network = NetworkKind::Main;

    let mut legacy_wallet = WalletMetadata::preview_new();
    legacy_wallet.network = cove_types::Network::Signet;
    legacy_wallet.wallet_type = WalletType::Hot;
    legacy_wallet.master_fingerprint = Some(Arc::new(
        crate::wallet::fingerprint::Fingerprint::from(tprv.fingerprint(&Secp256k1::new())),
    ));

    // a hot wallet keeps its xpub and public descriptors beside the secret, as a restore writes them
    let secret = KeychainWalletSecret::Xpriv(WalletXprv::try_from(legacy_xprv).unwrap());
    let xpub = crate::wallet_secret::WalletSecretExt::xpub(&secret, legacy_wallet.network);
    let descriptors = crate::wallet_secret::WalletSecretExt::into_descriptors(
        secret.clone(),
        legacy_wallet.network,
        legacy_wallet.address_type,
    );
    Keychain::global().save_wallet_secret(&legacy_wallet.id, secret).unwrap();
    Keychain::global().save_wallet_xpub(&legacy_wallet.id, xpub).unwrap();
    Keychain::global()
        .save_public_descriptor(
            &legacy_wallet.id,
            descriptors.external.extended_descriptor,
            descriptors.internal.extended_descriptor,
        )
        .unwrap();

    let mut legacy_entry = crate::manager::cloud_backup_manager::wallets::prepare_wallet_backup(
        &legacy_wallet,
        legacy_wallet.wallet_mode,
    )
    .await
    .unwrap()
    .entry;
    legacy_entry.secret = CloudWalletSecret::Xprv(legacy_xprv.to_string());
    legacy_entry.content_revision_hash =
        payload_test_support::content_revision_hash(&legacy_wallet, &legacy_entry);

    let critical_key = zeroize::Zeroizing::new(source_master_key.critical_data_key());
    let encrypted =
        cove_cspp::wallet_crypto::encrypt_wallet_entry(&legacy_entry, &critical_key).unwrap();
    assert!(Keychain::global().delete_wallet_items(&legacy_wallet.id));
    crate::wallet::delete_wallet_specific_data(&legacy_wallet.id).unwrap();

    let legacy_record_id = cove_cspp::backup_data::wallet_record_id(legacy_wallet.id.as_ref());
    globals.cloud.set_wallet_backup(
        source_namespace.clone(),
        legacy_record_id.clone(),
        serde_json::to_vec(&encrypted).unwrap(),
    );
    globals.cloud.set_wallet_files(
        source_namespace.clone(),
        vec![wallet_filename_from_record_id(&legacy_record_id)],
    );

    enable_cloud_backup_create_new(&manager).await.unwrap();

    assert_eq!(manager.current_status(), CloudBackupStatus::Enabled);
    assert_eq!(
        CloudBackupKeychain::global().namespace_id().unwrap(),
        Some(active_namespace.clone())
    );

    let restored = Keychain::global().get_wallet_secret(&legacy_wallet.id).unwrap().unwrap();
    assert_eq!(restored.as_xprv().unwrap().expose(), tprv.to_string());

    wait_for_test_condition(
        Duration::from_secs(1),
        "the merged source namespace should be deleted",
        || !globals.cloud.has_namespace(&source_namespace),
    )
    .await;

    let active_records =
        CloudStorage::global_explicit_client().list_wallet_backups(active_namespace).await.unwrap();
    assert!(active_records.contains(&legacy_record_id));
}

#[tokio::test(flavor = "current_thread")]
async fn enable_recovery_fails_closed_when_matched_wallet_listing_is_missing() {
    let _guard = async_test_lock().lock().await;
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

    assert_eq!(
        error.reader_message(),
        crate::manager::cloud_backup_manager::error::GENERIC_CLOUD_BACKUP_ERROR_MESSAGE
    );
    assert!(!error.to_string().contains("wallet files missing"), "{error}");
    assert_eq!(CloudBackupKeychain::global().namespace_id().unwrap(), None);
    assert_eq!(
        Database::global().cloud_backup_state.get().unwrap().status(),
        PersistedCloudBackupStatus::Disabled
    );
    assert!(globals.cloud.has_namespace(&empty_namespace));
    assert!(globals.cloud.has_namespace(&wallet_namespace));
}

#[tokio::test(flavor = "current_thread")]
async fn force_new_after_other_namespace_enter_detail_reuses_runtime_authorization() {
    let _guard = async_test_lock().lock().await;
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

    globals.passkey.set_discover_result(Err(PasskeyError::NotCompleted));
    let create_count = globals.passkey.create_count();
    let authenticate_count = globals.passkey.authenticate_count();
    let discover_count = globals.passkey.discover_count();

    call!(manager.supervisor.start_enter_detail()).await.unwrap();
    tokio::time::sleep(Duration::from_millis(50)).await;

    assert_eq!(globals.passkey.create_count(), create_count);
    assert_eq!(globals.passkey.authenticate_count(), authenticate_count);
    assert_eq!(globals.passkey.discover_count(), discover_count);
}

#[tokio::test(flavor = "current_thread")]
async fn fresh_start_new_keeps_prior_active_until_durable_success() {
    let _guard = async_test_lock().lock().await;
    let globals = test_globals();
    let manager = init_manager();

    reset_cloud_backup_test_state(&manager, globals);
    CONNECTIVITY_MANAGER.set_connection_state(true);
    let (prior_master_key, prior_metadata) = seed_retained_active_master();
    globals.passkey.set_create_result(Ok(vec![1, 2, 3]));
    globals.passkey.set_authenticate_result(Ok(vec![7; 32]));

    enable_cloud_backup_force_new(&manager).await.unwrap();

    let cspp = cove_cspp::Cspp::new(Keychain::global().clone());
    let staged_master_key = cspp.load_staged_master_key().unwrap().unwrap();
    let staged_master_key_bytes = *staged_master_key.as_bytes();
    let staged_namespace = staged_master_key.namespace_id();
    assert_ne!(staged_master_key_bytes, prior_master_key);
    assert_retained_active_master(&prior_master_key, &prior_metadata);
    assert_eq!(cspp.master_key_promotion_status().unwrap(), MasterKeyPromotionStatus::Staged);
    let journal = CloudBackupKeychain::global().load_pending_enable_journal().unwrap().unwrap();
    assert_eq!(journal.namespace_ownership(), PendingEnableNamespaceOwnership::FreshOwned);
    assert!(matches!(journal.phase(), PendingEnableJournalPhase::PasskeyRegistered(_)));
    assert!(!globals.cloud.has_master_key_backup(&staged_namespace));
    assert_eq!(
        manager.onboarding_enable_completion_readiness(),
        CloudBackupOnboardingCompletionReadiness::PendingEnableRecovery
    );

    // the cloud still reports the new master key as syncing, so the upload confirmation Enable
    // records stays pending instead of being settled by the pending upload worker mid-assertions
    globals.cloud.set_uploaded_master_key_pending_confirmation(true);
    confirm_saved_passkey_session(&manager).await;

    assert_eq!(manager.current_status(), CloudBackupStatus::Enabled);
    assert!(manager.pending_verification_completion().is_some());
    assert!(globals.cloud.has_master_key_backup(&staged_namespace));
    assert_eq!(
        cspp.load_master_key_from_store().unwrap().unwrap().as_bytes(),
        &staged_master_key_bytes
    );
    assert_eq!(cspp.master_key_promotion_status().unwrap(), MasterKeyPromotionStatus::None);
    assert!(cspp.load_staged_master_key().unwrap().is_none());
    assert!(CloudBackupKeychain::global().load_pending_enable_journal().unwrap().is_none());
    assert_eq!(
        CloudBackupKeychain::global().namespace_id().unwrap().as_deref(),
        Some(staged_namespace.as_str())
    );

    let relaunched_manager = init_manager();
    assert_eq!(
        relaunched_manager.onboarding_enable_completion_readiness(),
        CloudBackupOnboardingCompletionReadiness::Ready
    );
}

#[tokio::test(flavor = "current_thread")]
async fn onboarding_relaunch_readiness_fails_closed_during_pending_enable_recovery() {
    let _guard = async_test_lock().lock().await;
    let globals = test_globals();
    let manager = init_manager();

    configure_enabled_cloud_backup(&manager, globals, 0);
    let cloud_keychain = CloudBackupKeychain::global();
    assert_eq!(
        manager.onboarding_enable_completion_readiness(),
        CloudBackupOnboardingCompletionReadiness::Ready
    );

    let cspp = cove_cspp::Cspp::new(Keychain::global().clone());
    let staged_master_key = cove_cspp::master_key::MasterKey::generate();
    cspp.save_staged_master_key(&staged_master_key).unwrap();
    let journal = staged_pending_enable_journal(
        CloudBackupEnableContext {
            saved_passkey_confirmation: SavedPasskeyConfirmationMode::Automatic,
            verification_source: CloudBackupVerificationSource::Onboarding,
        },
        staged_master_key.namespace_id(),
        PendingEnableNamespaceOwnership::FreshOwned,
        cloud_keychain.snapshot_passkey_metadata().unwrap(),
    );
    cloud_keychain.save_pending_enable_journal(&journal).unwrap();

    assert!(matches!(manager.model_state().lifecycle, CloudBackupLifecycle::Configured(_)));
    assert_eq!(
        manager.onboarding_enable_completion_readiness(),
        CloudBackupOnboardingCompletionReadiness::PendingEnableRecovery
    );

    cloud_keychain.delete_pending_enable_journal().unwrap();
    assert_eq!(
        manager.onboarding_enable_completion_readiness(),
        CloudBackupOnboardingCompletionReadiness::PendingEnableRecovery
    );

    reset_cloud_backup_test_state(&manager, globals);
    assert_eq!(
        manager.onboarding_enable_completion_readiness(),
        CloudBackupOnboardingCompletionReadiness::NotReady
    );
}

#[tokio::test(flavor = "current_thread")]
async fn fresh_start_new_passkey_cancellation_preserves_prior_active_exactly() {
    let _guard = async_test_lock().lock().await;
    let globals = test_globals();
    let manager = init_manager();

    reset_cloud_backup_test_state(&manager, globals);
    CONNECTIVITY_MANAGER.set_connection_state(true);
    let (prior_master_key, prior_metadata) = seed_retained_active_master();
    globals.passkey.set_create_result(Err(PasskeyError::NotCompleted));

    enable_cloud_backup_force_new(&manager).await.unwrap();

    let cspp = cove_cspp::Cspp::new(Keychain::global().clone());
    assert_retained_active_master(&prior_master_key, &prior_metadata);
    assert_eq!(cspp.master_key_promotion_status().unwrap(), MasterKeyPromotionStatus::None);
    assert!(cspp.load_staged_master_key().unwrap().is_none());
    assert!(CloudBackupKeychain::global().load_pending_enable_journal().unwrap().is_none());
    assert_eq!(globals.cloud.delete_namespace_attempt_count(), 0);
}

#[tokio::test(flavor = "current_thread")]
async fn fresh_start_new_upload_failure_preserves_prior_active_exactly() {
    let _guard = async_test_lock().lock().await;
    let globals = test_globals();
    let manager = init_manager();

    reset_cloud_backup_test_state(&manager, globals);
    CONNECTIVITY_MANAGER.set_connection_state(true);
    let (prior_master_key, prior_metadata) = seed_retained_active_master();
    globals.passkey.set_create_result(Ok(vec![1, 2, 3]));
    globals.passkey.set_authenticate_result(Ok(vec![7; 32]));
    globals.cloud.fail_master_key_upload("upload failed");

    enable_cloud_backup_force_new(&manager).await.unwrap();

    let cspp = cove_cspp::Cspp::new(Keychain::global().clone());
    let staged_master_key = cspp.load_staged_master_key().unwrap().unwrap();
    let staged_master_key_bytes = *staged_master_key.as_bytes();
    assert_retained_active_master(&prior_master_key, &prior_metadata);

    confirm_saved_passkey_session(&manager).await;

    assert!(matches!(manager.current_status(), CloudBackupStatus::Error(_)));
    assert_retained_active_master(&prior_master_key, &prior_metadata);
    assert_eq!(
        cspp.load_staged_master_key().unwrap().unwrap().as_bytes(),
        &staged_master_key_bytes
    );
    assert_eq!(cspp.master_key_promotion_status().unwrap(), MasterKeyPromotionStatus::Staged);
    let journal = CloudBackupKeychain::global().load_pending_enable_journal().unwrap().unwrap();
    assert_eq!(journal.namespace_ownership(), PendingEnableNamespaceOwnership::FreshOwned);
    assert!(matches!(journal.phase(), PendingEnableJournalPhase::RemoteWritesStarted(_)));
    // the failed upload goes back to the journal's pause, which keeps no material in memory
    assert!(take_pending_enable_session_for_test(&manager).await.is_none());
    assert_eq!(
        manager.pending_enable.pending_work(),
        PendingWork::Paused(ContinuationOwner::Enable)
    );
    assert_eq!(globals.cloud.delete_namespace_attempt_count(), 0);
}

#[tokio::test(flavor = "current_thread")]
async fn fresh_start_new_saved_passkey_cancellation_preserves_prior_active_exactly() {
    let _guard = async_test_lock().lock().await;
    let globals = test_globals();
    let manager = init_manager();

    reset_cloud_backup_test_state(&manager, globals);
    CONNECTIVITY_MANAGER.set_connection_state(true);
    let (prior_master_key, prior_metadata) = seed_retained_active_master();
    let prior_namespace = CloudBackupKeychain::global().namespace_id().unwrap().unwrap();
    globals.cloud.set_master_key_backup(prior_namespace.clone(), vec![1, 2, 3]);
    globals.passkey.set_create_result(Ok(vec![1, 2, 3]));
    globals.passkey.set_authenticate_result(Err(PasskeyError::NotCompleted));

    enable_cloud_backup_force_new(&manager).await.unwrap();
    confirm_saved_passkey_session(&manager).await;

    let cspp = cove_cspp::Cspp::new(Keychain::global().clone());
    let staged_namespace = cspp.load_staged_master_key().unwrap().unwrap().namespace_id();
    assert_retained_active_master(&prior_master_key, &prior_metadata);
    assert_eq!(CloudBackupKeychain::global().namespace_id().unwrap().unwrap(), prior_namespace);
    assert_eq!(globals.cloud.master_key_backup_bytes(&prior_namespace), Some(vec![1, 2, 3]));
    assert!(!globals.cloud.has_master_key_backup(&staged_namespace));
    assert_eq!(cspp.master_key_promotion_status().unwrap(), MasterKeyPromotionStatus::Staged);
    assert!(take_pending_enable_session_for_test(&manager).await.is_some());
    assert_eq!(globals.cloud.delete_namespace_attempt_count(), 0);
}

#[tokio::test(flavor = "current_thread")]
async fn fresh_start_new_wallet_upload_failure_after_wrapper_preserves_prior_active() {
    let _guard = async_test_lock().lock().await;
    let globals = test_globals();
    let manager = init_manager();

    reset_cloud_backup_test_state(&manager, globals);
    CONNECTIVITY_MANAGER.set_connection_state(true);
    let (prior_master_key, prior_metadata) = seed_retained_active_master();
    persist_xpub_wallets(vec![xpub_only_wallet_metadata()]);
    globals.passkey.set_create_result(Ok(vec![1, 2, 3]));
    globals.passkey.set_authenticate_result(Ok(vec![7; 32]));
    globals.cloud.fail_wallet_backup_upload("upload failed");

    enable_cloud_backup_force_new(&manager).await.unwrap();
    confirm_saved_passkey_session(&manager).await;

    let cspp = cove_cspp::Cspp::new(Keychain::global().clone());
    let staged_namespace = cspp.load_staged_master_key().unwrap().unwrap().namespace_id();

    // the new wrapper reached the cloud and only the wallet upload failed, so the prior backup
    // must still be the active one
    assert!(globals.cloud.has_master_key_backup(&staged_namespace));
    assert!(globals.cloud.wallet_backup_upload_attempt_count() > 0);
    assert!(matches!(manager.current_status(), CloudBackupStatus::Error(_)));
    assert_retained_active_master(&prior_master_key, &prior_metadata);
    assert_ne!(CloudBackupKeychain::global().namespace_id().unwrap().unwrap(), staged_namespace);
    assert_eq!(globals.cloud.delete_namespace_attempt_count(), 0);
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn force_new_after_existing_backup_prompt_registers_without_discovery() {
    let _guard = async_test_lock().lock().await;
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
    globals.passkey.set_discover_result(Err(PasskeyError::NotCompleted));

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
    let existing_master_backup = serde_json::to_vec(&encrypted_existing_master).unwrap();
    globals.cloud.set_master_key_backup(existing_namespace.clone(), existing_master_backup.clone());
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

    // the existing passkey and backup are adopted, never replaced
    assert_eq!(globals.passkey.create_count(), 0);
    assert_eq!(CloudBackupKeychain::global().namespace_id().unwrap().unwrap(), existing_namespace);
    assert_eq!(
        globals.cloud.master_key_backup_bytes(&existing_namespace),
        Some(existing_master_backup)
    );

    let local_master_key = cove_cspp::Cspp::new(Keychain::global().clone())
        .load_master_key_from_store()
        .unwrap()
        .unwrap();
    assert_eq!(local_master_key.as_bytes(), existing_master_key.as_bytes());
}

#[tokio::test(flavor = "current_thread")]
async fn detail_entry_after_restart_never_asks_for_the_passkey() {
    let _guard = async_test_lock().lock().await;
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
    restarted_manager.apply_verification_state(VerificationState::Idle);
    Database::global().cloud_blob_sync_states.delete_all().unwrap();
    globals.passkey.set_discover_result(Err(PasskeyError::NotCompleted));

    let discover_count = globals.passkey.discover_count();
    let authenticate_count = globals.passkey.authenticate_count();

    call!(restarted_manager.supervisor.start_enter_detail()).await.unwrap();
    wait_for_detail_load_to_settle(&restarted_manager).await;

    assert_eq!(globals.passkey.discover_count(), discover_count);
    assert_eq!(globals.passkey.authenticate_count(), authenticate_count);
}

#[tokio::test(flavor = "current_thread")]
async fn detail_entry_refreshes_progressive_inventory_after_passkey_verification() {
    let _guard = async_test_lock().lock().await;
    let globals = test_globals();
    let manager = init_manager();

    reset_cloud_backup_test_state(&manager, globals);
    CONNECTIVITY_MANAGER.set_connection_state(true);
    globals.passkey.set_create_result(Ok(vec![1, 2, 3]));
    globals.passkey.set_authenticate_result(Ok(vec![7; 32]));

    enable_cloud_backup_no_discovery(&manager).await.unwrap();
    confirm_saved_passkey_session(&manager).await;

    manager.clear_pending_verification_completion();
    manager.reconcile_pending_upload_verification(PendingUploadVerificationState::Idle);
    manager.apply_verification_state(VerificationState::Idle);
    Database::global().cloud_blob_sync_states.delete_all().unwrap();
    globals.passkey.set_discover_result(Ok(DiscoveredPasskeyResult {
        prf_output: vec![7; 32],
        credential_id: vec![1, 2, 3],
    }));

    let snapshot_attempts = globals.cloud.list_wallet_files_snapshot_attempt_count();

    call!(manager.supervisor.start_enter_detail()).await.unwrap();
    wait_for_test_condition(Duration::from_secs(8), "detail inventory snapshot starts", || {
        globals.cloud.list_wallet_files_snapshot_attempt_count() > snapshot_attempts
    })
    .await;

    assert_eq!(globals.cloud.list_wallet_files_snapshot_attempt_count(), snapshot_attempts + 1);
}

#[tokio::test(flavor = "current_thread")]
async fn cancelled_enable_create_new_rolls_back_new_local_master_key() {
    let _guard = async_test_lock().lock().await;
    let globals = test_globals();
    let manager = init_manager();

    reset_cloud_backup_test_state(&manager, globals);
    globals.passkey.set_discover_result(Err(PasskeyError::NotCompleted));

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
    let globals = test_globals();
    let manager = init_manager();

    reset_cloud_backup_test_state(&manager, globals);
    globals.passkey.set_create_result(Err(PasskeyError::NotCompleted));

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
    let globals = test_globals();
    let manager = init_manager();

    reset_cloud_backup_test_state(&manager, globals);

    let local_master_key = cove_cspp::master_key::MasterKey::generate();
    let local_namespace_id = local_master_key.namespace_id();
    let cspp = cove_cspp::Cspp::new(Keychain::global().clone());
    cspp.save_master_key(&local_master_key).unwrap();
    globals.cloud.set_wallet_files(local_namespace_id.clone(), vec!["wallet-test.json".into()]);
    let passkey_metadata = CloudBackupKeychain::global().snapshot_passkey_metadata().unwrap();

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
    globals.passkey.set_discover_result(Err(PasskeyError::NotCompleted));

    let operation = new_restore_operation_for_test(&manager).await;
    let error = operation.restore_from_cloud_backup(&manager).await.unwrap_err();

    assert!(matches!(error, CloudBackupError::PasskeyDiscoveryCancelled));
    assert_eq!(CloudBackupKeychain::global().namespace_id().unwrap(), None);

    // the local master key and passkey metadata survive the cancelled restore untouched
    assert_eq!(
        cspp.load_master_key_from_store().unwrap().unwrap().as_bytes(),
        local_master_key.as_bytes()
    );
    assert_eq!(
        CloudBackupKeychain::global().snapshot_passkey_metadata().unwrap(),
        passkey_metadata
    );
}

/// A wallet the fresh enable test puts on the device, with what its backup must hold
struct FreshEnableWallet {
    metadata: WalletMetadata,
    mnemonic: Option<Mnemonic>,
    xpub: String,
    labels_jsonl: Option<&'static str>,
}

/// Saves a hot wallet's recovery words, xpub and public descriptors the way the app keeps them
fn save_fresh_enable_hot_wallet(
    name: &str,
    words: &str,
    network: Network,
    wallet_mode: WalletMode,
) -> FreshEnableWallet {
    use crate::wallet_secret::WalletSecretExt as _;

    let mnemonic = Mnemonic::parse(words).unwrap();
    let secret = cove_device::keychain::WalletSecret::Mnemonic(mnemonic.clone());

    let mut metadata = WalletMetadata::preview_new();
    metadata.name = name.into();
    metadata.network = network;
    metadata.wallet_mode = wallet_mode;
    metadata.wallet_type = WalletType::Hot;

    // the real fingerprint, so restore tells these wallets apart the way it does on a device
    let xpub = secret.xpub(network);
    metadata.master_fingerprint = Some(Arc::new(xpub.fingerprint().into()));
    let descriptors = secret.clone().into_descriptors(network, metadata.address_type);
    let keychain = Keychain::global();
    keychain.save_wallet_xpub(&metadata.id, xpub).unwrap();
    keychain
        .save_public_descriptor(
            &metadata.id,
            descriptors.external.extended_descriptor.clone(),
            descriptors.internal.extended_descriptor,
        )
        .unwrap();

    keychain.save_wallet_secret(&metadata.id, secret).unwrap();

    FreshEnableWallet {
        metadata,
        mnemonic: Some(mnemonic),
        xpub: xpub.to_string(),
        labels_jsonl: None,
    }
}

/// Saves wallets in three network and mode groups: on mainnet a hot wallet with labels and an
/// xpub-only wallet, a testnet hot wallet, and a hot wallet in decoy mode
fn save_fresh_enable_device_wallets() -> Vec<FreshEnableWallet> {
    let mut hot = save_fresh_enable_hot_wallet(
        "Fresh Enable Hot",
        "legal winner thank year wave sausage worth useful legal winner thank yellow",
        Network::Bitcoin,
        WalletMode::Main,
    );

    LabelManager::try_new(hot.metadata.id.clone())
        .unwrap()
        .import_without_cloud_backup_dirty(sample_labels_jsonl())
        .unwrap();

    hot.labels_jsonl = Some(sample_labels_jsonl());

    let mut watch = xpub_only_wallet_metadata();
    let watch_xpub: bdk_wallet::bitcoin::bip32::Xpub = sample_xpub(&watch).parse().unwrap();
    watch.master_fingerprint = Some(Arc::new(watch_xpub.fingerprint().into()));
    Keychain::global().save_wallet_xpub(&watch.id, watch_xpub).unwrap();
    let watch = FreshEnableWallet {
        xpub: sample_xpub(&watch),
        metadata: watch,
        mnemonic: None,
        labels_jsonl: None,
    };

    let testnet = save_fresh_enable_hot_wallet(
        "Fresh Enable Testnet",
        "letter advice cage absurd amount doctor acoustic avoid letter advice cage above",
        Network::Testnet,
        WalletMode::Main,
    );

    let decoy = save_fresh_enable_hot_wallet(
        "Fresh Enable Decoy",
        "zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo wrong",
        Network::Bitcoin,
        WalletMode::Decoy,
    );

    let wallets = Database::global().wallets();
    let mainnet = vec![hot.metadata.clone(), watch.metadata.clone()];
    wallets.save_all_wallets(Network::Bitcoin, WalletMode::Main, mainnet).unwrap();
    wallets
        .save_all_wallets(Network::Testnet, WalletMode::Main, vec![testnet.metadata.clone()])
        .unwrap();

    wallets
        .save_all_wallets(Network::Bitcoin, WalletMode::Decoy, vec![decoy.metadata.clone()])
        .unwrap();

    vec![hot, watch, testnet, decoy]
}

/// The restored wallet matches the one the fresh enable uploaded, from its metadata to its keys
/// and labels
async fn assert_fresh_enable_wallet_restored(wallet: &FreshEnableWallet) {
    let source = &wallet.metadata;
    let name = source.name.as_str();
    let restored = Database::global()
        .wallets()
        .get(&source.id, source.network, source.wallet_mode)
        .unwrap()
        .unwrap_or_else(|| panic!("{name} restored into its network and mode"));

    assert_eq!(restored.name, source.name);
    assert_eq!(restored.wallet_type, source.wallet_type, "{name} type");
    assert_eq!(restored.color, source.color, "{name} color");
    assert_eq!(restored.address_type, source.address_type, "{name} address type");
    assert_eq!(restored.master_fingerprint, source.master_fingerprint, "{name} fingerprint");

    let keychain = Keychain::global();
    assert_eq!(
        keychain.get_wallet_key(&source.id).unwrap().map(|mnemonic| mnemonic.to_string()),
        wallet.mnemonic.as_ref().map(ToString::to_string),
        "{name} recovery words"
    );

    assert_eq!(
        keychain.get_wallet_xpub(&source.id).unwrap().map(|xpub| xpub.to_string()),
        Some(wallet.xpub.clone()),
        "{name} xpub"
    );

    let exported = LabelManager::try_new(source.id.clone()).unwrap().export().await.unwrap();
    match wallet.labels_jsonl {
        Some(labels) => assert_labels_restored(name, labels, &exported),
        None => assert!(exported.trim().is_empty(), "{name} has no labels: {exported}"),
    }
}

#[tokio::test(flavor = "current_thread")]
async fn fresh_enable_on_empty_cloud_uploads_every_wallet_and_restores_it_on_a_new_device() {
    let _guard = async_test_lock().lock().await;
    let globals = test_globals();
    let manager = init_manager();

    reset_cloud_backup_test_state(&manager, globals);
    CONNECTIVITY_MANAGER.set_connection_state(true);
    globals.cloud.set_reflect_uploaded_wallets_in_listing(true);
    let wallets = save_fresh_enable_device_wallets();
    let prf_output = [0x5a; 32];
    globals.passkey.set_create_result(Ok(vec![9, 8, 7]));
    globals.passkey.set_authenticate_result(Ok(prf_output.to_vec()));

    enable_cloud_backup_create_new(&manager).await.unwrap();
    confirm_saved_passkey_session(&manager).await;

    assert_eq!(manager.current_status(), CloudBackupStatus::Enabled);
    assert_eq!(globals.passkey.create_count(), 1, "a fresh enable registers one passkey");
    assert_eq!(globals.passkey.authenticated_credential_ids(), vec![vec![9, 8, 7]]);

    // the master key backup opens with the new passkey's PRF output for the salt it was asked with
    let namespace = CloudBackupKeychain::global().namespace_id().unwrap().unwrap();
    let master_json =
        globals.cloud.master_key_backup_bytes(&namespace).expect("master key uploaded");

    let wrapper: cove_cspp::backup_data::EncryptedMasterKeyBackup =
        serde_json::from_slice(&master_json).unwrap();

    let asked_salt = globals.passkey.authenticated_requests()[0].prf_salt.clone();
    assert_eq!(wrapper.prf_salt.to_vec(), asked_salt, "master key wrapper PRF salt");

    let master_key =
        cove_cspp::master_key_crypto::decrypt_master_key(&wrapper, &prf_output).unwrap();

    assert_eq!(master_key.namespace_id(), namespace);
    assert_eq!(
        cove_cspp::Cspp::new(Keychain::global().clone())
            .load_master_key_from_store()
            .unwrap()
            .unwrap()
            .as_bytes(),
        master_key.as_bytes()
    );

    // every wallet on the device is in the cloud and opens with that master key
    let decrypt_wallet = |metadata: &WalletMetadata| {
        let record_id = wallet_record_id(metadata.id.as_ref());
        let bytes = globals
            .cloud
            .wallet_backup_bytes(&namespace, &record_id)
            .unwrap_or_else(|| panic!("{} uploaded", metadata.name));

        cove_cspp::wallet_crypto::decrypt_wallet_backup(
            &serde_json::from_slice(&bytes).unwrap(),
            &master_key.critical_data_key(),
        )
        .unwrap()
    };

    for wallet in &wallets {
        let name = wallet.metadata.name.as_str();
        let entry = decrypt_wallet(&wallet.metadata);

        match &wallet.mnemonic {
            Some(mnemonic) => assert!(
                matches!(&entry.secret, WalletSecret::Mnemonic(words) if *words == mnemonic.to_string()),
                "{name} recovery words"
            ),
            None => {
                assert!(matches!(entry.secret, WalletSecret::WatchOnly), "{name} is watch-only");
                assert_eq!(entry.xpub.as_ref(), Some(&wallet.xpub), "{name} xpub");
            }
        }

        assert_eq!(
            crate::manager::cloud_backup_manager::wallets::decode_cloud_labels_jsonl(&entry)
                .unwrap()
                .as_deref(),
            wallet.labels_jsonl,
            "{name} labels"
        );
    }

    // the background verifier reads the uploads back and reaches Verified
    for _ in 0..10 {
        if !verify_pending_uploads_once_for_test_async(&manager).await {
            break;
        }
    }

    match manager.model_snapshot().verification {
        VerificationState::Verified(report) => {
            assert_eq!(report.wallets_verified as usize, wallets.len(), "wallets verified");
            assert!(report.wallet_issues.is_empty(), "{:?}", report.wallet_issues);
        }

        other => panic!("expected Verified after a fresh enable, got {other:?}"),
    }

    assert_eq!(globals.passkey.create_count(), 1, "verification registered another passkey");

    // a new device restores those uploads, from the paths the enable wrote them to, and gets
    // every wallet back as it was
    let mut uploads = BTreeMap::new();
    for upload in globals.cloud.uploads() {
        if upload.namespace == namespace {
            uploads.insert(upload.record_id.clone(), upload);
        }
    }

    assert_eq!(uploads.len(), wallets.len() + 1, "master key and every wallet uploaded");

    reset_cloud_backup_test_state(&manager, globals);
    for wallet in &wallets {
        Keychain::global().delete_wallet_items(&wallet.metadata.id);
        crate::wallet::delete_wallet_specific_data(&wallet.metadata.id).unwrap();
    }

    let mut wallet_files = Vec::new();
    for upload in uploads.values() {
        if upload.record_id == MASTER_KEY_RECORD_ID {
            globals.cloud.set_master_key_backup(namespace.clone(), upload.data.clone());
            continue;
        }

        globals.cloud.set_wallet_backup(
            namespace.clone(),
            upload.record_id.clone(),
            upload.data.clone(),
        );

        wallet_files.push(upload.relative_path.clone());
    }

    globals.cloud.set_wallet_files(namespace.clone(), wallet_files);
    globals.passkey.set_discover_result(Ok(DiscoveredPasskeyResult {
        prf_output: prf_output.to_vec(),
        credential_id: vec![9, 8, 7],
    }));

    globals.passkey.set_authenticate_result(Ok(prf_output.to_vec()));

    let report = restore_like_onboarding(&manager).await;

    // the mock serves files by record id, so a restore reading another path than the upload
    // wrote would still find them here but not on a real provider
    let downloads = globals.cloud.download_requests();
    for download in &downloads {
        assert_eq!(download.namespace, namespace, "restore read another namespace");
    }

    for upload in uploads.values() {
        let read_paths: Vec<&str> = downloads
            .iter()
            .filter(|download| download.record_id == upload.record_id)
            .flat_map(|download| download.relative_paths.iter().map(String::as_str))
            .collect();

        assert!(
            read_paths.contains(&upload.relative_path.as_str()),
            "{} written but restore read {read_paths:?}",
            upload.relative_path
        );
    }

    assert_eq!(report.failed_wallet_errors, Vec::<String>::new(), "wallet restore errors");
    assert_eq!(report.labels_failed_errors, Vec::<String>::new(), "label restore errors");
    assert_eq!(report.wallets_restored as usize, wallets.len(), "wallets restored");

    for wallet in &wallets {
        assert_fresh_enable_wallet_restored(wallet).await;
    }

    assert_eq!(globals.passkey.create_count(), 0, "restore registered a passkey");

    reset_cloud_backup_test_state(&manager, globals);
    for wallet in &wallets {
        crate::wallet::delete_wallet_specific_data(&wallet.metadata.id).unwrap();
    }
}

/// A wipe while Enable waits for its saved-passkey confirmation ends that wait on screen: no
/// operation runs during the wait, so only the reset's projection can clear it
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn wipe_while_enable_awaits_its_saved_passkey_returns_enable_to_idle() {
    let _guard = async_test_lock().lock().await;
    let globals = test_globals();
    let manager = init_manager();

    reset_cloud_backup_test_state(&manager, globals);
    CONNECTIVITY_MANAGER.set_connection_state(true);
    globals.passkey.set_create_result(Ok(vec![1, 2, 3]));
    enable_cloud_backup_no_discovery(&manager).await.unwrap();
    assert!(matches!(
        manager.model_snapshot().enable_state,
        CloudBackupEnableState::AwaitingSavedPasskeyConfirmation(_)
    ));

    let permit = manager.prepare_local_reset().await.unwrap();
    permit.clear_local_state().await.unwrap();

    // the wipe's commit point for Cloud Backup's tables; the production wipe resets the whole
    // database, but this test shares its process database with other tests, so the wallet,
    // config, cache, price and diagnostics tables stay
    let db = Database::global();
    db.cloud_backup_state.delete().unwrap();
    db.cloud_blob_sync_states.delete_all().unwrap();
    permit.complete_after_database_reset().await.unwrap();

    assert_eq!(manager.model_snapshot().enable_state, CloudBackupEnableState::Idle);
}
