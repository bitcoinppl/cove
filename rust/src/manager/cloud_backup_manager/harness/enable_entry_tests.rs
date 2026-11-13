//! Every Enable entry point is decided by admission and takes the slot in the turn that decides it
//!
//! Each test body runs in child processes; see [`super::run_case`]
#![cfg(test)]

use cove_cspp::master_key::MasterKey;
use cove_cspp::master_key_crypto::encrypt_master_key;

use super::{
    CloudCall, CloudFault, Hold, NextAction as N, PasskeyMode, PhaseEnd, TestApp, run_case,
};
use crate::manager::cloud_backup_manager::admission::{
    Admission, EnableIntent, Rejection, UserIntent,
};
use crate::manager::cloud_backup_manager::screen::{
    CloudBackupScreenAction, CloudBackupScreenBody,
};
use crate::manager::cloud_backup_manager::{
    CloudBackupEnableContext, CloudBackupEnableFlow, CloudBackupEnablePromptChoice,
    CloudBackupExclusiveOperation as Op, CloudBackupManagerAction, CloudBackupPasskeyChoiceIntent,
    CloudBackupRootPrompt,
};
use crate::manager::onboarding_manager::{CloudRestoreDiscovery, RustOnboardingManager};

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

        app.event("Use Existing, and a Restore queued right behind it", |app| {
            app.manager().dispatch(CloudBackupManagerAction::AcceptEnablePrompt(
                CloudBackupEnablePromptChoice::UseExisting,
            ));
            app.manager().dispatch(CloudBackupManagerAction::RestoreFromCloudBackup);
        })
        .then_cannot(&[N::TurnOff]);

        let outcome = app.last_outcome().expect("the Restore was decided");
        assert_eq!(outcome.intent, UserIntent::RestoreFromCloudBackup);
        assert_eq!(outcome.admission, Admission::Rejected(Rejection::OperationRunning(Op::Enable)));
    });
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
