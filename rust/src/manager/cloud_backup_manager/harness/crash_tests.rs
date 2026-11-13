//! The Turn Off crash matrix, each row by real process kill: the next launch, a new
//! process on the same database, keychain and cloud files, must reach Disabled once faults clear,
//! deleting the namespace once and never prompting for a passkey
//!
//! Each test body runs in child processes; see [`super::run_case`]
#![cfg(test)]

use std::time::Duration;

use cove_device::cloud_storage::CloudStorageError;

use super::process::checkpoint;
use super::{
    CloudCall, CloudFault, FakeAccount, Hold, NextAction as N, PasskeyMode, PhaseEnd, TestApp,
    change_a_wallet, keychain_namespace, load_detail, open_durable_state, persisted, run_case,
};
use crate::database::cloud_backup::{PersistedCloudBackupState, PersistedCloudBackupStatus};
use crate::manager::cloud_backup_manager::CloudBackupManagerAction;

/// The record after the kill: committed means the disable started deleting
fn committed_at() -> Option<u64> {
    persisted().disabling().and_then(|disabling| disabling.delete_started_at)
}

/// The next launch resumes a committed disable to Disabled on its own, deleting the namespace
/// exactly once in this process and without a passkey prompt
fn next_launch_finishes_the_committed_disable(namespace: &str, deletes_this_launch: usize) {
    let mut app = TestApp::launch_with_passkey(PasskeyMode::FailOnAnyInteractiveCall);
    app.current().then_can(&[N::Enable]).then_cannot(&[N::TurnOff, N::Verify]);

    assert_eq!(persisted(), PersistedCloudBackupState::Disabled, "{}", app.describe());
    assert_eq!(keychain_namespace(), None);
    assert!(!app.cloud.inner.has_namespace(namespace), "the namespace survived");
    assert_eq!(app.cloud.count(CloudCall::DeleteNamespace), deletes_this_launch);
    assert!(app.passkey.interactive_calls().is_empty());
}

/// Killed while Turn Off waits for an upload in flight: nothing was committed, so the next
/// launch is still configured with its namespace, and a new Turn Off finishes once
#[test]
fn killed_during_the_turn_off_drain_keeps_the_backup_and_turns_off_later() {
    let test = concat!(
        module_path!(),
        "::killed_during_the_turn_off_drain_keeps_the_backup_and_turns_off_later"
    );
    run_case(test, &[PhaseEnd::KillAt("turn-off-draining"), PhaseEnd::Exit], |phase| {
        if phase == 0 {
            let mut app = TestApp::launch();
            FakeAccount::new(1, 0).seed(&app);
            load_detail(&mut app);

            let hold = Hold::default();
            app.cloud.script_next(CloudCall::UploadWallet, CloudFault::Hold(hold.clone()));
            app.event("a local wallet changes", |app| {
                change_a_wallet(app).expect("the seeded wallet");
            })
            .then_can(&[N::TurnOff]);
            app.advance_clock(Duration::from_secs(60)).then_can(&[N::TurnOff]);
            app.block_on("the upload to be held", hold.entered());

            app.dispatch(CloudBackupManagerAction::DisableCloudBackup)
                .then_can(&[N::ConfirmDisable]);
            app.confirm_turn_off().then_can(&[N::CancelDisable]).then_cannot(&[
                N::Enable,
                N::Verify,
                N::TurnOff,
            ]);
            assert_eq!(committed_at(), None, "Turn Off committed before its drain finished");
            assert_eq!(app.cloud.count(CloudCall::DeleteNamespace), 0);

            checkpoint("turn-off-draining");
            unreachable!("the parent kills this process while Turn Off drains");
        }

        open_durable_state();
        assert_eq!(persisted().status(), PersistedCloudBackupStatus::Enabled);
        let namespace = keychain_namespace().expect("the kill kept the namespace");

        let mut app = TestApp::launch_with_passkey(PasskeyMode::FailOnAnyInteractiveCall);
        app.current().then_can(&[N::TurnOff]).then_cannot(&[N::Enable]);
        assert!(app.cloud.inner.has_namespace(&namespace));
        load_detail(&mut app);

        app.dispatch(CloudBackupManagerAction::DisableCloudBackup).then_can(&[N::ConfirmDisable]);
        app.confirm_turn_off().then_can(&[N::Enable]).then_cannot(&[N::TurnOff]);
        assert_eq!(persisted(), PersistedCloudBackupState::Disabled);
        assert!(!app.cloud.inner.has_namespace(&namespace));
        assert_eq!(app.cloud.count(CloudCall::DeleteNamespace), 1);
    });
}

/// Killed after the commit and before the namespace delete starts: the next launch deletes it
#[test]
fn killed_after_commit_before_the_delete_resumes_to_disabled() {
    let test =
        concat!(module_path!(), "::killed_after_commit_before_the_delete_resumes_to_disabled");
    run_case(test, &[PhaseEnd::KillAt("before-namespace-delete"), PhaseEnd::Exit], |phase| {
        if phase == 0 {
            let mut app = TestApp::launch();
            FakeAccount::new(1, 0).seed(&app);
            app.cloud.script_next(
                CloudCall::DeleteNamespace,
                CloudFault::KillBefore("before-namespace-delete"),
            );
            load_detail(&mut app);

            app.dispatch(CloudBackupManagerAction::DisableCloudBackup)
                .then_can(&[N::ConfirmDisable]);
            let _killed = app.confirm_turn_off();
            unreachable!("the parent kills this process before the namespace delete");
        }

        open_durable_state();
        assert!(committed_at().is_some(), "the kill left {:?}, not the commit", persisted());
        let namespace = keychain_namespace().expect("the commit keeps the namespace until it ends");

        next_launch_finishes_the_committed_disable(&namespace, 1);
    });
}

/// The namespace delete succeeded but its response was lost, then the process died: the next
/// launch finds the namespace gone, and the committed disable still finishes
#[test]
fn delete_that_succeeded_with_its_response_lost_finishes_after_a_kill() {
    let test = concat!(
        module_path!(),
        "::delete_that_succeeded_with_its_response_lost_finishes_after_a_kill"
    );
    run_case(test, &[PhaseEnd::KillAt("delete-response-lost"), PhaseEnd::Exit], |phase| {
        if phase == 0 {
            let mut app = TestApp::launch();
            let namespace = FakeAccount::new(1, 0).seed(&app);
            let lost = CloudStorageError::Offline("the response was lost".into());
            app.cloud.script_next(CloudCall::DeleteNamespace, CloudFault::SucceedThenFail(lost));
            load_detail(&mut app);

            app.dispatch(CloudBackupManagerAction::DisableCloudBackup)
                .then_can(&[N::ConfirmDisable]);
            app.confirm_turn_off().then_cannot(&[N::Enable, N::Verify]);
            assert!(committed_at().is_some(), "the failed response undid the commit");
            assert!(!app.cloud.inner.has_namespace(&namespace), "the delete did not land");

            checkpoint("delete-response-lost");
            unreachable!("the parent kills this process after the lost response");
        }

        open_durable_state();
        assert!(committed_at().is_some(), "the kill left {:?}, not the commit", persisted());
        let namespace = keychain_namespace().expect("the commit keeps the namespace until it ends");

        next_launch_finishes_the_committed_disable(&namespace, 1);
    });
}
