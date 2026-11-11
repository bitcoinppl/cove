//! The Cloud Backup screen: the exported contract, its projection and how it is published
//!
//! After every change the reducer applies, the supervisor projects its runtime, the reducer, the
//! inventory facts and the notices into one [`CloudBackupScreen`], diffs it against the last one
//! published, and sends what changed as a single [`CloudBackupScreenDelta`]

mod delta;
mod fixtures;
mod identifiers;
mod notices;
mod project;
mod targets;
mod types;

use act_zero::send;
use parking_lot::Mutex;
use tracing::error;

use self::notices::Notices;
use self::types::CloudBackupActions;
pub(crate) use self::types::{
    CloudBackupDisableScreen, CloudBackupFinishingDisableStage, CloudBackupListedRecord,
    CloudBackupOwnRecord, CloudBackupOwnRow, CloudBackupProgressSection, CloudBackupTurnOffFailure,
};
pub use self::types::{
    CloudBackupDriveAccountSwitchRecovery, CloudBackupHealNotice, CloudBackupNoticeKind,
    CloudBackupScreen, CloudBackupScreenBody, CloudBackupScreenDelta,
};
#[cfg(test)]
pub(crate) use self::types::{CloudBackupScreenAction, CloudBackupSection};
use crate::database::cloud_backup::PersistedHealNotice;

use super::actors::supervisor::Runtime;
use super::inventory::InventoryFacts;
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

    /// Asks the supervisor to publish the screen
    ///
    /// The projection reads the supervisor's runtime, so publishing runs there; the request
    /// queues behind the handler that made the change, so the projection sees it
    pub(crate) fn request_screen_publish(&self) {
        send!(self.supervisor.publish_screen());
    }

    /// Projects the current state and sends what changed since the last published screen
    ///
    /// Only the supervisor calls this, with its runtime. The published lock is held across the
    /// projection and the send, so two publishes can't interleave and deliver an older screen
    /// after a newer one
    pub(crate) fn publish_screen(&self, runtime: &Runtime) {
        let mut published = self.published_screen.0.lock();
        let next = {
            // the reducer's lock is taken first, as every reader holding both does
            let reducer = self.state.read();
            let inventory = InventoryFacts::new(&self.inventory.read(), &reducer);
            let notices = Notices::from_reducer(&reducer);
            project::project(runtime, &reducer, &inventory, &notices)
        };

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
