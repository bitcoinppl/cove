use super::*;

#[derive(Clone, Copy)]
enum DriveAccountSwitchReleaseForTest {
    Commit,
    Rollback,
}

async fn assert_drive_account_switch_release_resumes_dirty_wallet(
    release: DriveAccountSwitchReleaseForTest,
) {
    cove_tokio::init();
    let globals = test_globals();
    let manager = init_manager();
    configure_enabled_cloud_backup(&manager, globals, 0);
    let metadata = xpub_only_wallet_metadata();
    let record_id = wallet_record_id(metadata.id.as_ref());
    persist_xpub_wallets(vec![metadata.clone()]);
    let initial_upload_attempt_count = globals.cloud.wallet_backup_upload_attempt_count();

    let transition_id = manager.begin_drive_account_switch().await.unwrap();
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

            manager.confirm_drive_account_switch_committed(transition_id).await.unwrap();
        }
        DriveAccountSwitchReleaseForTest::Rollback => {
            manager.cancel_drive_account_switch(transition_id).await.unwrap();
            manager.confirm_drive_account_switch_rolled_back(transition_id).await.unwrap();
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
