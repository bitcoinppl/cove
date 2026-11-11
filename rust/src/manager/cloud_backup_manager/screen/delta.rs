//! Screen deltas: the diff Rust sends and the one function every platform applies it with

use super::types::{
    CloudBackupActionAvailability, CloudBackupActions, CloudBackupHealNoticeUpdate,
    CloudBackupLastOutcomeUpdate, CloudBackupPublishedScreen, CloudBackupScreen,
    CloudBackupScreenAction, CloudBackupScreenDelta,
};

impl CloudBackupScreenDelta {
    /// The delta that publishes `next` over `previous`, at the revision after it, or `None`
    /// when nothing changed
    pub(crate) fn between(
        previous: &CloudBackupPublishedScreen,
        next: &CloudBackupScreen,
    ) -> Option<Self> {
        let CloudBackupPublishedScreen { revision, screen: previous } = previous;
        let delta = Self {
            revision: revision + 1,
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

        delta.changes_anything().then_some(delta)
    }

    fn changes_anything(&self) -> bool {
        let Self {
            revision: _,
            body,
            settings_row,
            heal_notice,
            last_outcome,
            root_prompt,
            actions,
        } = self;

        body.is_some()
            || settings_row.is_some()
            || heal_notice.is_some()
            || last_outcome.is_some()
            || root_prompt.is_some()
            || actions.is_some()
    }

    /// `screen` with this delta's parts replaced
    fn apply_to(self, screen: &CloudBackupScreen) -> CloudBackupScreen {
        let mut screen = screen.clone();
        let Self {
            revision: _,
            body,
            settings_row,
            heal_notice,
            last_outcome,
            root_prompt,
            actions,
        } = self;

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
impl CloudBackupPublishedScreen {
    /// This screen advanced by `delta`, or this screen as it is when it already includes `delta`
    ///
    /// The only way a platform updates its screen, so both apps and the Rust test reconciler
    /// apply a delta the same way. The platform's first screen is a snapshot, and deltas
    /// published before it was read can still be queued behind it; replaying one would show
    /// an older or mixed screen, so a delta at or before this revision changes nothing. Deltas
    /// arrive in revision order, so the next one after the snapshot is the one this revision
    /// precedes
    pub fn applying(&self, delta: CloudBackupScreenDelta) -> CloudBackupPublishedScreen {
        if delta.revision <= self.revision {
            return self.clone();
        }

        Self { revision: delta.revision, screen: delta.apply_to(&self.screen) }
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
