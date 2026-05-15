use super::{
    CloudBackupDetail, CloudBackupDisableOutcome, CloudBackupEnableContext, CloudBackupEnableState,
    CloudBackupPasskeyChoiceIntent, CloudBackupPasskeyHint, CloudBackupRootPrompt,
    CloudBackupStatus, CloudBackupVerificationMetadata, CloudBackupVerificationPresentation,
    CloudBackupVerificationReason, CloudOnlyOperation, CloudOnlyState, DeepVerificationFailure,
    DeepVerificationReport, OtherBackupsOperation, PendingUploadVerificationState, RecoveryAction,
    RecoveryState, SyncState, VerificationState,
};

use super::verify::coordinator::CloudBackupVerificationCoordinator;
use cove_device::cloud_storage::CloudSyncHealth;

const PENDING_UPLOAD_AUTHORIZATION_BLOCKED_MESSAGE: &str = "cloud authorization required";

#[derive(Debug, Clone, Default)]
pub(crate) struct CloudBackupModel {
    state: CloudBackupModelState,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CloudBackupModelState {
    phase: CloudBackupLifecyclePhase,
    configured: CloudBackupConfiguredModelState,
    sync_health: CloudSyncHealth,
    missing_passkey_dismissed: bool,
    should_prompt_verification: bool,
    verification_metadata: CloudBackupVerificationMetadata,
    verification_presentation: CloudBackupVerificationPresentation,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum CloudBackupLifecyclePhase {
    Disabled,
    Enabling(CloudBackupEnableFlow),
    Restoring(CloudBackupRestoreFlow),
    Configured,
    Failed(CloudBackupFailure),
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CloudBackupConfiguredModelState {
    passkey: CloudBackupPasskeyState,
    verification: CloudBackupVerificationState,
    sync: CloudBackupSyncState,
    destructive_operation: CloudBackupDestructiveOperationState,
    pending_upload_verification: PendingUploadVerificationState,
    detail: CloudBackupDetailState,
    prompt: CloudBackupConfiguredPrompt,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum CloudBackupConfiguredPrompt {
    None,
    PasskeyChoice(CloudBackupPasskeyChoiceIntent),
}

impl Default for CloudBackupModelState {
    fn default() -> Self {
        Self {
            phase: CloudBackupLifecyclePhase::Disabled,
            configured: CloudBackupConfiguredModelState::default(),
            sync_health: CloudSyncHealth::Unknown,
            missing_passkey_dismissed: false,
            should_prompt_verification: false,
            verification_metadata: CloudBackupVerificationMetadata::NotConfigured,
            verification_presentation: CloudBackupVerificationPresentation::Hidden { source: None },
        }
    }
}

impl Default for CloudBackupConfiguredModelState {
    fn default() -> Self {
        Self {
            passkey: CloudBackupPasskeyState::Available,
            verification: CloudBackupVerificationState::NotVerified,
            sync: CloudBackupSyncState::Idle,
            destructive_operation: CloudBackupDestructiveOperationState::Idle,
            pending_upload_verification: PendingUploadVerificationState::Idle,
            detail: CloudBackupDetailState::NotLoaded,
            prompt: CloudBackupConfiguredPrompt::None,
        }
    }
}

impl CloudBackupModelState {
    fn public_state(&self) -> super::CloudBackupState {
        super::CloudBackupState { lifecycle: self.public_lifecycle() }
    }

    fn public_lifecycle(&self) -> CloudBackupLifecycle {
        match &self.phase {
            CloudBackupLifecyclePhase::Disabled => CloudBackupLifecycle::Disabled,
            CloudBackupLifecyclePhase::Enabling(flow) => {
                CloudBackupLifecycle::Enabling(flow.clone())
            }
            CloudBackupLifecyclePhase::Restoring(flow) => {
                CloudBackupLifecycle::Restoring(flow.clone())
            }
            CloudBackupLifecyclePhase::Configured => {
                CloudBackupLifecycle::Configured(self.public_configured_state())
            }
            CloudBackupLifecyclePhase::Failed(failure) => {
                CloudBackupLifecycle::Failed(failure.clone())
            }
        }
    }

    fn public_configured_state(&self) -> CloudBackupConfiguredState {
        CloudBackupConfiguredState {
            passkey: self.configured.passkey.clone(),
            verification: self.public_verification_state(),
            sync: self.configured.sync.clone(),
            destructive_operation: self.configured.destructive_operation.clone(),
            detail: self.configured.detail.clone(),
            root_prompt: self.root_prompt(),
            sync_health: self.sync_health.clone(),
            verification_presentation: self.verification_presentation.clone(),
        }
    }

    fn public_verification_state(&self) -> CloudBackupVerificationState {
        if !matches!(
            self.configured.pending_upload_verification,
            PendingUploadVerificationState::Idle
        ) {
            return CloudBackupVerificationState::AwaitingUploadConfirmation;
        }

        match &self.configured.verification {
            CloudBackupVerificationState::Verified { report, .. } => {
                CloudBackupVerificationState::Verified {
                    report: report.clone(),
                    last_verified_at: Self::last_verified_at(&self.verification_metadata),
                }
            }
            verification => verification.clone(),
        }
    }

    fn last_verified_at(metadata: &CloudBackupVerificationMetadata) -> Option<u64> {
        match metadata {
            CloudBackupVerificationMetadata::Verified(timestamp) => Some(*timestamp),
            CloudBackupVerificationMetadata::NotConfigured
            | CloudBackupVerificationMetadata::ConfiguredNeverVerified
            | CloudBackupVerificationMetadata::NeedsVerification => None,
        }
    }

    fn root_prompt(&self) -> CloudBackupRootPrompt {
        match &self.phase {
            CloudBackupLifecyclePhase::Enabling(
                CloudBackupEnableFlow::AwaitingForceNewConfirmation(context, passkey_hint),
            ) => CloudBackupRootPrompt::ExistingBackupFound(*context, passkey_hint.clone()),
            CloudBackupLifecyclePhase::Enabling(CloudBackupEnableFlow::AwaitingPasskeyChoice(
                intent,
            )) => CloudBackupRootPrompt::PasskeyChoice(intent.clone()),
            CloudBackupLifecyclePhase::Configured => self.configured_root_prompt(),
            CloudBackupLifecyclePhase::Disabled
            | CloudBackupLifecyclePhase::Enabling(_)
            | CloudBackupLifecyclePhase::Restoring(_)
            | CloudBackupLifecyclePhase::Failed(_) => CloudBackupRootPrompt::None,
        }
    }

    fn configured_root_prompt(&self) -> CloudBackupRootPrompt {
        if let CloudBackupConfiguredPrompt::PasskeyChoice(intent) = &self.configured.prompt {
            return CloudBackupRootPrompt::PasskeyChoice(intent.clone());
        }

        if self.should_show_missing_passkey_reminder() {
            return CloudBackupRootPrompt::MissingPasskeyReminder;
        }

        match self.verification_presentation {
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

    fn should_show_missing_passkey_reminder(&self) -> bool {
        if self.missing_passkey_dismissed {
            return false;
        }

        matches!(
            self.configured.passkey,
            CloudBackupPasskeyState::Missing
                | CloudBackupPasskeyState::NeedsRepair {
                    state: CloudBackupPasskeyRepairState::Idle
                        | CloudBackupPasskeyRepairState::Failed(_),
                }
        )
    }

    fn status(&self) -> CloudBackupStatus {
        match &self.phase {
            CloudBackupLifecyclePhase::Disabled => CloudBackupStatus::Disabled,
            CloudBackupLifecyclePhase::Enabling(_) => CloudBackupStatus::Enabling,
            CloudBackupLifecyclePhase::Restoring(_) => CloudBackupStatus::Restoring,
            CloudBackupLifecyclePhase::Configured
                if matches!(
                    self.configured.destructive_operation,
                    CloudBackupDestructiveOperationState::Disabling
                ) =>
            {
                CloudBackupStatus::Disabling
            }
            CloudBackupLifecyclePhase::Configured => match &self.configured.passkey {
                CloudBackupPasskeyState::Available => CloudBackupStatus::Enabled,
                CloudBackupPasskeyState::Missing | CloudBackupPasskeyState::NeedsRepair { .. } => {
                    CloudBackupStatus::PasskeyMissing
                }
                CloudBackupPasskeyState::UnsupportedProvider => {
                    CloudBackupStatus::UnsupportedPasskeyProvider
                }
            },
            CloudBackupLifecyclePhase::Failed(failure) => {
                CloudBackupStatus::Error(failure.message.clone())
            }
        }
    }

    fn progress(&self) -> Option<super::CloudBackupProgress> {
        match &self.phase {
            CloudBackupLifecyclePhase::Enabling(
                CloudBackupEnableFlow::UploadingInitialBackup { progress },
            )
            | CloudBackupLifecyclePhase::Enabling(
                CloudBackupEnableFlow::RetryingUploadWithStagedMaterial { progress },
            ) => *progress,
            _ => None,
        }
    }

    fn detail(&self) -> Option<CloudBackupDetail> {
        self.loaded_detail().map(|state| state.detail.clone())
    }

    fn verification(&self) -> VerificationState {
        match &self.configured.verification {
            CloudBackupVerificationState::NotVerified
            | CloudBackupVerificationState::Required
            | CloudBackupVerificationState::AwaitingUploadConfirmation => VerificationState::Idle,
            CloudBackupVerificationState::Verified { report: Some(report), .. } => {
                VerificationState::Verified(report.clone())
            }
            CloudBackupVerificationState::Verified { report: None, .. } => {
                VerificationState::PasskeyConfirmed
            }
            CloudBackupVerificationState::Running => VerificationState::Verifying,
            CloudBackupVerificationState::Failed(failure) => {
                VerificationState::Failed(failure.clone())
            }
        }
    }

    fn cloud_only(&self) -> CloudOnlyState {
        self.loaded_detail()
            .map(|state| state.cloud_only.clone())
            .unwrap_or(CloudOnlyState::NotFetched)
    }

    fn cloud_only_operation(&self) -> CloudOnlyOperation {
        self.loaded_detail()
            .map(|state| state.cloud_only_operation.clone())
            .unwrap_or(CloudOnlyOperation::Idle)
    }

    fn other_backups_operation(&self) -> OtherBackupsOperation {
        self.loaded_detail()
            .map(|state| state.other_backups_operation.clone())
            .unwrap_or(OtherBackupsOperation::Idle)
    }

    fn loaded_detail(&self) -> Option<&LoadedCloudBackupDetail> {
        match &self.configured.detail {
            CloudBackupDetailState::Loaded { state } => Some(state),
            CloudBackupDetailState::NotLoaded
            | CloudBackupDetailState::Loading
            | CloudBackupDetailState::Failed(_) => None,
        }
    }

    fn enter_background_status(
        &mut self,
        status: CloudBackupStatus,
    ) -> Result<(), CloudBackupModelEventRejection> {
        let current_status = self.status();
        let already_disabling = current_status == CloudBackupStatus::Disabling
            && status == CloudBackupStatus::Disabling;

        if already_disabling {
            return Ok(());
        }

        let background_operation_in_progress = matches!(
            current_status,
            CloudBackupStatus::Disabling
                | CloudBackupStatus::Enabling
                | CloudBackupStatus::Restoring
        );

        if background_operation_in_progress {
            return Err(CloudBackupModelEventRejection::Busy(current_status));
        }

        self.apply_status(status);
        Ok(())
    }

    fn apply_status(&mut self, status: CloudBackupStatus) {
        match status {
            CloudBackupStatus::Disabled => {
                self.phase = CloudBackupLifecyclePhase::Disabled;
            }
            CloudBackupStatus::Disabling => {
                self.configured.destructive_operation =
                    CloudBackupDestructiveOperationState::Disabling;
                self.configured.prompt = CloudBackupConfiguredPrompt::None;
                self.phase = CloudBackupLifecyclePhase::Configured;
            }
            CloudBackupStatus::Enabling => {
                let flow = match &self.phase {
                    CloudBackupLifecyclePhase::Enabling(flow) => flow.clone(),
                    _ => CloudBackupEnableFlow::DiscoveringExistingBackup,
                };
                self.phase = CloudBackupLifecyclePhase::Enabling(flow);
            }
            CloudBackupStatus::Restoring => {
                let flow = match &self.phase {
                    CloudBackupLifecyclePhase::Restoring(flow) => flow.clone(),
                    _ => CloudBackupRestoreFlow::Finding,
                };
                self.phase = CloudBackupLifecyclePhase::Restoring(flow);
            }
            CloudBackupStatus::Enabled => {
                self.configured.passkey = CloudBackupPasskeyState::Available;
                self.configured.prompt = CloudBackupConfiguredPrompt::None;

                let should_reset_destructive_operation = matches!(
                    self.configured.destructive_operation,
                    CloudBackupDestructiveOperationState::Disabling
                );

                if should_reset_destructive_operation {
                    self.configured.destructive_operation =
                        CloudBackupDestructiveOperationState::Idle;
                }

                self.phase = CloudBackupLifecyclePhase::Configured;
            }
            CloudBackupStatus::PasskeyMissing => {
                self.configured.passkey = match &self.configured.passkey {
                    CloudBackupPasskeyState::NeedsRepair { state } => {
                        CloudBackupPasskeyState::NeedsRepair { state: state.clone() }
                    }
                    CloudBackupPasskeyState::Available
                    | CloudBackupPasskeyState::Missing
                    | CloudBackupPasskeyState::UnsupportedProvider => {
                        CloudBackupPasskeyState::NeedsRepair {
                            state: CloudBackupPasskeyRepairState::Idle,
                        }
                    }
                };
                self.phase = CloudBackupLifecyclePhase::Configured;
            }
            CloudBackupStatus::UnsupportedPasskeyProvider => {
                self.configured.passkey = CloudBackupPasskeyState::UnsupportedProvider;
                self.configured.prompt = CloudBackupConfiguredPrompt::None;
                self.phase = CloudBackupLifecyclePhase::Configured;
            }
            CloudBackupStatus::Error(message) => {
                self.phase = CloudBackupLifecyclePhase::Failed(CloudBackupFailure { message });
            }
        }
    }

    fn apply_enable_flow(&mut self, enable_state: CloudBackupEnableState) {
        let progress = self.progress();
        self.phase = CloudBackupLifecyclePhase::Enabling(match enable_state {
            CloudBackupEnableState::Idle => match &self.phase {
                CloudBackupLifecyclePhase::Enabling(
                    flow @ (CloudBackupEnableFlow::AwaitingForceNewConfirmation(_, _)
                    | CloudBackupEnableFlow::AwaitingPasskeyChoice(_)),
                ) => flow.clone(),
                _ => CloudBackupEnableFlow::DiscoveringExistingBackup,
            },
            CloudBackupEnableState::CreatingPasskey => CloudBackupEnableFlow::CreatingPasskey,
            CloudBackupEnableState::AwaitingSavedPasskeyConfirmation(mode) => {
                CloudBackupEnableFlow::AwaitingSavedPasskeyConfirmation(mode)
            }
            CloudBackupEnableState::ConfirmingSavedPasskey => {
                CloudBackupEnableFlow::ConfirmingSavedPasskey
            }
            CloudBackupEnableState::UploadingBackup => {
                CloudBackupEnableFlow::UploadingInitialBackup { progress }
            }
        });
    }

    fn report_enable_progress(&mut self, progress: Option<super::CloudBackupProgress>) {
        match &mut self.phase {
            CloudBackupLifecyclePhase::Enabling(
                CloudBackupEnableFlow::UploadingInitialBackup { progress: current }
                | CloudBackupEnableFlow::RetryingUploadWithStagedMaterial { progress: current },
            ) => *current = progress,
            CloudBackupLifecyclePhase::Enabling(_) if progress.is_some() => {
                self.phase = CloudBackupLifecyclePhase::Enabling(
                    CloudBackupEnableFlow::UploadingInitialBackup { progress },
                );
            }
            _ => {}
        }
    }

    fn report_restore_progress(&mut self, progress: CloudBackupRestoreFlow) {
        if let CloudBackupLifecyclePhase::Restoring(flow) = &mut self.phase {
            *flow = progress;
        }
    }

    fn reconcile_pending_upload_verification(&mut self, pending: PendingUploadVerificationState) {
        self.configured.pending_upload_verification = pending;

        match pending {
            PendingUploadVerificationState::Idle => {
                if matches!(self.configured.sync, CloudBackupSyncState::Blocked(_)) {
                    self.configured.sync = CloudBackupSyncState::Idle;
                }
            }
            PendingUploadVerificationState::Confirming => {
                if matches!(self.configured.sync, CloudBackupSyncState::Blocked(_)) {
                    self.configured.sync = CloudBackupSyncState::Idle;
                }
            }
            PendingUploadVerificationState::BlockedOnAuthorization => {
                self.configured.sync = CloudBackupSyncState::Blocked(
                    PENDING_UPLOAD_AUTHORIZATION_BLOCKED_MESSAGE.into(),
                );
            }
        }
    }

    fn reconcile_verification_flags(
        &mut self,
        metadata: CloudBackupVerificationMetadata,
        should_prompt: bool,
    ) {
        self.verification_metadata = metadata;
        self.should_prompt_verification = should_prompt;

        if matches!(
            self.configured.verification,
            CloudBackupVerificationState::NotVerified | CloudBackupVerificationState::Required
        ) {
            self.configured.verification = self.idle_verification_state();
        }

        let presentation = if let Some(presentation) =
            Self::verification_decision_presentation_for_state(self)
        {
            presentation
        } else if matches!(
            self.verification_presentation,
            CloudBackupVerificationPresentation::NeedsDecision { .. }
        ) {
            CloudBackupVerificationCoordinator::dismiss_decision(
                CloudBackupVerificationCoordinator::current_source(&self.verification_presentation),
            )
            .presentation
            .expect("dismiss decision effect should include presentation")
        } else {
            self.verification_presentation.clone()
        };

        self.verification_presentation = presentation;
    }

    fn idle_verification_state(&self) -> CloudBackupVerificationState {
        if self.should_prompt_verification {
            CloudBackupVerificationState::Required
        } else {
            CloudBackupVerificationState::NotVerified
        }
    }

    fn resolve_verification(&mut self, verification: VerificationState) {
        self.configured.verification = match verification {
            VerificationState::Idle | VerificationState::Cancelled => {
                self.idle_verification_state()
            }
            VerificationState::Verifying => CloudBackupVerificationState::Running,
            VerificationState::Verified(report) => CloudBackupVerificationState::Verified {
                report: Some(report),
                last_verified_at: Self::last_verified_at(&self.verification_metadata),
            },
            VerificationState::PasskeyConfirmed => CloudBackupVerificationState::Verified {
                report: None,
                last_verified_at: Self::last_verified_at(&self.verification_metadata),
            },
            VerificationState::Failed(failure) => CloudBackupVerificationState::Failed(failure),
        };
    }

    fn resolve_sync(&mut self, sync: SyncState) {
        if matches!(
            self.configured.pending_upload_verification,
            PendingUploadVerificationState::BlockedOnAuthorization
        ) {
            return;
        }

        self.configured.sync = match sync {
            SyncState::Idle => CloudBackupSyncState::Idle,
            SyncState::Syncing => CloudBackupSyncState::Syncing,
            SyncState::Failed(message) => CloudBackupSyncState::Failed(message),
        };
    }

    fn resolve_recovery(&mut self, recovery: RecoveryState) {
        match recovery {
            RecoveryState::Idle => {
                self.configured.destructive_operation = CloudBackupDestructiveOperationState::Idle;
                if matches!(self.configured.passkey, CloudBackupPasskeyState::NeedsRepair { .. }) {
                    self.configured.passkey = CloudBackupPasskeyState::NeedsRepair {
                        state: CloudBackupPasskeyRepairState::Idle,
                    };
                }
            }
            RecoveryState::Recovering(RecoveryAction::RecreateManifest) => {
                self.configured.destructive_operation =
                    CloudBackupDestructiveOperationState::RecreatingManifest;
            }
            RecoveryState::Recovering(RecoveryAction::ReinitializeBackup) => {
                self.configured.destructive_operation =
                    CloudBackupDestructiveOperationState::ReinitializingBackup;
            }
            RecoveryState::Recovering(RecoveryAction::RepairPasskey) => {
                self.configured.passkey = CloudBackupPasskeyState::NeedsRepair {
                    state: CloudBackupPasskeyRepairState::Running,
                };
                self.phase = CloudBackupLifecyclePhase::Configured;
            }
            RecoveryState::Failed {
                action: RecoveryAction::RecreateManifest | RecoveryAction::ReinitializeBackup,
                ..
            } => {
                self.configured.destructive_operation = CloudBackupDestructiveOperationState::Idle;
            }
            RecoveryState::Failed { action: RecoveryAction::RepairPasskey, error } => {
                self.configured.passkey = CloudBackupPasskeyState::NeedsRepair {
                    state: CloudBackupPasskeyRepairState::Failed(error),
                };
                self.phase = CloudBackupLifecyclePhase::Configured;
            }
        }
    }

    // reconciles database disable progress with the UI-facing destructive operation state
    fn resolve_disable(&mut self, outcome: CloudBackupDisableOutcome) {
        match outcome {
            CloudBackupDisableOutcome::Started => {
                self.configured.destructive_operation =
                    CloudBackupDestructiveOperationState::Disabling;
                self.configured.prompt = CloudBackupConfiguredPrompt::None;
                self.configured.sync = CloudBackupSyncState::Idle;
                self.configured.pending_upload_verification = PendingUploadVerificationState::Idle;
                self.phase = CloudBackupLifecyclePhase::Configured;
            }
            CloudBackupDisableOutcome::Failed { message, can_keep_enabled } => {
                self.configured.destructive_operation =
                    CloudBackupDestructiveOperationState::DisableFailed {
                        message,
                        can_keep_enabled,
                    };
                self.phase = CloudBackupLifecyclePhase::Configured;
            }
            CloudBackupDisableOutcome::ReturnedToIdle => {
                self.configured.destructive_operation = CloudBackupDestructiveOperationState::Idle;
                self.phase = CloudBackupLifecyclePhase::Configured;
            }
        }
    }

    fn apply_detail_refresh(&mut self, detail: Option<CloudBackupDetail>, reset_cloud_only: bool) {
        let previous_loaded = self.loaded_detail().cloned();
        self.configured.detail = match detail {
            Some(detail) => {
                let cloud_only = if reset_cloud_only {
                    CloudOnlyState::NotFetched
                } else {
                    previous_loaded
                        .as_ref()
                        .map(|state| state.cloud_only.clone())
                        .unwrap_or(CloudOnlyState::NotFetched)
                };

                CloudBackupDetailState::Loaded {
                    state: LoadedCloudBackupDetail {
                        detail,
                        cloud_only,
                        cloud_only_operation: previous_loaded
                            .as_ref()
                            .map(|state| state.cloud_only_operation.clone())
                            .unwrap_or(CloudOnlyOperation::Idle),
                        other_backups_operation: previous_loaded
                            .as_ref()
                            .map(|state| state.other_backups_operation.clone())
                            .unwrap_or(OtherBackupsOperation::Idle),
                    },
                }
            }
            None => CloudBackupDetailState::NotLoaded,
        };
    }

    fn resolve_cloud_only_state(&mut self, cloud_only: CloudOnlyState) {
        match (&mut self.configured.detail, cloud_only) {
            (CloudBackupDetailState::Loaded { state }, cloud_only) => {
                state.cloud_only = cloud_only;
            }
            (detail, CloudOnlyState::Loading) => {
                *detail = CloudBackupDetailState::Loading;
            }
            (detail, CloudOnlyState::Failed { error }) => {
                *detail = CloudBackupDetailState::Failed(error);
            }
            (detail, CloudOnlyState::NotFetched) => {
                *detail = CloudBackupDetailState::NotLoaded;
            }
            (CloudBackupDetailState::NotLoaded, CloudOnlyState::Loaded { .. })
            | (CloudBackupDetailState::Loading, CloudOnlyState::Loaded { .. })
            | (CloudBackupDetailState::Failed(_), CloudOnlyState::Loaded { .. }) => {}
        }
    }

    fn resolve_cloud_only_operation(&mut self, cloud_only_operation: CloudOnlyOperation) {
        if let CloudBackupDetailState::Loaded { state } = &mut self.configured.detail {
            state.cloud_only_operation = cloud_only_operation;
        }
    }

    fn resolve_other_backups_operation(&mut self, other_backups_operation: OtherBackupsOperation) {
        if let CloudBackupDetailState::Loaded { state } = &mut self.configured.detail {
            state.other_backups_operation = other_backups_operation;
        }
    }

    fn clear_prompt_state(&mut self) {
        self.configured.prompt = CloudBackupConfiguredPrompt::None;
        self.missing_passkey_dismissed = false;

        if matches!(
            self.phase,
            CloudBackupLifecyclePhase::Enabling(
                CloudBackupEnableFlow::AwaitingForceNewConfirmation(_, _)
                    | CloudBackupEnableFlow::AwaitingPasskeyChoice(_)
            )
        ) {
            self.phase = CloudBackupLifecyclePhase::Enabling(
                CloudBackupEnableFlow::DiscoveringExistingBackup,
            );
        }
    }

    fn verification_decision_presentation_for_state(
        state: &CloudBackupModelState,
    ) -> Option<CloudBackupVerificationPresentation> {
        if !matches!(
            state.verification_metadata,
            CloudBackupVerificationMetadata::NeedsVerification
        ) {
            return None;
        }

        if !state.should_prompt_verification {
            return None;
        }

        if !matches!(state.verification(), VerificationState::Idle | VerificationState::Cancelled) {
            return None;
        }

        CloudBackupVerificationCoordinator::needs_decision(
            CloudBackupVerificationReason::BackupChanged,
            CloudBackupVerificationCoordinator::current_source(&state.verification_presentation),
        )
        .presentation
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum CloudBackupModelEvent {
    EnableStarted,
    RestoreStarted,
    BackgroundStatusEntered(CloudBackupStatus),
    RuntimeStatusReconciled(CloudBackupStatus),
    ExistingBackupFoundPromptSet {
        context: CloudBackupEnableContext,
        passkey_hint: Option<CloudBackupPasskeyHint>,
    },
    ExistingBackupFoundPromptCleared,
    PasskeyChoicePromptSet(CloudBackupPasskeyChoiceIntent),
    PasskeyChoicePromptCleared,
    MissingPasskeyPromptDismissed,
    MissingPasskeyDismissalCleared,
    PromptStateCleared,
    EnableProgressReported(Option<super::CloudBackupProgress>),
    RestoreProgressReported(CloudBackupRestoreFlow),
    SyncHealthObserved(CloudSyncHealth),
    EnableFlowAdvanced(CloudBackupEnableState),
    PendingUploadVerificationReconciled(PendingUploadVerificationState),
    PendingUploadVerificationAndFlagsReconciled {
        pending: PendingUploadVerificationState,
        metadata: CloudBackupVerificationMetadata,
        should_prompt: bool,
    },
    VerificationFlagsReconciled {
        metadata: CloudBackupVerificationMetadata,
        should_prompt: bool,
    },
    VerificationPresentationReconciled(CloudBackupVerificationPresentation),
    VerificationStateResolved(VerificationState),
    SyncStateResolved(SyncState),
    RecoveryStateResolved(RecoveryState),
    DisableStateResolved(CloudBackupDisableOutcome),
    DetailRefreshApplied {
        detail: Option<CloudBackupDetail>,
        reset_cloud_only: bool,
    },
    CloudOnlyStateResolved(CloudOnlyState),
    CloudOnlyOperationResolved(CloudOnlyOperation),
    OtherBackupsOperationResolved(OtherBackupsOperation),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CloudBackupModelEventKind {
    EnableStarted,
    RestoreStarted,
    BackgroundStatusEntered,
    RuntimeStatusReconciled,
    ExistingBackupFoundPromptSet,
    ExistingBackupFoundPromptCleared,
    PasskeyChoicePromptSet,
    PasskeyChoicePromptCleared,
    MissingPasskeyPromptDismissed,
    MissingPasskeyDismissalCleared,
    PromptStateCleared,
    EnableProgressReported,
    RestoreProgressReported,
    SyncHealthObserved,
    EnableFlowAdvanced,
    PendingUploadVerificationReconciled,
    PendingUploadVerificationAndFlagsReconciled,
    VerificationFlagsReconciled,
    VerificationPresentationReconciled,
    VerificationStateResolved,
    SyncStateResolved,
    RecoveryStateResolved,
    DisableStateResolved,
    DetailRefreshApplied,
    CloudOnlyStateResolved,
    CloudOnlyOperationResolved,
    OtherBackupsOperationResolved,
}

impl CloudBackupModelEvent {
    pub(crate) fn kind(&self) -> CloudBackupModelEventKind {
        match self {
            Self::EnableStarted => CloudBackupModelEventKind::EnableStarted,
            Self::RestoreStarted => CloudBackupModelEventKind::RestoreStarted,
            Self::BackgroundStatusEntered(_) => CloudBackupModelEventKind::BackgroundStatusEntered,
            Self::RuntimeStatusReconciled(_) => CloudBackupModelEventKind::RuntimeStatusReconciled,
            Self::ExistingBackupFoundPromptSet { .. } => {
                CloudBackupModelEventKind::ExistingBackupFoundPromptSet
            }
            Self::ExistingBackupFoundPromptCleared => {
                CloudBackupModelEventKind::ExistingBackupFoundPromptCleared
            }
            Self::PasskeyChoicePromptSet(_) => CloudBackupModelEventKind::PasskeyChoicePromptSet,
            Self::PasskeyChoicePromptCleared => {
                CloudBackupModelEventKind::PasskeyChoicePromptCleared
            }
            Self::MissingPasskeyPromptDismissed => {
                CloudBackupModelEventKind::MissingPasskeyPromptDismissed
            }
            Self::MissingPasskeyDismissalCleared => {
                CloudBackupModelEventKind::MissingPasskeyDismissalCleared
            }
            Self::PromptStateCleared => CloudBackupModelEventKind::PromptStateCleared,
            Self::EnableProgressReported(_) => CloudBackupModelEventKind::EnableProgressReported,
            Self::RestoreProgressReported(_) => CloudBackupModelEventKind::RestoreProgressReported,
            Self::SyncHealthObserved(_) => CloudBackupModelEventKind::SyncHealthObserved,
            Self::EnableFlowAdvanced(_) => CloudBackupModelEventKind::EnableFlowAdvanced,
            Self::PendingUploadVerificationReconciled(_) => {
                CloudBackupModelEventKind::PendingUploadVerificationReconciled
            }
            Self::PendingUploadVerificationAndFlagsReconciled { .. } => {
                CloudBackupModelEventKind::PendingUploadVerificationAndFlagsReconciled
            }
            Self::VerificationFlagsReconciled { .. } => {
                CloudBackupModelEventKind::VerificationFlagsReconciled
            }
            Self::VerificationPresentationReconciled(_) => {
                CloudBackupModelEventKind::VerificationPresentationReconciled
            }
            Self::VerificationStateResolved(_) => {
                CloudBackupModelEventKind::VerificationStateResolved
            }
            Self::SyncStateResolved(_) => CloudBackupModelEventKind::SyncStateResolved,
            Self::RecoveryStateResolved(_) => CloudBackupModelEventKind::RecoveryStateResolved,
            Self::DisableStateResolved(_) => CloudBackupModelEventKind::DisableStateResolved,
            Self::DetailRefreshApplied { .. } => CloudBackupModelEventKind::DetailRefreshApplied,
            Self::CloudOnlyStateResolved(_) => CloudBackupModelEventKind::CloudOnlyStateResolved,
            Self::CloudOnlyOperationResolved(_) => {
                CloudBackupModelEventKind::CloudOnlyOperationResolved
            }
            Self::OtherBackupsOperationResolved(_) => {
                CloudBackupModelEventKind::OtherBackupsOperationResolved
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum CloudBackupModelEventRejection {
    Busy(CloudBackupStatus),
}

#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct CloudBackupModelEffects {
    pub(crate) lifecycle: Option<CloudBackupLifecycle>,
    pub(crate) status_changed: bool,
    pub(crate) verification_presentation_changed: bool,
    pub(crate) verification_decision_pending: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum CloudBackupPasskeyState {
    Available,
    Missing,
    UnsupportedProvider,
    NeedsRepair { state: CloudBackupPasskeyRepairState },
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum CloudBackupPasskeyRepairState {
    Idle,
    Running,
    Failed(String),
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum CloudBackupVerificationState {
    NotVerified,
    Verified { report: Option<DeepVerificationReport>, last_verified_at: Option<u64> },
    Required,
    Running,
    AwaitingUploadConfirmation,
    Failed(DeepVerificationFailure),
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum CloudBackupSyncState {
    Idle,
    Syncing,
    Blocked(String),
    Failed(String),
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum CloudBackupDestructiveOperationState {
    Idle,
    RecreatingManifest,
    ReinitializingBackup,
    Disabling,
    DisableFailed { message: String, can_keep_enabled: bool },
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct LoadedCloudBackupDetail {
    pub detail: CloudBackupDetail,
    pub cloud_only: CloudOnlyState,
    pub cloud_only_operation: CloudOnlyOperation,
    pub other_backups_operation: OtherBackupsOperation,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum CloudBackupDetailState {
    NotLoaded,
    Loading,
    Loaded { state: LoadedCloudBackupDetail },
    Failed(String),
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct CloudBackupConfiguredState {
    pub passkey: CloudBackupPasskeyState,
    pub verification: CloudBackupVerificationState,
    pub sync: CloudBackupSyncState,
    pub destructive_operation: CloudBackupDestructiveOperationState,
    pub detail: CloudBackupDetailState,
    pub root_prompt: CloudBackupRootPrompt,
    pub sync_health: CloudSyncHealth,
    pub verification_presentation: CloudBackupVerificationPresentation,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum CloudBackupEnableFlow {
    DiscoveringExistingBackup,
    AwaitingForceNewConfirmation(CloudBackupEnableContext, Option<CloudBackupPasskeyHint>),
    AwaitingPasskeyChoice(CloudBackupPasskeyChoiceIntent),
    CreatingPasskey,
    WaitingForPasskeyAvailability,
    AwaitingSavedPasskeyConfirmation(super::SavedPasskeyConfirmationMode),
    ConfirmingSavedPasskey,
    UploadingInitialBackup { progress: Option<super::CloudBackupProgress> },
    RetryingUploadWithStagedMaterial { progress: Option<super::CloudBackupProgress> },
}

#[derive(Debug, Clone, Hash, PartialEq, Eq, uniffi::Enum)]
pub enum CloudBackupRestoreFlow {
    Finding,
    Downloading { completed: u32, total: u32 },
    Restoring { completed: u32, total: u32 },
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct CloudBackupFailure {
    pub message: String,
}

#[expect(clippy::large_enum_variant, reason = "exported UniFFI enum keeps payloads inline")]
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum CloudBackupLifecycle {
    Disabled,
    Enabling(CloudBackupEnableFlow),
    Restoring(CloudBackupRestoreFlow),
    Configured(CloudBackupConfiguredState),
    Failed(CloudBackupFailure),
}

impl CloudBackupModel {
    pub(crate) fn public_state(&self) -> super::CloudBackupState {
        self.state.public_state()
    }

    pub(crate) fn status(&self) -> CloudBackupStatus {
        self.state.status()
    }

    pub(crate) fn verification(&self) -> VerificationState {
        self.state.verification()
    }

    pub(crate) fn verification_presentation(&self) -> &CloudBackupVerificationPresentation {
        &self.state.verification_presentation
    }

    pub(crate) fn detail(&self) -> Option<CloudBackupDetail> {
        self.state.detail()
    }

    pub(crate) fn cloud_only(&self) -> CloudOnlyState {
        self.state.cloud_only()
    }

    pub(crate) fn cloud_only_operation(&self) -> CloudOnlyOperation {
        self.state.cloud_only_operation()
    }

    pub(crate) fn other_backups_operation(&self) -> OtherBackupsOperation {
        self.state.other_backups_operation()
    }

    pub(crate) fn is_awaiting_enable_prompt(&self) -> bool {
        matches!(
            self.state.phase,
            CloudBackupLifecyclePhase::Enabling(
                CloudBackupEnableFlow::AwaitingForceNewConfirmation(_, _)
                    | CloudBackupEnableFlow::AwaitingPasskeyChoice(
                        CloudBackupPasskeyChoiceIntent::Enable(_, _)
                    )
            )
        )
    }

    pub(crate) fn apply_event(
        &mut self,
        event: CloudBackupModelEvent,
    ) -> Result<CloudBackupModelEffects, CloudBackupModelEventRejection> {
        let previous_status = self.state.status();
        let previous_lifecycle = self.state.public_lifecycle();
        let previous_presentation = self.state.verification_presentation.clone();
        let mut effects = CloudBackupModelEffects::default();

        match event {
            CloudBackupModelEvent::EnableStarted => {
                self.state.enter_background_status(CloudBackupStatus::Enabling)?;
            }
            CloudBackupModelEvent::RestoreStarted => {
                self.state.enter_background_status(CloudBackupStatus::Restoring)?;
            }
            CloudBackupModelEvent::BackgroundStatusEntered(status) => {
                self.state.enter_background_status(status)?;
            }
            CloudBackupModelEvent::RuntimeStatusReconciled(status) => {
                self.state.apply_status(status);
            }
            CloudBackupModelEvent::ExistingBackupFoundPromptSet { context, passkey_hint } => {
                self.state.phase = CloudBackupLifecyclePhase::Enabling(
                    CloudBackupEnableFlow::AwaitingForceNewConfirmation(context, passkey_hint),
                );
            }
            CloudBackupModelEvent::ExistingBackupFoundPromptCleared => {
                if matches!(
                    self.state.phase,
                    CloudBackupLifecyclePhase::Enabling(
                        CloudBackupEnableFlow::AwaitingForceNewConfirmation(_, _)
                    )
                ) {
                    self.state.phase = CloudBackupLifecyclePhase::Disabled;
                }
            }
            CloudBackupModelEvent::PasskeyChoicePromptSet(intent) => match &intent {
                CloudBackupPasskeyChoiceIntent::Enable(_, _) => {
                    self.state.phase = CloudBackupLifecyclePhase::Enabling(
                        CloudBackupEnableFlow::AwaitingPasskeyChoice(intent),
                    );
                }
                CloudBackupPasskeyChoiceIntent::RepairPasskey => {
                    self.state.configured.prompt =
                        CloudBackupConfiguredPrompt::PasskeyChoice(intent);
                    self.state.phase = CloudBackupLifecyclePhase::Configured;
                }
            },
            CloudBackupModelEvent::PasskeyChoicePromptCleared => {
                if matches!(
                    self.state.phase,
                    CloudBackupLifecyclePhase::Enabling(
                        CloudBackupEnableFlow::AwaitingPasskeyChoice(
                            CloudBackupPasskeyChoiceIntent::Enable(_, _)
                        )
                    )
                ) {
                    self.state.phase = CloudBackupLifecyclePhase::Disabled;
                }
                self.state.configured.prompt = CloudBackupConfiguredPrompt::None;
            }
            CloudBackupModelEvent::MissingPasskeyPromptDismissed => {
                self.state.missing_passkey_dismissed = true;
            }
            CloudBackupModelEvent::MissingPasskeyDismissalCleared => {
                self.state.missing_passkey_dismissed = false;
            }
            CloudBackupModelEvent::PromptStateCleared => {
                self.state.clear_prompt_state();
            }
            CloudBackupModelEvent::EnableProgressReported(progress) => {
                self.state.report_enable_progress(progress);
            }
            CloudBackupModelEvent::RestoreProgressReported(progress) => {
                self.state.report_restore_progress(progress);
            }
            CloudBackupModelEvent::SyncHealthObserved(sync_health) => {
                self.state.sync_health = sync_health;
            }
            CloudBackupModelEvent::EnableFlowAdvanced(enable_state) => {
                self.state.apply_enable_flow(enable_state);
            }
            CloudBackupModelEvent::PendingUploadVerificationReconciled(pending) => {
                self.state.reconcile_pending_upload_verification(pending);
            }
            CloudBackupModelEvent::PendingUploadVerificationAndFlagsReconciled {
                pending,
                metadata,
                should_prompt,
            } => {
                self.state.reconcile_pending_upload_verification(pending);
                self.state.reconcile_verification_flags(metadata, should_prompt);
                effects.verification_decision_pending =
                    CloudBackupModelState::verification_decision_presentation_for_state(
                        &self.state,
                    )
                    .is_some();
            }
            CloudBackupModelEvent::VerificationFlagsReconciled { metadata, should_prompt } => {
                self.state.reconcile_verification_flags(metadata, should_prompt);
            }
            CloudBackupModelEvent::VerificationPresentationReconciled(presentation) => {
                self.state.verification_presentation = presentation;
            }
            CloudBackupModelEvent::VerificationStateResolved(verification) => {
                if !matches!(verification, VerificationState::Idle | VerificationState::Cancelled) {
                    self.state.reconcile_pending_upload_verification(
                        PendingUploadVerificationState::Idle,
                    );
                }
                self.state.resolve_verification(verification);
            }
            CloudBackupModelEvent::SyncStateResolved(sync) => {
                self.state.resolve_sync(sync);
            }
            CloudBackupModelEvent::RecoveryStateResolved(recovery) => {
                self.state.resolve_recovery(recovery);
            }
            CloudBackupModelEvent::DisableStateResolved(outcome) => {
                self.state.resolve_disable(outcome);
            }
            CloudBackupModelEvent::DetailRefreshApplied { detail, reset_cloud_only } => {
                self.state.apply_detail_refresh(detail, reset_cloud_only);
            }
            CloudBackupModelEvent::CloudOnlyStateResolved(cloud_only) => {
                self.state.resolve_cloud_only_state(cloud_only);
            }
            CloudBackupModelEvent::CloudOnlyOperationResolved(cloud_only_operation) => {
                self.state.resolve_cloud_only_operation(cloud_only_operation);
            }
            CloudBackupModelEvent::OtherBackupsOperationResolved(other_backups_operation) => {
                self.state.resolve_other_backups_operation(other_backups_operation);
            }
        }

        let lifecycle = self.state.public_lifecycle();
        if lifecycle != previous_lifecycle {
            effects.lifecycle = Some(lifecycle);
        }
        effects.status_changed = self.state.status() != previous_status;
        effects.verification_presentation_changed =
            self.state.verification_presentation != previous_presentation;

        Ok(effects)
    }
}

#[cfg(test)]
pub(crate) mod test_support {
    use super::*;
    use crate::manager::cloud_backup_manager::test_support::CloudBackupModelSnapshot;

    impl CloudBackupModel {
        pub(crate) fn snapshot(&self) -> CloudBackupModelSnapshot {
            CloudBackupModelSnapshot {
                root_prompt: self.state.root_prompt(),
                status: self.state.status(),
                sync_health: self.state.sync_health.clone(),
                progress: self.state.progress(),
                restore_progress: restore_progress(&self.state),
                enable_state: enable_state(&self.state),
                pending_upload_verification: pending_upload_verification(&self.state),
                verification_presentation: self.state.verification_presentation.clone(),
                detail: self.state.detail(),
                verification: self.state.verification(),
            }
        }
    }

    fn restore_progress(state: &CloudBackupModelState) -> Option<CloudBackupRestoreFlow> {
        match &state.phase {
            CloudBackupLifecyclePhase::Restoring(flow) => Some(flow.clone()),
            _ => None,
        }
    }

    fn enable_state(state: &CloudBackupModelState) -> CloudBackupEnableState {
        let CloudBackupLifecyclePhase::Enabling(flow) = &state.phase else {
            return CloudBackupEnableState::Idle;
        };

        match flow {
            CloudBackupEnableFlow::CreatingPasskey => CloudBackupEnableState::CreatingPasskey,
            CloudBackupEnableFlow::AwaitingSavedPasskeyConfirmation(mode) => {
                CloudBackupEnableState::AwaitingSavedPasskeyConfirmation(*mode)
            }
            CloudBackupEnableFlow::ConfirmingSavedPasskey => {
                CloudBackupEnableState::ConfirmingSavedPasskey
            }
            CloudBackupEnableFlow::UploadingInitialBackup { .. }
            | CloudBackupEnableFlow::RetryingUploadWithStagedMaterial { .. } => {
                CloudBackupEnableState::UploadingBackup
            }
            CloudBackupEnableFlow::DiscoveringExistingBackup
            | CloudBackupEnableFlow::AwaitingForceNewConfirmation(_, _)
            | CloudBackupEnableFlow::AwaitingPasskeyChoice(_)
            | CloudBackupEnableFlow::WaitingForPasskeyAvailability => CloudBackupEnableState::Idle,
        }
    }

    fn pending_upload_verification(
        state: &CloudBackupModelState,
    ) -> PendingUploadVerificationState {
        state.configured.pending_upload_verification
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manager::cloud_backup_manager::{
        CloudBackupEnableContext, CloudBackupPasskeyHint, CloudBackupProgress,
        CloudBackupVerificationMetadata, CloudBackupVerificationPresentation,
        CloudBackupVerificationReason, CloudBackupVerificationSource, DeepVerificationReport,
    };

    #[test]
    fn disabled_projects_disabled_lifecycle() {
        let model = CloudBackupModel::default();

        assert_eq!(model.public_state().lifecycle, CloudBackupLifecycle::Disabled);
    }

    #[test]
    fn enabling_carries_enable_step_and_progress() {
        let mut model = CloudBackupModel::default();

        model
            .apply_event(CloudBackupModelEvent::EnableFlowAdvanced(
                CloudBackupEnableState::UploadingBackup,
            ))
            .unwrap();
        model
            .apply_event(CloudBackupModelEvent::EnableProgressReported(Some(CloudBackupProgress {
                completed: 1,
                total: 2,
            })))
            .unwrap();

        assert_eq!(
            model.public_state().lifecycle,
            CloudBackupLifecycle::Enabling(CloudBackupEnableFlow::UploadingInitialBackup {
                progress: Some(CloudBackupProgress { completed: 1, total: 2 }),
            }),
        );
    }

    #[test]
    fn enable_started_event_enters_enabling_and_clears_restore_progress() {
        let mut model = CloudBackupModel::default();
        model.apply_event(CloudBackupModelEvent::RestoreStarted).unwrap();
        model
            .apply_event(CloudBackupModelEvent::RuntimeStatusReconciled(CloudBackupStatus::Enabled))
            .unwrap();

        let effects = model.apply_event(CloudBackupModelEvent::EnableStarted).unwrap();

        assert_eq!(model.status(), CloudBackupStatus::Enabling);
        assert_eq!(model.snapshot().progress, None);
        assert_eq!(model.snapshot().restore_progress, None);
        assert!(effects.status_changed);
        assert_eq!(
            effects.lifecycle,
            Some(CloudBackupLifecycle::Enabling(CloudBackupEnableFlow::DiscoveringExistingBackup)),
        );
    }

    #[test]
    fn start_events_enter_background_lifecycle_from_top_level_states() {
        let cases = [
            (
                CloudBackupStatus::Disabled,
                CloudBackupModelEvent::EnableStarted,
                CloudBackupStatus::Enabling,
                CloudBackupLifecycle::Enabling(CloudBackupEnableFlow::DiscoveringExistingBackup),
            ),
            (
                CloudBackupStatus::Enabled,
                CloudBackupModelEvent::EnableStarted,
                CloudBackupStatus::Enabling,
                CloudBackupLifecycle::Enabling(CloudBackupEnableFlow::DiscoveringExistingBackup),
            ),
            (
                CloudBackupStatus::Error("cloud backup failed".into()),
                CloudBackupModelEvent::EnableStarted,
                CloudBackupStatus::Enabling,
                CloudBackupLifecycle::Enabling(CloudBackupEnableFlow::DiscoveringExistingBackup),
            ),
            (
                CloudBackupStatus::Disabled,
                CloudBackupModelEvent::RestoreStarted,
                CloudBackupStatus::Restoring,
                CloudBackupLifecycle::Restoring(CloudBackupRestoreFlow::Finding),
            ),
            (
                CloudBackupStatus::Enabled,
                CloudBackupModelEvent::RestoreStarted,
                CloudBackupStatus::Restoring,
                CloudBackupLifecycle::Restoring(CloudBackupRestoreFlow::Finding),
            ),
            (
                CloudBackupStatus::Error("cloud backup failed".into()),
                CloudBackupModelEvent::RestoreStarted,
                CloudBackupStatus::Restoring,
                CloudBackupLifecycle::Restoring(CloudBackupRestoreFlow::Finding),
            ),
        ];

        for (initial_status, event, expected_status, expected_lifecycle) in cases {
            let mut model = CloudBackupModel::default();
            model
                .apply_event(CloudBackupModelEvent::RuntimeStatusReconciled(initial_status))
                .unwrap();

            let effects = model.apply_event(event).unwrap();

            assert_eq!(model.status(), expected_status);
            assert!(effects.status_changed);
            assert_eq!(effects.lifecycle, Some(expected_lifecycle));
        }
    }

    #[test]
    fn start_events_reject_while_background_lifecycle_is_busy() {
        let cases = [
            (CloudBackupStatus::Enabling, CloudBackupModelEvent::EnableStarted),
            (CloudBackupStatus::Enabling, CloudBackupModelEvent::RestoreStarted),
            (CloudBackupStatus::Restoring, CloudBackupModelEvent::EnableStarted),
            (CloudBackupStatus::Restoring, CloudBackupModelEvent::RestoreStarted),
        ];

        for (busy_status, event) in cases {
            let mut model = CloudBackupModel::default();
            model
                .apply_event(CloudBackupModelEvent::RuntimeStatusReconciled(busy_status.clone()))
                .unwrap();

            let result = model.apply_event(event);

            assert_eq!(result, Err(CloudBackupModelEventRejection::Busy(busy_status.clone())),);
            assert_eq!(model.status(), busy_status);
        }
    }

    #[test]
    fn configured_model_events_emit_effects_and_refresh_lifecycle() {
        let mut model = CloudBackupModel::default();
        model
            .apply_event(CloudBackupModelEvent::RuntimeStatusReconciled(CloudBackupStatus::Enabled))
            .unwrap();

        let effects = model
            .apply_event(CloudBackupModelEvent::PendingUploadVerificationReconciled(
                PendingUploadVerificationState::BlockedOnAuthorization,
            ))
            .unwrap();

        assert_eq!(
            model.snapshot().pending_upload_verification,
            PendingUploadVerificationState::BlockedOnAuthorization,
        );
        assert_eq!(
            effects.lifecycle,
            Some(CloudBackupLifecycle::Configured(CloudBackupConfiguredState {
                passkey: CloudBackupPasskeyState::Available,
                verification: CloudBackupVerificationState::AwaitingUploadConfirmation,
                sync: CloudBackupSyncState::Blocked(
                    PENDING_UPLOAD_AUTHORIZATION_BLOCKED_MESSAGE.into(),
                ),
                destructive_operation: CloudBackupDestructiveOperationState::Idle,
                detail: CloudBackupDetailState::NotLoaded,
                root_prompt: CloudBackupRootPrompt::None,
                sync_health: CloudSyncHealth::Unknown,
                verification_presentation: CloudBackupVerificationPresentation::Hidden {
                    source: None,
                },
            })),
        );
    }

    #[test]
    fn blocked_pending_upload_authorization_survives_sync_resolution() {
        let mut model = CloudBackupModel::default();
        model
            .apply_event(CloudBackupModelEvent::RuntimeStatusReconciled(CloudBackupStatus::Enabled))
            .unwrap();
        model
            .apply_event(CloudBackupModelEvent::PendingUploadVerificationReconciled(
                PendingUploadVerificationState::BlockedOnAuthorization,
            ))
            .unwrap();

        model.apply_event(CloudBackupModelEvent::SyncStateResolved(SyncState::Syncing)).unwrap();

        let CloudBackupLifecycle::Configured(state) = model.public_state().lifecycle else {
            panic!("enabled backup should project configured lifecycle");
        };

        assert_eq!(
            state.sync,
            CloudBackupSyncState::Blocked(PENDING_UPLOAD_AUTHORIZATION_BLOCKED_MESSAGE.into()),
        );
    }

    #[test]
    fn runtime_status_reconcile_can_leave_background_status() {
        let mut model = CloudBackupModel::default();
        model.apply_event(CloudBackupModelEvent::EnableStarted).unwrap();

        let effects = model
            .apply_event(CloudBackupModelEvent::RuntimeStatusReconciled(CloudBackupStatus::Enabled))
            .unwrap();

        assert!(effects.status_changed);
        assert!(matches!(effects.lifecycle, Some(CloudBackupLifecycle::Configured(_)),));
    }

    #[test]
    fn runtime_enabled_preserves_disable_failed_signal() {
        let mut model = CloudBackupModel::default();
        model
            .apply_event(CloudBackupModelEvent::RuntimeStatusReconciled(CloudBackupStatus::Enabled))
            .unwrap();
        model
            .apply_event(CloudBackupModelEvent::DisableStateResolved(
                CloudBackupDisableOutcome::Failed {
                    message: "blocked".into(),
                    can_keep_enabled: true,
                },
            ))
            .unwrap();

        model
            .apply_event(CloudBackupModelEvent::RuntimeStatusReconciled(CloudBackupStatus::Enabled))
            .unwrap();

        let CloudBackupLifecycle::Configured(state) = model.public_state().lifecycle else {
            panic!("enabled backup should project configured lifecycle");
        };
        assert_eq!(
            state.destructive_operation,
            CloudBackupDestructiveOperationState::DisableFailed {
                message: "blocked".into(),
                can_keep_enabled: true,
            }
        );
    }

    #[test]
    fn returned_to_idle_clears_disable_failed_signal() {
        let mut model = CloudBackupModel::default();
        model
            .apply_event(CloudBackupModelEvent::RuntimeStatusReconciled(CloudBackupStatus::Enabled))
            .unwrap();
        model
            .apply_event(CloudBackupModelEvent::DisableStateResolved(
                CloudBackupDisableOutcome::Failed {
                    message: "blocked".into(),
                    can_keep_enabled: true,
                },
            ))
            .unwrap();

        model
            .apply_event(CloudBackupModelEvent::DisableStateResolved(
                CloudBackupDisableOutcome::ReturnedToIdle,
            ))
            .unwrap();

        let CloudBackupLifecycle::Configured(state) = model.public_state().lifecycle else {
            panic!("enabled backup should project configured lifecycle");
        };
        assert_eq!(state.destructive_operation, CloudBackupDestructiveOperationState::Idle);
    }

    #[test]
    fn disable_started_clears_configured_prompt() {
        let mut model = CloudBackupModel::default();
        model
            .apply_event(CloudBackupModelEvent::RuntimeStatusReconciled(CloudBackupStatus::Enabled))
            .unwrap();
        model
            .apply_event(CloudBackupModelEvent::PasskeyChoicePromptSet(
                CloudBackupPasskeyChoiceIntent::RepairPasskey,
            ))
            .unwrap();

        model
            .apply_event(CloudBackupModelEvent::DisableStateResolved(
                CloudBackupDisableOutcome::Started,
            ))
            .unwrap();

        let CloudBackupLifecycle::Configured(state) = model.public_state().lifecycle else {
            panic!("enabled backup should project configured lifecycle");
        };
        assert_eq!(state.root_prompt, CloudBackupRootPrompt::None);
        assert_eq!(state.destructive_operation, CloudBackupDestructiveOperationState::Disabling);
    }

    #[test]
    fn no_op_model_event_emits_no_effects() {
        let report = DeepVerificationReport {
            master_key_wrapper_repaired: false,
            local_master_key_repaired: false,
            credential_recovered: false,
            wallets_verified: 1,
            wallets_failed: 0,
            wallets_unsupported: 0,
            detail: None,
        };
        let mut model = CloudBackupModel::default();
        model
            .apply_event(CloudBackupModelEvent::VerificationStateResolved(
                VerificationState::Verified(report.clone()),
            ))
            .unwrap();

        let effects = model
            .apply_event(CloudBackupModelEvent::VerificationStateResolved(
                VerificationState::Verified(report),
            ))
            .unwrap();

        assert_eq!(effects, CloudBackupModelEffects::default());
    }

    #[test]
    fn restoring_carries_restore_progress() {
        let progress = CloudBackupRestoreFlow::Downloading { completed: 1, total: 3 };
        let mut model = CloudBackupModel::default();

        model.apply_event(CloudBackupModelEvent::RestoreStarted).unwrap();
        model
            .apply_event(CloudBackupModelEvent::RestoreProgressReported(progress.clone()))
            .unwrap();

        assert_eq!(model.public_state().lifecycle, CloudBackupLifecycle::Restoring(progress));
    }

    #[test]
    fn stray_enable_progress_does_not_enter_enabling() {
        let mut model = CloudBackupModel::default();

        let effects = model
            .apply_event(CloudBackupModelEvent::EnableProgressReported(Some(CloudBackupProgress {
                completed: 1,
                total: 2,
            })))
            .unwrap();

        assert_eq!(model.public_state().lifecycle, CloudBackupLifecycle::Disabled);
        assert_eq!(effects, CloudBackupModelEffects::default());
    }

    #[test]
    fn stray_restore_progress_does_not_enter_restoring() {
        let mut model = CloudBackupModel::default();

        let effects = model
            .apply_event(CloudBackupModelEvent::RestoreProgressReported(
                CloudBackupRestoreFlow::Downloading { completed: 1, total: 3 },
            ))
            .unwrap();

        assert_eq!(model.public_state().lifecycle, CloudBackupLifecycle::Disabled);
        assert_eq!(effects, CloudBackupModelEffects::default());
    }

    #[test]
    fn configured_projects_passkey_and_verification_state() {
        let report = DeepVerificationReport {
            master_key_wrapper_repaired: false,
            local_master_key_repaired: false,
            credential_recovered: false,
            wallets_verified: 1,
            wallets_failed: 0,
            wallets_unsupported: 0,
            detail: None,
        };
        let mut model = CloudBackupModel::default();

        model
            .apply_event(CloudBackupModelEvent::RuntimeStatusReconciled(CloudBackupStatus::Enabled))
            .unwrap();
        model
            .apply_event(CloudBackupModelEvent::VerificationStateResolved(
                VerificationState::Verified(report),
            ))
            .unwrap();
        model.apply_event(CloudBackupModelEvent::SyncStateResolved(SyncState::Syncing)).unwrap();
        model
            .apply_event(CloudBackupModelEvent::PendingUploadVerificationReconciled(
                PendingUploadVerificationState::Confirming,
            ))
            .unwrap();

        let CloudBackupLifecycle::Configured(state) = model.public_state().lifecycle else {
            panic!("enabled backup should project configured lifecycle");
        };

        assert_eq!(state.passkey, CloudBackupPasskeyState::Available);
        assert_eq!(state.verification, CloudBackupVerificationState::AwaitingUploadConfirmation);
        assert_eq!(state.sync, CloudBackupSyncState::Syncing);
    }

    #[test]
    fn passkey_missing_projects_missing_or_repairing() {
        let mut missing = CloudBackupModel::default();
        missing
            .apply_event(CloudBackupModelEvent::RuntimeStatusReconciled(
                CloudBackupStatus::PasskeyMissing,
            ))
            .unwrap();

        let CloudBackupLifecycle::Configured(state) = missing.public_state().lifecycle else {
            panic!("passkey-missing backup should still be configured");
        };
        assert_eq!(
            state.passkey,
            CloudBackupPasskeyState::NeedsRepair { state: CloudBackupPasskeyRepairState::Idle }
        );

        let mut repairing = CloudBackupModel::default();
        repairing
            .apply_event(CloudBackupModelEvent::RecoveryStateResolved(RecoveryState::Recovering(
                RecoveryAction::RepairPasskey,
            )))
            .unwrap();

        let CloudBackupLifecycle::Configured(state) = repairing.public_state().lifecycle else {
            panic!("repairing backup should still be configured");
        };
        assert_eq!(
            state.passkey,
            CloudBackupPasskeyState::NeedsRepair { state: CloudBackupPasskeyRepairState::Running }
        );
    }

    #[test]
    fn verification_flags_event_opens_decision_prompt() {
        let mut model = CloudBackupModel::default();
        model
            .apply_event(CloudBackupModelEvent::RuntimeStatusReconciled(CloudBackupStatus::Enabled))
            .unwrap();
        model
            .apply_event(CloudBackupModelEvent::VerificationPresentationReconciled(
                CloudBackupVerificationPresentation::Hidden {
                    source: Some(CloudBackupVerificationSource::Settings),
                },
            ))
            .unwrap();

        let effects = model
            .apply_event(CloudBackupModelEvent::VerificationFlagsReconciled {
                metadata: CloudBackupVerificationMetadata::NeedsVerification,
                should_prompt: true,
            })
            .unwrap();

        assert!(effects.verification_presentation_changed);
        assert_eq!(
            model.snapshot().verification_presentation,
            CloudBackupVerificationPresentation::NeedsDecision {
                reason: CloudBackupVerificationReason::BackupChanged,
                source: CloudBackupVerificationSource::Settings,
            },
        );
        assert!(matches!(effects.lifecycle, Some(CloudBackupLifecycle::Configured(_)),));
    }

    #[test]
    fn verification_flags_event_dismisses_stale_decision_prompt() {
        let mut model = CloudBackupModel::default();
        model
            .apply_event(CloudBackupModelEvent::RuntimeStatusReconciled(CloudBackupStatus::Enabled))
            .unwrap();
        model
            .apply_event(CloudBackupModelEvent::VerificationFlagsReconciled {
                metadata: CloudBackupVerificationMetadata::NeedsVerification,
                should_prompt: true,
            })
            .unwrap();
        model
            .apply_event(CloudBackupModelEvent::VerificationPresentationReconciled(
                CloudBackupVerificationPresentation::NeedsDecision {
                    reason: CloudBackupVerificationReason::BackupChanged,
                    source: CloudBackupVerificationSource::RootPrompt,
                },
            ))
            .unwrap();

        let effects = model
            .apply_event(CloudBackupModelEvent::VerificationFlagsReconciled {
                metadata: CloudBackupVerificationMetadata::Verified(42),
                should_prompt: false,
            })
            .unwrap();

        assert!(effects.verification_presentation_changed);
        assert_eq!(
            model.snapshot().verification_presentation,
            CloudBackupVerificationPresentation::Hidden {
                source: Some(CloudBackupVerificationSource::RootPrompt),
            },
        );
    }

    #[test]
    fn pending_upload_refresh_tracks_decision_pending_without_duplicate_presentation() {
        let mut model = CloudBackupModel::default();
        model
            .apply_event(CloudBackupModelEvent::RuntimeStatusReconciled(CloudBackupStatus::Enabled))
            .unwrap();
        model
            .apply_event(CloudBackupModelEvent::VerificationFlagsReconciled {
                metadata: CloudBackupVerificationMetadata::NeedsVerification,
                should_prompt: true,
            })
            .unwrap();

        let effects = model
            .apply_event(CloudBackupModelEvent::PendingUploadVerificationAndFlagsReconciled {
                pending: PendingUploadVerificationState::Idle,
                metadata: CloudBackupVerificationMetadata::NeedsVerification,
                should_prompt: true,
            })
            .unwrap();

        assert!(effects.verification_decision_pending);
        assert!(!effects.verification_presentation_changed);
    }

    #[test]
    fn root_prompt_projects_root_prompt() {
        let hint = CloudBackupPasskeyHint {
            provider_name: Some("iCloud Keychain".into()),
            name_suffix: "abc123".into(),
            registered_at: 1,
        };
        let mut model = CloudBackupModel::default();

        model
            .apply_event(CloudBackupModelEvent::ExistingBackupFoundPromptSet {
                context: CloudBackupEnableContext::settings_manual(),
                passkey_hint: Some(hint.clone()),
            })
            .unwrap();

        assert_eq!(
            model.snapshot().root_prompt,
            CloudBackupRootPrompt::ExistingBackupFound(
                CloudBackupEnableContext::settings_manual(),
                Some(hint),
            ),
        );
        assert_eq!(
            model.public_state().lifecycle,
            CloudBackupLifecycle::Enabling(CloudBackupEnableFlow::AwaitingForceNewConfirmation(
                CloudBackupEnableContext::settings_manual(),
                Some(CloudBackupPasskeyHint {
                    provider_name: Some("iCloud Keychain".into()),
                    name_suffix: "abc123".into(),
                    registered_at: 1,
                }),
            )),
        );
    }

    #[test]
    fn root_prompt_is_derived_from_configured_state() {
        let mut model = CloudBackupModel::default();

        let effects = model
            .apply_event(CloudBackupModelEvent::RuntimeStatusReconciled(
                CloudBackupStatus::PasskeyMissing,
            ))
            .unwrap();

        assert_eq!(model.snapshot().root_prompt, CloudBackupRootPrompt::MissingPasskeyReminder);
        assert!(matches!(effects.lifecycle, Some(CloudBackupLifecycle::Configured(_))));
    }
}
