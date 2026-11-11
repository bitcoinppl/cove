//! Screen deltas: the diff Rust sends and the one function every platform applies it with

use super::types::{
    CloudBackupActionAvailability, CloudBackupActions, CloudBackupHealNoticeUpdate,
    CloudBackupLastOutcomeUpdate, CloudBackupScreen, CloudBackupScreenAction,
    CloudBackupScreenDelta,
};

impl CloudBackupScreenDelta {
    /// The parts of `next` that differ from `previous`, or `None` when nothing changed
    pub(crate) fn between(previous: &CloudBackupScreen, next: &CloudBackupScreen) -> Option<Self> {
        let delta = Self {
            body: (previous.body != next.body).then(|| next.body.clone()),
            settings_row: (previous.settings_row != next.settings_row)
                .then(|| next.settings_row.clone()),
            heal_notice: (previous.heal_notice != next.heal_notice)
                .then_some(CloudBackupHealNoticeUpdate { notice: next.heal_notice }),
            last_outcome: (previous.last_outcome != next.last_outcome)
                .then(|| CloudBackupLastOutcomeUpdate { outcome: next.last_outcome.clone() }),
            root_prompt: (previous.root_prompt != next.root_prompt)
                .then(|| next.root_prompt.clone()),
            actions: (previous.actions != next.actions).then(|| next.actions.clone()),
        };

        (delta != Self::default()).then_some(delta)
    }
}

#[uniffi::export]
impl CloudBackupScreen {
    /// This screen with `delta` applied
    ///
    /// The only way a platform updates its screen, so both apps and the Rust test reconciler
    /// apply a delta the same way
    pub fn applying(&self, delta: CloudBackupScreenDelta) -> CloudBackupScreen {
        let mut screen = self.clone();
        let CloudBackupScreenDelta {
            body,
            settings_row,
            heal_notice,
            last_outcome,
            root_prompt,
            actions,
        } = delta;

        if let Some(body) = body {
            screen.body = body;
        }

        if let Some(settings_row) = settings_row {
            screen.settings_row = settings_row;
        }

        if let Some(update) = heal_notice {
            screen.heal_notice = update.notice;
        }

        if let Some(update) = last_outcome {
            screen.last_outcome = update.outcome;
        }

        if let Some(root_prompt) = root_prompt {
            screen.root_prompt = root_prompt;
        }

        if let Some(actions) = actions {
            screen.actions = actions;
        }

        screen
    }
}

#[uniffi::export]
impl CloudBackupActions {
    /// Whether `action` has a control on this screen, and whether it is enabled; `None` means
    /// the platform draws no control for it
    pub fn availability(
        &self,
        action: CloudBackupScreenAction,
    ) -> Option<CloudBackupActionAvailability> {
        self.entries.iter().find(|entry| entry.action == action).map(|entry| entry.availability)
    }

    /// Whether `action` is drawn and enabled
    pub fn is_available(&self, action: CloudBackupScreenAction) -> bool {
        self.availability(action) == Some(CloudBackupActionAvailability::Available)
    }
}

#[uniffi::export]
impl CloudBackupScreenAction {
    /// Whether only Android draws this action; iOS handles it without drawing anything
    pub fn is_android_only(&self) -> bool {
        matches!(
            self,
            Self::ReconnectDrive
                | Self::SwitchDriveAccount
                | Self::RetryDriveAccountSwitchRecovery
                | Self::CancelDriveAccountSwitch
        )
    }
}
