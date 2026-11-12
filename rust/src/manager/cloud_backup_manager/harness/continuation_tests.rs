//! A paused Enable or reinitialize goes on, or is discarded, only in the lifecycle it belongs to
//!
//! Each test body runs in child processes; see [`super::run_case`]
#![cfg(test)]

use std::time::Duration;

use act_zero::call;
use cove_cspp::master_key::MasterKey;
use cove_cspp::master_key_crypto::encrypt_master_key;
use cove_device::passkey::{DiscoveredPasskeyResult, PasskeyError};

use super::{
    FakeAccount, NextAction as N, PhaseEnd, TestApp, keychain_namespace, load_detail, persisted,
    run_case,
};
use crate::database::cloud_backup::{PersistedCloudBackupState, PersistedCloudBackupStatus};
use crate::manager::cloud_backup_manager::actors::write::Decision;
use crate::manager::cloud_backup_manager::admission::{Admission, Rejection, UserIntent};
use crate::manager::cloud_backup_manager::model::CloudBackupLifecycle;
use crate::manager::cloud_backup_manager::screen::{
    CloudBackupActionAvailability, CloudBackupSavedPasskeyControl as Control,
    CloudBackupSavedPasskeyLifecycle, CloudBackupScreenAction as A, CloudBackupScreenBody,
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

        app.dispatch(CloudBackupManagerAction::DiscardPendingEnableCloudBackup)
            .then_can(&[N::Enable])
            .then_cannot(&[N::TurnOff]);

        let outcome = app.last_outcome().expect("the Discard's admission");
        assert_eq!(outcome.admission, Admission::Accepted);
        assert_eq!(app.manager().model_snapshot().root_prompt, CloudBackupRootPrompt::None);
        assert_eq!(app.manager().current_status(), CloudBackupStatus::Disabled);
        assert!(app.cloud.inner.has_namespace(&namespace));
        assert_eq!(app.passkey.inner.create_count(), 0);
    });
}
