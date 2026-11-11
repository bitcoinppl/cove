use super::*;
use crate::manager::cloud_backup_manager::admission::{Admission, Rejection};
use crate::manager::cloud_backup_manager::screen::{
    CloudBackupActionAvailability, CloudBackupDisableConfirmation, CloudBackupInventoryAssessment,
    CloudBackupScreenAction,
};

/// Turn Off ended before its commit point: the record is still configured and nothing was
/// deleted from `namespace`
fn assert_nothing_turned_off(globals: &TestGlobals, namespace: &str) {
    assert_eq!(
        Database::global().cloud_backup_state.get().unwrap().status(),
        PersistedCloudBackupStatus::Enabled
    );
    assert!(current_disable_generation().is_none());
    assert!(globals.cloud.has_namespace(namespace));
}

/// Backs up one local wallet in `namespace` and returns it with its record id
fn backed_up_local_wallet(globals: &TestGlobals, namespace: &str) -> (WalletMetadata, String) {
    let metadata = xpub_only_wallet_metadata();
    persist_xpub_wallets(vec![metadata.clone()]);
    let record_id = wallet_record_id(metadata.id.as_ref());
    globals
        .cloud
        .set_wallet_files(namespace.to_owned(), vec![wallet_filename_from_record_id(&record_id)]);

    (metadata, record_id)
}

#[tokio::test(flavor = "current_thread")]
async fn disable_cloud_backup_deletes_active_namespace_and_clears_local_cloud_state() {
    let _guard = async_test_lock().lock().await;
    let globals = test_globals();
    let manager = init_manager();
    configure_enabled_cloud_backup(&manager, globals, 0);

    let namespace = CloudBackupKeychain::global().namespace_id().unwrap().unwrap();
    globals.cloud.set_master_key_backup(namespace.clone(), vec![1, 2, 3]);
    Database::global()
        .cloud_blob_sync_states
        .set(&PersistedCloudBlobSyncState::master_key_wrapper(
            namespace.clone(),
            PersistedCloudBlobState::Dirty(CloudBlobDirtyState { changed_at: 1 }),
        ))
        .unwrap();

    // the inventory never loaded, so even an empty backup asks twice
    let review = review_turn_off(&manager).await;
    assert!(matches!(review.impact.assessment, CloudBackupInventoryAssessment::Unknown { .. }));
    assert_eq!(review.impact.confirmation, CloudBackupDisableConfirmation::Two);
    assert_eq!(globals.cloud.delete_namespace_attempt_count(), 0, "a review deletes nothing");

    confirm_turn_off(&manager).await;
    wait_for_turn_off_run(&manager).await;

    assert!(!globals.cloud.has_namespace(&namespace));
    assert_eq!(
        Database::global().cloud_backup_state.get().unwrap().status(),
        PersistedCloudBackupStatus::Disabled
    );
    assert!(Database::global().cloud_blob_sync_states.list().unwrap().is_empty());
    assert!(CloudBackupKeychain::global().namespace_id().unwrap().is_none());
    assert!(
        globals.cloud.deleted_namespace_policies().contains(&CloudAccessPolicy::ConsentAllowed)
    );
}

#[tokio::test(flavor = "current_thread")]
async fn disable_cloud_backup_keeps_disabling_state_when_local_cleanup_fails() {
    let _guard = async_test_lock().lock().await;
    let globals = test_globals();
    let manager = init_manager();
    configure_enabled_cloud_backup(&manager, globals, 0);

    let namespace = CloudBackupKeychain::global().namespace_id().unwrap().unwrap();
    globals.cloud.set_master_key_backup(namespace.clone(), vec![1, 2, 3]);
    globals.keychain.fail_delete_at(1);

    run_turn_off(&manager).await;

    assert_eq!(turn_off_failure(&manager), CloudBackupTurnOffFailure::Other);
    assert!(!globals.cloud.has_namespace(&namespace));
    let disabling = persisted_disabling();
    assert!(disabling.delete_started_at.is_some());
    assert!(disabling.last_error.as_deref() == Some(GENERIC_CLOUD_BACKUP_ERROR_MESSAGE));
    assert_eq!(current_disable_generation(), Some(disabling.disable_generation));

    try_again_turn_off(&manager).await;

    assert_eq!(
        Database::global().cloud_backup_state.get().unwrap().status(),
        PersistedCloudBackupStatus::Disabled
    );
    assert!(current_disable_generation().is_none());
}

/// Turn Off's local cleanup reaches Disabled from any partial state: whichever cleanup delete
/// fails, and whichever of the namespace, credential and master key are already gone when it
/// retries, the retry finishes and leaves none of them
#[tokio::test(flavor = "current_thread")]
async fn turn_off_cleanup_finishes_from_every_partial_local_state() {
    use cove_device::keychain::KeychainAccess as _;

    use crate::manager::cloud_backup_manager::keychain::{
        CSPP_CREDENTIAL_ID_KEY, CSPP_NAMESPACE_ID_KEY,
    };

    let _guard = async_test_lock().lock().await;
    let globals = test_globals();
    let manager = init_manager();
    let mut failed_deletes = 0;

    for failing_delete in 1.. {
        let mut reached_cleanup = false;
        for missing in 0..8u8 {
            configure_enabled_cloud_backup(&manager, globals, 0);
            let namespace = CloudBackupKeychain::global().namespace_id().unwrap().unwrap();
            globals.cloud.set_master_key_backup(namespace.clone(), vec![1, 2, 3]);

            globals.keychain.fail_delete_at(failing_delete);
            run_turn_off(&manager).await;
            globals.keychain.fail_delete_at(usize::MAX);

            // past the cleanup's last delete, nothing fails and Turn Off finishes at once
            let status = Database::global().cloud_backup_state.get().unwrap().status();
            if status == PersistedCloudBackupStatus::Disabled {
                break;
            }

            assert!(persisted_disabling().last_error.is_some(), "delete {failing_delete}");

            reached_cleanup = true;
            failed_deletes += 1;
            let keychain = CloudBackupKeychain::global();
            if missing & 1 != 0 {
                globals.keychain.delete(CSPP_NAMESPACE_ID_KEY.to_string());
            }
            if missing & 2 != 0 {
                globals.keychain.delete(CSPP_CREDENTIAL_ID_KEY.to_string());
            }
            if missing & 4 != 0 {
                keychain.delete_master_key().unwrap();
            }

            try_again_turn_off(&manager).await;
            let case = format!("delete {failing_delete} failed, missing set {missing:03b}");
            assert_eq!(
                Database::global().cloud_backup_state.get().unwrap().status(),
                PersistedCloudBackupStatus::Disabled,
                "{case}"
            );
            assert_eq!(keychain.namespace_id().unwrap(), None, "{case}");
            assert_eq!(keychain.load_credential_id().unwrap(), None, "{case}");
            assert!(!globals.cloud.has_namespace(&namespace), "{case}");
        }

        if !reached_cleanup {
            break;
        }
    }

    // the configured backup holds a master key, namespace and credential, so the cleanup makes
    // three deletes; each failed once, under every one of the 8 combinations
    assert_eq!(failed_deletes, 3 * 8);
}

/// A backup the inventory hasn't checked might be a wallet that isn't on this device, so the
/// review asks twice; nothing is deleted until the user confirms, and then everything is
#[tokio::test(flavor = "current_thread")]
async fn turn_off_asks_twice_for_backups_it_could_not_check_and_deletes_only_after_consent() {
    let _guard = async_test_lock().lock().await;
    let globals = test_globals();
    let manager = init_manager();
    configure_enabled_cloud_backup(&manager, globals, 0);

    let namespace = CloudBackupKeychain::global().namespace_id().unwrap().unwrap();
    globals.cloud.set_master_key_backup(namespace.clone(), vec![1, 2, 3]);
    globals
        .cloud
        .set_wallet_files(namespace.clone(), vec![wallet_filename_from_record_id("cloud-only")]);

    let review = review_turn_off(&manager).await;

    assert_eq!(review.impact.unchecked, 1);
    assert_eq!(review.impact.confirmation, CloudBackupDisableConfirmation::Two);
    // the inventory never loaded, so nothing classifies the record
    assert!(matches!(review.impact.assessment, CloudBackupInventoryAssessment::Unknown { .. }));
    assert_nothing_turned_off(globals, &namespace);

    confirm_turn_off(&manager).await;
    wait_for_turn_off_run(&manager).await;

    assert!(!globals.cloud.has_namespace(&namespace));
    assert_eq!(
        Database::global().cloud_backup_state.get().unwrap().status(),
        PersistedCloudBackupStatus::Disabled
    );
}

/// Backups made with another passkey are a line in the dialog, not a blocker, and Turn Off
/// leaves them in the cloud, where they can still be restored
#[tokio::test(flavor = "current_thread")]
async fn turn_off_leaves_backups_made_with_another_passkey() {
    let _guard = async_test_lock().lock().await;
    let globals = test_globals();
    let manager = init_manager();
    configure_enabled_cloud_backup(&manager, globals, 0);

    let namespace = CloudBackupKeychain::global().namespace_id().unwrap().unwrap();
    let other_namespace = cove_cspp::master_key::MasterKey::generate().namespace_id();
    globals.cloud.set_master_key_backup(namespace.clone(), vec![1, 2, 3]);
    globals.cloud.set_master_key_backup(other_namespace.clone(), vec![4, 5, 6]);

    let review = review_turn_off(&manager).await;

    assert_eq!((review.impact.other_backups, review.impact.unchecked), (1, 0));

    confirm_turn_off(&manager).await;
    wait_for_turn_off_run(&manager).await;

    assert!(!globals.cloud.has_namespace(&namespace));
    assert!(globals.cloud.has_namespace(&other_namespace));
    assert_eq!(
        Database::global().cloud_backup_state.get().unwrap().status(),
        PersistedCloudBackupStatus::Disabled
    );
}

/// Each review gets its own id and persists nothing, so a confirmation of an earlier one, such as
/// a tap that raced Check Again, is rejected and deletes nothing
#[tokio::test(flavor = "current_thread")]
async fn a_confirmation_of_an_earlier_review_is_rejected() {
    let _guard = async_test_lock().lock().await;
    let globals = test_globals();
    let manager = init_manager();
    configure_enabled_cloud_backup(&manager, globals, 0);

    let namespace = CloudBackupKeychain::global().namespace_id().unwrap().unwrap();
    globals.cloud.set_master_key_backup(namespace.clone(), vec![1, 2, 3]);

    let first = review_turn_off(&manager).await;
    let second = review_turn_off(&manager).await;
    assert_ne!(first.review_id, second.review_id);

    let review_id = first.review_id;
    dispatch_and_publish(&manager, CloudBackupManagerAction::ConfirmDisable { review_id }).await;

    let outcome = call!(manager.supervisor.harness_last_outcome()).await.unwrap();
    assert_eq!(
        outcome.map(|outcome| outcome.admission),
        Some(Admission::Rejected(Rejection::ReviewStale))
    );
    assert_eq!(turn_off_screen(&manager), CloudBackupDisableScreen::Reviewing(second));
    assert_eq!(manager.projected_exclusive_operation(), None);
    assert_nothing_turned_off(globals, &namespace);
}

/// A wallet deleted from this device after the review leaves its backup as the only copy, so the
/// recheck shows a new review instead of deleting it
#[tokio::test(flavor = "current_thread")]
async fn a_wallet_deleted_between_review_and_confirm_asks_again_and_deletes_nothing() {
    let _guard = async_test_lock().lock().await;
    let globals = test_globals();
    let manager = init_manager();
    configure_enabled_cloud_backup(&manager, globals, 0);

    let namespace = CloudBackupKeychain::global().namespace_id().unwrap().unwrap();
    globals.cloud.set_master_key_backup(namespace.clone(), vec![1, 2, 3]);
    let (metadata, _) = backed_up_local_wallet(globals, &namespace);

    let review = review_turn_off(&manager).await;
    assert_eq!((review.impact.own_backups, review.impact.unchecked), (1, 0));

    Database::global()
        .wallets()
        .save_all_wallets(metadata.network, metadata.wallet_mode, Vec::new())
        .unwrap();
    confirm_turn_off(&manager).await;
    wait_for_turn_off_run(&manager).await;

    let CloudBackupDisableScreen::Reviewing(again) = turn_off_screen(&manager) else {
        panic!("expected a new review, got {:?}", turn_off_screen(&manager));
    };
    assert!(again.changed_since_confirmation);
    assert_ne!(again.review_id, review.review_id);
    assert_eq!((again.impact.own_backups, again.impact.unchecked), (0, 1));
    assert_eq!(again.impact.confirmation, CloudBackupDisableConfirmation::Two);
    assert_eq!(globals.cloud.delete_namespace_attempt_count(), 0);
    assert_nothing_turned_off(globals, &namespace);
}

/// A backup that appears in the cloud after the review is something the user never saw, so the
/// recheck asks again
#[tokio::test(flavor = "current_thread")]
async fn a_backup_listed_after_the_review_asks_again_and_deletes_nothing() {
    let _guard = async_test_lock().lock().await;
    let globals = test_globals();
    let manager = init_manager();
    configure_enabled_cloud_backup(&manager, globals, 0);

    let namespace = CloudBackupKeychain::global().namespace_id().unwrap().unwrap();
    globals.cloud.set_master_key_backup(namespace.clone(), vec![1, 2, 3]);

    let review = review_turn_off(&manager).await;
    assert_eq!(review.impact.unchecked, 0);

    globals
        .cloud
        .set_wallet_files(namespace.clone(), vec![wallet_filename_from_record_id("arrived")]);
    confirm_turn_off(&manager).await;
    wait_for_turn_off_run(&manager).await;

    let CloudBackupDisableScreen::Reviewing(again) = turn_off_screen(&manager) else {
        panic!("expected a new review, got {:?}", turn_off_screen(&manager));
    };
    assert!(again.changed_since_confirmation);
    assert_eq!(again.impact.unchecked, 1);
    assert_nothing_turned_off(globals, &namespace);

    // consent to the new review goes through
    confirm_turn_off(&manager).await;
    wait_for_turn_off_run(&manager).await;

    assert!(!globals.cloud.has_namespace(&namespace));
}

/// Building the review is silent: it lists and counts, and never asks for a passkey
#[tokio::test(flavor = "current_thread")]
async fn turn_off_review_never_asks_for_a_passkey() {
    let _guard = async_test_lock().lock().await;
    let globals = test_globals();
    let manager = init_manager();
    configure_enabled_cloud_backup(&manager, globals, 0);

    let namespace = CloudBackupKeychain::global().namespace_id().unwrap().unwrap();
    globals.cloud.set_master_key_backup(namespace.clone(), vec![1, 2, 3]);
    backed_up_local_wallet(globals, &namespace);
    let passkey_calls = || {
        (
            globals.passkey.create_count(),
            globals.passkey.authenticate_count(),
            globals.passkey.discover_count(),
        )
    };
    let before = passkey_calls();

    review_turn_off(&manager).await;

    assert_eq!(passkey_calls(), before);
}

/// Cancel while the confirmed Turn Off waits on the write in flight ends it before the commit:
/// the upload finishes, ordinary writes run again and nothing is deleted
#[tokio::test(flavor = "current_thread")]
async fn cancel_during_the_drain_keeps_backup_on() {
    let _guard = async_test_lock().lock().await;
    let globals = test_globals();
    let manager = init_manager();
    configure_enabled_cloud_backup(&manager, globals, 0);

    let metadata = xpub_only_wallet_metadata();
    persist_xpub_wallets(vec![metadata.clone()]);
    let namespace = CloudBackupKeychain::global().namespace_id().unwrap().unwrap();
    globals.cloud.set_master_key_backup(namespace.clone(), vec![1, 2, 3]);
    let (upload_gate, upload) =
        hold_wallet_upload_in_flight(&manager, globals, metadata.id.clone()).await;

    review_turn_off(&manager).await;
    confirm_turn_off(&manager).await;
    wait_for_queued_lane_write(&manager).await;
    assert_eq!(turn_off_screen(&manager), CloudBackupDisableScreen::Draining);

    dispatch_and_publish(&manager, CloudBackupManagerAction::CancelDisable).await;

    assert_eq!(turn_off_screen(&manager), CloudBackupDisableScreen::Idle);
    assert_eq!(manager.projected_exclusive_operation(), None);

    upload_gate.release();
    upload.await.unwrap().unwrap();
    supervisor_checkpoint(&manager).await;

    assert_eq!(globals.cloud.delete_namespace_attempt_count(), 0);
    assert_nothing_turned_off(globals, &namespace);
}

/// The escape from a committed Turn Off that can't reach the provider: Try Again first, and once
/// that fails, Stop Using on This Device finishes here and leaves the remote backup
#[tokio::test(flavor = "current_thread")]
async fn stop_using_on_this_device_is_offered_after_a_failed_try_again() {
    let _guard = async_test_lock().lock().await;
    let globals = test_globals();
    let manager = init_manager();
    configure_enabled_cloud_backup(&manager, globals, 0);

    let namespace = CloudBackupKeychain::global().namespace_id().unwrap().unwrap();
    globals.cloud.set_master_key_backup(namespace.clone(), vec![1, 2, 3]);
    globals.cloud.fail_delete_namespace("delete failed");
    let stop_using = |manager: &RustCloudBackupManager| {
        manager
            .published_screen()
            .actions
            .entries
            .into_iter()
            .find(|entry| entry.action == CloudBackupScreenAction::StopUsingOnThisDevice)
            .map(|entry| entry.availability)
    };

    run_turn_off(&manager).await;

    assert!(
        matches!(stop_using(&manager), Some(CloudBackupActionAvailability::Unavailable { .. })),
        "Stop Using waits for a failed Try Again"
    );

    try_again_turn_off(&manager).await;

    assert_eq!(stop_using(&manager), Some(CloudBackupActionAvailability::Available));

    dispatch_and_publish(&manager, CloudBackupManagerAction::StopUsingOnThisDevice).await;
    wait_for_turn_off_run(&manager).await;

    assert_eq!(
        Database::global().cloud_backup_state.get().unwrap().status(),
        PersistedCloudBackupStatus::Disabled
    );
    assert!(CloudBackupKeychain::global().namespace_id().unwrap().is_none());
    assert!(globals.cloud.has_namespace(&namespace), "the remote backup stays");
}

#[tokio::test(flavor = "current_thread")]
async fn disable_corrupted_persisted_state_fails_closed() {
    let _guard = async_test_lock().lock().await;
    let globals = test_globals();
    let manager = init_manager();

    reset_cloud_backup_test_state(&manager, globals);
    manager
        .persist_cloud_backup_state(
            &PersistedCloudBackupState::corrupted("decode failed"),
            "persist corrupt state for test",
        )
        .unwrap();
    globals.cloud.fail_list_wallet_files("disable should not inspect cloud wallets");
    let list_wallet_files_attempt_count = globals.cloud.list_wallet_files_attempt_count();

    dispatch_and_publish(&manager, CloudBackupManagerAction::DisableCloudBackup).await;
    supervisor_checkpoint(&manager).await;

    assert_eq!(turn_off_screen(&manager), CloudBackupDisableScreen::Idle);
    assert_eq!(
        Database::global().cloud_backup_state.get().unwrap().status(),
        PersistedCloudBackupStatus::Corrupted
    );
    assert!(matches!(
        manager.current_status(),
        CloudBackupStatus::Error(message) if message == CORRUPTED_CLOUD_BACKUP_STATE_MESSAGE
    ));
    assert_eq!(globals.cloud.list_wallet_files_attempt_count(), list_wallet_files_attempt_count);
}

#[tokio::test(flavor = "current_thread")]
async fn disable_cloud_backup_delete_failure_keeps_disabling_state_and_keychain() {
    let _guard = async_test_lock().lock().await;
    let globals = test_globals();
    let manager = init_manager();
    configure_enabled_cloud_backup(&manager, globals, 0);

    let namespace = CloudBackupKeychain::global().namespace_id().unwrap().unwrap();
    globals.cloud.set_master_key_backup(namespace.clone(), vec![1, 2, 3]);
    globals.cloud.fail_delete_namespace("delete failed");

    run_turn_off(&manager).await;

    assert_eq!(turn_off_failure(&manager), CloudBackupTurnOffFailure::Other);
    let disabling = persisted_disabling();
    assert!(disabling.delete_started_at.is_some());
    assert_eq!(disabling.last_error.as_deref(), Some(GENERIC_CLOUD_BACKUP_ERROR_MESSAGE));
    assert_eq!(
        CloudBackupKeychain::global().namespace_id().unwrap().as_deref(),
        Some(namespace.as_str())
    );
    assert_eq!(current_disable_generation(), Some(disabling.disable_generation));
}

/// A committed Turn Off whose delete fails and whose failure can't be saved still shows the
/// failure and keeps its commit, so Try Again finishes it once the faults clear
#[tokio::test(flavor = "current_thread")]
async fn turn_off_whose_failure_cannot_be_saved_finishes_on_try_again() {
    let _guard = async_test_lock().lock().await;
    let globals = test_globals();
    let manager = init_manager();
    configure_enabled_cloud_backup(&manager, globals, 0);

    let namespace = CloudBackupKeychain::global().namespace_id().unwrap().unwrap();
    globals.cloud.set_master_key_backup(namespace.clone(), vec![1, 2, 3]);
    globals.cloud.fail_delete_namespace("delete failed");
    Database::global().cloud_backup_state.fail_next_write_where(|state| {
        state.disabling().is_some_and(|disabling| disabling.last_error.is_some())
    });

    run_turn_off(&manager).await;

    assert_eq!(turn_off_failure(&manager), CloudBackupTurnOffFailure::Other);
    let disabling = persisted_disabling();
    assert!(disabling.delete_started_at.is_some());
    assert_eq!(disabling.last_error, None, "the failure was saved after all");
    assert!(globals.cloud.has_namespace(&namespace));

    globals.cloud.clear_delete_namespace_failure();
    try_again_turn_off(&manager).await;

    assert_eq!(
        Database::global().cloud_backup_state.get().unwrap().status(),
        PersistedCloudBackupStatus::Disabled
    );
    assert!(!globals.cloud.has_namespace(&namespace));
}

/// A listing that keeps failing never blocks Turn Off: Cove couldn't check what the backup
/// holds, so the review asks twice, and nothing is deleted before that consent
#[tokio::test(flavor = "current_thread")]
async fn a_failed_listing_asks_twice_and_deletes_nothing_until_confirmed() {
    let _guard = async_test_lock().lock().await;
    let globals = test_globals();
    let manager = init_manager();
    configure_enabled_cloud_backup(&manager, globals, 0);

    let namespace = CloudBackupKeychain::global().namespace_id().unwrap().unwrap();
    globals.cloud.set_master_key_backup(namespace.clone(), vec![1, 2, 3]);
    globals.cloud.fail_list_wallet_files_for_namespace(
        namespace.clone(),
        CloudStorageError::NotFound("missing".into()),
    );

    let review = review_turn_off(&manager).await;

    assert!(matches!(review.impact.assessment, CloudBackupInventoryAssessment::Unknown { .. }));
    assert_eq!(review.impact.own_backups, 0);
    assert_eq!(review.impact.confirmation, CloudBackupDisableConfirmation::Two);
    assert_eq!(globals.cloud.list_wallet_files_attempt_count_for_namespace(&namespace), 4);
    assert_eq!(globals.cloud.delete_namespace_attempt_count(), 0);
    assert_nothing_turned_off(globals, &namespace);

    confirm_turn_off(&manager).await;
    wait_for_turn_off_run(&manager).await;

    assert!(!globals.cloud.has_namespace(&namespace));
    assert_eq!(
        Database::global().cloud_backup_state.get().unwrap().status(),
        PersistedCloudBackupStatus::Disabled
    );
}

#[tokio::test(flavor = "current_thread")]
async fn disable_cloud_backup_delete_not_found_finishes_cleanup() {
    let _guard = async_test_lock().lock().await;
    let globals = test_globals();
    let manager = init_manager();
    configure_enabled_cloud_backup(&manager, globals, 0);

    let namespace = CloudBackupKeychain::global().namespace_id().unwrap().unwrap();
    globals.cloud.set_master_key_backup(namespace.clone(), vec![1, 2, 3]);
    globals.cloud.fail_delete_namespace_not_found("already deleted");

    run_turn_off(&manager).await;

    assert_eq!(
        Database::global().cloud_backup_state.get().unwrap().status(),
        PersistedCloudBackupStatus::Disabled
    );
    assert!(CloudBackupKeychain::global().namespace_id().unwrap().is_none());
}

#[tokio::test(flavor = "current_thread")]
async fn persisted_disabling_on_restart_resumes_to_disabled() {
    let _guard = async_test_lock().lock().await;
    let globals = test_globals();
    let manager = init_manager();
    configure_enabled_cloud_backup(&manager, globals, 0);

    let namespace = CloudBackupKeychain::global().namespace_id().unwrap().unwrap();
    globals.cloud.set_master_key_backup(namespace.clone(), vec![1, 2, 3]);
    let PersistedCloudBackupState::Configured(previous_configured) =
        Database::global().cloud_backup_state.get().unwrap()
    else {
        panic!("expected configured cloud backup state");
    };

    Database::global()
        .cloud_backup_state
        .set(&PersistedCloudBackupState::disabling_transition(PersistedDisablingCloudBackup {
            previous_configured,
            namespace_id: namespace.clone(),
            disable_generation: 42,
            started_at: 41,
            delete_started_at: Some(43),
            last_error: Some("interrupted".into()),
            retry_after: Some(44),
            disposition: PersistedDisableDisposition::DeleteRemote,
            try_again_failed: false,
        }))
        .unwrap();

    let restarted_manager = init_manager();
    wait_for_test_condition(Duration::from_secs(2), "restart disable recovery finishes", || {
        !globals.cloud.has_namespace(&namespace)
            && Database::global().cloud_backup_state.get().unwrap().status()
                == PersistedCloudBackupStatus::Disabled
            && CloudBackupKeychain::global().namespace_id().unwrap().is_none()
            && restarted_manager.model_snapshot().status == CloudBackupStatus::Disabled
    })
    .await;

    assert!(!globals.cloud.has_namespace(&namespace));
    assert_eq!(
        Database::global().cloud_backup_state.get().unwrap().status(),
        PersistedCloudBackupStatus::Disabled
    );
    assert!(CloudBackupKeychain::global().namespace_id().unwrap().is_none());
    assert_eq!(restarted_manager.model_snapshot().status, CloudBackupStatus::Disabled);
}

/// Turn Off's drain and namespace delete are lane commands: while an upload is the write in
/// flight, the disable neither commits nor deletes, and it finishes once the upload returns
#[tokio::test(flavor = "current_thread")]
async fn turn_off_waits_for_the_write_in_flight_before_it_commits_and_deletes() {
    let _guard = async_test_lock().lock().await;
    let globals = test_globals();
    let manager = init_manager();
    configure_enabled_cloud_backup(&manager, globals, 0);

    let metadata = xpub_only_wallet_metadata();
    persist_xpub_wallets(vec![metadata.clone()]);
    let namespace = CloudBackupKeychain::global().namespace_id().unwrap().unwrap();
    globals.cloud.set_master_key_backup(namespace.clone(), vec![1, 2, 3]);
    let (upload_gate, upload) =
        hold_wallet_upload_in_flight(&manager, globals, metadata.id.clone()).await;

    review_turn_off(&manager).await;
    confirm_turn_off(&manager).await;
    wait_for_queued_lane_write(&manager).await;

    assert!(matches!(
        Database::global().cloud_backup_state.get().unwrap(),
        PersistedCloudBackupState::Configured(_)
    ));
    assert_eq!(globals.cloud.delete_namespace_attempt_count(), 0);

    upload_gate.release();
    upload.await.unwrap().unwrap();
    wait_for_test_condition(
        Duration::from_secs(5),
        "the disable finishes after the upload",
        || manager.projected_exclusive_operation().is_none(),
    )
    .await;

    assert!(!globals.cloud.has_namespace(&namespace));
    assert_eq!(
        Database::global().cloud_backup_state.get().unwrap().status(),
        PersistedCloudBackupStatus::Disabled
    );
}
