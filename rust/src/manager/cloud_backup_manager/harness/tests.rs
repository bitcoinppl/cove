//! Proof that the harness drives the real manager: runtime, clock, relaunch, call-identity
//! fakes and real process kills
//!
//! Each test body runs in child processes; see [`super::run_case`]
#![cfg(test)]

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use act_zero::call;
use cove_device::cloud_storage::CloudStorageError;

use super::process::child_root;
use super::{
    CallId, CloudCall, CloudFault, FakeAccount, HarnessCloud, Hold, NextAction as N, PasskeyMode,
    PhaseEnd, TestApp, keychain_namespace, load_detail, open_durable_state, persisted, run_case,
    seed_legacy_stuck_disabling,
};
use crate::database::cloud_backup::{PersistedCloudBackupState, PersistedCloudBackupStatus};
use crate::manager::cloud_backup_manager::actors::write::Decision;
use crate::manager::cloud_backup_manager::admission::{
    Admission, PauseReason, Rejection, UserIntent,
};
use crate::manager::cloud_backup_manager::ops::CLOUD_BACKUP_LOAD_FAILED_MESSAGE;
use crate::manager::cloud_backup_manager::ops::test_support::MockCloudStorage;
use crate::manager::cloud_backup_manager::owner::lifecycle::test_support::bootstrap_with_failed_heal;
use crate::manager::cloud_backup_manager::{
    CloudBackupManagerAction, CloudBackupVerificationSource,
};
use crate::manager::cloud_backup_manager::{CloudBackupStatus, RustCloudBackupManager};

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

#[test]
fn soft_relaunch_keeps_durable_state_without_prompting() {
    let test = concat!(module_path!(), "::soft_relaunch_keeps_durable_state_without_prompting");
    run_case(test, &[PhaseEnd::Exit], |_| {
        let mut app = TestApp::launch_with_passkey(PasskeyMode::FailOnAnyInteractiveCall);
        let namespace = FakeAccount::new(2, 0).seed(&app);

        // the relaunched app has not loaded its detail inventory, and Turn Off is still admitted
        app.relaunch()
            .then_can(&[N::Verify, N::SyncUnsynced, N::TurnOff])
            .then_cannot(&[N::Enable, N::KeepEnabled]);

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

        app.dispatch(CloudBackupManagerAction::DisableCloudBackup)
            .then_can(&[N::Enable])
            .then_cannot(&[N::TurnOff]);
        app.block_on("the released call to return", hold.finished());
        assert_eq!(persisted(), PersistedCloudBackupState::Disabled);
        assert_eq!(app.cloud.count(CloudCall::DeleteNamespace), 1);
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

        app.dispatch(CloudBackupManagerAction::DisableCloudBackup).then_cannot(&[
            N::TurnOff,
            N::KeepEnabled,
            N::Verify,
            N::Enable,
        ]);
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

        app.dispatch(CloudBackupManagerAction::DisableCloudBackup)
            .then_can(&[N::TurnOff])
            .then_cannot(&[N::Enable, N::Verify]);
        assert!(app.cloud.inner.has_namespace(&namespace));
        assert_eq!(persisted().status(), PersistedCloudBackupStatus::Disabling);

        app.dispatch(CloudBackupManagerAction::DisableCloudBackup)
            .then_can(&[N::Enable])
            .then_cannot(&[N::TurnOff]);
        assert_eq!(app.cloud.count(CloudCall::DeleteNamespace), 2);
        assert!(!app.cloud.inner.has_namespace(&namespace));
        assert_eq!(keychain_namespace(), None);
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

            let _killed = app.dispatch(CloudBackupManagerAction::DisableCloudBackup);
            unreachable!("the parent kills this process at the namespace-deleted checkpoint");
        }

        // a new process: only the database file, keychain file and cloud file survived
        open_durable_state();
        let PersistedCloudBackupState::ExclusiveTransition(_) = persisted() else {
            panic!("the kill left {:?}, expected the committed disable", persisted());
        };

        let mut app = TestApp::launch_with_passkey(PasskeyMode::FailOnAnyInteractiveCall);
        app.current().then_can(&[N::Enable]).then_cannot(&[N::TurnOff, N::KeepEnabled]);
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
        app.current()
            .then_can(&[N::Verify, N::SyncUnsynced])
            .then_cannot(&[N::Enable, N::KeepEnabled]);
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
            app.current().then_cannot(&[
                N::Enable,
                N::Verify,
                N::SyncUnsynced,
                N::TurnOff,
                N::KeepEnabled,
            ]);
            assert_eq!(persisted(), stuck, "{}", app.describe());
            assert_eq!(
                RustCloudBackupManager::runtime_status_for(&persisted()),
                CloudBackupStatus::Error(CLOUD_BACKUP_LOAD_FAILED_MESSAGE.into())
            );

            // Turn Off cannot run on an unnormalized record, and nothing is deleted
            app.dispatch(CloudBackupManagerAction::DisableCloudBackup).then_cannot(&[N::Enable]);
            assert_eq!(persisted(), stuck, "{}", app.describe());
            assert_eq!(app.cloud.count(CloudCall::DeleteNamespace), 0);
            return;
        }

        let mut app = TestApp::launch_with_passkey(PasskeyMode::FailOnAnyInteractiveCall);
        app.current().then_can(&[N::Verify, N::SyncUnsynced]);
        let PersistedCloudBackupState::Configured(configured) = persisted() else {
            panic!("the relaunch did not heal:\n{}", app.describe());
        };
        assert!(configured.heal_notice.is_some());
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
        assert!(matches!(authenticate(&passkey), Err(PasskeyError::UserCancelled)));

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
/// so a tap before the inventory loads turns Cloud Backup off instead of being dropped
#[test]
fn turn_off_is_admitted_before_the_detail_inventory_loads() {
    let test = concat!(module_path!(), "::turn_off_is_admitted_before_the_detail_inventory_loads");
    run_case(test, &[PhaseEnd::Exit], |_| {
        let mut app = TestApp::launch();
        let namespace = FakeAccount::new(1, 0).seed(&app);
        assert!(!app.manager().detail_inventory_is_complete(), "{}", app.describe());

        app.current().then_can(&[N::TurnOff]);
        app.dispatch(CloudBackupManagerAction::DisableCloudBackup)
            .then_can(&[N::Enable])
            .then_cannot(&[N::TurnOff]);
        assert_eq!(persisted(), PersistedCloudBackupState::Disabled, "{}", app.describe());
        assert!(!app.cloud.inner.has_namespace(&namespace));
        let outcome = app.last_outcome().expect("dispatch recorded an outcome");
        assert_eq!((outcome.intent, outcome.admission), (UserIntent::TurnOff, Admission::Accepted));
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
        app.dispatch(CloudBackupManagerAction::DisableCloudBackup).then_can(&[N::Enable]);

        let outcome = app.last_outcome().expect("the new manager admitted the intent");
        assert_eq!(outcome.admission, Admission::Accepted);
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
        app.dispatch(CloudBackupManagerAction::DisableCloudBackup).then_cannot(&[N::Verify]);
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
