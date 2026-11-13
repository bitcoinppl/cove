use super::*;
use crate::manager::cloud_backup_manager::DriveAccountSwitchPlatformResult as PlatformResult;
use crate::manager::cloud_backup_manager::DriveAccountSwitchPlatformState;
use crate::manager::cloud_backup_manager::RequestedDriveAccountSwitchStep;
use crate::manager::cloud_backup_manager::admission::UserIntent;

/// Starts a switch the way the Switch control does and returns its transition
///
/// The platform acts only on the steps Rust asks for, so this waits for the account picker
/// request, which comes once the write lane has drained
pub(crate) async fn start_drive_account_switch(manager: &RustCloudBackupManager) -> u64 {
    call!(manager.supervisor.dispatch(UserIntent::SwitchDriveAccount)).await.unwrap();
    let transition_id = RustCloudBackupManager::load_persisted_state()
        .drive_account_switch()
        .expect("the switch is recorded")
        .transition_id
        .value();

    let select = RequestedDriveAccountSwitchStep::SelectAccount(transition_id);
    wait_for_test_condition(
        Duration::from_secs(8),
        "the switch asks the platform for an account",
        || manager.drained_drive_account_switch_steps().contains(&select),
    )
    .await;

    transition_id
}

/// The platform reports the account it staged for a switch whose reinitialize succeeded, and the
/// switch asks it to commit that account
///
/// The switch moves only on the answer to a step it asked for, so a test that put the switch in
/// its commit phase directly has the platform report its state, as it does at launch
async fn platform_asks_for_the_commit(manager: &RustCloudBackupManager, transition_id: u64) {
    manager
        .drive_account_switch_platform_result_handled(PlatformResult::State(
            DriveAccountSwitchPlatformState::Staged(transition_id),
        ))
        .await;

    let commit = RequestedDriveAccountSwitchStep::Commit(transition_id);
    assert!(manager.drained_drive_account_switch_steps().contains(&commit));
}

#[derive(Clone, Copy)]
enum DriveAccountSwitchReleaseForTest {
    Commit,
    Rollback,
}

async fn assert_drive_account_switch_release_resumes_dirty_wallet(
    release: DriveAccountSwitchReleaseForTest,
) {
    let globals = test_globals();
    let manager = init_manager();
    configure_enabled_cloud_backup(&manager, globals, 0);
    let metadata = xpub_only_wallet_metadata();
    let record_id = wallet_record_id(metadata.id.as_ref());
    persist_xpub_wallets(vec![metadata.clone()]);
    let initial_upload_attempt_count = globals.cloud.wallet_backup_upload_attempt_count();

    let transition_id = start_drive_account_switch(&manager).await;
    manager.handle_wallet_backup_change(metadata.id);

    assert_eq!(globals.cloud.wallet_backup_upload_attempt_count(), initial_upload_attempt_count);
    assert!(matches!(
        Database::global().cloud_blob_sync_states.get(&record_id).unwrap(),
        Some(PersistedCloudBlobSyncState { state: PersistedCloudBlobState::Dirty(_), .. })
    ));

    match release {
        DriveAccountSwitchReleaseForTest::Commit => {
            let mut persisted = RustCloudBackupManager::load_persisted_state();
            assert!(persisted.set_drive_account_switch_phase(
                transition_id.into(),
                PersistedDriveAccountSwitchPhase::AwaitingAccountCommitSucceeded,
            ));
            Database::global().cloud_backup_state.set(&persisted).unwrap();
            platform_asks_for_the_commit(&manager, transition_id).await;

            manager
                .drive_account_switch_platform_result_handled(PlatformResult::Committed(
                    transition_id,
                ))
                .await;
        }
        DriveAccountSwitchReleaseForTest::Rollback => {
            // the user backed out of the picker, so the platform discards the staged account
            manager
                .drive_account_switch_platform_result_handled(PlatformResult::AccountUnchanged(
                    transition_id,
                ))
                .await;
            manager
                .drive_account_switch_platform_result_handled(PlatformResult::RolledBack(
                    transition_id,
                ))
                .await;
        }
    }

    wait_for_test_condition(
        Duration::from_secs(5),
        "dirty wallet upload resumes after Google Drive account switch",
        || {
            let upload_attempted =
                globals.cloud.wallet_backup_upload_attempt_count() > initial_upload_attempt_count;
            let uploaded_or_confirmed = matches!(
                Database::global().cloud_blob_sync_states.get(&record_id).unwrap(),
                Some(PersistedCloudBlobSyncState {
                    state: PersistedCloudBlobState::UploadedPendingConfirmation(_)
                        | PersistedCloudBlobState::Confirmed(_),
                    ..
                })
            );

            upload_attempted && uploaded_or_confirmed
        },
    )
    .await;

    clear_wallet_upload_runtime_for_test_async(&manager).await;
}

#[tokio::test(flavor = "current_thread")]
async fn drive_account_switch_commit_resumes_dirty_wallet_uploads() {
    let _guard = async_test_lock().lock().await;

    assert_drive_account_switch_release_resumes_dirty_wallet(
        DriveAccountSwitchReleaseForTest::Commit,
    )
    .await;
}

#[tokio::test(flavor = "current_thread")]
async fn drive_account_switch_rollback_resumes_dirty_wallet_uploads() {
    let _guard = async_test_lock().lock().await;

    assert_drive_account_switch_release_resumes_dirty_wallet(
        DriveAccountSwitchReleaseForTest::Rollback,
    )
    .await;
}

#[tokio::test(flavor = "current_thread")]
async fn drive_account_switch_commit_releases_its_claim_on_a_running_supervisor() {
    let _guard = async_test_lock().lock().await;
    let globals = test_globals();
    let manager = init_manager();
    configure_enabled_cloud_backup(&manager, globals, 0);
    seed_verifiable_cloud_master_key(globals);
    CONNECTIVITY_MANAGER.set_connection_state(true);

    // the platform has signed in to the new account and reinitialized the backup there
    let transition_id = start_drive_account_switch(&manager).await;
    let mut persisted = RustCloudBackupManager::load_persisted_state();
    assert!(persisted.set_drive_account_switch_phase(
        transition_id.into(),
        PersistedDriveAccountSwitchPhase::AwaitingAccountCommitSucceeded,
    ));

    Database::global().cloud_backup_state.set(&persisted).unwrap();
    platform_asks_for_the_commit(&manager, transition_id).await;

    manager
        .drive_account_switch_platform_result_handled(PlatformResult::Committed(transition_id))
        .await;

    // the commit hands the slot to a verification of the new account's backup, which reads the
    // cloud and releases the slot once it reports; a commit that never schedules it leaves the
    // switch's claim held here
    wait_for_test_condition(
        Duration::from_secs(8),
        "the running supervisor releases the switch's claim after the commit",
        || manager.projected_exclusive_operation().is_none(),
    )
    .await;

    let CloudBackupLifecycle::Configured(configured) = manager.model_state().lifecycle else {
        panic!("expected configured cloud backup lifecycle");
    };

    assert_eq!(configured.destructive_operation, CloudBackupDestructiveOperationState::Idle);
    assert!(
        matches!(configured.verification, CloudBackupVerificationState::Verified { .. }),
        "the new account's backup should verify, got {:?}",
        configured.verification
    );

    clear_wallet_upload_runtime_for_test_async(&manager).await;
}
