//! A Google Drive account switch, with the test playing Android's half of it
//!
//! Rust asks for each platform step on the reconcile channel and Android answers each with a
//! typed result; the test reads the requests and answers them as the platform would. The user's
//! Switch, Try Again and Cancel go through dispatch like any other control
//!
//! Each test body runs in child processes; see [`super::run_case`]
#![cfg(test)]

use std::time::Duration;

use cove_cspp::backup_data::wallet_filename_from_record_id;
use cove_device::cloud_storage::CloudStorageError;
use cove_device::passkey::{DiscoveredPasskeyResult, PasskeyError};

use super::app::Step;
use super::process::checkpoint;
use super::{
    CloudCall, CloudFault, FakeAccount, Hold, NextAction as N, PhaseEnd, TestApp,
    assert_uploads_confirmed_in, keychain_namespace, load_detail, persisted, run_case,
    seed_an_uploaded_wallet, seed_verifiable_master_key,
};
use crate::database::Database;
use crate::database::cloud_backup::PersistedDriveAccountSwitchPhase;
use crate::manager::cloud_backup_manager::admission::{Admission, Rejection, UserIntent};
use crate::manager::cloud_backup_manager::keychain::CloudBackupKeychain;
use crate::manager::cloud_backup_manager::pending_enable::PendingEnableJournal;
use crate::manager::cloud_backup_manager::screen::{
    CloudBackupActionAvailability, CloudBackupDriveAccountSwitchStage, CloudBackupRejection,
    CloudBackupSavedPasskeyControl, CloudBackupSavedPasskeyLifecycle, CloudBackupScreenAction as A,
    CloudBackupScreenBody,
};
use crate::manager::cloud_backup_manager::{
    CloudBackupExclusiveOperation as Op, CloudBackupManagerAction, CloudBackupResetPermit,
    DriveAccountSwitchPlatformResult as Platform, DriveAccountSwitchPlatformState,
    RequestedDriveAccountSwitchStep as Step_,
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

/// The same unsaved result is lost with the process, and the record still says Reinitializing:
/// the relaunch shows Try Again, and the platform's launch report runs the reinitialize again
/// in the new account and commits it
#[test]
fn a_switch_whose_finished_reinitialize_was_not_saved_finishes_after_a_relaunch() {
    let test = concat!(
        module_path!(),
        "::a_switch_whose_finished_reinitialize_was_not_saved_finishes_after_a_relaunch"
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
        platform_reports(
            &mut app,
            Platform::State(DriveAccountSwitchPlatformState::Staged(transition_id)),
        )
        .then_cannot(&[N::Enable]);
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

fn pending_enable_journal() -> Option<PendingEnableJournal> {
    CloudBackupKeychain::global().load_pending_enable_journal().expect("the journal loads")
}
