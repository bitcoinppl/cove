//! Passkey policy: who may present a passkey request, and what a refused or unanswered one leaves
//!
//! Invariant 6 checks every request after each event; these tests drive the paths that must
//! present nothing, and the taps whose request the platform refuses or the user leaves.
//! Each test body runs in child processes; see [`super::run_case`]
#![cfg(test)]

use std::time::Duration;

use cove_device::keychain::Keychain;

use super::{
    FakeAccount, Hold, NextAction as N, PasskeyCall, PasskeyMode, PhaseEnd, TestApp,
    change_a_wallet, load_detail, run_case, seed_other_namespace, seed_verifiable_master_key,
};
use crate::manager::cloud_backup_manager::CloudBackupStore;
use crate::manager::cloud_backup_manager::admission::{Admission, UserIntent};
use crate::manager::cloud_backup_manager::detail::CloudBackupVerificationPresentation;
use crate::manager::cloud_backup_manager::{
    CloudBackupEnableContext, CloudBackupEnableState, CloudBackupKeychain,
    CloudBackupManagerAction, CloudBackupVerificationSource, VerificationState,
};
use crate::manager::connectivity_manager::ConnectivityStatus;

fn verify() -> CloudBackupManagerAction {
    CloudBackupManagerAction::StartVerification(CloudBackupVerificationSource::CloudBackupDetail)
}

/// The local master key is gone, so any path that wants it must either ask for the passkey or
/// stop with `NeedsPasskey`
fn lose_local_master_key() {
    CloudBackupKeychain::global().delete_master_key().expect("delete local master key");
    cove_cspp::Cspp::<Keychain>::clear_cached_master_key();
}

/// Silent paths never prompt: with the local key missing and a stored credential that
/// would answer, detail entry, refresh, a background upload, a reconnect, a cloud change, a
/// relaunch and Turn Off's preparation present no passkey request
#[test]
fn silent_paths_never_present_a_passkey_request() {
    let test = concat!(module_path!(), "::silent_paths_never_present_a_passkey_request");
    run_case(test, &[PhaseEnd::Exit], |_| {
        let mut app = TestApp::launch_with_passkey(PasskeyMode::FailOnAnyInteractiveCall);
        let namespace = FakeAccount::new(1, 0).seed(&app);
        seed_verifiable_master_key(&app, &namespace);
        lose_local_master_key();

        app.dispatch(CloudBackupManagerAction::EnterDetail).then_can(&[N::TurnOff]);
        load_detail(&mut app);

        app.event("a local wallet changes", |app| {
            change_a_wallet(app);
            app.manager().handle_wallet_set_change();
        })
        .then_can(&[N::TurnOff]);
        app.advance_clock(Duration::from_secs(60)).then_can(&[N::TurnOff]);

        app.event("the network comes back", |app| {
            app.manager().handle_connectivity_change(ConnectivityStatus::Connected);
        })
        .then_can(&[N::TurnOff]);
        app.event("the cloud storage changes", |app| app.manager().cloud_storage_did_change())
            .then_can(&[N::TurnOff]);

        app.relaunch().then_can(&[N::TurnOff]);
        app.dispatch(CloudBackupManagerAction::EnterDetail).then_can(&[N::TurnOff]);

        app.dispatch(CloudBackupManagerAction::DisableCloudBackup).then_can(&[N::ConfirmDisable]);
        assert_eq!(app.passkey.interactive_calls(), Vec::new());
    });
}

/// Single flight: a wipe ends the verification whose passkey request is still open,
/// and a tap before that request answers is refused as busy instead of stacking a second sheet;
/// once it answers, the next tap presents
#[test]
fn a_tap_while_an_ended_operations_request_is_open_is_refused_as_busy() {
    let test = concat!(
        module_path!(),
        "::a_tap_while_an_ended_operations_request_is_open_is_refused_as_busy"
    );
    run_case(test, &[PhaseEnd::Exit], |_| {
        let mut app = TestApp::launch();
        let namespace = FakeAccount::new(1, 0).seed(&app);
        seed_verifiable_master_key(&app, &namespace);
        load_detail(&mut app);
        lose_local_master_key();
        let hold = Hold::default();
        app.passkey.set_mode(PasskeyMode::Slow(hold.clone()));

        app.dispatch(verify()).then_cannot(&[N::Enable]);
        app.block_on("verification's passkey request to open", hold.entered());

        let manager = app.manager().clone();
        let permit = app
            .block_on("the reset to be ready", async move { manager.prepare_local_reset().await })
            .expect("reset permit");
        app.block_on("the owner to clear local state", permit.clear_local_state())
            .expect("clear local state");
        crate::database::Database::global().cloud_backup_state.delete().expect("reset the record");
        app.block_on("the reset to finish", permit.complete_after_database_reset())
            .expect("finish the reset");

        let opened = app.passkey.interactive_calls().len();
        app.dispatch(CloudBackupManagerAction::EnableCloudBackupForceNew(
            CloudBackupEnableContext::settings_manual(),
        ))
        .then_can(&[N::Enable]);
        assert_eq!(app.passkey.interactive_calls().len(), opened, "a second sheet was presented");
        assert_eq!(
            app.manager().model_snapshot().enable_state,
            CloudBackupEnableState::Idle,
            "{}",
            app.describe()
        );

        app.event("the open request answers", |_| hold.release()).then_can(&[N::Enable]);
        app.passkey.set_mode(PasskeyMode::Succeeds);
        app.passkey.inner.set_create_result(Ok(vec![4, 5, 6]));

        app.dispatch(CloudBackupManagerAction::EnableCloudBackupForceNew(
            CloudBackupEnableContext::settings_manual(),
        ))
        .then_cannot(&[N::Enable]);
        let calls = app.passkey.interactive_calls();
        assert_eq!(calls.get(opened).map(|call| call.kind), Some(PasskeyCall::Create), "{calls:?}");
    });
}

/// The platform checks the permit again once its presenter is ready, so a reset that
/// ends Verify while the platform waits for a presenter leaves that request nothing to present
#[test]
fn an_operation_that_ends_during_the_presenter_wait_presents_nothing() {
    let test = concat!(
        module_path!(),
        "::an_operation_that_ends_during_the_presenter_wait_presents_nothing"
    );
    run_case(test, &[PhaseEnd::Exit], |_| {
        let mut app = TestApp::launch();
        let namespace = FakeAccount::new(1, 0).seed(&app);
        seed_verifiable_master_key(&app, &namespace);
        load_detail(&mut app);
        lose_local_master_key();
        let hold = Hold::default();
        app.passkey.set_mode(PasskeyMode::AwaitingPresenter(hold.clone()));

        app.dispatch(verify()).then_cannot(&[N::Enable]);
        app.block_on("verification's request to wait for a presenter", hold.entered());

        let manager = app.manager().clone();
        let permit = app
            .block_on("the reset to be ready", async move { manager.prepare_local_reset().await })
            .expect("reset permit");
        app.block_on("the owner to clear local state", permit.clear_local_state())
            .expect("clear local state");
        crate::database::Database::global().cloud_backup_state.delete().expect("reset the record");
        app.block_on("the reset to finish", permit.complete_after_database_reset())
            .expect("finish the reset");

        app.event("the presenter becomes ready", |_| hold.release()).then_can(&[N::Enable]);
        app.block_on("the withdrawn request to return", hold.finished());
        let calls = app.passkey.interactive_calls();
        assert_eq!(calls.len(), 1, "{calls:?}");
        assert_eq!(
            app.passkey.withdrawn_calls(),
            calls,
            "the request presented after Verify ended"
        );
    });
}

/// A Verify the user leaves with the sheet unanswered asks once, falls back
/// to no discovery or registration, keeps the stored credential, and after a relaunch neither
/// the root prompt nor detail entry asks again
#[test]
fn a_not_completed_verify_asks_once_and_does_not_prompt_after_relaunch() {
    let test = concat!(
        module_path!(),
        "::a_not_completed_verify_asks_once_and_does_not_prompt_after_relaunch"
    );
    run_case(test, &[PhaseEnd::Exit], |_| {
        let mut app = TestApp::launch();
        let namespace = FakeAccount::new(1, 0).seed(&app);
        seed_verifiable_master_key(&app, &namespace);
        load_detail(&mut app);
        lose_local_master_key();

        // a wallet added or removed asks for a verification, which is what the root prompt shows
        app.event("the local wallet set changes", |app| app.manager().handle_wallet_set_change())
            .then_can(&[N::TurnOff]);
        assert!(
            matches!(
                app.manager().model_snapshot().verification_presentation,
                CloudBackupVerificationPresentation::NeedsDecision { .. }
            ),
            "the root prompt was not showing before the tap: {}",
            app.describe()
        );

        app.passkey.set_mode(PasskeyMode::NotCompleted);
        app.dispatch(verify()).then_can(&[N::TurnOff, N::Verify]);
        let calls = app.passkey.interactive_calls();
        assert_eq!(calls.len(), 1, "a not-completed request was retried or fell back: {calls:?}");
        assert_eq!(app.manager().model_snapshot().verification, VerificationState::Cancelled);
        assert_eq!(
            CloudBackupKeychain::global().load_credential_id().expect("read credential"),
            Some(vec![1, 2, 3]),
            "the credential was marked missing"
        );

        app.passkey.set_mode(PasskeyMode::FailOnAnyInteractiveCall);
        app.relaunch().then_can(&[N::TurnOff, N::Verify]);
        assert!(
            !matches!(
                app.manager().model_snapshot().verification_presentation,
                CloudBackupVerificationPresentation::NeedsDecision { .. }
            ),
            "the root prompt came back after relaunch: {}",
            app.describe()
        );

        app.dispatch(CloudBackupManagerAction::EnterDetail).then_can(&[N::TurnOff, N::Verify]);
        assert_eq!(app.passkey.interactive_calls().len(), 1);
    });
}

/// Confirm Saved Passkey is its own tap with its own permit; the registration's
/// Enable already ended, so only the confirmation tap can present the saved passkey
#[test]
fn manual_saved_passkey_confirmation_presents_under_its_own_tap() {
    let test =
        concat!(module_path!(), "::manual_saved_passkey_confirmation_presents_under_its_own_tap");
    run_case(test, &[PhaseEnd::Exit], |_| {
        let mut app = TestApp::launch();
        app.passkey.inner.set_create_result(Ok(vec![4, 5, 6]));
        app.passkey.inner.set_authenticate_result(Ok(vec![7; 32]));
        app.current().then_can(&[N::Enable]);

        app.dispatch(CloudBackupManagerAction::EnableCloudBackupForceNew(
            CloudBackupEnableContext::settings_manual(),
        ))
        .then_cannot(&[N::Enable]);
        let calls = app.passkey.interactive_calls();
        assert_eq!(
            calls.iter().map(|call| call.kind).collect::<Vec<_>>(),
            vec![PasskeyCall::Create],
            "{}",
            app.describe()
        );

        // the save-settle delay passes, then the Enable ends with the confirmation staged
        app.advance_clock(Duration::from_secs(5)).then_cannot(&[N::TurnOff]);
        assert_eq!(app.admission_of(UserIntent::ConfirmSavedPasskey), Admission::Accepted);
        assert!(
            matches!(
                app.manager().model_snapshot().enable_state,
                CloudBackupEnableState::AwaitingSavedPasskeyConfirmation(_)
            ),
            "{}",
            app.describe()
        );

        app.dispatch(CloudBackupManagerAction::ConfirmSavedPasskey).then_can(&[N::TurnOff]);
        let calls = app.passkey.interactive_calls();
        assert_eq!(
            calls.iter().map(|call| call.kind).collect::<Vec<_>>(),
            vec![PasskeyCall::Create, PasskeyCall::Authenticate],
            "{}",
            app.describe()
        );
    });
}

/// Restore from another namespace: reading the other backup is silent, and only the
/// Recover tap presents, once, to open that backup's master key; its wallet then joins this
/// device and the current backup, and the old namespace is removed
#[test]
fn recovering_another_namespace_presents_only_under_its_tap() {
    let test =
        concat!(module_path!(), "::recovering_another_namespace_presents_only_under_its_tap");
    run_case(test, &[PhaseEnd::Exit], |_| {
        let mut app = TestApp::launch_with_passkey(PasskeyMode::FailOnAnyInteractiveCall);

        // fixture wallets share one sample xpub, so a local wallet would make the recovered one
        // a duplicate; and recovery deletes the old namespace only once the current listing
        // shows what it uploaded
        let namespace = FakeAccount::new(0, 0).seed(&app);
        app.cloud.inner.set_reflect_uploaded_wallets_in_listing(true);
        seed_verifiable_master_key(&app, &namespace);
        let (other_namespace, wallet) = seed_other_namespace(&app);

        load_detail(&mut app);
        assert_eq!(
            app.admission_of(UserIntent::RecoverOtherBackups),
            Admission::Accepted,
            "the other backup was not found: {}",
            app.describe()
        );
        assert_eq!(app.passkey.interactive_calls(), Vec::new());

        app.passkey.set_mode(PasskeyMode::Succeeds);
        app.dispatch(CloudBackupManagerAction::RecoverOtherBackups).then_can(&[N::TurnOff]);

        let calls = app.passkey.interactive_calls();
        assert_eq!(
            calls.iter().map(|call| call.kind).collect::<Vec<_>>(),
            vec![PasskeyCall::Discover],
            "{}",
            app.describe()
        );
        assert!(
            !app.cloud.inner.has_namespace(&other_namespace),
            "the recovered namespace is still in the cloud: {}",
            app.describe()
        );

        let local = CloudBackupStore::global().all_wallets().expect("read local wallets");
        assert!(
            local.iter().any(|metadata| metadata.id == wallet),
            "the other backup's wallet is not on this device: {}",
            app.describe()
        );
    });
}
