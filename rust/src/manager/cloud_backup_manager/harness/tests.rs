//! Proof that the harness drives the real manager: runtime, clock, relaunch, call-identity
//! fakes and real process kills
//!
//! Each test body runs in child processes; see [`super::run_case`]
#![cfg(test)]

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Duration;

use act_zero::call;
use cove_device::cloud_storage::CloudStorageError;
use cove_device::passkey::{PasskeyError, PasskeyFailureReason, PasskeyOperation};

use super::process::child_root;
use super::{
    CallId, CloudCall, CloudFault, FakeAccount, HarnessCloud, Hold, NextAction as N, PasskeyMode,
    PhaseEnd, TestApp, keychain_namespace, load_detail, open_durable_state, persisted, run_case,
    seed_corrupted_record, seed_legacy_stuck_disabling, seed_verifiable_master_key,
};
use crate::database::cloud_backup::{PersistedCloudBackupState, PersistedCloudBackupStatus};
use crate::manager::cloud_backup_manager::actors::write::Decision;
use crate::manager::cloud_backup_manager::admission::{
    Admission, PauseReason, Rejection, UserIntent,
};
use crate::manager::cloud_backup_manager::model::CloudBackupPasskeyRepairState;
use crate::manager::cloud_backup_manager::ops::CLOUD_BACKUP_LOAD_FAILED_MESSAGE;
use crate::manager::cloud_backup_manager::ops::test_support::MockCloudStorage;
use crate::manager::cloud_backup_manager::owner::lifecycle::test_support::bootstrap_with_failed_heal;
use crate::manager::cloud_backup_manager::owner::ui_test_legacy_record;
use crate::manager::cloud_backup_manager::screen::{
    CloudBackupActionAvailability, CloudBackupDisableConfirmation, CloudBackupScreenAction,
    CloudBackupScreenBody, CloudBackupSection, CloudBackupVerificationControl,
};
use crate::manager::cloud_backup_manager::{
    CloudBackupEnableContext, CloudBackupManagerAction, CloudBackupNoticeKind,
    CloudBackupPasskeyState, CloudBackupSettingsRowStatus, CloudBackupVerificationSource,
    CloudBackupVerificationState, DeepVerificationFailure,
};
use crate::manager::cloud_backup_manager::{CloudBackupStatus, RustCloudBackupManager};

use CloudBackupScreenAction as A;

#[test]
fn paused_clock_moves_only_when_the_test_advances_it() {
    let test = concat!(module_path!(), "::paused_clock_moves_only_when_the_test_advances_it");
    run_case(test, &[PhaseEnd::Exit], |_| {
        let mut app = TestApp::launch();
        let start = app.clock.now_secs();
        let fired = Arc::new(AtomicBool::new(false));
        let timer_fired = fired.clone();
        let _timer = cove_tokio::task::spawn(async move {
            tokio::time::sleep(Duration::from_secs(60)).await;
            timer_fired.store(true, Ordering::SeqCst);
        });

        app.current().then_can(&[N::Enable]);
        assert!(!fired.load(Ordering::SeqCst), "a paused timer fired without an advance");

        app.advance_clock(Duration::from_secs(59)).then_can(&[N::Enable]);
        assert!(!fired.load(Ordering::SeqCst), "the timer fired early");

        app.advance_clock(Duration::from_secs(1)).then_can(&[N::Enable]);
        assert!(fired.load(Ordering::SeqCst), "the timer did not fire at its deadline");
        assert_eq!(app.clock.now_secs(), start + 60);
    });
}

/// A blocking job no fake and no operation stage knows about keeps the app unsettled until the
/// task waiting on it has run: the park can't see the job while it is dispatched, while it runs,
/// or while its answer is on the way back
#[test]
fn settle_waits_for_a_blocking_job_and_the_task_waiting_on_it() {
    let test =
        concat!(module_path!(), "::settle_waits_for_a_blocking_job_and_the_task_waiting_on_it");
    run_case(test, &[PhaseEnd::Exit], |_| {
        let mut app = TestApp::launch();
        let answered = Arc::new(AtomicBool::new(false));
        let job_answered = answered.clone();

        app.event("a task starts a blocking job", |_| {
            let _task = cove_tokio::task::spawn(async move {
                // a platform call that takes a while on its own thread, long enough that the
                // runtime parks and the old settle returned first
                cove_tokio::task::spawn_blocking(|| std::thread::sleep(Duration::from_millis(50)))
                    .await
                    .expect("blocking job");
                job_answered.store(true, Ordering::SeqCst);
            });
        })
        .then_can(&[N::Enable]);

        assert!(answered.load(Ordering::SeqCst), "settled before the blocking job's task ran");
    });
}

#[test]
fn soft_relaunch_keeps_durable_state_without_prompting() {
    let test = concat!(module_path!(), "::soft_relaunch_keeps_durable_state_without_prompting");
    run_case(test, &[PhaseEnd::Exit], |_| {
        let mut app = TestApp::launch_with_passkey(PasskeyMode::FailOnAnyInteractiveCall);
        let namespace = FakeAccount::new(2, 0).seed(&app);

        // the relaunched app has not loaded its detail inventory, and Turn Off is still admitted
        app.relaunch().then_can(&[N::Verify, N::TurnOff]).then_cannot(&[N::Enable]);

        assert_eq!(persisted().status(), PersistedCloudBackupStatus::Enabled);
        assert_eq!(keychain_namespace().as_deref(), Some(namespace.as_str()));
        assert!(app.cloud.inner.has_namespace(&namespace));
        assert!(app.passkey.interactive_calls().is_empty());
    });
}

/// A soft relaunch replaces the manager the way a new process would: the replaced manager's write
/// lane stays closed, so a timer or queued write it still holds can't act beside the new one
#[test]
fn soft_relaunch_fences_the_manager_it_replaces() {
    let test = concat!(module_path!(), "::soft_relaunch_fences_the_manager_it_replaces");
    run_case(test, &[PhaseEnd::Exit], |_| {
        let mut app = TestApp::launch();
        FakeAccount::new(1, 0).seed(&app);
        let replaced = app.manager().clone();

        app.relaunch().then_can(&[N::Verify]).then_cannot(&[N::Enable]);

        let writes = replaced.cloud_writes.clone();
        let old =
            app.block_on("the replaced lane's ticket", async move { call!(writes.ticket()).await });
        assert_eq!(old.expect("ticket").ordinary, Decision::Paused(PauseReason::LocalReset));

        let writes = app.manager().cloud_writes.clone();
        let new =
            app.block_on("the new lane's ticket", async move { call!(writes.ticket()).await });
        assert_eq!(new.expect("ticket").ordinary, Decision::Run);
    });
}

/// A hold the test releases before its call starts lets that call straight through, instead of
/// stranding it with nothing left to release it
#[test]
fn a_hold_released_before_its_call_starts_lets_the_call_through() {
    let test =
        concat!(module_path!(), "::a_hold_released_before_its_call_starts_lets_the_call_through");
    run_case(test, &[PhaseEnd::Exit], |_| {
        let mut app = TestApp::launch();
        FakeAccount::new(1, 0).seed(&app);
        load_detail(&mut app);
        let hold = Hold::default();
        app.cloud.script_next(CloudCall::DeleteNamespace, CloudFault::Hold(hold.clone()));
        hold.release();

        app.dispatch(CloudBackupManagerAction::DisableCloudBackup).then_can(&[N::ConfirmDisable]);
        app.confirm_turn_off().then_can(&[N::Enable]).then_cannot(&[N::TurnOff]);
        app.block_on("the released call to return", hold.finished());
        assert_eq!(persisted(), PersistedCloudBackupState::Disabled);
        assert_eq!(app.cloud.count(CloudCall::DeleteNamespace), 1);
    });
}

/// Restore and Enable each end their operation once their held calls answer and the clock moves
/// past their timers: at quiescence nothing is live, and the body they left offers its exits
#[test]
fn restore_and_enable_leave_nothing_live_at_quiescence() {
    let test = concat!(module_path!(), "::restore_and_enable_leave_nothing_live_at_quiescence");
    run_case(test, &[PhaseEnd::Exit], |_| {
        let mut app = TestApp::launch();

        // quiescence releases the held listing, which finds no backup
        app.cloud.script_next(CloudCall::ListNamespaces, CloudFault::Hold(Hold::default()));
        app.dispatch(CloudBackupManagerAction::RestoreFromCloudBackup).then_cannot(&[N::Enable]);
        assert!(app.runs_operation(), "the held listing holds the restore\n{}", app.describe());
        app.quiesce().then_can(&[N::Enable, N::RestoreFromCloudBackup]);

        // Enable registers a passkey, then gives it time on the paused clock before asking the
        // user to confirm it was saved; quiescence advances past that wait
        app.passkey.inner.set_discover_result(Err(PasskeyError::NoCredentialFound));
        app.passkey.inner.set_create_result(Ok(vec![4, 5, 6]));
        app.dispatch(CloudBackupManagerAction::EnableCloudBackup(
            CloudBackupEnableContext::settings_manual(),
        ))
        .then_cannot(&[N::RestoreFromCloudBackup]);
        assert!(app.runs_operation(), "Enable waits on the clock\n{}", app.describe());
        app.quiesce().then_cannot(&[N::TurnOff]);
        assert!(!app.runs_operation(), "{}", app.describe());
        assert_eq!(app.admission_of(UserIntent::ConfirmSavedPasskey), Admission::Accepted);

        app.passkey.inner.set_authenticate_result(Ok(vec![7; 32]));
        app.dispatch(CloudBackupManagerAction::ConfirmSavedPasskey).then_cannot(&[N::Enable]);
        app.quiesce().then_can(&[N::TurnOff]).then_cannot(&[N::Enable]);
        assert!(
            matches!(persisted(), PersistedCloudBackupState::Configured(_)),
            "{}",
            app.describe()
        );
    });
}

#[test]
fn held_namespace_delete_reports_entered_and_finished() {
    let test = concat!(module_path!(), "::held_namespace_delete_reports_entered_and_finished");
    run_case(test, &[PhaseEnd::Exit], |_| {
        let mut app = TestApp::launch();
        let namespace = FakeAccount::new(1, 0).seed(&app);
        let hold = Hold::default();
        let delete =
            app.cloud.script_next(CloudCall::DeleteNamespace, CloudFault::Hold(hold.clone()));
        load_detail(&mut app);

        app.dispatch(CloudBackupManagerAction::DisableCloudBackup).then_can(&[N::ConfirmDisable]);
        app.confirm_turn_off().then_cannot(&[N::TurnOff, N::Verify, N::Enable]);
        app.block_on("the held delete to start", hold.entered());
        assert!(app.cloud.calls().iter().any(|(id, _)| *id == delete));
        assert!(app.cloud.inner.has_namespace(&namespace));

        app.event("release the held delete", |_| hold.release())
            .then_can(&[N::Enable, N::RestoreFromCloudBackup])
            .then_cannot(&[N::TurnOff]);
        app.block_on("the held delete to return", hold.finished());
        assert!(!app.cloud.inner.has_namespace(&namespace));
        assert_eq!(persisted(), PersistedCloudBackupState::Disabled);
    });
}

#[test]
fn scripted_namespace_delete_failure_then_retry_disables() {
    let test = concat!(module_path!(), "::scripted_namespace_delete_failure_then_retry_disables");
    run_case(test, &[PhaseEnd::Exit], |_| {
        let mut app = TestApp::launch();
        let namespace = FakeAccount::new(1, 0).seed(&app);
        app.cloud.script(
            CallId::new(CloudCall::DeleteNamespace, 1),
            CloudFault::Fail(CloudStorageError::Offline("scripted".into())),
        );
        load_detail(&mut app);

        // the committed Turn Off failed, so the screen offers to resume it
        app.dispatch(CloudBackupManagerAction::DisableCloudBackup).then_can(&[N::ConfirmDisable]);
        app.confirm_turn_off().then_can(&[N::ResumeDisable]).then_cannot(&[
            N::TurnOff,
            N::Enable,
            N::Verify,
            N::StopUsingOnThisDevice,
        ]);
        assert!(app.cloud.inner.has_namespace(&namespace));
        assert_eq!(persisted().status(), PersistedCloudBackupStatus::Disabling);

        app.dispatch(CloudBackupManagerAction::ResumeDisable)
            .then_can(&[N::Enable])
            .then_cannot(&[N::TurnOff]);
        assert_eq!(app.cloud.count(CloudCall::DeleteNamespace), 2);
        assert!(!app.cloud.inner.has_namespace(&namespace));
        assert_eq!(keychain_namespace(), None);
    });
}

/// A Turn Off refused by a backup that is not on this device persisted nothing, so the actions
/// its refusal asks for stay open, and Turn Off finishes once the user has taken them
#[test]
fn turn_off_review_of_a_wallet_not_on_device_leaves_the_actions_that_clear_its_risk() {
    let test = concat!(
        module_path!(),
        "::turn_off_review_of_a_wallet_not_on_device_leaves_the_actions_that_clear_its_risk"
    );
    run_case(test, &[PhaseEnd::Exit], |_| {
        let mut app = TestApp::launch_with_passkey(PasskeyMode::FailOnAnyInteractiveCall);
        let account = FakeAccount::new(1, 1).seed_records(&app);
        let [elsewhere] = account.not_on_device.as_slice() else {
            // nothing read from the account: a panic message reaches the test log
            panic!("expected exactly one record not on this device");
        };
        load_detail(&mut app);

        app.dispatch(CloudBackupManagerAction::DisableCloudBackup)
            .then_can(&[N::ConfirmDisable, N::CancelDisable])
            .then_cannot(&[N::Enable]);
        let review = app.turn_off_review().expect("the Turn Off review");
        assert_eq!(review.impact.confirmation, CloudBackupDisableConfirmation::Two);
        assert_eq!(review.impact.not_on_device.names.len(), 1, "{review:?}");
        assert_eq!(persisted().status(), PersistedCloudBackupStatus::Enabled);
        assert_eq!(app.cloud.count(CloudCall::DeleteNamespace), 0);

        app.dispatch(CloudBackupManagerAction::CancelDisable)
            .then_can(&[N::TurnOff, N::Verify])
            .then_cannot(&[N::ConfirmDisable, N::Enable]);
        let delete = UserIntent::DeleteCloudWallet(elsewhere.clone().into());
        assert_eq!(app.admission_of(delete), Admission::Accepted, "{}", app.describe());
        app.dispatch(CloudBackupManagerAction::DeleteCloudWallet(elsewhere.clone().into()))
            .then_can(&[N::TurnOff]);
        assert!(!app.cloud.inner.has_wallet_backup(&account.namespace, elsewhere));

        // with the wallet gone nothing else is at risk, so the review asks once
        app.dispatch(CloudBackupManagerAction::DisableCloudBackup).then_can(&[N::ConfirmDisable]);
        let review = app.turn_off_review().expect("the second Turn Off review");
        assert_eq!(review.impact.confirmation, CloudBackupDisableConfirmation::One, "{review:?}");
        app.confirm_turn_off().then_can(&[N::Enable]).then_cannot(&[N::TurnOff]);
        assert_eq!(persisted(), PersistedCloudBackupState::Disabled, "{}", app.describe());
        assert!(!app.cloud.inner.has_namespace(&account.namespace));
        assert!(app.passkey.interactive_calls().is_empty());
    });
}

#[test]
fn killed_after_namespace_delete_resumes_to_disabled_on_next_launch() {
    let test = concat!(
        module_path!(),
        "::killed_after_namespace_delete_resumes_to_disabled_on_next_launch"
    );
    run_case(test, &[PhaseEnd::KillAt("namespace-deleted"), PhaseEnd::Exit], |phase| {
        if phase == 0 {
            let mut app = TestApp::launch();
            FakeAccount::new(1, 0).seed(&app);
            app.cloud.script_next(
                CloudCall::DeleteNamespace,
                CloudFault::KillAfter("namespace-deleted"),
            );
            load_detail(&mut app);

            app.dispatch(CloudBackupManagerAction::DisableCloudBackup)
                .then_can(&[N::ConfirmDisable]);
            let _killed = app.confirm_turn_off();
            unreachable!("the parent kills this process at the namespace-deleted checkpoint");
        }

        // a new process: only the database file, keychain file and cloud file survived
        open_durable_state();
        let PersistedCloudBackupState::ExclusiveTransition(_) = persisted() else {
            panic!("the kill left {:?}, expected the committed disable", persisted());
        };

        let mut app = TestApp::launch_with_passkey(PasskeyMode::FailOnAnyInteractiveCall);
        app.current().then_can(&[N::Enable]).then_cannot(&[N::TurnOff]);
        assert_eq!(persisted(), PersistedCloudBackupState::Disabled);
        assert_eq!(keychain_namespace(), None);
    });
}

/// The reporter's record heals at load: a disable that never deleted anything becomes its
/// configuration again, with a persisted notice, and nothing prompts for a passkey
#[test]
fn legacy_stuck_disabling_record_heals_at_load() {
    let test = concat!(module_path!(), "::legacy_stuck_disabling_record_heals_at_load");
    run_case(test, &[PhaseEnd::Exit, PhaseEnd::Exit, PhaseEnd::Exit], |phase| {
        let account = FakeAccount::lived_in();
        assert_eq!(account.record_count(), 35);

        if phase == 0 {
            let app = TestApp::launch();
            let namespace = account.seed(&app);
            seed_legacy_stuck_disabling(&namespace, app.clock.now_secs());
            return;
        }

        let mut app = TestApp::launch_with_passkey(PasskeyMode::FailOnAnyInteractiveCall);

        // writes are allowed again; Turn Off still waits for the detail inventory today
        app.current().then_can(&[N::Verify]).then_cannot(&[N::Enable]);
        let PersistedCloudBackupState::Configured(configured) = persisted() else {
            panic!("the legacy record did not heal:\n{}", app.describe());
        };
        // phase 2 is a full relaunch: the notice is still on the record it healed
        assert!(configured.heal_notice.is_some(), "phase {phase}: the heal notice is persisted");
        assert_eq!(app.cloud.count(CloudCall::DeleteNamespace), 0);
        assert!(app.passkey.interactive_calls().is_empty());
        assert!(keychain_namespace().is_some(), "the heal keeps the namespace");
    });
}

/// The UI test seam writes the legacy record at launch, only over a configured backup, and the
/// same launch heals it: the screen shows the notice until the user dismisses it
#[test]
fn ui_test_seam_seeds_the_legacy_record_that_the_launch_heals() {
    let test =
        concat!(module_path!(), "::ui_test_seam_seeds_the_legacy_record_that_the_launch_heals");
    run_case(test, &[PhaseEnd::Exit, PhaseEnd::Exit, PhaseEnd::Exit], |phase| {
        let root = child_root().expect("a harness child has a data root");
        let request = root.join(ui_test_legacy_record::REQUEST_FILE);

        if phase == 0 {
            let _app = TestApp::launch();
            std::fs::write(&request, b"").expect("write the seed request");
            return;
        }

        // SAFETY: the child runs this one test on one thread, and nothing has started reading
        // the environment yet
        unsafe { std::env::set_var(ui_test_legacy_record::LEGACY_STUCK_RECORD_ENV, &root) };

        if phase == 1 {
            // nothing is configured, so there is nothing to seed over: the request waits
            let app = TestApp::launch();
            assert_eq!(persisted(), PersistedCloudBackupState::Disabled);
            assert!(request.exists(), "a seed with nothing to replace kept its request");

            FakeAccount::new(2, 0).seed(&app);
            return;
        }

        let mut app = TestApp::launch_with_passkey(PasskeyMode::FailOnAnyInteractiveCall);
        assert!(!request.exists(), "the seed deleted its request");

        let PersistedCloudBackupState::Configured(configured) = persisted() else {
            panic!("the seeded record did not heal:\n{}", app.describe());
        };
        assert!(configured.heal_notice.is_some(), "the heal persisted its notice");

        let screen = app.manager().published_screen();
        assert!(screen.heal_notice.is_some(), "the screen shows the heal notice");
        assert_eq!(screen.settings_row, CloudBackupSettingsRowStatus::TurnedOnAgain);
        assert!(screen.actions.is_available(CloudBackupScreenAction::DismissHealNotice));

        app.dispatch(CloudBackupManagerAction::DismissNotice(CloudBackupNoticeKind::Heal))
            .then_cannot(&[N::Enable]);
        let PersistedCloudBackupState::Configured(configured) = persisted() else {
            panic!("dismissing the notice changed the record:\n{}", app.describe());
        };
        assert_eq!(configured.heal_notice, None);

        let screen = app.manager().published_screen();
        assert_eq!(screen.heal_notice, None);
        assert!(screen.actions.availability(CloudBackupScreenAction::DismissHealNotice).is_none());
        assert!(app.passkey.interactive_calls().is_empty());
    });
}

/// A failed heal write leaves the legacy record unchanged and fails the load, never Disabled;
/// the next launch heals it
#[test]
fn failed_heal_write_keeps_the_legacy_record_and_fails_the_load() {
    let test =
        concat!(module_path!(), "::failed_heal_write_keeps_the_legacy_record_and_fails_the_load");
    run_case(test, &[PhaseEnd::Exit, PhaseEnd::Exit, PhaseEnd::Exit], |phase| {
        let account = FakeAccount::new(2, 0);

        if phase == 0 {
            let app = TestApp::launch();
            let namespace = account.seed(&app);
            seed_legacy_stuck_disabling(&namespace, app.clock.now_secs());
            return;
        }

        if phase == 1 {
            let mut app = TestApp::launch_with_bootstrap(
                PasskeyMode::FailOnAnyInteractiveCall,
                bootstrap_with_failed_heal,
            );
            let stuck = persisted();
            let unchanged =
                stuck.disabling().is_some_and(|record| record.delete_started_at.is_none());
            assert!(unchanged, "the failed heal changed the record:\n{}", app.describe());
            app.current().then_cannot(&[N::Enable, N::Verify, N::SyncUnsynced, N::TurnOff]);
            assert_eq!(persisted(), stuck, "{}", app.describe());
            // the record gives no status; the owner's `LoadFailed` body shows the load failure
            assert_eq!(RustCloudBackupManager::runtime_status_for(&persisted()), None);
            assert_eq!(
                app.manager().current_status(),
                CloudBackupStatus::Error(CLOUD_BACKUP_LOAD_FAILED_MESSAGE.into()),
                "{}",
                app.describe()
            );

            // Turn Off cannot run on an unnormalized record, and nothing is deleted
            app.dispatch(CloudBackupManagerAction::DisableCloudBackup).then_cannot(&[N::Enable]);
            assert_eq!(persisted(), stuck, "{}", app.describe());
            assert_eq!(app.cloud.count(CloudCall::DeleteNamespace), 0);
            return;
        }

        let mut app = TestApp::launch_with_passkey(PasskeyMode::FailOnAnyInteractiveCall);
        app.current().then_can(&[N::Verify]);
        let PersistedCloudBackupState::Configured(configured) = persisted() else {
            panic!("the relaunch did not heal:\n{}", app.describe());
        };
        assert!(configured.heal_notice.is_some());
    });
}

/// A failed load offers Retry, which reads and heals the record as a launch would, with no
/// relaunch
#[test]
fn retry_after_a_failed_load_heals_without_a_relaunch() {
    let test = concat!(module_path!(), "::retry_after_a_failed_load_heals_without_a_relaunch");
    run_case(test, &[PhaseEnd::Exit, PhaseEnd::Exit], |phase| {
        if phase == 0 {
            let app = TestApp::launch();
            let namespace = FakeAccount::new(2, 0).seed(&app);
            seed_legacy_stuck_disabling(&namespace, app.clock.now_secs());
            return;
        }

        let mut app = TestApp::launch_with_bootstrap(
            PasskeyMode::FailOnAnyInteractiveCall,
            bootstrap_with_failed_heal,
        );
        app.current().then_can(&[N::RetryLoad]).then_cannot(&[N::Enable, N::Verify, N::TurnOff]);
        assert_eq!(app.manager().published_screen().body, CloudBackupScreenBody::LoadFailed);

        app.dispatch(CloudBackupManagerAction::RetryLoad)
            .then_can(&[N::Verify, N::TurnOff])
            .then_cannot(&[N::RetryLoad, N::Enable]);

        let PersistedCloudBackupState::Configured(configured) = persisted() else {
            panic!("the retry did not heal the record:\n{}", app.describe());
        };
        assert!(configured.heal_notice.is_some());

        let screen = app.manager().published_screen();
        assert!(matches!(screen.body, CloudBackupScreenBody::Configured(_)), "{screen:#?}");
        assert!(screen.heal_notice.is_some());
        assert_eq!(screen.settings_row, CloudBackupSettingsRowStatus::TurnedOnAgain);
        assert_eq!(app.cloud.count(CloudCall::DeleteNamespace), 0);
    });
}

/// A corrupted record holds Enable and Restore, and its one exit resets Cloud Backup on this
/// device without touching the cloud, after which it can be turned on again
#[test]
fn corrupted_record_resets_on_this_device_before_enable() {
    let test = concat!(module_path!(), "::corrupted_record_resets_on_this_device_before_enable");
    run_case(test, &[PhaseEnd::Exit, PhaseEnd::Exit], |phase| {
        if phase == 0 {
            let app = TestApp::launch();
            FakeAccount::new(2, 0).seed(&app);
            seed_corrupted_record("decode failed");
            return;
        }

        let mut app = TestApp::launch_with_passkey(PasskeyMode::FailOnAnyInteractiveCall);
        let namespace = keychain_namespace().expect("the seeded namespace");
        app.current().then_can(&[N::ResetOnThisDevice]).then_cannot(&[
            N::Enable,
            N::RestoreFromCloudBackup,
            N::TurnOff,
        ]);

        let screen = app.manager().published_screen();
        assert_eq!(screen.body, CloudBackupScreenBody::Corrupted);
        assert!(matches!(screen.settings_row, CloudBackupSettingsRowStatus::Error(_)));
        assert_eq!(
            app.admission_of(N::Enable.intent(None)),
            Admission::Rejected(Rejection::Corrupted)
        );

        app.dispatch(CloudBackupManagerAction::ResetOnThisDevice)
            .then_can(&[N::Enable, N::RestoreFromCloudBackup])
            .then_cannot(&[N::ResetOnThisDevice, N::TurnOff]);

        assert_eq!(persisted(), PersistedCloudBackupState::Disabled, "{}", app.describe());
        assert_eq!(keychain_namespace(), None);
        assert_eq!(app.manager().published_screen().body, CloudBackupScreenBody::Disabled);
        assert_eq!(app.manager().published_screen().last_outcome, None);

        // the backup in the cloud is untouched, so it can still be restored
        assert!(app.cloud.inner.has_namespace(&namespace));
        assert_eq!(app.cloud.count(CloudCall::DeleteNamespace), 0);
    });
}

/// The scripted faults at the provider boundary, without a manager
#[test]
fn cloud_faults_answer_by_call_identity() {
    let test = concat!(module_path!(), "::cloud_faults_answer_by_call_identity");
    run_case(test, &[PhaseEnd::Exit], |_| {
        use cove_device::cloud_storage::{CloudAccessPolicy, CloudStorageAccess as _};
        use futures::executor::block_on;

        let file = child_root().expect("case root").join("cloud.json");
        let cloud = HarnessCloud::load(MockCloudStorage::default(), file.clone());
        let policy = CloudAccessPolicy::Silent;
        let files = |ids: &[&str]| -> Vec<String> {
            ids.iter()
                .map(|id| cove_cspp::backup_data::wallet_filename_from_record_id(id))
                .collect()
        };

        for record in ["a", "b", "c"] {
            cloud.inner.set_wallet_backup("ns".into(), record.into(), vec![1]);
        }
        cloud.inner.set_wallet_files("ns".into(), files(&["a", "b", "c"]));

        // listings that differ between calls
        cloud
            .script(CallId::new(CloudCall::ListWalletFiles, 2), CloudFault::Listing(files(&["a"])));
        let first = block_on(cloud.list_wallet_files("ns".into(), policy)).expect("first listing");
        let second =
            block_on(cloud.list_wallet_files("ns".into(), policy)).expect("second listing");
        assert_eq!(first.len(), 3);
        assert_eq!(second, files(&["a"]));

        // a partial namespace delete removes what it names, then errors
        let error = CloudStorageError::Offline("dropped".into());
        cloud.script_next(
            CloudCall::DeleteNamespace,
            CloudFault::PartialDeleteThenFail { removed: vec!["a".into()], error: error.clone() },
        );
        let partial = block_on(cloud.delete_namespace("ns".into(), policy));
        assert_eq!(partial, Err(error.clone()));
        assert!(!cloud.inner.has_wallet_backup("ns", "a"));
        assert!(cloud.inner.has_wallet_backup("ns", "b"));

        // the delete lands but its response is lost
        cloud.script_next(CloudCall::DeleteNamespace, CloudFault::SucceedThenFail(error.clone()));
        assert_eq!(block_on(cloud.delete_namespace("ns".into(), policy)), Err(error));
        assert!(!cloud.inner.has_namespace("ns"));

        // a kill point outside its crash phase is a no-op
        cloud.script_next(CloudCall::ListNamespaces, CloudFault::KillBefore("not-this-phase"));
        assert!(block_on(cloud.list_namespaces(policy)).is_ok());

        // authorization required, as a provider reports it
        cloud.script_next(
            CloudCall::ListNamespaces,
            CloudFault::Fail(CloudStorageError::AuthorizationRequired("reconnect".into())),
        );
        assert!(matches!(
            block_on(cloud.list_namespaces(policy)),
            Err(CloudStorageError::AuthorizationRequired(_))
        ));

        // every mutation reached the file a resumed process loads
        let resumed = HarnessCloud::load(MockCloudStorage::default(), file);
        assert!(!resumed.inner.has_namespace("ns"));
    });
}

/// Interactive passkey answers by mode, with entered and finished acknowledgements
#[test]
fn passkey_modes_answer_interactive_calls() {
    let test = concat!(module_path!(), "::passkey_modes_answer_interactive_calls");
    run_case(test, &[PhaseEnd::Exit], |_| {
        use cove_device::passkey::{PasskeyError, PasskeyProvider as _};

        use super::{HarnessPasskey, PasskeyCall};
        use crate::manager::cloud_backup_manager::ops::test_support::MockPasskeyProviderImpl;

        let passkey = HarnessPasskey::new(MockPasskeyProviderImpl::default());
        let authenticate = |passkey: &HarnessPasskey| {
            passkey.authenticate_with_prf("rp".into(), vec![1], vec![2], vec![3])
        };

        passkey.set_mode(PasskeyMode::NotCompleted);
        assert!(matches!(authenticate(&passkey), Err(PasskeyError::NotCompleted)));

        passkey.set_mode(PasskeyMode::ProviderFailure);
        assert!(matches!(authenticate(&passkey), Err(PasskeyError::RequestFailed { .. })));

        let hold = Hold::default();
        passkey.set_mode(PasskeyMode::Slow(hold.clone()));
        let slow = std::thread::spawn({
            let passkey = passkey.clone();
            move || authenticate(&passkey).is_ok()
        });
        futures::executor::block_on(hold.entered());
        assert!(!slow.is_finished(), "a held request answered before release");
        hold.release();
        futures::executor::block_on(hold.finished());
        slow.join().expect("slow passkey thread");

        passkey.set_mode(PasskeyMode::FailOnAnyInteractiveCall);
        assert!(authenticate(&passkey).is_err());
        assert_eq!(passkey.forbidden_calls(), vec![CallId::new(PasskeyCall::Authenticate, 4)]);
        assert_eq!(passkey.interactive_calls().len(), 4);
    });
}

/// A hang fails the phase in seconds with what it was waiting on, instead of parking forever
#[test]
fn a_hold_that_never_starts_fails_at_its_deadline() {
    let test = concat!(module_path!(), "::a_hold_that_never_starts_fails_at_its_deadline");
    let never_started = "held cloud call DeleteNamespace #1 to start";
    run_case(test, &[PhaseEnd::TimesOutOn(never_started)], |_| {
        let file = child_root().expect("case root").join("cloud.json");
        let cloud = HarnessCloud::load(MockCloudStorage::default(), file);
        let hold = Hold::default();
        cloud.script_next(CloudCall::DeleteNamespace, CloudFault::Hold(hold.clone()));

        futures::executor::block_on(hold.entered());
        unreachable!("nothing calls delete_namespace, so the deadline ends this phase");
    });
}

/// Turn Off is admitted from the lifecycle alone: the detail inventory is not one of its inputs,
/// so a tap before the inventory loads reviews the backup, and consent turns it off, instead of
/// being dropped
#[test]
fn turn_off_is_admitted_before_the_detail_inventory_loads() {
    let test = concat!(module_path!(), "::turn_off_is_admitted_before_the_detail_inventory_loads");
    run_case(test, &[PhaseEnd::Exit], |_| {
        let mut app = TestApp::launch();
        let namespace = FakeAccount::new(1, 0).seed(&app);
        assert!(!app.manager().listing_is_provider_confirmed(), "{}", app.describe());

        app.current().then_can(&[N::TurnOff]);
        app.dispatch(CloudBackupManagerAction::DisableCloudBackup).then_can(&[N::ConfirmDisable]);
        let outcome = app.last_outcome().expect("dispatch recorded an outcome");
        assert_eq!((outcome.intent, outcome.admission), (UserIntent::TurnOff, Admission::Accepted));

        app.confirm_turn_off().then_can(&[N::Enable]).then_cannot(&[N::TurnOff]);
        assert_eq!(persisted(), PersistedCloudBackupState::Disabled, "{}", app.describe());
        assert!(!app.cloud.inner.has_namespace(&namespace));
    });
}

/// A soft relaunch builds a new manager; dispatching to it runs on that manager, not on the
/// process singleton the first launch created
#[test]
fn dispatch_runs_on_the_manager_it_is_called_on() {
    let test = concat!(module_path!(), "::dispatch_runs_on_the_manager_it_is_called_on");
    run_case(test, &[PhaseEnd::Exit], |_| {
        let mut app = TestApp::launch();
        let namespace = FakeAccount::new(1, 0).seed(&app);
        let first = app.manager().clone();

        app.relaunch().then_can(&[N::TurnOff]);
        assert!(!Arc::ptr_eq(&first, app.manager()));
        app.dispatch(CloudBackupManagerAction::DisableCloudBackup).then_can(&[N::ConfirmDisable]);
        let outcome = app.last_outcome().expect("the new manager admitted the intent");
        assert_eq!(outcome.admission, Admission::Accepted);

        app.confirm_turn_off().then_can(&[N::Enable]);
        assert_eq!(persisted(), PersistedCloudBackupState::Disabled, "{}", app.describe());
        assert!(!app.cloud.inner.has_namespace(&namespace));
    });
}

/// Admission parity: an action the app showed as available, dispatched after the state moved
/// on, gets one rejection with its reason and changes nothing
#[test]
fn an_action_dispatched_after_the_state_moved_on_is_rejected_with_a_reason() {
    let test = concat!(
        module_path!(),
        "::an_action_dispatched_after_the_state_moved_on_is_rejected_with_a_reason"
    );
    run_case(test, &[PhaseEnd::Exit], |_| {
        let mut app = TestApp::launch();
        FakeAccount::new(1, 0).seed(&app);
        app.current().then_can(&[N::TurnOff, N::Verify]);

        // the screen still offers Verify when Turn Off finishes underneath it
        app.dispatch(CloudBackupManagerAction::DisableCloudBackup).then_can(&[N::ConfirmDisable]);
        app.confirm_turn_off().then_cannot(&[N::Verify]);
        let disabled = persisted();
        let deletes = app.cloud.count(CloudCall::DeleteNamespace);

        let source = CloudBackupVerificationSource::CloudBackupDetail;
        app.dispatch(CloudBackupManagerAction::StartVerification(source))
            .then_can(&[N::Enable])
            .then_cannot(&[N::Verify, N::TurnOff]);
        let outcome = app.last_outcome().expect("the rejection is recorded");
        assert_eq!(outcome.admission, Admission::Rejected(Rejection::CloudBackupOff));
        assert_eq!(persisted(), disabled);
        assert_eq!(app.cloud.count(CloudCall::DeleteNamespace), deletes);
    });
}

/// A phase whose runtime is inside SQLCipher when its body returns stops that work before exit
///
/// Upload work opens wallets' SQLCipher stores on the case runtime, and process exit runs
/// OpenSSL's atexit cleanup, which frees the state that work uses. This keeps a task calling into
/// SQLCipher when the body returns, then holds exit open after the cleanup and fails the child if
/// the task made another call in that window: the race that crashes a loaded machine with SIGSEGV
#[test]
fn a_phase_stops_its_sqlcipher_work_before_openssl_cleanup() {
    let test = concat!(module_path!(), "::a_phase_stops_its_sqlcipher_work_before_openssl_cleanup");
    run_case(test, &[PhaseEnd::Exit], |_| {
        static CALLS: AtomicU64 = AtomicU64::new(0);

        // atexit handlers run last-registered first, and OpenSSL registers its cleanup on first
        // use below, so this runs after the cleanup
        extern "C" fn fail_on_sqlcipher_calls_after_openssl_cleanup() {
            let before = CALLS.load(Ordering::SeqCst);
            std::thread::sleep(Duration::from_millis(200));
            let after = CALLS.load(Ordering::SeqCst);
            if after == before {
                return;
            }

            eprintln!("{} SQLCipher calls ran after OpenSSL's exit cleanup", after - before);
            // SAFETY: ends the process without running the remaining exit handlers
            unsafe { libc::_exit(102) };
        }

        // SAFETY: the handler only reads an atomic, sleeps and may end the process
        let registered = unsafe { libc::atexit(fail_on_sqlcipher_calls_after_openssl_cleanup) };
        assert_eq!(registered, 0, "register the exit check");

        let path = child_root().expect("case root").join("churn.sqlite");
        let (started, on_started) = std::sync::mpsc::sync_channel(1);

        super::runtime::install().spawn(async move {
            let conn = rusqlite::Connection::open(&path).expect("open the churn database");
            conn.pragma_update(None, "key", "churn").expect("key the churn database");
            conn.execute_batch("CREATE TABLE churn (page BLOB)").expect("create the churn table");
            let _ = started.send(());

            // every commit encrypts its pages through OpenSSL; a failed call still counts, and
            // the loop goes on so a failure can't end the work early
            loop {
                CALLS.fetch_add(1, Ordering::SeqCst);
                let _ = conn.execute_batch(
                    "INSERT INTO churn VALUES (randomblob(65536)); DELETE FROM churn",
                );
                tokio::task::yield_now().await;
            }
        });

        let _deadline = super::deadline::Deadline::arm("the SQLCipher churn task to start");
        on_started.recv().expect("the churn task started");
    });
}

/// The inventory loads on every silent path without a passkey call, and with no local master key
/// it lists the records it could not check instead of asking for one
#[test]
fn silent_inventory_paths_never_prompt() {
    use act_zero::send;

    use crate::manager::cloud_backup_manager::screen::CloudBackupListedRecord;
    use crate::manager::cloud_backup_manager::{CloudBackupKeychain, CloudOnlyRow};
    use crate::manager::connectivity_manager::CONNECTIVITY_MANAGER;

    let test = concat!(module_path!(), "::silent_inventory_paths_never_prompt");
    run_case(test, &[PhaseEnd::Exit], |_| {
        let mut app = TestApp::launch_with_passkey(PasskeyMode::FailOnAnyInteractiveCall);
        FakeAccount::new(2, 2).seed(&app);

        // detail entry and Check Again
        app.dispatch(CloudBackupManagerAction::EnterDetail).then_cannot(&[N::Enable]);
        load_detail(&mut app);

        // a refresh that fails offline, then the connection comes back and the user checks again
        app.cloud.inner.fail_next_list_wallet_files_offline("offline");
        app.dispatch(CloudBackupManagerAction::RefreshDetail).then_cannot(&[N::Enable]);
        app.event("reconnect", |_| CONNECTIVITY_MANAGER.set_connection_state(true))
            .then_cannot(&[N::Enable]);
        load_detail(&mut app);

        // the provider reports a cloud change
        app.event("cloud change", |app| {
            send!(app.manager().supervisor.provider_inventory_did_change());
        })
        .then_cannot(&[N::Enable]);

        // a relaunch, then a relaunch with no local master key
        app.relaunch().then_cannot(&[N::Enable]);
        app.dispatch(CloudBackupManagerAction::EnterDetail).then_cannot(&[N::Enable]);
        CloudBackupKeychain::global().delete_master_key().expect("delete local master key");
        app.relaunch().then_cannot(&[N::Enable]);
        app.dispatch(CloudBackupManagerAction::EnterDetail).then_cannot(&[N::Enable]);
        load_detail(&mut app);

        let rows = app
            .manager()
            .inventory
            .read()
            .cloud_only()
            .loaded_rows()
            .map(|rows| rows.iter().map(CloudOnlyRow::listed_record).collect::<Vec<_>>());
        assert_eq!(rows, Some(vec![CloudBackupListedRecord::Unchecked; 2]), "{}", app.describe());
        assert!(
            app.passkey.interactive_calls().is_empty(),
            "{:?}",
            app.passkey.interactive_calls()
        );
    });
}

/// The passkey card the published screen draws, if any
fn passkey_card(app: &TestApp) -> Option<CloudBackupPasskeyState> {
    configured_sections(app).into_iter().find_map(|section| match section {
        CloudBackupSection::PasskeyCard(passkey) => Some(passkey),
        _ => None,
    })
}

/// The state the published screen's verification section shows
fn verification_state(app: &TestApp) -> Option<CloudBackupVerificationState> {
    configured_sections(app).into_iter().find_map(|section| match section {
        CloudBackupSection::Verification(section) => Some(section.view.state),
        _ => None,
    })
}

/// The controls the published screen's verification section draws
fn verification_controls(app: &TestApp) -> Vec<CloudBackupVerificationControl> {
    configured_sections(app)
        .into_iter()
        .find_map(|section| match section {
            CloudBackupSection::Verification(section) => Some(section.controls),
            _ => None,
        })
        .unwrap_or_default()
}

fn configured_sections(app: &TestApp) -> Vec<CloudBackupSection> {
    match app.manager().published_screen().body {
        CloudBackupScreenBody::Configured(configured) => configured.sections,
        _ => Vec::new(),
    }
}

/// Whether the published screen draws `action`, and whether it is enabled
fn control(
    app: &TestApp,
    action: CloudBackupScreenAction,
) -> Option<CloudBackupActionAvailability> {
    app.manager().published_screen().actions.availability(action)
}

fn verify() -> CloudBackupManagerAction {
    CloudBackupManagerAction::StartVerification(CloudBackupVerificationSource::CloudBackupDetail)
}

/// No passkey opens the backup, so verification asks for a new one
fn script_missing_passkey(app: &TestApp, create: PasskeyError) {
    let mock = &app.passkey.inner;
    mock.set_authenticate_result(Err(PasskeyError::NoCredentialFound));
    mock.set_discover_result(Err(PasskeyError::NoCredentialFound));
    mock.set_create_result(Err(create));
}

/// A verification that fails on the passkey shows the needs-repair card, which offers Add New
/// Passkey, and keeps Verify
#[test]
fn a_verification_that_fails_on_the_passkey_offers_a_repair() {
    let test =
        concat!(module_path!(), "::a_verification_that_fails_on_the_passkey_offers_a_repair");
    run_case(test, &[PhaseEnd::Exit], |_| {
        let mut app = TestApp::launch();
        FakeAccount::new(1, 0).seed(&app);
        seed_verifiable_master_key(&app);
        app.current().then_can(&[N::Verify]);
        assert_eq!(passkey_card(&app), None);
        assert_eq!(control(&app, CloudBackupScreenAction::RepairPasskey), None);

        // the provider can't make the replacement passkey
        script_missing_passkey(
            &app,
            PasskeyError::RequestFailed {
                operation: PasskeyOperation::Registration,
                reason: PasskeyFailureReason::PlatformAuthorizationFailedAfterPresentation,
            },
        );

        app.dispatch(verify()).then_can(&[N::Verify]);
        assert_eq!(
            passkey_card(&app),
            Some(CloudBackupPasskeyState::NeedsRepair {
                state: CloudBackupPasskeyRepairState::Idle
            })
        );
        assert_eq!(
            control(&app, CloudBackupScreenAction::RepairPasskey),
            Some(CloudBackupActionAvailability::Available)
        );
        assert_eq!(persisted().status(), PersistedCloudBackupStatus::PasskeyMissing);
    });
}

/// Dismissing the passkey sheet, while signing in or while making a replacement, only didn't
/// complete: the passkey is never marked missing and only Verify is offered again
#[test]
fn a_cancelled_verification_never_offers_a_passkey_repair() {
    let test = concat!(module_path!(), "::a_cancelled_verification_never_offers_a_passkey_repair");
    run_case(test, &[PhaseEnd::Exit], |_| {
        let mut app = TestApp::launch();
        FakeAccount::new(1, 0).seed(&app);
        seed_verifiable_master_key(&app);
        let status = persisted().status();

        app.passkey.set_mode(PasskeyMode::NotCompleted);
        app.dispatch(verify()).then_can(&[N::Verify]);
        assert_eq!(verification_state(&app), Some(CloudBackupVerificationState::NotCompleted));
        assert_eq!(passkey_card(&app), None);
        assert_eq!(control(&app, CloudBackupScreenAction::RepairPasskey), None);

        app.passkey.set_mode(PasskeyMode::Succeeds);
        script_missing_passkey(&app, PasskeyError::NotCompleted);
        app.dispatch(verify()).then_can(&[N::Verify]);
        assert_eq!(verification_state(&app), Some(CloudBackupVerificationState::NotCompleted));
        assert_eq!(passkey_card(&app), None);
        assert_eq!(control(&app, CloudBackupScreenAction::RepairPasskey), None);
        assert_eq!(persisted().status(), status);
    });
}

/// A verification that fails reading the cloud says nothing about the passkey, so it offers
/// Verify again and no repair
#[test]
fn a_verification_that_fails_off_the_passkey_offers_no_repair() {
    let test =
        concat!(module_path!(), "::a_verification_that_fails_off_the_passkey_offers_no_repair");
    run_case(test, &[PhaseEnd::Exit], |_| {
        let mut app = TestApp::launch();
        FakeAccount::new(1, 0).seed(&app);
        seed_verifiable_master_key(&app);

        let error = CloudStorageError::DownloadFailed("master key".into());
        app.cloud.script_next(CloudCall::DownloadMasterKey, CloudFault::Fail(error));
        app.dispatch(verify()).then_can(&[N::Verify]);
        assert!(matches!(verification_state(&app), Some(CloudBackupVerificationState::Failed(_))));
        assert_eq!(passkey_card(&app), None);
        assert_eq!(control(&app, CloudBackupScreenAction::RepairPasskey), None);
    });
}

/// A backup whose wallet is uploaded and that verifies offers Verify and Turn Off; it offers no
/// Sync Now with nothing to sync and no rebuild with nothing broken, since nothing draws them
#[test]
fn a_clean_backup_offers_no_sync_and_no_rebuild() {
    let test = concat!(module_path!(), "::a_clean_backup_offers_no_sync_and_no_rebuild");
    run_case(test, &[PhaseEnd::Exit], |_| {
        let mut app = TestApp::launch();
        FakeAccount::new(1, 0).seed(&app);
        seed_verifiable_master_key(&app);
        load_detail(&mut app);

        app.dispatch(verify()).then_can(&[N::Verify, N::TurnOff]);
        assert!(matches!(
            verification_state(&app),
            Some(CloudBackupVerificationState::Verified { .. })
        ));
        assert_eq!(verification_controls(&app), vec![CloudBackupVerificationControl::Verify]);

        for action in [A::SyncUnsynced, A::RecreateManifest, A::ReinitializeBackup] {
            assert_eq!(control(&app, action), None, "a clean backup offers {action:?}");
        }
    });
}

/// A backup with no wallets has nothing to sync, so it offers no Sync Now
#[test]
fn an_empty_backup_offers_no_sync() {
    let test = concat!(module_path!(), "::an_empty_backup_offers_no_sync");
    run_case(test, &[PhaseEnd::Exit], |_| {
        let mut app = TestApp::launch();
        FakeAccount::new(0, 0).seed(&app);
        load_detail(&mut app);

        app.current().then_can(&[N::Verify, N::TurnOff]);
        for action in [A::SyncUnsynced, A::RecreateManifest, A::ReinitializeBackup] {
            assert_eq!(control(&app, action), None, "an empty backup offers {action:?}");
        }
    });
}

/// A backup only a newer Cove can read fails verification with no recovery here: verifying
/// again can't read it either, and a rebuild would overwrite it, so the screen offers neither
#[test]
fn a_backup_from_a_newer_version_offers_no_verify_and_no_rebuild() {
    let test =
        concat!(module_path!(), "::a_backup_from_a_newer_version_offers_no_verify_and_no_rebuild");
    run_case(test, &[PhaseEnd::Exit], |_| {
        let mut app = TestApp::launch();
        let namespace = FakeAccount::new(1, 0).seed(&app);
        seed_verifiable_master_key(&app);
        write_newer_master_key_version(&app, &namespace);
        load_detail(&mut app);

        app.dispatch(verify()).then_can(&[N::TurnOff]);
        assert!(matches!(
            verification_state(&app),
            Some(CloudBackupVerificationState::Failed(
                DeepVerificationFailure::UnsupportedVersion { .. }
            ))
        ));
        assert_eq!(verification_controls(&app), Vec::new());

        for action in [A::Verify, A::RecreateManifest, A::ReinitializeBackup] {
            assert_eq!(control(&app, action), None, "an unreadable backup offers {action:?}");
        }
    });
}

/// Marks the cloud's master key backup as written by a newer Cove's format
fn write_newer_master_key_version(app: &TestApp, namespace: &str) {
    let bytes = app.cloud.inner.master_key_backup_bytes(namespace).expect("a master key backup");
    let mut json: serde_json::Value = serde_json::from_slice(&bytes).expect("master key json");
    json["version"] = serde_json::json!(99);

    let bytes = serde_json::to_vec(&json).expect("encode master key json");
    app.cloud.inner.set_master_key_backup(namespace.to_string(), bytes);
    app.cloud.persist();
}
