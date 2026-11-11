//! The Cloud Backup screen: the exported contract, its projection and how it is published
//!
//! Every change the reducer applies is projected into one [`CloudBackupScreen`], diffed against
//! the last one published, and sent as a single [`CloudBackupScreenDelta`]

mod delta;
mod fixtures;
mod identifiers;
mod project;
mod targets;
mod types;

use parking_lot::Mutex;
use tracing::error;

use self::types::CloudBackupActions;
#[cfg(test)]
pub(crate) use self::types::CloudBackupScreenAction;
pub use self::types::{
    CloudBackupDriveAccountSwitchRecovery, CloudBackupHealNotice, CloudBackupNoticeKind,
    CloudBackupScreen, CloudBackupScreenBody, CloudBackupScreenDelta,
};
use crate::database::cloud_backup::PersistedHealNotice;

use super::{CloudBackupReconcileMessage, LifecycleWriter, RustCloudBackupManager};

/// The screen last sent to the platform, which the next projection is diffed against
#[derive(Debug)]
pub(crate) struct PublishedScreen(Mutex<CloudBackupScreen>);

impl Default for PublishedScreen {
    fn default() -> Self {
        Self(Mutex::new(CloudBackupScreen::initial()))
    }
}

impl CloudBackupScreen {
    /// The screen before anything is loaded
    fn initial() -> Self {
        Self {
            body: CloudBackupScreenBody::Disabled,
            settings_row: super::CloudBackupSettingsRowStatus::Disabled,
            heal_notice: None,
            last_outcome: None,
            root_prompt: super::CloudBackupRootPrompt::None,
            actions: CloudBackupActions::default(),
        }
    }
}

impl RustCloudBackupManager {
    /// The screen last published, which is what the platform shows
    pub(crate) fn published_screen(&self) -> CloudBackupScreen {
        self.published_screen.0.lock().clone()
    }

    /// Projects the current state and sends what changed since the last published screen
    ///
    /// The published lock is held across the projection and the send, so two publishes can't
    /// interleave and deliver an older screen after a newer one
    pub(crate) fn publish_screen(&self) {
        let mut published = self.published_screen.0.lock();
        let next = project::project(&self.state.read());
        let Some(delta) = CloudBackupScreenDelta::between(&published, &next) else { return };

        *published = next;
        self.send(CloudBackupReconcileMessage::Screen(Box::new(delta)));
    }

    /// Dismisses the persisted heal notice and publishes the screen without it
    pub(crate) fn dismiss_heal_notice(&self, lifecycle: &LifecycleWriter) {
        if let Err(error) = lifecycle.dismiss_heal_notice() {
            error!("Failed to dismiss the Cloud Backup heal notice: {error}");
            return;
        }

        self.refresh_persisted_flags();
    }
}

impl From<PersistedHealNotice> for CloudBackupHealNotice {
    fn from(notice: PersistedHealNotice) -> Self {
        Self::StuckTurnOffHealed { healed_at: notice.healed_at }
    }
}
