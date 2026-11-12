//! Proof that the harness drives the real manager: runtime, clock, relaunch, call-identity
//! fakes and real process kills
//!
//! Each test body runs in child processes; see [`super::run_case`]
#![cfg(test)]

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Duration;

use act_zero::call;
use cove_cspp::backup_data::{PasskeyProviderHint, PasskeyRegistrationPlatform};
use cove_device::cloud_storage::{CloudAccessPolicy, CloudStorageError};
use cove_device::passkey::{PasskeyError, PasskeyFailureReason, PasskeyOperation};

use super::process::child_root;
use super::{
    CallId, CloudCall, CloudFault, FakeAccount, HarnessCloud, Hold, NextAction as N, PasskeyMode,
    PhaseEnd, TestApp, keychain_namespace, load_detail, open_durable_state, persisted, run_case,
    save_device_passkey, seed_corrupted_record, seed_legacy_stuck_disabling,
    seed_verifiable_master_key,
};
use crate::database::Database;
use crate::database::cloud_backup::{
    PersistedCloudBackupState, PersistedCloudBackupStatus, PersistedDeepVerificationReport,
    PersistedPendingVerificationCompletion,
};
use crate::manager::cloud_backup_manager::actors::write::Decision;
use crate::manager::cloud_backup_manager::admission::{
    Admission, PauseReason, Rejection, UserIntent,
};
use crate::manager::cloud_backup_manager::model::CloudBackupPasskeyRepairState;
use crate::manager::cloud_backup_manager::ops::test_support::MockCloudStorage;
use crate::manager::cloud_backup_manager::ops::{
    CLOUD_BACKUP_LOAD_FAILED_MESSAGE, DISABLE_EVIDENCE_DEADLINE,
};
use crate::manager::cloud_backup_manager::owner::lifecycle::test_support::bootstrap_with_failed_heal;
use crate::manager::cloud_backup_manager::owner::ui_test_legacy_record;
use crate::manager::cloud_backup_manager::screen::{
    CloudBackupActionAvailability, CloudBackupDisableConfirmation, CloudBackupDisableScreen,
    CloudBackupFinishingDisableStage, CloudBackupInventoryControl, CloudBackupOutcome,
    CloudBackupOutcomeTarget, CloudBackupProgressSection, CloudBackupRejection,
    CloudBackupScreenAction, CloudBackupScreenBody, CloudBackupSection, CloudBackupTurnOffFailure,
    CloudBackupVerificationControl,
};
use crate::manager::cloud_backup_manager::{
    CloudBackupEnableContext, CloudBackupInventoryIncompleteReason, CloudBackupManagerAction,
    CloudBackupNoticeKind, CloudBackupPasskeyHint, CloudBackupPasskeyState, CloudBackupRootPrompt,
    CloudBackupSettingsRowStatus, CloudBackupVerificationSource, CloudBackupVerificationState,
    DeepVerificationFailure, RecordId,
};
use crate::manager::cloud_backup_manager::{CloudBackupStatus, RustCloudBackupManager};
use crate::manager::connectivity_manager::{CONNECTIVITY_MANAGER, ConnectivityStatus};

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

/// Commits a Turn Off whose first namespace delete fails offline, then takes away Cove's
/// provider access, so the committed Turn Off can only finish with the user's consent
fn commit_turn_off_then_revoke_authorization(app: &mut TestApp) -> String {
    let namespace = FakeAccount::new(1, 0).seed(app);
    app.cloud.script(
        CallId::new(CloudCall::DeleteNamespace, 1),
        CloudFault::Fail(CloudStorageError::Offline("scripted".into())),
    );
    load_detail(app);

    app.dispatch(CloudBackupManagerAction::DisableCloudBackup).then_can(&[N::ConfirmDisable]);
    app.confirm_turn_off().then_can(&[N::ResumeDisable]).then_cannot(&[N::Enable]);
    assert_eq!(persisted().status(), PersistedCloudBackupStatus::Disabling);
    app.cloud.revoke_authorization();
    namespace
}

/// The access policy of every namespace delete after the first `skip`
fn delete_policies(app: &TestApp, skip: usize) -> Vec<Option<CloudAccessPolicy>> {
    (skip + 1..=app.cloud.count(CloudCall::DeleteNamespace))
        .map(|nth| app.cloud.policy_of(CallId::new(CloudCall::DeleteNamespace, nth)))
        .collect()
}

/// Each cloud call after the first `skip`, with the provider policy it asked for
fn call_policies(app: &TestApp, skip: usize) -> Vec<(CloudCall, Option<CloudAccessPolicy>)> {
    app.cloud
        .calls()
        .into_iter()
        .skip(skip)
        .map(|(id, _)| (id.kind, app.cloud.policy_of(id)))
        .collect()
}

/// Check Again while launch discovery's silent load runs joins that load instead of starting a
/// second one, and the load after it lists with the tap's authority, so it may ask for Google
/// Drive consent
#[test]
fn check_again_during_a_silent_load_lists_again_with_consent() {
    use crate::manager::cloud_backup_manager::admission::CoalescibleIntent;

    let test =
        concat!(module_path!(), "::check_again_during_a_silent_load_lists_again_with_consent");
    run_case(test, &[PhaseEnd::Exit], |_| {
        let mut app = TestApp::launch_with_passkey(PasskeyMode::FailOnAnyInteractiveCall);
        FakeAccount::new(1, 1).seed(&app);
        let before = app.cloud.calls().len();

        let listed = app.cloud.count(CloudCall::ListWalletFilesSnapshot);
        let hold = Hold::default();
        app.cloud.script_next(CloudCall::ListWalletFilesSnapshot, CloudFault::Hold(hold.clone()));
        app.event("launch discovery", |app| app.manager().start_background_inventory_discovery())
            .then_cannot(&[N::Enable]);
        app.advance_clock(Duration::from_secs(60)).then_cannot(&[N::Enable]);
        app.block_on("the silent listing to be held", hold.entered());

        // the held listing got in before Drive revoked its authorization, so only a listing that
        // may ask for consent gets in after it
        app.cloud.revoke_authorization();

        app.dispatch(CloudBackupManagerAction::RefreshDetail).then_cannot(&[N::Enable]);
        let outcome = app.last_outcome().expect("Check Again was decided");
        assert_eq!(outcome.admission, Admission::Coalesced(CoalescibleIntent::RefreshDetail));
        assert_eq!(
            app.cloud.count(CloudCall::ListWalletFilesSnapshot),
            listed + 1,
            "the tap started a second load beside the held one\n{}",
            app.describe()
        );

        app.event("the silent listing answers", |_| hold.release()).then_cannot(&[N::Enable]);
        app.advance_clock(Duration::from_secs(60)).then_cannot(&[N::Enable]);

        let listings = call_policies(&app, before)
            .into_iter()
            .filter(|(kind, _)| *kind == CloudCall::ListWalletFilesSnapshot)
            .map(|(_, policy)| policy)
            .collect::<Vec<_>>();
        assert_eq!(
            listings,
            vec![Some(CloudAccessPolicy::Silent), Some(CloudAccessPolicy::ConsentAllowed)],
            "the joined tap did not give the next listing its authority\n{}",
            app.describe()
        );
        assert_eq!(app.cloud.consent_sheets(), 1);
    });
}

/// Launch discovery loads the inventory with nobody tapping, so none of its Google Drive calls
/// may ask the user for consent: one that needs it shows Reconnect instead. Check Again is a tap,
/// so its listing may ask
#[test]
fn launch_discovery_never_asks_for_drive_consent_but_check_again_may() {
    let test = concat!(
        module_path!(),
        "::launch_discovery_never_asks_for_drive_consent_but_check_again_may"
    );
    run_case(test, &[PhaseEnd::Exit], |_| {
        let mut app = TestApp::launch_with_passkey(PasskeyMode::FailOnAnyInteractiveCall);
        FakeAccount::new(1, 1).seed(&app);
        app.cloud.revoke_authorization();
        let before = app.cloud.calls().len();

        app.event("launch discovery", |app| app.manager().start_background_inventory_discovery())
            .then_cannot(&[N::Enable]);
        app.advance_clock(Duration::from_secs(60)).then_cannot(&[N::Enable]);

        let launch = call_policies(&app, before);
        assert!(
            launch.iter().any(|(kind, _)| *kind == CloudCall::ListWalletFilesSnapshot),
            "launch discovery listed nothing\n{}",
            app.describe()
        );
        assert!(
            launch.iter().all(|(_, policy)| *policy == Some(CloudAccessPolicy::Silent)),
            "launch discovery asked for consent: {launch:?}"
        );
        assert_eq!(app.cloud.consent_sheets(), 0);

        let progress = configured_sections(&app).into_iter().find_map(|section| match section {
            CloudBackupSection::Progress(progress) => Some(progress),
            _ => None,
        });
        assert!(
            matches!(
                &progress,
                Some(CloudBackupProgressSection::InventoryIncomplete {
                    reason: CloudBackupInventoryIncompleteReason::AuthorizationRequired,
                    controls,
                }) if controls.contains(&CloudBackupInventoryControl::ReconnectDrive)
            ),
            "launch discovery that needs consent did not offer Reconnect: {progress:?}\n{}",
            app.describe()
        );

        let before = app.cloud.calls().len();
        app.dispatch(CloudBackupManagerAction::RefreshDetail).then_cannot(&[N::Enable]);
        app.advance_clock(Duration::from_secs(60)).then_cannot(&[N::Enable]);

        let tapped = call_policies(&app, before);
        assert_eq!(
            tapped.iter().find(|(kind, _)| *kind == CloudCall::ListWalletFilesSnapshot),
            Some(&(CloudCall::ListWalletFilesSnapshot, Some(CloudAccessPolicy::ConsentAllowed))),
            "{tapped:?}"
        );
        assert_eq!(app.cloud.consent_sheets(), 1);
    });
}

/// Reconnect Google Drive on a configured backup authorizes the provider and nothing else: its
/// tap lists the inventory with a client that may ask for consent and never asks for the
/// passkey, so checking the backup stays the separate Verify tap
#[test]
fn reconnect_drive_on_a_configured_backup_asks_for_consent_and_never_the_passkey() {
    let test = concat!(
        module_path!(),
        "::reconnect_drive_on_a_configured_backup_asks_for_consent_and_never_the_passkey"
    );
    run_case(test, &[PhaseEnd::Exit], |_| {
        let mut app = TestApp::launch();
        let namespace = FakeAccount::new(1, 0).seed(&app);
        seed_verifiable_master_key(&app, &namespace);
        app.cloud.revoke_authorization();

        app.event("launch discovery", |app| app.manager().start_background_inventory_discovery())
            .then_can(&[N::ReconnectDrive, N::Verify]);
        app.advance_clock(Duration::from_secs(60)).then_can(&[N::ReconnectDrive, N::Verify]);
        assert_eq!(
            inventory_progress(&app),
            Some(CloudBackupProgressSection::InventoryIncomplete {
                reason: CloudBackupInventoryIncompleteReason::AuthorizationRequired,
                controls: vec![
                    CloudBackupInventoryControl::ReconnectDrive,
                    CloudBackupInventoryControl::CheckAgain,
                ],
            }),
            "{}",
            app.describe()
        );

        // the master key wrapper is readable, so a verification here would ask for the passkey
        let before = app.cloud.calls().len();
        app.dispatch(CloudBackupManagerAction::ReconnectDrive).then_can(&[N::Verify]);
        app.advance_clock(Duration::from_secs(60)).then_can(&[N::Verify]);

        assert_eq!(app.passkey.interactive_calls(), Vec::new(), "{}", app.describe());

        let reconnected = call_policies(&app, before);
        assert_eq!(
            reconnected.iter().find(|(kind, _)| *kind == CloudCall::ListWalletFilesSnapshot),
            Some(&(CloudCall::ListWalletFilesSnapshot, Some(CloudAccessPolicy::ConsentAllowed))),
            "{reconnected:?}"
        );
        assert_eq!(app.cloud.consent_sheets(), 1);
        assert!(
            !matches!(
                inventory_progress(&app),
                Some(CloudBackupProgressSection::InventoryIncomplete { .. })
            ),
            "the reconnect left the inventory incomplete\n{}",
            app.describe()
        );
        assert!(
            !matches!(
                verification_state(&app),
                Some(
                    CloudBackupVerificationState::Running
                        | CloudBackupVerificationState::Verified { .. }
                )
            ),
            "the reconnect verified the backup\n{}",
            app.describe()
        );

        app.dispatch(verify()).then_can(&[N::Verify, N::TurnOff]);
        assert!(
            matches!(verification_state(&app), Some(CloudBackupVerificationState::Verified { .. })),
            "{}",
            app.describe()
        );
        assert!(!app.passkey.interactive_calls().is_empty());
    });
}

/// The pending upload verifier resumes at launch with nobody tapping, so the listing it finishes
/// with may not ask for Google Drive consent either
#[test]
fn a_pending_verification_resumed_at_launch_never_asks_for_drive_consent() {
    let test = concat!(
        module_path!(),
        "::a_pending_verification_resumed_at_launch_never_asks_for_drive_consent"
    );
    run_case(test, &[PhaseEnd::Exit], |_| {
        let mut app = TestApp::launch_with_passkey(PasskeyMode::FailOnAnyInteractiveCall);
        let namespace_id = FakeAccount::new(1, 0).seed(&app);

        // a verification that finished uploading and waits only for its closing listing
        let table = &Database::global().cloud_backup_state;
        let mut state = table.get().expect("read cloud backup state");
        let completion = PersistedPendingVerificationCompletion {
            report: PersistedDeepVerificationReport {
                master_key_wrapper_repaired: true,
                local_master_key_repaired: false,
                credential_recovered: false,
                wallets_verified: 0,
                wallets_failed: 0,
                wallets_unsupported: 0,
                wallet_issues: None,
            },
            namespace_id,
            uploads: Vec::new(),
            created_at: Some(cove_util::time::unix_timestamp_secs_or_zero()),
        };
        assert!(state.replace_pending_verification_completion(completion));
        table.set(&state).expect("persist the pending verification");
        let before = app.cloud.calls().len();

        // the platforms resume the verifier at startup, after the manager is built
        app.relaunch().then_cannot(&[N::Enable]);
        app.event("startup resumes the verifier", |app| {
            app.manager().resume_pending_cloud_upload_verification();
        })
        .then_cannot(&[N::Enable]);
        app.advance_clock(Duration::from_secs(60)).then_cannot(&[N::Enable]);

        let PersistedCloudBackupState::Configured(configured) = persisted() else {
            panic!("the relaunch lost the configuration:\n{}", app.describe());
        };
        assert!(
            configured.pending_verification_completion.is_none(),
            "the verifier never finished the pending verification\n{}",
            app.describe()
        );

        let resumed = call_policies(&app, before);
        assert!(
            resumed.iter().any(|(kind, _)| *kind == CloudCall::ListWalletFiles),
            "the resumed verification listed nothing\n{}",
            app.describe()
        );
        assert!(
            resumed.iter().all(|(_, policy)| *policy == Some(CloudAccessPolicy::Silent)),
            "the resumed verification asked for consent: {resumed:?}"
        );
    });
}

/// Where the published screen's committed Turn Off stands
fn finishing_stage(app: &TestApp) -> Option<CloudBackupFinishingDisableStage> {
    match app.manager().published_screen().body {
        CloudBackupScreenBody::FinishingDisable(finishing) => Some(finishing.stage),
        _ => None,
    }
}

/// A committed Turn Off resumes by itself at launch and on reconnect without asking the user for
/// provider consent; only Try Again, the user's tap, may
#[test]
fn automatic_turn_off_resume_never_asks_for_provider_consent() {
    let test =
        concat!(module_path!(), "::automatic_turn_off_resume_never_asks_for_provider_consent");
    run_case(test, &[PhaseEnd::Exit], |_| {
        let mut app = TestApp::launch_with_passkey(PasskeyMode::FailOnAnyInteractiveCall);
        let namespace = commit_turn_off_then_revoke_authorization(&mut app);

        app.relaunch().then_can(&[N::ResumeDisable]).then_cannot(&[N::Enable]);
        let launched = app.cloud.count(CloudCall::DeleteNamespace);
        assert!(launched > 1, "the launch resumed the delete\n{}", app.describe());

        app.advance_clock(Duration::from_secs(60)).then_cannot(&[N::Enable]);
        app.event("reconnect", |app| {
            app.manager().handle_connectivity_change(ConnectivityStatus::Connected);
        })
        .then_can(&[N::ResumeDisable])
        .then_cannot(&[N::Enable]);
        assert!(
            app.cloud.count(CloudCall::DeleteNamespace) > launched,
            "the reconnect resumed the delete\n{}",
            app.describe()
        );

        let automatic = delete_policies(&app, 1);
        assert!(
            automatic.iter().all(|policy| *policy == Some(CloudAccessPolicy::Silent)),
            "automatic resumes deleted with {automatic:?}"
        );
        assert_eq!(app.cloud.consent_sheets(), 0);
        assert!(app.cloud.inner.has_namespace(&namespace));
        assert_eq!(
            finishing_stage(&app),
            Some(CloudBackupFinishingDisableStage::Failed {
                reason: CloudBackupTurnOffFailure::AuthorizationRequired,
            })
        );

        let resumed = app.cloud.count(CloudCall::DeleteNamespace);
        app.dispatch(CloudBackupManagerAction::ResumeDisable)
            .then_can(&[N::Enable])
            .then_cannot(&[N::TurnOff]);
        assert_eq!(delete_policies(&app, resumed), vec![Some(CloudAccessPolicy::ConsentAllowed)]);
        assert_eq!(app.cloud.consent_sheets(), 1);
        assert!(!app.cloud.inner.has_namespace(&namespace));
        assert_eq!(persisted(), PersistedCloudBackupState::Disabled);
    });
}

/// A committed Turn Off that stopped for the provider's consent offers Reconnect Google Drive,
/// which finishes that same Turn Off with the user's consent instead of verifying a backup
#[test]
fn reconnect_drive_finishes_the_committed_turn_off_after_an_authorization_failure() {
    let test = concat!(
        module_path!(),
        "::reconnect_drive_finishes_the_committed_turn_off_after_an_authorization_failure"
    );
    run_case(test, &[PhaseEnd::Exit], |_| {
        let mut app = TestApp::launch_with_passkey(PasskeyMode::FailOnAnyInteractiveCall);
        let namespace = commit_turn_off_then_revoke_authorization(&mut app);

        // an offline failure is no reason to reconnect
        app.current().then_can(&[N::ResumeDisable]).then_cannot(&[N::ReconnectDrive]);

        app.relaunch().then_can(&[N::ResumeDisable, N::ReconnectDrive]).then_cannot(&[
            N::StopUsingOnThisDevice,
            N::Verify,
            N::TurnOff,
            N::Enable,
        ]);
        assert_eq!(
            app.admission_of(UserIntent::SwitchDriveAccount),
            Admission::Rejected(Rejection::TurnOffCommitted)
        );

        let resumed = app.cloud.count(CloudCall::DeleteNamespace);
        app.dispatch(CloudBackupManagerAction::ReconnectDrive)
            .then_can(&[N::Enable])
            .then_cannot(&[N::TurnOff, N::ReconnectDrive]);
        let deletes = app.cloud.calls();
        let reconnected = deletes
            .iter()
            .filter(|(id, _)| id.kind == CloudCall::DeleteNamespace && id.nth > resumed)
            .collect::<Vec<_>>();
        assert_eq!(
            reconnected,
            vec![&(CallId::new(CloudCall::DeleteNamespace, resumed + 1), Some(namespace))]
        );
        assert_eq!(delete_policies(&app, resumed), vec![Some(CloudAccessPolicy::ConsentAllowed)]);
        assert_eq!(app.cloud.consent_sheets(), 1);
        assert_eq!(persisted(), PersistedCloudBackupState::Disabled);
        assert!(app.passkey.interactive_calls().is_empty());
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
        let consent = app.row_delete(elsewhere).expect("the row's Delete");
        let delete = UserIntent::DeleteCloudWallet(consent.clone());
        assert_eq!(app.admission_of(delete), Admission::Accepted, "{}", app.describe());
        app.dispatch(CloudBackupManagerAction::DeleteCloudWallet(consent)).then_can(&[N::TurnOff]);
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

/// A row's Delete runs only on the evidence the user confirmed it on: a confirmation sent after
/// a wallet changed on this device is refused as stale next to the row and deletes nothing, and
/// the Delete the republished row offers deletes the record
///
/// The dispatch half of the rule; `delete_consent`'s tests hold the listing, category and
/// configuration halves
#[test]
fn a_row_delete_confirmed_before_a_wallet_change_is_refused_as_stale() {
    let test = concat!(
        module_path!(),
        "::a_row_delete_confirmed_before_a_wallet_change_is_refused_as_stale"
    );
    run_case(test, &[PhaseEnd::Exit], |_| {
        let mut app = TestApp::launch_with_passkey(PasskeyMode::FailOnAnyInteractiveCall);
        let account = FakeAccount::new(1, 1).seed_records(&app);
        let [elsewhere] = account.not_on_device.as_slice() else {
            // nothing read from the account: a panic message reaches the test log
            panic!("expected exactly one record not on this device");
        };
        load_detail(&mut app);

        // the user opens the confirmation, then a wallet changes on this device
        let confirmed = app.row_delete(elsewhere).expect("the row's Delete");
        app.event("a local wallet change", |app| app.manager().handle_wallet_set_change())
            .then_can(&[N::TurnOff]);
        app.dispatch(CloudBackupManagerAction::DeleteCloudWallet(confirmed.clone()))
            .then_can(&[N::TurnOff]);

        let Some(outcome) = app.manager().published_screen().last_outcome else {
            panic!("no rejection after {:?}: {}", app.last_outcome(), app.describe());
        };
        let CloudBackupOutcomeTarget::Row { target } = &outcome.target else {
            panic!("a row delete's rejection targets its row: {outcome:?}");
        };
        assert_eq!(target.record_id, RecordId::from(elsewhere.clone()));
        assert_eq!(
            outcome.outcome,
            CloudBackupOutcome::Rejected { reason: CloudBackupRejection::ReviewStale }
        );
        assert!(app.cloud.inner.has_wallet_backup(&account.namespace, elsewhere));
        assert_eq!(app.cloud.count(CloudCall::DeleteWallet), 0);

        let current = app.row_delete(elsewhere).expect("the republished row's Delete");
        assert_ne!(current, confirmed);
        app.dispatch(CloudBackupManagerAction::DeleteCloudWallet(current)).then_can(&[N::TurnOff]);
        assert!(!app.cloud.inner.has_wallet_backup(&account.namespace, elsewhere));
    });
}

/// The backups not on this device load after the detail, so a failed fetch of them leaves the
/// detail complete. The screen shows that failure with Check Again instead of drawing those
/// backups as absent, and Check Again fetches them again
#[test]
fn a_failed_fetch_of_backups_not_on_this_device_shows_check_again() {
    let test =
        concat!(module_path!(), "::a_failed_fetch_of_backups_not_on_this_device_shows_check_again");
    run_case(test, &[PhaseEnd::Exit], |_| {
        let mut app = TestApp::launch_with_passkey(PasskeyMode::FailOnAnyInteractiveCall);
        let account = FakeAccount::new(1, 1).seed_records(&app);
        let [elsewhere] = account.not_on_device.as_slice() else {
            // nothing read from the account: a panic message reaches the test log
            panic!("expected exactly one record not on this device");
        };

        // the fetch is tried again when the other supplemental loads finish, so the provider
        // keeps failing it, as it would while its sync is pending
        let listed = app.cloud.count(CloudCall::ListWalletFiles);
        for nth in listed + 1..=listed + 4 {
            app.cloud.script(
                CallId::new(CloudCall::ListWalletFiles, nth),
                CloudFault::Fail(CloudStorageError::SyncPending("scripted".into())),
            );
        }
        load_detail(&mut app);

        let failed = configured_sections(&app);
        assert!(
            failed.iter().any(|section| matches!(
                section,
                CloudBackupSection::Progress(CloudBackupProgressSection::InventoryIncomplete {
                    reason: CloudBackupInventoryIncompleteReason::ProviderSyncPending,
                    controls,
                }) if controls == &[CloudBackupInventoryControl::CheckAgain]
            )),
            "{}",
            app.describe()
        );
        assert!(app.row_delete(elsewhere).is_none(), "{}", app.describe());

        // the provider recovers, and the user taps Check Again
        app.cloud.clear_scripts();
        let listings = app.cloud.count(CloudCall::ListWalletFiles);
        load_detail(&mut app);

        assert!(app.cloud.count(CloudCall::ListWalletFiles) > listings, "{}", app.describe());
        assert!(
            !configured_sections(&app)
                .iter()
                .any(|section| matches!(section, CloudBackupSection::Progress(_))),
            "{}",
            app.describe()
        );
        assert!(app.row_delete(elsewhere).is_some(), "{}", app.describe());
    });
}

/// A wallet listing that never answers ends at Turn Off's evidence deadline as unknown evidence,
/// so the review asks twice, and a confirmed Turn Off commits past a recheck listing that never
/// answers either
#[test]
fn turn_off_reaches_a_review_and_a_commit_past_a_wallet_listing_that_never_answers() {
    let test = concat!(
        module_path!(),
        "::turn_off_reaches_a_review_and_a_commit_past_a_wallet_listing_that_never_answers"
    );
    run_case(test, &[PhaseEnd::Exit], |_| {
        let mut app = TestApp::launch_with_passkey(PasskeyMode::FailOnAnyInteractiveCall);
        let namespace = FakeAccount::new(1, 0).seed(&app);
        load_detail(&mut app);

        let review_listing = Hold::default();
        app.cloud.script_next(CloudCall::ListWalletFiles, CloudFault::Hold(review_listing.clone()));
        app.dispatch(CloudBackupManagerAction::DisableCloudBackup)
            .then_cannot(&[N::ConfirmDisable, N::Enable]);
        app.block_on("the review's listing to be held", review_listing.entered());
        assert!(
            app.turn_off_review().is_none(),
            "a review before its evidence: {}",
            app.describe()
        );

        app.advance_clock(DISABLE_EVIDENCE_DEADLINE)
            .then_can(&[N::ConfirmDisable, N::CancelDisable])
            .then_cannot(&[N::Enable]);
        let review = app.turn_off_review().expect("the Turn Off review past its deadline");
        assert_eq!(review.impact.confirmation, CloudBackupDisableConfirmation::Two, "{review:?}");
        assert!(app.cloud.inner.has_namespace(&namespace), "a review deleted the backup");

        let recheck_listing = Hold::default();
        app.cloud
            .script_next(CloudCall::ListWalletFiles, CloudFault::Hold(recheck_listing.clone()));
        app.confirm_turn_off().then_cannot(&[N::Enable]);
        app.block_on("the recheck's listing to be held", recheck_listing.entered());

        app.advance_clock(DISABLE_EVIDENCE_DEADLINE)
            .then_can(&[N::Enable])
            .then_cannot(&[N::TurnOff]);
        assert_eq!(persisted(), PersistedCloudBackupState::Disabled, "{}", app.describe());
        assert!(!app.cloud.inner.has_namespace(&namespace));
        assert!(app.passkey.interactive_calls().is_empty());
    });
}

/// The Offline dialog's Try Again is Check Again: a review whose classification refresh waits out
/// the detail rate limit is live work that quiescence gives the clock, so it reaches a review and
/// a Turn Off that finishes, instead of quiescing while it prepares
///
/// Minimized from model seed 777799, whose escape tapped Turn Off on the Offline dialog and then
/// joined the preparing review on every tap without ever moving the clock
#[test]
fn quiescence_moves_a_check_again_review_past_its_rate_limited_refresh() {
    let test = concat!(
        module_path!(),
        "::quiescence_moves_a_check_again_review_past_its_rate_limited_refresh"
    );
    run_case(test, &[PhaseEnd::Exit], |_| {
        let mut app = TestApp::launch_with_passkey(PasskeyMode::FailOnAnyInteractiveCall);
        let namespace = FakeAccount::new(1, 0).seed(&app);
        load_detail(&mut app);

        app.event("the connection drops", |_| {
            CONNECTIVITY_MANAGER.set_status_without_broadcast(ConnectivityStatus::Disconnected);
        })
        .then_can(&[N::TurnOff]);
        app.dispatch(CloudBackupManagerAction::DisableCloudBackup)
            .then_can(&[N::TurnOff, N::CancelDisable])
            .then_cannot(&[N::ConfirmDisable]);
        assert_eq!(disable_screen(&app), CloudBackupDisableScreen::Offline, "{}", app.describe());

        app.event("the connection returns", |app| {
            CONNECTIVITY_MANAGER.set_status_without_broadcast(ConnectivityStatus::Connected);
            app.manager().handle_connectivity_change(ConnectivityStatus::Connected);
        })
        .then_can(&[N::TurnOff]);

        // a refresh started at this instant holds the next one behind the detail rate limit, so
        // Check Again's classification waits on the clock
        app.dispatch(CloudBackupManagerAction::RefreshDetail).then_can(&[N::TurnOff]);
        app.dispatch(CloudBackupManagerAction::DisableCloudBackup)
            .then_can(&[N::CancelDisable])
            .then_cannot(&[N::ConfirmDisable, N::Enable]);
        assert_eq!(disable_screen(&app), CloudBackupDisableScreen::Preparing, "{}", app.describe());
        assert!(app.holds_live_work(), "a preparing review is live work\n{}", app.describe());

        app.quiesce().then_can(&[N::ConfirmDisable, N::CancelDisable]).then_cannot(&[N::Enable]);
        assert!(app.turn_off_review().is_some(), "no review after quiescence\n{}", app.describe());

        app.confirm_turn_off().then_can(&[N::Enable]).then_cannot(&[N::TurnOff]);
        assert_eq!(persisted(), PersistedCloudBackupState::Disabled, "{}", app.describe());
        assert!(!app.cloud.inner.has_namespace(&namespace));
        assert!(app.passkey.interactive_calls().is_empty());
    });
}

/// The Turn Off dialog the published configured body shows
fn disable_screen(app: &TestApp) -> CloudBackupDisableScreen {
    match app.manager().published_screen().body {
        CloudBackupScreenBody::Configured(configured) => configured.disable,
        other => panic!("expected the configured body, got {other:?}"),
    }
}

/// The other-backups count is information for the dialog, so a count that never answers holds
/// neither the review nor the recheck, with no clock advance, and the review leaves it out
#[test]
fn turn_off_reaches_a_review_and_a_commit_past_a_backup_count_that_never_answers() {
    let test = concat!(
        module_path!(),
        "::turn_off_reaches_a_review_and_a_commit_past_a_backup_count_that_never_answers"
    );
    run_case(test, &[PhaseEnd::Exit], |_| {
        let mut app = TestApp::launch_with_passkey(PasskeyMode::FailOnAnyInteractiveCall);
        let namespace = FakeAccount::new(1, 0).seed(&app);
        load_detail(&mut app);

        let review_count = Hold::default();
        app.cloud.script_next(CloudCall::ListNamespaces, CloudFault::Hold(review_count.clone()));
        app.dispatch(CloudBackupManagerAction::DisableCloudBackup).then_cannot(&[N::Enable]);
        app.block_on("the review's count to have started", review_count.entered());

        // the evidence that decides lands while the count is held, and the stage drops the count
        app.event("the review drops its count", abandon_held_calls)
            .then_can(&[N::ConfirmDisable, N::CancelDisable])
            .then_cannot(&[N::Enable]);
        let review = app.turn_off_review().expect("the Turn Off review without its count");
        assert_eq!(review.impact.confirmation, CloudBackupDisableConfirmation::One, "{review:?}");
        assert_eq!(review.impact.other_backups, 0, "{review:?}");

        let recheck_count = Hold::default();
        app.cloud.script_next(CloudCall::ListNamespaces, CloudFault::Hold(recheck_count.clone()));
        app.confirm_turn_off().then_cannot(&[N::TurnOff]);
        app.block_on("the recheck's count to have started", recheck_count.entered());
        app.event("the recheck drops its count", abandon_held_calls)
            .then_can(&[N::Enable])
            .then_cannot(&[N::TurnOff]);
        assert_eq!(persisted(), PersistedCloudBackupState::Disabled, "{}", app.describe());
        assert!(!app.cloud.inner.has_namespace(&namespace));
    });
}

/// Waits until every held call's caller has dropped it, without releasing any
fn abandon_held_calls(app: &TestApp) {
    app.block_on("the held calls to be abandoned", super::fakes::until_no_call_is_held());
}

/// Restore Them First on a review naming one wallet not on this device ends the review and
/// restores that wallet in the same tap, after which Turn Off asks once
#[test]
fn restore_first_restores_the_one_wallet_the_review_names() {
    let test = concat!(module_path!(), "::restore_first_restores_the_one_wallet_the_review_names");
    run_case(test, &[PhaseEnd::Exit], |_| {
        let mut app = TestApp::launch_with_passkey(PasskeyMode::FailOnAnyInteractiveCall);
        FakeAccount::new(1, 1).seed_records(&app);
        load_detail(&mut app);

        app.dispatch(CloudBackupManagerAction::DisableCloudBackup)
            .then_can(&[N::ConfirmDisable, N::RestoreFirst]);
        let review = app.turn_off_review().expect("the Turn Off review");
        assert_eq!(review.impact.not_on_device.names.len(), 1, "{review:?}");
        assert_eq!(control(&app, CloudBackupScreenAction::VerifyFirst), None);

        app.dispatch(CloudBackupManagerAction::RestoreBeforeTurnOff {
            review_id: review.review_id,
        })
        .then_can(&[N::TurnOff])
        .then_cannot(&[N::ConfirmDisable, N::RestoreFirst]);
        assert!(app.turn_off_review().is_none(), "{}", app.describe());
        assert_eq!(app.manager().published_screen().last_outcome, None, "{}", app.describe());

        // the wallet is on this device now, so nothing is at risk and the review asks once
        app.dispatch(CloudBackupManagerAction::DisableCloudBackup)
            .then_can(&[N::ConfirmDisable])
            .then_cannot(&[N::RestoreFirst]);
        let review = app.turn_off_review().expect("the second Turn Off review");
        assert_eq!(review.impact.confirmation, CloudBackupDisableConfirmation::One, "{review:?}");
        assert_eq!(app.cloud.count(CloudCall::DeleteNamespace), 0);
        assert!(app.passkey.interactive_calls().is_empty());
    });
}

/// Restore Them First on a review naming several wallets not on this device starts Restore All
/// and ends the review; one for a review no longer on screen is stale, starts nothing and leaves
/// the review open with the reason next to the button
#[test]
fn restore_first_restores_every_wallet_the_review_names_through_restore_all() {
    let test = concat!(
        module_path!(),
        "::restore_first_restores_every_wallet_the_review_names_through_restore_all"
    );
    run_case(test, &[PhaseEnd::Exit], |_| {
        let mut app = TestApp::launch_with_passkey(PasskeyMode::FailOnAnyInteractiveCall);
        FakeAccount::new(1, 2).seed_records(&app);
        load_detail(&mut app);

        app.dispatch(CloudBackupManagerAction::DisableCloudBackup)
            .then_can(&[N::ConfirmDisable, N::RestoreFirst]);
        let review = app.turn_off_review().expect("the Turn Off review");
        assert_eq!(review.impact.not_on_device.names.len(), 2, "{review:?}");

        let stale = review.review_id + 1;
        app.dispatch(CloudBackupManagerAction::RestoreBeforeTurnOff { review_id: stale })
            .then_can(&[N::ConfirmDisable, N::RestoreFirst]);
        assert_eq!(app.turn_off_review().map(|open| open.review_id), Some(review.review_id));
        let outcome = app.manager().published_screen().last_outcome.expect("the stale rejection");
        assert_eq!(
            outcome.target,
            CloudBackupOutcomeTarget::Action(CloudBackupScreenAction::RestoreFirst)
        );
        assert_eq!(
            outcome.outcome,
            CloudBackupOutcome::Rejected { reason: CloudBackupRejection::ReviewStale }
        );

        app.dispatch(CloudBackupManagerAction::RestoreBeforeTurnOff {
            review_id: review.review_id,
        })
        .then_can(&[N::TurnOff])
        .then_cannot(&[N::ConfirmDisable, N::RestoreFirst]);
        assert!(app.turn_off_review().is_none(), "{}", app.describe());
        assert_eq!(app.manager().published_screen().last_outcome, None, "{}", app.describe());

        // both wallets are on this device now, so the review asks once
        app.dispatch(CloudBackupManagerAction::DisableCloudBackup)
            .then_can(&[N::ConfirmDisable])
            .then_cannot(&[N::RestoreFirst]);
        let review = app.turn_off_review().expect("the second Turn Off review");
        assert_eq!(review.impact.confirmation, CloudBackupDisableConfirmation::One, "{review:?}");
        assert_eq!(app.cloud.count(CloudCall::DeleteNamespace), 0);
        assert!(app.passkey.interactive_calls().is_empty());
    });
}

/// Verify First on a review with backups Cove couldn't check ends the review and runs the
/// verification; one the user doesn't finish changes nothing, and Turn Off still asks twice
#[test]
fn verify_first_ends_the_review_and_runs_the_verification() {
    let test = concat!(module_path!(), "::verify_first_ends_the_review_and_runs_the_verification");
    run_case(test, &[PhaseEnd::Exit], |_| {
        let mut app = TestApp::launch();
        let namespace = FakeAccount::new(1, 1).seed_records(&app).namespace;
        seed_verifiable_master_key(&app, &namespace);

        // with no inventory loaded, the record not on this device is one Cove couldn't check
        app.dispatch(CloudBackupManagerAction::DisableCloudBackup)
            .then_can(&[N::ConfirmDisable, N::VerifyFirst])
            .then_cannot(&[N::RestoreFirst]);
        let review = app.turn_off_review().expect("the Turn Off review");
        assert!(review.impact.unchecked > 0, "{review:?}");

        app.passkey.set_mode(PasskeyMode::NotCompleted);
        app.dispatch(CloudBackupManagerAction::VerifyBeforeTurnOff { review_id: review.review_id })
            .then_can(&[N::TurnOff, N::Verify])
            .then_cannot(&[N::ConfirmDisable, N::VerifyFirst]);
        assert!(app.turn_off_review().is_none(), "{}", app.describe());
        assert_eq!(verification_state(&app), Some(CloudBackupVerificationState::NotCompleted));

        app.dispatch(CloudBackupManagerAction::DisableCloudBackup).then_can(&[N::ConfirmDisable]);
        let review = app.turn_off_review().expect("the second Turn Off review");
        assert_eq!(review.impact.confirmation, CloudBackupDisableConfirmation::Two, "{review:?}");
        assert_eq!(app.cloud.count(CloudCall::DeleteNamespace), 0);
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
        use crate::manager::cloud_backup_manager::wallets::{PermitIntent, authority_test_support};

        let passkey = HarnessPasskey::new(MockPasskeyProviderImpl::default());
        let authenticate = |passkey: &HarnessPasskey| {
            passkey.authenticate_with_prf("rp".into(), vec![1], vec![2], vec![3])
        };

        // the fake presents only for the request holding the gate with a live permit, as Rust's
        // retrier opens one before every call
        let (grant, authority) = authority_test_support::revocable_permit(PermitIntent::Verify);
        let request = authority.permit().expect("interactive").begin_request().expect("free gate");

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

        // the operation ends while the request waits for a presenter, so it presents nothing
        let hold = Hold::default();
        passkey.set_mode(PasskeyMode::AwaitingPresenter(hold.clone()));
        let waiting = std::thread::spawn({
            let passkey = passkey.clone();
            move || authenticate(&passkey)
        });
        futures::executor::block_on(hold.entered());
        drop(grant);
        hold.release();
        let answer = waiting.join().expect("waiting passkey thread");
        assert!(matches!(answer, Err(PasskeyError::NotCompleted)), "{answer:?}");
        assert_eq!(passkey.withdrawn_calls(), vec![CallId::new(PasskeyCall::Authenticate, 5)]);
        drop(request);
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

        // a relaunch whose launch listings fail, the inventory's and the other backups scan's,
        // so only a later listing loads the inventory
        app.relaunch().then_cannot(&[N::Enable]);
        let listed = app.cloud.count(CloudCall::ListWalletFilesSnapshot);
        for nth in [listed + 1, listed + 2] {
            app.cloud.script(
                CallId::new(CloudCall::ListWalletFilesSnapshot, nth),
                CloudFault::Fail(CloudStorageError::NotAvailable("the provider is busy".into())),
            );
        }

        app.event("launch discovery", |app| app.manager().start_background_inventory_discovery())
            .then_cannot(&[N::Enable]);
        app.advance_clock(Duration::from_secs(60)).then_cannot(&[N::Enable]);
        assert!(!app.manager().listing_is_provider_confirmed(), "{}", app.describe());

        // the provider reports a cloud change, which nobody tapped for, so it lists again
        // without asking for consent
        let before_change = app.cloud.calls().len();
        app.event("cloud change", |app| {
            send!(app.manager().supervisor.provider_inventory_did_change());
        })
        .then_cannot(&[N::Enable]);
        app.advance_clock(Duration::from_secs(60)).then_cannot(&[N::Enable]);

        let changed = call_policies(&app, before_change);
        assert!(
            changed.iter().any(|(kind, _)| *kind == CloudCall::ListWalletFilesSnapshot),
            "the cloud change listed nothing\n{}",
            app.describe()
        );
        assert!(
            changed.iter().all(|(_, policy)| *policy == Some(CloudAccessPolicy::Silent)),
            "the cloud change asked for consent: {changed:?}"
        );
        assert!(
            app.manager().listing_is_provider_confirmed(),
            "the cloud change did not load the inventory\n{}",
            app.describe()
        );
        assert_eq!(
            app.manager().inventory.read().cloud_only().loaded_rows().map(<[_]>::len),
            Some(2),
            "{}",
            app.describe()
        );

        // a relaunch with no local master key
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

/// Where the passkey the published screen's passkey card asks for was created, if it says
fn passkey_hint(app: &TestApp) -> Option<CloudBackupPasskeyHint> {
    match app.manager().published_screen().body {
        CloudBackupScreenBody::Configured(configured) => configured.passkey_hint,
        _ => None,
    }
}

/// The published screen's progress section, which draws the inventory loading or incomplete
fn inventory_progress(app: &TestApp) -> Option<CloudBackupProgressSection> {
    configured_sections(app).into_iter().find_map(|section| match section {
        CloudBackupSection::Progress(progress) => Some(progress),
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
        let namespace = FakeAccount::new(1, 0).seed(&app);
        seed_verifiable_master_key(&app, &namespace);
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

/// The missing passkey's card says where that passkey was created, and never names a hint
/// stored for another credential
#[test]
fn the_missing_passkey_card_says_where_its_passkey_was_created() {
    let test =
        concat!(module_path!(), "::the_missing_passkey_card_says_where_its_passkey_was_created");
    run_case(test, &[PhaseEnd::Exit], |_| {
        let mut app = TestApp::launch();
        let namespace = FakeAccount::new(1, 0).seed(&app);
        seed_verifiable_master_key(&app, &namespace);
        let one_password = PasskeyProviderHint {
            aaguid: "bada5566-a7aa-401f-bd96-45619a55120d".into(),
            registered_platform: PasskeyRegistrationPlatform::Ios,
            registered_at: 1_700_000_000,
            name_suffix: "09IX".into(),
        };
        save_device_passkey(&[1, 2, 3], Some(&one_password));

        script_missing_passkey(
            &app,
            PasskeyError::RequestFailed {
                operation: PasskeyOperation::Registration,
                reason: PasskeyFailureReason::PlatformAuthorizationFailedAfterPresentation,
            },
        );

        app.dispatch(verify()).then_can(&[N::Verify]);
        assert!(matches!(passkey_card(&app), Some(CloudBackupPasskeyState::NeedsRepair { .. })));
        assert_eq!(
            passkey_hint(&app),
            Some(CloudBackupPasskeyHint {
                provider_name: Some("1Password".into()),
                name_suffix: "09IX".into(),
                registered_at: 1_700_000_000,
            })
        );

        // a passkey saved without a hint leaves the earlier passkey's hint stored, naming that one
        save_device_passkey(&[4, 5, 6], None);
        app.dispatch(verify()).then_can(&[N::Verify]);
        assert!(matches!(passkey_card(&app), Some(CloudBackupPasskeyState::NeedsRepair { .. })));
        assert_eq!(passkey_hint(&app), None);
    });
}

/// Dismissing the passkey sheet, while signing in or while making a replacement, only didn't
/// complete: the passkey is never marked missing and only Verify is offered again
#[test]
fn a_cancelled_verification_never_offers_a_passkey_repair() {
    let test = concat!(module_path!(), "::a_cancelled_verification_never_offers_a_passkey_repair");
    run_case(test, &[PhaseEnd::Exit], |_| {
        let mut app = TestApp::launch();
        let namespace = FakeAccount::new(1, 0).seed(&app);
        seed_verifiable_master_key(&app, &namespace);
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
        assert_eq!(app.manager().published_screen().root_prompt, CloudBackupRootPrompt::None);
        assert_eq!(persisted().status(), status);
    });
}

/// A verification that fails reading the cloud, or on a passkey that answered moments before,
/// never showed the passkey missing, so it offers Verify again and no repair
#[test]
fn a_verification_that_fails_off_the_passkey_offers_no_repair() {
    let test =
        concat!(module_path!(), "::a_verification_that_fails_off_the_passkey_offers_no_repair");
    run_case(test, &[PhaseEnd::Exit], |_| {
        let mut app = TestApp::launch();
        let namespace = FakeAccount::new(1, 0).seed(&app);
        seed_verifiable_master_key(&app, &namespace);

        let error = CloudStorageError::DownloadFailed("master key".into());
        app.cloud.script_next(CloudCall::DownloadMasterKey, CloudFault::Fail(error));
        app.dispatch(verify()).then_can(&[N::Verify]);
        assert!(matches!(verification_state(&app), Some(CloudBackupVerificationState::Failed(_))));
        assert_eq!(passkey_card(&app), None);
        assert_eq!(control(&app, CloudBackupScreenAction::RepairPasskey), None);

        // the passkey answers with a key the wrapper no longer opens, then fails the rebuild
        let mock = &app.passkey.inner;
        mock.set_authenticate_result(Err(PasskeyError::RequestFailed {
            operation: PasskeyOperation::AuthenticateAssertion,
            reason: PasskeyFailureReason::InvalidResponse,
        }));
        mock.push_authenticate_result(Ok(vec![8; 32]));
        app.dispatch(verify()).then_can(&[N::Verify]);
        assert!(matches!(verification_state(&app), Some(CloudBackupVerificationState::Failed(_))));
        assert_eq!(passkey_card(&app), None);
        assert_eq!(control(&app, CloudBackupScreenAction::RepairPasskey), None);
        assert_eq!(app.manager().published_screen().root_prompt, CloudBackupRootPrompt::None);
    });
}

/// A replacement that can't be presented yet, as in a background verification, says nothing
/// about the passkey: no repair card, no missing-passkey reminder and no stored change
#[test]
fn a_passkey_request_that_cannot_present_yet_changes_nothing() {
    let test =
        concat!(module_path!(), "::a_passkey_request_that_cannot_present_yet_changes_nothing");
    run_case(test, &[PhaseEnd::Exit], |_| {
        let mut app = TestApp::launch();
        let namespace = FakeAccount::new(1, 0).seed(&app);
        seed_verifiable_master_key(&app, &namespace);
        let status = persisted().status();

        script_missing_passkey(
            &app,
            PasskeyError::NotReady { operation: PasskeyOperation::Registration },
        );
        app.dispatch(verify()).then_can(&[N::Verify]);
        assert_eq!(passkey_card(&app), None);
        assert_eq!(control(&app, CloudBackupScreenAction::RepairPasskey), None);
        assert_eq!(app.manager().published_screen().root_prompt, CloudBackupRootPrompt::None);
        assert_eq!(persisted().status(), status);
    });
}

/// A backup whose wallet is uploaded and that verifies offers Verify and Turn Off; it offers no
/// Sync Now with nothing to sync and no rebuild with nothing broken, since nothing draws them
#[test]
fn a_clean_backup_offers_no_sync_and_no_rebuild() {
    let test = concat!(module_path!(), "::a_clean_backup_offers_no_sync_and_no_rebuild");
    run_case(test, &[PhaseEnd::Exit], |_| {
        let mut app = TestApp::launch();
        let namespace = FakeAccount::new(1, 0).seed(&app);
        seed_verifiable_master_key(&app, &namespace);
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
        seed_verifiable_master_key(&app, &namespace);
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
