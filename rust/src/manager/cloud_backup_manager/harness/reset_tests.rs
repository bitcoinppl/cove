//! Effects, reset and the entry points routed through admission: what an operation in an
//! `Effecting` stage keeps, what a reset waits for, and what it leaves behind
//!
//! Each test body runs in child processes; see [`super::run_case`]
#![cfg(test)]

use std::time::Duration;

use cove_cspp::master_key::MasterKey;
use cove_device::keychain::Keychain;
use cove_device::passkey::DiscoveredPasskeyResult;

use super::{
    CloudCall, FakeAccount, Hold, NextAction as N, PasskeyCall, PasskeyMode, PhaseEnd, TestApp,
    change_a_wallet, keychain_namespace, load_detail, persisted, run_case,
};
use crate::database::Database;
use crate::database::cloud_backup::{PersistedCloudBackupState, PersistedCloudBackupStatus};
use crate::manager::cloud_backup_manager::actors::supervisor::OwnedKeychainWrite;
use crate::manager::cloud_backup_manager::admission::{Admission, Rejection, UserIntent};
use crate::manager::cloud_backup_manager::catastrophic_recovery::reset_local_data_for_catastrophic_recovery;
use crate::manager::cloud_backup_manager::ops::test_support::background_owner;
use crate::manager::cloud_backup_manager::{
    CloudBackupDriveAccountSwitchError, CloudBackupEnableContext, CloudBackupEnableState,
    CloudBackupError, CloudBackupExclusiveOperation as Op, CloudBackupKeychain,
    CloudBackupManagerAction, CloudBackupVerificationSource, DriveAccountSwitchPlatformState,
    DriveAccountSwitchReconcileAction,
};

fn local_master_key() -> MasterKey {
    cove_cspp::Cspp::new(Keychain::global().clone())
        .load_master_key_from_store()
        .expect("read local master key")
        .expect("a local master key")
}

fn local_master_key_present() -> bool {
    cove_cspp::Cspp::new(Keychain::global().clone())
        .load_master_key_from_store()
        .expect("read local master key")
        .is_some()
}

/// Replace the seeded placeholder wrapper with one the scripted passkey opens, and keep the
/// passkey's credential, so verification can recover a missing local master key
fn seed_verifiable_master_key(app: &TestApp, namespace: &str) {
    let prf_key = [7u8; 32];
    let prf_salt = [9u8; 32];
    let master_key = cove_cspp::Cspp::new(Keychain::global().clone())
        .load_master_key_from_store()
        .expect("read local master key")
        .expect("seeded master key");
    let encrypted =
        cove_cspp::master_key_crypto::encrypt_master_key(&master_key, &prf_key, &prf_salt)
            .expect("encrypt master key");

    app.cloud.inner.set_master_key_backup(
        namespace.to_string(),
        serde_json::to_vec(&encrypted).expect("encode master key wrapper"),
    );
    app.cloud.persist();
    CloudBackupKeychain::global().save_passkey(&[1, 2, 3], prf_salt).expect("save passkey");
    app.passkey.inner.set_discover_result(Ok(DiscoveredPasskeyResult {
        prf_output: prf_key.to_vec(),
        credential_id: vec![1, 2, 3],
    }));
    app.passkey.inner.set_authenticate_result(Ok(prf_key.to_vec()));
}

/// Runs the Cloud Backup part of a full wipe: the owner clears local state, the database reset
/// commits, and the reset finishes
fn finish_wipe(
    app: &TestApp,
    permit: crate::manager::cloud_backup_manager::CloudBackupResetPermit,
) {
    app.block_on("the owner to clear local state", permit.clear_local_state())
        .expect("clear local state");
    // the wipe's database reset, which `wipe_all_data_with_tier` commits before finishing
    Database::global().cloud_backup_state.delete().expect("reset the record");
    app.block_on("the reset to finish", permit.complete_after_database_reset())
        .expect("finish the reset");
}

/// Turn Off's cleanup runs on a blocking thread, which the runtime park can't see; settle waits
/// on the supervisor's stage checkpoint until the effect reports, so the next step sees it done
#[test]
fn settle_waits_for_an_effect_running_off_the_runtime() {
    let test = concat!(module_path!(), "::settle_waits_for_an_effect_running_off_the_runtime");
    run_case(test, &[PhaseEnd::Exit], |_| {
        let mut app = TestApp::launch();
        FakeAccount::new(1, 0).seed(&app);
        load_detail(&mut app);
        app.keychain.slow_deletes(Duration::from_millis(100));

        app.dispatch(CloudBackupManagerAction::DisableCloudBackup)
            .then_can(&[N::Enable])
            .then_cannot(&[N::TurnOff]);
        assert_eq!(keychain_namespace(), None);
        assert_eq!(persisted(), PersistedCloudBackupState::Disabled);
    });
}

/// An operation in an `Effecting` stage keeps authority until its completion arrives, and a reset
/// that begins meanwhile waits for that completion, then frees the operation without letting it
/// go on
#[test]
fn reset_waits_for_an_effect_in_flight_then_frees_its_operation() {
    let test =
        concat!(module_path!(), "::reset_waits_for_an_effect_in_flight_then_frees_its_operation");
    run_case(test, &[PhaseEnd::Exit], |_| {
        let mut app = TestApp::launch();
        FakeAccount::new(1, 0).seed(&app);
        load_detail(&mut app);
        let hold = Hold::default();
        app.keychain.hold_namespace_delete(hold.clone());

        app.dispatch(CloudBackupManagerAction::DisableCloudBackup)
            .then_cannot(&[N::Enable, N::TurnOff]);
        app.block_on("the held cleanup to start", hold.entered());
        assert_eq!(
            app.admission(N::Enable),
            Admission::Rejected(Rejection::OperationRunning(Op::Disable))
        );

        let manager = app.manager().clone();
        let reset = cove_tokio::task::spawn(async move { manager.prepare_local_reset().await });
        app.event("a full wipe begins", |_| {}).then_cannot(&[N::Enable, N::TurnOff]);
        assert!(!reset.is_finished(), "the reset went ahead while the cleanup was in flight");
        assert_eq!(app.admission(N::Enable), Admission::Rejected(Rejection::Resetting));
        assert_eq!(
            app.admission_of(UserIntent::LocalReset),
            Admission::Rejected(Rejection::Resetting)
        );

        app.event("release the held cleanup", |_| hold.release()).then_cannot(&[N::Enable]);
        let permit = app
            .block_on("the reset to be ready", reset)
            .expect("reset task")
            .expect("reset permit");

        // the cleanup's completion went to the reset, so Turn Off did not record Disabled
        assert_eq!(persisted().status(), PersistedCloudBackupStatus::Disabling);

        finish_wipe(&app, permit);
        app.current().then_can(&[N::Enable]).then_cannot(&[N::TurnOff]);
        assert_eq!(keychain_namespace(), None);
    });
}

/// A passkey registration can't be taken back once the platform starts it, so a reset that
/// begins during one waits for its answer; the credential it returns is never saved
#[test]
fn reset_waits_for_a_held_passkey_registration() {
    let test = concat!(module_path!(), "::reset_waits_for_a_held_passkey_registration");
    run_case(test, &[PhaseEnd::Exit], |_| {
        let mut app = TestApp::launch();
        app.current().then_can(&[N::Enable]);
        let hold = Hold::default();
        app.passkey.set_mode(PasskeyMode::Slow(hold.clone()));

        app.dispatch(CloudBackupManagerAction::EnableCloudBackupForceNew(
            CloudBackupEnableContext::settings_manual(),
        ))
        .then_cannot(&[N::Enable]);
        app.block_on("the passkey registration to start", hold.entered());
        let calls = app.passkey.interactive_calls();
        assert_eq!(calls.last().map(|call| call.kind), Some(PasskeyCall::Create), "{calls:?}");
        assert_ne!(app.manager().model_snapshot().enable_state, CloudBackupEnableState::Idle);

        let manager = app.manager().clone();
        let reset = cove_tokio::task::spawn(async move { manager.prepare_local_reset().await });
        app.event("a full wipe begins", |_| {}).then_cannot(&[N::Enable]);
        assert!(!reset.is_finished(), "the reset went ahead while the registration was held");
        assert_eq!(app.admission(N::Enable), Admission::Rejected(Rejection::Resetting));

        app.event("release the held registration", |_| hold.release()).then_cannot(&[N::Enable]);
        let permit = app
            .block_on("the reset to be ready", reset)
            .expect("reset task")
            .expect("reset permit");
        finish_wipe(&app, permit);

        // the wipe ends the interrupted Enable's progress along with its keys
        app.current().then_can(&[N::Enable]).then_cannot(&[N::TurnOff]);
        assert_eq!(app.manager().model_snapshot().enable_state, CloudBackupEnableState::Idle);
        assert_eq!(CloudBackupKeychain::global().load_credential_id().expect("read"), None);
        assert_eq!(keychain_namespace(), None);
    });
}

/// Control for the next test: verification puts a missing local master key back, through the
/// owner, once the passkey proves the cloud copy
#[test]
fn verification_restores_a_missing_local_master_key_through_the_owner() {
    let test = concat!(
        module_path!(),
        "::verification_restores_a_missing_local_master_key_through_the_owner"
    );
    run_case(test, &[PhaseEnd::Exit], |_| {
        let mut app = TestApp::launch();
        let namespace = FakeAccount::new(1, 0).seed(&app);
        seed_verifiable_master_key(&app, &namespace);
        load_detail(&mut app);
        CloudBackupKeychain::global().delete_master_key().expect("delete local master key");

        app.dispatch(CloudBackupManagerAction::StartVerification(
            CloudBackupVerificationSource::CloudBackupDetail,
        ))
        .then_cannot(&[N::Enable]);
        assert!(local_master_key_present(), "{}", app.describe());
    });
}

/// A wipe during a running verification stops it: the key it proves after the wipe is refused by
/// the owner, so nothing the wipe removed comes back
#[test]
fn wipe_during_verification_leaves_no_stale_key() {
    let test = concat!(module_path!(), "::wipe_during_verification_leaves_no_stale_key");
    run_case(test, &[PhaseEnd::Exit], |_| {
        let mut app = TestApp::launch();
        let namespace = FakeAccount::new(1, 0).seed(&app);
        seed_verifiable_master_key(&app, &namespace);
        load_detail(&mut app);
        CloudBackupKeychain::global().delete_master_key().expect("delete local master key");
        let hold = Hold::default();
        app.passkey.set_mode(PasskeyMode::Slow(hold.clone()));

        app.dispatch(CloudBackupManagerAction::StartVerification(
            CloudBackupVerificationSource::CloudBackupDetail,
        ))
        .then_cannot(&[N::Enable]);
        app.block_on("verification's passkey request to start", hold.entered());

        let manager = app.manager().clone();
        let permit = app
            .block_on("the reset to be ready", async move { manager.prepare_local_reset().await })
            .expect("reset permit");
        finish_wipe(&app, permit);

        app.event("release the held passkey request", |_| hold.release())
            .then_can(&[N::Enable])
            .then_cannot(&[N::TurnOff]);
        assert!(!local_master_key_present(), "the wiped master key came back");
        assert_eq!(CloudBackupKeychain::global().load_credential_id().expect("read"), None);
        assert_eq!(keychain_namespace(), None);
    });
}

/// Keys a background worker prepared for one configuration never land on the next: after Turn
/// Off and a new configuration, the first configuration's late keychain write is refused and the
/// new configuration's own write still lands
#[test]
fn a_turned_off_configurations_late_keychain_write_is_refused() {
    let test =
        concat!(module_path!(), "::a_turned_off_configurations_late_keychain_write_is_refused");
    run_case(test, &[PhaseEnd::Exit], |_| {
        let mut app = TestApp::launch_with_passkey(PasskeyMode::FailOnAnyInteractiveCall);
        let manager = app.manager().clone();
        let first = FakeAccount::new(1, 0).seed(&app);
        let first_key = *local_master_key().as_bytes();
        let first_owner = app.block_on("the first owner", background_owner(&manager));
        load_detail(&mut app);

        app.dispatch(CloudBackupManagerAction::DisableCloudBackup)
            .then_can(&[N::Enable])
            .then_cannot(&[N::TurnOff]);
        let second = FakeAccount::new(1, 0).seed(&app);
        let second_key = *local_master_key().as_bytes();
        let second_credential = CloudBackupKeychain::global().load_credential_id().expect("read");
        assert_ne!(first, second);
        assert_ne!(first_key, second_key);

        // the first configuration's worker answers late with its keys
        let late = vec![
            OwnedKeychainWrite::Passkey { credential_id: vec![0xa1; 16], prf_salt: [0xa2; 32] },
            OwnedKeychainWrite::MasterKey(MasterKey::from_bytes(first_key)),
        ];
        let refused =
            app.block_on("the late write", manager.apply_owned_keychain_writes(first_owner, late));

        assert!(matches!(refused, Err(CloudBackupError::Cancelled)), "{refused:?}");
        assert_eq!(keychain_namespace(), Some(second));
        assert_eq!(*local_master_key().as_bytes(), second_key);
        assert_eq!(
            CloudBackupKeychain::global().load_credential_id().expect("read"),
            second_credential
        );

        // the current configuration's background work still owns its keys
        let second_owner = app.block_on("the second owner", background_owner(&manager));
        let own = vec![OwnedKeychainWrite::MasterKey(MasterKey::from_bytes(second_key))];
        let applied = app
            .block_on("the current write", manager.apply_owned_keychain_writes(second_owner, own));
        assert!(applied.is_ok(), "{applied:?}");
    });
}

/// The platform calls that start or recover a Google Drive account switch, and a full wipe, are
/// admitted like dispatched intents, and a rejection reaches the caller as a typed error
#[test]
fn routed_entry_points_are_admitted_or_rejected_with_a_reason() {
    let test =
        concat!(module_path!(), "::routed_entry_points_are_admitted_or_rejected_with_a_reason");
    run_case(test, &[PhaseEnd::Exit], |_| {
        let mut app = TestApp::launch();
        app.current().then_can(&[N::Enable]);
        assert_eq!(
            app.admission_of(UserIntent::SwitchDriveAccount),
            Admission::Rejected(Rejection::CloudBackupOff)
        );
        assert_eq!(
            app.admission_of(UserIntent::CancelDriveAccountSwitch),
            Admission::Rejected(Rejection::NothingToCancel)
        );
        assert_eq!(
            app.admission_of(UserIntent::ConfirmSavedPasskey),
            Admission::Rejected(Rejection::NothingToContinue)
        );
        assert_eq!(
            app.admission_of(UserIntent::RetryDriveAccountSwitchRecovery),
            Admission::Accepted
        );
        assert_eq!(app.admission_of(UserIntent::LocalReset), Admission::Accepted);

        let manager = app.manager().clone();
        let switch =
            app.block_on("the switch call to answer", manager.begin_drive_account_switch());
        assert_eq!(switch, Err(CloudBackupDriveAccountSwitchError::NotConfigured));
        let recovery = app.block_on(
            "the recovery call to answer",
            manager.reconcile_drive_account_switch(DriveAccountSwitchPlatformState::NoTransition),
        );
        assert_eq!(recovery, Ok(DriveAccountSwitchReconcileAction::None));

        FakeAccount::new(1, 0).seed(&app);
        app.current().then_can(&[N::TurnOff]);
        assert_eq!(app.admission_of(UserIntent::SwitchDriveAccount), Admission::Accepted);
        assert_eq!(
            app.admission_of(UserIntent::CancelDriveAccountSwitch),
            Admission::Rejected(Rejection::NothingToCancel)
        );
    });
}

/// Catastrophic recovery clears the namespace only while no Cloud Backup manager exists; with
/// the lifecycle owner running it is refused and the namespace stays
#[test]
fn catastrophic_recovery_cannot_clear_the_namespace_while_its_owner_runs() {
    let test = concat!(
        module_path!(),
        "::catastrophic_recovery_cannot_clear_the_namespace_while_its_owner_runs"
    );
    run_case(test, &[PhaseEnd::Exit], |_| {
        let mut app = TestApp::launch();
        let namespace = FakeAccount::new(1, 0).seed(&app);
        app.current().then_can(&[N::TurnOff]);

        assert!(reset_local_data_for_catastrophic_recovery().is_err());
        assert_eq!(keychain_namespace(), Some(namespace));
        app.current().then_can(&[N::TurnOff]);
    });
}

/// A wipe that fails before its commit leaves the record as it was, and ending the reset wakes
/// the uploads it paused: the wallet changed during the reset uploads once, right away, and the
/// change's own debounce adds no second upload
#[test]
fn abandoned_reset_wakes_the_uploads_it_paused_exactly_once() {
    let test =
        concat!(module_path!(), "::abandoned_reset_wakes_the_uploads_it_paused_exactly_once");
    run_case(test, &[PhaseEnd::Exit], |_| {
        let mut app = TestApp::launch();
        FakeAccount::new(1, 0).seed(&app);

        let manager = app.manager().clone();
        let permit = app
            .block_on("the reset to be ready", async move { manager.prepare_local_reset().await })
            .expect("reset permit");

        let mut changed = None;
        app.event("a local wallet changes during the reset", |app| {
            changed = change_a_wallet(app);
        })
        .then_cannot(&[N::TurnOff, N::Enable]);

        let wallet_id = changed.expect("the seeded wallet");
        let record_id = cove_cspp::backup_data::wallet_record_id(wallet_id.as_ref());
        let row = || {
            Database::global()
                .cloud_blob_sync_states
                .get(&record_id)
                .expect("read the wallet's sync row")
                .expect("the wallet has a sync row")
        };

        let uploads = app.cloud.count(CloudCall::UploadWallet);
        assert!(row().is_dirty(), "the change was not recorded: {:?}", row());

        // the upload's debounce ends while the reset holds the lane, so the lane pauses it
        app.advance_clock(Duration::from_secs(60)).then_cannot(&[N::TurnOff, N::Enable]);
        assert_eq!(app.cloud.count(CloudCall::UploadWallet), uploads, "an upload ran in the reset");
        assert!(row().is_dirty(), "the paused upload cleared the row: {:?}", row());

        app.event("the wipe fails before its commit", |app| {
            app.block_on("the reset to end", permit.resume_after_failed_reset())
                .expect("abandon the reset");
        })
        .then_can(&[N::TurnOff])
        .then_cannot(&[N::Enable]);

        assert_eq!(persisted().status(), PersistedCloudBackupStatus::Enabled);
        assert_eq!(
            app.cloud.count(CloudCall::UploadWallet),
            uploads + 1,
            "the wake did not upload"
        );
        assert!(!row().is_dirty(), "the upload left the row dirty: {:?}", row());

        app.advance_clock(Duration::from_secs(600)).then_can(&[N::TurnOff]);
        assert_eq!(app.cloud.count(CloudCall::UploadWallet), uploads + 1, "a second upload ran");
    });
}
