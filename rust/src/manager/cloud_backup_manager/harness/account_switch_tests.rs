//! A Google Drive account switch, with the test playing Android's half of it
//!
//! Rust asks for each platform step on the reconcile channel and Android answers each with a
//! typed result; the test reads the requests and answers them as the platform would. The user's
//! Switch, Try Again and Cancel go through dispatch like any other control
//!
//! Each test body runs in child processes; see [`super::run_case`]
#![cfg(test)]

use std::sync::Arc;
use std::time::Duration;

use cove_cspp::backup_data::wallet_filename_from_record_id;
use cove_cspp::master_key::MasterKey;
use cove_cspp::{Cspp, MasterKeyPromotionStatus};
use cove_device::cloud_storage::CloudStorageError;
use cove_device::keychain::Keychain;
use cove_device::passkey::{DiscoveredPasskeyResult, PasskeyError};
use parking_lot::Mutex;

use super::app::Step;
use super::process::checkpoint;
use super::{
    CloudCall, CloudFault, FakeAccount, Hold, NextAction as N, PhaseEnd, TestApp,
    assert_uploads_confirmed_in, finish_wipe, keychain_namespace, launch_keychain, load_detail,
    open_durable_state, persisted, run_case, seed_an_uploaded_wallet, seed_verifiable_master_key,
};
use crate::database::Database;
use crate::database::cloud_backup::{
    PersistedDriveAccountSwitchPhase, PersistedPendingVerificationCompletion,
};
use crate::manager::cloud_backup_manager::admission::{Admission, Rejection, UserIntent};
use crate::manager::cloud_backup_manager::keychain::{
    CSPP_NAMESPACE_ID_KEY, CSPP_PENDING_ENABLE_JOURNAL_KEY, CloudBackupKeychain,
};
use crate::manager::cloud_backup_manager::pending_enable::{
    PendingEnableJournal, PendingEnableJournalPhase, PendingEnableNamespaceOwnership,
};
use crate::manager::cloud_backup_manager::screen::{
    CloudBackupActionAvailability, CloudBackupDriveAccountSwitchStage, CloudBackupRejection,
    CloudBackupSavedPasskeyControl, CloudBackupSavedPasskeyLifecycle, CloudBackupScreenAction as A,
    CloudBackupScreenBody,
};
use crate::manager::cloud_backup_manager::{
    CloudBackupExclusiveOperation as Op, CloudBackupManagerAction, CloudBackupResetPermit,
    DriveAccountSwitchPlatformResult as Platform, DriveAccountSwitchPlatformState,
    PendingEnableLocalMetadataSnapshot, RequestedDriveAccountSwitchStep as Step_,
    VerificationState,
};

/// How long a newly registered passkey is given before Cove asks the user to confirm it
const PASSKEY_AVAILABILITY_WAIT: Duration = Duration::from_secs(3);

/// Android answers the step Rust asked for with `result`
fn platform_reports(app: &mut TestApp, result: Platform) -> Step<'_> {
    app.event(&format!("the platform reports {result:?}"), |app| {
        let manager = app.manager().clone();
        app.block_on(
            "the switch's owner to take the platform's result",
            manager.drive_account_switch_platform_result_handled(result),
        );
    })
}

/// The steps Rust asked the platform for since the last read
fn requested_steps(app: &TestApp) -> Vec<Step_> {
    app.manager().drained_drive_account_switch_steps()
}

fn switch_phase() -> Option<PersistedDriveAccountSwitchPhase> {
    persisted().drive_account_switch().map(|transition| transition.phase)
}

fn switch_stage(app: &TestApp) -> Option<CloudBackupDriveAccountSwitchStage> {
    match app.manager().published_screen().body {
        CloudBackupScreenBody::DriveAccountSwitch(switch) => Some(switch.stage),
        _ => None,
    }
}

/// The switch paused on its saved passkey, as the controls its confirmation draws
fn paused_switch_controls(app: &TestApp) -> Option<Vec<CloudBackupSavedPasskeyControl>> {
    app.saved_passkey_confirmation()
        .filter(|paused| paused.lifecycle == CloudBackupSavedPasskeyLifecycle::DriveAccountSwitch)
        .map(|paused| paused.controls)
}

fn availability(app: &TestApp, action: A) -> Option<CloudBackupActionAvailability> {
    app.manager().published_screen().actions.availability(action)
}

/// The user's Switch, run to the platform's commit of the new account, with the transition
fn switch_to_a_new_account(app: &mut TestApp) -> u64 {
    app.dispatch(CloudBackupManagerAction::SwitchDriveAccount).then_cannot(&[N::Enable]);
    let transition_id =
        persisted().drive_account_switch().expect("the switch is recorded").transition_id.value();
    assert_eq!(requested_steps(app), vec![Step_::SelectAccount(transition_id)]);

    platform_reports(app, Platform::AccountSelected(transition_id)).then_cannot(&[N::Enable]);
    transition_id
}

/// The launch integrity check reads, uploads and reads again under the one configuration it
/// started in: when a switch starts a new configuration while the check's first read of the old
/// account is held, the upload the check then starts and its follow-up read of the old namespace
/// never fill the new configuration's inventory with the old account's records or the provider's
/// authority
///
/// The switch's reinitialize fails at the new account's master key upload, so the device keeps
/// the old master key and the old account's records still decrypt. A switch that succeeds
/// replaces the key, and the old records then fail to read before anything is applied
#[test]
fn an_integrity_check_from_before_a_switch_leaves_the_new_inventory_alone() {
    let test = concat!(
        module_path!(),
        "::an_integrity_check_from_before_a_switch_leaves_the_new_inventory_alone"
    );
    run_case(test, &[PhaseEnd::Exit], |_| {
        let mut app = TestApp::launch();
        let old = FakeAccount::new(1, 1).seed_records(&app);
        let old_local = old.local[0].record_id.clone();
        let old_cloud_only = old.not_on_device[0].clone();

        // the check runs only with a stored passkey, which the switch's reinitialize then uses
        seed_verifiable_master_key(&app, &old.namespace);

        // the old namespace lost the local wallet's backup, so the check uploads that wallet and
        // then lists the namespace again
        let old_files = vec![wallet_filename_from_record_id(&old_cloud_only)];
        app.cloud.inner.set_wallet_files(old.namespace, old_files);
        app.cloud.persist();
        let old_scope = app.manager().inventory_scope();

        let held_list = Hold::default();
        app.cloud.script_next(CloudCall::ListWalletFiles, CloudFault::Hold(held_list.clone()));
        let manager = app.manager().clone();
        let check =
            cove_tokio::task::spawn(async move { manager.verify_backup_integrity_impl().await });
        app.block_on("the integrity check's first listing to start", held_list.entered());

        app.event("script the new account's master key upload to fail", |app| {
            let error = CloudStorageError::NotAvailable("the new account is offline".into());
            app.cloud.script_next(CloudCall::UploadMasterKey, CloudFault::Fail(error));
        })
        .then_cannot(&[N::Enable]);
        switch_to_a_new_account(&mut app);
        assert_eq!(
            switch_phase(),
            Some(PersistedDriveAccountSwitchPhase::AwaitingReinitializationRetry),
            "{}",
            app.describe()
        );
        assert_ne!(
            app.manager().inventory_scope(),
            old_scope,
            "the switch's reinitialize ended without starting a new configuration\n{}",
            app.describe()
        );
        assert!(app.manager().inventory.read().loaded_detail().is_none());

        app.event("release the integrity check's first listing", |_| held_list.release())
            .then_cannot(&[N::Enable]);
        app.block_on("the integrity check to finish", check).expect("integrity check task");
        app.settle();

        // read in a scope so the inventory lock is released before `describe` reads it again
        let (provider_confirmed, listed_records) = {
            let inventory = app.manager().inventory.read();
            let listed_records: Vec<_> = inventory
                .loaded_detail()
                .map(|loaded| {
                    let detail = &loaded.detail;
                    let items = detail.up_to_date.iter().chain(&detail.needs_sync);
                    items.map(|item| item.record_id.clone()).collect()
                })
                .unwrap_or_default();

            (inventory.listing_is_provider_confirmed(), listed_records)
        };

        assert!(
            !provider_confirmed,
            "the new configuration took the old account's listing as provider confirmed\n{}",
            app.describe()
        );
        assert!(
            listed_records.is_empty(),
            "the new configuration lists the old account's records {listed_records:?} \
             (local {old_local}, cloud-only {old_cloud_only})\n{}",
            app.describe()
        );
    });
}

/// A finalize that fails on the platform after the commit is the switch's to recover: Rust
/// shows the recovery and its Try Again, and Try Again asks the platform for its state and
/// finishes the switch from it
#[test]
fn a_failed_finalize_recovers_through_try_again() {
    let test = concat!(module_path!(), "::a_failed_finalize_recovers_through_try_again");
    run_case(test, &[PhaseEnd::Exit], |_| {
        let mut app = TestApp::launch();
        let old_namespace = FakeAccount::new(1, 0).seed(&app);
        load_detail(&mut app);

        // the new account's backup is created under a passkey the platform already has
        app.passkey.inner.set_discover_result(Ok(DiscoveredPasskeyResult {
            prf_output: vec![7; 32],
            credential_id: vec![1, 2, 3],
        }));

        let transition_id = switch_to_a_new_account(&mut app);
        assert_eq!(requested_steps(&app), vec![Step_::Commit(transition_id)], "{}", app.describe());
        assert_eq!(
            switch_phase(),
            Some(PersistedDriveAccountSwitchPhase::AwaitingAccountCommitSucceeded)
        );

        platform_reports(&mut app, Platform::Committed(transition_id)).then_cannot(&[N::Enable]);
        assert_eq!(requested_steps(&app), vec![Step_::Finalize(transition_id)]);
        assert_eq!(switch_phase(), None);

        // the platform could not remove its committed marker
        platform_reports(&mut app, Platform::FinalizeFailed(transition_id))
            .then_cannot(&[N::Enable]);
        assert!(
            matches!(
                switch_stage(&app),
                Some(CloudBackupDriveAccountSwitchStage::NeedsAttention(_))
            ),
            "{}",
            app.describe()
        );
        assert_eq!(
            availability(&app, A::RetryDriveAccountSwitchRecovery),
            Some(CloudBackupActionAvailability::Available)
        );
        // the new account is committed, so the previous one can't come back
        assert_eq!(
            availability(&app, A::CancelDriveAccountSwitch),
            Some(CloudBackupActionAvailability::Unavailable {
                reason: CloudBackupRejection::NothingToCancel
            })
        );

        app.dispatch(CloudBackupManagerAction::RetryDriveAccountSwitchRecovery)
            .then_cannot(&[N::Enable]);
        assert_eq!(requested_steps(&app), vec![Step_::ReportState]);
        assert_eq!(switch_stage(&app), None, "{}", app.describe());

        platform_reports(
            &mut app,
            Platform::State(DriveAccountSwitchPlatformState::Committed(transition_id)),
        )
        .then_cannot(&[N::Enable]);
        assert_eq!(requested_steps(&app), vec![Step_::Finalize(transition_id)]);

        platform_reports(&mut app, Platform::Finalized(transition_id))
            .then_can(&[N::TurnOff])
            .then_cannot(&[N::Enable]);

        // the backup lives in the new account and nothing of the switch is left
        let new_namespace = keychain_namespace().expect("a configured namespace");
        assert_ne!(new_namespace, old_namespace, "{}", app.describe());
        assert!(app.cloud.inner.has_namespace(&new_namespace));
        assert_eq!(switch_phase(), None);
        assert_eq!(app.manager().state.read().drive_account_switch_recovery(), None);
        assert!(matches!(
            app.manager().published_screen().body,
            CloudBackupScreenBody::Configured(_)
        ));
        assert_eq!(requested_steps(&app), vec![]);
    });
}

/// The user's Switch to an account whose commit fails on the platform, which leaves the switch's
/// run holding the slot without the Switch tap's permit
fn switch_whose_commit_fails(app: &mut TestApp) {
    FakeAccount::new(1, 0).seed(app);
    load_detail(app);
    app.passkey.inner.set_discover_result(Ok(DiscoveredPasskeyResult {
        prf_output: vec![7; 32],
        credential_id: vec![1, 2, 3],
    }));

    let transition_id = switch_to_a_new_account(app);
    assert_eq!(requested_steps(app), vec![Step_::Commit(transition_id)], "{}", app.describe());

    // the run stops for recovery, and the Switch tap's permit stops with it
    platform_reports(app, Platform::CommitFailed(transition_id)).then_cannot(&[N::Enable]);
    assert!(
        matches!(switch_stage(app), Some(CloudBackupDriveAccountSwitchStage::NeedsAttention(_))),
        "{}",
        app.describe()
    );
    assert_eq!(app.running_operation_may_present(), Some(false), "{}", app.describe());
}

/// After a restart, Android's launch report asks for the commit again
fn launch_report_asks_for_the_commit(app: &mut TestApp) -> u64 {
    let transition_id =
        persisted().drive_account_switch().expect("the switch is recorded").transition_id.value();
    platform_reports(app, Platform::State(DriveAccountSwitchPlatformState::Staged(transition_id)))
        .then_cannot(&[N::Enable]);
    assert_eq!(requested_steps(app), vec![Step_::Commit(transition_id)], "{}", app.describe());

    transition_id
}

/// Try Again on a switch whose commit failed, across a restart and a launch commit that failed
/// again, holds its own tap's permit through the commit and the new backup's verification that
/// follows it
#[test]
fn try_again_after_a_failed_commit_verifies_under_its_own_tap() {
    let test =
        concat!(module_path!(), "::try_again_after_a_failed_commit_verifies_under_its_own_tap");
    run_case(test, &[PhaseEnd::Exit, PhaseEnd::Exit], |phase| {
        if phase == 0 {
            let mut app = TestApp::launch();
            switch_whose_commit_fails(&mut app);
            return;
        }

        let mut app = TestApp::launch();
        app.passkey.inner.set_discover_result(Ok(DiscoveredPasskeyResult {
            prf_output: vec![7; 32],
            credential_id: vec![1, 2, 3],
        }));
        app.passkey.inner.set_authenticate_result(Ok(vec![7; 32]));

        let transition_id = launch_report_asks_for_the_commit(&mut app);
        platform_reports(&mut app, Platform::CommitFailed(transition_id)).then_cannot(&[N::Enable]);
        assert_eq!(
            availability(&app, A::RetryDriveAccountSwitchRecovery),
            Some(CloudBackupActionAvailability::Available)
        );
        assert_ne!(app.running_operation_may_present(), Some(true), "{}", app.describe());
        let calls_before = app.passkey.interactive_calls().len();

        app.dispatch(CloudBackupManagerAction::RetryDriveAccountSwitchRecovery)
            .then_cannot(&[N::Enable]);
        assert_eq!(requested_steps(&app), vec![Step_::ReportState]);
        assert_eq!(app.running_operation_may_present(), Some(true), "{}", app.describe());

        platform_reports(
            &mut app,
            Platform::State(DriveAccountSwitchPlatformState::Staged(transition_id)),
        )
        .then_cannot(&[N::Enable]);
        assert_eq!(requested_steps(&app), vec![Step_::Commit(transition_id)], "{}", app.describe());

        // the commit continues into the new backup's verification under the Try Again permit;
        // invariant 6 checks each of its passkey requests against that live permit
        platform_reports(&mut app, Platform::Committed(transition_id)).then_cannot(&[N::Enable]);
        assert_eq!(requested_steps(&app), vec![Step_::Finalize(transition_id)]);
        assert!(
            matches!(app.manager().model_snapshot().verification, VerificationState::Verified(_)),
            "{}",
            app.describe()
        );
        assert!(app.passkey.interactive_calls().len() > calls_before, "{}", app.describe());

        platform_reports(&mut app, Platform::Finalized(transition_id))
            .then_can(&[N::TurnOff])
            .then_cannot(&[N::Enable]);
        assert_eq!(switch_phase(), None);
    });
}

/// A commit no tap stands behind, as the launch report's after a restart, ends the switch
/// without verifying the new backup, which would need its passkey, and offers the user's Verify
/// instead
#[test]
fn a_launch_commit_settles_without_verifying_and_offers_verify() {
    let test =
        concat!(module_path!(), "::a_launch_commit_settles_without_verifying_and_offers_verify");
    run_case(test, &[PhaseEnd::Exit, PhaseEnd::Exit], |phase| {
        if phase == 0 {
            let mut app = TestApp::launch();
            switch_whose_commit_fails(&mut app);
            return;
        }

        let mut app = TestApp::launch();
        let transition_id = launch_report_asks_for_the_commit(&mut app);

        platform_reports(&mut app, Platform::Committed(transition_id)).then_cannot(&[N::Enable]);
        assert_eq!(requested_steps(&app), vec![Step_::Finalize(transition_id)]);
        assert!(!app.runs_operation(), "{}", app.describe());
        // no verification of the new backup ran: it would have asked for the passkey. The
        // background upload confirmation may still settle on its own, since it presents nothing
        assert!(app.passkey.interactive_calls().is_empty(), "{}", app.describe());
        assert!(
            !matches!(app.manager().model_snapshot().verification, VerificationState::Verified(_)),
            "{}",
            app.describe()
        );

        platform_reports(&mut app, Platform::Finalized(transition_id))
            .then_can(&[N::TurnOff, N::Verify])
            .then_cannot(&[N::Enable]);
        assert_eq!(switch_phase(), None);
    });
}

/// A switch whose reinitialize registers a passkey pauses for the user to confirm it, as a
/// plain reinitialize does: it holds the switch, asks the platform for no commit, and offers
/// Confirm Saved Passkey, which goes on as that switch's reinitialize and then commits
#[test]
fn a_switch_paused_for_its_saved_passkey_holds_until_confirmed() {
    let test =
        concat!(module_path!(), "::a_switch_paused_for_its_saved_passkey_holds_until_confirmed");
    run_case(test, &[PhaseEnd::Exit], |_| {
        let mut app = TestApp::launch();
        let old_namespace = FakeAccount::new(1, 0).seed(&app);
        load_detail(&mut app);

        // no passkey is discoverable, so the new account's reinitialize registers one
        app.passkey.inner.set_discover_result(Err(PasskeyError::NoCredentialFound));
        app.passkey.inner.set_create_result(Ok(vec![4, 5, 6]));

        let transition_id = switch_to_a_new_account(&mut app);
        app.advance_clock(PASSKEY_AVAILABILITY_WAIT).then_cannot(&[N::Enable]);

        // paused: the switch holds, nothing runs, no commit was asked for and nothing replaced
        assert_eq!(
            switch_phase(),
            Some(PersistedDriveAccountSwitchPhase::AwaitingSavedPasskeyConfirmation),
            "{}",
            app.describe()
        );
        assert!(!app.runs_operation(), "{}", app.describe());
        assert_eq!(requested_steps(&app), vec![]);
        assert_eq!(keychain_namespace().as_deref(), Some(old_namespace.as_str()));
        assert_eq!(
            paused_switch_controls(&app),
            Some(vec![CloudBackupSavedPasskeyControl::Confirm])
        );
        assert_eq!(
            availability(&app, A::ConfirmSavedPasskey),
            Some(CloudBackupActionAvailability::Available)
        );
        assert_eq!(
            app.admission_of(UserIntent::DiscardPendingEnable),
            Admission::Rejected(Rejection::NothingToCancel)
        );

        // the platform reports the staged account at launch; the paused switch keeps holding
        platform_reports(
            &mut app,
            Platform::State(DriveAccountSwitchPlatformState::Staged(transition_id)),
        )
        .then_cannot(&[N::Enable]);
        assert_eq!(requested_steps(&app), vec![]);
        assert!(!app.runs_operation(), "{}", app.describe());

        app.passkey.inner.set_authenticate_result(Ok(vec![7; 32]));
        app.dispatch(CloudBackupManagerAction::ConfirmSavedPasskey).then_cannot(&[N::Enable]);

        // the confirmed reinitialize finished in the new account, which the platform commits
        assert_eq!(requested_steps(&app), vec![Step_::Commit(transition_id)], "{}", app.describe());
        assert_eq!(
            switch_phase(),
            Some(PersistedDriveAccountSwitchPhase::AwaitingAccountCommitSucceeded)
        );

        platform_reports(&mut app, Platform::Committed(transition_id)).then_cannot(&[N::Enable]);
        platform_reports(&mut app, Platform::Finalized(transition_id))
            .then_can(&[N::TurnOff])
            .then_cannot(&[N::Enable]);

        let new_namespace = keychain_namespace().expect("a configured namespace");
        assert_ne!(new_namespace, old_namespace, "{}", app.describe());
        assert_eq!(switch_phase(), None);
        assert_eq!(app.passkey.inner.create_count(), 1);
    });
}

/// The switch's reinitialize uploads every wallet to the new account's namespace, and each is
/// confirmed there, though the wallet's upload row began in the namespace the switch replaced
#[test]
fn a_switch_confirms_its_uploads_in_the_new_namespace() {
    let test = concat!(module_path!(), "::a_switch_confirms_its_uploads_in_the_new_namespace");
    run_case(test, &[PhaseEnd::Exit], |_| {
        let mut app = TestApp::launch();
        let old_namespace = seed_an_uploaded_wallet(&mut app);

        // the new account's backup is created under a passkey the platform already has
        app.passkey.inner.set_discover_result(Ok(DiscoveredPasskeyResult {
            prf_output: vec![7; 32],
            credential_id: vec![1, 2, 3],
        }));

        let transition_id = switch_to_a_new_account(&mut app);
        assert_eq!(requested_steps(&app), vec![Step_::Commit(transition_id)], "{}", app.describe());
        platform_reports(&mut app, Platform::Committed(transition_id)).then_cannot(&[N::Enable]);
        platform_reports(&mut app, Platform::Finalized(transition_id))
            .then_can(&[N::TurnOff])
            .then_cannot(&[N::Enable]);
        app.advance_clock(Duration::from_secs(60)).then_can(&[N::TurnOff]);

        let new_namespace = keychain_namespace().expect("a configured namespace");
        assert_ne!(new_namespace, old_namespace, "{}", app.describe());
        assert_uploads_confirmed_in(&app, &new_namespace);
    });
}

/// The user's Switch to an account whose reinitialize fails, held at its retry, with the
/// transition and the namespace the device was on before the switch
fn switch_whose_reinitialize_fails(app: &mut TestApp) -> (u64, String) {
    let old_namespace = FakeAccount::new(1, 0).seed(app);
    load_detail(app);
    app.passkey.inner.set_discover_result(Ok(DiscoveredPasskeyResult {
        prf_output: vec![7; 32],
        credential_id: vec![1, 2, 3],
    }));

    app.event("script the new account's master key upload to fail", |app| {
        let error = CloudStorageError::NotAvailable("the new account is offline".into());
        app.cloud.script_next(CloudCall::UploadMasterKey, CloudFault::Fail(error));
    })
    .then_cannot(&[N::Enable]);
    let transition_id = switch_to_a_new_account(app);

    assert_eq!(
        switch_phase(),
        Some(PersistedDriveAccountSwitchPhase::AwaitingReinitializationRetry),
        "{}",
        app.describe()
    );
    assert!(!app.runs_operation(), "{}", app.describe());
    assert_eq!(requested_steps(app), vec![]);
    assert!(
        matches!(switch_stage(app), Some(CloudBackupDriveAccountSwitchStage::NeedsAttention(_))),
        "{}",
        app.describe()
    );
    assert_eq!(
        availability(app, A::RetryDriveAccountSwitchRecovery),
        Some(CloudBackupActionAvailability::Available)
    );
    assert_eq!(keychain_namespace().as_deref(), Some(old_namespace.as_str()));

    (transition_id, old_namespace)
}

/// A switch whose reinitialize failed is not a dead end, with or without a relaunch
///
/// Without one, Try Again goes back to the failed attempt's registered passkey, the pause the
/// journal records, and Confirm Saved Passkey uploads again under it. A relaunch restores the failed attempt's
/// registered passkey for confirmation, and the switch shows that pause at once, before Android
/// reports anything: Confirm Saved Passkey goes on as the switch's run and the platform commits.
/// Cancel is never offered, since the failed reinitialize may have begun moving this device to
/// the new backup
#[test]
fn a_failed_switch_reinitialize_finishes_through_try_again() {
    let test = concat!(module_path!(), "::a_failed_switch_reinitialize_finishes_through_try_again");
    run_case(test, &[PhaseEnd::Exit], |_| {
        let mut app = TestApp::launch();
        let (transition_id, old_namespace) = switch_whose_reinitialize_fails(&mut app);
        assert_eq!(
            availability(&app, A::CancelDriveAccountSwitch),
            Some(CloudBackupActionAvailability::Unavailable {
                reason: CloudBackupRejection::NothingToCancel
            })
        );

        app.dispatch(CloudBackupManagerAction::RetryDriveAccountSwitchRecovery)
            .then_cannot(&[N::Enable]);
        assert_switch_paused_with_its_confirm(&app);
        assert_eq!(requested_steps(&app), vec![], "{}", app.describe());

        app.passkey.inner.set_authenticate_result(Ok(vec![7; 32]));
        app.dispatch(CloudBackupManagerAction::ConfirmSavedPasskey).then_cannot(&[N::Enable]);
        assert_eq!(requested_steps(&app), vec![Step_::Commit(transition_id)], "{}", app.describe());

        platform_reports(&mut app, Platform::Committed(transition_id)).then_cannot(&[N::Enable]);
        platform_reports(&mut app, Platform::Finalized(transition_id))
            .then_can(&[N::TurnOff])
            .then_cannot(&[N::Enable]);

        let new_namespace = keychain_namespace().expect("a configured namespace");
        assert_ne!(new_namespace, old_namespace, "{}", app.describe());
        assert!(app.cloud.inner.has_namespace(&new_namespace));
        assert_eq!(switch_phase(), None);
    });
}

/// A relaunch after a switch's reinitialize failed shows the switch's way out at once, before
/// Android reports anything, and keeps it after the platform's launch report
#[test]
fn a_failed_switch_reinitialize_finishes_after_a_relaunch() {
    let test = concat!(module_path!(), "::a_failed_switch_reinitialize_finishes_after_a_relaunch");
    run_case(test, &[PhaseEnd::Exit], |_| {
        let mut app = TestApp::launch();
        let (transition_id, old_namespace) = switch_whose_reinitialize_fails(&mut app);

        app.relaunch().then_cannot(&[N::Enable]);
        let paused_with_its_confirm = |app: &TestApp| {
            assert!(!app.runs_operation(), "{}", app.describe());
            assert_eq!(
                switch_phase(),
                Some(PersistedDriveAccountSwitchPhase::AwaitingSavedPasskeyConfirmation),
                "{}",
                app.describe()
            );
            assert_eq!(
                paused_switch_controls(app),
                Some(vec![CloudBackupSavedPasskeyControl::Confirm]),
                "{}",
                app.describe()
            );
            assert_eq!(
                availability(app, A::ConfirmSavedPasskey),
                Some(CloudBackupActionAvailability::Available)
            );
        };
        paused_with_its_confirm(&app);

        // Android reports its staged account at every launch, which holds the paused switch
        platform_reports(
            &mut app,
            Platform::State(DriveAccountSwitchPlatformState::Staged(transition_id)),
        )
        .then_cannot(&[N::Enable]);
        assert_eq!(requested_steps(&app), vec![]);
        paused_with_its_confirm(&app);

        app.passkey.inner.set_authenticate_result(Ok(vec![7; 32]));
        app.dispatch(CloudBackupManagerAction::ConfirmSavedPasskey).then_cannot(&[N::Enable]);
        assert_eq!(requested_steps(&app), vec![Step_::Commit(transition_id)], "{}", app.describe());

        platform_reports(&mut app, Platform::Committed(transition_id)).then_cannot(&[N::Enable]);
        platform_reports(&mut app, Platform::Finalized(transition_id))
            .then_can(&[N::TurnOff])
            .then_cannot(&[N::Enable]);

        let new_namespace = keychain_namespace().expect("a configured namespace");
        assert_ne!(new_namespace, old_namespace, "{}", app.describe());
        assert!(app.cloud.inner.has_namespace(&new_namespace));
        assert_eq!(switch_phase(), None);
    });
}

/// A switch paused for its saved passkey whose platform lost the staged account rolls back,
/// and takes its paused reinitialize with it: nothing of the new account's key material is left
/// to come back at the next launch as a reinitialize of the previous account's backup
#[test]
fn a_paused_switch_that_rolls_back_leaves_nothing_of_its_reinitialize() {
    let test = concat!(
        module_path!(),
        "::a_paused_switch_that_rolls_back_leaves_nothing_of_its_reinitialize"
    );
    run_case(test, &[PhaseEnd::Exit], |_| {
        let mut app = TestApp::launch();
        let old_namespace = FakeAccount::new(1, 0).seed(&app);
        load_detail(&mut app);

        // no passkey is discoverable, so the new account's reinitialize registers one
        app.passkey.inner.set_discover_result(Err(PasskeyError::NoCredentialFound));
        app.passkey.inner.set_create_result(Ok(vec![4, 5, 6]));

        let transition_id = switch_to_a_new_account(&mut app);
        app.advance_clock(PASSKEY_AVAILABILITY_WAIT).then_cannot(&[N::Enable]);
        assert_eq!(
            switch_phase(),
            Some(PersistedDriveAccountSwitchPhase::AwaitingSavedPasskeyConfirmation),
            "{}",
            app.describe()
        );
        assert!(pending_enable_journal().is_some(), "{}", app.describe());

        // the platform lost the staged account
        platform_reports(&mut app, Platform::State(DriveAccountSwitchPlatformState::NoTransition))
            .then_cannot(&[N::Enable]);
        assert_eq!(requested_steps(&app), vec![Step_::Rollback(transition_id)]);
        assert_eq!(switch_phase(), Some(PersistedDriveAccountSwitchPhase::AwaitingAccountRollback));

        platform_reports(&mut app, Platform::RolledBack(transition_id))
            .then_can(&[N::TurnOff])
            .then_cannot(&[N::Enable]);
        assert_eq!(switch_phase(), None);
        assert_eq!(pending_enable_journal(), None, "{}", app.describe());

        // the next launch finds the previous account's backup and no reinitialize to confirm
        app.relaunch().then_can(&[N::TurnOff]).then_cannot(&[N::Enable]);
        assert_eq!(keychain_namespace().as_deref(), Some(old_namespace.as_str()));
        assert!(
            matches!(app.manager().published_screen().body, CloudBackupScreenBody::Configured(_)),
            "{}",
            app.describe()
        );
        assert_eq!(availability(&app, A::ConfirmSavedPasskey), None, "{}", app.describe());
        assert_eq!(availability(&app, A::DiscardPendingEnable), None, "{}", app.describe());
    });
}

/// Fails the next save of the switch's phase as `phase`, the way a failed redb commit does
fn fail_the_next_switch_phase_save(phase: PersistedDriveAccountSwitchPhase) {
    Database::global().cloud_backup_state.fail_next_write_where(move |state| {
        state.drive_account_switch().is_some_and(|transition| transition.phase == phase)
    });
}

/// A switch whose reinitialize ended but whose next phase was not saved: its run is over, so
/// nothing is live at quiescence (invariant 4a), the record still says Reinitializing, and the
/// screen shows Try Again and no Cancel
fn assert_switch_stopped_with_its_completion_unsaved(app: &mut TestApp) {
    app.quiesce().then_cannot(&[N::Enable]);
    assert_eq!(
        switch_phase(),
        Some(PersistedDriveAccountSwitchPhase::Reinitializing),
        "{}",
        app.describe()
    );
    assert!(!app.runs_operation(), "{}", app.describe());
    assert_eq!(requested_steps(app), vec![]);
    assert!(
        matches!(switch_stage(app), Some(CloudBackupDriveAccountSwitchStage::NeedsAttention(_))),
        "{}",
        app.describe()
    );
    assert_eq!(
        availability(app, A::RetryDriveAccountSwitchRecovery),
        Some(CloudBackupActionAvailability::Available)
    );
    assert_eq!(
        availability(app, A::CancelDriveAccountSwitch),
        Some(CloudBackupActionAvailability::Unavailable {
            reason: CloudBackupRejection::NothingToCancel
        })
    );
}

/// The user's Switch to an account whose reinitialize succeeds while saving the switch's next
/// phase fails once, with the transition and the namespace the device was on before the switch
fn switch_whose_finished_reinitialize_is_not_saved(app: &mut TestApp) -> (u64, String) {
    let old_namespace = FakeAccount::new(1, 0).seed(app);
    load_detail(app);
    app.passkey.inner.set_discover_result(Ok(DiscoveredPasskeyResult {
        prf_output: vec![7; 32],
        credential_id: vec![1, 2, 3],
    }));

    fail_the_next_switch_phase_save(
        PersistedDriveAccountSwitchPhase::AwaitingAccountCommitSucceeded,
    );
    let transition_id = switch_to_a_new_account(app);
    assert_switch_stopped_with_its_completion_unsaved(app);

    (transition_id, old_namespace)
}

/// The platform commits and finalizes the switch Rust asked it to commit, which leaves the
/// backup in the new account and nothing of the switch
fn platform_finishes_the_switch(app: &mut TestApp, transition_id: u64, old_namespace: &str) {
    assert_eq!(requested_steps(app), vec![Step_::Commit(transition_id)], "{}", app.describe());

    platform_reports(app, Platform::Committed(transition_id)).then_cannot(&[N::Enable]);
    assert_eq!(requested_steps(app), vec![Step_::Finalize(transition_id)], "{}", app.describe());

    platform_reports(app, Platform::Finalized(transition_id))
        .then_can(&[N::TurnOff])
        .then_cannot(&[N::Enable]);

    let new_namespace = keychain_namespace().expect("a configured namespace");
    assert_ne!(new_namespace, old_namespace, "{}", app.describe());
    assert!(app.cloud.inner.has_namespace(&new_namespace));
    assert_eq!(switch_phase(), None);
    assert!(!app.runs_operation(), "{}", app.describe());
}

/// A reinitialize that succeeded but whose switch could not save that it now waits for the
/// platform's commit is not a dead end: the switch keeps the result, and Try Again saves it
/// and goes on to the commit from the platform's state
#[test]
fn a_switch_whose_finished_reinitialize_was_not_saved_goes_on_through_try_again() {
    let test = concat!(
        module_path!(),
        "::a_switch_whose_finished_reinitialize_was_not_saved_goes_on_through_try_again"
    );
    run_case(test, &[PhaseEnd::Exit], |_| {
        let mut app = TestApp::launch();
        let (transition_id, old_namespace) =
            switch_whose_finished_reinitialize_is_not_saved(&mut app);

        app.dispatch(CloudBackupManagerAction::RetryDriveAccountSwitchRecovery)
            .then_cannot(&[N::Enable]);
        assert_eq!(
            switch_phase(),
            Some(PersistedDriveAccountSwitchPhase::AwaitingAccountCommitSucceeded),
            "{}",
            app.describe()
        );
        assert_eq!(requested_steps(&app), vec![Step_::ReportState], "{}", app.describe());

        platform_reports(
            &mut app,
            Platform::State(DriveAccountSwitchPlatformState::Staged(transition_id)),
        )
        .then_cannot(&[N::Enable]);
        platform_finishes_the_switch(&mut app, transition_id, &old_namespace);
    });
}

/// Android reports its state again when its activity is recreated in the same process; that
/// report goes on from the phase the run could not save, as Try Again does
#[test]
fn a_launch_report_goes_on_from_a_switch_completion_that_was_not_saved() {
    let test = concat!(
        module_path!(),
        "::a_launch_report_goes_on_from_a_switch_completion_that_was_not_saved"
    );
    run_case(test, &[PhaseEnd::Exit], |_| {
        let mut app = TestApp::launch();
        let (transition_id, old_namespace) =
            switch_whose_finished_reinitialize_is_not_saved(&mut app);

        platform_reports(
            &mut app,
            Platform::State(DriveAccountSwitchPlatformState::Staged(transition_id)),
        )
        .then_cannot(&[N::Enable]);
        assert_eq!(
            switch_phase(),
            Some(PersistedDriveAccountSwitchPhase::AwaitingAccountCommitSucceeded),
            "{}",
            app.describe()
        );
        platform_finishes_the_switch(&mut app, transition_id, &old_namespace);
        assert_eq!(app.cloud.count(CloudCall::UploadMasterKey), 1, "no second reinitialize");
    });
}

/// A local reset that fails before its wipe, as the reset's owner abandons one
fn abandon_a_local_reset(app: &mut TestApp) {
    let manager = app.manager().clone();
    let permit = app
        .block_on("the reset to be ready", async move { manager.prepare_local_reset().await })
        .expect("reset permit");
    app.event("the wipe fails before its commit", |app| {
        app.block_on("the reset to end", permit.resume_after_failed_reset())
            .expect("abandon the reset");
    })
    .then_cannot(&[N::Enable]);
}

/// A switch whose reinitialize a reset revoked before the reset failed before its wipe: the
/// switch is still Reinitializing with no run, and offers Try Again. Returns the transition
fn switch_a_failed_reset_stopped_in_its_reinitialize(app: &mut TestApp) -> u64 {
    let namespace = FakeAccount::new(1, 0).seed(app);
    // the cloud's master key opens under the passkey the fake provider answers with, so a run
    // that holds a permit can recover the key it needs
    seed_verifiable_master_key(app, &namespace);
    load_detail(app);

    // without a local master key the reinitialize first lists the cloud's namespaces, a provider
    // read that holds the run in Reinitializing while the reset revokes it
    CloudBackupKeychain::global().delete_master_key().expect("delete local master key");
    let revoked_listing = Hold::default();
    app.cloud.script_next(CloudCall::ListNamespaces, CloudFault::Hold(revoked_listing.clone()));
    let transition_id = switch_to_a_new_account(app);
    app.block_on("the first run's listing to start", revoked_listing.entered());

    abandon_a_local_reset(app);
    app.event("release the revoked run's listing", |_| revoked_listing.release())
        .then_cannot(&[N::Enable]);
    app.block_on("the revoked run's listing to return", revoked_listing.finished());

    assert_eq!(switch_phase(), Some(PersistedDriveAccountSwitchPhase::Reinitializing));
    assert!(!app.runs_operation(), "{}", app.describe());
    assert_eq!(
        availability(app, A::RetryDriveAccountSwitchRecovery),
        Some(CloudBackupActionAvailability::Available),
        "{}",
        app.describe()
    );

    transition_id
}

/// The switch's run, which holds a tap's permit, commits, verifies the new backup and finalizes
///
/// The namespace isn't checked: the fake shares namespaces across accounts, so
/// platform_finishes_the_switch's namespace check can't apply to a run that recovered the
/// cloud's master key
fn platform_finishes_the_recovered_switch(app: &mut TestApp, transition_id: u64) {
    assert_eq!(requested_steps(app), vec![Step_::Commit(transition_id)], "{}", app.describe());
    platform_reports(app, Platform::Committed(transition_id)).then_cannot(&[N::Enable]);
    assert_eq!(requested_steps(app), vec![Step_::Finalize(transition_id)]);
    assert!(
        matches!(app.manager().model_snapshot().verification, VerificationState::Verified(_)),
        "{}",
        app.describe()
    );

    platform_reports(app, Platform::Finalized(transition_id))
        .then_can(&[N::TurnOff])
        .then_cannot(&[N::Enable]);
    assert_eq!(switch_phase(), None);
    assert!(!app.runs_operation(), "{}", app.describe());
}

/// A reset that revokes a switch's reinitialize and then fails before its wipe leaves the switch
/// in Reinitializing with no run. Try Again there asks the platform for its state first, and the
/// staged account's report restarts the reinitialize under Try Again's permit, so the passkey
/// it needs is asked for under that tap and the switch finishes
#[test]
fn try_again_restarts_a_reinitialize_a_failed_reset_stopped_under_its_tap() {
    let test = concat!(
        module_path!(),
        "::try_again_restarts_a_reinitialize_a_failed_reset_stopped_under_its_tap"
    );
    run_case(test, &[PhaseEnd::Exit], |_| {
        let mut app = TestApp::launch();
        let transition_id = switch_a_failed_reset_stopped_in_its_reinitialize(&mut app);
        let calls_before = app.passkey.interactive_calls().len();

        // Try Again asks the platform first: only its report can say the staged account is
        // still there, so nothing runs yet
        app.dispatch(CloudBackupManagerAction::RetryDriveAccountSwitchRecovery)
            .then_cannot(&[N::Enable]);
        assert_eq!(requested_steps(&app), vec![Step_::ReportState], "{}", app.describe());
        assert!(!app.runs_operation(), "{}", app.describe());

        // the staged account's report restarts the reinitialize under Try Again's permit;
        // invariant 6 checks each of its passkey requests against that live permit
        platform_reports(
            &mut app,
            Platform::State(DriveAccountSwitchPlatformState::Staged(transition_id)),
        )
        .then_cannot(&[N::Enable]);
        assert_eq!(app.running_operation_may_present(), Some(true), "{}", app.describe());
        assert!(app.passkey.interactive_calls().len() > calls_before, "{}", app.describe());

        platform_finishes_the_recovered_switch(&mut app, transition_id);
    });
}

/// A reset revokes every permit, including Try Again's held for the platform's report: a report
/// that arrives after a reset that failed before its wipe restarts the reinitialize with no
/// permit, so it asks for no passkey. Only a new Try Again recovers the switch
#[test]
fn a_reset_cancels_try_again_s_permit_held_for_the_platform_s_report() {
    let test = concat!(
        module_path!(),
        "::a_reset_cancels_try_again_s_permit_held_for_the_platform_s_report"
    );
    run_case(test, &[PhaseEnd::Exit], |_| {
        let mut app = TestApp::launch();
        let transition_id = switch_a_failed_reset_stopped_in_its_reinitialize(&mut app);
        let calls_before = app.passkey.interactive_calls().len();
        let withdrawn_before = app.passkey.withdrawn_calls().len();

        app.dispatch(CloudBackupManagerAction::RetryDriveAccountSwitchRecovery)
            .then_cannot(&[N::Enable]);
        assert_eq!(requested_steps(&app), vec![Step_::ReportState], "{}", app.describe());

        // a reset begins before the report arrives and fails before its wipe
        abandon_a_local_reset(&mut app);

        // the delayed report restarts the reinitialize with no permit, so it fails before the
        // passkey provider and the switch waits for a new Try Again
        platform_reports(
            &mut app,
            Platform::State(DriveAccountSwitchPlatformState::Staged(transition_id)),
        )
        .then_cannot(&[N::Enable]);
        assert_ne!(app.running_operation_may_present(), Some(true), "{}", app.describe());
        assert_eq!(app.passkey.interactive_calls().len(), calls_before, "{}", app.describe());
        assert_eq!(app.passkey.withdrawn_calls().len(), withdrawn_before, "{}", app.describe());
        assert_eq!(requested_steps(&app), vec![], "{}", app.describe());
        assert_eq!(
            switch_phase(),
            Some(PersistedDriveAccountSwitchPhase::AwaitingReinitializationRetry),
            "{}",
            app.describe()
        );
        assert_eq!(
            availability(&app, A::RetryDriveAccountSwitchRecovery),
            Some(CloudBackupActionAvailability::Available),
            "{}",
            app.describe()
        );

        // a new Try Again reruns the reinitialize under its own permit
        app.dispatch(CloudBackupManagerAction::RetryDriveAccountSwitchRecovery)
            .then_cannot(&[N::Enable]);
        assert_eq!(app.running_operation_may_present(), Some(true), "{}", app.describe());
        assert!(app.passkey.interactive_calls().len() > calls_before, "{}", app.describe());

        platform_finishes_the_recovered_switch(&mut app, transition_id);
    });
}

/// The same unsaved result is lost with the process, and the record still says Reinitializing.
/// The platform's launch report runs the reinitialize again, but no tap stands behind that run,
/// so it stops before asking for the passkey and the switch waits for Try Again. Try Again
/// reruns it under its own tap's permit and commits it
#[test]
fn try_again_reruns_an_unsaved_switch_reinitialize_after_a_relaunch() {
    let test = concat!(
        module_path!(),
        "::try_again_reruns_an_unsaved_switch_reinitialize_after_a_relaunch"
    );
    run_case(test, &[PhaseEnd::Exit], |_| {
        let mut app = TestApp::launch();
        let (transition_id, old_namespace) =
            switch_whose_finished_reinitialize_is_not_saved(&mut app);

        app.relaunch().then_cannot(&[N::Enable]);
        assert_eq!(
            switch_phase(),
            Some(PersistedDriveAccountSwitchPhase::Reinitializing),
            "{}",
            app.describe()
        );
        assert!(!app.runs_operation(), "{}", app.describe());
        assert_eq!(
            availability(&app, A::RetryDriveAccountSwitchRecovery),
            Some(CloudBackupActionAvailability::Available),
            "{}",
            app.describe()
        );

        app.passkey.inner.set_discover_result(Ok(DiscoveredPasskeyResult {
            prf_output: vec![7; 32],
            credential_id: vec![1, 2, 3],
        }));
        let calls_before = app.passkey.interactive_calls().len();
        let withdrawn_before = app.passkey.withdrawn_calls().len();

        // the launch report's rerun holds no permit, so it fails before the passkey provider,
        // registration and discovery alike, and the switch keeps its record for Try Again
        platform_reports(
            &mut app,
            Platform::State(DriveAccountSwitchPlatformState::Staged(transition_id)),
        )
        .then_cannot(&[N::Enable]);
        assert_eq!(app.passkey.interactive_calls().len(), calls_before, "{}", app.describe());
        assert_eq!(app.passkey.withdrawn_calls().len(), withdrawn_before, "{}", app.describe());
        assert_eq!(requested_steps(&app), vec![], "{}", app.describe());
        assert_eq!(
            switch_phase(),
            Some(PersistedDriveAccountSwitchPhase::AwaitingReinitializationRetry),
            "{}",
            app.describe()
        );
        assert_eq!(
            persisted().drive_account_switch().map(|switch| switch.transition_id.value()),
            Some(transition_id)
        );
        assert!(!app.runs_operation(), "{}", app.describe());
        assert!(
            matches!(
                switch_stage(&app),
                Some(CloudBackupDriveAccountSwitchStage::NeedsAttention(_))
            ),
            "{}",
            app.describe()
        );
        assert_eq!(
            availability(&app, A::RetryDriveAccountSwitchRecovery),
            Some(CloudBackupActionAvailability::Available),
            "{}",
            app.describe()
        );

        // Try Again binds its ticket to the rerun, so its passkey requests run under that grant;
        // invariant 6 checks each one against the live permit
        app.dispatch(CloudBackupManagerAction::RetryDriveAccountSwitchRecovery)
            .then_cannot(&[N::Enable]);
        assert_eq!(app.running_operation_may_present(), Some(true), "{}", app.describe());
        assert!(app.passkey.interactive_calls().len() > calls_before, "{}", app.describe());

        platform_finishes_the_switch(&mut app, transition_id, &old_namespace);
    });
}

/// A reinitialize that failed and whose switch could not save that it waits for Try Again
/// stops the same way; Try Again saves it and goes back to the journal's pause, which Confirm
/// continues
#[test]
fn a_switch_whose_failed_reinitialize_was_not_saved_reruns_through_try_again() {
    let test = concat!(
        module_path!(),
        "::a_switch_whose_failed_reinitialize_was_not_saved_reruns_through_try_again"
    );
    run_case(test, &[PhaseEnd::Exit], |_| {
        let mut app = TestApp::launch();
        let old_namespace = FakeAccount::new(1, 0).seed(&app);
        load_detail(&mut app);
        app.passkey.inner.set_discover_result(Ok(DiscoveredPasskeyResult {
            prf_output: vec![7; 32],
            credential_id: vec![1, 2, 3],
        }));

        app.event("script the new account's master key upload to fail", |app| {
            let error = CloudStorageError::NotAvailable("the new account is offline".into());
            app.cloud.script_next(CloudCall::UploadMasterKey, CloudFault::Fail(error));
        })
        .then_cannot(&[N::Enable]);
        fail_the_next_switch_phase_save(
            PersistedDriveAccountSwitchPhase::AwaitingReinitializationRetry,
        );
        let transition_id = switch_to_a_new_account(&mut app);
        assert_switch_stopped_with_its_completion_unsaved(&mut app);
        assert_eq!(keychain_namespace().as_deref(), Some(old_namespace.as_str()));

        app.dispatch(CloudBackupManagerAction::RetryDriveAccountSwitchRecovery)
            .then_cannot(&[N::Enable]);
        assert_switch_paused_with_its_confirm(&app);

        app.passkey.inner.set_authenticate_result(Ok(vec![7; 32]));
        app.dispatch(CloudBackupManagerAction::ConfirmSavedPasskey).then_cannot(&[N::Enable]);
        platform_finishes_the_switch(&mut app, transition_id, &old_namespace);
    });
}

/// The user's Switch to an account whose reinitialize pauses for the passkey it registered
/// while saving that pause fails once, with the transition and the previous namespace
fn switch_whose_pause_is_not_saved(app: &mut TestApp) -> (u64, String) {
    let old_namespace = FakeAccount::new(1, 0).seed(app);
    load_detail(app);

    // no passkey is discoverable, so the new account's reinitialize registers one
    app.passkey.inner.set_discover_result(Err(PasskeyError::NoCredentialFound));
    app.passkey.inner.set_create_result(Ok(vec![4, 5, 6]));

    fail_the_next_switch_phase_save(
        PersistedDriveAccountSwitchPhase::AwaitingSavedPasskeyConfirmation,
    );
    let transition_id = switch_to_a_new_account(app);
    app.advance_clock(PASSKEY_AVAILABILITY_WAIT).then_cannot(&[N::Enable]);
    assert_switch_stopped_with_its_completion_unsaved(app);

    (transition_id, old_namespace)
}

/// A reinitialize that paused for its saved passkey and whose switch could not save the pause
/// stops the same way; Try Again saves the pause, and Confirm Saved Passkey then goes on as the
/// switch's run
#[test]
fn a_switch_whose_pause_was_not_saved_goes_on_through_try_again() {
    let test =
        concat!(module_path!(), "::a_switch_whose_pause_was_not_saved_goes_on_through_try_again");
    run_case(test, &[PhaseEnd::Exit], |_| {
        let mut app = TestApp::launch();
        let (transition_id, old_namespace) = switch_whose_pause_is_not_saved(&mut app);

        app.dispatch(CloudBackupManagerAction::RetryDriveAccountSwitchRecovery)
            .then_cannot(&[N::Enable]);
        assert_eq!(
            switch_phase(),
            Some(PersistedDriveAccountSwitchPhase::AwaitingSavedPasskeyConfirmation),
            "{}",
            app.describe()
        );
        assert_eq!(requested_steps(&app), vec![Step_::ReportState], "{}", app.describe());

        platform_reports(
            &mut app,
            Platform::State(DriveAccountSwitchPlatformState::Staged(transition_id)),
        )
        .then_cannot(&[N::Enable]);
        assert_eq!(requested_steps(&app), vec![]);
        assert_eq!(
            paused_switch_controls(&app),
            Some(vec![CloudBackupSavedPasskeyControl::Confirm]),
            "{}",
            app.describe()
        );

        app.passkey.inner.set_authenticate_result(Ok(vec![7; 32]));
        app.dispatch(CloudBackupManagerAction::ConfirmSavedPasskey).then_cannot(&[N::Enable]);
        platform_finishes_the_switch(&mut app, transition_id, &old_namespace);
        assert_eq!(app.passkey.inner.create_count(), 1);
    });
}

/// Dispatch still admits Confirm Saved Passkey for the paused run while the switch shows its
/// recovery, as from a confirmation left open; it saves the pause first and goes on as the
/// switch's run instead of finding no paused switch
#[test]
fn a_confirm_goes_on_from_a_switch_pause_that_was_not_saved() {
    let test =
        concat!(module_path!(), "::a_confirm_goes_on_from_a_switch_pause_that_was_not_saved");
    run_case(test, &[PhaseEnd::Exit], |_| {
        let mut app = TestApp::launch();
        let (transition_id, old_namespace) = switch_whose_pause_is_not_saved(&mut app);
        assert_eq!(
            app.admission_of(UserIntent::ConfirmSavedPasskey),
            Admission::Accepted,
            "{}",
            app.describe()
        );

        app.passkey.inner.set_authenticate_result(Ok(vec![7; 32]));
        app.dispatch(CloudBackupManagerAction::ConfirmSavedPasskey).then_cannot(&[N::Enable]);
        platform_finishes_the_switch(&mut app, transition_id, &old_namespace);
        assert_eq!(app.passkey.inner.create_count(), 1);
    });
}

/// Scripts the new account's next master key upload to fail, as when it is offline
fn fail_the_next_master_key_upload(app: &mut TestApp) {
    app.event("script the new account's master key upload to fail", |app| {
        let error = CloudStorageError::NotAvailable("the new account is offline".into());
        app.cloud.script_next(CloudCall::UploadMasterKey, CloudFault::Fail(error));
    })
    .then_cannot(&[N::Enable]);
}

/// The switch's reinitialize stopped on its saved passkey, with Confirm as its way on
fn assert_switch_paused_with_its_confirm(app: &TestApp) {
    assert!(!app.runs_operation(), "{}", app.describe());
    assert_eq!(
        switch_phase(),
        Some(PersistedDriveAccountSwitchPhase::AwaitingSavedPasskeyConfirmation),
        "{}",
        app.describe()
    );
    assert_eq!(
        paused_switch_controls(app),
        Some(vec![CloudBackupSavedPasskeyControl::Confirm]),
        "{}",
        app.describe()
    );
    assert_eq!(
        availability(app, A::ConfirmSavedPasskey),
        Some(CloudBackupActionAvailability::Available)
    );
}

/// Model seed 54182536095460878, minimized: a switch killed while its reinitialize uploads, whose
/// upload then fails after the relaunch and again on Try Again, is not a dead end
///
/// The relaunch restores the registered passkey from the pending-enable journal. A failed upload
/// keeps no material in memory, but the journal stays, so each Try Again takes its pause up
/// again instead of staging a second backup the journal refuses. Confirm then finishes the
/// switch under the same passkey and leaves no journal behind
#[test]
fn a_switch_killed_during_its_reinitialize_upload_recovers_through_try_again() {
    let test = concat!(
        module_path!(),
        "::a_switch_killed_during_its_reinitialize_upload_recovers_through_try_again"
    );
    run_case(test, &[PhaseEnd::KillAt("switch-uploading"), PhaseEnd::Exit], |phase| {
        if phase == 0 {
            let mut app = TestApp::launch();
            FakeAccount::new(1, 0).seed(&app);
            load_detail(&mut app);

            // no passkey is discoverable, so the new account's reinitialize registers one
            app.passkey.inner.set_discover_result(Err(PasskeyError::NoCredentialFound));
            app.passkey.inner.set_create_result(Ok(vec![4, 5, 6]));
            switch_to_a_new_account(&mut app);
            app.advance_clock(PASSKEY_AVAILABILITY_WAIT).then_cannot(&[N::Enable]);
            assert_switch_paused_with_its_confirm(&app);

            let upload = Hold::default();
            app.cloud.script_next(CloudCall::UploadMasterKey, CloudFault::Hold(upload.clone()));
            app.passkey.inner.set_authenticate_result(Ok(vec![7; 32]));
            app.dispatch(CloudBackupManagerAction::ConfirmSavedPasskey).then_cannot(&[N::Enable]);
            app.block_on("the new account's master key upload to be held", upload.entered());

            checkpoint("switch-uploading");
            unreachable!("the parent kills this process while the switch uploads");
        }

        let mut app = TestApp::launch();
        let old_namespace = keychain_namespace().expect("the kill kept the previous namespace");
        let transition_id =
            persisted().drive_account_switch().expect("the switch survived").transition_id.value();
        platform_reports(
            &mut app,
            Platform::State(DriveAccountSwitchPlatformState::Staged(transition_id)),
        )
        .then_cannot(&[N::Enable]);
        assert_switch_paused_with_its_confirm(&app);

        // the confirmed upload fails, and so does the next Confirm's, after Try Again's pause
        app.passkey.inner.set_authenticate_result(Ok(vec![7; 32]));
        fail_the_next_master_key_upload(&mut app);
        app.dispatch(CloudBackupManagerAction::ConfirmSavedPasskey).then_cannot(&[N::Enable]);
        app.dispatch(CloudBackupManagerAction::RetryDriveAccountSwitchRecovery)
            .then_cannot(&[N::Enable]);
        assert_switch_paused_with_its_confirm(&app);
        fail_the_next_master_key_upload(&mut app);
        app.dispatch(CloudBackupManagerAction::ConfirmSavedPasskey).then_cannot(&[N::Enable]);
        assert_eq!(
            switch_phase(),
            Some(PersistedDriveAccountSwitchPhase::AwaitingReinitializationRetry),
            "{}",
            app.describe()
        );
        assert!(pending_enable_journal().is_some(), "{}", app.describe());

        // Try Again takes up the journal's pause, and Confirm finishes the switch with it
        app.dispatch(CloudBackupManagerAction::RetryDriveAccountSwitchRecovery)
            .then_cannot(&[N::Enable]);
        assert_switch_paused_with_its_confirm(&app);

        app.dispatch(CloudBackupManagerAction::ConfirmSavedPasskey).then_cannot(&[N::Enable]);
        platform_finishes_the_switch(&mut app, transition_id, &old_namespace);
        assert_eq!(app.passkey.inner.create_count(), 0, "a second passkey was registered");
        assert_eq!(pending_enable_journal(), None, "{}", app.describe());
    });
}

/// Model seed 777791, minimized: a reset that fails before its wipe while a reinitialize waits
/// for its saved passkey gives that pause back
///
/// The reset drops the paused reinitialize from memory, but its journal stays, and the abandoned
/// reset takes it up again as a relaunch does. The pause holds the backup, so a Google Drive
/// account switch can't stage a second backup beside it, and Confirm finishes the reinitialize
/// with the passkey it registered
#[test]
fn a_failed_reset_gives_back_the_paused_reinitialize() {
    let test = concat!(module_path!(), "::a_failed_reset_gives_back_the_paused_reinitialize");
    run_case(test, &[PhaseEnd::Exit], |_| {
        let mut app = TestApp::launch();
        let old_namespace = FakeAccount::new(1, 0).seed(&app);
        load_detail(&mut app);

        // no passkey is discoverable, so the reinitialize registers one
        app.passkey.inner.set_discover_result(Err(PasskeyError::NoCredentialFound));
        app.passkey.inner.set_create_result(Ok(vec![4, 5, 6]));
        app.dispatch(CloudBackupManagerAction::ReinitializeBackup).then_cannot(&[N::Enable]);
        app.advance_clock(PASSKEY_AVAILABILITY_WAIT).then_cannot(&[N::Enable]);
        assert_reinitialize_paused(&app);

        abandon_a_reset(&mut app);
        assert_reinitialize_paused(&app);
        assert!(pending_enable_journal().is_some(), "{}", app.describe());
        assert_eq!(
            app.admission_of(UserIntent::SwitchDriveAccount),
            Admission::Rejected(Rejection::OperationRunning(Op::ReinitializeBackup))
        );

        app.passkey.inner.set_authenticate_result(Ok(vec![7; 32]));
        app.dispatch(CloudBackupManagerAction::ConfirmSavedPasskey)
            .then_can(&[N::TurnOff])
            .then_cannot(&[N::Enable]);
        assert_ne!(keychain_namespace(), Some(old_namespace), "{}", app.describe());
        assert_eq!(app.passkey.inner.create_count(), 1, "a second passkey was registered");
        assert_eq!(pending_enable_journal(), None, "{}", app.describe());
    });
}

/// Model seed 777818, minimized: a local reset that completes ends a switch whose reinitialize
/// failed, so the screen shows Cloud Backup off instead of the switch's recovery and its Try Again
///
/// The wipe clears the switch's record with the rest of the lifecycle; the platform's half is
/// reconciled from its next state report, which finalizes or rolls back a switch Rust no longer
/// records
#[test]
fn a_completed_reset_ends_a_switch_whose_step_failed() {
    let test = concat!(module_path!(), "::a_completed_reset_ends_a_switch_whose_step_failed");
    run_case(test, &[PhaseEnd::Exit], |_| {
        let mut app = TestApp::launch();
        switch_whose_reinitialize_fails(&mut app);

        let manager = app.manager().clone();
        let permit = app
            .block_on("the reset to be ready", async move { manager.prepare_local_reset().await })
            .expect("reset permit");
        app.event("the wipe completes", |app| finish_wipe(app, permit))
            .then_can(&[N::Enable])
            .then_cannot(&[N::TurnOff]);

        assert_eq!(switch_phase(), None);
        assert_eq!(switch_stage(&app), None, "{}", app.describe());
        assert_eq!(app.manager().published_screen().body, CloudBackupScreenBody::Disabled);
        assert_eq!(
            app.admission_of(UserIntent::RetryDriveAccountSwitchRecovery),
            Admission::Rejected(Rejection::NothingToContinue)
        );
    });
}

/// Greptile P1 on #1113: a local reset that completes while the platform is still working on a
/// switch step ends the switch, so the platform's late answer for it finds no step awaited and
/// moves nothing
///
/// Before, a late failure republished the switch's recovery over the reset, which hid Enable and
/// Restore until a retry or a relaunch
fn a_late_answer_for_a_switch_a_reset_ended_moves_nothing(
    app: &mut TestApp,
    late_answer: impl FnOnce(u64) -> Platform,
    transition_id: u64,
) {
    let manager = app.manager().clone();
    let permit = app
        .block_on("the reset to be ready", async move { manager.prepare_local_reset().await })
        .expect("reset permit");
    app.event("the wipe completes", |app| finish_wipe(app, permit))
        .then_can(&[N::Enable, N::RestoreFromCloudBackup])
        .then_cannot(&[N::TurnOff]);
    requested_steps(app);

    platform_reports(app, late_answer(transition_id))
        .then_can(&[N::Enable, N::RestoreFromCloudBackup])
        .then_cannot(&[N::TurnOff]);
    assert_eq!(app.manager().state.read().drive_account_switch_recovery(), None);
    assert_eq!(switch_stage(app), None, "{}", app.describe());
    assert_eq!(app.manager().published_screen().body, CloudBackupScreenBody::Disabled);
    assert_eq!(requested_steps(app), vec![], "{}", app.describe());
}

/// A seeded backup whose switch has the account picker open, with the transition
fn switch_choosing_an_account(app: &mut TestApp) -> u64 {
    FakeAccount::new(1, 0).seed(app);
    load_detail(app);

    app.dispatch(CloudBackupManagerAction::SwitchDriveAccount).then_cannot(&[N::Enable]);
    let transition_id =
        persisted().drive_account_switch().expect("the switch is recorded").transition_id.value();
    assert_eq!(requested_steps(app), vec![Step_::SelectAccount(transition_id)]);

    transition_id
}

/// A seeded backup whose switch reinitialized in the new account and asked the platform to
/// commit it, with the transition
fn switch_committing_its_account(app: &mut TestApp) -> u64 {
    FakeAccount::new(1, 0).seed(app);
    load_detail(app);

    // the new account's backup is created under a passkey the platform already has
    app.passkey.inner.set_discover_result(Ok(DiscoveredPasskeyResult {
        prf_output: vec![7; 32],
        credential_id: vec![1, 2, 3],
    }));

    let transition_id = switch_to_a_new_account(app);
    assert_eq!(requested_steps(app), vec![Step_::Commit(transition_id)], "{}", app.describe());

    transition_id
}

#[test]
fn a_selection_answered_after_a_completed_reset_moves_nothing() {
    let test =
        concat!(module_path!(), "::a_selection_answered_after_a_completed_reset_moves_nothing");
    run_case(test, &[PhaseEnd::Exit], |_| {
        let mut app = TestApp::launch();
        let transition_id = switch_choosing_an_account(&mut app);

        a_late_answer_for_a_switch_a_reset_ended_moves_nothing(
            &mut app,
            Platform::AccountUnchanged,
            transition_id,
        );
    });
}

#[test]
fn a_commit_failed_after_a_completed_reset_moves_nothing() {
    let test = concat!(module_path!(), "::a_commit_failed_after_a_completed_reset_moves_nothing");
    run_case(test, &[PhaseEnd::Exit], |_| {
        let mut app = TestApp::launch();
        let transition_id = switch_committing_its_account(&mut app);

        a_late_answer_for_a_switch_a_reset_ended_moves_nothing(
            &mut app,
            Platform::CommitFailed,
            transition_id,
        );
    });
}

#[test]
fn a_finalize_failed_after_a_completed_reset_moves_nothing() {
    let test = concat!(module_path!(), "::a_finalize_failed_after_a_completed_reset_moves_nothing");
    run_case(test, &[PhaseEnd::Exit], |_| {
        let mut app = TestApp::launch();
        let transition_id = switch_committing_its_account(&mut app);
        platform_reports(&mut app, Platform::Committed(transition_id)).then_cannot(&[N::Enable]);
        assert_eq!(requested_steps(&app), vec![Step_::Finalize(transition_id)]);

        a_late_answer_for_a_switch_a_reset_ended_moves_nothing(
            &mut app,
            Platform::FinalizeFailed,
            transition_id,
        );
    });
}

#[test]
fn a_rollback_failed_after_a_completed_reset_moves_nothing() {
    let test = concat!(module_path!(), "::a_rollback_failed_after_a_completed_reset_moves_nothing");
    run_case(test, &[PhaseEnd::Exit], |_| {
        let mut app = TestApp::launch();
        let transition_id = switch_choosing_an_account(&mut app);
        platform_reports(&mut app, Platform::AccountUnchanged(transition_id))
            .then_cannot(&[N::Enable]);
        assert_eq!(requested_steps(&app), vec![Step_::Rollback(transition_id)]);

        a_late_answer_for_a_switch_a_reset_ended_moves_nothing(
            &mut app,
            Platform::RollbackFailed,
            transition_id,
        );
    });
}

/// Model seed 20261511: after a relaunch the launch report asks for a rollback, and the user's
/// Try Again asks for a state report before Android answers it. The rollback's answer still ends
/// the switch
///
/// Before, the state report took the rollback's place as the awaited step, so Android's
/// RolledBack was dropped, and each later Try Again did the same to the rollback the reconcile
/// asked for again, leaving the switch stuck
#[test]
fn a_rollback_answered_after_try_again_asked_for_a_report_ends_the_switch() {
    let test = concat!(
        module_path!(),
        "::a_rollback_answered_after_try_again_asked_for_a_report_ends_the_switch"
    );
    run_case(test, &[PhaseEnd::Exit, PhaseEnd::Exit], |phase| {
        if phase == 0 {
            let mut app = TestApp::launch();
            switch_choosing_an_account(&mut app);
            return;
        }

        // the account picker didn't survive the process, so Android has nothing staged
        let mut app = TestApp::launch();
        let transition_id = persisted()
            .drive_account_switch()
            .expect("the switch is recorded")
            .transition_id
            .value();
        platform_reports(&mut app, Platform::State(DriveAccountSwitchPlatformState::NoTransition))
            .then_cannot(&[N::Enable]);
        assert_eq!(switch_phase(), Some(PersistedDriveAccountSwitchPhase::AwaitingAccountRollback));
        assert_eq!(
            availability(&app, A::RetryDriveAccountSwitchRecovery),
            Some(CloudBackupActionAvailability::Available),
            "{}",
            app.describe()
        );

        app.dispatch(CloudBackupManagerAction::RetryDriveAccountSwitchRecovery)
            .then_cannot(&[N::Enable]);
        assert_eq!(
            requested_steps(&app),
            vec![Step_::Rollback(transition_id), Step_::ReportState],
            "{}",
            app.describe()
        );

        // Android answers the two steps in the order Rust asked for them
        platform_reports(&mut app, Platform::RolledBack(transition_id)).then_cannot(&[N::Enable]);
        assert_eq!(switch_phase(), None, "{}", app.describe());
        assert_eq!(switch_stage(&app), None, "{}", app.describe());

        platform_reports(&mut app, Platform::State(DriveAccountSwitchPlatformState::NoTransition))
            .then_cannot(&[N::Enable]);
        assert_eq!(switch_phase(), None, "{}", app.describe());
        assert_eq!(requested_steps(&app), vec![], "{}", app.describe());
    });
}

/// The configured backup's reinitialize waits on its saved passkey, with Confirm and Discard
fn assert_reinitialize_paused(app: &TestApp) {
    let paused = app
        .saved_passkey_confirmation()
        .unwrap_or_else(|| panic!("the reinitialize is not paused\n{}", app.describe()));
    assert_eq!(paused.lifecycle, CloudBackupSavedPasskeyLifecycle::Reinitialize);
    assert_eq!(
        paused.controls,
        vec![CloudBackupSavedPasskeyControl::Confirm, CloudBackupSavedPasskeyControl::Discard]
    );
}

/// A reset that fails before its wipe, after it dropped what Cloud Backup kept in memory
fn abandon_a_reset(app: &mut TestApp) {
    let manager = app.manager().clone();
    let permit = app
        .block_on("the reset to be ready", async move { manager.prepare_local_reset().await })
        .expect("reset permit");
    abandon_reset_permit(app, permit);
}

fn abandon_reset_permit(app: &mut TestApp, permit: CloudBackupResetPermit) {
    app.event("the wipe fails before its commit", |app| {
        app.block_on("the reset to end", permit.resume_after_failed_reset())
            .expect("abandon the reset");
    })
    .then_cannot(&[N::Enable]);
}

fn active_master_key() -> MasterKey {
    Cspp::new(Keychain::global().clone())
        .load_master_key_from_store()
        .expect("read the active master key")
        .expect("an active master key")
}

/// Model review sequence: a reset that begins after a switch's confirmed reinitialize started its
/// local promotion, and before its finalization recorded the durable completion
///
/// The reset cancels the finalization before it can commit or roll back the promotion, and the
/// write lane drops the finalization's late completion. A reset that then fails before its wipe
/// takes the journal up as a relaunch does: with no durable completion the promotion rolls back,
/// the previous account's backup is active again, and the switch waits on its saved passkey.
/// Confirm then finishes the switch from the rolled-back stage, with no relaunch and no second
/// passkey
#[test]
fn a_switch_whose_promotion_a_failed_reset_cut_off_finishes_through_confirm() {
    let test = concat!(
        module_path!(),
        "::a_switch_whose_promotion_a_failed_reset_cut_off_finishes_through_confirm"
    );
    run_case(test, &[PhaseEnd::Exit], |_| {
        let mut app = TestApp::launch();
        let old_namespace = FakeAccount::new(1, 0).seed(&app);
        load_detail(&mut app);
        let old_key = active_master_key();

        // no passkey is discoverable, so the new account's reinitialize registers one
        app.passkey.inner.set_discover_result(Err(PasskeyError::NoCredentialFound));
        app.passkey.inner.set_create_result(Ok(vec![4, 5, 6]));
        let transition_id = switch_to_a_new_account(&mut app);
        app.advance_clock(PASSKEY_AVAILABILITY_WAIT).then_cannot(&[N::Enable]);
        assert_switch_paused_with_its_confirm(&app);

        // the confirmed reinitialize uploads, promotes its stage, and holds on the listing its
        // finalization counts the uploaded wallets with
        let listing = Hold::default();
        app.cloud.script_next(CloudCall::ListWalletFiles, CloudFault::Hold(listing.clone()));
        app.passkey.inner.set_authenticate_result(Ok(vec![7; 32]));
        app.dispatch(CloudBackupManagerAction::ConfirmSavedPasskey).then_cannot(&[N::Enable]);
        app.block_on("the finalization's listing to be held", listing.entered());
        let journal = pending_enable_journal().expect("the switch's journal");
        assert!(
            matches!(journal.phase(), PendingEnableJournalPhase::LocalPromotionStarted(_)),
            "{journal:?}\n{}",
            app.describe()
        );
        let new_namespace = journal.namespace_id().to_owned();
        assert_eq!(keychain_namespace().as_deref(), Some(new_namespace.as_str()));

        // a reset cancels the finalization, and its wipe then fails before its commit
        let manager = app.manager().clone();
        let reset = cove_tokio::task::spawn(async move { manager.prepare_local_reset().await });
        app.event("a full wipe begins", |_| {}).then_cannot(&[N::Enable]);
        app.event("release the held listing", |_| listing.release()).then_cannot(&[N::Enable]);
        let permit = app
            .block_on("the reset to be ready", reset)
            .expect("reset task")
            .expect("reset permit");
        abandon_reset_permit(&mut app, permit);

        // with no durable completion the promotion rolled back to its stage, kept for a retry
        let journal = pending_enable_journal().expect("the switch's journal");
        assert!(
            matches!(journal.phase(), PendingEnableJournalPhase::RemoteWritesStarted(_)),
            "{journal:?}\n{}",
            app.describe()
        );
        assert_eq!(journal.namespace_id(), new_namespace);
        assert_eq!(keychain_namespace().as_deref(), Some(old_namespace.as_str()));
        assert_eq!(active_master_key().as_bytes(), old_key.as_bytes());
        assert!(
            persisted()
                .pending_verification_completion()
                .is_none_or(|completion| { completion.namespace_id() != new_namespace }),
            "{}",
            app.describe()
        );
        assert_switch_paused_with_its_confirm(&app);

        // Confirm finishes the switch from that stage, with no relaunch and no second passkey
        app.dispatch(CloudBackupManagerAction::ConfirmSavedPasskey).then_cannot(&[N::Enable]);
        platform_finishes_the_switch(&mut app, transition_id, &old_namespace);
        assert_eq!(keychain_namespace().as_deref(), Some(new_namespace.as_str()));
        assert_ne!(active_master_key().as_bytes(), old_key.as_bytes());
        assert_eq!(app.passkey.inner.create_count(), 1, "a second passkey was registered");
        assert_eq!(pending_enable_journal(), None, "{}", app.describe());
    });
}

fn pending_enable_journal() -> Option<PendingEnableJournal> {
    CloudBackupKeychain::global().load_pending_enable_journal().expect("the journal loads")
}

/// The user's Switch to a new account, whose reinitialize registered a passkey and waits for its
/// Confirm, with the transition and the namespace the device was on before the switch
fn switch_paused_on_its_reinitialize(app: &mut TestApp) -> (u64, String) {
    let old_namespace = FakeAccount::new(1, 0).seed(app);
    load_detail(app);

    // no passkey is discoverable, so the new account's reinitialize registers one
    app.passkey.inner.set_discover_result(Err(PasskeyError::NoCredentialFound));
    app.passkey.inner.set_create_result(Ok(vec![4, 5, 6]));
    let transition_id = switch_to_a_new_account(app);
    app.advance_clock(PASSKEY_AVAILABILITY_WAIT).then_cannot(&[N::Enable]);
    assert_switch_paused_with_its_confirm(app);
    app.passkey.inner.set_authenticate_result(Ok(vec![7; 32]));

    (transition_id, old_namespace)
}

/// The new namespace whose durable completion the switch's reinitialize saved, while the switch
/// still records its reinitialize and the journal its started promotion
fn assert_switch_completion_saved(app: &TestApp) -> String {
    let journal = pending_enable_journal().expect("the switch's journal");
    assert!(
        matches!(journal.phase(), PendingEnableJournalPhase::LocalPromotionStarted(_)),
        "{journal:?}\n{}",
        app.describe()
    );
    let new_namespace = journal.namespace_id().to_owned();
    assert!(
        persisted()
            .pending_verification_completion()
            .is_some_and(|completion| completion.namespace_id() == new_namespace),
        "{}",
        app.describe()
    );
    assert_eq!(switch_phase(), Some(PersistedDriveAccountSwitchPhase::Reinitializing));
    assert_eq!(keychain_namespace().as_deref(), Some(new_namespace.as_str()));
    new_namespace
}

/// The recovery finished the switch's promotion into `new_namespace` under `new_key`, and the
/// switch, moved on to wait for the platform's commit, asked for it
fn assert_switch_reinitialized_into(
    app: &TestApp,
    transition_id: u64,
    new_namespace: &str,
    new_key: &MasterKey,
) {
    assert_eq!(keychain_namespace().as_deref(), Some(new_namespace), "{}", app.describe());
    assert_eq!(active_master_key().as_bytes(), new_key.as_bytes());
    assert!(
        persisted()
            .pending_verification_completion()
            .is_some_and(|completion| completion.namespace_id() == new_namespace),
        "{}",
        app.describe()
    );
    assert_eq!(pending_enable_journal(), None, "{}", app.describe());
    assert_eq!(
        switch_phase(),
        Some(PersistedDriveAccountSwitchPhase::AwaitingAccountCommitSucceeded),
        "{}",
        app.describe()
    );
    assert_eq!(requested_steps(app), vec![Step_::Commit(transition_id)], "{}", app.describe());
}

/// The platform's state report goes on to the commit, with no new key and no passkey
fn a_state_report_commits_without_a_new_reinitialize(
    app: &mut TestApp,
    transition_id: u64,
    new_key: &MasterKey,
) {
    let created = app.passkey.inner.create_count();
    let authenticated = app.passkey.inner.authenticate_count();
    platform_reports(app, Platform::State(DriveAccountSwitchPlatformState::Staged(transition_id)))
        .then_cannot(&[N::Enable]);
    assert!(!app.runs_operation(), "{}", app.describe());
    assert_eq!(active_master_key().as_bytes(), new_key.as_bytes());
    assert_eq!(app.passkey.inner.create_count(), created, "a second passkey was registered");
    assert_eq!(app.passkey.inner.authenticate_count(), authenticated);
}

/// Model review sequence: a reset that begins after a switch's reinitialize saved its durable
/// completion, and before its finalization's callback saved the switch's next phase
///
/// The reset cancels the callback, and then fails before its wipe. Its recovery finds the
/// promotion's durable completion, so it commits the promotion and hands the switch its
/// finished reinitialize: the switch moves on to the platform's commit, and the platform's
/// next report doesn't run a second reinitialize beside the backup that exists
#[test]
fn a_switch_promotion_a_failed_reset_found_saved_goes_on_to_its_commit() {
    let test = concat!(
        module_path!(),
        "::a_switch_promotion_a_failed_reset_found_saved_goes_on_to_its_commit"
    );
    run_case(test, &[PhaseEnd::Exit], |_| {
        let mut app = TestApp::launch();
        let (transition_id, old_namespace) = switch_paused_on_its_reinitialize(&mut app);

        // the reset starts as the record takes the new namespace's completion, so it reaches the
        // supervisor before the write lane hands the finalization its result, and cancels the
        // finalization before its callback
        let reset = Arc::new(Mutex::new(None));
        let started = reset.clone();
        let manager = app.manager().clone();
        let previous = old_namespace.clone();
        Database::global().cloud_backup_state.fail_next_write_where(move |state| {
            let saved = state.drive_account_switch().is_some()
                && state
                    .pending_verification_completion()
                    .is_some_and(|completion| completion.namespace_id() != previous);
            let mut started = started.lock();
            if saved && started.is_none() {
                let manager = manager.clone();
                *started =
                    Some(cove_tokio::task::spawn(
                        async move { manager.prepare_local_reset().await },
                    ));
            }

            // the write itself goes through
            false
        });
        app.dispatch(CloudBackupManagerAction::ConfirmSavedPasskey).then_cannot(&[N::Enable]);
        Database::global().cloud_backup_state.fail_next_write_where(|_| false);

        let reset = reset.lock().take().expect("the reset started at the completion's save");
        let new_namespace = assert_switch_completion_saved(&app);
        let new_key = active_master_key();
        let permit = app
            .block_on("the reset to be ready", reset)
            .expect("reset task")
            .expect("reset permit");
        abandon_reset_permit(&mut app, permit);

        assert_switch_reinitialized_into(&app, transition_id, &new_namespace, &new_key);
        a_state_report_commits_without_a_new_reinitialize(&mut app, transition_id, &new_key);
        platform_finishes_the_switch(&mut app, transition_id, &old_namespace);
        assert_eq!(keychain_namespace().as_deref(), Some(new_namespace.as_str()));
        assert_eq!(app.passkey.inner.create_count(), 1, "a second passkey was registered");
    });
}

/// The same promotion, saved and then cut off by a kill instead of a reset: the relaunch's
/// recovery hands the switch its finished reinitialize the same way
#[test]
fn a_switch_promotion_saved_before_a_kill_goes_on_to_its_commit() {
    let test =
        concat!(module_path!(), "::a_switch_promotion_saved_before_a_kill_goes_on_to_its_commit");
    run_case(test, &[PhaseEnd::KillAt("switch-completion-saved"), PhaseEnd::Exit], |phase| {
        if phase == 0 {
            let mut app = TestApp::launch();
            switch_paused_on_its_reinitialize(&mut app);

            // the upload reads the wallet's key too, so the marking's read is held only once
            // the finalization's listing, after the upload, is. The held read blocks the
            // harness's one runtime thread, so the kill comes with nothing else run
            let listing = Hold::default();
            app.cloud.script_next(CloudCall::ListWalletFiles, CloudFault::Hold(listing.clone()));
            app.dispatch(CloudBackupManagerAction::ConfirmSavedPasskey).then_cannot(&[N::Enable]);
            app.block_on("the finalization's listing to be held", listing.entered());

            let marking = Hold::default();
            app.keychain
                .hold_next_read_where(|key| key.ends_with("::wallet_xpub"), marking.clone());
            listing.release();
            app.block_on("the uploaded wallet's marking to be held", marking.entered());
            assert_switch_completion_saved(&app);
            checkpoint("switch-completion-saved");
            unreachable!("the parent kills this process before the finalization's callback");
        }

        // the promotion moved the active key and namespace to the new backup before the kill
        open_durable_state();
        let new_key = active_master_key();
        let new_namespace = keychain_namespace().expect("the promoted namespace");
        let transition_id =
            persisted().drive_account_switch().expect("the switch survived").transition_id.value();

        let mut app = TestApp::launch();
        assert_switch_reinitialized_into(&app, transition_id, &new_namespace, &new_key);
        a_state_report_commits_without_a_new_reinitialize(&mut app, transition_id, &new_key);
        assert_eq!(app.passkey.inner.create_count(), 0, "a second passkey was registered");
        assert_eq!(app.passkey.inner.authenticate_count(), 0);

        assert_eq!(requested_steps(&app), vec![Step_::Commit(transition_id)], "{}", app.describe());
        platform_reports(&mut app, Platform::Committed(transition_id)).then_cannot(&[N::Enable]);
        platform_reports(&mut app, Platform::Finalized(transition_id))
            .then_can(&[N::TurnOff])
            .then_cannot(&[N::Enable]);
        assert_eq!(keychain_namespace().as_deref(), Some(new_namespace.as_str()));
        assert_eq!(switch_phase(), None);
    });
}

/// What a switch's promotion left once its journal cleanup failed: the new namespace, active
/// key, credential and durable completion the recovery has to keep
struct StoppedPromotion {
    namespace: String,
    key: MasterKey,
    credential_id: Option<Vec<u8>>,
    completion: Option<PersistedPendingVerificationCompletion>,
}

impl StoppedPromotion {
    fn assert_kept(&self) {
        assert_eq!(keychain_namespace().as_deref(), Some(self.namespace.as_str()));
        assert_eq!(active_master_key().as_bytes(), self.key.as_bytes());
        assert_eq!(credential_id(), self.credential_id);
        assert_eq!(persisted().pending_verification_completion().cloned(), self.completion);
    }
}

fn credential_id() -> Option<Vec<u8>> {
    CloudBackupKeychain::global().load_credential_id().expect("the credential loads")
}

/// The switch's Confirm uploads and commits its promotion, saving the durable completion, but
/// the keychain refuses the journal's final delete once
///
/// That is not a failed reinitialize: the switch stays Reinitializing with its claim given up,
/// and the started promotion is the journal recovery's, so the screen offers Try Again for it
fn switch_promotion_stopped_at_its_journal_cleanup(app: &mut TestApp) -> StoppedPromotion {
    app.keychain.fail_next_delete_of(CSPP_PENDING_ENABLE_JOURNAL_KEY);
    app.dispatch(CloudBackupManagerAction::ConfirmSavedPasskey)
        .then_can(&[N::RetryLoad])
        .then_cannot(&[N::Enable, N::TurnOff]);

    let namespace = assert_switch_completion_saved(app);
    assert!(!app.runs_operation(), "{}", app.describe());
    assert_eq!(app.manager().published_screen().body, CloudBackupScreenBody::LoadFailed);
    assert_eq!(requested_steps(app), vec![], "{}", app.describe());
    StoppedPromotion {
        namespace,
        key: active_master_key(),
        credential_id: credential_id(),
        completion: persisted().pending_verification_completion().cloned(),
    }
}

/// While the promotion waits for its recovery, the platform's state report starts nothing
fn a_state_report_waits_for_the_recovery(app: &mut TestApp, transition_id: u64) {
    platform_reports(app, Platform::State(DriveAccountSwitchPlatformState::Staged(transition_id)))
        .then_can(&[N::RetryLoad])
        .then_cannot(&[N::Enable]);
    assert!(!app.runs_operation(), "{}", app.describe());
    assert_eq!(switch_phase(), Some(PersistedDriveAccountSwitchPhase::Reinitializing));
    assert_eq!(requested_steps(app), vec![], "{}", app.describe());
}

/// Try Again runs the journal recovery, which finishes the promotion with what it kept and hands
/// the switch on to the platform's commit, with no new key, passkey or master key upload
fn try_again_hands_the_promotion_to_the_switch(
    app: &mut TestApp,
    transition_id: u64,
    stopped: &StoppedPromotion,
) {
    let created = app.passkey.inner.create_count();
    let authenticated = app.passkey.inner.authenticate_count();
    let master_uploads = app.cloud.count(CloudCall::UploadMasterKey);

    app.dispatch(CloudBackupManagerAction::RetryLoad).then_cannot(&[N::RetryLoad, N::Enable]);
    assert_switch_reinitialized_into(app, transition_id, &stopped.namespace, &stopped.key);
    stopped.assert_kept();
    a_state_report_commits_without_a_new_reinitialize(app, transition_id, &stopped.key);
    assert_eq!(app.passkey.inner.create_count(), created, "a second passkey was registered");
    assert_eq!(app.passkey.inner.authenticate_count(), authenticated);
    assert_eq!(app.cloud.count(CloudCall::UploadMasterKey), master_uploads);
}

/// Model review sequence: a switch's promotion commits and saves its durable completion, and
/// only the journal's final delete fails
///
/// Its reinitialize didn't fail, so the switch neither offers it again nor reports it failed.
/// Once the keychain works again, the screen's Try Again finishes the promotion through the
/// journal recovery and the switch goes on to the platform's commit
#[test]
fn a_switch_promotion_whose_journal_cleanup_failed_goes_on_to_its_commit() {
    let test = concat!(
        module_path!(),
        "::a_switch_promotion_whose_journal_cleanup_failed_goes_on_to_its_commit"
    );
    run_case(test, &[PhaseEnd::Exit], |_| {
        let mut app = TestApp::launch();
        let (transition_id, old_namespace) = switch_paused_on_its_reinitialize(&mut app);
        let stopped = switch_promotion_stopped_at_its_journal_cleanup(&mut app);
        a_state_report_waits_for_the_recovery(&mut app, transition_id);

        // the keychain refused only that one delete
        try_again_hands_the_promotion_to_the_switch(&mut app, transition_id, &stopped);
        platform_finishes_the_switch(&mut app, transition_id, &old_namespace);
        assert_eq!(app.passkey.inner.create_count(), 1, "a second passkey was registered");
        assert_eq!(app.cloud.count(CloudCall::UploadMasterKey), 1);
    });
}

/// The same stopped promotion after a relaunch whose keychain can't read the journal at first:
/// the launch shows Try Again, and once reads work it hands the switch on the same way
#[test]
fn a_switch_promotion_whose_journal_cleanup_failed_goes_on_after_a_relaunch() {
    let test = concat!(
        module_path!(),
        "::a_switch_promotion_whose_journal_cleanup_failed_goes_on_after_a_relaunch"
    );
    run_case(test, &[PhaseEnd::Exit, PhaseEnd::Exit], |phase| {
        if phase == 0 {
            let mut app = TestApp::launch();
            switch_paused_on_its_reinitialize(&mut app);
            switch_promotion_stopped_at_its_journal_cleanup(&mut app);
            return;
        }

        open_durable_state();
        let stopped = StoppedPromotion {
            namespace: keychain_namespace().expect("the promoted namespace"),
            key: active_master_key(),
            credential_id: credential_id(),
            completion: persisted().pending_verification_completion().cloned(),
        };
        let transition_id =
            persisted().drive_account_switch().expect("the switch survived").transition_id.value();

        launch_keychain().inner.fail_reads_for(CSPP_PENDING_ENABLE_JOURNAL_KEY);
        let mut app = TestApp::launch();
        app.current().then_can(&[N::RetryLoad]).then_cannot(&[N::Enable, N::TurnOff]);
        assert_eq!(app.manager().published_screen().body, CloudBackupScreenBody::LoadFailed);
        a_state_report_waits_for_the_recovery(&mut app, transition_id);

        app.keychain.inner.clear_read_failures();
        try_again_hands_the_promotion_to_the_switch(&mut app, transition_id, &stopped);
        assert_eq!(app.passkey.inner.create_count(), 0, "a second passkey was registered");
        assert_eq!(app.passkey.inner.authenticate_count(), 0);
        assert_eq!(app.cloud.count(CloudCall::UploadMasterKey), 0);
    });
}

/// The keychain item holding CSPP's active master-key encryption key, which every promotion and
/// rollback of the active key rewrites
const ACTIVE_ENCRYPTION_KEY_ITEM: &str = "cspp::v1::master_key_encryption_key_and_nonce";

fn staged_master_key() -> MasterKey {
    Cspp::new(Keychain::global().clone())
        .load_staged_master_key()
        .expect("read the staged master key")
        .expect("a staged master key")
}

/// Model review sequence: a switch's Confirm uploads backup N, and its promotion fails while it
/// starts, after the journal records it: the keychain refuses the active key's write, and the
/// rollback's write of the previous one too
///
/// That is not a failed reinitialize the switch could run again, since staging another backup
/// would meet the started promotion's journal. The switch stays Reinitializing, and once the
/// keychain works, the screen's Try Again runs the journal recovery: with no durable completion
/// it rolls the promotion back, and the switch is paused on N's passkey again, whose Confirm
/// finishes it under the same staged key and credential
#[test]
fn a_switch_promotion_that_fails_to_start_or_roll_back_pauses_again_on_try_again() {
    let test = concat!(
        module_path!(),
        "::a_switch_promotion_that_fails_to_start_or_roll_back_pauses_again_on_try_again"
    );
    run_case(test, &[PhaseEnd::Exit], |_| {
        let mut app = TestApp::launch();
        let (transition_id, old_namespace) = switch_paused_on_its_reinitialize(&mut app);
        let old_key = active_master_key();
        let old_credential = credential_id();
        let staged = staged_master_key();

        app.keychain.fail_saves_of(ACTIVE_ENCRYPTION_KEY_ITEM);
        app.dispatch(CloudBackupManagerAction::ConfirmSavedPasskey)
            .then_can(&[N::RetryLoad])
            .then_cannot(&[N::Enable, N::TurnOff]);
        let journal = pending_enable_journal().expect("the switch's journal");
        assert!(
            matches!(journal.phase(), PendingEnableJournalPhase::LocalPromotionStarted(_)),
            "{journal:?}\n{}",
            app.describe()
        );
        assert_eq!(journal.namespace_id(), staged.namespace_id());
        assert_eq!(switch_phase(), Some(PersistedDriveAccountSwitchPhase::Reinitializing));
        assert!(!app.runs_operation(), "{}", app.describe());
        assert_eq!(app.manager().published_screen().body, CloudBackupScreenBody::LoadFailed);
        a_state_report_waits_for_the_recovery(&mut app, transition_id);

        // the recovery is local: no passkey, key or upload
        app.keychain.clear_save_failures();
        let created = app.passkey.inner.create_count();
        let authenticated = app.passkey.inner.authenticate_count();
        let uploads =
            (app.cloud.count(CloudCall::UploadMasterKey), app.cloud.count(CloudCall::UploadWallet));
        app.dispatch(CloudBackupManagerAction::RetryLoad).then_cannot(&[N::RetryLoad, N::Enable]);
        assert_eq!(app.passkey.inner.create_count(), created);
        assert_eq!(app.passkey.inner.authenticate_count(), authenticated);
        assert_eq!(
            (app.cloud.count(CloudCall::UploadMasterKey), app.cloud.count(CloudCall::UploadWallet)),
            uploads
        );

        // the previous backup is active again, and N waits on its passkey as before
        assert_eq!(active_master_key().as_bytes(), old_key.as_bytes());
        assert_eq!(keychain_namespace().as_deref(), Some(old_namespace.as_str()));
        assert_eq!(credential_id(), old_credential);
        let journal = pending_enable_journal().expect("the switch's journal");
        assert!(
            matches!(journal.phase(), PendingEnableJournalPhase::RemoteWritesStarted(_)),
            "{journal:?}\n{}",
            app.describe()
        );
        assert_eq!(journal.namespace_id(), staged.namespace_id());
        assert_eq!(
            journal.passkey().map(|passkey| passkey.credential_id.clone()),
            Some(vec![4, 5, 6])
        );
        assert_eq!(staged_master_key().as_bytes(), staged.as_bytes());
        assert_switch_paused_with_its_confirm(&app);

        // Confirm asks for the same passkey once more and finishes the switch into N
        app.dispatch(CloudBackupManagerAction::ConfirmSavedPasskey).then_cannot(&[N::Enable]);
        assert_eq!(app.passkey.inner.create_count(), 1, "a second passkey was registered");
        assert_eq!(app.passkey.inner.authenticate_count(), authenticated + 1);
        assert_eq!(active_master_key().as_bytes(), staged.as_bytes());
        assert_eq!(credential_id(), Some(vec![4, 5, 6]));
        assert_eq!(pending_enable_journal(), None, "{}", app.describe());
        platform_finishes_the_switch(&mut app, transition_id, &old_namespace);
        assert_eq!(keychain_namespace().as_deref(), Some(staged.namespace_id().as_str()));
    });
}

/// A switch whose reinitialize recovers a backup the user's passkey opens rather than creating
/// one, since the device's local key is gone, and what a full rollback of that recovery restores
///
/// The harness has one cloud, so the account the switch moves to holds the configured backup
struct RecoveringSwitch {
    transition_id: u64,
    metadata: PendingEnableLocalMetadataSnapshot,
    recovered_key: MasterKey,
}

/// Seeds the backup the switch's reinitialize recovers, and the passkey that opens it; the
/// switch starts once `faults` are armed
fn switch_recovering_a_backup(
    app: &mut TestApp,
    faults: impl FnOnce(&mut TestApp),
) -> RecoveringSwitch {
    let namespace = FakeAccount::new(1, 0).seed(app);
    seed_verifiable_master_key(app, &namespace);
    load_detail(app);
    let recovered_key = active_master_key();
    CloudBackupKeychain::global().delete_master_key().expect("delete the local master key");
    let metadata = CloudBackupKeychain::global().snapshot_passkey_metadata().expect("metadata");

    faults(app);
    let transition_id = switch_to_a_new_account(app);
    RecoveringSwitch { transition_id, metadata, recovered_key }
}

/// The switch's recovered promotion stopped for the journal recovery: the switch still
/// reinitializes, nothing is running, and the screen offers Try Again for the recovery
fn assert_recovered_promotion_stopped(app: &mut TestApp, switch: &RecoveringSwitch) {
    let journal = assert_switch_stopped_for_the_recovery(app, switch);
    assert!(
        matches!(journal.phase(), PendingEnableJournalPhase::LocalPromotionStarted(_)),
        "{journal:?}\n{}",
        app.describe()
    );
}

/// The switch's recovered run stopped with its journal left for the journal recovery, which
/// the screen's Try Again runs rather than the switch's own; returns that journal
fn assert_switch_stopped_for_the_recovery(
    app: &mut TestApp,
    switch: &RecoveringSwitch,
) -> PendingEnableJournal {
    app.current().then_can(&[N::RetryLoad]).then_cannot(&[N::Enable, N::TurnOff]);
    let journal = pending_enable_journal().expect("the recovery's journal");
    assert_eq!(journal.namespace_ownership(), PendingEnableNamespaceOwnership::RecoveredExisting);
    assert_eq!(switch_phase(), Some(PersistedDriveAccountSwitchPhase::Reinitializing));
    assert!(!app.runs_operation(), "{}", app.describe());
    assert_eq!(app.manager().published_screen().body, CloudBackupScreenBody::LoadFailed);
    assert_eq!(requested_steps(app), vec![], "{}", app.describe());
    a_state_report_waits_for_the_recovery(app, switch.transition_id);
    journal
}

/// Try Again finds no durable completion, so it rolls the recovery all the way back: a
/// recovered backup has no saved-passkey pause to go back to. The switch, still stopped, offers
/// its own Try Again, whose next run recovers the backup again and goes on to the commit
fn try_again_rolls_the_recovery_back_for_the_switch_to_run_again(
    app: &mut TestApp,
    switch: &RecoveringSwitch,
) {
    let uploads =
        (app.cloud.count(CloudCall::UploadMasterKey), app.cloud.count(CloudCall::UploadWallet));
    app.dispatch(CloudBackupManagerAction::RetryLoad).then_cannot(&[N::RetryLoad, N::Enable]);
    assert_eq!(
        (app.cloud.count(CloudCall::UploadMasterKey), app.cloud.count(CloudCall::UploadWallet)),
        uploads,
        "the recovery is local"
    );
    assert_rolled_back_for_the_switch_to_run_again(app, switch);
}

/// Nothing of the recovery is left and the switch, still stopped, offers its own Try Again,
/// whose next run recovers the backup again and goes on to the commit
fn assert_rolled_back_for_the_switch_to_run_again(app: &mut TestApp, switch: &RecoveringSwitch) {
    assert!(recovered_active_key().is_none(), "{}", app.describe());
    assert_eq!(
        CloudBackupKeychain::global().snapshot_passkey_metadata().expect("metadata"),
        switch.metadata
    );
    assert_eq!(promotion_status(), MasterKeyPromotionStatus::None);
    assert_eq!(pending_enable_journal(), None, "{}", app.describe());
    assert_eq!(paused_switch_controls(app), None, "a recovered backup has no pause");
    assert_eq!(switch_phase(), Some(PersistedDriveAccountSwitchPhase::Reinitializing));
    assert_eq!(
        availability(app, A::RetryDriveAccountSwitchRecovery),
        Some(CloudBackupActionAvailability::Available),
        "{}",
        app.describe()
    );

    app.dispatch(CloudBackupManagerAction::RetryDriveAccountSwitchRecovery)
        .then_cannot(&[N::Enable]);
    assert_eq!(requested_steps(app), vec![Step_::ReportState], "{}", app.describe());
    the_passkey_opens_the_recovered_backup(app);
    platform_reports(
        app,
        Platform::State(DriveAccountSwitchPlatformState::Staged(switch.transition_id)),
    )
    .then_cannot(&[N::Enable]);
    assert_switch_committing_the_recovered_backup(app, switch);
}

/// The recovered key is active under the backup's metadata, nothing of the recovery is left, and
/// the switch asked the platform for its commit
fn assert_switch_committing_the_recovered_backup(app: &TestApp, switch: &RecoveringSwitch) {
    let active = recovered_active_key().expect("the recovered key is active");
    assert_eq!(active.as_bytes(), switch.recovered_key.as_bytes(), "{}", app.describe());
    assert_eq!(
        CloudBackupKeychain::global().snapshot_passkey_metadata().expect("metadata"),
        switch.metadata
    );
    assert_eq!(promotion_status(), MasterKeyPromotionStatus::None);
    assert_eq!(pending_enable_journal(), None, "{}", app.describe());
    assert_eq!(
        switch_phase(),
        Some(PersistedDriveAccountSwitchPhase::AwaitingAccountCommitSucceeded),
        "{}",
        app.describe()
    );
    assert_eq!(
        requested_steps(app),
        vec![Step_::Commit(switch.transition_id)],
        "{}",
        app.describe()
    );
}

/// Each discovery takes the answer it finds, so the next recovery needs the passkey again
fn the_passkey_opens_the_recovered_backup(app: &TestApp) {
    app.passkey.inner.set_discover_result(Ok(DiscoveredPasskeyResult {
        prf_output: vec![7; 32],
        credential_id: vec![1, 2, 3],
    }));
}

fn recovered_active_key() -> Option<MasterKey> {
    Cspp::new(Keychain::global().clone()).load_master_key_from_store().expect("read active key")
}

fn promotion_status() -> MasterKeyPromotionStatus {
    Cspp::new(Keychain::global().clone()).master_key_promotion_status().expect("read promotion")
}

/// Arms `fault` on the finalization's lifecycle write that saves the recovery's durable
/// completion, once; `fails` says whether that write itself fails
fn at_the_durable_completion(fails: bool, fault: impl Fn() + Send + 'static) {
    let fired = Arc::new(Mutex::new(false));
    Database::global().cloud_backup_state.fail_next_write_where(move |state| {
        let mut fired = fired.lock();
        if *fired || state.pending_verification_completion().is_none() {
            return false;
        }

        *fired = true;
        fault();
        fails
    });
}

/// Model review sequence: a switch's recovered promotion fails while it starts, after the
/// journal records it: the keychain refuses the backup's namespace for the promotion and again
/// for its rollback
///
/// The promotion stays started, which only the journal recovery can roll back, so the switch
/// stops rather than failing. Its Try Again rolls the recovery all the way back, with no
/// saved-passkey pause, and the switch's own Try Again then recovers the backup again
#[test]
fn a_recovered_switch_promotion_that_fails_to_start_or_roll_back_rolls_back_on_try_again() {
    let test = concat!(
        module_path!(),
        "::a_recovered_switch_promotion_that_fails_to_start_or_roll_back_rolls_back_on_try_again"
    );
    run_case(test, &[PhaseEnd::Exit], |_| {
        let mut app = TestApp::launch();
        let switch = switch_recovering_a_backup(&mut app, |app| {
            app.keychain.fail_saves_of(CSPP_NAMESPACE_ID_KEY);
        });
        assert_recovered_promotion_stopped(&mut app, &switch);

        app.keychain.clear_save_failures();
        try_again_rolls_the_recovery_back_for_the_switch_to_run_again(&mut app, &switch);
    });
}

/// Model review sequence: a switch's recovered finalization fails to save the durable
/// completion, and the rollback of its promotion fails too, as the keychain refuses to remove
/// the recovered key
///
/// The switch stops for the journal recovery, whose Try Again rolls the recovery back
#[test]
fn a_recovered_switch_finalization_whose_rollback_fails_rolls_back_on_try_again() {
    let test = concat!(
        module_path!(),
        "::a_recovered_switch_finalization_whose_rollback_fails_rolls_back_on_try_again"
    );
    run_case(test, &[PhaseEnd::Exit], |_| {
        let mut app = TestApp::launch();
        let switch = switch_recovering_a_backup(&mut app, |app| {
            let keychain = app.keychain.clone();
            at_the_durable_completion(true, move || {
                keychain.fail_next_delete_of(ACTIVE_ENCRYPTION_KEY_ITEM);
            });
        });
        assert_recovered_promotion_stopped(&mut app, &switch);
        assert_eq!(persisted().pending_verification_completion(), None);

        try_again_rolls_the_recovery_back_for_the_switch_to_run_again(&mut app, &switch);
    });
}

/// A switch's recovered finalization saves its durable completion, and the read that would
/// activate it fails, so the run can't tell whether the completion is there
fn switch_whose_recovered_activation_read_fails(app: &mut TestApp) -> RecoveringSwitch {
    // the lifecycle publish, the screen projection and the flags reconcile each read the record
    // between the write and its callback, in the order the harness runtime runs them; the sixth
    // read is the activation's
    let switch = switch_recovering_a_backup(app, |_| {
        at_the_durable_completion(false, || {
            Database::global().cloud_backup_state.fail_read_after(5);
        });
    });
    assert_recovered_promotion_stopped(app, &switch);
    assert!(persisted().pending_verification_completion().is_some(), "{}", app.describe());
    switch
}

/// Model review sequence: a switch's recovered finalization saves its durable completion, and
/// the read that activates it fails
///
/// The completion is there, so the switch's Try Again finishes the promotion and hands the
/// switch on to the platform's commit, with no second recovery
#[test]
fn a_recovered_switch_activation_that_fails_finishes_on_try_again() {
    let test =
        concat!(module_path!(), "::a_recovered_switch_activation_that_fails_finishes_on_try_again");
    run_case(test, &[PhaseEnd::Exit], |_| {
        let mut app = TestApp::launch();
        let switch = switch_whose_recovered_activation_read_fails(&mut app);
        let authenticated = app.passkey.inner.authenticate_count();

        app.dispatch(CloudBackupManagerAction::RetryLoad).then_cannot(&[N::RetryLoad, N::Enable]);
        assert_switch_committing_the_recovered_backup(&app, &switch);
        assert_eq!(app.passkey.inner.authenticate_count(), authenticated);
    });
}

/// Model review sequence: as above, but the durable completion is gone by the time Try Again
/// reads it
///
/// A finalization that succeeded always saved its completion, so the test removes it the way a
/// lifecycle writer dropping it would. Without it, Try Again rolls the recovery back
#[test]
fn a_recovered_switch_activation_that_fails_without_its_completion_rolls_back_on_try_again() {
    let test = concat!(
        module_path!(),
        "::a_recovered_switch_activation_that_fails_without_its_completion_rolls_back_on_try_again"
    );
    run_case(test, &[PhaseEnd::Exit], |_| {
        let mut app = TestApp::launch();
        let switch = switch_whose_recovered_activation_read_fails(&mut app);

        let mut record = persisted();
        assert!(record.clear_pending_verification_completion());
        Database::global().cloud_backup_state.set(&record).expect("drop the completion");

        try_again_rolls_the_recovery_back_for_the_switch_to_run_again(&mut app, &switch);
    });
}

/// Model review sequence: a switch's recovered promotion commits and saves its durable
/// completion, and only the journal's final delete fails
///
/// The switch stops for the journal recovery, whose Try Again finds the promotion committed,
/// removes the journal and hands the switch on to the platform's commit
#[test]
fn a_recovered_switch_promotion_whose_journal_cleanup_failed_goes_on_to_its_commit() {
    let test = concat!(
        module_path!(),
        "::a_recovered_switch_promotion_whose_journal_cleanup_failed_goes_on_to_its_commit"
    );
    run_case(test, &[PhaseEnd::Exit], |_| {
        let mut app = TestApp::launch();
        let switch = switch_recovering_a_backup(&mut app, |app| {
            app.keychain.fail_next_delete_of(CSPP_PENDING_ENABLE_JOURNAL_KEY);
        });
        assert_recovered_promotion_stopped(&mut app, &switch);
        assert_eq!(
            recovered_active_key().map(|key| *key.as_bytes()),
            Some(*switch.recovered_key.as_bytes())
        );

        app.dispatch(CloudBackupManagerAction::RetryLoad).then_cannot(&[N::RetryLoad, N::Enable]);
        assert_switch_committing_the_recovered_backup(&app, &switch);
    });
}

/// A switch's recovered promotion fails before its commit and rolls back, and the full rollback
/// after it gets past the key and the metadata but can't delete the journal
///
/// The device's key was already gone, so the rollback leaves no key active while the journal
/// still names the backup's metadata as the prior state. The switch stops for the journal
/// recovery rather than failing, since its own retry would skip that recovery
fn switch_whose_recovered_rollback_keeps_its_journal(app: &mut TestApp) -> RecoveringSwitch {
    let switch = switch_recovering_a_backup(app, |app| {
        app.keychain.fail_saves_of(ACTIVE_ENCRYPTION_KEY_ITEM);
        app.keychain.fail_next_delete_of(CSPP_PENDING_ENABLE_JOURNAL_KEY);
    });
    let journal = assert_switch_stopped_for_the_recovery(app, &switch);
    assert!(
        matches!(journal.phase(), PendingEnableJournalPhase::RemoteWritesStarted(_)),
        "{journal:?}\n{}",
        app.describe()
    );
    assert!(recovered_active_key().is_none(), "{}", app.describe());
    assert_eq!(promotion_status(), MasterKeyPromotionStatus::None);
    assert_eq!(
        CloudBackupKeychain::global().snapshot_passkey_metadata().expect("metadata"),
        switch.metadata
    );

    app.keychain.clear_save_failures();
    switch
}

/// Model review sequence: a switch's recovered promotion fails before its commit, and its
/// rollback stops at the journal's delete
///
/// Try Again finishes the rollback from where it stopped, and the switch's own Try Again then
/// recovers the backup again
#[test]
fn a_recovered_switch_rollback_that_keeps_its_journal_finishes_on_try_again() {
    let test = concat!(
        module_path!(),
        "::a_recovered_switch_rollback_that_keeps_its_journal_finishes_on_try_again"
    );
    run_case(test, &[PhaseEnd::Exit], |_| {
        let mut app = TestApp::launch();
        let switch = switch_whose_recovered_rollback_keeps_its_journal(&mut app);

        try_again_rolls_the_recovery_back_for_the_switch_to_run_again(&mut app, &switch);
    });
}

/// As above, but the app relaunches instead: the launch's journal recovery finishes the
/// rollback, and the switch's Try Again recovers the backup again
#[test]
fn a_recovered_switch_rollback_that_keeps_its_journal_finishes_after_a_relaunch() {
    let test = concat!(
        module_path!(),
        "::a_recovered_switch_rollback_that_keeps_its_journal_finishes_after_a_relaunch"
    );
    run_case(test, &[PhaseEnd::Exit], |_| {
        let mut app = TestApp::launch();
        let switch = switch_whose_recovered_rollback_keeps_its_journal(&mut app);

        app.relaunch().then_cannot(&[N::RetryLoad, N::Enable]);
        assert_rolled_back_for_the_switch_to_run_again(&mut app, &switch);
    });
}
