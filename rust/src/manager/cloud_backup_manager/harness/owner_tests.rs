//! The lifecycle owner's own facts: how an operation finishes, the record it rebases on, and
//! what a failed local reset leaves
//!
//! Each test body runs in child processes; see [`super::run_case`]
#![cfg(test)]

use std::time::Duration;

use act_zero::call;

use super::invariants::Invariant;
use super::process::checkpoint;
use super::{CloudCall, CloudFault, FakeAccount, Hold, NextAction as N, PasskeyMode, PhaseEnd};
use super::{TestApp, change_a_wallet, keychain_namespace, load_detail, open_durable_state};
use super::{persisted, run_case};
use crate::database::Database;
use crate::database::cloud_backup::{PersistedCloudBackupState, PersistedCloudBackupStatus};
use crate::manager::cloud_backup_manager::actors::supervisor::Finish;
use crate::manager::cloud_backup_manager::actors::write::Decision;
use crate::manager::cloud_backup_manager::admission::Admission;
use crate::manager::cloud_backup_manager::model::CloudBackupExclusiveOperationClaim;
use crate::manager::cloud_backup_manager::owner::lifecycle::{LifecycleLoad, LifecycleWriter};
use crate::manager::cloud_backup_manager::{
    CloudBackupManagerAction, CloudBackupResetPermit, CloudBackupResetRecovery,
    CloudBackupRuntimeOwnership, CloudBackupStatus, CloudBackupVerificationSource,
};

fn running_claim(app: &TestApp) -> Option<CloudBackupExclusiveOperationClaim> {
    let supervisor = app.manager().supervisor.clone();
    app.block_on(
        "the running claim",
        async move { call!(supervisor.harness_running_claim()).await },
    )
    .expect("the supervisor answers")
}

/// What the write lane decides now for an ordinary write
fn ordinary_decision(app: &TestApp) -> Decision {
    let writes = app.writes();
    app.block_on("the write lane's ticket", async move { call!(writes.ticket()).await })
        .expect("the write lane answers")
        .ordinary
}

/// Starts Turn Off and holds it at its namespace delete; returns the hold and its claim
fn held_turn_off(app: &mut TestApp) -> (Hold, CloudBackupExclusiveOperationClaim) {
    FakeAccount::new(1, 0).seed(app);
    load_detail(app);
    let hold = Hold::default();
    app.cloud.script_next(CloudCall::DeleteNamespace, CloudFault::Hold(hold.clone()));

    app.dispatch(CloudBackupManagerAction::DisableCloudBackup)
        .then_cannot(&[N::TurnOff, N::Enable]);
    app.block_on("the held delete to start", hold.entered());

    let claim = running_claim(app).expect("Turn Off runs");
    (hold, claim)
}

/// A finish names the operation it ends: a second finish of an operation that already ended,
/// arriving while another runs, ends nothing, and invariant 5 reports the second finish
#[test]
fn a_second_finish_of_an_ended_operation_ends_nothing_and_breaks_invariant_5() {
    let test = concat!(
        module_path!(),
        "::a_second_finish_of_an_ended_operation_ends_nothing_and_breaks_invariant_5"
    );
    run_case(test, &[PhaseEnd::Exit], |_| {
        let mut app = TestApp::launch();
        let (hold, first) = held_turn_off(&mut app);
        app.event("release the first Turn Off", |_| hold.release())
            .then_can(&[N::Enable])
            .then_cannot(&[N::TurnOff]);
        assert_eq!(running_claim(&app), None);

        let (hold, second) = held_turn_off(&mut app);
        assert_ne!(first, second);

        let supervisor = app.manager().supervisor.clone();
        let finish = app
            .block_on("a second finish of the first Turn Off", async move {
                call!(supervisor.harness_finish(first)).await
            })
            .expect("the supervisor answers");

        assert_eq!(finish, Finish::NotCurrent { running: Some(second) });
        assert_eq!(running_claim(&app), Some(second), "the second finish ended another operation");

        let violation = app.settled_violation().expect("invariant 5 sees the second finish");
        assert_eq!(violation.invariant, Invariant::OneFinishOneAdmission, "{}", violation.detail);

        hold.release();
        app.settle();
    });
}

/// A record read error is not another record: an operation that starts and finishes while the
/// record can't be read leaves the configured body, its admission and its write policy
#[test]
fn an_operation_finishing_while_the_record_is_unreadable_keeps_the_configured_body() {
    let test = concat!(
        module_path!(),
        "::an_operation_finishing_while_the_record_is_unreadable_keeps_the_configured_body"
    );
    run_case(test, &[PhaseEnd::Exit], |_| {
        let mut app = TestApp::launch();
        FakeAccount::new(1, 0).seed(&app);
        load_detail(&mut app);

        let fault = Database::global().cloud_backup_state.fail_reads();
        app.dispatch_unchecked(CloudBackupManagerAction::StartVerification(
            CloudBackupVerificationSource::CloudBackupDetail,
        ))
        .then_can(&[N::TurnOff])
        .then_cannot(&[N::Enable, N::RestoreFromCloudBackup]);

        let outcome = app.last_outcome().expect("the verification's admission is recorded");
        assert_eq!(outcome.admission, Admission::Accepted);
        assert_eq!(running_claim(&app), None, "the verification did not finish");
        assert_eq!(ordinary_decision(&app), Decision::Run, "the owner published another policy");
        drop(fault);

        app.current().then_can(&[N::TurnOff, N::Verify]).then_cannot(&[N::Enable]);
    });
}

/// Bootstrap, then fail the next record read, which is the new owner's first
fn bootstrap_then_fail_the_next_read(
    ownership: &CloudBackupRuntimeOwnership,
) -> (LifecycleWriter, LifecycleLoad) {
    let bootstrap = LifecycleWriter::bootstrap(ownership);
    Database::global().cloud_backup_state.fail_next_reads(1);
    bootstrap
}

/// The owner starts from the record bootstrap read: a read error right after a successful
/// bootstrap leaves the configured body, not an off one
#[test]
fn a_read_error_after_bootstrap_keeps_the_configured_body() {
    let test = concat!(module_path!(), "::a_read_error_after_bootstrap_keeps_the_configured_body");
    run_case(test, &[PhaseEnd::Exit, PhaseEnd::Exit], |phase| {
        if phase == 0 {
            let app = TestApp::launch();
            FakeAccount::new(1, 0).seed(&app);
            return;
        }

        let mut app = TestApp::launch_with_bootstrap(
            PasskeyMode::Succeeds,
            bootstrap_then_fail_the_next_read,
        );
        app.current().then_can(&[N::TurnOff, N::Verify]).then_cannot(&[N::Enable]);
        assert_eq!(ordinary_decision(&app), Decision::Run, "the owner published another policy");
    });
}

/// Seeds a configured backup and prepares a local reset of it, as a full wipe does
fn configured_reset(app: &mut TestApp) -> CloudBackupResetPermit {
    FakeAccount::new(1, 0).seed(app);
    let manager = app.manager().clone();
    app.block_on("the reset to be ready", async move { manager.prepare_local_reset().await })
        .expect("reset permit")
}

/// Clears the local keys through the owner, as the wipe does before its database reset
fn clear_keys(app: &TestApp, permit: &CloudBackupResetPermit) {
    app.block_on("the owner to clear local state", permit.clear_local_state())
        .expect("clear local state");
}

/// Changes a wallet while the reset holds the lane, so a write would be queued if any could run
fn change_a_wallet_during_the_reset(app: &mut TestApp) {
    let mut changed = None;
    app.event("a local wallet changes during the reset", |app| {
        changed = change_a_wallet(app);
    })
    .then_cannot(&[N::TurnOff, N::Enable]);
    assert!(changed.is_some(), "no seeded wallet changed");
}

/// Ends the reset as a wipe that failed before its database reset
fn abandon(
    app: &mut TestApp,
    permit: CloudBackupResetPermit,
    name: &str,
) -> CloudBackupResetRecovery {
    let mut recovery = None;
    app.event(name, |app| {
        let ended = app.block_on("the reset to end", permit.resume_after_failed_reset());
        recovery = Some(ended.expect("abandon the reset"));
    })
    .then_cannot(&[N::KeepEnabled]);
    recovery.expect("the reset ended")
}

/// A wipe that fails before the owner clears the local keys leaves the backup as it was
#[test]
fn a_reset_abandoned_before_its_key_clear_resumes_the_configured_backup() {
    let test = concat!(
        module_path!(),
        "::a_reset_abandoned_before_its_key_clear_resumes_the_configured_backup"
    );
    run_case(test, &[PhaseEnd::Exit], |_| {
        let mut app = TestApp::launch();
        let permit = configured_reset(&mut app);
        change_a_wallet_during_the_reset(&mut app);
        let uploads = app.cloud.count(CloudCall::UploadWallet);

        let recovery = abandon(&mut app, permit, "the wipe fails before the key clear");
        assert_eq!(recovery, CloudBackupResetRecovery::PreviousStatusResumed);
        app.current().then_can(&[N::TurnOff, N::Verify]).then_cannot(&[N::Enable]);

        assert_eq!(persisted().status(), PersistedCloudBackupStatus::Enabled);
        assert_eq!(ordinary_decision(&app), Decision::Run);
        assert_eq!(
            app.cloud.count(CloudCall::UploadWallet),
            uploads + 1,
            "the wake did not upload"
        );
    });
}

/// A wipe that fails after the owner began clearing the local keys can't take the clear back:
/// the reset ends with Cloud Backup off, and no write runs for the backup it had
#[test]
fn a_reset_abandoned_after_its_key_clear_ends_with_backup_off() {
    let test =
        concat!(module_path!(), "::a_reset_abandoned_after_its_key_clear_ends_with_backup_off");
    run_case(test, &[PhaseEnd::Exit], |_| {
        let mut app = TestApp::launch();
        let permit = configured_reset(&mut app);
        clear_keys(&app, &permit);
        change_a_wallet_during_the_reset(&mut app);
        let uploads = app.cloud.count(CloudCall::UploadWallet);

        let recovery = abandon(&mut app, permit, "the wipe fails after the key clear");
        assert_eq!(recovery, CloudBackupResetRecovery::SafelyDisabled);
        app.current().then_can(&[N::Enable]).then_cannot(&[N::TurnOff, N::Verify, N::SyncUnsynced]);

        assert_eq!(persisted(), PersistedCloudBackupState::Disabled);
        assert_ne!(ordinary_decision(&app), Decision::Run);
        assert_eq!(app.manager().current_status(), CloudBackupStatus::Disabled);

        app.advance_clock(Duration::from_secs(600)).then_can(&[N::Enable]);
        assert_eq!(app.cloud.count(CloudCall::UploadWallet), uploads, "an upload ran without keys");
    });
}

/// When Cloud Backup can't be turned off before the key clear, the owner refuses the clear: the
/// keys stay with the configured backup, which the abandoned reset and the next launch resume
#[test]
fn a_reset_that_cannot_turn_off_keeps_its_keys_and_resumes_the_configured_backup() {
    let test = concat!(
        module_path!(),
        "::a_reset_that_cannot_turn_off_keeps_its_keys_and_resumes_the_configured_backup"
    );
    run_case(test, &[PhaseEnd::Exit, PhaseEnd::Exit], |phase| {
        if phase == 1 {
            let mut app = TestApp::launch();
            app.current().then_can(&[N::TurnOff, N::Verify]).then_cannot(&[N::Enable]);
            assert!(keychain_namespace().is_some(), "the relaunch found no keys");
            return;
        }

        let mut app = TestApp::launch();
        let permit = configured_reset(&mut app);
        let namespace = keychain_namespace();
        let configured = persisted();

        Database::global()
            .cloud_backup_state
            .fail_next_write_where(|state| matches!(state, PersistedCloudBackupState::Disabled));
        let cleared = app.block_on("the owner to refuse the key clear", permit.clear_local_state());
        assert!(cleared.is_err(), "the keys were cleared while the record says configured");
        assert_eq!(keychain_namespace(), namespace);
        assert_eq!(persisted(), configured);

        let recovery = abandon(&mut app, permit, "the wipe fails at its key clear");
        assert_eq!(recovery, CloudBackupResetRecovery::PreviousStatusResumed);
        app.current().then_can(&[N::TurnOff, N::Verify]).then_cannot(&[N::Enable]);
        assert_eq!(ordinary_decision(&app), Decision::Run);
    });
}

/// The process dies after the reset cleared the local keys and before its database reset: the
/// next launch reads a record that already says Cloud Backup is off
#[test]
fn a_reset_killed_after_its_key_clear_relaunches_with_backup_off() {
    let test =
        concat!(module_path!(), "::a_reset_killed_after_its_key_clear_relaunches_with_backup_off");
    run_case(test, &[PhaseEnd::KillAt("reset-keys-cleared"), PhaseEnd::Exit], |phase| {
        if phase == 0 {
            let mut app = TestApp::launch();
            let permit = configured_reset(&mut app);
            clear_keys(&app, &permit);

            checkpoint("reset-keys-cleared");
            unreachable!("the parent kills this process after the key clear");
        }

        open_durable_state();
        assert_eq!(keychain_namespace(), None, "the clear kept the namespace");
        assert_eq!(persisted(), PersistedCloudBackupState::Disabled);

        let mut app = TestApp::launch();
        app.current().then_can(&[N::Enable]).then_cannot(&[N::TurnOff, N::Verify, N::SyncUnsynced]);
        assert_ne!(ordinary_decision(&app), Decision::Run);
        assert_eq!(app.manager().current_status(), CloudBackupStatus::Disabled);
    });
}
