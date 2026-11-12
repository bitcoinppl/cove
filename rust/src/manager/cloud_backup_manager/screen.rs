//! The Cloud Backup screen: the exported contract, its projection and how it is published
//!
//! After every change to the runtime, the reducer or the notices, the supervisor projects them
//! with the inventory facts and its admission snapshot into one [`CloudBackupScreen`], diffs it
//! against the last one published, and sends what changed as a single [`CloudBackupScreenDelta`]
//! at the next revision. The platform starts from the [`CloudBackupPublishedScreen`] `state()`
//! returns and skips every delta at or before its revision

mod contract;
mod controls;
mod delta;
mod fixtures;
mod identifiers;
mod notices;
mod project;
mod targets;
mod types;

use std::sync::atomic::{AtomicBool, Ordering};

use act_zero::send;
use parking_lot::Mutex;
use tracing::error;

#[cfg(test)]
pub(crate) use self::contract::{
    mismatched_delete_consents, repeated_section_kinds, section_kind, undrawn_actions,
};
#[cfg(test)]
pub(crate) use self::fixtures::cloud_backup_screen_fixtures;
pub(crate) use self::notices::{LastOutcome, Notices};
#[cfg(test)]
pub(crate) use self::types::CloudBackupActionAvailability;
use self::types::CloudBackupActions;
pub(crate) use self::types::{
    CloudBackupDeleteConfirmation, CloudBackupDisableConfirmation, CloudBackupDisableImpact,
    CloudBackupDisableNotStartedReason, CloudBackupDisableReview, CloudBackupDisableScreen,
    CloudBackupFinishingDisableStage, CloudBackupInventoryAssessment, CloudBackupKeyTrust,
    CloudBackupListedRecord, CloudBackupNamedCount, CloudBackupOutcome, CloudBackupOwnRecord,
    CloudBackupOwnRow, CloudBackupRejection, CloudBackupScreenAction, CloudBackupTurnOffFailure,
    CloudBackupUnreadableKind,
};
pub use self::types::{
    CloudBackupDriveAccountSwitchRecovery, CloudBackupHealNotice, CloudBackupNoticeKind,
    CloudBackupPublishedScreen, CloudBackupScreen, CloudBackupScreenBody, CloudBackupScreenDelta,
};
#[cfg(test)]
pub(crate) use self::types::{
    CloudBackupDriveAccountSwitchStage, CloudBackupInventoryControl, CloudBackupListedRow,
    CloudBackupOutcomeTarget, CloudBackupProgressSection, CloudBackupRowActionKind,
    CloudBackupSavedPasskeyControl, CloudBackupSavedPasskeyLifecycle,
    CloudBackupSavedPasskeyScreen, CloudBackupSection, CloudBackupVerificationControl,
};
use crate::database::cloud_backup::PersistedHealNotice;

use super::actors::supervisor::Runtime;
use super::actors::write::ConfigGeneration;
use super::admission::{AdmissionBody, AdmissionSnapshot};
use super::inventory::InventoryFacts;
use super::{CloudBackupReconcileMessage, LifecycleWriter, RustCloudBackupManager};

/// The screen last sent to the platform, which the next projection is diffed against
#[derive(Debug)]
pub(crate) struct PublishedScreen {
    /// Written with its delta's send under this lock, so a snapshot read from it never runs
    /// ahead of the deltas queued for the platform
    screen: Mutex<Published>,
    /// A publish is queued on the supervisor and has not started projecting yet
    ///
    /// A burst of changes, such as a reset's dozen reducer events or a restore's progress, queues
    /// one publish instead of one per change: the queued publish projects after all of them
    requested: AtomicBool,
}

/// What the supervisor reads for one projection, besides its runtime
#[derive(Debug)]
pub(crate) struct ScreenInputs {
    /// The configuration in force, which every delete consent the screen offers names
    pub(crate) configuration: ConfigGeneration,
    /// The snapshot dispatch would decide on now, so the screen offers what dispatch accepts
    pub(crate) admission: AdmissionSnapshot,
    pub(crate) inventory: InventoryFacts,
    pub(crate) notices: Notices,
}

/// What the platform was last sent: the screen, and whether the owner rested off
#[derive(Debug)]
struct Published {
    screen: CloudBackupPublishedScreen,
    rest: Rest,
}

/// Whether the owner rests with Cloud Backup off on this device
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Rest {
    /// A backup is configured, being set up or changed, or a local reset may still resume one
    Holding,
    /// Nothing on this device belongs to a backup any more
    SettledOff,
}

impl Rest {
    /// The rest the owner is at for one projection
    ///
    /// A running local reset projects as Disabled, but its own admission body tells it apart:
    /// a wipe that fails before its key clear resumes the backup it found, so the reset is no
    /// rest. A Disabled body whose Enable waits on the user projects as that Enable, which still
    /// uses the account it started with
    fn of(admission: &AdmissionSnapshot, screen: &CloudBackupScreen) -> Self {
        let off = admission.body == AdmissionBody::Disabled
            && matches!(screen.body, CloudBackupScreenBody::Disabled);

        if off { Self::SettledOff } else { Self::Holding }
    }
}

impl Default for PublishedScreen {
    fn default() -> Self {
        let screen =
            CloudBackupPublishedScreen { revision: 0, screen: CloudBackupScreen::initial() };

        // nothing was sent yet, so a launch that finds Cloud Backup off reports it once; that
        // releases what a process killed between a Turn Off's commit and its report still holds
        let published = Published { screen, rest: Rest::Holding };
        Self { screen: Mutex::new(published), requested: AtomicBool::new(false) }
    }
}

impl CloudBackupScreen {
    /// The screen before anything is loaded, which the platform starts from
    pub(crate) fn initial() -> Self {
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
    /// The screen last published with its revision, the snapshot a platform starts from
    pub(crate) fn published_snapshot(&self) -> CloudBackupPublishedScreen {
        self.published_screen.screen.lock().screen.clone()
    }

    /// Asks the supervisor to publish the screen
    ///
    /// The projection reads the supervisor's runtime, so publishing runs there; the request
    /// queues behind the handler that made the change, so the projection sees it. While one
    /// request is queued, later changes join it: it has not projected yet, so it will see them
    pub(crate) fn request_screen_publish(&self) {
        if self.published_screen.requested.swap(true, Ordering::AcqRel) {
            return;
        }

        send!(self.supervisor.publish_screen());
    }

    /// Projects the current state and sends what changed since the last published screen
    ///
    /// Only the supervisor calls this, with its runtime and a reader of its other inputs. The
    /// inputs are read after the queued request is cleared, so a change made while they are read
    /// queues another publish instead of being lost. Settling off is sent once, after the delta
    /// that shows it
    pub(crate) fn publish_screen(&self, runtime: &Runtime, inputs: impl FnOnce() -> ScreenInputs) {
        self.published_screen.requested.store(false, Ordering::Release);

        let inputs = inputs();
        let next = self.project_screen(runtime, &inputs);
        let rest = Rest::of(&inputs.admission, &next);
        let mut published = self.published_screen.screen.lock();

        if let Some(delta) = CloudBackupScreenDelta::between(&published.screen, &next) {
            published.screen =
                CloudBackupPublishedScreen { revision: delta.revision, screen: next };
            self.send(CloudBackupReconcileMessage::Screen(Box::new(delta)));
        }

        let settled_off = rest == Rest::SettledOff && published.rest != Rest::SettledOff;
        published.rest = rest;
        if settled_off {
            self.send(CloudBackupReconcileMessage::SettledOff);
        }
    }

    /// The screen for `runtime` and `inputs` with the reducer as it stands now
    pub(crate) fn project_screen(
        &self,
        runtime: &Runtime,
        inputs: &ScreenInputs,
    ) -> CloudBackupScreen {
        let ScreenInputs { configuration, admission, inventory, notices } = inputs;
        project::project(runtime, *configuration, admission, &self.state.read(), inventory, notices)
    }

    /// Dismisses the persisted heal notice; the commit around it publishes the screen without it
    pub(crate) fn dismiss_heal_notice(&self, lifecycle: &LifecycleWriter) {
        if let Err(error) = lifecycle.dismiss_heal_notice() {
            error!("Failed to dismiss the Cloud Backup heal notice: {error}");
        }
    }
}

impl From<PersistedHealNotice> for CloudBackupHealNotice {
    fn from(notice: PersistedHealNotice) -> Self {
        Self::StuckTurnOffHealed { healed_at: notice.healed_at }
    }
}

#[cfg(test)]
mod test_support {
    use std::sync::atomic::Ordering;

    use super::CloudBackupScreen;
    use crate::manager::cloud_backup_manager::RustCloudBackupManager;

    impl RustCloudBackupManager {
        /// The screen last published, which is what the platform shows
        pub(crate) fn published_screen(&self) -> CloudBackupScreen {
            self.published_screen.screen.lock().screen.screen.clone()
        }

        /// Whether a publish is queued on the supervisor and has not started projecting yet
        pub(crate) fn screen_publish_requested(&self) -> bool {
            self.published_screen.requested.load(Ordering::Acquire)
        }
    }
}
