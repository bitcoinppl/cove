//! A paused Enable or reinitialize goes on, or is discarded, only in the lifecycle it belongs to
//!
//! Each test body runs in child processes; see [`super::run_case`]
#![cfg(test)]

use std::time::Duration;

use act_zero::call;
use cove_cspp::Cspp;
use cove_cspp::master_key::MasterKey;
use cove_cspp::master_key_crypto::encrypt_master_key;
use cove_device::cloud_storage::CloudStorageError;
use cove_device::keychain::{Keychain, KeychainAccess};
use cove_device::passkey::{DiscoveredPasskeyResult, PasskeyError};

use super::{
    CloudCall, CloudFault, FakeAccount, Hold, NextAction as N, PasskeyMode, PhaseEnd, TestApp,
    assert_uploads_confirmed_in, change_a_wallet, keychain_namespace, launch_keychain, load_detail,
    persisted, run_case, seed_an_uploaded_wallet, seed_legacy_stuck_disabling,
    seed_wallet_backup_elsewhere,
};
use crate::database::cloud_backup::{PersistedCloudBackupState, PersistedCloudBackupStatus};
use crate::manager::cloud_backup_manager::actors::write::Decision;
use crate::manager::cloud_backup_manager::admission::{
    Admission, EnableIntent, Rejection, UserIntent,
};
use crate::manager::cloud_backup_manager::keychain::CSPP_PENDING_ENABLE_JOURNAL_KEY;
use crate::manager::cloud_backup_manager::model::{
    CloudBackupExclusiveOperation, CloudBackupLifecycle,
};
use crate::manager::cloud_backup_manager::owner::lifecycle::test_support::bootstrap_with_failed_heal;
use crate::manager::cloud_backup_manager::pending_enable::PendingEnableJournal;
use crate::manager::cloud_backup_manager::screen::{
    CloudBackupActionAvailability, CloudBackupOutcome, CloudBackupOutcomeTarget,
    CloudBackupSavedPasskeyControl as Control, CloudBackupSavedPasskeyLifecycle,
    CloudBackupScreenAction as A, CloudBackupScreenBody,
};
use crate::manager::cloud_backup_manager::{
    CloudBackupEnableContext, CloudBackupEnablePromptChoice, CloudBackupEnableState,
    CloudBackupKeychain, CloudBackupManagerAction, CloudBackupRootPrompt, CloudBackupStatus,
    SavedPasskeyConfirmationMode, VerificationState,
};

const NEW_CREDENTIAL: [u8; 3] = [4, 5, 6];

/// How long a newly registered passkey is given before Cove asks the user to confirm it
const PASSKEY_AVAILABILITY_WAIT: Duration = Duration::from_secs(3);

/// Configures a backup, then runs a reinitialize that registers a new passkey to its pause for
/// the user to confirm it, and returns the configured namespace and credential it would replace
fn pause_a_configured_reinitialize(app: &mut TestApp) -> (String, Option<Vec<u8>>) {
    let old_namespace = FakeAccount::new(1, 0).seed(app);
    let old_credential =
        CloudBackupKeychain::global().load_credential_id().expect("read credential");
    load_detail(app);

    // no passkey is discoverable, so the reinitialize registers one
    app.passkey.inner.set_discover_result(Err(PasskeyError::NoCredentialFound));
    app.passkey.inner.set_create_result(Ok(NEW_CREDENTIAL.to_vec()));

    app.dispatch(CloudBackupManagerAction::ReinitializeBackup)
        .then_cannot(&[N::TurnOff, N::Verify]);
    app.advance_clock(PASSKEY_AVAILABILITY_WAIT).then_cannot(&[N::Enable, N::TurnOff, N::Verify]);

    // paused: nothing was replaced, no reinitialize shows running, and the confirmation waits
    assert_eq!(app.passkey.inner.create_count(), 1);
    assert_eq!(app.passkey.inner.authenticate_count(), 0);
    assert_eq!(keychain_namespace().as_deref(), Some(old_namespace.as_str()));
    assert_eq!(
        app.manager().model_snapshot().enable_state,
        CloudBackupEnableState::AwaitingSavedPasskeyConfirmation(
            SavedPasskeyConfirmationMode::Manual
        )
    );
    assert!(!app.runs_operation(), "{}", app.describe());

    // the pause owns the screen, and offers both ways out of it
    let paused = app
        .saved_passkey_confirmation()
        .unwrap_or_else(|| panic!("the paused reinitialize is not on screen\n{}", app.describe()));
    assert_eq!(paused.lifecycle, CloudBackupSavedPasskeyLifecycle::Reinitialize);
    assert_eq!(paused.controls, vec![Control::Confirm, Control::Discard]);

    (old_namespace, old_credential)
}

/// A reinitialize that registers a new passkey pauses for the user to confirm it, and the pause
/// owns the screen with its Confirm and Discard. Confirm, tapped there, continues it as the
/// reinitialize of the configured backup, which uploads the new backup, switches to it and
/// verifies it
#[test]
fn saved_passkey_confirmation_continues_a_configured_reinitialize() {
    let test =
        concat!(module_path!(), "::saved_passkey_confirmation_continues_a_configured_reinitialize");
    run_case(test, &[PhaseEnd::Exit], |_| {
        let mut app = TestApp::launch();
        let (old_namespace, _) = pause_a_configured_reinitialize(&mut app);

        app.passkey.inner.set_authenticate_result(Ok(vec![7; 32]));
        app.tap_saved_passkey(Control::Confirm)
            .then_can(&[N::TurnOff, N::Verify])
            .then_cannot(&[N::Enable]);
        assert_eq!(app.saved_passkey_confirmation(), None, "{}", app.describe());

        let new_namespace = keychain_namespace().expect("a configured namespace");
        assert_ne!(new_namespace, old_namespace, "{}", app.describe());
        assert!(app.cloud.inner.has_namespace(&new_namespace));
        // the confirmation and the verification of the new backup both authenticate the new
        // credential; nothing authenticates the replaced one or registers another
        let authenticated = app.passkey.inner.authenticated_credential_ids();
        assert!(!authenticated.is_empty());
        assert!(
            authenticated.iter().all(|id| id == NEW_CREDENTIAL.as_slice()),
            "{authenticated:?}"
        );
        assert_eq!(app.passkey.inner.create_count(), 1);
        assert_eq!(
            CloudBackupKeychain::global().load_credential_id().expect("read credential"),
            Some(NEW_CREDENTIAL.to_vec())
        );
        assert_eq!(persisted().status(), PersistedCloudBackupStatus::Enabled);
        assert_eq!(app.manager().current_status(), CloudBackupStatus::Enabled);
        assert!(
            matches!(app.manager().model_snapshot().verification, VerificationState::Verified(_)),
            "{}",
            app.describe()
        );
        assert_eq!(
            app.admission_of(UserIntent::ConfirmSavedPasskey),
            Admission::Rejected(Rejection::NothingToContinue)
        );
    });
}

/// Discard, tapped on a paused reinitialize, ends only the reinitialize: the backup stays
/// configured on its namespace and passkey, as stored, and nothing confirms the new passkey
#[test]
fn discard_of_a_paused_reinitialize_keeps_the_current_backup() {
    let test =
        concat!(module_path!(), "::discard_of_a_paused_reinitialize_keeps_the_current_backup");
    run_case(test, &[PhaseEnd::Exit], |_| {
        let mut app = TestApp::launch();
        let (old_namespace, old_credential) = pause_a_configured_reinitialize(&mut app);
        let record = persisted();

        app.tap_saved_passkey(Control::Discard)
            .then_can(&[N::TurnOff, N::Verify])
            .then_cannot(&[N::Enable]);

        assert_eq!(app.saved_passkey_confirmation(), None, "{}", app.describe());
        assert!(
            matches!(app.manager().published_screen().body, CloudBackupScreenBody::Configured(_)),
            "{}",
            app.describe()
        );
        assert_eq!(keychain_namespace().as_deref(), Some(old_namespace.as_str()));
        assert!(app.cloud.inner.has_namespace(&old_namespace));
        assert_eq!(
            CloudBackupKeychain::global().load_credential_id().expect("read credential"),
            old_credential
        );
        assert_eq!(persisted(), record);
        assert_eq!(app.manager().current_status(), CloudBackupStatus::Enabled);
        assert_eq!(app.passkey.inner.authenticate_count(), 0);
        assert_eq!(
            app.admission_of(UserIntent::ConfirmSavedPasskey),
            Admission::Rejected(Rejection::NothingToContinue)
        );
    });
}

/// A Discard from the Enable prompt that arrives after Enable finished has nothing left to end:
/// admission rejects it before it changes anything, so the backup stays on in what the screen
/// shows, what is stored and what the write lane runs
#[test]
fn discard_after_enable_finished_is_rejected_and_changes_nothing() {
    let test =
        concat!(module_path!(), "::discard_after_enable_finished_is_rejected_and_changes_nothing");
    run_case(test, &[PhaseEnd::Exit], |_| {
        let mut app = TestApp::launch();
        app.passkey.inner.set_discover_result(Ok(DiscoveredPasskeyResult {
            prf_output: vec![7; 32],
            credential_id: vec![1, 2, 3],
        }));

        app.dispatch(CloudBackupManagerAction::EnableCloudBackup(
            CloudBackupEnableContext::settings_manual(),
        ))
        .then_can(&[N::TurnOff])
        .then_cannot(&[N::Enable]);

        assert_late_discard_changes_nothing(&mut app);
    });
}

/// The same late Discard after an Enable that registered a passkey and finished through Confirm
/// Saved Passkey: the confirmation consumed the continuation, so nothing is left to discard
#[test]
fn discard_after_a_confirmed_enable_is_rejected_and_changes_nothing() {
    let test = concat!(
        module_path!(),
        "::discard_after_a_confirmed_enable_is_rejected_and_changes_nothing"
    );
    run_case(test, &[PhaseEnd::Exit], |_| {
        let mut app = TestApp::launch();
        app.passkey.inner.set_create_result(Ok(NEW_CREDENTIAL.to_vec()));

        app.dispatch(CloudBackupManagerAction::EnableCloudBackupNoDiscovery(
            CloudBackupEnableContext::settings_manual(),
        ))
        .then_cannot(&[N::TurnOff]);
        app.advance_clock(PASSKEY_AVAILABILITY_WAIT).then_cannot(&[N::TurnOff]);
        assert_eq!(
            app.manager().model_snapshot().enable_state,
            CloudBackupEnableState::AwaitingSavedPasskeyConfirmation(
                SavedPasskeyConfirmationMode::Manual
            ),
            "{}",
            app.describe()
        );
        let paused = app
            .saved_passkey_confirmation()
            .unwrap_or_else(|| panic!("the paused Enable is not on screen\n{}", app.describe()));
        assert_eq!(paused.lifecycle, CloudBackupSavedPasskeyLifecycle::Enable);
        assert_eq!(paused.controls, vec![Control::Confirm, Control::Discard]);

        app.passkey.inner.set_authenticate_result(Ok(vec![7; 32]));
        app.tap_saved_passkey(Control::Confirm).then_can(&[N::TurnOff]).then_cannot(&[N::Enable]);

        assert_late_discard_changes_nothing(&mut app);
    });
}

/// Dispatches a Discard to a backup that Enable already turned on, and checks admission rejected
/// it and the screen, the stored record and the write lane are as they were
fn assert_late_discard_changes_nothing(app: &mut TestApp) {
    assert_eq!(app.manager().current_status(), CloudBackupStatus::Enabled);
    let record = persisted();
    let namespace = keychain_namespace().expect("a configured namespace");

    app.dispatch(CloudBackupManagerAction::DiscardPendingEnableCloudBackup)
        .then_can(&[N::TurnOff])
        .then_cannot(&[N::Enable]);

    let outcome = app.last_outcome().expect("the Discard's admission");
    assert_eq!(outcome.intent, UserIntent::DiscardPendingEnable);
    assert_eq!(outcome.admission, Admission::Rejected(Rejection::NothingToContinue));

    // displayed
    assert_eq!(app.manager().current_status(), CloudBackupStatus::Enabled);
    assert!(matches!(app.manager().model_state().lifecycle, CloudBackupLifecycle::Configured(_)));

    // durable
    assert!(matches!(record, PersistedCloudBackupState::Configured(_)), "{record:?}");
    assert_eq!(persisted(), record);
    assert_eq!(keychain_namespace().as_deref(), Some(namespace.as_str()));

    // write lane
    let writes = app.manager().cloud_writes.clone();
    let ticket = app
        .block_on("the lane's ticket", async move { call!(writes.ticket()).await })
        .expect("ticket");
    assert_eq!(ticket.ordinary, Decision::Run);
}

/// The existing-backup prompt's choices are controls like any other: each has its own entry with
/// admission's decision, which the platform's prompt binds its button to, and answering the
/// prompt takes both away with the prompt
#[test]
fn the_existing_backup_prompt_offers_each_choice_as_its_own_entry() {
    let test =
        concat!(module_path!(), "::the_existing_backup_prompt_offers_each_choice_as_its_own_entry");
    run_case(test, &[PhaseEnd::Exit], |_| {
        let mut app = TestApp::launch();
        let existing = MasterKey::generate();
        let namespace = existing.namespace_id();
        let encrypted = encrypt_master_key(&existing, &[7; 32], &[9; 32]).expect("encrypt");
        app.cloud.inner.set_wallet_files(namespace.clone(), vec!["wallet-1.json".into()]);
        app.cloud.inner.set_master_key_backup(
            namespace,
            serde_json::to_vec(&encrypted).expect("encode master key"),
        );

        app.dispatch(CloudBackupManagerAction::EnableCloudBackupNoDiscovery(
            CloudBackupEnableContext::settings_manual(),
        ))
        .then_cannot(&[N::TurnOff]);

        let screen = app.manager().published_screen();
        assert!(
            matches!(screen.root_prompt, CloudBackupRootPrompt::ExistingBackupFound(..)),
            "{}",
            app.describe()
        );
        for choice in [A::AcceptEnablePromptUseExisting, A::AcceptEnablePromptCreateNew] {
            assert_eq!(
                screen.actions.availability(choice),
                Some(CloudBackupActionAvailability::Available),
                "{choice:?}"
            );
        }

        app.dispatch(CloudBackupManagerAction::AcceptEnablePrompt(
            CloudBackupEnablePromptChoice::CreateNew,
        ))
        .then_cannot(&[N::TurnOff]);

        let screen = app.manager().published_screen();
        assert_eq!(screen.root_prompt, CloudBackupRootPrompt::None, "{}", app.describe());
        assert_eq!(screen.actions.availability(A::AcceptEnablePromptUseExisting), None);
        assert_eq!(screen.actions.availability(A::AcceptEnablePromptCreateNew), None);
    });
}

/// Dismissing the existing-backup prompt is a Discard of an Enable that holds no material: it
/// clears the prompt and leaves the backup off, and the existing backup stays in the cloud
#[test]
fn discard_from_the_existing_backup_prompt_clears_it() {
    let test = concat!(module_path!(), "::discard_from_the_existing_backup_prompt_clears_it");
    run_case(test, &[PhaseEnd::Exit], |_| {
        let mut app = TestApp::launch();
        let existing = MasterKey::generate();
        let namespace = existing.namespace_id();
        let encrypted = encrypt_master_key(&existing, &[7; 32], &[9; 32]).expect("encrypt");
        app.cloud.inner.set_wallet_files(namespace.clone(), vec!["wallet-1.json".into()]);
        app.cloud.inner.set_master_key_backup(
            namespace.clone(),
            serde_json::to_vec(&encrypted).expect("encode master key"),
        );

        let context = CloudBackupEnableContext::settings_manual();
        app.dispatch(CloudBackupManagerAction::EnableCloudBackupNoDiscovery(context))
            .then_cannot(&[N::TurnOff]);
        assert!(
            matches!(
                app.manager().model_snapshot().root_prompt,
                CloudBackupRootPrompt::ExistingBackupFound(..)
            ),
            "{}",
            app.describe()
        );

        // the prompt holds no material, so its Discard reads no journal and a keychain that can't
        // be read doesn't keep the prompt
        app.keychain.inner.fail_reads_for(CSPP_PENDING_ENABLE_JOURNAL_KEY);
        app.dispatch(CloudBackupManagerAction::DiscardPendingEnableCloudBackup)
            .then_can(&[N::Enable])
            .then_cannot(&[N::TurnOff]);
        app.keychain.inner.clear_read_failures();

        let outcome = app.last_outcome().expect("the Discard's admission");
        assert_eq!(outcome.admission, Admission::Accepted);
        assert_eq!(app.manager().model_snapshot().root_prompt, CloudBackupRootPrompt::None);
        assert_eq!(app.manager().current_status(), CloudBackupStatus::Disabled);
        assert!(app.cloud.inner.has_namespace(&namespace));
        assert_eq!(app.passkey.inner.create_count(), 0);
    });
}

/// A reinitialize moves the backup to a new namespace under a new master key, and every wallet
/// it uploads is confirmed there, though the wallet's upload row began in the replaced namespace:
/// a row left naming that one has its confirmation read the replaced backup, which the new key
/// can't decrypt, and fails for good
#[test]
fn a_reinitialize_confirms_its_uploads_in_the_new_namespace() {
    let test =
        concat!(module_path!(), "::a_reinitialize_confirms_its_uploads_in_the_new_namespace");
    run_case(test, &[PhaseEnd::Exit], |_| {
        let mut app = TestApp::launch();
        let old_namespace = seed_an_uploaded_wallet(&mut app);

        app.passkey.inner.set_discover_result(Err(PasskeyError::NoCredentialFound));
        app.passkey.inner.set_create_result(Ok(NEW_CREDENTIAL.to_vec()));
        app.dispatch(CloudBackupManagerAction::ReinitializeBackup)
            .then_cannot(&[N::TurnOff, N::Verify]);
        app.advance_clock(PASSKEY_AVAILABILITY_WAIT).then_cannot(&[
            N::Enable,
            N::TurnOff,
            N::Verify,
        ]);
        app.passkey.inner.set_authenticate_result(Ok(vec![7; 32]));
        app.tap_saved_passkey(Control::Confirm).then_can(&[N::TurnOff, N::Verify]);
        app.advance_clock(Duration::from_secs(60)).then_can(&[N::TurnOff]);

        let new_namespace = keychain_namespace().expect("a configured namespace");
        assert_ne!(new_namespace, old_namespace, "{}", app.describe());
        assert_uploads_confirmed_in(&app, &new_namespace);
    });
}

/// A wallet that changes while a reinitialize uploads it keeps its row from the replaced
/// namespace, since the reinitialize uploaded an older revision. Its own upload, once the
/// reinitialize ends, goes to the new namespace and is confirmed there
#[test]
fn a_change_during_a_reinitialize_uploads_and_confirms_in_the_new_namespace() {
    let test = concat!(
        module_path!(),
        "::a_change_during_a_reinitialize_uploads_and_confirms_in_the_new_namespace"
    );
    run_case(test, &[PhaseEnd::Exit], |_| {
        let mut app = TestApp::launch();
        let old_namespace = seed_an_uploaded_wallet(&mut app);

        app.passkey.inner.set_discover_result(Err(PasskeyError::NoCredentialFound));
        app.passkey.inner.set_create_result(Ok(NEW_CREDENTIAL.to_vec()));
        app.dispatch(CloudBackupManagerAction::ReinitializeBackup)
            .then_cannot(&[N::TurnOff, N::Verify]);
        app.advance_clock(PASSKEY_AVAILABILITY_WAIT).then_cannot(&[
            N::Enable,
            N::TurnOff,
            N::Verify,
        ]);

        // the confirmed reinitialize holds on its wallet upload while the wallet changes
        let hold = Hold::default();
        app.cloud.script_next(CloudCall::UploadWallet, CloudFault::Hold(hold.clone()));
        app.passkey.inner.set_authenticate_result(Ok(vec![7; 32]));
        app.tap_saved_passkey(Control::Confirm).then_cannot(&[N::Enable]);
        app.block_on("the reinitialize's wallet upload to be held", hold.entered());
        app.event("a local wallet changed", |app| {
            change_a_wallet(app).expect("a seeded wallet changed");
        })
        .then_cannot(&[N::Enable]);

        app.event("the held upload completes", |_| hold.release()).then_cannot(&[N::Enable]);
        app.advance_clock(Duration::from_secs(60)).then_can(&[N::TurnOff]);

        let new_namespace = keychain_namespace().expect("a configured namespace");
        assert_ne!(new_namespace, old_namespace, "{}", app.describe());
        assert_uploads_confirmed_in(&app, &new_namespace);
    });
}

/// What the passkey of the backup a test restores answers with
const RESTORED_PRF: [u8; 32] = [11; 32];
const RESTORED_SALT: [u8; 32] = [9; 32];
const RESTORED_CREDENTIAL: [u8; 3] = [1, 2, 3];

/// Puts a backup in the cloud that Restore finds through a discoverable passkey, and returns its
/// master key
fn a_backup_to_restore(app: &TestApp) -> MasterKey {
    let master_key = MasterKey::generate();
    let namespace = master_key.namespace_id();
    let encrypted =
        encrypt_master_key(&master_key, &RESTORED_PRF, &RESTORED_SALT).expect("encrypt");
    app.cloud.inner.set_master_key_backup(
        namespace.clone(),
        serde_json::to_vec(&encrypted).expect("encode master key"),
    );
    seed_wallet_backup_elsewhere(app, &namespace, &master_key);
    app.passkey.inner.set_discover_result(Ok(DiscoveredPasskeyResult {
        prf_output: RESTORED_PRF.to_vec(),
        credential_id: RESTORED_CREDENTIAL.to_vec(),
    }));
    app.passkey.inner.set_authenticate_result(Ok(RESTORED_PRF.to_vec()));
    master_key
}

/// The restored backup is configured on this device as restored: its namespace, passkey
/// credential and salt, and its master key active
fn assert_restored_backup_kept(app: &TestApp, restored: &MasterKey) {
    let keychain = CloudBackupKeychain::global();
    assert_eq!(keychain_namespace(), Some(restored.namespace_id()), "{}", app.describe());
    assert_eq!(
        keychain.load_credential_id().expect("read credential"),
        Some(RESTORED_CREDENTIAL.to_vec())
    );
    assert_eq!(keychain.load_prf_salt().expect("read salt"), Some(RESTORED_SALT));
    let active = Cspp::new(Keychain::global().clone())
        .load_master_key_from_store()
        .expect("read the active master key")
        .expect("an active master key");
    assert_eq!(active.as_bytes(), restored.as_bytes());
    assert!(matches!(persisted(), PersistedCloudBackupState::Configured(_)), "{}", app.describe());
}

/// Pauses a fresh Enable on the passkey it registered
fn pause_a_fresh_enable(app: &mut TestApp) {
    app.passkey.inner.set_discover_result(Err(PasskeyError::NoCredentialFound));
    app.passkey.inner.set_create_result(Ok(NEW_CREDENTIAL.to_vec()));
    app.dispatch(CloudBackupManagerAction::EnableCloudBackup(
        CloudBackupEnableContext::settings_manual(),
    ))
    .then_cannot(&[N::TurnOff]);
    app.advance_clock(PASSKEY_AVAILABILITY_WAIT).then_cannot(&[N::TurnOff]);
    assert_paused_enable_holds_the_device(app);
}

/// A fresh Enable paused on its saved passkey holds the device: its Confirm and Discard are its
/// ways on, its journal records the registered passkey, and Restore can't replace the
/// configuration that Discard would put back
fn assert_paused_enable_holds_the_device(app: &TestApp) {
    let paused = app
        .saved_passkey_confirmation()
        .unwrap_or_else(|| panic!("the paused Enable is not on screen\n{}", app.describe()));
    assert_eq!(paused.lifecycle, CloudBackupSavedPasskeyLifecycle::Enable);
    assert_eq!(paused.controls, vec![Control::Confirm, Control::Discard]);
    assert!(
        pending_enable_journal().is_some_and(|journal| journal.awaits_saved_passkey()),
        "{}",
        app.describe()
    );
    assert_eq!(
        app.admission_of(UserIntent::RestoreFromCloudBackup),
        Admission::Rejected(Rejection::OperationRunning(CloudBackupExclusiveOperation::Enable))
    );
}

fn pending_enable_journal() -> Option<PendingEnableJournal> {
    CloudBackupKeychain::global().load_pending_enable_journal().expect("the journal loads")
}

/// A reset that fails before its wipe, after it dropped what Cloud Backup kept in memory
fn abandon_a_reset(app: &mut TestApp) {
    let manager = app.manager().clone();
    let permit = app
        .block_on("the reset to be ready", async move { manager.prepare_local_reset().await })
        .expect("reset permit");
    app.event("the wipe fails before its commit", |app| {
        app.block_on("the reset to end", permit.resume_after_failed_reset())
            .expect("abandon the reset");
    })
    .then_cannot(&[N::TurnOff]);
}

/// Model review sequence: a fresh Enable paused on its saved passkey, a reset that fails before
/// its wipe, a Restore of another backup, and a Discard of that backup's own reinitialize
///
/// The reset drops the pause's material in memory, but the journal records the pause, and the
/// abandoned reset takes it up again as a relaunch does. So Restore can't put another backup
/// under the Enable's journal, whose Discard would put back the empty metadata it snapshotted.
/// Once the pause's own Discard ends it, the backup is restored, and a Discard of that backup's
/// paused reinitialize keeps its namespace, passkey, key and record as restored
#[test]
fn a_restore_waits_for_a_paused_enable_a_failed_reset_left() {
    let test = concat!(module_path!(), "::a_restore_waits_for_a_paused_enable_a_failed_reset_left");
    run_case(test, &[PhaseEnd::Exit], |_| {
        let mut app = TestApp::launch();
        pause_a_fresh_enable(&mut app);
        let restored = a_backup_to_restore(&app);

        abandon_a_reset(&mut app);
        assert_paused_enable_holds_the_device(&app);
        app.dispatch(CloudBackupManagerAction::RestoreFromCloudBackup).then_cannot(&[N::TurnOff]);
        assert_eq!(keychain_namespace(), None, "{}", app.describe());
        assert!(
            !matches!(persisted(), PersistedCloudBackupState::Configured(_)),
            "{}",
            app.describe()
        );

        // the pause's own Discard ends it, and Restore then sets up the other backup
        app.tap_saved_passkey(Control::Discard).then_can(&[N::Enable]).then_cannot(&[N::TurnOff]);
        assert_eq!(pending_enable_journal(), None, "{}", app.describe());
        app.dispatch(CloudBackupManagerAction::RestoreFromCloudBackup).then_cannot(&[N::Enable]);
        app.quiesce().then_can(&[N::TurnOff]).then_cannot(&[N::Enable]);
        assert_restored_backup_kept(&app, &restored);

        // the restored backup's own reinitialize pauses, and its Discard keeps it as restored
        load_detail(&mut app);
        // no passkey is discoverable, so the reinitialize registers one
        app.passkey.inner.set_discover_result(Err(PasskeyError::NoCredentialFound));
        app.passkey.inner.set_create_result(Ok(NEW_CREDENTIAL.to_vec()));
        app.dispatch(CloudBackupManagerAction::ReinitializeBackup)
            .then_cannot(&[N::TurnOff, N::Verify]);
        app.advance_clock(PASSKEY_AVAILABILITY_WAIT).then_cannot(&[N::Enable, N::TurnOff]);
        let paused = app
            .saved_passkey_confirmation()
            .unwrap_or_else(|| panic!("the reinitialize did not pause\n{}", app.describe()));
        assert_eq!(paused.lifecycle, CloudBackupSavedPasskeyLifecycle::Reinitialize);

        app.tap_saved_passkey(Control::Discard).then_can(&[N::TurnOff]).then_cannot(&[N::Enable]);
        assert_restored_backup_kept(&app, &restored);
        assert_eq!(pending_enable_journal(), None, "{}", app.describe());
        assert_eq!(app.passkey.inner.create_count(), 2);
    });
}

/// A confirmed Enable whose upload fails goes back to the pause its journal records
///
/// Nothing stopped is kept outside that pause: the screen offers its Confirm and Discard, and the
/// pause refuses everything that would replace the configuration Discard puts back. Confirm is
/// the retry, asking for the saved passkey once more instead of registering another
#[test]
fn a_failed_upload_goes_back_to_the_saved_passkey_pause() {
    let test = concat!(module_path!(), "::a_failed_upload_goes_back_to_the_saved_passkey_pause");
    run_case(test, &[PhaseEnd::Exit], |_| {
        let mut app = TestApp::launch();
        fail_the_confirmed_upload_of_a_paused_enable(&mut app);

        // the pause refuses each of them, not the body: the rejection names the paused Enable
        let enable =
            UserIntent::Enable(EnableIntent::Standard, CloudBackupEnableContext::settings_manual());
        for intent in [
            UserIntent::RestoreFromCloudBackup,
            enable,
            UserIntent::TurnOff,
            UserIntent::SwitchDriveAccount,
        ] {
            assert_eq!(
                app.admission_of(intent.clone()),
                Admission::Rejected(Rejection::OperationRunning(
                    CloudBackupExclusiveOperation::Enable
                )),
                "{intent:?} ran beside the paused Enable\n{}",
                app.describe()
            );
        }

        confirm_finishes_the_paused_enable(&mut app);
    });
}

/// Confirms a fresh Enable paused on its saved passkey, whose upload then fails
fn fail_the_confirmed_upload_of_a_paused_enable(app: &mut TestApp) {
    pause_a_fresh_enable(app);
    app.passkey.inner.set_authenticate_result(Ok(vec![7; 32]));
    let offline = CloudStorageError::NotAvailable("offline".into());
    app.cloud.script_next(CloudCall::UploadMasterKey, CloudFault::Fail(offline));
    app.tap_saved_passkey(Control::Confirm).then_cannot(&[N::TurnOff, N::Enable]);
    assert!(!app.runs_operation(), "{}", app.describe());
    assert_paused_enable_holds_the_device(app);
}

/// The paused Enable's confirmation is on screen with both its ways out
fn assert_paused_enable_on_screen(app: &TestApp) {
    let paused = app
        .saved_passkey_confirmation()
        .unwrap_or_else(|| panic!("the paused Enable is not on screen\n{}", app.describe()));
    assert_eq!(paused.lifecycle, CloudBackupSavedPasskeyLifecycle::Enable);
    assert_eq!(paused.controls, vec![Control::Confirm, Control::Discard]);
}

/// Confirm finishes the paused Enable with one more tap of the passkey it registered
fn confirm_finishes_the_paused_enable(app: &mut TestApp) {
    let authenticated = app.passkey.inner.authenticate_count();
    let created = app.passkey.inner.create_count();
    app.tap_saved_passkey(Control::Confirm).then_can(&[N::TurnOff]).then_cannot(&[N::Enable]);
    assert!(matches!(persisted(), PersistedCloudBackupState::Configured(_)), "{}", app.describe());
    assert_eq!(
        CloudBackupKeychain::global().load_credential_id().expect("read credential"),
        Some(NEW_CREDENTIAL.to_vec())
    );
    assert_eq!(app.passkey.inner.authenticate_count(), authenticated + 1);
    assert_eq!(app.passkey.inner.create_count(), created, "a second passkey was registered");
    assert_eq!(pending_enable_journal(), None, "{}", app.describe());
}

/// A journal the keychain can't read is never a passkey the user saved
///
/// Admission and the screen read the coordinator's view of the journal, which the launch filled
/// in, so an idle backup with no journal stays as it is, on or off, and reads no journal at all
#[test]
fn an_unreadable_journal_never_shows_a_pause() {
    let test = concat!(module_path!(), "::an_unreadable_journal_never_shows_a_pause");
    run_case(test, &[PhaseEnd::Exit], |_| {
        let mut app = TestApp::launch();
        app.keychain.inner.fail_reads_for(CSPP_PENDING_ENABLE_JOURNAL_KEY);
        app.advance_clock(Duration::from_secs(60)).then_can(&[N::Enable]);
        assert_eq!(app.saved_passkey_confirmation(), None, "{}", app.describe());

        app.keychain.inner.clear_read_failures();
        FakeAccount::new(1, 0).seed(&app);
        load_detail(&mut app);
        app.keychain.inner.record_reads();
        app.keychain.inner.fail_reads_for(CSPP_PENDING_ENABLE_JOURNAL_KEY);
        app.event("the screen publishes", |app| app.manager().request_screen_publish())
            .then_can(&[N::TurnOff, N::Verify]);
        assert_eq!(app.saved_passkey_confirmation(), None, "{}", app.describe());
        assert!(
            matches!(app.manager().published_screen().body, CloudBackupScreenBody::Configured(_)),
            "{}",
            app.describe()
        );

        let journal_reads = app
            .keychain
            .inner
            .take_recorded_reads()
            .into_iter()
            .filter(|key| key == CSPP_PENDING_ENABLE_JOURNAL_KEY)
            .count();
        assert_eq!(journal_reads, 0, "an idle screen read the journal\n{}", app.describe());
    });
}

/// A paused Enable whose journal can't be read for a moment stays paused
///
/// After a failed upload no material is in memory, so Confirm and Discard read the journal to
/// take the pause up. While that read fails each shows why next to its control, and nothing
/// replaces the pause; once the keychain reads again, the same Confirm finishes the Enable with
/// no relaunch and no cleanup
#[test]
fn a_pause_outlasts_journal_reads_that_fail() {
    let test = concat!(module_path!(), "::a_pause_outlasts_journal_reads_that_fail");
    run_case(test, &[PhaseEnd::Exit], |_| {
        let mut app = TestApp::launch();
        fail_the_confirmed_upload_of_a_paused_enable(&mut app);
        let journal = pending_enable_journal();

        app.keychain.inner.fail_reads_for(CSPP_PENDING_ENABLE_JOURNAL_KEY);
        let authenticated = app.passkey.inner.authenticate_count();
        for (control, action) in [
            (Control::Confirm, A::ConfirmSavedPasskey),
            (Control::Discard, A::DiscardPendingEnable),
        ] {
            app.tap_saved_passkey(control).then_cannot(&[N::TurnOff, N::Enable]);
            let outcome = app
                .manager()
                .published_screen()
                .last_outcome
                .unwrap_or_else(|| panic!("{control:?} showed no failure\n{}", app.describe()));
            assert_eq!(outcome.target, CloudBackupOutcomeTarget::Action(action));
            assert!(
                matches!(outcome.outcome, CloudBackupOutcome::Failed { .. }),
                "{outcome:?}\n{}",
                app.describe()
            );
            assert_paused_enable_on_screen(&app);
            assert!(app.runtime_recovery().is_none(), "{}", app.describe());
        }
        assert_eq!(app.passkey.inner.authenticate_count(), authenticated);

        app.keychain.inner.clear_read_failures();
        assert_eq!(pending_enable_journal(), journal, "{}", app.describe());
        assert_paused_enable_holds_the_device(&app);
        confirm_finishes_the_paused_enable(&mut app);
    });
}

/// A launch that can't read the journal shows the load failure, not a pause it can't see, and
/// Try Again, once the keychain reads again, gives the pause back with no relaunch
#[test]
fn a_launch_that_cant_read_the_journal_gives_the_pause_back_on_try_again() {
    let test = concat!(
        module_path!(),
        "::a_launch_that_cant_read_the_journal_gives_the_pause_back_on_try_again"
    );
    run_case(test, &[PhaseEnd::Exit, PhaseEnd::Exit], |phase| {
        if phase == 0 {
            let mut app = TestApp::launch();
            fail_the_confirmed_upload_of_a_paused_enable(&mut app);
            return;
        }

        launch_keychain().inner.fail_reads_for(CSPP_PENDING_ENABLE_JOURNAL_KEY);
        let mut app = TestApp::launch();
        app.current().then_can(&[N::RetryLoad]).then_cannot(&[N::Enable, N::TurnOff]);
        assert_eq!(app.manager().published_screen().body, CloudBackupScreenBody::LoadFailed);
        assert_eq!(app.saved_passkey_confirmation(), None, "{}", app.describe());
        assert_eq!(
            app.admission_of(UserIntent::RestoreFromCloudBackup),
            Admission::Rejected(Rejection::LoadFailed),
            "{}",
            app.describe()
        );

        // a Try Again that still can't read changes nothing
        app.dispatch(CloudBackupManagerAction::RetryLoad)
            .then_can(&[N::RetryLoad])
            .then_cannot(&[N::Enable, N::TurnOff]);

        app.keychain.inner.clear_read_failures();
        app.dispatch(CloudBackupManagerAction::RetryLoad).then_cannot(&[
            N::RetryLoad,
            N::Enable,
            N::TurnOff,
        ]);
        assert_paused_enable_holds_the_device(&app);

        app.passkey.inner.set_authenticate_result(Ok(vec![7; 32]));
        confirm_finishes_the_paused_enable(&mut app);
    });
}

/// A launch that can't read the lifecycle record leaves a paused reinitialize's journal for the
/// record to name its owner. The Try Again that loads the record gives the pause back as the
/// reinitialize's, with its Confirm and Discard, and Confirm finishes it with no relaunch
#[test]
fn a_reinitialize_paused_behind_a_failed_load_stays_the_reinitialize_on_try_again() {
    let test = concat!(
        module_path!(),
        "::a_reinitialize_paused_behind_a_failed_load_stays_the_reinitialize_on_try_again"
    );
    run_case(test, &[PhaseEnd::Exit, PhaseEnd::Exit], |phase| {
        if phase == 0 {
            let mut app = TestApp::launch();
            let (old_namespace, _) = pause_a_configured_reinitialize(&mut app);
            seed_legacy_stuck_disabling(&old_namespace, app.clock.now_secs());
            return;
        }

        // read past the journal's accessors, which would fill in the evidence the launch left
        let stored_journal = || {
            launch_keychain()
                .inner
                .get(CSPP_PENDING_ENABLE_JOURNAL_KEY.into())
                .expect("the keychain reads")
                .expect("the reinitialize's journal")
        };

        let journal = stored_journal();
        let mut app =
            TestApp::launch_with_bootstrap(PasskeyMode::Succeeds, bootstrap_with_failed_heal);
        app.current().then_can(&[N::RetryLoad]).then_cannot(&[N::Enable, N::TurnOff]);
        assert_eq!(app.manager().published_screen().body, CloudBackupScreenBody::LoadFailed);
        assert_eq!(app.saved_passkey_confirmation(), None, "{}", app.describe());
        assert_eq!(stored_journal(), journal);

        app.dispatch(CloudBackupManagerAction::RetryLoad).then_cannot(&[
            N::RetryLoad,
            N::Enable,
            N::TurnOff,
        ]);
        let paused = app.saved_passkey_confirmation().unwrap_or_else(|| {
            panic!("the reinitialize's pause did not come back\n{}", app.describe())
        });
        assert_eq!(paused.lifecycle, CloudBackupSavedPasskeyLifecycle::Reinitialize);
        assert_eq!(paused.controls, vec![Control::Confirm, Control::Discard]);
        assert_eq!(
            app.admission_of(UserIntent::ConfirmSavedPasskey),
            Admission::Accepted,
            "{}",
            app.describe()
        );
        assert_eq!(
            app.admission_of(UserIntent::DiscardPendingEnable),
            Admission::Accepted,
            "{}",
            app.describe()
        );

        app.passkey.inner.set_authenticate_result(Ok(vec![7; 32]));
        app.tap_saved_passkey(Control::Confirm)
            .then_can(&[N::TurnOff, N::Verify])
            .then_cannot(&[N::Enable]);
        assert_eq!(app.saved_passkey_confirmation(), None, "{}", app.describe());
        let journal: PendingEnableJournal = serde_json::from_str(&journal).expect("decode");
        assert_eq!(keychain_namespace().as_deref(), Some(journal.namespace_id()));
    });
}

/// Model review sequence: a reinitialize whose confirmed upload failed, then a Google Drive
/// account switch
///
/// The failed upload goes back to the reinitialize's pause, which holds the configured backup
/// its Discard would keep, so the switch is refused before it asks the platform for an account
/// and records nothing. The reinitialize's own Confirm then finishes it
#[test]
fn a_switch_waits_for_a_reinitialize_whose_upload_failed() {
    let test = concat!(module_path!(), "::a_switch_waits_for_a_reinitialize_whose_upload_failed");
    run_case(test, &[PhaseEnd::Exit], |_| {
        let mut app = TestApp::launch();
        let (old_namespace, old_credential) = pause_a_configured_reinitialize(&mut app);
        let old_key = active_master_key();

        app.passkey.inner.set_authenticate_result(Ok(vec![7; 32]));
        let offline = CloudStorageError::NotAvailable("offline".into());
        app.cloud.script_next(CloudCall::UploadMasterKey, CloudFault::Fail(offline));
        app.tap_saved_passkey(Control::Confirm).then_cannot(&[N::TurnOff, N::Enable]);
        assert!(!app.runs_operation(), "{}", app.describe());
        let paused = app
            .saved_passkey_confirmation()
            .unwrap_or_else(|| panic!("the reinitialize did not pause again\n{}", app.describe()));
        assert_eq!(paused.lifecycle, CloudBackupSavedPasskeyLifecycle::Reinitialize);
        let journal = pending_enable_journal().expect("the reinitialize's journal");

        app.dispatch(CloudBackupManagerAction::SwitchDriveAccount)
            .then_cannot(&[N::TurnOff, N::Enable]);
        let outcome = app.last_outcome().expect("the switch's admission");
        assert!(matches!(outcome.admission, Admission::Rejected(_)), "{outcome:?}");
        assert_eq!(app.manager().drained_drive_account_switch_steps(), vec![]);
        assert_eq!(persisted().drive_account_switch(), None, "{}", app.describe());
        assert_eq!(pending_enable_journal().as_ref(), Some(&journal));
        assert_eq!(keychain_namespace().as_deref(), Some(old_namespace.as_str()));
        assert_eq!(
            CloudBackupKeychain::global().load_credential_id().expect("read credential"),
            old_credential
        );
        assert_eq!(active_master_key().as_bytes(), old_key.as_bytes());

        app.tap_saved_passkey(Control::Confirm)
            .then_can(&[N::TurnOff, N::Verify])
            .then_cannot(&[N::Enable]);
        let new_namespace = keychain_namespace().expect("a configured namespace");
        assert_ne!(new_namespace, old_namespace, "{}", app.describe());
        assert_eq!(new_namespace, journal.namespace_id());
        assert_eq!(app.passkey.inner.create_count(), 1, "a second passkey was registered");
        assert_eq!(pending_enable_journal(), None, "{}", app.describe());
    });
}

fn active_master_key() -> MasterKey {
    Cspp::new(Keychain::global().clone())
        .load_master_key_from_store()
        .expect("read the active master key")
        .expect("an active master key")
}
