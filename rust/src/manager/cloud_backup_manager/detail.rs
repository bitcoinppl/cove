use act_zero::send;
use serde::{Deserialize, Serialize};
use tracing::error;

use super::verify::coordinator::CloudBackupVerificationCoordinator;
use super::{
    CLOUD_BACKUP_MANAGER, CloudBackupDetail, CloudBackupExclusiveOperationClaim,
    CloudBackupInventoryAuthority, CloudBackupInventoryIncompleteReason, CloudBackupListedDetail,
    CloudBackupManagerAction, CloudBackupOtherBackupsState, CloudBackupPasskeyChoiceIntent,
    CloudBackupRestoreAllState, CloudBackupRestoreFlow, CloudBackupStateReducerEvent,
    CloudBackupWalletItem, CloudBackupWalletRestoreFailure, CloudBackupWalletStatus,
    CloudOnlyFetch, CloudOnlyListingUpdate, DeepVerificationFailure, DeepVerificationReport,
    DeepVerificationResult, OtherBackupsOperation, RustCloudBackupManager,
};

type Action = CloudBackupManagerAction;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum RecoveryAction {
    RecreateManifest,
    ReinitializeBackup,
    RepairPasskey,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum VerificationState {
    Idle,
    Verifying,
    Verified(DeepVerificationReport),
    NeedsAttention(DeepVerificationReport),
    PasskeyConfirmed,
    Failed(DeepVerificationFailure),
    Cancelled,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum CloudBackupVerificationReason {
    BackupChanged,
}

#[derive(Debug, Clone, Copy, Hash, PartialEq, Eq, Serialize, Deserialize, uniffi::Enum)]
pub enum CloudBackupVerificationSource {
    RootPrompt,
    Settings,
    CloudBackupDetail,
    Onboarding,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum CloudBackupVerificationPresentation {
    Hidden {
        source: Option<CloudBackupVerificationSource>,
    },
    /// The verification sheet is only for an unanswered user decision
    NeedsDecision {
        reason: CloudBackupVerificationReason,
        source: CloudBackupVerificationSource,
    },
    /// Native passkey UI may appear while this state is active
    ManualVerifying {
        source: CloudBackupVerificationSource,
    },
    BackgroundConfirming(CloudBackupVerificationSource),
    BackgroundBlockedOnAuthorization(CloudBackupVerificationSource),
    /// Completion feedback should match the source instead of reopening the sheet
    Completed {
        source: CloudBackupVerificationSource,
    },
    /// Failure is a result, not another request to show the decision sheet
    Failed {
        source: CloudBackupVerificationSource,
        message: String,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PendingUploadVerificationState {
    Idle,
    Confirming,
    BlockedOnAuthorization,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum SyncState {
    Idle,
    Syncing,
    Failed(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum RecoveryState {
    Idle,
    Recovering(RecoveryAction),
    Failed { action: RecoveryAction, error: String },
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum CloudOnlyState {
    NotFetched,
    Loading,
    Loaded {
        wallets: Vec<CloudBackupWalletItem>,
        /// Some listed backups could not be read, so `wallets` may be missing cloud-only wallets
        incomplete: bool,
    },
    Failed {
        error: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum CloudOnlyOperation {
    Idle,
    Operating { record_id: String },
    Warning { message: String, error: String },
    Failed { error: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum CloudBackupCloudOnlyFetchOutcome {
    Reset,
    Started,
    Loaded(CloudOnlyFetch),
    Failed(String),
}

/// Whether a loaded fetch result became the cloud-only cache or was dropped as stale
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CloudOnlyFetchApplication {
    Applied,
    Discarded,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CloudBackupCloudOnlyOperationWarning {
    pub(crate) message: String,
    pub(crate) error: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum CloudBackupCloudOnlyWalletOutcome {
    Started {
        record_id: String,
    },
    Restored {
        record_id: String,
        warning: Option<CloudBackupCloudOnlyOperationWarning>,
    },
    /// A local wallet holds everything the record can recover, so it stays as a deletable row
    ///
    /// `listing_generation` is the generation observed before the restore read the local
    /// wallets, so a later local wallet change can discard the result
    AlreadyOnDevice {
        record_id: String,
        listing_generation: u64,
    },
    Deleted {
        record_id: String,
    },
    RestoreFailed {
        record_id: String,
        error: String,
    },
    Failed(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum CloudBackupOtherBackupsOutcome {
    Idle,
    Recovering,
    Recovered { wallets_restored: u32, wallets_failed: u32, failed_wallet_errors: Vec<String> },
    Deleting,
    Deleted,
    Failed(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum CloudBackupRestoreOutcome {
    ProgressCleared,
    ProgressReported(CloudBackupRestoreFlow),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum CloudBackupDetailOutcome {
    Cleared,
    Checking,
    Provisional(CloudBackupDetail),
    Refreshed(CloudBackupListedDetail),
    RefreshedWithAuthority {
        listed: CloudBackupListedDetail,
        authority: CloudBackupInventoryAuthority,
    },
    /// Detail from a path whose report carries no listing, so loaded cloud-only rows cannot be
    /// matched to a listing and are reset
    RefreshedWithoutListing(CloudBackupDetail),
    Failed {
        reason: CloudBackupInventoryIncompleteReason,
        error: String,
    },
}

#[uniffi::export]
impl RustCloudBackupManager {
    #[uniffi::method]
    pub fn dispatch(&self, action: Action) {
        use Action as A;
        match action {
            A::EnableCloudBackup(context) => {
                self.enable_cloud_backup(context);
            }
            A::EnableCloudBackupForceNew(context) => {
                self.enable_cloud_backup_force_new(context);
            }
            A::EnableCloudBackupNoDiscovery(context) => {
                self.enable_cloud_backup_no_discovery(context);
            }
            A::ConfirmSavedPasskey => {
                self.confirm_saved_passkey();
            }
            A::DiscardPendingEnableCloudBackup => {
                self.discard_pending_enable_cloud_backup();
            }
            A::ConfirmPendingEnableCleanup => {
                send!(self.supervisor.confirm_pending_enable_cleanup());
            }
            A::DismissPasskeyChoicePrompt => self.clear_passkey_choice_prompt(),
            A::DismissMissingPasskeyReminder => self.dismiss_missing_passkey_prompt(),
            A::RestoreFromCloudBackup => self.restore_from_cloud_backup(),
            A::CancelRestore => self.cancel_restore(),
            A::StartVerification(source) => self.start_verification(source),
            A::StartVerificationDiscoverable(source) => {
                self.start_verification_discoverable(source);
            }
            A::DismissVerificationPrompt => self.dismiss_verification_prompt(),
            A::RecreateManifest => {
                if self.detail_inventory_is_complete() {
                    CLOUD_BACKUP_MANAGER.clone().spawn_recovery(RecoveryAction::RecreateManifest);
                }
            }
            A::ReinitializeBackup => {
                if self.detail_inventory_is_complete() {
                    CLOUD_BACKUP_MANAGER.clone().spawn_recovery(RecoveryAction::ReinitializeBackup);
                }
            }
            A::RepairPasskey => {
                self.clear_passkey_choice_prompt();
                CLOUD_BACKUP_MANAGER.clone().spawn_repair_passkey(false);
            }
            A::RepairPasskeyNoDiscovery => {
                self.clear_passkey_choice_prompt();
                CLOUD_BACKUP_MANAGER.clone().spawn_repair_passkey(true);
            }
            A::SyncUnsynced => CLOUD_BACKUP_MANAGER.clone().spawn_sync(),
            A::RestoreCloudWallet(record_id) => {
                if self.detail_inventory_is_ready() {
                    CLOUD_BACKUP_MANAGER.clone().spawn_restore_cloud_wallet(record_id);
                }
            }
            A::StartRestoreAll => {
                if matches!(
                    self.projected_restore_all_state(),
                    CloudBackupRestoreAllState::StartAvailable { .. }
                ) {
                    CLOUD_BACKUP_MANAGER.clone().spawn_restore_all(false);
                }
            }
            A::RetryRestoreAllRemaining => {
                if matches!(
                    self.projected_restore_all_state(),
                    CloudBackupRestoreAllState::RetryAvailable { .. }
                ) {
                    CLOUD_BACKUP_MANAGER.clone().spawn_restore_all(true);
                }
            }
            A::CancelRestoreAll => {
                if matches!(
                    self.projected_restore_all_state(),
                    CloudBackupRestoreAllState::Running { .. }
                ) {
                    CLOUD_BACKUP_MANAGER.clone().cancel_restore_all();
                }
            }
            A::DeleteCloudWallet(record_id) => {
                if self.detail_inventory_is_ready() {
                    CLOUD_BACKUP_MANAGER.clone().spawn_delete_cloud_wallet(record_id);
                }
            }
            A::DeleteUndecryptableWalletBackups => {
                CLOUD_BACKUP_MANAGER.clone().spawn_delete_undecryptable_wallet_backups();
            }
            A::RecoverOtherBackups => {
                if self.other_backups_inventory_is_ready() {
                    CLOUD_BACKUP_MANAGER.clone().spawn_recover_other_backups();
                }
            }
            A::DeleteOtherBackups => {
                if self.other_backups_inventory_is_ready() {
                    CLOUD_BACKUP_MANAGER.clone().spawn_delete_other_backups();
                }
            }
            A::DisableCloudBackup => {
                if self.detail_inventory_is_complete() {
                    CLOUD_BACKUP_MANAGER.disable_cloud_backup();
                }
            }
            A::KeepCloudBackupEnabled => CLOUD_BACKUP_MANAGER.keep_cloud_backup_enabled(),
            A::RefreshDetail => CLOUD_BACKUP_MANAGER.clone().spawn_refresh_detail(),
            A::EnterDetail => CLOUD_BACKUP_MANAGER.clone().spawn_enter_detail(),
            A::PromptEnablePasskeyChoice(context) => {
                self.present_passkey_choice_prompt(CloudBackupPasskeyChoiceIntent::Enable(
                    context, None,
                ));
            }
            A::AcceptEnablePrompt(choice) => self.accept_enable_prompt(choice),
        }
    }
}

impl RustCloudBackupManager {
    pub(crate) fn detail_inventory_is_complete(&self) -> bool {
        self.state.read().detail_inventory_is_complete()
    }

    pub(crate) fn detail_inventory_is_ready(&self) -> bool {
        self.state.read().detail_inventory_is_ready()
    }

    fn other_backups_inventory_is_ready(&self) -> bool {
        matches!(
            self.state.read().other_backups_state(),
            CloudBackupOtherBackupsState::Loaded { .. }
        )
    }

    fn start_verification(&self, source: CloudBackupVerificationSource) {
        if let Err(error) = self.dismiss_verification_prompt_impl() {
            error!("Failed to dismiss verification prompt before verification: {error}");
        }

        if self.has_pending_cloud_upload_verification() {
            self.apply_verification_effect(
                CloudBackupVerificationCoordinator::begin_background_confirmation(source),
            );
            self.resume_pending_cloud_upload_verification();
            return;
        }

        self.apply_verification_effect(
            CloudBackupVerificationCoordinator::begin_manual_presentation(source),
        );
        send!(self.supervisor.start_verification(false));
    }

    fn start_verification_discoverable(&self, source: CloudBackupVerificationSource) {
        if let Err(error) = self.dismiss_verification_prompt_impl() {
            error!("Failed to dismiss verification prompt before verification: {error}");
        }
        self.apply_verification_effect(
            CloudBackupVerificationCoordinator::begin_manual_presentation(source),
        );
        send!(self.supervisor.start_verification(true));
    }

    fn dismiss_verification_prompt(&self) {
        if let Err(error) = self.dismiss_verification_prompt_impl() {
            error!("Failed to dismiss verification prompt: {error}");
        }
        self.apply_verification_effect(CloudBackupVerificationCoordinator::dismiss_decision(
            self.current_verification_source(),
        ));
    }

    fn spawn_recovery(self: std::sync::Arc<Self>, action: RecoveryAction) {
        send!(self.supervisor.start_recovery_operation(action));
    }

    fn spawn_repair_passkey(self: std::sync::Arc<Self>, no_discovery: bool) {
        send!(self.supervisor.start_repair_passkey_operation(no_discovery));
    }

    fn spawn_sync(self: std::sync::Arc<Self>) {
        send!(self.supervisor.start_sync_operation());
    }

    fn spawn_restore_cloud_wallet(self: std::sync::Arc<Self>, record_id: super::RecordId) {
        send!(self.supervisor.start_restore_cloud_wallet_operation(record_id.into()));
    }

    fn spawn_restore_all(self: std::sync::Arc<Self>, retry: bool) {
        send!(self.supervisor.start_restore_all_operation(retry));
    }

    fn cancel_restore_all(self: std::sync::Arc<Self>) {
        send!(self.supervisor.cancel_restore_all_operation());
    }

    fn spawn_delete_cloud_wallet(self: std::sync::Arc<Self>, record_id: super::RecordId) {
        send!(self.supervisor.start_delete_cloud_wallet_operation(record_id.into()));
    }

    fn spawn_delete_undecryptable_wallet_backups(self: std::sync::Arc<Self>) {
        send!(self.supervisor.start_delete_undecryptable_wallet_backups_operation());
    }

    fn spawn_recover_other_backups(self: std::sync::Arc<Self>) {
        if !matches!(
            self.state.read().other_backups_operation(),
            OtherBackupsOperation::Idle
                | OtherBackupsOperation::Recovered { .. }
                | OtherBackupsOperation::Deleted
                | OtherBackupsOperation::Failed { .. }
        ) {
            return;
        }

        send!(self.supervisor.start_recover_other_backups_operation());
    }

    fn spawn_delete_other_backups(self: std::sync::Arc<Self>) {
        if !matches!(
            self.state.read().other_backups_operation(),
            OtherBackupsOperation::Idle
                | OtherBackupsOperation::Recovered { .. }
                | OtherBackupsOperation::Deleted
                | OtherBackupsOperation::Failed { .. }
        ) {
            return;
        }

        send!(self.supervisor.start_delete_other_backups_operation());
    }

    fn spawn_refresh_detail(self: std::sync::Arc<Self>) {
        send!(self.supervisor.start_refresh_detail());
    }

    fn spawn_enter_detail(self: std::sync::Arc<Self>) {
        send!(self.supervisor.start_enter_detail());
    }

    fn confirm_saved_passkey(&self) {
        send!(self.supervisor.confirm_saved_passkey());
    }

    pub(crate) fn handle_deep_verification_result(&self, result: DeepVerificationResult) {
        self.apply_deep_verification_result(result);
    }

    pub(crate) fn apply_deep_verification_result(&self, result: DeepVerificationResult) {
        match result {
            DeepVerificationResult::Verified(report) => {
                self.apply_verified_report(report);
            }
            DeepVerificationResult::NeedsAttention(report) => {
                self.apply_needs_attention_report(report);
            }
            DeepVerificationResult::AwaitingUploadConfirmation(report) => {
                if let Some(detail) = report.detail {
                    self.apply_detail_outcome(CloudBackupDetailOutcome::RefreshedWithoutListing(
                        detail,
                    ));
                }
                self.apply_verification_effect(
                    CloudBackupVerificationCoordinator::begin_background_confirmation(
                        self.current_verification_source(),
                    ),
                );
            }
            DeepVerificationResult::PasskeyConfirmed(detail) => {
                if let Some(detail) = detail {
                    self.apply_detail_outcome(CloudBackupDetailOutcome::RefreshedWithoutListing(
                        detail,
                    ));
                }
                self.apply_verification_state(VerificationState::PasskeyConfirmed);
            }
            DeepVerificationResult::PasskeyMissing(detail) => {
                if let Some(detail) = detail {
                    self.apply_detail_outcome(CloudBackupDetailOutcome::RefreshedWithoutListing(
                        detail,
                    ));
                }
                self.apply_verification_state(VerificationState::Idle);
                self.apply_recovery_state(RecoveryState::Idle);
            }
            DeepVerificationResult::UserCancelled(detail) => {
                if let Some(detail) = detail {
                    self.apply_detail_outcome(CloudBackupDetailOutcome::RefreshedWithoutListing(
                        detail,
                    ));
                }
                self.apply_verification_state(VerificationState::Cancelled);
            }
            DeepVerificationResult::NotEnabled => {
                self.apply_verification_state(VerificationState::Idle);
                self.apply_recovery_state(RecoveryState::Idle);
            }
            DeepVerificationResult::Failed(failure) => {
                self.apply_failed_verification(failure);
            }
        }
    }
}

impl RustCloudBackupManager {
    /// Applies a detail refresh, keeping loaded cloud-only rows only when they are a complete read
    /// of exactly the wallets the listing found that are not local
    pub(crate) fn apply_detail_outcome(&self, outcome: CloudBackupDetailOutcome) {
        let (detail, authority, cloud_only_listing) = match outcome {
            CloudBackupDetailOutcome::Checking => {
                self.apply_model_event(CloudBackupStateReducerEvent::DetailRefreshStarted);
                return;
            }
            CloudBackupDetailOutcome::Provisional(detail) => {
                self.apply_model_event(CloudBackupStateReducerEvent::DetailRefreshProvisional(
                    detail,
                ));
                return;
            }
            CloudBackupDetailOutcome::Failed { reason, error } => {
                self.apply_model_event(CloudBackupStateReducerEvent::DetailRefreshFailed {
                    reason,
                    error,
                });
                return;
            }
            CloudBackupDetailOutcome::Cleared => {
                self.apply_model_event(CloudBackupStateReducerEvent::DetailCleared);
                return;
            }
            CloudBackupDetailOutcome::Refreshed(listed) => (
                listed.detail,
                CloudBackupInventoryAuthority::ProviderConfirmed,
                CloudOnlyListingUpdate::Listed(listed.cloud_only_record_ids),
            ),
            CloudBackupDetailOutcome::RefreshedWithAuthority { listed, authority } => (
                listed.detail,
                authority,
                CloudOnlyListingUpdate::Listed(listed.cloud_only_record_ids),
            ),
            CloudBackupDetailOutcome::RefreshedWithoutListing(detail) => (
                detail,
                CloudBackupInventoryAuthority::ProviderConfirmed,
                CloudOnlyListingUpdate::Unlisted,
            ),
        };

        // the reducer decides whether loaded rows survive and asks for a refetch when they do not
        self.apply_model_event(CloudBackupStateReducerEvent::DetailRefreshApplied {
            detail: (detail, authority),
            cloud_only_listing,
        });
    }

    fn apply_cloud_only_state(&self, cloud_only: CloudOnlyState) {
        self.apply_model_event(CloudBackupStateReducerEvent::CloudOnlyStateResolved(cloud_only));
    }

    pub(crate) fn apply_local_wallets_changed(&self) {
        self.apply_model_event(CloudBackupStateReducerEvent::CloudOnlyLocalWalletsChanged);
    }

    /// Applies a fetch outcome, reporting whether a `Loaded` result was dropped as stale
    pub(crate) fn apply_cloud_only_fetch_outcome(
        &self,
        outcome: CloudBackupCloudOnlyFetchOutcome,
    ) -> CloudOnlyFetchApplication {
        match outcome {
            CloudBackupCloudOnlyFetchOutcome::Reset => {
                self.apply_model_event(CloudBackupStateReducerEvent::CloudOnlyInventoryInvalidated);
                self.apply_cloud_only_state(CloudOnlyState::NotFetched);
                self.apply_cloud_only_operation(CloudOnlyOperation::Idle);
            }
            CloudBackupCloudOnlyFetchOutcome::Started => {
                self.apply_cloud_only_state(CloudOnlyState::Loading);
                self.apply_cloud_only_operation(CloudOnlyOperation::Idle);
            }
            CloudBackupCloudOnlyFetchOutcome::Loaded(fetch) => {
                let effects = self
                    .state
                    .write()
                    .apply_event(CloudBackupStateReducerEvent::CloudOnlyFetchLoaded(fetch));
                let discarded = effects.cloud_only_refetch_needed;
                self.send_model_effects(effects);

                return if discarded {
                    CloudOnlyFetchApplication::Discarded
                } else {
                    CloudOnlyFetchApplication::Applied
                };
            }
            CloudBackupCloudOnlyFetchOutcome::Failed(error) => {
                self.apply_cloud_only_state(CloudOnlyState::Failed { error });
            }
        }

        CloudOnlyFetchApplication::Applied
    }

    pub(crate) fn apply_cloud_only_operation(&self, cloud_only_operation: CloudOnlyOperation) {
        self.apply_model_event(CloudBackupStateReducerEvent::CloudOnlyOperationResolved(
            cloud_only_operation,
        ));
    }

    pub(crate) fn apply_cloud_only_wallet_outcome(
        &self,
        outcome: CloudBackupCloudOnlyWalletOutcome,
    ) {
        match outcome {
            CloudBackupCloudOnlyWalletOutcome::Started { record_id } => {
                self.clear_cloud_only_restore_failures(std::slice::from_ref(&record_id));
                self.apply_cloud_only_operation(CloudOnlyOperation::Operating { record_id });
            }
            CloudBackupCloudOnlyWalletOutcome::Restored { record_id, warning } => {
                self.apply_finished_cloud_only_wallet_operation(record_id, warning);
            }
            CloudBackupCloudOnlyWalletOutcome::AlreadyOnDevice {
                record_id,
                listing_generation,
            } => {
                self.apply_cloud_only_operation(CloudOnlyOperation::Idle);
                self.apply_model_event(CloudBackupStateReducerEvent::CloudOnlyRowAlreadyOnDevice {
                    record_id,
                    listing_generation,
                });
            }
            CloudBackupCloudOnlyWalletOutcome::Deleted { record_id } => {
                self.apply_finished_cloud_only_wallet_operation(record_id, None);
            }
            CloudBackupCloudOnlyWalletOutcome::RestoreFailed { record_id, error } => {
                self.apply_cloud_only_restore_failure(record_id, error.clone());
                self.apply_cloud_only_operation(CloudOnlyOperation::Failed { error });
            }
            CloudBackupCloudOnlyWalletOutcome::Failed(error) => {
                self.apply_cloud_only_operation(CloudOnlyOperation::Failed { error });
            }
        }
    }

    pub(crate) fn clear_cloud_only_restore_failures(&self, record_ids: &[String]) {
        let mut cloud_only = self.state.read().cloud_only();
        if let CloudOnlyState::Loaded { wallets, .. } = &mut cloud_only {
            for wallet in wallets {
                if record_ids.contains(&wallet.record_id) {
                    wallet.restore_failure = None;
                }
            }
        }
        self.apply_cloud_only_state(cloud_only);
    }

    pub(crate) fn apply_cloud_only_restore_failure(&self, record_id: String, error: String) {
        let mut cloud_only = self.state.read().cloud_only();
        if let CloudOnlyState::Loaded { wallets, .. } = &mut cloud_only
            && let Some(wallet) = wallets.iter_mut().find(|wallet| wallet.record_id == record_id)
        {
            wallet.restore_failure = Some(CloudBackupWalletRestoreFailure { message: error });
        }
        self.apply_cloud_only_state(cloud_only);
    }

    pub(crate) fn projected_restore_all_state(&self) -> CloudBackupRestoreAllState {
        let super::CloudBackupLifecycle::Configured(configured) = self.state().lifecycle else {
            return CloudBackupRestoreAllState::NotShown;
        };

        configured.restore_all
    }

    /// Wallets Restore All can start with, or `None` while the cloud-only cache is not `Loaded`
    pub(crate) fn restore_all_eligible_wallets(&self) -> Option<Vec<CloudBackupWalletItem>> {
        let CloudOnlyState::Loaded { wallets, .. } = self.state.read().cloud_only() else {
            return None;
        };

        Some(Self::eligible_restore_all_wallets(wallets))
    }

    /// Wallets Restore All still has to restore, or `None` when the cache does not prove it
    ///
    /// Proof means the cache is loaded, every listed record was read, and the rows are current
    /// for the latest listing. Anything less is unknown rather than empty, so the Restore All
    /// marker must stay
    pub(crate) fn restore_all_proven_eligible_wallets(&self) -> Option<Vec<CloudBackupWalletItem>> {
        let state = self.state.read();
        let CloudOnlyState::Loaded { wallets, .. } = state.cloud_only() else {
            return None;
        };

        state
            .cloud_only_is_complete_and_current()
            .then(|| Self::eligible_restore_all_wallets(wallets))
    }

    fn eligible_restore_all_wallets(
        wallets: Vec<CloudBackupWalletItem>,
    ) -> Vec<CloudBackupWalletItem> {
        wallets
            .into_iter()
            .filter(|wallet| wallet.sync_status == CloudBackupWalletStatus::DeletedFromDevice)
            .collect()
    }

    pub(crate) fn apply_restore_all_started(
        &self,
        claim: CloudBackupExclusiveOperationClaim,
        total: u32,
    ) {
        self.apply_model_event(CloudBackupStateReducerEvent::RestoreAllStarted { claim, total });
    }

    pub(crate) fn apply_restore_all_progress(
        &self,
        claim: CloudBackupExclusiveOperationClaim,
        completed: u32,
        current_wallet_name: Option<String>,
    ) {
        self.apply_model_event(CloudBackupStateReducerEvent::RestoreAllProgressed {
            claim,
            completed,
            current_wallet_name,
        });
    }

    pub(crate) fn apply_restore_all_cancellation_requested(
        &self,
        claim: CloudBackupExclusiveOperationClaim,
    ) {
        self.apply_model_event(CloudBackupStateReducerEvent::RestoreAllCancellationRequested(
            claim,
        ));
    }

    pub(crate) fn apply_restore_all_retry_required(&self) {
        self.apply_model_event(CloudBackupStateReducerEvent::RestoreAllRetryRequired);
    }

    pub(crate) fn finish_restore_all_run(
        &self,
        claim: CloudBackupExclusiveOperationClaim,
        retry_remaining: bool,
    ) {
        self.apply_model_event(CloudBackupStateReducerEvent::RestoreAllFinished {
            claim,
            retry_remaining,
        });
    }

    pub(crate) fn reset_restore_all(&self) {
        self.apply_model_event(CloudBackupStateReducerEvent::RestoreAllReset);
    }

    fn apply_finished_cloud_only_wallet_operation(
        &self,
        record_id: String,
        warning: Option<CloudBackupCloudOnlyOperationWarning>,
    ) {
        if let Some(warning) = warning {
            self.apply_cloud_only_operation(CloudOnlyOperation::Warning {
                message: warning.message,
                error: warning.error,
            });
        } else {
            self.apply_cloud_only_operation(CloudOnlyOperation::Idle);
        }

        self.apply_model_event(CloudBackupStateReducerEvent::CloudOnlyRowRemoved(record_id));
    }
}

#[cfg(test)]
mod tests {
    use super::super::ops::test_support::{
        complete_loaded_outcome, ensure_cloud_backup_test_tokio_runtime, listed_detail,
        test_globals, test_lock,
    };
    use super::super::{CloudBackupWalletStatus, PersistedCloudBackupState};
    use super::*;
    use crate::database::Database;

    fn init_manager() -> std::sync::Arc<RustCloudBackupManager> {
        ensure_cloud_backup_test_tokio_runtime();
        test_globals().reset();
        Database::global().cloud_backup_state.set(&PersistedCloudBackupState::default()).unwrap();
        RustCloudBackupManager::init()
    }

    fn cloud_backup_wallet_item(record_id: &str) -> CloudBackupWalletItem {
        CloudBackupWalletItem {
            name: record_id.into(),
            network: None,
            wallet_mode: None,
            wallet_type: None,
            fingerprint: None,
            label_count: None,
            backup_updated_at: None,
            sync_status: CloudBackupWalletStatus::DeletedFromDevice,
            restore_failure: None,
            record_id: record_id.into(),
        }
    }

    #[test]
    fn detail_refresh_resets_empty_cloud_only_cache_when_remote_count_increases() {
        let _guard = test_lock().lock();
        let manager = init_manager();

        manager.apply_cloud_only_fetch_outcome(complete_loaded_outcome(&manager, Vec::new()));
        manager.apply_detail_outcome(CloudBackupDetailOutcome::Refreshed(listed_detail(&[
            "wallet-1",
        ])));

        assert!(matches!(manager.state.read().cloud_only(), CloudOnlyState::NotFetched));
    }

    #[test]
    fn detail_refresh_resets_loaded_cloud_only_cache_when_remote_count_drops_to_zero() {
        let _guard = test_lock().lock();
        let manager = init_manager();

        manager.apply_cloud_only_fetch_outcome(complete_loaded_outcome(
            &manager,
            vec![cloud_backup_wallet_item("wallet-1")],
        ));
        manager.apply_detail_outcome(CloudBackupDetailOutcome::Refreshed(listed_detail(&[])));

        assert!(matches!(manager.state.read().cloud_only(), CloudOnlyState::NotFetched));
    }

    #[test]
    fn detail_refresh_preserves_loaded_cloud_only_cache_after_delete_when_listing_matches() {
        let _guard = test_lock().lock();
        let manager = init_manager();
        let wallet_a = cloud_backup_wallet_item("wallet-a");
        let wallet_b = cloud_backup_wallet_item("wallet-b");

        manager.apply_detail_outcome(CloudBackupDetailOutcome::Refreshed(listed_detail(&[
            "wallet-a", "wallet-b",
        ])));
        manager.apply_cloud_only_fetch_outcome(complete_loaded_outcome(
            &manager,
            vec![wallet_a.clone(), wallet_b.clone()],
        ));
        manager.apply_cloud_only_wallet_outcome(CloudBackupCloudOnlyWalletOutcome::Deleted {
            record_id: wallet_a.record_id,
        });
        manager.apply_detail_outcome(CloudBackupDetailOutcome::Refreshed(listed_detail(&[
            "wallet-b",
        ])));

        assert_eq!(
            manager.state.read().cloud_only(),
            CloudOnlyState::Loaded { wallets: vec![wallet_b], incomplete: false }
        );
    }

    #[test]
    fn detail_refresh_preserves_empty_loaded_cloud_only_cache_after_restore_when_listing_matches() {
        let _guard = test_lock().lock();
        let manager = init_manager();
        let wallet = cloud_backup_wallet_item("wallet-a");

        manager.apply_detail_outcome(CloudBackupDetailOutcome::Refreshed(listed_detail(&[
            "wallet-a",
        ])));
        manager.apply_cloud_only_fetch_outcome(complete_loaded_outcome(
            &manager,
            vec![wallet.clone()],
        ));
        manager.apply_cloud_only_wallet_outcome(CloudBackupCloudOnlyWalletOutcome::Restored {
            record_id: wallet.record_id,
            warning: None,
        });
        manager.apply_detail_outcome(CloudBackupDetailOutcome::Refreshed(listed_detail(&[])));

        assert_eq!(
            manager.state.read().cloud_only(),
            CloudOnlyState::Loaded { wallets: vec![], incomplete: false }
        );
    }

    #[test]
    fn detail_refresh_resets_cloud_only_cache_when_listing_replaced_a_wallet() {
        let _guard = test_lock().lock();
        let manager = init_manager();
        let wallet_a = cloud_backup_wallet_item("wallet-a");
        let wallet_b = cloud_backup_wallet_item("wallet-b");

        manager.apply_detail_outcome(CloudBackupDetailOutcome::Refreshed(listed_detail(&[
            "wallet-a", "wallet-b",
        ])));
        manager.apply_cloud_only_fetch_outcome(complete_loaded_outcome(
            &manager,
            vec![wallet_a, wallet_b],
        ));

        // same count and same local rows, so only the listing's identity shows the replacement
        manager.apply_detail_outcome(CloudBackupDetailOutcome::Refreshed(listed_detail(&[
            "wallet-b", "wallet-c",
        ])));

        assert!(matches!(manager.state.read().cloud_only(), CloudOnlyState::NotFetched));
    }

    #[test]
    fn detail_without_a_listing_resets_loaded_cloud_only_cache() {
        let _guard = test_lock().lock();
        let manager = init_manager();
        let wallet = cloud_backup_wallet_item("wallet-a");

        manager.apply_cloud_only_fetch_outcome(complete_loaded_outcome(&manager, vec![wallet]));
        manager.apply_detail_outcome(CloudBackupDetailOutcome::RefreshedWithoutListing(
            listed_detail(&["wallet-a"]).detail,
        ));

        assert!(matches!(manager.state.read().cloud_only(), CloudOnlyState::NotFetched));
    }

    #[test]
    fn detail_refresh_resets_cloud_only_cache_when_loaded_wallet_is_now_local() {
        let _guard = test_lock().lock();
        let manager = init_manager();
        let wallet = cloud_backup_wallet_item("wallet-1");
        let mut listed = listed_detail(&[]);
        listed.detail.up_to_date.push(wallet.clone());

        manager.apply_cloud_only_fetch_outcome(complete_loaded_outcome(&manager, vec![wallet]));
        manager.apply_detail_outcome(CloudBackupDetailOutcome::Refreshed(listed));

        assert!(matches!(manager.state.read().cloud_only(), CloudOnlyState::NotFetched));
    }

    #[test]
    fn restore_failure_stays_on_its_row_across_authoritative_refetch_and_clears_on_retry() {
        let _guard = test_lock().lock();
        let manager = init_manager();
        let wallet = cloud_backup_wallet_item("wallet-1");
        manager.apply_detail_outcome(CloudBackupDetailOutcome::Refreshed(listed_detail(&[
            "wallet-1",
        ])));
        manager.apply_cloud_only_fetch_outcome(complete_loaded_outcome(
            &manager,
            vec![wallet.clone()],
        ));

        manager.apply_cloud_only_wallet_outcome(CloudBackupCloudOnlyWalletOutcome::RestoreFailed {
            record_id: wallet.record_id.clone(),
            error: "wallet data could not be restored".into(),
        });
        manager.apply_cloud_only_fetch_outcome(complete_loaded_outcome(
            &manager,
            vec![wallet.clone()],
        ));

        let CloudOnlyState::Loaded { wallets, .. } = manager.state.read().cloud_only() else {
            panic!("expected loaded cloud-only rows");
        };
        assert_eq!(
            wallets[0].restore_failure,
            Some(CloudBackupWalletRestoreFailure {
                message: "wallet data could not be restored".into(),
            }),
        );

        manager.apply_cloud_only_wallet_outcome(CloudBackupCloudOnlyWalletOutcome::Started {
            record_id: wallet.record_id,
        });

        let CloudOnlyState::Loaded { wallets, .. } = manager.state.read().cloud_only() else {
            panic!("expected loaded cloud-only rows");
        };
        assert_eq!(wallets[0].restore_failure, None);
    }
}
