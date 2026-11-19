//! Every Enable entry point is decided by admission and takes the slot in the turn that decides it
//!
//! Each test body runs in child processes; see [`super::run_case`]
#![cfg(test)]

use cove_cspp::master_key::MasterKey;
use cove_cspp::master_key_crypto::encrypt_master_key;
use cove_device::passkey::{DiscoveredPasskeyResult, PasskeyError};

use super::{
    CloudCall, CloudFault, Hold, NextAction as N, PasskeyMode, PhaseEnd, TestApp, run_case,
};
use crate::manager::cloud_backup_manager::admission::{
    Admission, EnableIntent, Rejection, UserIntent,
};
use crate::manager::cloud_backup_manager::screen::{
    CloudBackupActionAvailability, CloudBackupOutcome, CloudBackupOutcomeTarget,
    CloudBackupScreenAction, CloudBackupScreenBody,
};
use crate::manager::cloud_backup_manager::{
    CloudBackupEnableContext, CloudBackupEnableFlow, CloudBackupEnablePromptChoice,
    CloudBackupError, CloudBackupExclusiveOperation as Op, CloudBackupManagerAction,
    CloudBackupPasskeyChoiceIntent, CloudBackupRootPrompt,
};
use crate::manager::onboarding_manager::{
    CloudCheckIssue, CloudRestoreDiscovery, RustOnboardingManager,
};

/// Onboarding's new-backup Enable is decided like a tap: while a restore holds the slot it is
/// rejected as busy, and no passkey is registered for it
#[test]
fn onboarding_enable_while_a_restore_runs_is_rejected_as_busy() {
    let test =
        concat!(module_path!(), "::onboarding_enable_while_a_restore_runs_is_rejected_as_busy");
    run_case(test, &[PhaseEnd::Exit], |_| {
        let mut app = TestApp::launch();
        let hold = Hold::default();
        app.cloud.script_next(CloudCall::ListNamespaces, CloudFault::Hold(hold.clone()));
        app.dispatch(CloudBackupManagerAction::RestoreFromCloudBackup).then_cannot(&[N::Enable]);
        app.block_on("the restore's listing to be held", hold.entered());

        app.event("onboarding starts Cloud Backup with no backup found", |_| {
            RustOnboardingManager::begin_cloud_backup_enable(CloudRestoreDiscovery::NoBackupFound);
        })
        .then_cannot(&[N::Enable]);

        let outcome = app.last_outcome().expect("onboarding's Enable was decided");
        assert!(
            matches!(outcome.intent, UserIntent::Enable(EnableIntent::NoDiscovery, _)),
            "{outcome:?}"
        );
        assert_eq!(
            outcome.admission,
            Admission::Rejected(Rejection::OperationRunning(Op::Restore))
        );
        assert_eq!(app.passkey.inner.create_count(), 0);

        app.event("the restore's listing answers", move |_| hold.release())
            .then_cannot(&[N::TurnOff]);
    });
}

/// Use Existing on the existing-backup prompt starts its Enable in the turn that consumes the
/// prompt, so a Restore queued right behind the answer finds that Enable running instead of
/// taking the slot from it
#[test]
fn a_restore_queued_behind_a_prompt_answer_finds_its_enable_running() {
    let test = concat!(
        module_path!(),
        "::a_restore_queued_behind_a_prompt_answer_finds_its_enable_running"
    );
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
        assert!(
            matches!(
                app.manager().model_snapshot().root_prompt,
                CloudBackupRootPrompt::ExistingBackupFound(..)
            ),
            "{}",
            app.describe()
        );

        // queued together, so the Restore is decided in the turn after the answer's and finds
        // its Enable running only if the answer started that Enable in its own turn
        app.dispatch_queued(&[
            CloudBackupManagerAction::AcceptEnablePrompt(
                CloudBackupEnablePromptChoice::UseExisting,
            ),
            CloudBackupManagerAction::RestoreFromCloudBackup,
        ])
        .then_cannot(&[N::TurnOff]);

        let outcome = app.last_outcome().expect("the Restore was decided");
        assert_eq!(outcome.intent, UserIntent::RestoreFromCloudBackup);
        assert_eq!(outcome.admission, Admission::Rejected(Rejection::OperationRunning(Op::Enable)));
    });
}

/// Use Existing Passkey on the passkey-choice prompt, as onboarding offers it, leads to the
/// files-unavailable prompt for the first time, so the prompt shows its own message without the
/// failed-retry outcome its Try Again draws
#[test]
fn passkey_choice_use_existing_shows_files_unavailable_without_a_retry_outcome() {
    let test = concat!(
        module_path!(),
        "::passkey_choice_use_existing_shows_files_unavailable_without_a_retry_outcome"
    );
    run_case(test, &[PhaseEnd::Exit], |_| {
        let mut app = TestApp::launch();
        let folder = MasterKey::generate().namespace_id();
        app.cloud.inner.set_wallet_files(folder, Vec::new());
        let context = CloudBackupEnableContext::settings_manual();

        app.dispatch(CloudBackupManagerAction::PromptEnablePasskeyChoice(context))
            .then_cannot(&[N::TurnOff]);
        assert!(
            matches!(
                app.manager().published_screen().root_prompt,
                CloudBackupRootPrompt::PasskeyChoice(CloudBackupPasskeyChoiceIntent::Enable(..))
            ),
            "{}",
            app.describe()
        );

        app.dispatch(CloudBackupManagerAction::AcceptEnablePrompt(
            CloudBackupEnablePromptChoice::UseExisting,
        ))
        .then_cannot(&[N::TurnOff]);
        let screen = app.manager().published_screen();
        assert_eq!(
            screen.root_prompt,
            CloudBackupRootPrompt::BackupFilesUnavailable(context),
            "{}",
            app.describe()
        );
        assert_eq!(screen.last_outcome, None, "{}", app.describe());
        assert_eq!(
            app.passkey.inner.discover_count(),
            0,
            "asked for a passkey with nothing to open"
        );

        // Try Again on the prompt it reached is a retry, so its outcome is drawn there
        app.dispatch(CloudBackupManagerAction::AcceptEnablePrompt(
            CloudBackupEnablePromptChoice::UseExisting,
        ))
        .then_cannot(&[N::TurnOff]);
        let outcome = app
            .manager()
            .published_screen()
            .last_outcome
            .expect("Try Again should say the files are still missing");
        assert_eq!(
            outcome.outcome,
            CloudBackupOutcome::Failed {
                message: CloudBackupError::BackupFilesUnavailable.reader_message(),
            }
        );
    });
}

/// Onboarding that found backup folders without their master key files asks whether to wait
/// for them or create a new backup, as Enable does, instead of offering only a passkey that has
/// nothing to open
#[test]
fn onboarding_with_pending_backup_metadata_offers_try_again_and_create_new() {
    let test = concat!(
        module_path!(),
        "::onboarding_with_pending_backup_metadata_offers_try_again_and_create_new"
    );
    run_case(test, &[PhaseEnd::Exit], |_| {
        let mut app = TestApp::launch();
        let folder = MasterKey::generate().namespace_id();
        app.cloud.inner.set_wallet_files(folder, Vec::new());

        app.event("onboarding starts Cloud Backup with backup metadata pending", |_| {
            RustOnboardingManager::begin_cloud_backup_enable(CloudRestoreDiscovery::Inconclusive(
                CloudCheckIssue::BackupMetadataPending,
            ));
        })
        .then_cannot(&[N::TurnOff]);
        let screen = app.manager().published_screen();
        assert!(
            matches!(screen.root_prompt, CloudBackupRootPrompt::BackupFilesUnavailable(_)),
            "{}",
            app.describe()
        );
        assert_eq!(
            screen.actions.availability(CloudBackupScreenAction::AcceptEnablePromptCreateNew),
            Some(CloudBackupActionAvailability::Available),
            "the prompt should offer Create New Backup: {}",
            app.describe()
        );

        // Try Again checks the cloud again and says on the prompt that the files are still missing
        app.dispatch(CloudBackupManagerAction::AcceptEnablePrompt(
            CloudBackupEnablePromptChoice::UseExisting,
        ))
        .then_cannot(&[N::TurnOff]);
        let screen = app.manager().published_screen();
        assert!(
            matches!(screen.root_prompt, CloudBackupRootPrompt::BackupFilesUnavailable(_)),
            "{}",
            app.describe()
        );
        assert_eq!(
            screen.last_outcome.map(|outcome| outcome.outcome),
            Some(CloudBackupOutcome::Failed {
                message: CloudBackupError::BackupFilesUnavailable.reader_message(),
            })
        );
        assert_eq!(
            app.passkey.inner.discover_count(),
            0,
            "asked for a passkey with nothing to open"
        );
    });
}

/// A namespace whose master key isn't there gives no passkey anything to open, so Enable asks
/// whether to wait for its files or create a new backup instead of asking for a passkey. Try
/// Again that still finds nothing says so on the prompt rather than bringing it back unchanged,
/// and Create New starts a new backup without deleting the folder
#[test]
fn enable_without_master_key_files_offers_try_again_and_create_new() {
    let test = concat!(
        module_path!(),
        "::enable_without_master_key_files_offers_try_again_and_create_new"
    );
    run_case(test, &[PhaseEnd::Exit], |_| {
        let mut app = TestApp::launch();
        let folder = MasterKey::generate().namespace_id();
        app.cloud.inner.set_wallet_files(folder.clone(), Vec::new());
        let context = CloudBackupEnableContext::settings_manual();
        let try_again = CloudBackupOutcomeTarget::Action(
            CloudBackupScreenAction::AcceptEnablePromptUseExisting,
        );

        app.dispatch(CloudBackupManagerAction::EnableCloudBackup(context))
            .then_cannot(&[N::TurnOff]);
        let screen = app.manager().published_screen();
        assert_eq!(
            screen.root_prompt,
            CloudBackupRootPrompt::BackupFilesUnavailable(context),
            "{}",
            app.describe()
        );

        // the prompt's own message says what it found, so the Enable that opened it adds nothing
        assert_eq!(screen.last_outcome, None, "{}", app.describe());
        assert_eq!(
            app.passkey.inner.discover_count(),
            0,
            "asked for a passkey with nothing to open"
        );

        app.dispatch(CloudBackupManagerAction::AcceptEnablePrompt(
            CloudBackupEnablePromptChoice::UseExisting,
        ))
        .then_cannot(&[N::TurnOff]);
        let screen = app.manager().published_screen();
        assert_eq!(
            screen.root_prompt,
            CloudBackupRootPrompt::BackupFilesUnavailable(context),
            "{}",
            app.describe()
        );
        let outcome =
            screen.last_outcome.expect("Try Again should say the files are still missing");
        assert_eq!(outcome.target, try_again);
        assert_eq!(
            outcome.outcome,
            CloudBackupOutcome::Failed {
                message: CloudBackupError::BackupFilesUnavailable.reader_message(),
            }
        );

        app.dispatch(CloudBackupManagerAction::AcceptEnablePrompt(
            CloudBackupEnablePromptChoice::CreateNew,
        ))
        .then_cannot(&[N::TurnOff]);
        assert_eq!(
            app.manager().model_snapshot().root_prompt,
            CloudBackupRootPrompt::None,
            "{}",
            app.describe()
        );
        assert!(app.cloud.inner.has_namespace(&folder), "Create New deleted the folder");
        assert_eq!(app.cloud.count(CloudCall::DeleteNamespace), 0, "{}", app.describe());
    });
}

/// A passkey that opens none of the readable backups brings Enable back to the existing-backup
/// prompt; Try Existing Passkey that opens nothing again says so on the prompt instead of bringing
/// it back unchanged
#[test]
fn try_existing_passkey_that_opens_nothing_says_so() {
    let test = concat!(module_path!(), "::try_existing_passkey_that_opens_nothing_says_so");
    run_case(test, &[PhaseEnd::Exit], |_| {
        let mut app = TestApp::launch();

        // the platform offers a passkey each time, whose PRF opens no backup here
        for _ in 0..2 {
            app.passkey.inner.push_discover_result(Ok(DiscoveredPasskeyResult {
                prf_output: vec![1; 32],
                credential_id: vec![1, 2, 3],
            }));
        }

        assert_try_existing_passkey_says(&mut app, &CloudBackupError::PasskeyMismatch);
    });
}

/// Use Existing on Settings' passkey choice whose passkey opens none of the backups says why on
/// the existing-backup prompt it reaches, so that prompt doesn't ask for the same answer blind
#[test]
fn passkey_choice_use_existing_that_opens_nothing_says_so() {
    let test = concat!(module_path!(), "::passkey_choice_use_existing_that_opens_nothing_says_so");
    run_case(test, &[PhaseEnd::Exit], |_| {
        let mut app = TestApp::launch();
        seed_readable_backup(&mut app);
        app.passkey.inner.push_discover_result(Ok(DiscoveredPasskeyResult {
            prf_output: vec![1; 32],
            credential_id: vec![1, 2, 3],
        }));
        let context = CloudBackupEnableContext::settings_manual();

        app.dispatch(CloudBackupManagerAction::PromptEnablePasskeyChoice(context))
            .then_cannot(&[N::TurnOff]);
        app.dispatch(CloudBackupManagerAction::AcceptEnablePrompt(
            CloudBackupEnablePromptChoice::UseExisting,
        ))
        .then_cannot(&[N::TurnOff]);

        let screen = app.manager().published_screen();
        assert!(
            matches!(screen.root_prompt, CloudBackupRootPrompt::ExistingBackupFound(..)),
            "{}",
            app.describe()
        );
        let outcome =
            screen.last_outcome.expect("Use Existing should say the passkey opened nothing");
        assert_eq!(
            outcome.target,
            CloudBackupOutcomeTarget::Action(
                CloudBackupScreenAction::AcceptEnablePromptUseExisting
            )
        );
        assert_eq!(
            outcome.outcome,
            CloudBackupOutcome::Failed {
                message: CloudBackupError::PasskeyMismatch.reader_message(),
            }
        );
    });
}

/// A passkey that opens none of the readable backups says so even while another backup's master
/// key is still uploading, instead of closing the prompt as if the cloud were unreachable
#[test]
fn try_existing_passkey_beside_an_uploading_backup_says_it_opened_nothing() {
    let test = concat!(
        module_path!(),
        "::try_existing_passkey_beside_an_uploading_backup_says_it_opened_nothing"
    );
    run_case(test, &[PhaseEnd::Exit], |_| {
        let mut app = TestApp::launch();
        for _ in 0..2 {
            app.passkey.inner.push_discover_result(Ok(DiscoveredPasskeyResult {
                prf_output: vec![1; 32],
                credential_id: vec![1, 2, 3],
            }));
        }

        // a second backup whose master key upload this device hasn't confirmed yet
        let uploading = MasterKey::generate();
        let namespace = uploading.namespace_id();
        let encrypted = encrypt_master_key(&uploading, &[7; 32], &[9; 32]).expect("encrypt");
        app.cloud.inner.set_wallet_files(namespace.clone(), Vec::new());
        app.cloud.inner.set_master_key_backup(
            namespace.clone(),
            serde_json::to_vec(&encrypted).expect("encode master key"),
        );
        app.cloud.inner.set_master_key_upload_pending(namespace);

        assert_try_existing_passkey_says(&mut app, &CloudBackupError::PasskeyMismatch);
    });
}

/// Try Again while the backup's master key is listed but still uploading keeps the prompt and
/// says the files still aren't available, instead of closing it as if the cloud were unreachable
#[test]
fn try_again_while_the_master_key_still_uploads_keeps_the_prompt() {
    let test =
        concat!(module_path!(), "::try_again_while_the_master_key_still_uploads_keeps_the_prompt");
    run_case(test, &[PhaseEnd::Exit], |_| {
        let mut app = TestApp::launch();
        let existing = MasterKey::generate();
        let namespace = existing.namespace_id();
        app.cloud.inner.set_wallet_files(namespace.clone(), Vec::new());
        let context = CloudBackupEnableContext::settings_manual();

        app.dispatch(CloudBackupManagerAction::EnableCloudBackup(context))
            .then_cannot(&[N::TurnOff]);
        assert_eq!(
            app.manager().published_screen().root_prompt,
            CloudBackupRootPrompt::BackupFilesUnavailable(context),
            "{}",
            app.describe()
        );

        // the master key reaches the cloud listing before its upload is confirmed
        let encrypted = encrypt_master_key(&existing, &[7; 32], &[9; 32]).expect("encrypt");
        app.cloud.inner.set_master_key_backup(
            namespace,
            serde_json::to_vec(&encrypted).expect("encode master key"),
        );
        app.cloud.inner.set_uploaded_master_key_pending_confirmation(true);

        app.dispatch(CloudBackupManagerAction::AcceptEnablePrompt(
            CloudBackupEnablePromptChoice::UseExisting,
        ))
        .then_cannot(&[N::TurnOff]);
        let screen = app.manager().published_screen();
        assert_eq!(
            screen.root_prompt,
            CloudBackupRootPrompt::BackupFilesUnavailable(context),
            "{}",
            app.describe()
        );
        assert_eq!(
            screen.last_outcome.map(|outcome| outcome.outcome),
            Some(CloudBackupOutcome::Failed {
                message: CloudBackupError::BackupFilesUnavailable.reader_message(),
            })
        );
        assert_eq!(
            app.passkey.inner.discover_count(),
            0,
            "asked for a passkey with nothing to open"
        );
    });
}

/// A device without the backup's passkey, such as one whose passkey hasn't synced from another
/// device yet, says Try Existing Passkey found none rather than that the passkey opened nothing
#[test]
fn try_existing_passkey_without_one_on_this_device_says_none_was_found() {
    let test = concat!(
        module_path!(),
        "::try_existing_passkey_without_one_on_this_device_says_none_was_found"
    );
    run_case(test, &[PhaseEnd::Exit], |_| {
        let mut app = TestApp::launch();
        app.passkey.inner.set_discover_result(Err(PasskeyError::NoCredentialFound));
        app.passkey.inner.push_discover_result(Err(PasskeyError::NoCredentialFound));

        assert_try_existing_passkey_says(
            &mut app,
            &CloudBackupError::from(PasskeyError::NoCredentialFound),
        );
    });
}

/// Seeds one readable backup, enables onto its prompt, then answers Try Existing Passkey and
/// checks the prompt comes back with `error` on it
/// Lists one backup whose master key a passkey with PRF output `[7; 32]` opens
fn seed_readable_backup(app: &mut TestApp) {
    let existing = MasterKey::generate();
    let namespace = existing.namespace_id();
    let encrypted = encrypt_master_key(&existing, &[7; 32], &[9; 32]).expect("encrypt");
    app.cloud.inner.set_wallet_files(namespace.clone(), vec!["wallet-1.json".into()]);
    app.cloud.inner.set_master_key_backup(
        namespace,
        serde_json::to_vec(&encrypted).expect("encode master key"),
    );
}

fn assert_try_existing_passkey_says(app: &mut TestApp, error: &CloudBackupError) {
    {
        seed_readable_backup(app);
        let context = CloudBackupEnableContext::settings_manual();

        app.dispatch(CloudBackupManagerAction::EnableCloudBackup(context))
            .then_cannot(&[N::TurnOff]);
        let screen = app.manager().published_screen();
        assert!(
            matches!(screen.root_prompt, CloudBackupRootPrompt::ExistingBackupFound(..)),
            "{}",
            app.describe()
        );
        assert_eq!(screen.last_outcome, None, "{}", app.describe());

        app.dispatch(CloudBackupManagerAction::AcceptEnablePrompt(
            CloudBackupEnablePromptChoice::UseExisting,
        ))
        .then_cannot(&[N::TurnOff]);
        let screen = app.manager().published_screen();
        assert!(
            matches!(screen.root_prompt, CloudBackupRootPrompt::ExistingBackupFound(..)),
            "{}",
            app.describe()
        );
        let outcome =
            screen.last_outcome.expect("Try Existing Passkey should say it opened nothing");
        assert_eq!(
            outcome.target,
            CloudBackupOutcomeTarget::Action(
                CloudBackupScreenAction::AcceptEnablePromptUseExisting
            )
        );
        assert_eq!(outcome.outcome, CloudBackupOutcome::Failed { message: error.reader_message() });
    }
}

/// An Enable whose passkey sheet the user closed waits on the passkey-choice prompt with nothing
/// running or held, and the Discard its screen offers ends that prompt like the existing-backup
/// prompt's, back to off
#[test]
fn discard_ends_an_enable_waiting_on_its_passkey_choice() {
    let test = concat!(module_path!(), "::discard_ends_an_enable_waiting_on_its_passkey_choice");
    run_case(test, &[PhaseEnd::Exit], |_| {
        let mut app = TestApp::launch();
        app.passkey.set_mode(PasskeyMode::NotCompleted);

        app.dispatch(CloudBackupManagerAction::EnableCloudBackup(
            CloudBackupEnableContext::settings_manual(),
        ))
        .then_cannot(&[N::TurnOff]);

        let screen = app.manager().published_screen();
        assert!(
            matches!(
                screen.body,
                CloudBackupScreenBody::Enabling(CloudBackupEnableFlow::AwaitingPasskeyChoice(
                    CloudBackupPasskeyChoiceIntent::Enable(..)
                ))
            ),
            "{}",
            app.describe()
        );
        assert!(
            screen.actions.is_available(CloudBackupScreenAction::DiscardPendingEnable),
            "{}",
            app.describe()
        );

        app.dispatch(CloudBackupManagerAction::DiscardPendingEnableCloudBackup)
            .then_can(&[N::Enable]);

        let outcome = app.last_outcome().expect("the Discard was decided");
        assert_eq!(outcome.intent, UserIntent::DiscardPendingEnable);
        assert_eq!(outcome.admission, Admission::Accepted);
        assert_eq!(
            app.manager().published_screen().body,
            CloudBackupScreenBody::Disabled,
            "{}",
            app.describe()
        );
        assert_eq!(app.manager().model_snapshot().root_prompt, CloudBackupRootPrompt::None);
    });
}
