//! Effects, reset and the entry points routed through admission: what an operation in an
//! `Effecting` stage keeps, what a reset waits for, and what it leaves behind
//!
//! Each test body runs in child processes; see [`super::run_case`]
#![cfg(test)]

use std::time::Duration;

use cove_cspp::MasterKeyPromotionStatus;
use cove_cspp::backup_data::wallet_record_id;
use cove_device::keychain::Keychain;
use cove_device::passkey::DiscoveredPasskeyResult;

use super::{
    CloudCall, CloudFault, FakeAccount, Hold, NextAction as N, PasskeyCall, PasskeyMode, PhaseEnd,
    TestApp, change_a_wallet, keychain_namespace, load_detail, persisted, run_case,
};
use crate::database::Database;
use crate::database::cloud_backup::{
    CloudBlobUploadedPendingConfirmationState, PersistedBackupVerificationState,
    PersistedCloudBackupState, PersistedCloudBackupStatus, PersistedCloudBlobState,
    PersistedCloudBlobSyncState, PersistedDriveAccountSwitchPhase,
};
use crate::manager::cloud_backup_manager::actors::OwnedKeychainWrite;
use crate::manager::cloud_backup_manager::admission::{
    Admission, CoalescibleIntent, PauseReason, Rejection, UserIntent,
};
use crate::manager::cloud_backup_manager::catastrophic_recovery::reset_local_data_for_catastrophic_recovery;
use crate::manager::cloud_backup_manager::detail::VerificationState;
use crate::manager::cloud_backup_manager::model::CloudBackupLifecycle;
use crate::manager::cloud_backup_manager::ops::test_support::{
    current_background_writes, wallet_entry_with_labels,
};
use crate::manager::cloud_backup_manager::{
    CloudBackupDriveAccountSwitchError, CloudBackupEnableContext, CloudBackupEnableState,
    CloudBackupError, CloudBackupExclusiveOperation as Op, CloudBackupKeychain,
    CloudBackupManagerAction, CloudBackupStore, CloudBackupVerificationSource,
    DriveAccountSwitchPlatformState, DriveAccountSwitchReconcileAction,
};

fn local_master_key_present() -> bool {
    cove_cspp::Cspp::new(Keychain::global().clone())
        .load_master_key_from_store()
        .expect("read local master key")
        .is_some()
}

/// The local master key's bytes, which tell one backup's key from another's
fn master_key_bytes() -> Option<Vec<u8>> {
    cove_cspp::Cspp::new(Keychain::global().clone())
        .load_master_key_from_store()
        .expect("read local master key")
        .map(|master_key| master_key.as_bytes().to_vec())
}

fn credential_id() -> Option<Vec<u8>> {
    CloudBackupKeychain::global().load_credential_id().expect("read credential")
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
    // the wipe's commit point: `wipe_all_data_with_tier` resets the whole database, so every
    // Cloud Backup table, the blob sync rows included, is gone before the reset finishes
    Database::global().dangerous_reset_all_data().expect("reset the database");
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

/// An upload exports its wallet's labels on a blocking thread, which parks the runtime while
/// the export runs; settle waits for the answer, so the step after a wake sees the upload done
#[test]
fn settle_waits_for_a_blocking_task_a_runtime_task_awaits() {
    let test = concat!(module_path!(), "::settle_waits_for_a_blocking_task_a_runtime_task_awaits");
    run_case(test, &[PhaseEnd::Exit], |_| {
        let mut app = TestApp::launch();
        let answered = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));

        let flag = answered.clone();
        app.event("a task waits on slow blocking work", |_| {
            cove_tokio::task::spawn(async move {
                let slow = || std::thread::sleep(Duration::from_millis(100));
                cove_tokio::task::spawn_blocking(slow).await.expect("the blocking work");
                flag.store(true, std::sync::atomic::Ordering::Release);
            });
        })
        .then_can(&[N::Enable])
        .then_cannot(&[N::TurnOff]);

        assert!(
            answered.load(std::sync::atomic::Ordering::Acquire),
            "settle returned while the blocking work ran"
        );
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

/// A manual verification is an operation: it holds the slot while it reads the backup, a second
/// Verify joins it, and evidence it read before a wallet change can't mark the new wallet set
/// verified
#[test]
fn verification_holds_the_slot_and_judges_its_own_wallet_set() {
    let test =
        concat!(module_path!(), "::verification_holds_the_slot_and_judges_its_own_wallet_set");
    run_case(test, &[PhaseEnd::Exit], |_| {
        let mut app = TestApp::launch();
        let namespace = FakeAccount::new(1, 0).seed(&app);
        seed_verifiable_master_key(&app, &namespace);
        load_detail(&mut app);
        let hold = Hold::default();
        app.cloud.script_next(CloudCall::DownloadWallet, CloudFault::Hold(hold.clone()));

        app.dispatch(CloudBackupManagerAction::StartVerification(
            CloudBackupVerificationSource::CloudBackupDetail,
        ))
        .then_cannot(&[N::Enable, N::TurnOff, N::Verify]);
        app.block_on("verification's wallet read to start", hold.entered());
        assert_eq!(
            app.admission(N::TurnOff),
            Admission::Rejected(Rejection::OperationRunning(Op::Verify))
        );

        app.event("a wallet change that needs a new verification lands meanwhile", |app| {
            let changed = change_a_wallet(app).expect("a local wallet");
            app.manager().handle_wallet_backup_change_and_reverify(changed);
        })
        .then_cannot(&[N::TurnOff]);

        app.dispatch(CloudBackupManagerAction::StartVerification(
            CloudBackupVerificationSource::CloudBackupDetail,
        ))
        .then_cannot(&[N::TurnOff]);
        let outcome = app.last_outcome().expect("the second Verify is recorded");
        assert_eq!(
            outcome.admission,
            Admission::Coalesced(CoalescibleIntent::RepeatOfRunning(Op::Verify))
        );

        app.event("release the held wallet read", |_| hold.release()).then_cannot(&[N::Enable]);
        app.advance_clock(Duration::from_secs(60)).then_can(&[N::TurnOff]);
        let PersistedCloudBackupState::Configured(configured) = persisted() else {
            panic!("expected a configured record\n{}", app.describe());
        };

        assert!(
            !matches!(configured.verification, PersistedBackupVerificationState::Verified { .. }),
            "evidence from before the wallet change was kept: {:?}",
            configured.verification
        );
        assert!(
            !matches!(app.manager().model_snapshot().verification, VerificationState::Verified(_)),
            "evidence from before the wallet change shows as verified\n{}",
            app.describe()
        );
    });
}

/// A read of backup A that outlives its configuration proves nothing about backup B: a detail
/// refresh of A is held inside its wallet download, a reset ends A, B is enabled, and B's upload of
/// the same wallet revision waits for its confirmation. Once A's read lands, B's upload still
/// waits, because only B's own cloud can confirm it
#[test]
fn a_held_read_of_an_ended_backup_confirms_nothing_in_the_next_one() {
    let test = concat!(
        module_path!(),
        "::a_held_read_of_an_ended_backup_confirms_nothing_in_the_next_one"
    );
    run_case(test, &[PhaseEnd::Exit], |_| {
        let mut app = TestApp::launch();
        let first = FakeAccount::new(1, 0).seed(&app);
        let wallet = CloudBackupStore::global().all_wallets().expect("read wallets").remove(0);
        let record_id = wallet_record_id(wallet.id.as_ref());
        // the revision A's cloud holds for the wallet, as the seed wrote it
        let revision = wallet_entry_with_labels(&wallet, None).content_revision_hash.clone();

        let hold = Hold::default();
        let held_download =
            app.cloud.script_next(CloudCall::DownloadWallet, CloudFault::Hold(hold.clone()));
        app.dispatch(CloudBackupManagerAction::RefreshDetail).then_can(&[N::TurnOff]);
        app.block_on("A's detail read to download the wallet", hold.entered());

        let manager = app.manager().clone();
        let permit = app
            .block_on("the reset to be ready", async move { manager.prepare_local_reset().await })
            .expect("reset permit");
        finish_wipe(&app, permit);

        let second = FakeAccount::new(1, 0).seed(&app);
        assert_ne!(second, first);

        // the same wallet is on this device again, and B's upload of it landed but B's cloud
        // doesn't show it yet
        let mut wallets = CloudBackupStore::global().all_wallets().expect("read wallets");
        wallets.push(wallet.clone());
        Database::global()
            .wallets()
            .save_all_wallets(wallet.network, wallet.wallet_mode, wallets)
            .expect("save the wallet again");
        let waiting = PersistedCloudBlobSyncState::wallet(
            second.clone(),
            wallet.id,
            record_id.clone(),
            PersistedCloudBlobState::UploadedPendingConfirmation(
                CloudBlobUploadedPendingConfirmationState {
                    revision_hash: revision,
                    uploaded_at: cove_util::time::unix_timestamp_secs_or_zero(),
                    attempt_count: 0,
                    last_checked_at: None,
                },
            ),
        );
        Database::global().cloud_blob_sync_states.set(&waiting).expect("persist B's upload");

        app.event("release A's held wallet download", |_| hold.release()).then_can(&[N::TurnOff]);

        let held_namespace = app
            .cloud
            .calls()
            .into_iter()
            .find_map(|(id, namespace)| (id == held_download).then_some(namespace))
            .flatten();
        assert_eq!(held_namespace, Some(first), "the held download was not A's read");

        let row = Database::global()
            .cloud_blob_sync_states
            .get(&record_id)
            .expect("read the wallet's row")
            .expect("B's upload row");
        assert_eq!(row.namespace_id, second);
        assert!(
            matches!(row.state, PersistedCloudBlobState::UploadedPendingConfirmation(_)),
            "A's read confirmed B's upload: {:?}\n{}",
            row.state,
            app.describe()
        );
    });
}

/// A key recovery that outlives its configuration can't write the next one's keys: a dirty
/// upload starts proving backup A's master key, a reset ends A and backup B is enabled before the
/// proof lands, and B's namespace, master key and passkey stay exactly as B's Enable left them
#[test]
fn background_key_recovery_from_an_ended_backup_leaves_the_next_backups_keys() {
    let test = concat!(
        module_path!(),
        "::background_key_recovery_from_an_ended_backup_leaves_the_next_backups_keys"
    );
    run_case(test, &[PhaseEnd::Exit], |_| {
        assert_held_recovery_leaves_the_next_backups_keys(|app| {
            app.event("a wallet change needs an upload", |app| {
                change_a_wallet(app).expect("a local wallet");
            })
            .then_can(&[N::TurnOff]);
            app.advance_clock(Duration::from_secs(60)).then_can(&[N::TurnOff]);
        });
    });
}

/// The same for a manual Verify: its claim ends with the reset, so the master key and passkey it
/// proves for A after B's Enable are refused
#[test]
fn verification_key_recovery_from_an_ended_backup_leaves_the_next_backups_keys() {
    let test = concat!(
        module_path!(),
        "::verification_key_recovery_from_an_ended_backup_leaves_the_next_backups_keys"
    );
    run_case(test, &[PhaseEnd::Exit], |_| {
        assert_held_recovery_leaves_the_next_backups_keys(|app| {
            app.dispatch(CloudBackupManagerAction::StartVerification(
                CloudBackupVerificationSource::CloudBackupDetail,
            ))
            .then_cannot(&[N::Enable]);
        });
    });
}

/// Keys background work proved under one configuration never land on the next: after Turn Off
/// and a new Enable, the lane refuses the first configuration's late save and still saves the new
/// configuration's own
#[test]
fn a_turned_off_configurations_late_keychain_save_is_refused() {
    let test =
        concat!(module_path!(), "::a_turned_off_configurations_late_keychain_save_is_refused");
    run_case(test, &[PhaseEnd::Exit], |_| {
        let mut app = TestApp::launch_with_passkey(PasskeyMode::FailOnAnyInteractiveCall);
        let manager = app.manager().clone();
        let first = FakeAccount::new(1, 0).seed(&app);
        let first_key = master_key_bytes().expect("A's master key");
        let first_writes = app.block_on("A's writes", current_background_writes(&manager));
        load_detail(&mut app);

        app.dispatch(CloudBackupManagerAction::DisableCloudBackup)
            .then_can(&[N::Enable])
            .then_cannot(&[N::TurnOff]);
        let second = FakeAccount::new(1, 0).seed(&app);
        let second_key = master_key_bytes().expect("B's master key");
        let second_credential = credential_id();
        assert_ne!(first, second);
        assert_ne!(first_key, second_key);

        // A's worker answers late with A's keys, for B's namespace so only its authority differs
        let late = vec![
            OwnedKeychainWrite::Passkey { credential_id: vec![0xa1; 16], prf_salt: [0xa2; 32] },
            OwnedKeychainWrite::MasterKey(master_key(&first_key)),
        ];
        let refused =
            app.block_on("A's late save", first_writes.save_keychain(second.clone(), late));

        assert!(
            matches!(
                refused,
                Err(CloudBackupError::WritesPaused(PauseReason::ConfigurationChanged))
            ),
            "{refused:?}"
        );
        assert_eq!(keychain_namespace(), Some(second.clone()));
        assert_eq!(master_key_bytes(), Some(second_key.clone()));
        assert_eq!(credential_id(), second_credential);

        // B's own background work still saves its keys
        let second_writes = app.block_on("B's writes", current_background_writes(&manager));
        let own = vec![OwnedKeychainWrite::MasterKey(master_key(&second_key))];
        let saved = app.block_on("B's save", second_writes.save_keychain(second, own));
        assert!(saved.is_ok(), "{saved:?}");
    });
}

fn master_key(bytes: &[u8]) -> cove_cspp::master_key::MasterKey {
    cove_cspp::master_key::MasterKey::from_bytes(bytes.try_into().expect("a 32 byte master key"))
}

/// Seeds backup A without a local master key, lets `start` begin a recovery of it whose passkey
/// answer is held, then resets, enables backup B and releases A's answer
fn assert_held_recovery_leaves_the_next_backups_keys(start: impl FnOnce(&mut TestApp)) {
    let mut app = TestApp::launch();
    let namespace = FakeAccount::new(1, 0).seed(&app);
    seed_verifiable_master_key(&app, &namespace);
    load_detail(&mut app);
    CloudBackupKeychain::global().delete_master_key().expect("delete local master key");
    let hold = Hold::default();
    app.passkey.set_mode(PasskeyMode::Slow(hold.clone()));

    start(&mut app);
    app.block_on("A's key recovery to authenticate", hold.entered());

    let manager = app.manager().clone();
    let permit = app
        .block_on("the reset to be ready", async move { manager.prepare_local_reset().await })
        .expect("reset permit");
    finish_wipe(&app, permit);

    let next = FakeAccount::new(1, 0).seed(&app);
    let next_master_key = master_key_bytes();
    let next_credential = credential_id();
    assert_ne!(next, namespace);
    assert!(next_master_key.is_some(), "B's Enable left no master key");

    app.event("release A's held authentication", |_| hold.release()).then_can(&[N::TurnOff]);
    app.advance_clock(Duration::from_secs(60)).then_can(&[N::TurnOff]);
    assert_eq!(keychain_namespace(), Some(next), "{}", app.describe());
    assert_eq!(master_key_bytes(), next_master_key, "A's master key replaced B's");
    assert_eq!(credential_id(), next_credential, "A's passkey replaced B's");
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

fn persisted_switch_phase() -> Option<PersistedDriveAccountSwitchPhase> {
    persisted().drive_account_switch().map(|transition| transition.phase)
}

/// A reset revokes the running reinitialization of a Google Drive account switch. When the wipe
/// fails before its commit, the switch is still recorded and its recovery resumes it in a new
/// run. The revoked run's completion, released after that, belongs to no run: it can't end the
/// resumed run or move the switch on
#[test]
fn a_switch_run_a_reset_revoked_cannot_answer_for_the_run_that_resumed_the_switch() {
    let test = concat!(
        module_path!(),
        "::a_switch_run_a_reset_revoked_cannot_answer_for_the_run_that_resumed_the_switch"
    );
    run_case(test, &[PhaseEnd::Exit], |_| {
        let mut app = TestApp::launch();
        FakeAccount::new(1, 0).seed(&app);
        // without a local master key the reinitialize first lists the cloud's namespaces, a
        // provider read that a reset cancels by revoking the run's claim
        CloudBackupKeychain::global().delete_master_key().expect("delete local master key");

        let manager = app.manager().clone();
        let transition_id = app
            .block_on("the switch to drain the lane", manager.begin_drive_account_switch())
            .expect("the switch starts");
        let revoked_listing = Hold::default();
        app.cloud.script_next(CloudCall::ListNamespaces, CloudFault::Hold(revoked_listing.clone()));

        app.event("the platform staged the selected account", |app| {
            let manager = app.manager().clone();
            app.block_on(
                "the switch to continue",
                manager.continue_drive_account_switch(transition_id),
            )
            .expect("continue the switch");
        })
        .then_cannot(&[N::Enable]);
        app.block_on("the first run's listing to start", revoked_listing.entered());
        let revoked = app.manager().projected_exclusive_operation().expect("the first run");

        let manager = app.manager().clone();
        let permit = app
            .block_on("the reset to be ready", async move { manager.prepare_local_reset().await })
            .expect("reset permit");
        app.event("the wipe fails before its commit", |app| {
            app.block_on("the reset to end", permit.resume_after_failed_reset())
                .expect("abandon the reset");
        })
        .then_cannot(&[N::Enable]);
        assert_eq!(
            persisted_switch_phase(),
            Some(PersistedDriveAccountSwitchPhase::Reinitializing)
        );

        let resumed_listing = Hold::default();
        app.cloud.script_next(CloudCall::ListNamespaces, CloudFault::Hold(resumed_listing.clone()));
        app.event("the app reconciles the switch the platform still stages", |app| {
            let manager = app.manager().clone();
            let action = app.block_on(
                "the reconcile to answer",
                manager.reconcile_drive_account_switch(DriveAccountSwitchPlatformState::Staged(
                    transition_id,
                )),
            );
            assert_eq!(action, Ok(DriveAccountSwitchReconcileAction::None));
        })
        .then_cannot(&[N::Enable]);
        app.block_on("the resumed run's listing to start", resumed_listing.entered());
        let resumed = app.manager().projected_exclusive_operation().expect("the resumed run");
        assert_eq!(resumed.drive_account_switch_id(), revoked.drive_account_switch_id());
        assert_ne!(resumed, revoked, "the resumed run reused the revoked run's identity");

        app.event("release the revoked run's listing", |_| revoked_listing.release())
            .then_cannot(&[N::Enable]);
        app.block_on("the revoked run's listing to return", revoked_listing.finished());
        assert_eq!(
            app.manager().projected_exclusive_operation(),
            Some(resumed),
            "the revoked run's completion ended the resumed run\n{}",
            app.describe()
        );
        assert_eq!(
            persisted_switch_phase(),
            Some(PersistedDriveAccountSwitchPhase::Reinitializing)
        );

        app.event("release the resumed run's listing", |_| resumed_listing.release())
            .then_cannot(&[N::Enable]);
        assert_ne!(
            persisted_switch_phase(),
            Some(PersistedDriveAccountSwitchPhase::Reinitializing),
            "the resumed run never reported\n{}",
            app.describe()
        );
    });
}

/// A reset that is abandoned before its wipe leaves the interrupted Enable's evidence in the
/// keychain, so the recovery that owns that evidence comes back: it shows through the reset and
/// after it, and the recovery's cleanup still ends it
#[test]
fn abandoned_reset_keeps_the_pending_enable_recovery() {
    let test = concat!(module_path!(), "::abandoned_reset_keeps_the_pending_enable_recovery");
    run_case(test, &[PhaseEnd::Exit], |_| {
        let mut app = TestApp::launch();
        let cspp = cove_cspp::Cspp::new(Keychain::global().clone());

        // staged material without its journal is evidence restart can only hand to recovery
        cspp.save_staged_master_key(&cove_cspp::master_key::MasterKey::generate())
            .expect("stage a master key");
        app.relaunch().then_can(&[N::Enable]).then_cannot(&[N::TurnOff]);

        let recovery = match app.manager().state().lifecycle {
            CloudBackupLifecycle::PendingEnableRecovery(recovery) => recovery,
            lifecycle => panic!("not in recovery after restart: {lifecycle:?}"),
        };
        assert_eq!(app.runtime_recovery(), Some(recovery.clone()));

        let manager = app.manager().clone();
        let permit = app
            .block_on("the reset to be ready", async move { manager.prepare_local_reset().await })
            .expect("reset permit");

        // the reset owns the runtime, and the recovery still shows
        app.current().then_cannot(&[N::Enable, N::TurnOff]);
        assert_eq!(
            app.manager().state().lifecycle,
            CloudBackupLifecycle::PendingEnableRecovery(recovery.clone())
        );

        app.event("the wipe fails before its commit", |app| {
            app.block_on("the reset to end", permit.resume_after_failed_reset())
                .expect("abandon the reset");
        })
        .then_can(&[N::Enable])
        .then_cannot(&[N::TurnOff]);

        assert_eq!(
            app.manager().state().lifecycle,
            CloudBackupLifecycle::PendingEnableRecovery(recovery.clone()),
            "{}",
            app.describe()
        );
        assert_eq!(app.runtime_recovery(), Some(recovery), "{}", app.describe());
        assert_eq!(
            cspp.master_key_promotion_status().expect("read"),
            MasterKeyPromotionStatus::Staged
        );

        app.dispatch(CloudBackupManagerAction::ConfirmPendingEnableCleanup)
            .then_can(&[N::Enable])
            .then_cannot(&[N::TurnOff]);
        assert_eq!(
            cspp.master_key_promotion_status().expect("read"),
            MasterKeyPromotionStatus::None
        );
        assert_eq!(app.manager().state().lifecycle, CloudBackupLifecycle::Disabled);
    });
}
