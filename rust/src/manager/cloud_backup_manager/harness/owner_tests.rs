//! The lifecycle owner's own facts: how an operation finishes, the record it rebases on, and
//! what a failed local reset leaves
//!
//! Each test body runs in child processes; see [`super::run_case`]
#![cfg(test)]

use act_zero::call;

use super::invariants::Invariant;
use super::{CloudCall, CloudFault, FakeAccount, Hold, NextAction as N, PasskeyMode, PhaseEnd};
use super::{TestApp, load_detail, run_case};
use crate::database::Database;
use crate::manager::cloud_backup_manager::actors::supervisor::Finish;
use crate::manager::cloud_backup_manager::actors::write::Decision;
use crate::manager::cloud_backup_manager::admission::Admission;
use crate::manager::cloud_backup_manager::model::CloudBackupExclusiveOperationClaim;
use crate::manager::cloud_backup_manager::owner::lifecycle::{LifecycleLoad, LifecycleWriter};
use crate::manager::cloud_backup_manager::{
    CloudBackupManagerAction, CloudBackupRuntimeOwnership, CloudBackupVerificationSource,
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
