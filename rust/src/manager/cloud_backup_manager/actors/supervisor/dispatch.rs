use super::*;
use crate::manager::cloud_backup_manager::admission::{
    Admission, AdmissionBody, AdmissionSnapshot, ConfiguredAdmission, DetailInventory,
    EnableIntent, RestoreAllAdmission, UserIntent, evaluate_admission,
};
use crate::manager::cloud_backup_manager::model::{
    CloudBackupDestructiveOperationState, CloudBackupLifecycle,
};
use crate::manager::cloud_backup_manager::owner::lifecycle::DisablingRecord;
use tracing::debug;

/// What the last dispatched intent got, for diagnostics and tests
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DispatchOutcome {
    pub(crate) intent: UserIntent,
    pub(crate) admission: Admission,
}

impl CloudBackupSupervisor {
    /// The only entry point for a user intent
    ///
    /// Admission and the start of the admitted work run in one actor turn, so nothing can take
    /// the operation slot between the decision and the start
    pub async fn dispatch(&mut self, intent: UserIntent) -> ActorResult<()> {
        let Some(manager) = self.manager() else { return Produces::ok(()) };

        let admission = evaluate_admission(&intent, &self.admission_snapshot(&manager));
        match admission {
            Admission::Accepted => {}
            Admission::Coalesced(into) => {
                debug!("Cloud Backup coalesced {intent:?} into {into:?}");
            }
            Admission::Rejected(rejection) => {
                info!("Cloud Backup rejected {intent:?}: {rejection:?}");
            }
        }

        self.last_outcome = Some(DispatchOutcome { intent: intent.clone(), admission });
        if admission == Admission::Accepted {
            self.start_admitted(manager, intent).await?;
        }

        Produces::ok(())
    }

    pub(crate) fn admission_snapshot(&self, manager: &RustCloudBackupManager) -> AdmissionSnapshot {
        let persisted = RustCloudBackupManager::load_persisted_state();
        let lifecycle = manager.state().lifecycle;
        let body = match DisablingRecord::of(&persisted) {
            Some(DisablingRecord::Committed(_)) => AdmissionBody::FinishingDisable,
            Some(DisablingRecord::Unnormalized) => AdmissionBody::LoadFailed,
            None => match &lifecycle {
                CloudBackupLifecycle::Disabled => AdmissionBody::Disabled,
                CloudBackupLifecycle::Enabling(_) => AdmissionBody::Enabling,
                CloudBackupLifecycle::Restoring(_) => AdmissionBody::Restoring,
                CloudBackupLifecycle::PendingEnableRecovery(_) => {
                    AdmissionBody::PendingEnableRecovery
                }
                CloudBackupLifecycle::Failed(_) => AdmissionBody::Failed,
                CloudBackupLifecycle::Configured(configured) => {
                    AdmissionBody::Configured(match &configured.destructive_operation {
                        CloudBackupDestructiveOperationState::DisableFailed {
                            can_keep_enabled,
                            ..
                        } => ConfiguredAdmission::TurnOffFailed {
                            can_keep_enabled: *can_keep_enabled,
                        },
                        CloudBackupDestructiveOperationState::Idle
                        | CloudBackupDestructiveOperationState::RecreatingManifest
                        | CloudBackupDestructiveOperationState::ReinitializingBackup
                        | CloudBackupDestructiveOperationState::Disabling => {
                            ConfiguredAdmission::Ready
                        }
                    })
                }
            },
        };

        let detail_inventory = {
            let state = manager.state.read();
            if state.detail_inventory_is_complete() {
                DetailInventory::Complete
            } else if state.detail_inventory_is_ready() {
                DetailInventory::Ready
            } else {
                DetailInventory::NotLoaded
            }
        };

        let restore_all = match manager.projected_restore_all_state() {
            CloudBackupRestoreAllState::StartAvailable { .. } => {
                RestoreAllAdmission::StartAvailable
            }
            CloudBackupRestoreAllState::RetryAvailable { .. } => {
                RestoreAllAdmission::RetryAvailable
            }
            CloudBackupRestoreAllState::Running { .. } => RestoreAllAdmission::Running,
            _ => RestoreAllAdmission::Unavailable,
        };

        AdmissionSnapshot {
            body,
            running: self
                .active_operation
                .claim()
                .map(CloudBackupExclusiveOperationClaim::operation),
            sync_running: self.active_sync_request.is_some(),
            detail_inventory,
            other_backups_loaded: manager.other_backups_inventory_is_ready(),
            restore_all,
        }
    }

    // exhaustive, like the intent map: every admitted intent starts exactly one piece of work
    #[deny(clippy::wildcard_enum_match_arm)]
    async fn start_admitted(
        &mut self,
        manager: Arc<RustCloudBackupManager>,
        intent: UserIntent,
    ) -> ActorResult<()> {
        use UserIntent as I;

        match intent {
            I::Enable(EnableIntent::Standard, context) => {
                self.start_enable_operation(context).await?;
            }
            I::Enable(EnableIntent::ForceNew, context) => {
                self.start_enable_force_new_operation(context).await?;
            }
            I::Enable(EnableIntent::NoDiscovery, context) => {
                self.start_enable_no_discovery_operation(context).await?;
            }
            I::ConfirmSavedPasskey => {
                self.confirm_saved_passkey().await?;
            }
            I::DiscardPendingEnable => {
                self.discard_pending_enable_cloud_backup().await?;
                manager.clear_existing_backup_found_prompt();
            }
            I::ConfirmPendingEnableCleanup => {
                self.confirm_pending_enable_cleanup().await?;
            }
            I::DismissPasskeyChoicePrompt => manager.clear_passkey_choice_prompt(),
            I::DismissMissingPasskeyReminder => manager.dismiss_missing_passkey_prompt(),
            I::RestoreFromCloudBackup => {
                self.start_restore_from_cloud_backup().await?;
            }
            I::CancelRestore => {
                self.cancel_restore().await?;
            }
            I::StartVerification { source, discoverable: false } => {
                manager.start_verification(source);
            }
            I::StartVerification { source, discoverable: true } => {
                manager.start_verification_discoverable(source);
            }
            I::DismissVerificationPrompt => manager.dismiss_verification_prompt(),
            I::RecreateManifest => {
                self.start_recovery_operation(RecoveryAction::RecreateManifest).await?;
            }
            I::ReinitializeBackup => {
                self.start_recovery_operation(RecoveryAction::ReinitializeBackup).await?;
            }
            I::RepairPasskey { no_discovery } => {
                manager.clear_passkey_choice_prompt();
                self.start_repair_passkey_operation(no_discovery).await?;
            }
            I::SyncUnsynced => {
                self.start_sync_operation().await?;
            }
            I::RestoreCloudWallet(record_id) => {
                self.start_restore_cloud_wallet_operation(record_id.into()).await?;
            }
            I::StartRestoreAll => {
                self.start_restore_all_operation(false).await?;
            }
            I::RetryRestoreAllRemaining => {
                self.start_restore_all_operation(true).await?;
            }
            I::CancelRestoreAll => {
                self.cancel_restore_all_operation().await?;
            }
            I::DeleteCloudWallet(record_id) => {
                self.start_delete_cloud_wallet_operation(record_id.into()).await?;
            }
            I::DeleteUndecryptableWalletBackups => {
                self.start_delete_undecryptable_wallet_backups_operation().await?;
            }
            I::RecoverOtherBackups => manager.spawn_recover_other_backups(),
            I::DeleteOtherBackups => manager.spawn_delete_other_backups(),
            I::TurnOff => {
                self.start_disable_operation().await?;
            }
            I::KeepEnabled => {
                self.keep_cloud_backup_enabled().await?;
            }
            I::RefreshDetail => manager.spawn_refresh_detail(),
            I::EnterDetail => manager.spawn_enter_detail(),
            I::PromptEnablePasskeyChoice(context) => manager.present_passkey_choice_prompt(
                CloudBackupPasskeyChoiceIntent::Enable(context, None),
            ),
            I::AcceptEnablePrompt(choice) => manager.accept_enable_prompt(choice),
        }

        Produces::ok(())
    }
}
