//! Write-policy rows that need the real supervisor: the generation a background command carries
//! comes from the policy the owner published, so only the running owner can replace it
//!
//! Each test body runs in child processes; see [`super::run_case`]
#![cfg(test)]

use act_zero::call;
use cove_device::cloud_storage::CloudStorage;

use super::{
    CloudCall, FakeAccount, NextAction as N, PhaseEnd, TestApp, keychain_namespace, load_detail,
    persisted, run_case,
};
use crate::database::cloud_backup::PersistedCloudBackupState;
use crate::manager::cloud_backup_manager::actors::write::{CloudBackupWriteClient, Decision};
use crate::manager::cloud_backup_manager::admission::PauseReason;
use crate::manager::cloud_backup_manager::{CloudBackupError, CloudBackupManagerAction};

/// Submits one background upload under `client` and waits for the lane's answer
fn upload(
    app: &TestApp,
    client: CloudBackupWriteClient,
    record: &str,
) -> Result<(), CloudBackupError> {
    let namespace = keychain_namespace().expect("a configured namespace");
    let record = record.to_string();
    app.block_on("the background upload's answer", async move {
        let cloud = CloudStorage::global_explicit_client();
        client.upload_wallet_backup(cloud, namespace, record, vec![1, 2, 3]).await
    })
}

/// A background upload that read its configuration before Turn Off reaches the lane only after
/// a later Enable: the lane drops it, so it never writes into the new backup
#[test]
fn old_background_upload_after_turn_off_and_a_later_enable_never_runs() {
    let test = concat!(
        module_path!(),
        "::old_background_upload_after_turn_off_and_a_later_enable_never_runs"
    );
    run_case(test, &[PhaseEnd::Exit], |_| {
        let mut app = TestApp::launch();
        FakeAccount::new(1, 0).seed(&app);
        load_detail(&mut app);

        let writes = app.manager().cloud_writes.clone();
        let old = app
            .block_on("a ticket before Turn Off", async move { call!(writes.ticket()).await })
            .expect("ticket");
        assert_eq!(old.ordinary, Decision::Run);
        let old_client =
            CloudBackupWriteClient::background(app.manager().cloud_writes.clone(), old.generation);

        app.dispatch(CloudBackupManagerAction::DisableCloudBackup)
            .then_can(&[N::Enable])
            .then_cannot(&[N::TurnOff]);

        // the fixture enables the way a finished Enable ends for the lane: the record becomes
        // configured and the owner publishes a policy of a new body kind
        FakeAccount::new(1, 0).seed(&app);
        app.current().then_cannot(&[N::Enable, N::RestoreFromCloudBackup]);
        assert!(
            matches!(persisted(), PersistedCloudBackupState::Configured(_)),
            "{}",
            app.describe()
        );

        let uploads = app.cloud.count(CloudCall::UploadWallet);
        let mut dropped = None;
        app.event("the upload from before Turn Off reaches the lane", |app| {
            dropped = Some(upload(app, old_client, "old-record"));
        })
        .then_can(&[N::TurnOff]);

        let dropped = dropped.expect("the old upload's answer");
        assert!(
            matches!(
                dropped,
                Err(CloudBackupError::WritesPaused(PauseReason::ConfigurationChanged))
            ),
            "{dropped:?}"
        );
        assert_eq!(app.cloud.count(CloudCall::UploadWallet), uploads, "the old upload ran");

        // control: the same upload under the current configuration runs
        let writes = app.manager().cloud_writes.clone();
        let current = app
            .block_on("a ticket after Enable", async move { call!(writes.ticket()).await })
            .expect("ticket");
        let current_client = CloudBackupWriteClient::background(
            app.manager().cloud_writes.clone(),
            current.generation,
        );
        app.event("an upload under the current configuration reaches the lane", |app| {
            upload(app, current_client, "new-record").expect("the current upload runs");
        })
        .then_can(&[N::TurnOff])
        .then_cannot(&[N::Enable]);

        assert_eq!(app.cloud.count(CloudCall::UploadWallet), uploads + 1);
    });
}
