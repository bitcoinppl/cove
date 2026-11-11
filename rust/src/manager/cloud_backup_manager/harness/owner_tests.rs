//! The lifecycle owner's own facts: how an operation finishes, the record it rebases on, and
//! what a failed local reset leaves
//!
//! Each test body runs in child processes; see [`super::run_case`]
#![cfg(test)]

use act_zero::call;

use super::invariants::Invariant;
use super::{CloudCall, CloudFault, FakeAccount, Hold, NextAction as N, PhaseEnd, TestApp};
use super::{load_detail, run_case};
use crate::manager::cloud_backup_manager::CloudBackupManagerAction;
use crate::manager::cloud_backup_manager::actors::supervisor::Finish;
use crate::manager::cloud_backup_manager::model::CloudBackupExclusiveOperationClaim;

fn running_claim(app: &TestApp) -> Option<CloudBackupExclusiveOperationClaim> {
    let supervisor = app.manager().supervisor.clone();
    app.block_on(
        "the running claim",
        async move { call!(supervisor.harness_running_claim()).await },
    )
    .expect("the supervisor answers")
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
