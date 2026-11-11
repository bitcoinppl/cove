use act_zero::send;
use cove_device::cloud_storage::CloudSyncHealth;

use crate::database::cloud_backup::PersistedCloudBackupState;

use super::model::{CloudBackupStateReducerEffects, CloudBackupStateReducerEvent};
use super::owner::inventory::{InventoryEffects, InventoryInput};
use super::verify::coordinator::{
    CloudBackupVerificationCoordinator, CloudBackupVerificationEffect, VerificationDetail,
};
use super::{
    CloudBackupDetailOutcome, CloudBackupEnableContext, CloudBackupScreenDelta, CloudBackupStatus,
    CloudBackupVerificationMetadata, CloudBackupVerificationPresentation,
    CloudBackupVerificationSource, PendingUploadVerificationState, RustCloudBackupManager,
};

/// Durable Google Drive account-switch state owned by the platform
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum DriveAccountSwitchPlatformState {
    /// No platform transition exists
    NoTransition,
    /// A selected identity is staged but not yet committed
    Staged(u64),
    /// The selected identity is committed but Rust has not finalized the transition
    Committed(u64),
}

/// Platform action required to reconcile a persisted Google Drive account switch
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum DriveAccountSwitchReconcileAction {
    /// Rust and platform state require no immediate platform mutation
    None,
    /// Atomically commit the staged Google Drive identity
    Commit(u64),
    /// Atomically discard the staged Google Drive identity
    Rollback(u64),
    /// Remove the completed transition marker without changing identity
    Finalize(u64),
}

/// Typed state delta sent from Rust to Swift and Kotlin reconcilers
#[derive(Debug, Clone, uniffi::Enum)]
pub enum CloudBackupReconcileMessage {
    /// What changed on the screen; the platform applies it with
    /// `CloudBackupPublishedScreen::applying`, which skips one its snapshot already includes
    Screen(Box<CloudBackupScreenDelta>),
    EnableCompleted(CloudBackupEnableContext),
    /// Android must atomically commit its staged Google Drive identity
    DriveAccountSwitchCommitRequired(u64),
    /// Android must atomically discard its staged Google Drive identity
    DriveAccountSwitchRollbackRequired(u64),
    /// Android must remove the completed transition marker without changing identity
    DriveAccountSwitchFinalizeRequired(u64),
}

#[uniffi::export(callback_interface)]
pub trait CloudBackupManagerReconciler: Send + Sync + std::fmt::Debug + 'static {
    fn reconcile(&self, message: CloudBackupReconcileMessage);
}

crate::manager::reconcile_channel::impl_reconcile_sink!(
    dyn CloudBackupManagerReconciler,
    CloudBackupReconcileMessage
);

type Message = CloudBackupReconcileMessage;

impl RustCloudBackupManager {
    fn load_persisted_flags() -> (CloudBackupVerificationMetadata, bool) {
        let db_state = Self::load_persisted_state();
        ((&db_state).into(), db_state.should_prompt_verification())
    }

    pub(crate) fn send(&self, message: Message) {
        self.reconciler.send_sync(message);
    }

    pub(crate) fn apply_model_event(&self, event: CloudBackupStateReducerEvent) -> bool {
        let effects = self.state.write().apply_event(event);

        self.send_model_effects(effects);
        true
    }

    pub(crate) fn send_model_effects(&self, effects: CloudBackupStateReducerEffects) {
        // the projection reads more than the lifecycle, so every change is diffed against the
        // published screen, and only a real difference is sent
        self.request_screen_publish();

        if let Some(context) = effects.enable_completed {
            self.send(Message::EnableCompleted(context));
        }
    }

    /// Applies one input to the inventory owner, then publishes the screen and asks for a
    /// cloud-only fetch when the input invalidated the rows
    pub(crate) fn apply_inventory(&self, input: InventoryInput) -> InventoryEffects {
        let effects = self.inventory.write().apply(input);

        if effects.changed {
            self.request_screen_publish();
        }

        // the supervisor owns fetches, so it decides from fresh state whether one can start; a
        // fetch dropped as belonging to an earlier configuration is replaced the same way
        if effects.refetch_needed || effects.dropped {
            send!(self.supervisor.ensure_cloud_only_fetch());
        }

        effects
    }

    pub(crate) fn reconcile_runtime_status(&self, status: CloudBackupStatus) {
        let event = CloudBackupStateReducerEvent::RuntimeStatusReconciled(status);
        let effects = self.state.write().apply_event(event);
        let status_changed = effects.status_changed;
        self.send_model_effects(effects);

        if !status_changed {
            return;
        }

        self.apply_model_event(CloudBackupStateReducerEvent::MissingPasskeyDismissalCleared);
    }

    pub(crate) fn observe_sync_health(&self, sync_health: CloudSyncHealth) {
        self.apply_model_event(CloudBackupStateReducerEvent::SyncHealthObserved(sync_health));
    }

    pub(crate) fn reconcile_verification_presentation(
        &self,
        presentation: CloudBackupVerificationPresentation,
    ) {
        self.apply_model_event(CloudBackupStateReducerEvent::VerificationPresentationReconciled(
            presentation,
        ));
    }

    pub(crate) fn current_verification_source(&self) -> CloudBackupVerificationSource {
        CloudBackupVerificationCoordinator::current_source(
            self.state.read().verification_presentation(),
        )
    }

    pub(crate) fn apply_verification_effect(&self, effect: CloudBackupVerificationEffect) {
        if let Some(VerificationDetail { detail, read_at }) = effect.detail {
            self.apply_detail_outcome(
                read_at.scope(),
                CloudBackupDetailOutcome::VerifiedWithoutListing { detail, read_at },
            );
        }

        if let Some(pending_upload_verification) = effect.pending_upload_verification {
            self.apply_pending_upload_verification_value(pending_upload_verification);
        }

        if let Some(presentation) = effect.presentation {
            self.reconcile_verification_presentation(presentation);
        }

        if let Some(verification) = effect.verification {
            self.apply_verification_state(verification);
        }

        if let Some(recovery) = effect.recovery {
            self.apply_recovery_state(recovery);
        }

        if effect.refresh_sync_health {
            self.refresh_sync_health();
        }
    }

    pub(crate) fn refresh_persisted_flags(&self) {
        let (verification_metadata, should_prompt_verification) = Self::load_persisted_flags();

        self.apply_model_event(CloudBackupStateReducerEvent::VerificationFlagsReconciled {
            metadata: verification_metadata,
            should_prompt: should_prompt_verification,
        });
    }

    /// Projects a just-persisted cloud backup state into runtime status and flags
    pub(crate) fn reconcile_persisted_state(&self, state: &PersistedCloudBackupState) {
        if let Some(status) = Self::runtime_status_for(state) {
            self.reconcile_runtime_status(status);
        }
        self.refresh_persisted_flags();
    }

    fn apply_pending_upload_verification_value(&self, pending: PendingUploadVerificationState) {
        self.apply_model_event(CloudBackupStateReducerEvent::PendingUploadVerificationReconciled(
            pending,
        ));
    }

    pub(crate) fn reconcile_pending_upload_verification(
        &self,
        pending: PendingUploadVerificationState,
    ) {
        self.reconcile_pending_upload_verification_for_source(
            pending,
            self.current_verification_source(),
        );
    }

    pub(crate) fn reconcile_pending_upload_verification_for_source(
        &self,
        pending: PendingUploadVerificationState,
        source: CloudBackupVerificationSource,
    ) {
        let (verification_metadata, should_prompt_verification) = Self::load_persisted_flags();
        let event = CloudBackupStateReducerEvent::PendingUploadVerificationAndFlagsReconciled {
            pending,
            metadata: verification_metadata,
            should_prompt: should_prompt_verification,
        };
        let effects = self.state.write().apply_event(event);
        let decision_pending = effects.verification_decision_pending;
        let presentation_changed = effects.verification_presentation_changed;
        self.send_model_effects(effects);

        if presentation_changed || decision_pending {
            return;
        }

        self.apply_verification_effect(CloudBackupVerificationCoordinator::pending_upload_state(
            pending, source,
        ));
    }

    pub(crate) fn refresh_pending_upload_verification_state(&self) {
        self.reconcile_pending_upload_verification(
            self.current_pending_upload_verification_state(),
        );
    }

    pub(crate) fn current_pending_upload_verification_state(
        &self,
    ) -> PendingUploadVerificationState {
        if self.has_pending_cloud_upload_verification() {
            return PendingUploadVerificationState::Confirming;
        }

        if self.pending_verification_completion().is_some() {
            return PendingUploadVerificationState::Confirming;
        }

        PendingUploadVerificationState::Idle
    }
}

#[uniffi::export]
impl RustCloudBackupManager {
    /// Delivers every message, in the order it was sent, starting with those queued before this
    /// call
    ///
    /// Screen deltas a platform's `state()` snapshot already includes are among them; it skips
    /// those by revision. The other messages are events with no revision, delivered in order
    pub fn listen_for_updates(&self, reconciler: Box<dyn CloudBackupManagerReconciler>) {
        self.reconciler.listen_sink(reconciler);
    }
}
