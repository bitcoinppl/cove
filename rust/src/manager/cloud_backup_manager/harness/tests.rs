//! Proof that the harness drives the real manager: runtime, clock, relaunch, call-identity
//! fakes and real process kills
//!
//! Each test body runs in child processes; see [`super::run_case`]
#![cfg(test)]

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use cove_device::cloud_storage::CloudStorageError;

use super::process::child_root;
use super::{
    CallId, CloudCall, CloudFault, FakeAccount, HarnessCloud, Hold, NextAction as N, PasskeyMode,
    PhaseEnd, TestApp, open_durable_state, run_case, seed_legacy_stuck_disabling,
};
use crate::database::Database;
use crate::database::cloud_backup::{
    PersistedCloudBackupState, PersistedCloudBackupStatus, PersistedCloudBackupTransition,
};
use crate::manager::cloud_backup_manager::ops::test_support::MockCloudStorage;
use crate::manager::cloud_backup_manager::{CloudBackupKeychain, CloudBackupManagerAction};

fn persisted() -> PersistedCloudBackupState {
    Database::global().cloud_backup_state.get().expect("read cloud backup state")
}

fn keychain_namespace() -> Option<String> {
    CloudBackupKeychain::global().namespace_id().expect("read namespace")
}

/// Load the detail inventory the way the detail screen does; today's Turn Off waits for it
fn load_detail(app: &mut TestApp) {
    app.dispatch(CloudBackupManagerAction::RefreshDetail).then_cannot(&[N::Enable]);
    app.advance_clock(Duration::from_secs(60)).then_can(&[N::TurnOff]);
}

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

        // the relaunched app has not loaded its detail inventory, so today's Turn Off is dropped
        app.relaunch().then_can(&[N::Verify, N::SyncUnsynced]).then_cannot(&[
            N::TurnOff,
            N::Enable,
            N::KeepEnabled,
        ]);

        assert_eq!(persisted().status(), PersistedCloudBackupStatus::Enabled);
        assert_eq!(keychain_namespace().as_deref(), Some(namespace.as_str()));
        assert!(app.cloud.inner.has_namespace(&namespace));
        assert!(app.passkey.interactive_calls().is_empty());
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

/// Today's deadlock on the reporter's record. Phase B's load normalization flips this test:
/// the record becomes Configured with the heal notice, and Turn Off works without a passkey
#[test]
fn legacy_stuck_disabling_record_deadlocks_today() {
    let test = concat!(module_path!(), "::legacy_stuck_disabling_record_deadlocks_today");
    run_case(test, &[PhaseEnd::Exit, PhaseEnd::Exit], |phase| {
        let account = FakeAccount::lived_in();
        assert_eq!(account.record_count(), 35);

        if phase == 0 {
            let app = TestApp::launch();
            let namespace = account.seed(&app);
            seed_legacy_stuck_disabling(&namespace, app.clock.now_secs());
            return;
        }

        let mut app = TestApp::launch_with_passkey(PasskeyMode::FailOnAnyInteractiveCall);
        app.current().then_cannot(&[N::Enable, N::Verify, N::SyncUnsynced]);
        let stuck = persisted();
        let PersistedCloudBackupState::ExclusiveTransition(
            PersistedCloudBackupTransition::Disabling(disabling),
        ) = &stuck
        else {
            panic!("launch moved the legacy record:\n{}", app.describe());
        };
        assert_eq!(disabling.delete_started_at, None, "{}", app.describe());
        assert_eq!(app.cloud.count(CloudCall::DeleteNamespace), 0);

        // Turn Off again without the detail inventory: today's Rust drops it, nothing moves
        app.dispatch(CloudBackupManagerAction::DisableCloudBackup).then_cannot(&[
            N::Enable,
            N::Verify,
            N::SyncUnsynced,
        ]);
        assert_eq!(persisted(), stuck, "{}", app.describe());
        assert_eq!(app.cloud.count(CloudCall::DeleteNamespace), 0);
        assert!(app.passkey.interactive_calls().is_empty());
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
