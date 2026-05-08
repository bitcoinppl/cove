use super::{
    CloudBackupEnableContext, CloudBackupPasskeyChoiceIntent, CloudBackupPasskeyHint,
    CloudBackupRootPrompt, CloudBackupState, CloudBackupStatus,
    CloudBackupVerificationPresentation, RecoveryAction, RecoveryState,
};

#[derive(Debug, Clone, Default)]
pub(crate) struct CloudBackupPromptState {
    existing_backup_found: Option<ExistingBackupFoundPrompt>,
    passkey_choice: Option<CloudBackupPasskeyChoiceIntent>,
    missing_passkey_dismissed: bool,
}

#[derive(Debug, Clone)]
struct ExistingBackupFoundPrompt {
    context: CloudBackupEnableContext,
    passkey_hint: Option<CloudBackupPasskeyHint>,
}

impl CloudBackupPromptState {
    pub(crate) fn clear_existing_backup_found(&mut self) {
        self.existing_backup_found = None;
    }

    pub(crate) fn set_existing_backup_found(
        &mut self,
        context: CloudBackupEnableContext,
        passkey_hint: Option<CloudBackupPasskeyHint>,
    ) {
        self.existing_backup_found = Some(ExistingBackupFoundPrompt { context, passkey_hint });
    }

    pub(crate) fn clear_passkey_choice(&mut self) {
        self.passkey_choice = None;
    }

    pub(crate) fn set_passkey_choice(&mut self, intent: CloudBackupPasskeyChoiceIntent) {
        self.passkey_choice = Some(intent);
    }

    pub(crate) fn dismiss_missing_passkey(&mut self) {
        self.missing_passkey_dismissed = true;
    }

    pub(crate) fn clear_missing_passkey_dismissal(&mut self) {
        self.missing_passkey_dismissed = false;
    }

    pub(crate) fn resolve(&self, state: &CloudBackupState) -> CloudBackupRootPrompt {
        if let Some(prompt) = &self.existing_backup_found {
            return CloudBackupRootPrompt::ExistingBackupFound(
                prompt.context,
                prompt.passkey_hint.clone(),
            );
        }

        if let Some(intent) = &self.passkey_choice {
            return CloudBackupRootPrompt::PasskeyChoice(intent.clone());
        }

        // show a reminder while cloud backup needs a passkey, unless repair is already underway
        if matches!(state.status, CloudBackupStatus::PasskeyMissing)
            && !self.missing_passkey_dismissed
            && !matches!(state.recovery, RecoveryState::Recovering(RecoveryAction::RepairPasskey))
        {
            return CloudBackupRootPrompt::MissingPasskeyReminder;
        }

        // the verification sheet is an unanswered decision, not a status surface
        match state.verification_presentation {
            CloudBackupVerificationPresentation::NeedsDecision { .. } => {
                CloudBackupRootPrompt::Verification
            }
            CloudBackupVerificationPresentation::Hidden { .. }
            | CloudBackupVerificationPresentation::ManualVerifying { .. }
            | CloudBackupVerificationPresentation::BackgroundConfirming(_)
            | CloudBackupVerificationPresentation::BackgroundBlockedOnAuthorization(_)
            | CloudBackupVerificationPresentation::Completed { .. }
            | CloudBackupVerificationPresentation::Failed { .. } => CloudBackupRootPrompt::None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        CloudBackupEnableContext, CloudBackupPasskeyChoiceIntent, CloudBackupPromptState,
        CloudBackupRootPrompt, CloudBackupState, CloudBackupStatus,
        CloudBackupVerificationPresentation, RecoveryAction, RecoveryState,
    };
    use crate::manager::cloud_backup_manager::{
        CloudBackupVerificationReason, CloudBackupVerificationSource, SavedPasskeyConfirmationMode,
    };

    fn onboarding_context() -> CloudBackupEnableContext {
        CloudBackupEnableContext {
            saved_passkey_confirmation: SavedPasskeyConfirmationMode::Automatic,
            verification_source: CloudBackupVerificationSource::Onboarding,
        }
    }

    #[test]
    fn existing_backup_prompt_has_highest_priority() {
        let mut prompt_state = CloudBackupPromptState::default();
        let state = CloudBackupState {
            status: CloudBackupStatus::PasskeyMissing,
            should_prompt_verification: true,
            ..CloudBackupState::default()
        };

        let context = onboarding_context();
        prompt_state.set_existing_backup_found(context, None);

        assert_eq!(
            prompt_state.resolve(&state),
            CloudBackupRootPrompt::ExistingBackupFound(context, None),
        );
    }

    #[test]
    fn passkey_choice_beats_missing_passkey_and_verification() {
        let mut prompt_state = CloudBackupPromptState::default();
        let state = CloudBackupState {
            status: CloudBackupStatus::PasskeyMissing,
            should_prompt_verification: true,
            ..CloudBackupState::default()
        };

        prompt_state.set_passkey_choice(CloudBackupPasskeyChoiceIntent::RepairPasskey);

        assert_eq!(
            prompt_state.resolve(&state),
            CloudBackupRootPrompt::PasskeyChoice(CloudBackupPasskeyChoiceIntent::RepairPasskey,),
        );
    }

    #[test]
    fn enable_passkey_choice_carries_context() {
        let mut prompt_state = CloudBackupPromptState::default();
        let context = onboarding_context();

        prompt_state.set_passkey_choice(CloudBackupPasskeyChoiceIntent::Enable(context, None));

        assert_eq!(
            prompt_state.resolve(&CloudBackupState::default()),
            CloudBackupRootPrompt::PasskeyChoice(CloudBackupPasskeyChoiceIntent::Enable(
                context, None,
            )),
        );
    }

    #[test]
    fn dismissed_missing_passkey_stays_hidden_until_reset() {
        let mut prompt_state = CloudBackupPromptState::default();
        let state = CloudBackupState {
            status: CloudBackupStatus::PasskeyMissing,
            ..CloudBackupState::default()
        };

        prompt_state.dismiss_missing_passkey();
        assert_eq!(prompt_state.resolve(&state), CloudBackupRootPrompt::None);

        prompt_state.clear_missing_passkey_dismissal();
        assert_eq!(prompt_state.resolve(&state), CloudBackupRootPrompt::MissingPasskeyReminder,);
    }

    #[test]
    fn repair_flow_suppresses_missing_passkey_prompt() {
        let prompt_state = CloudBackupPromptState::default();
        let state = CloudBackupState {
            status: CloudBackupStatus::PasskeyMissing,
            recovery: RecoveryState::Recovering(RecoveryAction::RepairPasskey),
            ..CloudBackupState::default()
        };

        assert_eq!(prompt_state.resolve(&state), CloudBackupRootPrompt::None);
    }

    #[test]
    fn background_verification_suppresses_verification_prompt() {
        let prompt_state = CloudBackupPromptState::default();
        let state = CloudBackupState {
            verification_presentation: CloudBackupVerificationPresentation::BackgroundConfirming(
                CloudBackupVerificationSource::Settings,
            ),
            ..CloudBackupState::default()
        };

        assert_eq!(prompt_state.resolve(&state), CloudBackupRootPrompt::None);
    }

    #[test]
    fn failed_verification_keeps_prompt_active() {
        let prompt_state = CloudBackupPromptState::default();
        let state = CloudBackupState {
            verification_presentation: CloudBackupVerificationPresentation::Failed {
                source: CloudBackupVerificationSource::RootPrompt,
                message: "verification failed".into(),
            },
            ..CloudBackupState::default()
        };

        assert_eq!(prompt_state.resolve(&state), CloudBackupRootPrompt::None);
    }

    #[test]
    fn unanswered_verification_decision_shows_prompt() {
        let prompt_state = CloudBackupPromptState::default();
        let state = CloudBackupState {
            verification_presentation: CloudBackupVerificationPresentation::NeedsDecision {
                reason: CloudBackupVerificationReason::BackupChanged,
                source: CloudBackupVerificationSource::Settings,
            },
            ..CloudBackupState::default()
        };

        assert_eq!(prompt_state.resolve(&state), CloudBackupRootPrompt::Verification,);
    }
}
