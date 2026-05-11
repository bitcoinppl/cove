use super::{
    CloudBackupDetail, CloudBackupEnableContext, CloudBackupEnableState, CloudBackupModelSnapshot,
    CloudBackupPasskeyChoiceIntent, CloudBackupPasskeyHint, CloudBackupRestoreProgress,
    CloudBackupRestoreReport, CloudBackupRootPrompt, CloudBackupStatus,
    CloudBackupVerificationMetadata, CloudBackupVerificationPresentation,
    CloudBackupVerificationReason, CloudOnlyOperation, CloudOnlyState, DeepVerificationFailure,
    DeepVerificationReport, OtherBackupsOperation, PendingUploadVerificationState, RecoveryAction,
    RecoveryState, SyncState, VerificationState,
};

use super::verify::coordinator::CloudBackupVerificationCoordinator;
use cove_device::cloud_storage::CloudSyncHealth;

#[derive(Debug, Clone)]
pub(crate) struct CloudBackupModel {
    state: CloudBackupModelState,
}

#[derive(Debug, Clone)]
struct CloudBackupModelState {
    lifecycle: CloudBackupLifecycle,
    sync_health: CloudSyncHealth,
    missing_passkey_dismissed: bool,
    should_prompt_verification: bool,
    verification_metadata: CloudBackupVerificationMetadata,
    verification_presentation: CloudBackupVerificationPresentation,
}

impl Default for CloudBackupModelState {
    fn default() -> Self {
        Self {
            lifecycle: CloudBackupLifecycle::Disabled,
            sync_health: CloudSyncHealth::Unknown,
            missing_passkey_dismissed: false,
            should_prompt_verification: false,
            verification_metadata: CloudBackupVerificationMetadata::NotConfigured,
            verification_presentation: CloudBackupVerificationPresentation::Hidden { source: None },
        }
    }
}

impl From<CloudBackupModelSnapshot> for CloudBackupModelState {
    fn from(state: CloudBackupModelSnapshot) -> Self {
        let lifecycle = CloudBackupModelState::project_lifecycle_from_snapshot(&state);

        Self {
            lifecycle,
            sync_health: state.sync_health,
            missing_passkey_dismissed: state.missing_passkey_dismissed,
            should_prompt_verification: state.should_prompt_verification,
            verification_metadata: state.verification_metadata,
            verification_presentation: state.verification_presentation,
        }
    }
}

impl CloudBackupModelState {
    fn snapshot(&self) -> CloudBackupModelSnapshot {
        CloudBackupModelSnapshot {
            lifecycle: self.lifecycle.clone(),
            root_prompt: self.explicit_root_prompt(),
            status: self.status(),
            sync_health: self.sync_health.clone(),
            progress: self.progress(),
            restore_progress: self.restore_progress(),
            restore_report: self.restore_report(),
            enable_state: self.enable_state(),
            pending_upload_verification: self.pending_upload_verification(),
            missing_passkey_dismissed: self.missing_passkey_dismissed,
            should_prompt_verification: self.should_prompt_verification,
            verification_metadata: self.verification_metadata.clone(),
            verification_presentation: self.verification_presentation.clone(),
            detail: self.detail(),
            verification: self.verification(),
            sync: self.sync(),
            recovery: self.recovery(),
            cloud_only: self.cloud_only(),
            cloud_only_operation: self.cloud_only_operation(),
            other_backups_operation: self.other_backups_operation(),
        }
    }

    fn explicit_root_prompt(&self) -> CloudBackupRootPrompt {
        match &self.lifecycle {
            CloudBackupLifecycle::Enabling(
                CloudBackupEnableFlow::AwaitingForceNewConfirmation(context, passkey_hint),
            ) => CloudBackupRootPrompt::ExistingBackupFound(*context, passkey_hint.clone()),
            CloudBackupLifecycle::Enabling(CloudBackupEnableFlow::AwaitingPasskeyChoice(
                intent,
            )) => CloudBackupRootPrompt::PasskeyChoice(intent.clone()),
            CloudBackupLifecycle::Enabling(
                CloudBackupEnableFlow::DiscoveringExistingBackup
                | CloudBackupEnableFlow::CreatingPasskey
                | CloudBackupEnableFlow::WaitingForPasskeyAvailability
                | CloudBackupEnableFlow::AwaitingSavedPasskeyConfirmation(_)
                | CloudBackupEnableFlow::ConfirmingSavedPasskey
                | CloudBackupEnableFlow::UploadingInitialBackup { .. }
                | CloudBackupEnableFlow::RetryingUploadWithStagedMaterial { .. },
            ) => CloudBackupRootPrompt::None,
            CloudBackupLifecycle::Configured(configured) => match &configured.root_prompt {
                CloudBackupRootPrompt::PasskeyChoice(intent) => {
                    CloudBackupRootPrompt::PasskeyChoice(intent.clone())
                }
                CloudBackupRootPrompt::None
                | CloudBackupRootPrompt::ExistingBackupFound(_, _)
                | CloudBackupRootPrompt::MissingPasskeyReminder
                | CloudBackupRootPrompt::Verification => CloudBackupRootPrompt::None,
            },
            CloudBackupLifecycle::Disabled
            | CloudBackupLifecycle::Restoring(_)
            | CloudBackupLifecycle::Failed(_) => CloudBackupRootPrompt::None,
        }
    }

    fn status(&self) -> CloudBackupStatus {
        match &self.lifecycle {
            CloudBackupLifecycle::Disabled => CloudBackupStatus::Disabled,
            CloudBackupLifecycle::Enabling(_) => CloudBackupStatus::Enabling,
            CloudBackupLifecycle::Restoring(_) => CloudBackupStatus::Restoring,
            CloudBackupLifecycle::Configured(configured) => match &configured.passkey {
                CloudBackupPasskeyState::Missing | CloudBackupPasskeyState::NeedsRepair { .. } => {
                    CloudBackupStatus::PasskeyMissing
                }
                CloudBackupPasskeyState::UnsupportedProvider => {
                    CloudBackupStatus::UnsupportedPasskeyProvider
                }
                CloudBackupPasskeyState::Available => CloudBackupStatus::Enabled,
            },
            CloudBackupLifecycle::Failed(failure) => {
                CloudBackupStatus::Error(failure.message.clone())
            }
        }
    }

    fn progress(&self) -> Option<super::CloudBackupProgress> {
        match &self.lifecycle {
            CloudBackupLifecycle::Enabling(CloudBackupEnableFlow::UploadingInitialBackup {
                progress,
            })
            | CloudBackupLifecycle::Enabling(
                CloudBackupEnableFlow::RetryingUploadWithStagedMaterial { progress },
            ) => *progress,
            _ => None,
        }
    }

    fn restore_progress(&self) -> Option<CloudBackupRestoreProgress> {
        match &self.lifecycle {
            CloudBackupLifecycle::Restoring(flow) => flow.progress.clone(),
            _ => None,
        }
    }

    fn restore_report(&self) -> Option<CloudBackupRestoreReport> {
        match &self.lifecycle {
            CloudBackupLifecycle::Restoring(flow) => flow.report.clone(),
            CloudBackupLifecycle::Configured(configured) => configured.last_restore_report.clone(),
            CloudBackupLifecycle::Failed(failure) => failure.restore_report.clone(),
            _ => None,
        }
    }

    fn enable_state(&self) -> CloudBackupEnableState {
        let CloudBackupLifecycle::Enabling(flow) = &self.lifecycle else {
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

    fn pending_upload_verification(&self) -> PendingUploadVerificationState {
        let CloudBackupLifecycle::Configured(configured) = &self.lifecycle else {
            return PendingUploadVerificationState::Idle;
        };

        if !matches!(
            configured.verification,
            CloudBackupVerificationState::AwaitingUploadConfirmation
        ) {
            return PendingUploadVerificationState::Idle;
        }

        match configured.sync {
            CloudBackupSyncState::Blocked(_) => {
                PendingUploadVerificationState::BlockedOnAuthorization
            }
            CloudBackupSyncState::Idle
            | CloudBackupSyncState::Syncing
            | CloudBackupSyncState::Failed(_) => PendingUploadVerificationState::Confirming,
        }
    }

    fn detail(&self) -> Option<CloudBackupDetail> {
        self.loaded_detail().map(|state| state.detail.clone())
    }

    fn verification(&self) -> VerificationState {
        let CloudBackupLifecycle::Configured(configured) = &self.lifecycle else {
            return VerificationState::Idle;
        };

        match &configured.verification {
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

    fn sync(&self) -> SyncState {
        let CloudBackupLifecycle::Configured(configured) = &self.lifecycle else {
            return SyncState::Idle;
        };

        match &configured.sync {
            CloudBackupSyncState::Idle | CloudBackupSyncState::Blocked(_) => SyncState::Idle,
            CloudBackupSyncState::Syncing => SyncState::Syncing,
            CloudBackupSyncState::Failed(message) => SyncState::Failed(message.clone()),
        }
    }

    fn recovery(&self) -> RecoveryState {
        let CloudBackupLifecycle::Configured(configured) = &self.lifecycle else {
            return RecoveryState::Idle;
        };

        match &configured.passkey {
            CloudBackupPasskeyState::NeedsRepair {
                state: CloudBackupPasskeyRepairState::Running,
            } => RecoveryState::Recovering(RecoveryAction::RepairPasskey),
            CloudBackupPasskeyState::NeedsRepair {
                state: CloudBackupPasskeyRepairState::Failed(error),
            } => RecoveryState::Failed {
                action: RecoveryAction::RepairPasskey,
                error: error.clone(),
            },
            CloudBackupPasskeyState::Available
            | CloudBackupPasskeyState::Missing
            | CloudBackupPasskeyState::UnsupportedProvider
            | CloudBackupPasskeyState::NeedsRepair { state: CloudBackupPasskeyRepairState::Idle } => {
                RecoveryState::Idle
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
        match &self.lifecycle {
            CloudBackupLifecycle::Configured(CloudBackupConfiguredState {
                detail: CloudBackupDetailState::Loaded { state },
                ..
            }) => Some(state),
            _ => None,
        }
    }

    fn project_lifecycle_from_snapshot(
        snapshot: &CloudBackupModelSnapshot,
    ) -> CloudBackupLifecycle {
        match &snapshot.status {
            CloudBackupStatus::Disabled => CloudBackupLifecycle::Disabled,
            CloudBackupStatus::Enabling => {
                CloudBackupLifecycle::Enabling(Self::enable_flow_from_snapshot(snapshot))
            }
            CloudBackupStatus::Restoring => {
                CloudBackupLifecycle::Restoring(CloudBackupRestoreFlow {
                    progress: snapshot.restore_progress.clone(),
                    report: snapshot.restore_report.clone(),
                })
            }
            CloudBackupStatus::Enabled
            | CloudBackupStatus::PasskeyMissing
            | CloudBackupStatus::UnsupportedPasskeyProvider => {
                CloudBackupLifecycle::Configured(Self::configured_from_snapshot(snapshot))
            }
            CloudBackupStatus::Error(message) => CloudBackupLifecycle::Failed(CloudBackupFailure {
                message: message.clone(),
                restore_report: snapshot.restore_report.clone(),
            }),
        }
    }

    fn enable_flow_from_snapshot(snapshot: &CloudBackupModelSnapshot) -> CloudBackupEnableFlow {
        if let (CloudBackupEnableState::Idle, Some(flow)) =
            (&snapshot.enable_state, Self::prompt_enable_flow(&snapshot.root_prompt))
        {
            return flow;
        }

        match &snapshot.enable_state {
            CloudBackupEnableState::Idle => CloudBackupEnableFlow::DiscoveringExistingBackup,
            CloudBackupEnableState::CreatingPasskey => CloudBackupEnableFlow::CreatingPasskey,
            CloudBackupEnableState::AwaitingSavedPasskeyConfirmation(mode) => {
                CloudBackupEnableFlow::AwaitingSavedPasskeyConfirmation(*mode)
            }
            CloudBackupEnableState::ConfirmingSavedPasskey => {
                CloudBackupEnableFlow::ConfirmingSavedPasskey
            }
            CloudBackupEnableState::UploadingBackup => {
                CloudBackupEnableFlow::UploadingInitialBackup { progress: snapshot.progress }
            }
        }
    }

    fn prompt_enable_flow(root_prompt: &CloudBackupRootPrompt) -> Option<CloudBackupEnableFlow> {
        match root_prompt {
            CloudBackupRootPrompt::ExistingBackupFound(context, passkey_hint) => Some(
                CloudBackupEnableFlow::AwaitingForceNewConfirmation(*context, passkey_hint.clone()),
            ),
            CloudBackupRootPrompt::PasskeyChoice(intent) => {
                Some(CloudBackupEnableFlow::AwaitingPasskeyChoice(intent.clone()))
            }
            CloudBackupRootPrompt::None
            | CloudBackupRootPrompt::MissingPasskeyReminder
            | CloudBackupRootPrompt::Verification => None,
        }
    }

    fn configured_from_snapshot(snapshot: &CloudBackupModelSnapshot) -> CloudBackupConfiguredState {
        CloudBackupConfiguredState {
            passkey: Self::passkey_from_snapshot(snapshot),
            verification: Self::verification_from_snapshot(snapshot),
            sync: Self::sync_from_snapshot(snapshot),
            detail: Self::detail_from_snapshot(snapshot),
            last_restore_report: snapshot.restore_report.clone(),
            root_prompt: snapshot.root_prompt.clone(),
            sync_health: snapshot.sync_health.clone(),
            verification_presentation: snapshot.verification_presentation.clone(),
        }
    }

    fn passkey_from_snapshot(snapshot: &CloudBackupModelSnapshot) -> CloudBackupPasskeyState {
        match &snapshot.status {
            CloudBackupStatus::PasskeyMissing => match &snapshot.recovery {
                RecoveryState::Recovering(RecoveryAction::RepairPasskey) => {
                    CloudBackupPasskeyState::NeedsRepair {
                        state: CloudBackupPasskeyRepairState::Running,
                    }
                }
                RecoveryState::Failed { action: RecoveryAction::RepairPasskey, error } => {
                    CloudBackupPasskeyState::NeedsRepair {
                        state: CloudBackupPasskeyRepairState::Failed(error.clone()),
                    }
                }
                RecoveryState::Idle
                | RecoveryState::Recovering(_)
                | RecoveryState::Failed { .. } => CloudBackupPasskeyState::NeedsRepair {
                    state: CloudBackupPasskeyRepairState::Idle,
                },
            },
            CloudBackupStatus::UnsupportedPasskeyProvider => {
                CloudBackupPasskeyState::UnsupportedProvider
            }
            CloudBackupStatus::Enabled
            | CloudBackupStatus::Enabling
            | CloudBackupStatus::Restoring => CloudBackupPasskeyState::Available,
            CloudBackupStatus::Disabled | CloudBackupStatus::Error(_) => {
                CloudBackupPasskeyState::Missing
            }
        }
    }

    fn verification_from_snapshot(
        snapshot: &CloudBackupModelSnapshot,
    ) -> CloudBackupVerificationState {
        match &snapshot.pending_upload_verification {
            PendingUploadVerificationState::Confirming
            | PendingUploadVerificationState::BlockedOnAuthorization => {
                return CloudBackupVerificationState::AwaitingUploadConfirmation;
            }
            PendingUploadVerificationState::Idle => {}
        }

        match &snapshot.verification {
            VerificationState::Idle | VerificationState::Cancelled => {
                if snapshot.should_prompt_verification {
                    CloudBackupVerificationState::Required
                } else {
                    CloudBackupVerificationState::NotVerified
                }
            }
            VerificationState::Verifying => CloudBackupVerificationState::Running,
            VerificationState::Verified(report) => CloudBackupVerificationState::Verified {
                report: Some(report.clone()),
                last_verified_at: Self::last_verified_at(&snapshot.verification_metadata),
            },
            VerificationState::PasskeyConfirmed => CloudBackupVerificationState::Verified {
                report: None,
                last_verified_at: Self::last_verified_at(&snapshot.verification_metadata),
            },
            VerificationState::Failed(failure) => {
                CloudBackupVerificationState::Failed(failure.clone())
            }
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

    fn sync_from_snapshot(snapshot: &CloudBackupModelSnapshot) -> CloudBackupSyncState {
        match &snapshot.pending_upload_verification {
            PendingUploadVerificationState::BlockedOnAuthorization => {
                return CloudBackupSyncState::Blocked("cloud authorization required".into());
            }
            PendingUploadVerificationState::Idle | PendingUploadVerificationState::Confirming => {}
        }

        match &snapshot.sync {
            SyncState::Idle => CloudBackupSyncState::Idle,
            SyncState::Syncing => CloudBackupSyncState::Syncing,
            SyncState::Failed(message) => CloudBackupSyncState::Failed(message.clone()),
        }
    }

    fn detail_from_snapshot(snapshot: &CloudBackupModelSnapshot) -> CloudBackupDetailState {
        match &snapshot.cloud_only {
            CloudOnlyState::Loading => return CloudBackupDetailState::Loading,
            CloudOnlyState::Failed { error } if snapshot.detail.is_none() => {
                return CloudBackupDetailState::Failed(error.clone());
            }
            CloudOnlyState::NotFetched
            | CloudOnlyState::Loaded { .. }
            | CloudOnlyState::Failed { .. } => {}
        }

        let Some(detail) = &snapshot.detail else {
            return CloudBackupDetailState::NotLoaded;
        };

        CloudBackupDetailState::Loaded {
            state: LoadedCloudBackupDetail {
                detail: detail.clone(),
                cloud_only: snapshot.cloud_only.clone(),
                cloud_only_operation: snapshot.cloud_only_operation.clone(),
                other_backups_operation: snapshot.other_backups_operation.clone(),
            },
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
/// Private model adapter events used while manager workers still report through narrow callbacks
pub(crate) enum CloudBackupModelEvent {
    EnableStarted,
    RestoreStarted,
    RootPromptRefreshed,
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
    StatusEntered(CloudBackupStatus),
    StatusUpdated(CloudBackupStatus),
    ProgressUpdated(Option<super::CloudBackupProgress>),
    RestoreProgressUpdated(Option<CloudBackupRestoreProgress>),
    RestoreReportUpdated(Option<CloudBackupRestoreReport>),
    SyncHealthUpdated(CloudSyncHealth),
    EnableStateUpdated(CloudBackupEnableState),
    PendingUploadVerificationUpdated(PendingUploadVerificationState),
    PendingUploadVerificationRefreshed {
        pending: PendingUploadVerificationState,
        metadata: CloudBackupVerificationMetadata,
        should_prompt: bool,
    },
    VerificationFlagsRefreshed {
        metadata: CloudBackupVerificationMetadata,
        should_prompt: bool,
    },
    VerificationPresentationUpdated(CloudBackupVerificationPresentation),
    VerificationUpdated(VerificationState),
    SyncUpdated(SyncState),
    RecoveryUpdated(RecoveryState),
    DetailUpdated {
        detail: Option<CloudBackupDetail>,
        reset_cloud_only: bool,
    },
    CloudOnlyUpdated(CloudOnlyState),
    CloudOnlyOperationUpdated(CloudOnlyOperation),
    OtherBackupsOperationUpdated(OtherBackupsOperation),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum CloudBackupModelEventRejection {
    Busy(CloudBackupStatus),
}

#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct CloudBackupModelEffects {
    pub(crate) root_prompt: Option<CloudBackupRootPrompt>,
    pub(crate) status: Option<CloudBackupStatus>,
    pub(crate) progress: Option<Option<super::CloudBackupProgress>>,
    pub(crate) restore_progress: Option<Option<CloudBackupRestoreProgress>>,
    pub(crate) restore_report: Option<Option<CloudBackupRestoreReport>>,
    pub(crate) sync_health: Option<CloudSyncHealth>,
    pub(crate) enable_state: Option<CloudBackupEnableState>,
    pub(crate) pending_upload_verification: Option<PendingUploadVerificationState>,
    pub(crate) verification_metadata: Option<CloudBackupVerificationMetadata>,
    pub(crate) should_prompt_verification: Option<bool>,
    pub(crate) verification_presentation: Option<CloudBackupVerificationPresentation>,
    pub(crate) verification_decision_pending: bool,
    pub(crate) verification: Option<VerificationState>,
    pub(crate) sync: Option<SyncState>,
    pub(crate) recovery: Option<RecoveryState>,
    pub(crate) detail: Option<Option<CloudBackupDetail>>,
    pub(crate) cloud_only: Option<CloudOnlyState>,
    pub(crate) cloud_only_operation: Option<CloudOnlyOperation>,
    pub(crate) other_backups_operation: Option<OtherBackupsOperation>,
    pub(crate) lifecycle: Option<CloudBackupLifecycle>,
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
    pub detail: CloudBackupDetailState,
    pub last_restore_report: Option<CloudBackupRestoreReport>,
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

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct CloudBackupRestoreFlow {
    pub progress: Option<CloudBackupRestoreProgress>,
    pub report: Option<CloudBackupRestoreReport>,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct CloudBackupFailure {
    pub message: String,
    pub restore_report: Option<CloudBackupRestoreReport>,
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

impl Default for CloudBackupModel {
    fn default() -> Self {
        Self::from_snapshot(CloudBackupModelSnapshot::default())
    }
}

impl CloudBackupModel {
    pub(crate) fn from_snapshot(state: CloudBackupModelSnapshot) -> Self {
        Self { state: state.into() }
    }

    pub(crate) fn snapshot(&self) -> CloudBackupModelSnapshot {
        let mut state = self.state.snapshot();
        state.root_prompt = Self::resolve_root_prompt(&state);
        state.projected_with_fresh_domain()
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

    pub(crate) fn other_backups_operation(&self) -> OtherBackupsOperation {
        self.state.other_backups_operation()
    }

    pub(crate) fn apply_event(
        &mut self,
        event: CloudBackupModelEvent,
    ) -> Result<CloudBackupModelEffects, CloudBackupModelEventRejection> {
        let previous_lifecycle = self.state.lifecycle.clone();
        let mut next_state = self.state.snapshot();
        let mut effects = CloudBackupModelEffects::default();

        match event {
            CloudBackupModelEvent::EnableStarted => {
                Self::enter_background_status(
                    &mut next_state,
                    CloudBackupStatus::Enabling,
                    &mut effects,
                )?;
            }
            CloudBackupModelEvent::RestoreStarted => {
                Self::enter_background_status(
                    &mut next_state,
                    CloudBackupStatus::Restoring,
                    &mut effects,
                )?;
            }
            CloudBackupModelEvent::RootPromptRefreshed => {
                Self::refresh_root_prompt_effect(&mut next_state, &mut effects);
            }
            CloudBackupModelEvent::ExistingBackupFoundPromptSet { context, passkey_hint } => {
                Self::update_status(&mut next_state, CloudBackupStatus::Enabling, &mut effects);
                Self::update_field(
                    CloudBackupEnableState::Idle,
                    &mut next_state.enable_state,
                    &mut effects.enable_state,
                );
                next_state.root_prompt =
                    CloudBackupRootPrompt::ExistingBackupFound(context, passkey_hint);
            }
            CloudBackupModelEvent::ExistingBackupFoundPromptCleared => {
                if matches!(
                    next_state.root_prompt,
                    CloudBackupRootPrompt::ExistingBackupFound(_, _)
                ) {
                    next_state.root_prompt = CloudBackupRootPrompt::None;
                    Self::update_status(&mut next_state, CloudBackupStatus::Disabled, &mut effects);
                }
            }
            CloudBackupModelEvent::PasskeyChoicePromptSet(intent) => {
                if matches!(intent, CloudBackupPasskeyChoiceIntent::Enable(_, _)) {
                    Self::update_status(&mut next_state, CloudBackupStatus::Enabling, &mut effects);
                    Self::update_field(
                        CloudBackupEnableState::Idle,
                        &mut next_state.enable_state,
                        &mut effects.enable_state,
                    );
                }
                next_state.root_prompt = CloudBackupRootPrompt::PasskeyChoice(intent);
            }
            CloudBackupModelEvent::PasskeyChoicePromptCleared => {
                if let CloudBackupRootPrompt::PasskeyChoice(intent) = &next_state.root_prompt {
                    let clears_enable_prompt =
                        matches!(intent, CloudBackupPasskeyChoiceIntent::Enable(_, _));
                    next_state.root_prompt = CloudBackupRootPrompt::None;
                    if clears_enable_prompt {
                        Self::update_status(
                            &mut next_state,
                            CloudBackupStatus::Disabled,
                            &mut effects,
                        );
                    }
                }
            }
            CloudBackupModelEvent::MissingPasskeyPromptDismissed => {
                next_state.missing_passkey_dismissed = true;
            }
            CloudBackupModelEvent::MissingPasskeyDismissalCleared => {
                next_state.missing_passkey_dismissed = false;
            }
            CloudBackupModelEvent::PromptStateCleared => {
                next_state.root_prompt = CloudBackupRootPrompt::None;
                next_state.missing_passkey_dismissed = false;
            }
            CloudBackupModelEvent::StatusEntered(status) => {
                Self::enter_background_status(&mut next_state, status, &mut effects)?;
            }
            CloudBackupModelEvent::StatusUpdated(status) => {
                Self::update_status(&mut next_state, status, &mut effects);
            }
            CloudBackupModelEvent::ProgressUpdated(progress) => {
                Self::update_field(progress, &mut next_state.progress, &mut effects.progress);
            }
            CloudBackupModelEvent::RestoreProgressUpdated(progress) => {
                if progress.is_some() && !matches!(next_state.status, CloudBackupStatus::Restoring)
                {
                    Self::update_status(
                        &mut next_state,
                        CloudBackupStatus::Restoring,
                        &mut effects,
                    );
                }
                Self::update_field(
                    progress,
                    &mut next_state.restore_progress,
                    &mut effects.restore_progress,
                );
            }
            CloudBackupModelEvent::RestoreReportUpdated(report) => {
                if report.is_some() && !matches!(next_state.status, CloudBackupStatus::Restoring) {
                    Self::update_status(
                        &mut next_state,
                        CloudBackupStatus::Restoring,
                        &mut effects,
                    );
                }
                Self::update_field(
                    report,
                    &mut next_state.restore_report,
                    &mut effects.restore_report,
                );
            }
            CloudBackupModelEvent::SyncHealthUpdated(sync_health) => {
                Self::update_field(
                    sync_health,
                    &mut next_state.sync_health,
                    &mut effects.sync_health,
                );
            }
            CloudBackupModelEvent::EnableStateUpdated(enable_state) => {
                if !matches!(enable_state, CloudBackupEnableState::Idle)
                    && !matches!(next_state.status, CloudBackupStatus::Enabling)
                {
                    Self::update_status(&mut next_state, CloudBackupStatus::Enabling, &mut effects);
                }
                Self::update_field(
                    enable_state,
                    &mut next_state.enable_state,
                    &mut effects.enable_state,
                );
            }
            CloudBackupModelEvent::PendingUploadVerificationUpdated(pending) => {
                Self::update_field(
                    pending,
                    &mut next_state.pending_upload_verification,
                    &mut effects.pending_upload_verification,
                );
            }
            CloudBackupModelEvent::PendingUploadVerificationRefreshed {
                pending,
                metadata,
                should_prompt,
            } => {
                Self::update_field(
                    pending,
                    &mut next_state.pending_upload_verification,
                    &mut effects.pending_upload_verification,
                );
                Self::refresh_verification_flags(
                    &mut next_state,
                    metadata,
                    should_prompt,
                    &mut effects,
                );
                effects.verification_decision_pending =
                    Self::verification_decision_presentation_for_state(&next_state).is_some();
            }
            CloudBackupModelEvent::VerificationFlagsRefreshed { metadata, should_prompt } => {
                Self::refresh_verification_flags(
                    &mut next_state,
                    metadata,
                    should_prompt,
                    &mut effects,
                );
            }
            CloudBackupModelEvent::VerificationPresentationUpdated(presentation) => {
                Self::update_field(
                    presentation,
                    &mut next_state.verification_presentation,
                    &mut effects.verification_presentation,
                );
            }
            CloudBackupModelEvent::VerificationUpdated(verification) => {
                if !matches!(verification, VerificationState::Idle | VerificationState::Cancelled) {
                    Self::update_field(
                        PendingUploadVerificationState::Idle,
                        &mut next_state.pending_upload_verification,
                        &mut effects.pending_upload_verification,
                    );
                }
                Self::update_field(
                    verification,
                    &mut next_state.verification,
                    &mut effects.verification,
                );
            }
            CloudBackupModelEvent::SyncUpdated(sync) => {
                Self::update_field(sync, &mut next_state.sync, &mut effects.sync);
            }
            CloudBackupModelEvent::RecoveryUpdated(recovery) => {
                Self::update_field(recovery, &mut next_state.recovery, &mut effects.recovery);
            }
            CloudBackupModelEvent::DetailUpdated { detail, reset_cloud_only } => {
                Self::update_field(detail, &mut next_state.detail, &mut effects.detail);
                if reset_cloud_only {
                    Self::update_field(
                        CloudOnlyState::NotFetched,
                        &mut next_state.cloud_only,
                        &mut effects.cloud_only,
                    );
                }
            }
            CloudBackupModelEvent::CloudOnlyUpdated(cloud_only) => {
                Self::update_field(cloud_only, &mut next_state.cloud_only, &mut effects.cloud_only);
            }
            CloudBackupModelEvent::CloudOnlyOperationUpdated(cloud_only_operation) => {
                Self::update_field(
                    cloud_only_operation,
                    &mut next_state.cloud_only_operation,
                    &mut effects.cloud_only_operation,
                );
            }
            CloudBackupModelEvent::OtherBackupsOperationUpdated(other_backups_operation) => {
                Self::update_field(
                    other_backups_operation,
                    &mut next_state.other_backups_operation,
                    &mut effects.other_backups_operation,
                );
            }
        }

        self.state = next_state.into();

        let lifecycle = self.state.lifecycle.clone();
        if lifecycle != previous_lifecycle {
            effects.lifecycle = Some(lifecycle);
        }

        Ok(effects)
    }

    fn enter_background_status(
        state: &mut CloudBackupModelSnapshot,
        status: CloudBackupStatus,
        effects: &mut CloudBackupModelEffects,
    ) -> Result<(), CloudBackupModelEventRejection> {
        let current_status = state.status.clone();
        if matches!(current_status, CloudBackupStatus::Enabling | CloudBackupStatus::Restoring) {
            return Err(CloudBackupModelEventRejection::Busy(current_status));
        }

        if state.progress.take().is_some() {
            effects.progress = Some(None);
        }
        if state.restore_progress.take().is_some() {
            effects.restore_progress = Some(None);
        }
        if matches!(status, CloudBackupStatus::Enabling | CloudBackupStatus::Restoring)
            && state.restore_report.take().is_some()
        {
            effects.restore_report = Some(None);
        }

        Self::update_status(state, status, effects);

        Ok(())
    }

    fn update_status(
        state: &mut CloudBackupModelSnapshot,
        status: CloudBackupStatus,
        effects: &mut CloudBackupModelEffects,
    ) {
        if state.status == status {
            return;
        }

        state.status = status.clone();
        effects.status = Some(status);
    }

    fn update_field<T>(value: T, slot: &mut T, effect: &mut Option<T>)
    where
        T: PartialEq + Clone,
    {
        if *slot == value {
            return;
        }

        *slot = value.clone();
        *effect = Some(value);
    }

    fn refresh_verification_flags(
        state: &mut CloudBackupModelSnapshot,
        metadata: CloudBackupVerificationMetadata,
        should_prompt: bool,
        effects: &mut CloudBackupModelEffects,
    ) {
        Self::update_field(
            metadata,
            &mut state.verification_metadata,
            &mut effects.verification_metadata,
        );
        Self::update_field(
            should_prompt,
            &mut state.should_prompt_verification,
            &mut effects.should_prompt_verification,
        );

        let presentation =
            if let Some(presentation) = Self::verification_decision_presentation_for_state(state) {
                presentation
            } else if matches!(
                state.verification_presentation,
                CloudBackupVerificationPresentation::NeedsDecision { .. }
            ) {
                CloudBackupVerificationCoordinator::dismiss_decision(
                    CloudBackupVerificationCoordinator::current_source(
                        &state.verification_presentation,
                    ),
                )
                .presentation
                .expect("dismiss decision effect should include presentation")
            } else {
                state.verification_presentation.clone()
            };

        Self::update_field(
            presentation,
            &mut state.verification_presentation,
            &mut effects.verification_presentation,
        );
    }

    pub(crate) fn verification_decision_presentation_for_state(
        state: &CloudBackupModelSnapshot,
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

        if !matches!(state.verification, VerificationState::Idle | VerificationState::Cancelled) {
            return None;
        }

        CloudBackupVerificationCoordinator::needs_decision(
            CloudBackupVerificationReason::BackupChanged,
            CloudBackupVerificationCoordinator::current_source(&state.verification_presentation),
        )
        .presentation
    }

    fn refresh_root_prompt_effect(
        state: &mut CloudBackupModelSnapshot,
        effects: &mut CloudBackupModelEffects,
    ) {
        let previous_root_prompt = state.root_prompt.clone();
        let root_prompt = Self::resolve_root_prompt(state);
        state.root_prompt = root_prompt.clone();
        if state.root_prompt != previous_root_prompt {
            effects.root_prompt = Some(root_prompt);
        }
    }

    fn resolve_root_prompt(state: &CloudBackupModelSnapshot) -> CloudBackupRootPrompt {
        match &state.root_prompt {
            CloudBackupRootPrompt::ExistingBackupFound(context, passkey_hint)
                if matches!(state.status, CloudBackupStatus::Enabling) =>
            {
                return CloudBackupRootPrompt::ExistingBackupFound(*context, passkey_hint.clone());
            }
            CloudBackupRootPrompt::PasskeyChoice(intent) => {
                return CloudBackupRootPrompt::PasskeyChoice(intent.clone());
            }
            CloudBackupRootPrompt::ExistingBackupFound(_, _)
            | CloudBackupRootPrompt::None
            | CloudBackupRootPrompt::MissingPasskeyReminder
            | CloudBackupRootPrompt::Verification => {}
        }

        if matches!(state.status, CloudBackupStatus::PasskeyMissing)
            && !state.missing_passkey_dismissed
            && !matches!(state.recovery, RecoveryState::Recovering(RecoveryAction::RepairPasskey))
        {
            return CloudBackupRootPrompt::MissingPasskeyReminder;
        }

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

impl CloudBackupModelSnapshot {
    pub(crate) fn public_state(&self) -> super::CloudBackupState {
        super::CloudBackupState { lifecycle: self.lifecycle.clone() }
    }

    pub(crate) fn refresh_domain_projection(&mut self) {
        self.lifecycle = CloudBackupModelState::project_lifecycle_from_snapshot(self);
    }

    pub(crate) fn projected_with_fresh_domain(mut self) -> Self {
        self.refresh_domain_projection();
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manager::cloud_backup_manager::{
        CloudBackupEnableContext, CloudBackupPasskeyHint, CloudBackupProgress,
        CloudBackupRestoreStage, CloudBackupRootPrompt, CloudBackupVerificationMetadata,
        CloudBackupVerificationPresentation, CloudBackupVerificationReason,
        CloudBackupVerificationSource, DeepVerificationReport,
    };

    #[test]
    fn disabled_projects_disabled_lifecycle() {
        let state = CloudBackupModelSnapshot::default().projected_with_fresh_domain();

        assert_eq!(state.lifecycle, CloudBackupLifecycle::Disabled);
    }

    #[test]
    fn enabling_carries_enable_step_and_progress() {
        let state = CloudBackupModelSnapshot {
            status: CloudBackupStatus::Enabling,
            enable_state: CloudBackupEnableState::UploadingBackup,
            progress: Some(CloudBackupProgress { completed: 1, total: 2 }),
            ..CloudBackupModelSnapshot::default()
        }
        .projected_with_fresh_domain();

        assert_eq!(
            state.lifecycle,
            CloudBackupLifecycle::Enabling(CloudBackupEnableFlow::UploadingInitialBackup {
                progress: Some(CloudBackupProgress { completed: 1, total: 2 }),
            }),
        );
    }

    #[test]
    fn enable_started_event_enters_enabling_and_clears_stale_progress() {
        let mut model = CloudBackupModel::from_snapshot(CloudBackupModelSnapshot {
            progress: Some(CloudBackupProgress { completed: 1, total: 2 }),
            restore_progress: Some(CloudBackupRestoreProgress {
                stage: CloudBackupRestoreStage::Downloading,
                completed: 1,
                total: Some(3),
            }),
            ..CloudBackupModelSnapshot::default()
        });

        let effects = model.apply_event(CloudBackupModelEvent::EnableStarted).unwrap();

        assert_eq!(model.status(), CloudBackupStatus::Enabling);
        assert_eq!(model.snapshot().progress, None);
        assert_eq!(model.snapshot().restore_progress, None);
        assert_eq!(effects.status, Some(CloudBackupStatus::Enabling));
        assert_eq!(effects.progress, None);
        assert_eq!(effects.restore_progress, None);
        assert_eq!(
            effects.lifecycle,
            Some(CloudBackupLifecycle::Enabling(CloudBackupEnableFlow::DiscoveringExistingBackup)),
        );
    }

    #[test]
    fn start_events_enter_background_lifecycle_from_top_level_states() {
        let cases = [
            (
                CloudBackupModelSnapshot::default(),
                CloudBackupModelEvent::EnableStarted,
                CloudBackupStatus::Enabling,
                CloudBackupLifecycle::Enabling(CloudBackupEnableFlow::DiscoveringExistingBackup),
            ),
            (
                CloudBackupModelSnapshot {
                    status: CloudBackupStatus::Enabled,
                    ..CloudBackupModelSnapshot::default()
                },
                CloudBackupModelEvent::EnableStarted,
                CloudBackupStatus::Enabling,
                CloudBackupLifecycle::Enabling(CloudBackupEnableFlow::DiscoveringExistingBackup),
            ),
            (
                CloudBackupModelSnapshot {
                    status: CloudBackupStatus::Error("cloud backup failed".into()),
                    ..CloudBackupModelSnapshot::default()
                },
                CloudBackupModelEvent::EnableStarted,
                CloudBackupStatus::Enabling,
                CloudBackupLifecycle::Enabling(CloudBackupEnableFlow::DiscoveringExistingBackup),
            ),
            (
                CloudBackupModelSnapshot::default(),
                CloudBackupModelEvent::RestoreStarted,
                CloudBackupStatus::Restoring,
                CloudBackupLifecycle::Restoring(CloudBackupRestoreFlow {
                    progress: None,
                    report: None,
                }),
            ),
            (
                CloudBackupModelSnapshot {
                    status: CloudBackupStatus::Enabled,
                    ..CloudBackupModelSnapshot::default()
                },
                CloudBackupModelEvent::RestoreStarted,
                CloudBackupStatus::Restoring,
                CloudBackupLifecycle::Restoring(CloudBackupRestoreFlow {
                    progress: None,
                    report: None,
                }),
            ),
            (
                CloudBackupModelSnapshot {
                    status: CloudBackupStatus::Error("cloud backup failed".into()),
                    ..CloudBackupModelSnapshot::default()
                },
                CloudBackupModelEvent::RestoreStarted,
                CloudBackupStatus::Restoring,
                CloudBackupLifecycle::Restoring(CloudBackupRestoreFlow {
                    progress: None,
                    report: None,
                }),
            ),
        ];

        for (snapshot, event, expected_status, expected_lifecycle) in cases {
            let mut model = CloudBackupModel::from_snapshot(snapshot);

            let effects = model.apply_event(event).unwrap();

            assert_eq!(model.status(), expected_status);
            assert_eq!(effects.status, Some(expected_status));
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
            let mut model = CloudBackupModel::from_snapshot(CloudBackupModelSnapshot {
                status: busy_status.clone(),
                ..CloudBackupModelSnapshot::default()
            });

            let result = model.apply_event(event);

            assert_eq!(result, Err(CloudBackupModelEventRejection::Busy(busy_status.clone())),);
            assert_eq!(model.status(), busy_status);
        }
    }

    #[test]
    fn configured_model_events_emit_effects_and_refresh_lifecycle() {
        let mut model = CloudBackupModel::from_snapshot(CloudBackupModelSnapshot {
            status: CloudBackupStatus::Enabled,
            ..CloudBackupModelSnapshot::default()
        });

        let effects = model
            .apply_event(CloudBackupModelEvent::PendingUploadVerificationUpdated(
                PendingUploadVerificationState::BlockedOnAuthorization,
            ))
            .unwrap();

        assert_eq!(
            effects.pending_upload_verification,
            Some(PendingUploadVerificationState::BlockedOnAuthorization),
        );
        assert_eq!(
            effects.lifecycle,
            Some(CloudBackupLifecycle::Configured(CloudBackupConfiguredState {
                passkey: CloudBackupPasskeyState::Available,
                verification: CloudBackupVerificationState::AwaitingUploadConfirmation,
                sync: CloudBackupSyncState::Blocked("cloud authorization required".into()),
                detail: CloudBackupDetailState::NotLoaded,
                last_restore_report: None,
                root_prompt: CloudBackupRootPrompt::None,
                sync_health: CloudSyncHealth::Unknown,
                verification_presentation: CloudBackupVerificationPresentation::Hidden {
                    source: None,
                },
            })),
        );
    }

    #[test]
    fn status_updated_event_can_leave_background_status() {
        let mut model = CloudBackupModel::from_snapshot(CloudBackupModelSnapshot {
            status: CloudBackupStatus::Enabling,
            ..CloudBackupModelSnapshot::default()
        });

        let effects = model
            .apply_event(CloudBackupModelEvent::StatusUpdated(CloudBackupStatus::Enabled))
            .unwrap();

        assert_eq!(effects.status, Some(CloudBackupStatus::Enabled));
        assert!(matches!(effects.lifecycle, Some(CloudBackupLifecycle::Configured(_)),));
    }

    #[test]
    fn no_op_model_event_emits_no_effects() {
        let mut model = CloudBackupModel::from_snapshot(CloudBackupModelSnapshot {
            status: CloudBackupStatus::Enabled,
            verification: VerificationState::PasskeyConfirmed,
            ..CloudBackupModelSnapshot::default()
        });

        let effects = model
            .apply_event(CloudBackupModelEvent::VerificationUpdated(
                VerificationState::PasskeyConfirmed,
            ))
            .unwrap();

        assert_eq!(effects, CloudBackupModelEffects::default());
    }

    #[test]
    fn restoring_carries_restore_progress() {
        let progress = CloudBackupRestoreProgress {
            stage: CloudBackupRestoreStage::Downloading,
            completed: 1,
            total: Some(3),
        };
        let state = CloudBackupModelSnapshot {
            status: CloudBackupStatus::Restoring,
            restore_progress: Some(progress.clone()),
            ..CloudBackupModelSnapshot::default()
        }
        .projected_with_fresh_domain();

        assert_eq!(
            state.lifecycle,
            CloudBackupLifecycle::Restoring(CloudBackupRestoreFlow {
                progress: Some(progress),
                report: None,
            }),
        );
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
        let state = CloudBackupModelSnapshot {
            status: CloudBackupStatus::Enabled,
            verification: VerificationState::Verified(report.clone()),
            sync: SyncState::Syncing,
            pending_upload_verification: PendingUploadVerificationState::Confirming,
            ..CloudBackupModelSnapshot::default()
        }
        .projected_with_fresh_domain();

        let CloudBackupLifecycle::Configured(state) = state.lifecycle else {
            panic!("enabled backup should project configured lifecycle");
        };

        assert_eq!(state.passkey, CloudBackupPasskeyState::Available);
        assert_eq!(state.verification, CloudBackupVerificationState::AwaitingUploadConfirmation);
        assert_eq!(state.sync, CloudBackupSyncState::Syncing);
    }

    #[test]
    fn passkey_missing_projects_missing_or_repairing() {
        let missing = CloudBackupModelSnapshot {
            status: CloudBackupStatus::PasskeyMissing,
            ..CloudBackupModelSnapshot::default()
        }
        .projected_with_fresh_domain();

        let CloudBackupLifecycle::Configured(state) = missing.lifecycle else {
            panic!("passkey-missing backup should still be configured");
        };
        assert_eq!(
            state.passkey,
            CloudBackupPasskeyState::NeedsRepair { state: CloudBackupPasskeyRepairState::Idle }
        );

        let repairing = CloudBackupModelSnapshot {
            status: CloudBackupStatus::PasskeyMissing,
            recovery: RecoveryState::Recovering(RecoveryAction::RepairPasskey),
            ..CloudBackupModelSnapshot::default()
        }
        .projected_with_fresh_domain();

        let CloudBackupLifecycle::Configured(state) = repairing.lifecycle else {
            panic!("repairing backup should still be configured");
        };
        assert_eq!(
            state.passkey,
            CloudBackupPasskeyState::NeedsRepair { state: CloudBackupPasskeyRepairState::Running }
        );
    }

    #[test]
    fn verification_flags_event_opens_decision_prompt() {
        let mut model = CloudBackupModel::from_snapshot(CloudBackupModelSnapshot {
            status: CloudBackupStatus::Enabled,
            verification_presentation: CloudBackupVerificationPresentation::Hidden {
                source: Some(CloudBackupVerificationSource::Settings),
            },
            ..CloudBackupModelSnapshot::default()
        });

        let effects = model
            .apply_event(CloudBackupModelEvent::VerificationFlagsRefreshed {
                metadata: CloudBackupVerificationMetadata::NeedsVerification,
                should_prompt: true,
            })
            .unwrap();

        assert_eq!(
            effects.verification_metadata,
            Some(CloudBackupVerificationMetadata::NeedsVerification),
        );
        assert_eq!(effects.should_prompt_verification, Some(true));
        assert_eq!(
            effects.verification_presentation,
            Some(CloudBackupVerificationPresentation::NeedsDecision {
                reason: CloudBackupVerificationReason::BackupChanged,
                source: CloudBackupVerificationSource::Settings,
            }),
        );
        assert!(matches!(effects.lifecycle, Some(CloudBackupLifecycle::Configured(_)),));
    }

    #[test]
    fn verification_flags_event_dismisses_stale_decision_prompt() {
        let mut model = CloudBackupModel::from_snapshot(CloudBackupModelSnapshot {
            status: CloudBackupStatus::Enabled,
            verification_metadata: CloudBackupVerificationMetadata::NeedsVerification,
            should_prompt_verification: true,
            verification_presentation: CloudBackupVerificationPresentation::NeedsDecision {
                reason: CloudBackupVerificationReason::BackupChanged,
                source: CloudBackupVerificationSource::RootPrompt,
            },
            ..CloudBackupModelSnapshot::default()
        });

        let effects = model
            .apply_event(CloudBackupModelEvent::VerificationFlagsRefreshed {
                metadata: CloudBackupVerificationMetadata::Verified(42),
                should_prompt: false,
            })
            .unwrap();

        assert_eq!(
            effects.verification_presentation,
            Some(CloudBackupVerificationPresentation::Hidden {
                source: Some(CloudBackupVerificationSource::RootPrompt),
            }),
        );
    }

    #[test]
    fn pending_upload_refresh_tracks_decision_pending_without_duplicate_presentation() {
        let mut model = CloudBackupModel::from_snapshot(CloudBackupModelSnapshot {
            status: CloudBackupStatus::Enabled,
            verification_metadata: CloudBackupVerificationMetadata::NeedsVerification,
            should_prompt_verification: true,
            verification_presentation: CloudBackupVerificationPresentation::NeedsDecision {
                reason: CloudBackupVerificationReason::BackupChanged,
                source: CloudBackupVerificationSource::RootPrompt,
            },
            ..CloudBackupModelSnapshot::default()
        });

        let effects = model
            .apply_event(CloudBackupModelEvent::PendingUploadVerificationRefreshed {
                pending: PendingUploadVerificationState::Idle,
                metadata: CloudBackupVerificationMetadata::NeedsVerification,
                should_prompt: true,
            })
            .unwrap();

        assert!(effects.verification_decision_pending);
        assert_eq!(effects.verification_presentation, None);
    }

    #[test]
    fn root_prompt_projects_root_prompt() {
        let hint = CloudBackupPasskeyHint {
            provider_name: Some("iCloud Keychain".into()),
            name_suffix: "abc123".into(),
            registered_at: 1,
        };
        let state = CloudBackupModelSnapshot {
            status: CloudBackupStatus::Enabling,
            root_prompt: CloudBackupRootPrompt::ExistingBackupFound(
                CloudBackupEnableContext::settings_manual(),
                Some(hint.clone()),
            ),
            ..CloudBackupModelSnapshot::default()
        }
        .projected_with_fresh_domain();

        assert_eq!(
            state.root_prompt,
            CloudBackupRootPrompt::ExistingBackupFound(
                CloudBackupEnableContext::settings_manual(),
                Some(hint),
            ),
        );
        assert_eq!(
            state.lifecycle,
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
    fn root_prompt_refresh_event_emits_prompt_effect() {
        let mut model = CloudBackupModel::from_snapshot(CloudBackupModelSnapshot {
            status: CloudBackupStatus::PasskeyMissing,
            ..CloudBackupModelSnapshot::default()
        });

        let effects = model.apply_event(CloudBackupModelEvent::RootPromptRefreshed).unwrap();

        assert_eq!(effects.root_prompt, Some(CloudBackupRootPrompt::MissingPasskeyReminder),);
    }
}
