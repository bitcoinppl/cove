//! Cloud Backup private reducer and public UI state projection
//!
//! The reducer keeps impossible intermediate states out of the UniFFI model and
//! projects a smaller public state for Swift and Kotlin. Exclusive operation
//! claims are tracked here so stale async completions cannot clear newer work

use super::{
    CloudBackupDetail, CloudBackupEnableContext, CloudBackupEnablePromptChoice, CloudBackupError,
    CloudBackupPasskeyChoiceIntent, CloudBackupRootPrompt, CloudBackupSettingsRowStatus,
    CloudBackupVerificationMetadata, CloudBackupVerificationPresentation,
    CloudBackupVerificationReason, CloudBackupWalletStatus, CloudOnlyOperation, CloudOnlyState,
    OtherBackupsOperation, PendingUploadVerificationState, RecoveryAction, RecoveryState,
    SyncState, VerificationState, is_connectivity_related_issue,
};

use super::verify::coordinator::CloudBackupVerificationCoordinator;
use cove_device::cloud_storage::CloudSyncHealth;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

mod events;
mod state_types;

pub(crate) use self::events::{
    CloudBackupAcceptedEnablePrompt, CloudBackupExclusiveOperation,
    CloudBackupExclusiveOperationClaim, CloudBackupLifecycleEffect,
    CloudBackupRestoreAllRuntimeState, CloudBackupStateReducerEffects,
    CloudBackupStateReducerEvent, CloudBackupStateReducerEventRejection,
};
pub use self::state_types::{
    CloudBackupConfiguredState, CloudBackupDestructiveOperationState, CloudBackupDetailState,
    CloudBackupEnableFlow, CloudBackupFailure, CloudBackupInventoryIncompleteReason,
    CloudBackupLifecycle, CloudBackupPasskeyRepairState, CloudBackupPasskeyState,
    CloudBackupPendingEnableCleanupState, CloudBackupPendingEnableRecovery,
    CloudBackupRestoreAllState, CloudBackupRestoreFlow, CloudBackupSyncState,
    CloudBackupVerificationState, LoadedCloudBackupDetail,
};

const PENDING_UPLOAD_AUTHORIZATION_BLOCKED_MESSAGE: &str = "cloud authorization required";
const STALE_VERIFICATION_THRESHOLD: Duration = Duration::from_secs(60 * 60 * 24 * 30);

/// Runtime cloud backup status projected from persisted and in-process state
#[derive(Debug, Clone, Hash, Eq, PartialEq)]
pub(crate) enum CloudBackupStatus {
    Disabled,
    Disabling,
    Enabling,
    Restoring,
    Enabled,
    PasskeyMissing,
    UnsupportedPasskeyProvider,
    Error(String),
}

/// Internal enable status before projection into the public lifecycle
#[derive(Debug, Clone, Hash, Eq, PartialEq)]
pub(crate) enum CloudBackupEnableState {
    Idle,
    CreatingPasskey,
    WaitingForPasskeyAvailability,
    AwaitingSavedPasskeyConfirmation(super::SavedPasskeyConfirmationMode),
    ConfirmingSavedPasskey,
    UploadingBackup,
}

/// Result of a disable attempt after the supervisor resolves remote and local work
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum CloudBackupDisableOutcome {
    Started,
    ReturnedToIdle,
    Failed { message: String, can_keep_enabled: bool },
}

/// Remote detail fetch result that keeps access errors distinguishable from failed detail state
#[derive(Debug)]
pub(crate) enum CloudBackupDetailResult {
    Success(CloudBackupDetail),
    AccessError(CloudBackupError),
}

impl CloudBackupDetailResult {
    pub(crate) fn is_connectivity_access_error(&self) -> bool {
        matches!(self, Self::AccessError(error) if is_connectivity_related_issue(error))
    }
}

#[derive(Debug)]
pub(crate) struct CloudBackupDetailInventorySnapshot {
    pub(crate) namespace: String,
    pub(crate) wallet_record_ids: Vec<String>,
    pub(crate) is_complete: bool,
    pub(crate) provisional_detail: Option<CloudBackupDetail>,
}

#[derive(Debug)]
pub(crate) enum CloudBackupDetailInventorySnapshotResult {
    Success(CloudBackupDetailInventorySnapshot),
    AccessError(CloudBackupError),
}

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
    active_enable_context: Option<CloudBackupEnableContext>,
    restore_all: CloudBackupRestoreAllRuntimeState,
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
    PendingEnableRecovery(CloudBackupPendingEnableRecovery),
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
    detail_refresh: DetailRefreshActivity,
    prompt: CloudBackupConfiguredPrompt,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
enum DetailRefreshActivity {
    #[default]
    Idle,
    InFlight,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum CloudBackupConfiguredPrompt {
    None,
    PasskeyChoice(CloudBackupPasskeyChoiceIntent),
}

impl Default for CloudBackupReducerState {
    fn default() -> Self {
        Self {
            phase: CloudBackupLifecyclePhase::Disabled,
            configured: CloudBackupConfiguredReducerState::default(),
            active_operation: None,
            active_enable_context: None,
            restore_all: CloudBackupRestoreAllRuntimeState::Idle,
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
            detail_refresh: DetailRefreshActivity::Idle,
            prompt: CloudBackupConfiguredPrompt::None,
        }
    }
}

impl CloudBackupReducerState {
    fn public_state(&self) -> super::CloudBackupState {
        super::CloudBackupState {
            lifecycle: self.public_lifecycle(),
            settings_row_status: self.settings_row_status(),
        }
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
            CloudBackupLifecyclePhase::PendingEnableRecovery(recovery) => {
                CloudBackupLifecycle::PendingEnableRecovery(recovery.clone())
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
            restore_all: self.public_restore_all_state(),
            root_prompt: self.root_prompt(),
            sync_health: self.sync_health.clone(),
            verification_presentation: self.verification_presentation.clone(),
        }
    }

    fn public_restore_all_state(&self) -> CloudBackupRestoreAllState {
        let wallet_count = self.restore_all_eligible_wallet_count();

        match &self.restore_all {
            CloudBackupRestoreAllRuntimeState::Running {
                claim: _,
                completed,
                total,
                current_wallet_name,
                cancellation_requested,
            } => CloudBackupRestoreAllState::Running {
                completed: *completed,
                total: *total,
                current_wallet_name: current_wallet_name.clone(),
                cancellation_requested: *cancellation_requested,
            },
            CloudBackupRestoreAllRuntimeState::RetryRemaining => {
                if wallet_count == 0 {
                    return CloudBackupRestoreAllState::NotShown;
                }

                if self.restore_all_action_is_available() {
                    CloudBackupRestoreAllState::RetryAvailable { wallet_count }
                } else {
                    CloudBackupRestoreAllState::RetryDisabled { wallet_count }
                }
            }
            CloudBackupRestoreAllRuntimeState::Idle => {
                if wallet_count < 2 {
                    return CloudBackupRestoreAllState::NotShown;
                }

                if self.restore_all_action_is_available() {
                    CloudBackupRestoreAllState::StartAvailable { wallet_count }
                } else {
                    CloudBackupRestoreAllState::StartDisabled { wallet_count }
                }
            }
        }
    }

    fn restore_all_eligible_wallet_count(&self) -> u32 {
        let Some(LoadedCloudBackupDetail {
            cloud_only: CloudOnlyState::Loaded { wallets }, ..
        }) = self.loaded_detail()
        else {
            return 0;
        };

        wallets
            .iter()
            .filter(|wallet| wallet.sync_status == CloudBackupWalletStatus::DeletedFromDevice)
            .count()
            .try_into()
            .unwrap_or(u32::MAX)
    }

    fn restore_all_action_is_available(&self) -> bool {
        self.detail_inventory_is_complete() && self.active_operation.is_none()
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
            | CloudBackupLifecyclePhase::PendingEnableRecovery(_)
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
                    CloudBackupPasskeyChoiceIntent::EnableExistingPasskeyOnly(context, _),
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
                | CloudBackupExclusiveOperation::RestoreAllCloudWallets
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
            CloudBackupLifecyclePhase::PendingEnableRecovery(_) => {
                CloudBackupStatus::Error("Cloud Backup recovery required".into())
            }
            CloudBackupLifecyclePhase::Failed(failure) => {
                CloudBackupStatus::Error(failure.message.clone())
            }
        }
    }

    fn settings_row_status(&self) -> CloudBackupSettingsRowStatus {
        if matches!(self.phase, CloudBackupLifecyclePhase::PendingEnableRecovery(_)) {
            return CloudBackupSettingsRowStatus::RecoveryRequired;
        }

        match self.status() {
            CloudBackupStatus::Disabled => return CloudBackupSettingsRowStatus::Disabled,
            CloudBackupStatus::Disabling => return CloudBackupSettingsRowStatus::Disabling,
            CloudBackupStatus::Enabling => return CloudBackupSettingsRowStatus::SettingUp,
            CloudBackupStatus::Restoring => return CloudBackupSettingsRowStatus::Restoring,
            CloudBackupStatus::PasskeyMissing => {
                return CloudBackupSettingsRowStatus::PasskeyMissing;
            }
            CloudBackupStatus::UnsupportedPasskeyProvider => {
                return CloudBackupSettingsRowStatus::PasskeyProviderUnsupported;
            }
            CloudBackupStatus::Error(message) => {
                return CloudBackupSettingsRowStatus::Error(message);
            }
            CloudBackupStatus::Enabled => {}
        }

        if !matches!(
            self.configured.pending_upload_verification,
            PendingUploadVerificationState::Idle
        ) {
            return CloudBackupSettingsRowStatus::Confirming;
        }

        if matches!(
            self.configured.verification,
            CloudBackupVerificationState::Required | CloudBackupVerificationState::Cancelled
        ) {
            return CloudBackupSettingsRowStatus::Unverified;
        }

        match &self.sync_health {
            CloudSyncHealth::AllUploaded if self.is_verification_stale() => {
                CloudBackupSettingsRowStatus::VerificationRecommended
            }
            CloudSyncHealth::AllUploaded => CloudBackupSettingsRowStatus::Active,
            CloudSyncHealth::Uploading => CloudBackupSettingsRowStatus::Syncing,
            CloudSyncHealth::Unknown => CloudBackupSettingsRowStatus::CheckingSync,
            CloudSyncHealth::NoFiles => CloudBackupSettingsRowStatus::NoFiles,
            CloudSyncHealth::AuthorizationRequired(message) => {
                CloudBackupSettingsRowStatus::AuthorizationRequired(message.clone())
            }
            CloudSyncHealth::Unavailable => CloudBackupSettingsRowStatus::DriveUnavailable,
            CloudSyncHealth::Failed(message) => {
                CloudBackupSettingsRowStatus::Error(message.clone())
            }
        }
    }

    fn is_verification_stale(&self) -> bool {
        if !matches!(self.configured.passkey, CloudBackupPasskeyState::Available) {
            return false;
        }

        let CloudBackupVerificationState::Verified { last_verified_at, .. } =
            self.configured.verification
        else {
            return false;
        };

        let Some(last_verified_at) = last_verified_at else {
            return true;
        };

        let Ok(now) = SystemTime::now().duration_since(UNIX_EPOCH) else {
            return false;
        };

        now.as_secs().saturating_sub(last_verified_at) >= STALE_VERIFICATION_THRESHOLD.as_secs()
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
            CloudBackupVerificationState::Cancelled => VerificationState::Cancelled,
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
            CloudBackupDetailState::Complete { state } => Some(state),
            CloudBackupDetailState::Checking { retained }
            | CloudBackupDetailState::Failed { retained, .. } => retained.as_ref(),
            CloudBackupDetailState::NotLoaded => None,
        }
    }

    fn loaded_detail_mut(&mut self) -> Option<&mut LoadedCloudBackupDetail> {
        match &mut self.configured.detail {
            CloudBackupDetailState::Complete { state } => Some(state),
            CloudBackupDetailState::Checking { retained }
            | CloudBackupDetailState::Failed { retained, .. } => retained.as_mut(),
            CloudBackupDetailState::NotLoaded => None,
        }
    }

    fn detail_inventory_is_complete(&self) -> bool {
        matches!(self.configured.detail, CloudBackupDetailState::Complete { .. })
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
            | CloudBackupExclusiveOperation::RestoreAllCloudWallets
            | CloudBackupExclusiveOperation::DeleteCloudWallet => {}
        }
    }

    fn start_restore_all(&mut self, claim: CloudBackupExclusiveOperationClaim, total: u32) {
        if total == 0 || self.active_operation.is_some_and(|active| active != claim) {
            return;
        }

        if self.active_operation.is_none() {
            self.active_operation = Some(claim);
            self.apply_exclusive_operation_start(claim.operation());
        }

        self.restore_all = CloudBackupRestoreAllRuntimeState::Running {
            claim,
            completed: 0,
            total,
            current_wallet_name: None,
            cancellation_requested: false,
        };
    }

    fn report_restore_all_progress(
        &mut self,
        claim: CloudBackupExclusiveOperationClaim,
        completed: u32,
        current_wallet_name: Option<String>,
    ) {
        let CloudBackupRestoreAllRuntimeState::Running {
            claim: active_claim,
            completed: current_completed,
            total,
            current_wallet_name: current_name,
            ..
        } = &mut self.restore_all
        else {
            return;
        };
        if *active_claim != claim || completed < *current_completed || completed > *total {
            return;
        }

        *current_completed = completed;
        *current_name = current_wallet_name;
    }

    fn request_restore_all_cancellation(&mut self, claim: CloudBackupExclusiveOperationClaim) {
        let CloudBackupRestoreAllRuntimeState::Running {
            claim: active_claim,
            cancellation_requested,
            ..
        } = &mut self.restore_all
        else {
            return;
        };
        if *active_claim != claim {
            return;
        }

        *cancellation_requested = true;
    }

    fn finish_restore_all(
        &mut self,
        claim: CloudBackupExclusiveOperationClaim,
        retry_remaining: bool,
    ) {
        let CloudBackupRestoreAllRuntimeState::Running { claim: active_claim, .. } =
            &self.restore_all
        else {
            return;
        };
        if *active_claim != claim || self.active_operation != Some(claim) {
            return;
        }

        self.active_operation = None;
        self.restore_all = if retry_remaining {
            CloudBackupRestoreAllRuntimeState::RetryRemaining
        } else {
            CloudBackupRestoreAllRuntimeState::Idle
        };
    }

    fn apply_status(&mut self, status: CloudBackupStatus) {
        match status {
            CloudBackupStatus::Disabled => {
                self.active_enable_context = None;
                self.phase = CloudBackupLifecyclePhase::Disabled;
            }
            CloudBackupStatus::Disabling => {
                self.active_enable_context = None;
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
                self.active_enable_context = None;
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
                self.active_enable_context = None;
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
                self.active_enable_context = None;
                self.configured.passkey = CloudBackupPasskeyState::UnsupportedProvider;
                self.configured.prompt = CloudBackupConfiguredPrompt::None;
                self.phase = CloudBackupLifecyclePhase::Configured;
            }
            CloudBackupStatus::Error(message) => {
                self.active_enable_context = None;
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
            VerificationState::Idle => self.idle_verification_state(),
            VerificationState::Cancelled => CloudBackupVerificationState::Cancelled,
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
        self.configured.detail_refresh = DetailRefreshActivity::Idle;
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

                CloudBackupDetailState::Complete {
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

    fn apply_detail_refresh_started(&mut self) {
        let retained = self.loaded_detail().cloned();
        self.configured.detail_refresh = DetailRefreshActivity::InFlight;
        self.configured.detail = CloudBackupDetailState::Checking { retained };
    }

    fn apply_detail_refresh_provisional(&mut self, detail: CloudBackupDetail) {
        self.configured.detail_refresh = DetailRefreshActivity::InFlight;

        if let Some(previous_loaded) = self.loaded_detail().cloned() {
            self.configured.detail =
                CloudBackupDetailState::Checking { retained: Some(previous_loaded) };
            return;
        }

        self.configured.detail = CloudBackupDetailState::Checking {
            retained: Some(LoadedCloudBackupDetail {
                detail,
                cloud_only: CloudOnlyState::NotFetched,
                cloud_only_operation: CloudOnlyOperation::Idle,
                other_backups_operation: OtherBackupsOperation::Idle,
            }),
        };
    }

    fn apply_detail_refresh_failure(
        &mut self,
        reason: CloudBackupInventoryIncompleteReason,
        error: String,
    ) {
        let retained = self.loaded_detail().cloned();
        self.configured.detail_refresh = DetailRefreshActivity::Idle;
        self.configured.detail = CloudBackupDetailState::Failed { reason, error, retained };
    }

    fn resolve_cloud_only_state(&mut self, cloud_only: CloudOnlyState) {
        let detail_refresh_in_flight =
            self.configured.detail_refresh == DetailRefreshActivity::InFlight;

        match (&mut self.configured.detail, cloud_only) {
            (CloudBackupDetailState::Complete { state }, cloud_only) => {
                state.cloud_only = cloud_only;
            }
            (CloudBackupDetailState::Checking { retained }, CloudOnlyState::Loaded { wallets }) => {
                let Some(state) = retained.as_mut() else {
                    return;
                };

                state.cloud_only = CloudOnlyState::Loaded { wallets };
                if !detail_refresh_in_flight && let Some(state) = retained.take() {
                    self.configured.detail = CloudBackupDetailState::Complete { state };
                }
            }
            (detail, CloudOnlyState::Loading) => {
                let mut retained = match detail {
                    CloudBackupDetailState::Complete { state } => Some(state.clone()),
                    CloudBackupDetailState::Checking { retained }
                    | CloudBackupDetailState::Failed { retained, .. } => retained.clone(),
                    CloudBackupDetailState::NotLoaded => None,
                };
                if let Some(state) = &mut retained {
                    state.cloud_only = CloudOnlyState::Loading;
                }
                *detail = CloudBackupDetailState::Checking { retained };
            }
            (
                CloudBackupDetailState::Failed { retained, .. },
                failed @ CloudOnlyState::Failed { .. },
            ) => {
                if let Some(state) = retained {
                    state.cloud_only = failed;
                }
            }
            (detail, CloudOnlyState::Failed { error }) => {
                let mut retained = match detail {
                    CloudBackupDetailState::Complete { state } => Some(state.clone()),
                    CloudBackupDetailState::Checking { retained }
                    | CloudBackupDetailState::Failed { retained, .. } => retained.clone(),
                    CloudBackupDetailState::NotLoaded => None,
                };
                if let Some(state) = &mut retained {
                    state.cloud_only = CloudOnlyState::Failed { error: error.clone() };
                }

                if detail_refresh_in_flight {
                    *detail = CloudBackupDetailState::Checking { retained };
                } else {
                    *detail = CloudBackupDetailState::Failed {
                        reason: CloudBackupInventoryIncompleteReason::Unknown,
                        error,
                        retained,
                    };
                }
            }
            (detail, CloudOnlyState::NotFetched) => {
                if detail_refresh_in_flight {
                    if let CloudBackupDetailState::Checking { retained: Some(state) } = detail {
                        state.cloud_only = CloudOnlyState::NotFetched;
                    }
                } else {
                    *detail = CloudBackupDetailState::NotLoaded;
                }
            }
            (CloudBackupDetailState::Failed { retained: Some(state), .. }, loaded) => {
                state.cloud_only = loaded;
            }
            (CloudBackupDetailState::NotLoaded, CloudOnlyState::Loaded { .. })
            | (
                CloudBackupDetailState::Failed { retained: None, .. },
                CloudOnlyState::Loaded { .. },
            ) => {}
        }
    }

    fn resolve_cloud_only_operation(&mut self, cloud_only_operation: CloudOnlyOperation) {
        if let Some(state) = self.loaded_detail_mut() {
            state.cloud_only_operation = cloud_only_operation;
        }
    }

    fn resolve_other_backups_operation(&mut self, other_backups_operation: OtherBackupsOperation) {
        if let Some(state) = self.loaded_detail_mut() {
            state.other_backups_operation = other_backups_operation;
        }
    }

    fn clear_prompt_state(&mut self) {
        self.configured.prompt = CloudBackupConfiguredPrompt::None;
        self.missing_passkey_dismissed = false;
        self.active_enable_context = None;

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

    pub(crate) fn detail_inventory_is_complete(&self) -> bool {
        self.state.detail_inventory_is_complete()
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
                            | CloudBackupPasskeyChoiceIntent::EnableExistingPasskeyOnly(_, _)
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
            CloudBackupStateReducerEvent::EnableContextStarted(context) => {
                self.state.active_enable_context = Some(context);
            }
            CloudBackupStateReducerEvent::RuntimeStatusReconciled(status) => {
                self.state.apply_status(status);
            }
            CloudBackupStateReducerEvent::PendingEnableRecoveryProjected(recovery) => {
                self.state.active_enable_context = None;
                self.state.phase = CloudBackupLifecyclePhase::PendingEnableRecovery(recovery);
            }
            CloudBackupStateReducerEvent::ExistingBackupFoundPromptSet {
                context,
                passkey_hint,
            } => {
                self.state.active_enable_context = Some(context);
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
                    self.state.active_enable_context = None;
                    self.state.phase = CloudBackupLifecyclePhase::Disabled;
                }
            }
            CloudBackupStateReducerEvent::PasskeyChoicePromptSet(intent) => match &intent {
                CloudBackupPasskeyChoiceIntent::Enable(context, _)
                | CloudBackupPasskeyChoiceIntent::EnableExistingPasskeyOnly(context, _) => {
                    self.state.active_enable_context = Some(*context);
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
                                | CloudBackupPasskeyChoiceIntent::EnableExistingPasskeyOnly(_, _)
                        )
                    )
                ) {
                    self.state.active_enable_context = None;
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
            CloudBackupStateReducerEvent::RestoreAllStarted { claim, total } => {
                self.state.start_restore_all(claim, total);
            }
            CloudBackupStateReducerEvent::RestoreAllProgressed {
                claim,
                completed,
                current_wallet_name,
            } => {
                self.state.report_restore_all_progress(claim, completed, current_wallet_name);
            }
            CloudBackupStateReducerEvent::RestoreAllCancellationRequested(claim) => {
                self.state.request_restore_all_cancellation(claim);
            }
            CloudBackupStateReducerEvent::RestoreAllFinished { claim, retry_remaining } => {
                self.state.finish_restore_all(claim, retry_remaining);
            }
            CloudBackupStateReducerEvent::RestoreAllRetryRequired => {
                if !matches!(
                    self.state.restore_all,
                    CloudBackupRestoreAllRuntimeState::Running { .. }
                ) {
                    self.state.restore_all = CloudBackupRestoreAllRuntimeState::RetryRemaining;
                }
            }
            CloudBackupStateReducerEvent::RestoreAllReset => {
                if !matches!(
                    self.state.restore_all,
                    CloudBackupRestoreAllRuntimeState::Running { .. }
                ) {
                    self.state.restore_all = CloudBackupRestoreAllRuntimeState::Idle;
                }
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
            CloudBackupStateReducerEvent::DetailRefreshStarted => {
                self.state.apply_detail_refresh_started();
            }
            CloudBackupStateReducerEvent::DetailRefreshProvisional(detail) => {
                self.state.apply_detail_refresh_provisional(detail);
            }
            CloudBackupStateReducerEvent::DetailRefreshApplied { detail, reset_cloud_only } => {
                self.state.apply_detail_refresh(detail, reset_cloud_only);
            }
            CloudBackupStateReducerEvent::DetailRefreshFailed { reason, error } => {
                self.state.apply_detail_refresh_failure(reason, error);
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
        &mut self,
        previous_status: CloudBackupStatus,
        previous_lifecycle: CloudBackupLifecycle,
        previous_presentation: CloudBackupVerificationPresentation,
        effects: &mut CloudBackupStateReducerEffects,
    ) {
        let lifecycle = self.state.public_lifecycle();
        if lifecycle != previous_lifecycle {
            effects.lifecycle = Some(CloudBackupLifecycleEffect {
                lifecycle,
                settings_row_status: self.state.settings_row_status(),
            });
        }

        let status = self.state.status();
        if previous_status == CloudBackupStatus::Enabling && status == CloudBackupStatus::Enabled {
            effects.enable_completed = self.state.active_enable_context.take();
        }

        effects.status_changed = status != previous_status;
        effects.verification_presentation_changed =
            self.state.verification_presentation != previous_presentation;
    }
}

#[cfg(test)]
pub(crate) mod test_support {
    use super::*;
    use crate::manager::cloud_backup_manager::CloudBackupProgress;

    #[derive(Debug, Clone)]
    pub(crate) struct CloudBackupModelSnapshot {
        pub(crate) root_prompt: CloudBackupRootPrompt,
        pub(crate) status: CloudBackupStatus,
        pub(crate) sync_health: CloudSyncHealth,
        pub(crate) progress: Option<CloudBackupProgress>,
        pub(crate) restore_progress: Option<CloudBackupRestoreFlow>,
        pub(crate) enable_state: CloudBackupEnableState,
        pub(crate) pending_upload_verification: PendingUploadVerificationState,
        pub(crate) verification_presentation: CloudBackupVerificationPresentation,
        pub(crate) detail: Option<CloudBackupDetail>,
        pub(crate) verification: VerificationState,
    }

    impl Default for CloudBackupModelSnapshot {
        fn default() -> Self {
            Self {
                root_prompt: CloudBackupRootPrompt::None,
                status: CloudBackupStatus::Disabled,
                sync_health: CloudSyncHealth::Unknown,
                progress: None,
                restore_progress: None,
                enable_state: CloudBackupEnableState::Idle,
                pending_upload_verification: PendingUploadVerificationState::Idle,
                verification_presentation: CloudBackupVerificationPresentation::Hidden {
                    source: None,
                },
                detail: None,
                verification: VerificationState::Idle,
            }
        }
    }

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
        CloudBackupEnableContext, CloudBackupOtherBackupsState, CloudBackupPasskeyHint,
        CloudBackupProgress, CloudBackupVerificationMetadata, CloudBackupVerificationPresentation,
        CloudBackupVerificationReason, CloudBackupVerificationSource, CloudBackupWalletItem,
        DeepVerificationReport,
    };

    fn operation_event(
        operation: CloudBackupExclusiveOperation,
        generation: u64,
    ) -> CloudBackupStateReducerEvent {
        CloudBackupStateReducerEvent::ExclusiveOperationStarted(
            CloudBackupExclusiveOperationClaim::new(operation, generation),
        )
    }

    fn effect_lifecycle(effects: &CloudBackupStateReducerEffects) -> Option<&CloudBackupLifecycle> {
        effects.lifecycle.as_ref().map(|effect| &effect.lifecycle)
    }

    fn configured_state(
        verification: CloudBackupVerificationState,
        sync_health: CloudSyncHealth,
    ) -> CloudBackupReducerState {
        CloudBackupReducerState {
            phase: CloudBackupLifecyclePhase::Configured,
            configured: CloudBackupConfiguredReducerState {
                passkey: CloudBackupPasskeyState::Available,
                verification,
                ..Default::default()
            },
            sync_health,
            ..Default::default()
        }
    }

    fn test_detail(cloud_only_count: u32) -> CloudBackupDetail {
        CloudBackupDetail {
            last_sync: None,
            up_to_date: Vec::new(),
            needs_sync: Vec::new(),
            cloud_only_count,
            other_backups: CloudBackupOtherBackupsState::Loaded { summary: Default::default() },
        }
    }

    fn cloud_only_wallet(
        record_id: &str,
        sync_status: CloudBackupWalletStatus,
    ) -> CloudBackupWalletItem {
        CloudBackupWalletItem {
            name: record_id.into(),
            network: None,
            wallet_mode: None,
            wallet_type: None,
            fingerprint: None,
            label_count: None,
            backup_updated_at: None,
            sync_status,
            restore_failure: None,
            record_id: record_id.into(),
        }
    }

    fn configured_model_with_cloud_only(
        wallets: Vec<CloudBackupWalletItem>,
    ) -> CloudBackupStateReducer {
        let mut model = CloudBackupStateReducer {
            state: configured_state(
                CloudBackupVerificationState::NotVerified,
                CloudSyncHealth::Unknown,
            ),
        };
        model
            .apply_event(CloudBackupStateReducerEvent::DetailRefreshApplied {
                detail: Some(test_detail(wallets.len() as u32)),
                reset_cloud_only: false,
            })
            .unwrap();
        model
            .apply_event(CloudBackupStateReducerEvent::CloudOnlyStateResolved(
                CloudOnlyState::Loaded { wallets },
            ))
            .unwrap();
        model
    }

    fn restore_all_state(model: &CloudBackupStateReducer) -> CloudBackupRestoreAllState {
        let CloudBackupLifecycle::Configured(configured) = model.public_state().lifecycle else {
            panic!("expected configured lifecycle");
        };

        configured.restore_all
    }

    #[test]
    fn restore_all_start_requires_two_supported_complete_rows() {
        let one_wallet = configured_model_with_cloud_only(vec![cloud_only_wallet(
            "wallet-1",
            CloudBackupWalletStatus::DeletedFromDevice,
        )]);
        assert_eq!(restore_all_state(&one_wallet), CloudBackupRestoreAllState::NotShown);

        let two_wallets = configured_model_with_cloud_only(vec![
            cloud_only_wallet("wallet-1", CloudBackupWalletStatus::DeletedFromDevice),
            cloud_only_wallet("wallet-2", CloudBackupWalletStatus::DeletedFromDevice),
            cloud_only_wallet("unsupported", CloudBackupWalletStatus::UnsupportedVersion),
            cloud_only_wallet("unknown", CloudBackupWalletStatus::RemoteStateUnknown),
        ]);
        assert_eq!(
            restore_all_state(&two_wallets),
            CloudBackupRestoreAllState::StartAvailable { wallet_count: 2 },
        );
    }

    #[test]
    fn retained_restore_all_count_is_disabled_while_inventory_is_incomplete() {
        let wallets = vec![
            cloud_only_wallet("wallet-1", CloudBackupWalletStatus::DeletedFromDevice),
            cloud_only_wallet("wallet-2", CloudBackupWalletStatus::DeletedFromDevice),
        ];
        let mut checking = configured_model_with_cloud_only(wallets.clone());

        checking.apply_event(CloudBackupStateReducerEvent::DetailRefreshStarted).unwrap();
        assert_eq!(
            restore_all_state(&checking),
            CloudBackupRestoreAllState::StartDisabled { wallet_count: 2 },
        );

        let mut failed = configured_model_with_cloud_only(wallets);
        failed
            .apply_event(CloudBackupStateReducerEvent::DetailRefreshFailed {
                reason: CloudBackupInventoryIncompleteReason::ProviderUnavailable,
                error: "provider unavailable".into(),
            })
            .unwrap();
        assert_eq!(
            restore_all_state(&failed),
            CloudBackupRestoreAllState::StartDisabled { wallet_count: 2 },
        );
    }

    #[test]
    fn completed_cloud_only_fetch_restores_complete_detail_with_loaded_wallets() {
        let mut model = configured_model_with_cloud_only(vec![cloud_only_wallet(
            "stale-wallet",
            CloudBackupWalletStatus::DeletedFromDevice,
        )]);
        let loaded_wallets = vec![
            cloud_only_wallet("wallet-1", CloudBackupWalletStatus::DeletedFromDevice),
            cloud_only_wallet("wallet-2", CloudBackupWalletStatus::DeletedFromDevice),
        ];

        model
            .apply_event(CloudBackupStateReducerEvent::CloudOnlyStateResolved(
                CloudOnlyState::Loading,
            ))
            .unwrap();
        model
            .apply_event(CloudBackupStateReducerEvent::CloudOnlyStateResolved(
                CloudOnlyState::Loaded { wallets: loaded_wallets.clone() },
            ))
            .unwrap();

        let CloudBackupLifecycle::Configured(configured) = model.public_state().lifecycle else {
            panic!("expected configured lifecycle");
        };
        let CloudBackupDetailState::Complete { state } = configured.detail else {
            panic!("expected complete detail after cloud-only fetch");
        };

        assert_eq!(state.cloud_only, CloudOnlyState::Loaded { wallets: loaded_wallets });
        assert_eq!(
            configured.restore_all,
            CloudBackupRestoreAllState::StartAvailable { wallet_count: 2 },
        );
    }

    #[test]
    fn cloud_only_completion_does_not_finish_authoritative_detail_refresh() {
        let mut model = configured_model_with_cloud_only(vec![cloud_only_wallet(
            "stale-wallet",
            CloudBackupWalletStatus::DeletedFromDevice,
        )]);
        let loaded_wallets = vec![
            cloud_only_wallet("wallet-1", CloudBackupWalletStatus::DeletedFromDevice),
            cloud_only_wallet("wallet-2", CloudBackupWalletStatus::DeletedFromDevice),
        ];

        model.apply_event(CloudBackupStateReducerEvent::DetailRefreshStarted).unwrap();
        model
            .apply_event(CloudBackupStateReducerEvent::CloudOnlyStateResolved(
                CloudOnlyState::Loading,
            ))
            .unwrap();
        model
            .apply_event(CloudBackupStateReducerEvent::CloudOnlyStateResolved(
                CloudOnlyState::Loaded { wallets: loaded_wallets.clone() },
            ))
            .unwrap();

        let CloudBackupLifecycle::Configured(configured) = model.public_state().lifecycle else {
            panic!("expected configured lifecycle");
        };
        let CloudBackupDetailState::Checking { retained: Some(retained) } = configured.detail
        else {
            panic!("expected authoritative detail refresh to remain checking");
        };
        assert_eq!(retained.cloud_only, CloudOnlyState::Loaded { wallets: loaded_wallets });
        assert_eq!(
            configured.restore_all,
            CloudBackupRestoreAllState::StartDisabled { wallet_count: 2 },
        );
    }

    #[test]
    fn retained_detail_receives_terminal_operation_states() {
        let mut model = configured_model_with_cloud_only(vec![cloud_only_wallet(
            "wallet-1",
            CloudBackupWalletStatus::DeletedFromDevice,
        )]);
        let cloud_only_failure = CloudOnlyOperation::Failed { error: "restore failed".into() };
        let other_backups_failure =
            OtherBackupsOperation::Failed { error: "recovery failed".into() };

        model
            .apply_event(CloudBackupStateReducerEvent::CloudOnlyOperationResolved(
                CloudOnlyOperation::Operating { record_id: "wallet-1".into() },
            ))
            .unwrap();
        model
            .apply_event(CloudBackupStateReducerEvent::OtherBackupsOperationResolved(
                OtherBackupsOperation::Recovering,
            ))
            .unwrap();
        model.apply_event(CloudBackupStateReducerEvent::DetailRefreshStarted).unwrap();
        model
            .apply_event(CloudBackupStateReducerEvent::CloudOnlyOperationResolved(
                cloud_only_failure.clone(),
            ))
            .unwrap();
        model
            .apply_event(CloudBackupStateReducerEvent::OtherBackupsOperationResolved(
                other_backups_failure.clone(),
            ))
            .unwrap();

        let CloudBackupLifecycle::Configured(configured) = model.public_state().lifecycle else {
            panic!("expected configured lifecycle");
        };
        let CloudBackupDetailState::Checking { retained: Some(retained) } = configured.detail
        else {
            panic!("expected checking detail with retained operation states");
        };
        assert_eq!(retained.cloud_only_operation, cloud_only_failure);
        assert_eq!(retained.other_backups_operation, other_backups_failure);

        model
            .apply_event(CloudBackupStateReducerEvent::DetailRefreshFailed {
                reason: CloudBackupInventoryIncompleteReason::Offline,
                error: "provider offline".into(),
            })
            .unwrap();
        model
            .apply_event(CloudBackupStateReducerEvent::CloudOnlyOperationResolved(
                CloudOnlyOperation::Idle,
            ))
            .unwrap();
        model
            .apply_event(CloudBackupStateReducerEvent::OtherBackupsOperationResolved(
                OtherBackupsOperation::Deleted,
            ))
            .unwrap();

        let CloudBackupLifecycle::Configured(configured) = model.public_state().lifecycle else {
            panic!("expected configured lifecycle");
        };
        let CloudBackupDetailState::Failed { retained: Some(retained), .. } = configured.detail
        else {
            panic!("expected failed detail with retained operation states");
        };
        assert_eq!(retained.cloud_only_operation, CloudOnlyOperation::Idle);
        assert_eq!(retained.other_backups_operation, OtherBackupsOperation::Deleted);
    }

    #[test]
    fn conflicting_exclusive_operation_disables_restore_all_start() {
        let mut model = configured_model_with_cloud_only(vec![
            cloud_only_wallet("wallet-1", CloudBackupWalletStatus::DeletedFromDevice),
            cloud_only_wallet("wallet-2", CloudBackupWalletStatus::DeletedFromDevice),
        ]);

        model
            .apply_event(operation_event(CloudBackupExclusiveOperation::RestoreCloudWallet, 1))
            .unwrap();

        assert_eq!(
            restore_all_state(&model),
            CloudBackupRestoreAllState::StartDisabled { wallet_count: 2 },
        );
    }

    #[test]
    fn restore_all_retry_allows_one_remaining_wallet_only_when_complete() {
        let detail = test_detail(1);
        let mut model = configured_model_with_cloud_only(vec![cloud_only_wallet(
            "wallet-1",
            CloudBackupWalletStatus::DeletedFromDevice,
        )]);

        model.apply_event(CloudBackupStateReducerEvent::RestoreAllRetryRequired).unwrap();
        assert_eq!(
            restore_all_state(&model),
            CloudBackupRestoreAllState::RetryAvailable { wallet_count: 1 },
        );

        model.apply_event(CloudBackupStateReducerEvent::DetailRefreshStarted).unwrap();
        assert_eq!(
            restore_all_state(&model),
            CloudBackupRestoreAllState::RetryDisabled { wallet_count: 1 },
        );

        model
            .apply_event(CloudBackupStateReducerEvent::DetailRefreshApplied {
                detail: Some(detail),
                reset_cloud_only: false,
            })
            .unwrap();
        assert_eq!(
            restore_all_state(&model),
            CloudBackupRestoreAllState::RetryAvailable { wallet_count: 1 },
        );
    }

    #[test]
    fn restore_all_runtime_events_project_progress_and_cancellation() {
        let mut model = configured_model_with_cloud_only(vec![
            cloud_only_wallet("wallet-1", CloudBackupWalletStatus::DeletedFromDevice),
            cloud_only_wallet("wallet-2", CloudBackupWalletStatus::DeletedFromDevice),
        ]);
        let claim = CloudBackupExclusiveOperationClaim::new(
            CloudBackupExclusiveOperation::RestoreAllCloudWallets,
            1,
        );
        model.apply_event(CloudBackupStateReducerEvent::ExclusiveOperationStarted(claim)).unwrap();

        model
            .apply_event(CloudBackupStateReducerEvent::RestoreAllStarted { claim, total: 2 })
            .unwrap();
        model
            .apply_event(CloudBackupStateReducerEvent::RestoreAllProgressed {
                claim,
                completed: 1,
                current_wallet_name: Some("Savings".into()),
            })
            .unwrap();
        model
            .apply_event(CloudBackupStateReducerEvent::RestoreAllProgressed {
                claim,
                completed: 0,
                current_wallet_name: Some("Regressed".into()),
            })
            .unwrap();
        let stale_claim = CloudBackupExclusiveOperationClaim::new(
            CloudBackupExclusiveOperation::RestoreAllCloudWallets,
            0,
        );
        model
            .apply_event(CloudBackupStateReducerEvent::RestoreAllProgressed {
                claim: stale_claim,
                completed: 2,
                current_wallet_name: Some("Stale".into()),
            })
            .unwrap();
        model
            .apply_event(CloudBackupStateReducerEvent::RestoreAllCancellationRequested(stale_claim))
            .unwrap();
        model
            .apply_event(CloudBackupStateReducerEvent::RestoreAllCancellationRequested(claim))
            .unwrap();

        assert_eq!(
            restore_all_state(&model),
            CloudBackupRestoreAllState::Running {
                completed: 1,
                total: 2,
                current_wallet_name: Some("Savings".into()),
                cancellation_requested: true,
            },
        );

        model
            .apply_event(CloudBackupStateReducerEvent::RestoreAllFinished {
                claim,
                retry_remaining: false,
            })
            .unwrap();
        assert_eq!(
            restore_all_state(&model),
            CloudBackupRestoreAllState::StartAvailable { wallet_count: 2 },
        );
    }

    #[test]
    fn detail_refresh_failure_retains_last_known_rows() {
        let mut model = CloudBackupStateReducer {
            state: configured_state(
                CloudBackupVerificationState::NotVerified,
                CloudSyncHealth::Unknown,
            ),
        };
        let detail = test_detail(2);

        model
            .apply_event(CloudBackupStateReducerEvent::DetailRefreshApplied {
                detail: Some(detail.clone()),
                reset_cloud_only: false,
            })
            .unwrap();
        model
            .apply_event(CloudBackupStateReducerEvent::DetailRefreshFailed {
                reason: CloudBackupInventoryIncompleteReason::ProviderUnavailable,
                error: "iCloud unavailable".into(),
            })
            .unwrap();

        let CloudBackupLifecycle::Configured(configured) = model.public_state().lifecycle else {
            panic!("expected configured lifecycle");
        };
        let CloudBackupDetailState::Failed { reason, error, retained: Some(retained) } =
            configured.detail
        else {
            panic!("expected retained failed detail");
        };

        assert_eq!(reason, CloudBackupInventoryIncompleteReason::ProviderUnavailable);
        assert_eq!(error, "iCloud unavailable");
        assert_eq!(retained.detail, detail);
        assert_eq!(model.state.detail(), Some(detail));
    }

    #[test]
    fn checking_retains_rows_but_disables_completeness_dependent_actions() {
        let mut model = CloudBackupStateReducer {
            state: configured_state(
                CloudBackupVerificationState::NotVerified,
                CloudSyncHealth::Unknown,
            ),
        };
        let detail = test_detail(2);

        model
            .apply_event(CloudBackupStateReducerEvent::DetailRefreshApplied {
                detail: Some(detail.clone()),
                reset_cloud_only: false,
            })
            .unwrap();
        assert!(model.detail_inventory_is_complete());

        model.apply_event(CloudBackupStateReducerEvent::DetailRefreshStarted).unwrap();

        let CloudBackupLifecycle::Configured(configured) = model.public_state().lifecycle else {
            panic!("expected configured lifecycle");
        };
        let CloudBackupDetailState::Checking { retained: Some(retained) } = configured.detail
        else {
            panic!("expected checking detail with retained rows");
        };

        assert_eq!(retained.detail, detail);
        assert_eq!(model.state.detail(), Some(detail));
        assert!(!model.detail_inventory_is_complete());
    }

    #[test]
    fn provisional_detail_is_visible_without_becoming_complete() {
        let mut model = CloudBackupStateReducer {
            state: configured_state(
                CloudBackupVerificationState::NotVerified,
                CloudSyncHealth::Unknown,
            ),
        };
        let provisional = test_detail(1);

        model
            .apply_event(CloudBackupStateReducerEvent::DetailRefreshProvisional(
                provisional.clone(),
            ))
            .unwrap();

        let CloudBackupLifecycle::Configured(configured) = model.public_state().lifecycle else {
            panic!("expected configured lifecycle");
        };
        let CloudBackupDetailState::Checking { retained: Some(retained) } = configured.detail
        else {
            panic!("expected provisional checking detail");
        };

        assert_eq!(retained.detail, provisional);
        assert!(!model.detail_inventory_is_complete());
    }

    #[test]
    fn smaller_provisional_snapshot_does_not_drop_retained_rows() {
        let mut model = CloudBackupStateReducer {
            state: configured_state(
                CloudBackupVerificationState::NotVerified,
                CloudSyncHealth::Unknown,
            ),
        };
        let retained = test_detail(3);

        model
            .apply_event(CloudBackupStateReducerEvent::DetailRefreshApplied {
                detail: Some(retained.clone()),
                reset_cloud_only: false,
            })
            .unwrap();
        model
            .apply_event(CloudBackupStateReducerEvent::DetailRefreshProvisional(test_detail(0)))
            .unwrap();

        assert_eq!(model.state.detail(), Some(retained));
        assert!(!model.detail_inventory_is_complete());
    }

    #[test]
    fn initial_detail_refresh_failure_never_projects_confirmed_zero() {
        let mut model = CloudBackupStateReducer {
            state: configured_state(
                CloudBackupVerificationState::NotVerified,
                CloudSyncHealth::Unknown,
            ),
        };

        model
            .apply_event(CloudBackupStateReducerEvent::DetailRefreshFailed {
                reason: CloudBackupInventoryIncompleteReason::Offline,
                error: "Drive unavailable".into(),
            })
            .unwrap();

        let CloudBackupLifecycle::Configured(configured) = model.public_state().lifecycle else {
            panic!("expected configured lifecycle");
        };
        assert_eq!(
            configured.detail,
            CloudBackupDetailState::Failed {
                reason: CloudBackupInventoryIncompleteReason::Offline,
                error: "Drive unavailable".into(),
                retained: None,
            }
        );
        assert_eq!(model.state.detail(), None);
    }

    #[test]
    fn disabled_projects_disabled_lifecycle() {
        let model = CloudBackupStateReducer::default();

        assert_eq!(model.public_state().lifecycle, CloudBackupLifecycle::Disabled);
        assert_eq!(
            model.public_state().settings_row_status,
            CloudBackupSettingsRowStatus::Disabled
        );
    }

    #[test]
    fn settings_row_status_projects_sync_health() {
        let state = configured_state(
            CloudBackupVerificationState::NotVerified,
            CloudSyncHealth::AuthorizationRequired("wrong account".into()),
        );

        assert_eq!(
            state.settings_row_status(),
            CloudBackupSettingsRowStatus::AuthorizationRequired("wrong account".into())
        );
    }

    #[test]
    fn settings_row_status_projects_active_sync_health() {
        let state = configured_state(
            CloudBackupVerificationState::NotVerified,
            CloudSyncHealth::AllUploaded,
        );

        assert_eq!(state.settings_row_status(), CloudBackupSettingsRowStatus::Active);
    }

    #[test]
    fn settings_row_status_projects_failed_sync_health() {
        let state = configured_state(
            CloudBackupVerificationState::NotVerified,
            CloudSyncHealth::Failed("upload failed".into()),
        );

        assert_eq!(
            state.settings_row_status(),
            CloudBackupSettingsRowStatus::Error("upload failed".into())
        );
    }

    #[test]
    fn settings_row_status_prioritizes_verification_before_sync_health() {
        let state =
            configured_state(CloudBackupVerificationState::Required, CloudSyncHealth::AllUploaded);

        assert_eq!(state.settings_row_status(), CloudBackupSettingsRowStatus::Unverified);
    }

    #[test]
    fn settings_row_status_projects_pending_upload_confirmation() {
        let mut state = configured_state(
            CloudBackupVerificationState::NotVerified,
            CloudSyncHealth::AllUploaded,
        );

        state.configured.pending_upload_verification = PendingUploadVerificationState::Confirming;

        assert_eq!(state.settings_row_status(), CloudBackupSettingsRowStatus::Confirming);

        state.configured.pending_upload_verification =
            PendingUploadVerificationState::BlockedOnAuthorization;

        assert_eq!(state.settings_row_status(), CloudBackupSettingsRowStatus::Confirming);
    }

    #[test]
    fn settings_row_status_recommends_stale_verification() {
        let state = configured_state(
            CloudBackupVerificationState::Verified { report: None, last_verified_at: Some(0) },
            CloudSyncHealth::AllUploaded,
        );

        assert_eq!(
            state.settings_row_status(),
            CloudBackupSettingsRowStatus::VerificationRecommended
        );
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
            effect_lifecycle(&effects),
            Some(&CloudBackupLifecycle::Enabling(CloudBackupEnableFlow::DiscoveringExistingBackup)),
        );
        assert_eq!(
            effects.lifecycle.as_ref().map(|effect| effect.settings_row_status.clone()),
            Some(CloudBackupSettingsRowStatus::SettingUp)
        );
    }

    #[test]
    fn enable_completion_emits_context_after_operation_finishes() {
        let context = CloudBackupEnableContext::settings_manual();
        let claim =
            CloudBackupExclusiveOperationClaim::new(CloudBackupExclusiveOperation::Enable, 1);
        let mut model = CloudBackupStateReducer::default();

        model.apply_event(CloudBackupStateReducerEvent::ExclusiveOperationStarted(claim)).unwrap();
        model.apply_event(CloudBackupStateReducerEvent::EnableContextStarted(context)).unwrap();

        let configured_effects = model
            .apply_event(CloudBackupStateReducerEvent::RuntimeStatusReconciled(
                CloudBackupStatus::Enabled,
            ))
            .unwrap();

        assert_eq!(configured_effects.enable_completed, None);
        assert!(matches!(
            configured_effects.lifecycle,
            Some(effect) if matches!(effect.lifecycle, CloudBackupLifecycle::Configured(_))
        ));
        assert_eq!(model.status(), CloudBackupStatus::Enabling);

        let finished_effects = model
            .apply_event(CloudBackupStateReducerEvent::ExclusiveOperationFinished(claim))
            .unwrap();

        assert_eq!(finished_effects.enable_completed, Some(context));
        assert!(finished_effects.status_changed);
        assert_eq!(model.status(), CloudBackupStatus::Enabled);

        let repeated_effects = model
            .apply_event(CloudBackupStateReducerEvent::ExclusiveOperationFinished(claim))
            .unwrap();

        assert_eq!(repeated_effects.enable_completed, None);
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
            assert_eq!(effect_lifecycle(&effects), Some(&expected_lifecycle));
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
        assert_eq!(effect_lifecycle(&effects), Some(&CloudBackupLifecycle::Disabled));
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
            effect_lifecycle(&effects),
            Some(&CloudBackupLifecycle::Configured(CloudBackupConfiguredState {
                passkey: CloudBackupPasskeyState::Available,
                verification: CloudBackupVerificationState::AwaitingUploadConfirmation,
                sync: CloudBackupSyncState::Blocked(
                    PENDING_UPLOAD_AUTHORIZATION_BLOCKED_MESSAGE.into(),
                ),
                destructive_operation: CloudBackupDestructiveOperationState::Idle,
                detail: CloudBackupDetailState::NotLoaded,
                restore_all: CloudBackupRestoreAllState::NotShown,
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

        assert!(matches!(
            runtime_effects.lifecycle,
            Some(effect) if matches!(effect.lifecycle, CloudBackupLifecycle::Configured(_))
        ));
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
        assert!(matches!(
            effects.lifecycle,
            Some(effect) if matches!(effect.lifecycle, CloudBackupLifecycle::Configured(_))
        ));
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
            effect_lifecycle(&effects),
            Some(
                &CloudBackupLifecycle::Enabling(CloudBackupEnableFlow::DiscoveringExistingBackup,)
            ),
        );
    }

    #[test]
    fn existing_passkey_only_prompt_rejects_create_new() {
        let context = CloudBackupEnableContext::settings_manual();
        let intent = CloudBackupPasskeyChoiceIntent::EnableExistingPasskeyOnly(context, None);
        let mut model = CloudBackupStateReducer::default();

        model
            .apply_event(CloudBackupStateReducerEvent::PasskeyChoicePromptSet(intent.clone()))
            .unwrap();

        let expected_lifecycle = CloudBackupLifecycle::Enabling(
            CloudBackupEnableFlow::AwaitingPasskeyChoice(intent.clone()),
        );
        assert_eq!(model.snapshot().root_prompt, CloudBackupRootPrompt::PasskeyChoice(intent),);
        assert_eq!(model.public_state().lifecycle, expected_lifecycle);
        assert!(model.is_awaiting_enable_prompt());

        let (accepted, effects) =
            model.accept_enable_prompt(CloudBackupEnablePromptChoice::CreateNew);

        assert_eq!(accepted, None);
        assert_eq!(effects, CloudBackupStateReducerEffects::default());
        assert_eq!(model.public_state().lifecycle, expected_lifecycle);

        let (accepted, _) = model.accept_enable_prompt(CloudBackupEnablePromptChoice::UseExisting);

        assert_eq!(accepted, Some(CloudBackupAcceptedEnablePrompt::Enable(context)));
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
            effect_lifecycle(&effects),
            Some(
                &CloudBackupLifecycle::Enabling(CloudBackupEnableFlow::DiscoveringExistingBackup,)
            ),
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
        assert!(matches!(
            effects.lifecycle,
            Some(effect) if matches!(effect.lifecycle, CloudBackupLifecycle::Configured(_))
        ));
    }
}
