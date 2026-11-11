//! A committed Turn Off going forward: what the record keeps across failures and kills, the
//! provider access each start allows, and the retry a reconnect before `retry_after` arms
//!
//! Each test body runs in child processes; see [`super::run_case`]
#![cfg(test)]

use std::time::Duration;

use cove_device::cloud_storage::{CloudAccessPolicy, CloudStorageError};

use super::process::checkpoint;
use super::{
    CallId, CloudCall, CloudFault, FakeAccount, Hold, NextAction as N, PasskeyMode, PhaseEnd,
    TestApp, keychain_namespace, load_detail, open_durable_state, persisted, run_case,
};
use crate::database::Database;
use crate::database::cloud_backup::{
    PersistedCloudBackupState, PersistedCloudBackupStatus, PersistedDisableDisposition,
};
use crate::manager::cloud_backup_manager::CloudBackupManagerAction;
use crate::manager::cloud_backup_manager::admission::{Admission, Rejection, UserIntent};
use crate::manager::connectivity_manager::ConnectivityStatus;

fn offline() -> CloudFault {
    CloudFault::Fail(CloudStorageError::Offline("scripted".into()))
}

/// What the committed record says finishing still has to do
fn disposition() -> Option<PersistedDisableDisposition> {
    persisted().disabling().map(|disabling| disabling.disposition)
}

/// Commits a Turn Off whose delete fails offline, so it waits in `FinishingDisable`
fn commit_a_turn_off_whose_delete_fails(app: &mut TestApp) -> String {
    let namespace = FakeAccount::new(1, 0).seed(app);
    app.cloud.script(CallId::new(CloudCall::DeleteNamespace, 1), offline());
    load_detail(app);

    app.dispatch(CloudBackupManagerAction::DisableCloudBackup).then_can(&[N::ConfirmDisable]);
    app.confirm_turn_off()
        .then_can(&[N::ResumeDisable])
        .then_cannot(&[N::StopUsingOnThisDevice, N::Enable]);
    assert_eq!(persisted().status(), PersistedCloudBackupStatus::Disabling);
    namespace
}

/// Commits a Turn Off whose delete fails, then a Try Again that fails too, which offers Stop
/// Using on This Device
fn fail_a_committed_turn_off_and_its_try_again(app: &mut TestApp) -> String {
    let namespace = commit_a_turn_off_whose_delete_fails(app);
    app.cloud.script(CallId::new(CloudCall::DeleteNamespace, 2), offline());

    app.dispatch(CloudBackupManagerAction::ResumeDisable)
        .then_can(&[N::ResumeDisable, N::StopUsingOnThisDevice])
        .then_cannot(&[N::Enable]);
    assert_eq!(app.cloud.count(CloudCall::DeleteNamespace), 2);
    namespace
}

/// Stop Using on This Device is saved before its cleanup runs, so a cleanup that fails, then a
/// relaunch whose automatic resume has no tap behind it, still only cleans up this device
#[test]
fn stop_using_survives_a_failed_cleanup_and_never_deletes_the_remote_backup() {
    let test = concat!(
        module_path!(),
        "::stop_using_survives_a_failed_cleanup_and_never_deletes_the_remote_backup"
    );
    run_case(test, &[PhaseEnd::Exit], |_| {
        let mut app = TestApp::launch_with_passkey(PasskeyMode::FailOnAnyInteractiveCall);
        let namespace = fail_a_committed_turn_off_and_its_try_again(&mut app);

        app.keychain.inner.fail_delete_at(1);
        app.dispatch(CloudBackupManagerAction::StopUsingOnThisDevice)
            .then_can(&[N::ResumeDisable])
            .then_cannot(&[N::Enable]);
        assert_eq!(disposition(), Some(PersistedDisableDisposition::LocalOnly));
        assert_eq!(app.cloud.count(CloudCall::DeleteNamespace), 2);

        app.relaunch().then_can(&[N::Enable]).then_cannot(&[N::TurnOff, N::ResumeDisable]);
        assert_eq!(persisted(), PersistedCloudBackupState::Disabled);
        assert_eq!(app.cloud.count(CloudCall::DeleteNamespace), 2, "a resume deleted remotely");
        assert!(app.cloud.inner.has_namespace(&namespace));
        assert_eq!(keychain_namespace(), None);
        assert!(app.passkey.interactive_calls().is_empty());
    });
}

/// A local-only finish whose Disabled write fails keeps its disposition, so Try Again finishes
/// it locally instead of deleting the backup the user chose to keep
#[test]
fn stop_using_survives_a_failed_disabled_write_and_try_again_finishes_locally() {
    let test = concat!(
        module_path!(),
        "::stop_using_survives_a_failed_disabled_write_and_try_again_finishes_locally"
    );
    run_case(test, &[PhaseEnd::Exit], |_| {
        let mut app = TestApp::launch_with_passkey(PasskeyMode::FailOnAnyInteractiveCall);
        let namespace = fail_a_committed_turn_off_and_its_try_again(&mut app);

        Database::global()
            .cloud_backup_state
            .fail_next_write_where(|state| *state == PersistedCloudBackupState::Disabled);
        app.dispatch(CloudBackupManagerAction::StopUsingOnThisDevice)
            .then_can(&[N::ResumeDisable])
            .then_cannot(&[N::Enable]);
        assert_eq!(disposition(), Some(PersistedDisableDisposition::LocalOnly));

        app.dispatch(CloudBackupManagerAction::ResumeDisable)
            .then_can(&[N::Enable])
            .then_cannot(&[N::TurnOff]);
        assert_eq!(persisted(), PersistedCloudBackupState::Disabled);
        assert_eq!(app.cloud.count(CloudCall::DeleteNamespace), 2, "Try Again deleted remotely");
        assert!(app.cloud.inner.has_namespace(&namespace));
    });
}

/// Killed while Stop Using cleans up this device: the next launch, which resumes with nobody
/// tapping, finishes locally and leaves the remote backup in place
#[test]
fn stop_using_survives_a_kill_during_its_cleanup() {
    let test = concat!(module_path!(), "::stop_using_survives_a_kill_during_its_cleanup");
    run_case(test, &[PhaseEnd::KillAt("stop-using-cleaning-up"), PhaseEnd::Exit], |phase| {
        if phase == 0 {
            let mut app = TestApp::launch();
            fail_a_committed_turn_off_and_its_try_again(&mut app);

            let hold = Hold::default();
            app.keychain.hold_namespace_delete(hold.clone());
            app.dispatch(CloudBackupManagerAction::StopUsingOnThisDevice)
                .then_cannot(&[N::Enable, N::StopUsingOnThisDevice]);
            app.block_on("the held cleanup to start", hold.entered());
            assert_eq!(disposition(), Some(PersistedDisableDisposition::LocalOnly));

            checkpoint("stop-using-cleaning-up");
            unreachable!("the parent kills this process while Stop Using cleans up");
        }

        open_durable_state();
        assert_eq!(disposition(), Some(PersistedDisableDisposition::LocalOnly));
        let namespace = keychain_namespace().expect("the kill came before the namespace delete");

        let app = &mut TestApp::launch_with_passkey(PasskeyMode::FailOnAnyInteractiveCall);
        app.current().then_can(&[N::Enable]).then_cannot(&[N::TurnOff, N::ResumeDisable]);
        assert_eq!(persisted(), PersistedCloudBackupState::Disabled);
        assert_eq!(app.cloud.count(CloudCall::DeleteNamespace), 0, "the resume deleted remotely");
        assert!(app.cloud.inner.has_namespace(&namespace));
        assert_eq!(keychain_namespace(), None);
    });
}

/// Stop Using is offered after a failed Try Again, and that stays true across a relaunch; a
/// failure no one tapped for, before or after a relaunch, asks for Try Again first
#[test]
fn a_failed_try_again_offers_stop_using_across_a_relaunch() {
    let test = concat!(module_path!(), "::a_failed_try_again_offers_stop_using_across_a_relaunch");
    run_case(test, &[PhaseEnd::Exit], |_| {
        let mut app = TestApp::launch();
        commit_a_turn_off_whose_delete_fails(&mut app);

        // the launch resume fails too, and nobody tapped Try Again
        app.cloud.script_next(CloudCall::DeleteNamespace, offline());
        app.relaunch().then_can(&[N::ResumeDisable]).then_cannot(&[N::Enable]);
        assert_eq!(
            app.admission_of(UserIntent::StopUsingOnThisDevice),
            Admission::Rejected(Rejection::TryAgainFirst)
        );

        app.cloud.script_next(CloudCall::DeleteNamespace, offline());
        app.dispatch(CloudBackupManagerAction::ResumeDisable)
            .then_can(&[N::ResumeDisable, N::StopUsingOnThisDevice]);

        // the next launch resume fails again, and Stop Using is still there
        app.cloud.script_next(CloudCall::DeleteNamespace, offline());
        app.relaunch()
            .then_can(&[N::ResumeDisable, N::StopUsingOnThisDevice])
            .then_cannot(&[N::Enable]);
        assert!(persisted().disabling().is_some_and(|disabling| disabling.try_again_failed));
    });
}

/// A reconnect before the last failure's `retry_after` arms one retry, which runs at the
/// deadline with no further event
#[test]
fn a_reconnect_before_retry_after_resumes_at_the_deadline() {
    let test = concat!(module_path!(), "::a_reconnect_before_retry_after_resumes_at_the_deadline");
    run_case(test, &[PhaseEnd::Exit], |_| {
        let mut app = TestApp::launch_with_passkey(PasskeyMode::FailOnAnyInteractiveCall);
        let namespace = commit_a_turn_off_whose_delete_fails(&mut app);

        app.event("reconnect before retry_after", |app| {
            app.manager().handle_connectivity_change(ConnectivityStatus::Connected);
        })
        .then_can(&[N::ResumeDisable]);
        assert_eq!(
            app.cloud.count(CloudCall::DeleteNamespace),
            1,
            "it resumed before the deadline"
        );

        app.advance_clock(Duration::from_secs(6)).then_can(&[N::Enable]).then_cannot(&[N::TurnOff]);
        assert_eq!(app.cloud.count(CloudCall::DeleteNamespace), 2);
        let retry = app.cloud.policy_of(CallId::new(CloudCall::DeleteNamespace, 2));
        assert_eq!(retry, Some(CloudAccessPolicy::Silent));
        assert!(!app.cloud.inner.has_namespace(&namespace));
        assert_eq!(persisted(), PersistedCloudBackupState::Disabled);
    });
}

/// The Turn Off review and its recheck list silently, so a provider that wants consent never
/// shows its sheet before the user confirmed anything, and the Turn Off ends before its commit
#[test]
fn turn_off_review_and_recheck_never_ask_for_provider_consent() {
    let test =
        concat!(module_path!(), "::turn_off_review_and_recheck_never_ask_for_provider_consent");
    run_case(test, &[PhaseEnd::Exit], |_| {
        let mut app = TestApp::launch_with_passkey(PasskeyMode::FailOnAnyInteractiveCall);
        let namespace = FakeAccount::new(1, 0).seed(&app);
        load_detail(&mut app);

        let before = app.cloud.calls().len();
        app.cloud.revoke_authorization();
        app.dispatch(CloudBackupManagerAction::DisableCloudBackup).then_can(&[N::ConfirmDisable]);
        app.confirm_turn_off().then_can(&[N::TurnOff]).then_cannot(&[N::Enable]);

        let calls = app.cloud.calls().split_off(before);
        let listings = calls.iter().filter(|(id, _)| id.kind == CloudCall::ListWalletFiles);
        assert!(listings.count() >= 2, "the review and the recheck each list: {calls:?}");

        let policies: Vec<_> =
            calls.iter().map(|(id, _)| (*id, app.cloud.policy_of(*id))).collect();
        assert!(
            policies.iter().all(|(_, policy)| *policy == Some(CloudAccessPolicy::Silent)),
            "the review or the recheck allowed consent: {policies:?}"
        );
        assert_eq!(app.cloud.consent_sheets(), 0);
        assert_eq!(persisted().status(), PersistedCloudBackupStatus::Enabled);
        assert_eq!(app.cloud.count(CloudCall::DeleteNamespace), 0);
        assert!(app.cloud.inner.has_namespace(&namespace));
    });
}
