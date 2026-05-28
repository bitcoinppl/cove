//! Cloud Backup private reducer and public UI state projection
//!
//! The reducer keeps impossible intermediate states out of the UniFFI model and
//! projects a smaller public state for Swift and Kotlin. Exclusive operation
//! claims are tracked here so stale async completions cannot clear newer work

use super::{
    CloudBackupDetail, CloudBackupDisableOutcome, CloudBackupEnableContext,
    CloudBackupEnablePromptChoice, CloudBackupEnableState, CloudBackupPasskeyChoiceIntent,
    CloudBackupPasskeyHint, CloudBackupRootPrompt, CloudBackupStatus,
    CloudBackupVerificationMetadata, CloudBackupVerificationPresentation,
    CloudBackupVerificationReason, CloudOnlyOperation, CloudOnlyState, DeepVerificationFailure,
    DeepVerificationReport, OtherBackupsOperation, PendingUploadVerificationState, RecoveryAction,
    RecoveryState, SyncState, VerificationState,
};

use super::verify::coordinator::CloudBackupVerificationCoordinator;
use cove_device::cloud_storage::CloudSyncHealth;

const PENDING_UPLOAD_AUTHORIZATION_BLOCKED_MESSAGE: &str = "cloud authorization required";

/// Private reducer wrapper that projects public Cloud Backup state
#[derive(Debug, Clone, Default)]
pub(crate) struct CloudBackupStateReducer {
    state: CloudBackupReducerState,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CloudBackupReducerState {
    phase: CloudBackupLifecyclePhase,
    configured: CloudBackupConfiguredReducerState,
    active_operation: Option<CloudBackupExclusiveOperationClaim>,
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
struct CloudBackupConfiguredReducerState {
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CloudBackupAcceptedEnablePrompt {
    Enable(CloudBackupEnableContext),
    ForceNew(CloudBackupEnableContext),
    NoDiscovery(CloudBackupEnableContext),
}

/// Exclusive operation category where newer claims replace active older claims
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CloudBackupExclusiveOperation {
    Enable,
    EnableForceNew,
    EnableNoDiscovery,
    Restore,
    Disable,
    RecreateManifest,
    ReinitializeBackup,
    RepairPasskey,
    VerificationRepair,
    RecoverOtherBackups,
    DeleteOtherBackups,
    RestoreCloudWallet,
    DeleteCloudWallet,
}

/// Generation-tagged ownership proof for an exclusive operation
///
/// Async completions must present the claim they started with before the
/// reducer or supervisor accepts their result
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct CloudBackupExclusiveOperationClaim {
    operation: CloudBackupExclusiveOperation,
    generation: u64,
}

impl CloudBackupExclusiveOperationClaim {
    pub(crate) fn new(operation: CloudBackupExclusiveOperation, generation: u64) -> Self {
        Self { operation, generation }
    }

    pub(crate) fn operation(self) -> CloudBackupExclusiveOperation {
        self.operation
    }
}

impl Default for CloudBackupReducerState {
    fn default() -> Self {
        Self {
            phase: CloudBackupLifecyclePhase::Disabled,
            configured: CloudBackupConfiguredReducerState::default(),
            active_operation: None,
            sync_health: CloudSyncHealth::Unknown,
            missing_passkey_dismissed: false,
            should_prompt_verification: false,
            verification_metadata: CloudBackupVerificationMetadata::NotConfigured,
            verification_presentation: CloudBackupVerificationPresentation::Hidden { source: None },
        }
    }
}

impl Default for CloudBackupConfiguredReducerState {
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

impl CloudBackupReducerState {
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
            destructive_operation: self.public_destructive_operation(),
            detail: self.configured.detail.clone(),
            root_prompt: self.root_prompt(),
            sync_health: self.sync_health.clone(),
            verification_presentation: self.verification_presentation.clone(),
        }
    }

    fn public_destructive_operation(&self) -> CloudBackupDestructiveOperationState {
        match self.active_operation.map(CloudBackupExclusiveOperationClaim::operation) {
            Some(CloudBackupExclusiveOperation::Disable) => {
                CloudBackupDestructiveOperationState::Disabling
            }
            Some(CloudBackupExclusiveOperation::RecreateManifest) => {
                CloudBackupDestructiveOperationState::RecreatingManifest
            }
            Some(CloudBackupExclusiveOperation::ReinitializeBackup) => {
                CloudBackupDestructiveOperationState::ReinitializingBackup
            }
            _ => self.configured.destructive_operation.clone(),
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

    fn accept_enable_prompt(
        &mut self,
        choice: CloudBackupEnablePromptChoice,
    ) -> Option<CloudBackupAcceptedEnablePrompt> {
        let accepted = match (&self.phase, choice) {
            (
                CloudBackupLifecyclePhase::Enabling(
                    CloudBackupEnableFlow::AwaitingForceNewConfirmation(context, _),
                ),
                CloudBackupEnablePromptChoice::UseExisting,
            ) => Some(CloudBackupAcceptedEnablePrompt::Enable(*context)),
            (
                CloudBackupLifecyclePhase::Enabling(
                    CloudBackupEnableFlow::AwaitingForceNewConfirmation(context, _),
                ),
                CloudBackupEnablePromptChoice::CreateNew,
            ) => Some(CloudBackupAcceptedEnablePrompt::ForceNew(*context)),
            (
                CloudBackupLifecyclePhase::Enabling(CloudBackupEnableFlow::AwaitingPasskeyChoice(
                    CloudBackupPasskeyChoiceIntent::Enable(context, _),
                )),
                CloudBackupEnablePromptChoice::UseExisting,
            ) => Some(CloudBackupAcceptedEnablePrompt::Enable(*context)),
            (
                CloudBackupLifecyclePhase::Enabling(CloudBackupEnableFlow::AwaitingPasskeyChoice(
                    CloudBackupPasskeyChoiceIntent::Enable(context, _),
                )),
                CloudBackupEnablePromptChoice::CreateNew,
            ) => Some(CloudBackupAcceptedEnablePrompt::NoDiscovery(*context)),
            _ => None,
        };

        if accepted.is_some() {
            // accept_enable_prompt deliberately routes every accepted choice through
            // the CloudBackupLifecyclePhase::Enabling state with
            // the CloudBackupEnableFlow::DiscoveringExistingBackup flow
            // and CloudBackupConfiguredPrompt::None so discovery/preparation can choose
            // the use-existing, force-new, or no-discovery follow-up
            self.phase = CloudBackupLifecyclePhase::Enabling(
                CloudBackupEnableFlow::DiscoveringExistingBackup,
            );
            self.configured.prompt = CloudBackupConfiguredPrompt::None;
        }

        accepted
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
        match self.active_operation.map(CloudBackupExclusiveOperationClaim::operation) {
            Some(
                CloudBackupExclusiveOperation::Enable
                | CloudBackupExclusiveOperation::EnableForceNew
                | CloudBackupExclusiveOperation::EnableNoDiscovery
                | CloudBackupExclusiveOperation::ReinitializeBackup,
            ) => return CloudBackupStatus::Enabling,
            Some(CloudBackupExclusiveOperation::Restore) => return CloudBackupStatus::Restoring,
            Some(CloudBackupExclusiveOperation::Disable) => return CloudBackupStatus::Disabling,
            Some(
                CloudBackupExclusiveOperation::RecreateManifest
                | CloudBackupExclusiveOperation::RepairPasskey
                | CloudBackupExclusiveOperation::VerificationRepair
                | CloudBackupExclusiveOperation::RecoverOtherBackups
                | CloudBackupExclusiveOperation::DeleteOtherBackups
                | CloudBackupExclusiveOperation::RestoreCloudWallet
                | CloudBackupExclusiveOperation::DeleteCloudWallet,
            )
            | None => {}
        }

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

    fn active_operation(&self) -> Option<CloudBackupExclusiveOperationClaim> {
        self.active_operation
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

    fn project_exclusive_operation_start(&mut self, claim: CloudBackupExclusiveOperationClaim) {
        self.active_operation = Some(claim);
        self.apply_exclusive_operation_start(claim.operation());
    }

    fn finish_exclusive_operation(&mut self, claim: CloudBackupExclusiveOperationClaim) {
        if self.active_operation != Some(claim) {
            return;
        }

        self.active_operation = None;
    }

    fn apply_exclusive_operation_start(&mut self, operation: CloudBackupExclusiveOperation) {
        match operation {
            CloudBackupExclusiveOperation::Enable
            | CloudBackupExclusiveOperation::EnableForceNew
            | CloudBackupExclusiveOperation::EnableNoDiscovery => {
                self.apply_status(CloudBackupStatus::Enabling);
            }
            CloudBackupExclusiveOperation::Restore => {
                self.apply_status(CloudBackupStatus::Restoring);
            }
            CloudBackupExclusiveOperation::Disable => {
                self.resolve_disable(CloudBackupDisableOutcome::Started);
            }
            CloudBackupExclusiveOperation::RecreateManifest => {
                self.resolve_recovery(RecoveryState::Recovering(RecoveryAction::RecreateManifest));
            }
            CloudBackupExclusiveOperation::ReinitializeBackup => {
                self.resolve_recovery(RecoveryState::Recovering(
                    RecoveryAction::ReinitializeBackup,
                ));
            }
            CloudBackupExclusiveOperation::RepairPasskey => {
                self.resolve_recovery(RecoveryState::Recovering(RecoveryAction::RepairPasskey));
            }
            CloudBackupExclusiveOperation::VerificationRepair => {}
            CloudBackupExclusiveOperation::RecoverOtherBackups
            | CloudBackupExclusiveOperation::DeleteOtherBackups
            | CloudBackupExclusiveOperation::RestoreCloudWallet
            | CloudBackupExclusiveOperation::DeleteCloudWallet => {}
        }
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
            CloudBackupEnableState::WaitingForPasskeyAvailability => {
                CloudBackupEnableFlow::WaitingForPasskeyAvailability
            }
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
        state: &CloudBackupReducerState,
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

/// Event accepted by the private reducer
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum CloudBackupStateReducerEvent {
    ExclusiveOperationStarted(CloudBackupExclusiveOperationClaim),
    ExclusiveOperationFinished(CloudBackupExclusiveOperationClaim),
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

/// Intentionally uninhabited marker because reducer events are currently total
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum CloudBackupStateReducerEventRejection {}

/// Side effects the manager should emit after applying a reducer event
#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct CloudBackupStateReducerEffects {
    pub(crate) lifecycle: Option<CloudBackupLifecycle>,
    pub(crate) status_changed: bool,
    pub(crate) verification_presentation_changed: bool,
    pub(crate) verification_decision_pending: bool,
}

/// Public passkey health state for the configured backup
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum CloudBackupPasskeyState {
    Available,
    Missing,
    UnsupportedProvider,
    NeedsRepair { state: CloudBackupPasskeyRepairState },
}

/// Public repair status for a missing or stale backup passkey
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum CloudBackupPasskeyRepairState {
    Idle,
    Running,
    Failed(String),
}

/// Public backup verification state shown by settings and prompts
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum CloudBackupVerificationState {
    NotVerified,
    Verified { report: Option<DeepVerificationReport>, last_verified_at: Option<u64> },
    Required,
    Running,
    AwaitingUploadConfirmation,
    Failed(DeepVerificationFailure),
}

/// Public sync status for background cloud backup work
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum CloudBackupSyncState {
    Idle,
    Syncing,
    Blocked(String),
    Failed(String),
}

/// Public status for destructive operations that can affect remote backup data
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum CloudBackupDestructiveOperationState {
    Idle,
    RecreatingManifest,
    ReinitializingBackup,
    Disabling,
    DisableFailed { message: String, can_keep_enabled: bool },
}

/// Detail payload shown after remote backup detail has loaded
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct LoadedCloudBackupDetail {
    pub detail: CloudBackupDetail,
    pub cloud_only: CloudOnlyState,
    pub cloud_only_operation: CloudOnlyOperation,
    pub other_backups_operation: OtherBackupsOperation,
}

/// Public loading state for the cloud backup detail screen
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum CloudBackupDetailState {
    NotLoaded,
    Loading,
    Loaded { state: LoadedCloudBackupDetail },
    Failed(String),
}

/// Public configured-backup state projected from the private reducer
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

/// Public enable flow state for onboarding and settings
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

/// Public restore progress state
#[derive(Debug, Clone, Hash, PartialEq, Eq, uniffi::Enum)]
pub enum CloudBackupRestoreFlow {
    Finding,
    Downloading { completed: u32, total: u32 },
    Restoring { completed: u32, total: u32 },
}

/// Public terminal cloud backup failure
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct CloudBackupFailure {
    pub message: String,
}

/// Public top-level cloud backup lifecycle
#[expect(clippy::large_enum_variant, reason = "exported UniFFI enum keeps payloads inline")]
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum CloudBackupLifecycle {
    Disabled,
    Enabling(CloudBackupEnableFlow),
    Restoring(CloudBackupRestoreFlow),
    Configured(CloudBackupConfiguredState),
    Failed(CloudBackupFailure),
}

impl CloudBackupStateReducer {
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

    pub(crate) fn active_operation(&self) -> Option<CloudBackupExclusiveOperationClaim> {
        self.state.active_operation()
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
        event: CloudBackupStateReducerEvent,
    ) -> Result<CloudBackupStateReducerEffects, CloudBackupStateReducerEventRejection> {
        let previous_status = self.state.status();
        let previous_lifecycle = self.state.public_lifecycle();
        let previous_presentation = self.state.verification_presentation.clone();
        let mut effects = CloudBackupStateReducerEffects::default();

        match event {
            CloudBackupStateReducerEvent::ExclusiveOperationStarted(claim) => {
                self.state.project_exclusive_operation_start(claim);
            }
            CloudBackupStateReducerEvent::ExclusiveOperationFinished(claim) => {
                self.state.finish_exclusive_operation(claim);
            }
            CloudBackupStateReducerEvent::RuntimeStatusReconciled(status) => {
                self.state.apply_status(status);
            }
            CloudBackupStateReducerEvent::ExistingBackupFoundPromptSet {
                context,
                passkey_hint,
            } => {
                self.state.phase = CloudBackupLifecyclePhase::Enabling(
                    CloudBackupEnableFlow::AwaitingForceNewConfirmation(context, passkey_hint),
                );
            }
            CloudBackupStateReducerEvent::ExistingBackupFoundPromptCleared => {
                if matches!(
                    self.state.phase,
                    CloudBackupLifecyclePhase::Enabling(
                        CloudBackupEnableFlow::AwaitingForceNewConfirmation(_, _)
                    )
                ) {
                    self.state.phase = CloudBackupLifecyclePhase::Disabled;
                }
            }
            CloudBackupStateReducerEvent::PasskeyChoicePromptSet(intent) => match &intent {
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
            CloudBackupStateReducerEvent::PasskeyChoicePromptCleared => {
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
            CloudBackupStateReducerEvent::MissingPasskeyPromptDismissed => {
                self.state.missing_passkey_dismissed = true;
            }
            CloudBackupStateReducerEvent::MissingPasskeyDismissalCleared => {
                self.state.missing_passkey_dismissed = false;
            }
            CloudBackupStateReducerEvent::PromptStateCleared => {
                self.state.clear_prompt_state();
            }
            CloudBackupStateReducerEvent::EnableProgressReported(progress) => {
                self.state.report_enable_progress(progress);
            }
            CloudBackupStateReducerEvent::RestoreProgressReported(progress) => {
                self.state.report_restore_progress(progress);
            }
            CloudBackupStateReducerEvent::SyncHealthObserved(sync_health) => {
                self.state.sync_health = sync_health;
            }
            CloudBackupStateReducerEvent::EnableFlowAdvanced(enable_state) => {
                self.state.apply_enable_flow(enable_state);
            }
            CloudBackupStateReducerEvent::PendingUploadVerificationReconciled(pending) => {
                self.state.reconcile_pending_upload_verification(pending);
            }
            CloudBackupStateReducerEvent::PendingUploadVerificationAndFlagsReconciled {
                pending,
                metadata,
                should_prompt,
            } => {
                self.state.reconcile_pending_upload_verification(pending);
                self.state.reconcile_verification_flags(metadata, should_prompt);
                effects.verification_decision_pending =
                    CloudBackupReducerState::verification_decision_presentation_for_state(
                        &self.state,
                    )
                    .is_some();
            }
            CloudBackupStateReducerEvent::VerificationFlagsReconciled {
                metadata,
                should_prompt,
            } => {
                self.state.reconcile_verification_flags(metadata, should_prompt);
            }
            CloudBackupStateReducerEvent::VerificationPresentationReconciled(presentation) => {
                self.state.verification_presentation = presentation;
            }
            CloudBackupStateReducerEvent::VerificationStateResolved(verification) => {
                if !matches!(verification, VerificationState::Idle | VerificationState::Cancelled) {
                    self.state.reconcile_pending_upload_verification(
                        PendingUploadVerificationState::Idle,
                    );
                }
                self.state.resolve_verification(verification);
            }
            CloudBackupStateReducerEvent::SyncStateResolved(sync) => {
                self.state.resolve_sync(sync);
            }
            CloudBackupStateReducerEvent::RecoveryStateResolved(recovery) => {
                self.state.resolve_recovery(recovery);
            }
            CloudBackupStateReducerEvent::DisableStateResolved(outcome) => {
                self.state.resolve_disable(outcome);
            }
            CloudBackupStateReducerEvent::DetailRefreshApplied { detail, reset_cloud_only } => {
                self.state.apply_detail_refresh(detail, reset_cloud_only);
            }
            CloudBackupStateReducerEvent::CloudOnlyStateResolved(cloud_only) => {
                self.state.resolve_cloud_only_state(cloud_only);
            }
            CloudBackupStateReducerEvent::CloudOnlyOperationResolved(cloud_only_operation) => {
                self.state.resolve_cloud_only_operation(cloud_only_operation);
            }
            CloudBackupStateReducerEvent::OtherBackupsOperationResolved(
                other_backups_operation,
            ) => {
                self.state.resolve_other_backups_operation(other_backups_operation);
            }
        }

        self.resolve_effects(
            previous_status,
            previous_lifecycle,
            previous_presentation,
            &mut effects,
        );

        Ok(effects)
    }

    pub(crate) fn accept_enable_prompt(
        &mut self,
        choice: CloudBackupEnablePromptChoice,
    ) -> (Option<CloudBackupAcceptedEnablePrompt>, CloudBackupStateReducerEffects) {
        let previous_status = self.state.status();
        let previous_lifecycle = self.state.public_lifecycle();
        let previous_presentation = self.state.verification_presentation.clone();
        let mut effects = CloudBackupStateReducerEffects::default();
        let accepted = self.state.accept_enable_prompt(choice);

        self.resolve_effects(
            previous_status,
            previous_lifecycle,
            previous_presentation,
            &mut effects,
        );

        (accepted, effects)
    }

    fn resolve_effects(
        &self,
        previous_status: CloudBackupStatus,
        previous_lifecycle: CloudBackupLifecycle,
        previous_presentation: CloudBackupVerificationPresentation,
        effects: &mut CloudBackupStateReducerEffects,
    ) {
        let lifecycle = self.state.public_lifecycle();
        if lifecycle != previous_lifecycle {
            effects.lifecycle = Some(lifecycle);
        }
        effects.status_changed = self.state.status() != previous_status;
        effects.verification_presentation_changed =
            self.state.verification_presentation != previous_presentation;
    }
}

#[cfg(test)]
pub(crate) mod test_support {
    use super::*;
    use crate::manager::cloud_backup_manager::test_support::CloudBackupModelSnapshot;

    impl CloudBackupStateReducer {
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

    fn restore_progress(state: &CloudBackupReducerState) -> Option<CloudBackupRestoreFlow> {
        match &state.phase {
            CloudBackupLifecyclePhase::Restoring(flow) => Some(flow.clone()),
            _ => None,
        }
    }

    fn enable_state(state: &CloudBackupReducerState) -> CloudBackupEnableState {
        let CloudBackupLifecyclePhase::Enabling(flow) = &state.phase else {
            return CloudBackupEnableState::Idle;
        };

        match flow {
            CloudBackupEnableFlow::CreatingPasskey => CloudBackupEnableState::CreatingPasskey,
            CloudBackupEnableFlow::WaitingForPasskeyAvailability => {
                CloudBackupEnableState::WaitingForPasskeyAvailability
            }
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
            | CloudBackupEnableFlow::AwaitingPasskeyChoice(_) => CloudBackupEnableState::Idle,
        }
    }

    fn pending_upload_verification(
        state: &CloudBackupReducerState,
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

    fn operation_event(
        operation: CloudBackupExclusiveOperation,
        generation: u64,
    ) -> CloudBackupStateReducerEvent {
        CloudBackupStateReducerEvent::ExclusiveOperationStarted(
            CloudBackupExclusiveOperationClaim::new(operation, generation),
        )
    }

    #[test]
    fn disabled_projects_disabled_lifecycle() {
        let model = CloudBackupStateReducer::default();

        assert_eq!(model.public_state().lifecycle, CloudBackupLifecycle::Disabled);
    }

    #[test]
    fn enabling_carries_enable_step_and_progress() {
        let mut model = CloudBackupStateReducer::default();

        model
            .apply_event(CloudBackupStateReducerEvent::EnableFlowAdvanced(
                CloudBackupEnableState::UploadingBackup,
            ))
            .unwrap();
        model
            .apply_event(CloudBackupStateReducerEvent::EnableProgressReported(Some(
                CloudBackupProgress { completed: 1, total: 2 },
            )))
            .unwrap();

        assert_eq!(
            model.public_state().lifecycle,
            CloudBackupLifecycle::Enabling(CloudBackupEnableFlow::UploadingInitialBackup {
                progress: Some(CloudBackupProgress { completed: 1, total: 2 }),
            }),
        );
    }

    #[test]
    fn enable_operation_enters_enabling_and_clears_restore_progress() {
        let mut model = CloudBackupStateReducer::default();
        model.apply_event(operation_event(CloudBackupExclusiveOperation::Restore, 1)).unwrap();
        model
            .apply_event(CloudBackupStateReducerEvent::RuntimeStatusReconciled(
                CloudBackupStatus::Enabled,
            ))
            .unwrap();
        model
            .apply_event(CloudBackupStateReducerEvent::ExclusiveOperationFinished(
                CloudBackupExclusiveOperationClaim::new(CloudBackupExclusiveOperation::Restore, 1),
            ))
            .unwrap();

        let effects =
            model.apply_event(operation_event(CloudBackupExclusiveOperation::Enable, 2)).unwrap();

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
    fn exclusive_operations_enter_background_lifecycle_from_top_level_states() {
        let cases = [
            (
                CloudBackupStatus::Disabled,
                CloudBackupExclusiveOperation::Enable,
                CloudBackupStatus::Enabling,
                CloudBackupLifecycle::Enabling(CloudBackupEnableFlow::DiscoveringExistingBackup),
            ),
            (
                CloudBackupStatus::Enabled,
                CloudBackupExclusiveOperation::Enable,
                CloudBackupStatus::Enabling,
                CloudBackupLifecycle::Enabling(CloudBackupEnableFlow::DiscoveringExistingBackup),
            ),
            (
                CloudBackupStatus::Error("cloud backup failed".into()),
                CloudBackupExclusiveOperation::Enable,
                CloudBackupStatus::Enabling,
                CloudBackupLifecycle::Enabling(CloudBackupEnableFlow::DiscoveringExistingBackup),
            ),
            (
                CloudBackupStatus::Disabled,
                CloudBackupExclusiveOperation::Restore,
                CloudBackupStatus::Restoring,
                CloudBackupLifecycle::Restoring(CloudBackupRestoreFlow::Finding),
            ),
            (
                CloudBackupStatus::Enabled,
                CloudBackupExclusiveOperation::Restore,
                CloudBackupStatus::Restoring,
                CloudBackupLifecycle::Restoring(CloudBackupRestoreFlow::Finding),
            ),
            (
                CloudBackupStatus::Error("cloud backup failed".into()),
                CloudBackupExclusiveOperation::Restore,
                CloudBackupStatus::Restoring,
                CloudBackupLifecycle::Restoring(CloudBackupRestoreFlow::Finding),
            ),
        ];

        for (index, (initial_status, operation, expected_status, expected_lifecycle)) in
            cases.into_iter().enumerate()
        {
            let mut model = CloudBackupStateReducer::default();
            model
                .apply_event(CloudBackupStateReducerEvent::RuntimeStatusReconciled(initial_status))
                .unwrap();

            let effects = model.apply_event(operation_event(operation, index as u64)).unwrap();

            assert_eq!(model.status(), expected_status);
            assert!(effects.status_changed);
            assert_eq!(effects.lifecycle, Some(expected_lifecycle));
        }
    }

    #[test]
    fn exclusive_operation_start_projects_latest_operation_without_locking() {
        let active_claim = CloudBackupExclusiveOperationClaim::new(
            CloudBackupExclusiveOperation::RecreateManifest,
            1,
        );
        let cases = [
            CloudBackupExclusiveOperation::Enable,
            CloudBackupExclusiveOperation::Restore,
            CloudBackupExclusiveOperation::Disable,
        ];

        for (index, operation) in cases.into_iter().enumerate() {
            let mut model = CloudBackupStateReducer::default();
            model
                .apply_event(CloudBackupStateReducerEvent::ExclusiveOperationStarted(active_claim))
                .unwrap();
            let next_claim = CloudBackupExclusiveOperationClaim::new(operation, index as u64 + 2);

            let result = model
                .apply_event(CloudBackupStateReducerEvent::ExclusiveOperationStarted(next_claim));

            assert!(result.is_ok());
            assert_eq!(model.active_operation(), Some(next_claim));
        }
    }

    #[test]
    fn runtime_disabled_reconcile_finishes_disabling_lifecycle_view() {
        let mut model = CloudBackupStateReducer::default();
        model.apply_event(operation_event(CloudBackupExclusiveOperation::Disable, 1)).unwrap();

        let effects = model
            .apply_event(CloudBackupStateReducerEvent::RuntimeStatusReconciled(
                CloudBackupStatus::Disabled,
            ))
            .unwrap();

        assert_eq!(model.public_state().lifecycle, CloudBackupLifecycle::Disabled);
        assert_eq!(effects.lifecycle, Some(CloudBackupLifecycle::Disabled));
    }

    #[test]
    fn stale_exclusive_operation_finish_does_not_clear_newer_operation() {
        let mut model = CloudBackupStateReducer::default();
        let stale_claim =
            CloudBackupExclusiveOperationClaim::new(CloudBackupExclusiveOperation::Disable, 1);
        let current_claim =
            CloudBackupExclusiveOperationClaim::new(CloudBackupExclusiveOperation::Disable, 2);

        model
            .apply_event(CloudBackupStateReducerEvent::ExclusiveOperationStarted(stale_claim))
            .unwrap();
        model
            .apply_event(CloudBackupStateReducerEvent::ExclusiveOperationFinished(stale_claim))
            .unwrap();
        model
            .apply_event(CloudBackupStateReducerEvent::ExclusiveOperationStarted(current_claim))
            .unwrap();
        model
            .apply_event(CloudBackupStateReducerEvent::ExclusiveOperationFinished(stale_claim))
            .unwrap();

        assert_eq!(model.active_operation(), Some(current_claim));
        assert_eq!(model.status(), CloudBackupStatus::Disabling);
    }

    #[test]
    fn configured_model_events_emit_effects_and_refresh_lifecycle() {
        let mut model = CloudBackupStateReducer::default();
        model
            .apply_event(CloudBackupStateReducerEvent::RuntimeStatusReconciled(
                CloudBackupStatus::Enabled,
            ))
            .unwrap();

        let effects = model
            .apply_event(CloudBackupStateReducerEvent::PendingUploadVerificationReconciled(
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
        let mut model = CloudBackupStateReducer::default();
        model
            .apply_event(CloudBackupStateReducerEvent::RuntimeStatusReconciled(
                CloudBackupStatus::Enabled,
            ))
            .unwrap();
        model
            .apply_event(CloudBackupStateReducerEvent::PendingUploadVerificationReconciled(
                PendingUploadVerificationState::BlockedOnAuthorization,
            ))
            .unwrap();

        model
            .apply_event(CloudBackupStateReducerEvent::SyncStateResolved(SyncState::Syncing))
            .unwrap();

        let CloudBackupLifecycle::Configured(state) = model.public_state().lifecycle else {
            panic!("enabled backup should project configured lifecycle");
        };

        assert_eq!(
            state.sync,
            CloudBackupSyncState::Blocked(PENDING_UPLOAD_AUTHORIZATION_BLOCKED_MESSAGE.into()),
        );
    }

    #[test]
    fn matching_finish_releases_reconciled_background_status() {
        let mut model = CloudBackupStateReducer::default();
        let claim =
            CloudBackupExclusiveOperationClaim::new(CloudBackupExclusiveOperation::Enable, 1);
        model.apply_event(CloudBackupStateReducerEvent::ExclusiveOperationStarted(claim)).unwrap();

        let runtime_effects = model
            .apply_event(CloudBackupStateReducerEvent::RuntimeStatusReconciled(
                CloudBackupStatus::Enabled,
            ))
            .unwrap();
        let effects = model
            .apply_event(CloudBackupStateReducerEvent::ExclusiveOperationFinished(claim))
            .unwrap();

        assert!(matches!(runtime_effects.lifecycle, Some(CloudBackupLifecycle::Configured(_))));
        assert!(effects.status_changed);
        assert_eq!(model.status(), CloudBackupStatus::Enabled);
    }

    #[test]
    fn runtime_enabled_preserves_disable_failed_signal() {
        let mut model = CloudBackupStateReducer::default();
        model
            .apply_event(CloudBackupStateReducerEvent::RuntimeStatusReconciled(
                CloudBackupStatus::Enabled,
            ))
            .unwrap();
        model
            .apply_event(CloudBackupStateReducerEvent::DisableStateResolved(
                CloudBackupDisableOutcome::Failed {
                    message: "blocked".into(),
                    can_keep_enabled: true,
                },
            ))
            .unwrap();

        model
            .apply_event(CloudBackupStateReducerEvent::RuntimeStatusReconciled(
                CloudBackupStatus::Enabled,
            ))
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
        let mut model = CloudBackupStateReducer::default();
        model
            .apply_event(CloudBackupStateReducerEvent::RuntimeStatusReconciled(
                CloudBackupStatus::Enabled,
            ))
            .unwrap();
        model
            .apply_event(CloudBackupStateReducerEvent::DisableStateResolved(
                CloudBackupDisableOutcome::Failed {
                    message: "blocked".into(),
                    can_keep_enabled: true,
                },
            ))
            .unwrap();

        model
            .apply_event(CloudBackupStateReducerEvent::DisableStateResolved(
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
        let mut model = CloudBackupStateReducer::default();
        model
            .apply_event(CloudBackupStateReducerEvent::RuntimeStatusReconciled(
                CloudBackupStatus::Enabled,
            ))
            .unwrap();
        model
            .apply_event(CloudBackupStateReducerEvent::PasskeyChoicePromptSet(
                CloudBackupPasskeyChoiceIntent::RepairPasskey,
            ))
            .unwrap();

        model
            .apply_event(CloudBackupStateReducerEvent::DisableStateResolved(
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
        let mut model = CloudBackupStateReducer::default();
        model
            .apply_event(CloudBackupStateReducerEvent::VerificationStateResolved(
                VerificationState::Verified(report.clone()),
            ))
            .unwrap();

        let effects = model
            .apply_event(CloudBackupStateReducerEvent::VerificationStateResolved(
                VerificationState::Verified(report),
            ))
            .unwrap();

        assert_eq!(effects, CloudBackupStateReducerEffects::default());
    }

    #[test]
    fn restoring_carries_restore_progress() {
        let progress = CloudBackupRestoreFlow::Downloading { completed: 1, total: 3 };
        let mut model = CloudBackupStateReducer::default();

        model.apply_event(operation_event(CloudBackupExclusiveOperation::Restore, 1)).unwrap();
        model
            .apply_event(CloudBackupStateReducerEvent::RestoreProgressReported(progress.clone()))
            .unwrap();

        assert_eq!(model.public_state().lifecycle, CloudBackupLifecycle::Restoring(progress));
    }

    #[test]
    fn stray_enable_progress_does_not_enter_enabling() {
        let mut model = CloudBackupStateReducer::default();

        let effects = model
            .apply_event(CloudBackupStateReducerEvent::EnableProgressReported(Some(
                CloudBackupProgress { completed: 1, total: 2 },
            )))
            .unwrap();

        assert_eq!(model.public_state().lifecycle, CloudBackupLifecycle::Disabled);
        assert_eq!(effects, CloudBackupStateReducerEffects::default());
    }

    #[test]
    fn stray_restore_progress_does_not_enter_restoring() {
        let mut model = CloudBackupStateReducer::default();

        let effects = model
            .apply_event(CloudBackupStateReducerEvent::RestoreProgressReported(
                CloudBackupRestoreFlow::Downloading { completed: 1, total: 3 },
            ))
            .unwrap();

        assert_eq!(model.public_state().lifecycle, CloudBackupLifecycle::Disabled);
        assert_eq!(effects, CloudBackupStateReducerEffects::default());
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
        let mut model = CloudBackupStateReducer::default();

        model
            .apply_event(CloudBackupStateReducerEvent::RuntimeStatusReconciled(
                CloudBackupStatus::Enabled,
            ))
            .unwrap();
        model
            .apply_event(CloudBackupStateReducerEvent::VerificationStateResolved(
                VerificationState::Verified(report),
            ))
            .unwrap();
        model
            .apply_event(CloudBackupStateReducerEvent::SyncStateResolved(SyncState::Syncing))
            .unwrap();
        model
            .apply_event(CloudBackupStateReducerEvent::PendingUploadVerificationReconciled(
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
        let mut missing = CloudBackupStateReducer::default();
        missing
            .apply_event(CloudBackupStateReducerEvent::RuntimeStatusReconciled(
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

        let mut repairing = CloudBackupStateReducer::default();
        repairing
            .apply_event(CloudBackupStateReducerEvent::RecoveryStateResolved(
                RecoveryState::Recovering(RecoveryAction::RepairPasskey),
            ))
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
        let mut model = CloudBackupStateReducer::default();
        model
            .apply_event(CloudBackupStateReducerEvent::RuntimeStatusReconciled(
                CloudBackupStatus::Enabled,
            ))
            .unwrap();
        model
            .apply_event(CloudBackupStateReducerEvent::VerificationPresentationReconciled(
                CloudBackupVerificationPresentation::Hidden {
                    source: Some(CloudBackupVerificationSource::Settings),
                },
            ))
            .unwrap();

        let effects = model
            .apply_event(CloudBackupStateReducerEvent::VerificationFlagsReconciled {
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
        let mut model = CloudBackupStateReducer::default();
        model
            .apply_event(CloudBackupStateReducerEvent::RuntimeStatusReconciled(
                CloudBackupStatus::Enabled,
            ))
            .unwrap();
        model
            .apply_event(CloudBackupStateReducerEvent::VerificationFlagsReconciled {
                metadata: CloudBackupVerificationMetadata::NeedsVerification,
                should_prompt: true,
            })
            .unwrap();
        model
            .apply_event(CloudBackupStateReducerEvent::VerificationPresentationReconciled(
                CloudBackupVerificationPresentation::NeedsDecision {
                    reason: CloudBackupVerificationReason::BackupChanged,
                    source: CloudBackupVerificationSource::RootPrompt,
                },
            ))
            .unwrap();

        let effects = model
            .apply_event(CloudBackupStateReducerEvent::VerificationFlagsReconciled {
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
        let mut model = CloudBackupStateReducer::default();
        model
            .apply_event(CloudBackupStateReducerEvent::RuntimeStatusReconciled(
                CloudBackupStatus::Enabled,
            ))
            .unwrap();
        model
            .apply_event(CloudBackupStateReducerEvent::VerificationFlagsReconciled {
                metadata: CloudBackupVerificationMetadata::NeedsVerification,
                should_prompt: true,
            })
            .unwrap();

        let effects = model
            .apply_event(
                CloudBackupStateReducerEvent::PendingUploadVerificationAndFlagsReconciled {
                    pending: PendingUploadVerificationState::Idle,
                    metadata: CloudBackupVerificationMetadata::NeedsVerification,
                    should_prompt: true,
                },
            )
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
        let mut model = CloudBackupStateReducer::default();

        model
            .apply_event(CloudBackupStateReducerEvent::ExistingBackupFoundPromptSet {
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
    fn accepting_existing_backup_prompt_keeps_enable_lifecycle_active() {
        let context = CloudBackupEnableContext::settings_manual();
        let mut model = CloudBackupStateReducer::default();

        model
            .apply_event(CloudBackupStateReducerEvent::ExistingBackupFoundPromptSet {
                context,
                passkey_hint: None,
            })
            .unwrap();

        let (accepted, effects) =
            model.accept_enable_prompt(CloudBackupEnablePromptChoice::CreateNew);

        assert_eq!(accepted, Some(CloudBackupAcceptedEnablePrompt::ForceNew(context)));
        assert_eq!(model.snapshot().root_prompt, CloudBackupRootPrompt::None);
        assert_eq!(
            model.public_state().lifecycle,
            CloudBackupLifecycle::Enabling(CloudBackupEnableFlow::DiscoveringExistingBackup),
        );
        assert_eq!(
            effects.lifecycle,
            Some(CloudBackupLifecycle::Enabling(CloudBackupEnableFlow::DiscoveringExistingBackup,)),
        );
    }

    #[test]
    fn accepting_enable_passkey_choice_keeps_enable_lifecycle_active() {
        let context = CloudBackupEnableContext::settings_manual();
        let mut model = CloudBackupStateReducer::default();

        model
            .apply_event(CloudBackupStateReducerEvent::PasskeyChoicePromptSet(
                CloudBackupPasskeyChoiceIntent::Enable(context, None),
            ))
            .unwrap();

        let (accepted, effects) =
            model.accept_enable_prompt(CloudBackupEnablePromptChoice::UseExisting);

        assert_eq!(accepted, Some(CloudBackupAcceptedEnablePrompt::Enable(context)));
        assert_eq!(model.snapshot().root_prompt, CloudBackupRootPrompt::None);
        assert_eq!(
            model.public_state().lifecycle,
            CloudBackupLifecycle::Enabling(CloudBackupEnableFlow::DiscoveringExistingBackup),
        );
        assert_eq!(
            effects.lifecycle,
            Some(CloudBackupLifecycle::Enabling(CloudBackupEnableFlow::DiscoveringExistingBackup,)),
        );
    }

    #[test]
    fn root_prompt_is_derived_from_configured_state() {
        let mut model = CloudBackupStateReducer::default();

        let effects = model
            .apply_event(CloudBackupStateReducerEvent::RuntimeStatusReconciled(
                CloudBackupStatus::PasskeyMissing,
            ))
            .unwrap();

        assert_eq!(model.snapshot().root_prompt, CloudBackupRootPrompt::MissingPasskeyReminder);
        assert!(matches!(effects.lifecycle, Some(CloudBackupLifecycle::Configured(_))));
    }
}
