use super::{
    CloudBackupPasskeyChoiceFlow, CloudBackupPromptIntent, CloudBackupState, CloudBackupStatus,
    CloudBackupVerificationPresentation, RecoveryAction, RecoveryState,
};

#[derive(Debug, Clone, Default)]
pub(crate) struct CloudBackupPromptState {
    existing_backup_found: bool,
    passkey_choice_flow: Option<CloudBackupPasskeyChoiceFlow>,
    missing_passkey_dismissed: bool,
}

impl CloudBackupPromptState {
    pub(crate) fn clear_existing_backup_found(&mut self) {
        self.existing_backup_found = false;
    }

    pub(crate) fn set_existing_backup_found(&mut self) {
        self.existing_backup_found = true;
    }

    pub(crate) fn clear_passkey_choice(&mut self) {
        self.passkey_choice_flow = None;
    }

    pub(crate) fn set_passkey_choice(&mut self, flow: CloudBackupPasskeyChoiceFlow) {
        self.passkey_choice_flow = Some(flow);
    }

    pub(crate) fn dismiss_missing_passkey(&mut self) {
        self.missing_passkey_dismissed = true;
    }

    pub(crate) fn clear_missing_passkey_dismissal(&mut self) {
        self.missing_passkey_dismissed = false;
    }

    pub(crate) fn resolve(&self, state: &CloudBackupState) -> CloudBackupPromptIntent {
        if self.existing_backup_found {
            return CloudBackupPromptIntent::ExistingBackupFound;
        }

        if let Some(flow) = &self.passkey_choice_flow {
            return CloudBackupPromptIntent::PasskeyChoice(flow.clone());
        }

        // show a reminder while cloud backup needs a passkey, unless repair is already underway
        if matches!(state.status, CloudBackupStatus::PasskeyMissing)
            && !self.missing_passkey_dismissed
            && !matches!(state.recovery, RecoveryState::Recovering(RecoveryAction::RepairPasskey))
        {
            return CloudBackupPromptIntent::MissingPasskeyReminder;
        }

        // the verification sheet is an unanswered decision, not a status surface
        match state.verification_presentation {
            CloudBackupVerificationPresentation::NeedsDecision { .. } => {
                CloudBackupPromptIntent::VerificationPrompt
            }
            CloudBackupVerificationPresentation::Hidden
            | CloudBackupVerificationPresentation::ManualVerifying { .. }
            | CloudBackupVerificationPresentation::BackgroundConfirming
            | CloudBackupVerificationPresentation::BackgroundBlockedOnAuthorization
            | CloudBackupVerificationPresentation::Completed { .. }
            | CloudBackupVerificationPresentation::Failed { .. } => CloudBackupPromptIntent::None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        CloudBackupPasskeyChoiceFlow, CloudBackupPromptIntent, CloudBackupPromptState,
        CloudBackupState, CloudBackupStatus, CloudBackupVerificationPresentation, RecoveryAction,
        RecoveryState,
    };
    use crate::manager::cloud_backup_manager::{
        CloudBackupVerificationReason, CloudBackupVerificationSource,
    };

    #[test]
    fn existing_backup_prompt_has_highest_priority() {
        let mut prompt_state = CloudBackupPromptState::default();
        let state = CloudBackupState {
            status: CloudBackupStatus::PasskeyMissing,
            should_prompt_verification: true,
            ..CloudBackupState::default()
        };

        prompt_state.set_existing_backup_found();

        assert_eq!(prompt_state.resolve(&state), CloudBackupPromptIntent::ExistingBackupFound,);
    }

    #[test]
    fn passkey_choice_beats_missing_passkey_and_verification() {
        let mut prompt_state = CloudBackupPromptState::default();
        let state = CloudBackupState {
            status: CloudBackupStatus::PasskeyMissing,
            should_prompt_verification: true,
            ..CloudBackupState::default()
        };

        prompt_state.set_passkey_choice(CloudBackupPasskeyChoiceFlow::RepairPasskey);

        assert_eq!(
            prompt_state.resolve(&state),
            CloudBackupPromptIntent::PasskeyChoice(CloudBackupPasskeyChoiceFlow::RepairPasskey,),
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
        assert_eq!(prompt_state.resolve(&state), CloudBackupPromptIntent::None);

        prompt_state.clear_missing_passkey_dismissal();
        assert_eq!(prompt_state.resolve(&state), CloudBackupPromptIntent::MissingPasskeyReminder,);
    }

    #[test]
    fn repair_flow_suppresses_missing_passkey_prompt() {
        let prompt_state = CloudBackupPromptState::default();
        let state = CloudBackupState {
            status: CloudBackupStatus::PasskeyMissing,
            recovery: RecoveryState::Recovering(RecoveryAction::RepairPasskey),
            ..CloudBackupState::default()
        };

        assert_eq!(prompt_state.resolve(&state), CloudBackupPromptIntent::None);
    }

    #[test]
    fn background_verification_suppresses_verification_prompt() {
        let prompt_state = CloudBackupPromptState::default();
        let state = CloudBackupState {
            verification_presentation: CloudBackupVerificationPresentation::BackgroundConfirming,
            ..CloudBackupState::default()
        };

        assert_eq!(prompt_state.resolve(&state), CloudBackupPromptIntent::None);
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

        assert_eq!(prompt_state.resolve(&state), CloudBackupPromptIntent::None);
    }

    #[test]
    fn unanswered_verification_decision_shows_prompt() {
        let prompt_state = CloudBackupPromptState::default();
        let state = CloudBackupState {
            verification_presentation: CloudBackupVerificationPresentation::NeedsDecision {
                reason: CloudBackupVerificationReason::BackupChanged,
            },
            ..CloudBackupState::default()
        };

        assert_eq!(prompt_state.resolve(&state), CloudBackupPromptIntent::VerificationPrompt,);
    }
}
