use super::*;
use crate::manager::cloud_backup_manager::admission::{
    Admission, AdmissionSnapshot, ConfiguredAdmission, DetailInventory, EnableIntent, Rejection,
    RestoreAllAdmission, UserIntent, evaluate_admission,
};
use crate::manager::cloud_backup_manager::model::{
    CloudBackupDestructiveOperationState, CloudBackupLifecycle,
};
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

        #[cfg(test)]
        let record = self.begin_dispatch_record(&intent);

        let started = match self.admit(&manager, &intent) {
            Admission::Accepted => self.start_admitted(manager, intent).await,
            Admission::Coalesced(_) | Admission::Rejected(_) => Produces::ok(()),
        };

        #[cfg(test)]
        self.finish_dispatch_record(record);

        started?;
        Produces::ok(())
    }

    /// Decides `intent` on the supervisor's current body and records the outcome
    ///
    /// Dispatch and the named entry points that start work outside it, such as the onboarding
    /// restore, decide here, so no start skips admission
    pub(crate) fn admit(
        &mut self,
        manager: &RustCloudBackupManager,
        intent: &UserIntent,
    ) -> Admission {
        let admission = self.decide(manager, intent);
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

        #[cfg(test)]
        {
            self.invariant_log.admissions += 1;
        }

        admission
    }

    /// What dispatch decides for `intent` now, on the body rebased onto the committed record
    pub(crate) fn decide(
        &mut self,
        manager: &RustCloudBackupManager,
        intent: &UserIntent,
    ) -> Admission {
        evaluate_admission(intent, &self.admission_snapshot(manager))
    }

    pub(crate) fn admission_snapshot(&self, manager: &RustCloudBackupManager) -> AdmissionSnapshot {
        // whether the last Turn Off failed is presentation the reducer still holds; it moves
        // into `Configured`'s facts once Turn Off has a review step
        let configured = match &manager.state().lifecycle {
            CloudBackupLifecycle::Configured(configured) => match &configured.destructive_operation
            {
                CloudBackupDestructiveOperationState::DisableFailed {
                    can_keep_enabled, ..
                } => ConfiguredAdmission::TurnOffFailed { can_keep_enabled: *can_keep_enabled },
                CloudBackupDestructiveOperationState::Idle
                | CloudBackupDestructiveOperationState::RecreatingManifest
                | CloudBackupDestructiveOperationState::ReinitializingBackup
                | CloudBackupDestructiveOperationState::Disabling => ConfiguredAdmission::Ready,
            },
            CloudBackupLifecycle::Disabled
            | CloudBackupLifecycle::Enabling(_)
            | CloudBackupLifecycle::Restoring(_)
            | CloudBackupLifecycle::PendingEnableRecovery(_)
            | CloudBackupLifecycle::Failed(_) => ConfiguredAdmission::Ready,
        };
        let body = self.runtime.admission_body(configured);

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
            running: self.runtime.claim().map(CloudBackupExclusiveOperationClaim::operation),
            sync_running: self.active_sync_request.is_some(),
            detail_inventory,
            other_backups_loaded: manager.other_backups_inventory_is_ready(),
            restore_all,
            enable_awaits_saved_passkey: self
                .pending_enable_session
                .as_ref()
                .is_some_and(PendingEnableSession::is_awaiting_saved_passkey_confirmation),
        }
    }

    /// Admits an intent that answers its caller, as a typed error on anything but `Accepted`
    pub(crate) fn admit_or<E: From<Rejection>>(
        &mut self,
        manager: &RustCloudBackupManager,
        intent: UserIntent,
        coalesced: E,
    ) -> Result<(), E> {
        match self.admit(manager, &intent) {
            Admission::Accepted => Ok(()),
            Admission::Coalesced(_) => Err(coalesced),
            Admission::Rejected(rejection) => Err(rejection.into()),
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
            I::StartVerification { source, discoverable } => {
                self.start_user_verification(manager, source, discoverable);
            }
            I::DismissVerificationPrompt => {
                self.runtime.commit(|lifecycle| manager.dismiss_verification_prompt(lifecycle));
            }
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
            // started in this turn, so the operation is allocated with its admission
            I::RecoverOtherBackups => self.begin_recover_other_backups_operation(),
            I::DeleteOtherBackups => self.begin_delete_other_backups_operation(),
            I::TurnOff => {
                self.start_disable_operation().await?;
            }
            I::KeepEnabled => {
                self.keep_cloud_backup_enabled().await?;
            }
            I::RefreshDetail => {
                self.start_refresh_detail().await?;
            }
            I::EnterDetail => {
                self.start_enter_detail().await?;
            }
            I::PromptEnablePasskeyChoice(context) => manager.present_passkey_choice_prompt(
                CloudBackupPasskeyChoiceIntent::Enable(context, None),
            ),
            I::AcceptEnablePrompt(choice) => manager.accept_enable_prompt(choice),
            // these answer their caller, so their own handlers admit them and reply
            I::SwitchDriveAccount
            | I::CancelDriveAccountSwitch
            | I::RetryDriveAccountSwitchRecovery
            | I::LocalReset => {
                warn!("Cloud Backup {intent:?} reached dispatch, which can't answer it");
            }
        }

        Produces::ok(())
    }
}
