//! Where each control is drawn: the controls a body and each of its sections name
//!
//! The screen's action table is derived from these, one entry for each action, so no action is
//! offered without a surface that draws it. The platforms draw each section's own controls, and
//! their contract tests find each available action's control on the surface named here

use super::types::{
    CloudBackupConfiguredScreen, CloudBackupControlSurface, CloudBackupDisableScreen,
    CloudBackupDrawnControl, CloudBackupFinishingDisableStage, CloudBackupHealNotice,
    CloudBackupInventoryControl, CloudBackupMenuControl, CloudBackupOtherBackupsControl,
    CloudBackupProgressSection, CloudBackupRecoveryKind, CloudBackupScreen,
    CloudBackupScreenAction, CloudBackupScreenBody, CloudBackupSection, CloudBackupTurnOffFailure,
    CloudBackupVerificationControl, CloudBackupWalletsControl,
};
use crate::manager::cloud_backup_manager::{
    CloudBackupEnableFlow, CloudBackupPasskeyChoiceIntent, CloudBackupPendingEnableCleanupState,
    CloudBackupRestoreAllState, CloudBackupRootPrompt,
};

#[uniffi::export]
impl CloudBackupScreen {
    /// Every control this screen draws and the surface that draws it, in drawing order
    ///
    /// `actions` has one entry for each action named here and no other; an action drawn on more
    /// than one surface is named once for each
    pub fn drawn_controls(&self) -> Vec<CloudBackupDrawnControl> {
        drawn_controls(&self.body, &self.root_prompt, self.heal_notice.as_ref())
    }
}

#[uniffi::export]
impl CloudBackupDrawnControl {
    /// Whether only Android draws this control; iOS draws nothing for it
    pub fn is_android_only(&self) -> bool {
        self.action.is_android_only()
    }
}

/// The controls `body`, the root prompt and the heal notice draw, in drawing order
pub(crate) fn drawn_controls(
    body: &CloudBackupScreenBody,
    root_prompt: &CloudBackupRootPrompt,
    heal_notice: Option<&CloudBackupHealNotice>,
) -> Vec<CloudBackupDrawnControl> {
    let mut drawn = Drawn::default();
    match body {
        CloudBackupScreenBody::Configured(configured) => drawn.configured(configured),
        body => drawn.body_exits(body),
    }

    drawn.extend(CloudBackupControlSurface::RootPrompt, prompt_actions(root_prompt));

    if heal_notice.is_some() {
        drawn.push(
            CloudBackupControlSurface::HealNotice,
            CloudBackupScreenAction::DismissHealNotice,
        );

        // the notice repeats Turn Off where the user reads that backup is on again
        if matches!(body, CloudBackupScreenBody::Configured(_)) {
            drawn.push(CloudBackupControlSurface::HealNotice, CloudBackupScreenAction::TurnOff);
        }
    }

    drawn.0
}

/// The choices a root prompt draws, which bind to their entries as every other control does
///
/// Each prompt's Cancel or Not Now is its always-present escape and sends a dismiss intent that
/// admission always accepts, so it has no entry. Open Cloud Backup on the missing-passkey reminder
/// only navigates
fn prompt_actions(prompt: &CloudBackupRootPrompt) -> Vec<CloudBackupScreenAction> {
    use CloudBackupPasskeyChoiceIntent as Choice;
    use CloudBackupScreenAction as A;

    match prompt {
        CloudBackupRootPrompt::None | CloudBackupRootPrompt::MissingPasskeyReminder => Vec::new(),
        CloudBackupRootPrompt::ExistingBackupFound(..)
        | CloudBackupRootPrompt::PasskeyChoice(Choice::Enable(..)) => {
            vec![A::AcceptEnablePromptUseExisting, A::AcceptEnablePromptCreateNew]
        }
        CloudBackupRootPrompt::PasskeyChoice(Choice::EnableExistingPasskeyOnly(..)) => {
            vec![A::AcceptEnablePromptUseExisting]
        }
        // both of its choices repair the passkey; they differ only in whether the passkey
        // request may offer an existing passkey
        CloudBackupRootPrompt::PasskeyChoice(Choice::RepairPasskey) => vec![A::RepairPasskey],
        CloudBackupRootPrompt::Verification => vec![A::Verify],
    }
}

#[derive(Default)]
struct Drawn(Vec<CloudBackupDrawnControl>);

impl Drawn {
    fn push(&mut self, surface: CloudBackupControlSurface, action: CloudBackupScreenAction) {
        self.0.push(CloudBackupDrawnControl { action, surface });
    }

    fn extend(
        &mut self,
        surface: CloudBackupControlSurface,
        actions: impl IntoIterator<Item = CloudBackupScreenAction>,
    ) {
        for action in actions {
            self.push(surface, action);
        }
    }

    /// The exits of each body other than a configured one (PLAN §3.6)
    fn body_exits(&mut self, body: &CloudBackupScreenBody) {
        use CloudBackupScreenAction as A;

        let exits: &[A] = match body {
            CloudBackupScreenBody::Disabled => &[A::Enable, A::RestoreFromCloudBackup],
            CloudBackupScreenBody::Enabling(
                CloudBackupEnableFlow::AwaitingSavedPasskeyConfirmation(_),
            ) => &[A::ConfirmSavedPasskey, A::DiscardPendingEnable],
            CloudBackupScreenBody::Enabling(
                CloudBackupEnableFlow::AwaitingForceNewConfirmation(..)
                | CloudBackupEnableFlow::AwaitingPasskeyChoice(_),
                // the prompt's choices are the root prompt's own controls
            ) => &[A::DiscardPendingEnable],
            // a running Enable has no Cancel; each of its steps is bounded
            CloudBackupScreenBody::Enabling(_) => &[],
            CloudBackupScreenBody::Restoring(_) => &[A::CancelRestore],
            // removing the incomplete setup is the recovery's only control; with support-only
            // cleanup the user contacts support, and nothing draws a Discard here
            CloudBackupScreenBody::PendingEnableRecovery(recovery) => match recovery.cleanup {
                CloudBackupPendingEnableCleanupState::Available => {
                    &[A::ConfirmPendingEnableCleanup]
                }
                CloudBackupPendingEnableCleanupState::SupportOnly
                | CloudBackupPendingEnableCleanupState::Cleaning => &[],
            },
            CloudBackupScreenBody::DriveAccountSwitch(_) => {
                &[A::RetryDriveAccountSwitchRecovery, A::CancelDriveAccountSwitch]
            }
            // Reconnect asks for the consent the provider stopped the last run for
            CloudBackupScreenBody::FinishingDisable(finishing) => match finishing.stage {
                CloudBackupFinishingDisableStage::Failed {
                    reason: CloudBackupTurnOffFailure::AuthorizationRequired,
                } => &[A::ResumeDisable, A::ReconnectDrive, A::StopUsingOnThisDevice],
                CloudBackupFinishingDisableStage::Failed { .. }
                | CloudBackupFinishingDisableStage::Resuming => {
                    &[A::ResumeDisable, A::StopUsingOnThisDevice]
                }
            },
            CloudBackupScreenBody::LoadFailed => &[A::RetryLoad],
            CloudBackupScreenBody::Corrupted => &[A::ResetOnThisDevice],
            CloudBackupScreenBody::Configured(_) => &[],
        };

        self.extend(CloudBackupControlSurface::Body, exits.iter().copied());
    }

    /// Each section's controls in section order, then the menu, then the Turn Off dialogs
    fn configured(&mut self, screen: &CloudBackupConfiguredScreen) {
        for section in &screen.sections {
            self.section(section);
        }

        let menu = screen.menu.iter().map(|control| control.action());
        self.extend(CloudBackupControlSurface::Menu, menu);
        self.turn_off_dialogs(&screen.disable);
    }

    fn section(&mut self, section: &CloudBackupSection) {
        use CloudBackupControlSurface as Surface;
        use CloudBackupRestoreAllState as RestoreAll;
        use CloudBackupScreenAction as A;

        match section {
            CloudBackupSection::PasskeyCard(_) => self.push(Surface::PasskeyCard, A::RepairPasskey),
            CloudBackupSection::Progress(CloudBackupProgressSection::InventoryIncomplete {
                controls,
                ..
            }) => {
                let actions = controls.iter().map(|control| control.action());
                self.extend(Surface::InventoryIncomplete, actions);
            }
            CloudBackupSection::Progress(_) | CloudBackupSection::ExtraCopies(_) => {}
            CloudBackupSection::Verification(verification) => {
                let actions = verification.controls.iter().map(|control| control.action());
                self.extend(Surface::Verification, actions);
            }
            CloudBackupSection::Wallets(wallets) => {
                let actions = wallets.controls.iter().map(|control| control.action());
                self.extend(Surface::Wallets, actions);
            }
            CloudBackupSection::NotOnDevice(not_on_device) => {
                let action = match not_on_device.restore_all {
                    RestoreAll::NotShown => None,
                    RestoreAll::StartDisabled { .. } | RestoreAll::StartAvailable { .. } => {
                        Some(A::StartRestoreAll)
                    }
                    RestoreAll::RetryDisabled { .. } | RestoreAll::RetryAvailable { .. } => {
                        Some(A::RetryRestoreAllRemaining)
                    }
                    RestoreAll::Running { .. } => Some(A::CancelRestoreAll),
                };

                self.extend(Surface::NotOnDevice, action);
            }
            CloudBackupSection::OtherBackups(other) => {
                let actions = other.controls.iter().map(|control| control.action());
                self.extend(Surface::OtherBackups, actions);
            }
            CloudBackupSection::TurnOff => self.push(Surface::TurnOff, A::TurnOff),
        }
    }

    /// Confirm only on a review, and Cancel while any Turn Off dialog is open
    fn turn_off_dialogs(&mut self, disable: &CloudBackupDisableScreen) {
        use CloudBackupScreenAction as A;

        let surface = CloudBackupControlSurface::TurnOffDialogs;
        match disable {
            CloudBackupDisableScreen::Idle => {}
            CloudBackupDisableScreen::Reviewing(_) => {
                self.extend(surface, [A::ConfirmDisable, A::CancelDisable]);
            }
            CloudBackupDisableScreen::Preparing
            | CloudBackupDisableScreen::Offline
            | CloudBackupDisableScreen::Draining => self.push(surface, A::CancelDisable),
        }
    }
}

impl CloudBackupRecoveryKind {
    /// The action that runs this rebuild
    pub(crate) fn action(self) -> CloudBackupScreenAction {
        match self {
            Self::RecreateManifest => CloudBackupScreenAction::RecreateManifest,
            Self::ReinitializeBackup => CloudBackupScreenAction::ReinitializeBackup,
        }
    }
}

impl CloudBackupInventoryControl {
    pub(crate) fn action(self) -> CloudBackupScreenAction {
        match self {
            Self::CheckAgain => CloudBackupScreenAction::RefreshDetail,
            Self::ReconnectDrive => CloudBackupScreenAction::ReconnectDrive,
        }
    }
}

impl CloudBackupVerificationControl {
    pub(crate) fn action(self) -> CloudBackupScreenAction {
        match self {
            Self::Verify => CloudBackupScreenAction::Verify,
            Self::Recover { kind } => kind.action(),
            Self::DeleteUnreadableBackups => CloudBackupScreenAction::DeleteUnreadableBackups,
            Self::RetryUpload => CloudBackupScreenAction::SyncUnsynced,
            Self::ReconnectDrive => CloudBackupScreenAction::ReconnectDrive,
        }
    }
}

impl CloudBackupWalletsControl {
    pub(crate) fn action(self) -> CloudBackupScreenAction {
        match self {
            Self::SyncNow => CloudBackupScreenAction::SyncUnsynced,
        }
    }
}

impl CloudBackupOtherBackupsControl {
    pub(crate) fn action(self) -> CloudBackupScreenAction {
        match self {
            Self::Recover => CloudBackupScreenAction::RecoverOtherBackups,
            Self::Delete => CloudBackupScreenAction::DeleteOtherBackups,
            Self::CheckAgain => CloudBackupScreenAction::RefreshDetail,
        }
    }
}

impl CloudBackupMenuControl {
    pub(crate) fn action(self) -> CloudBackupScreenAction {
        match self {
            Self::SwitchDriveAccount => CloudBackupScreenAction::SwitchDriveAccount,
        }
    }
}
