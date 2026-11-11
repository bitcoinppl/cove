use super::*;
use crate::manager::cloud_backup_manager::CloudBackupNoticeKind;
use crate::manager::cloud_backup_manager::admission::{
    Admission, AdmissionSnapshot, ContinuationOwner, ContinuationStage, EnableIntent,
    EnablePromptAnswers, PausedContinuation, Rejection, RestoreAllAdmission, UserIntent,
    evaluate_admission,
};
use crate::manager::cloud_backup_manager::{
    CloudBackupAcceptedEnablePrompt, CloudBackupEnableContext, CloudBackupEnablePromptChoice,
    CloudBackupStateReducer,
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
        let body = self.runtime.admission_body();

        let detail_inventory = manager.inventory.read().readiness();

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
            continuation: self.paused_continuation(manager),
            enable_prompt: enable_prompt_answers(&manager.state.read()),
        }
    }

    /// The paused Enable or reinitialize admission decides Confirm Saved Passkey and Discard on
    ///
    /// The existing-backup prompt is a paused Enable that holds no material; the reducer still
    /// holds that prompt until the screen projection owns it
    fn paused_continuation(&self, manager: &RustCloudBackupManager) -> Option<PausedContinuation> {
        if let Some(continuation) = &self.enable_continuation {
            return Some(continuation.paused());
        }

        manager.state.read().shows_existing_backup_prompt().then_some(PausedContinuation {
            owner: ContinuationOwner::Enable,
            stage: ContinuationStage::ExistingBackupPrompt,
        })
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
                // admitted with no continuation only for the existing-backup prompt, which holds
                // no material, so clearing the prompt ends it
                if let Some(continuation) = self.enable_continuation.take() {
                    self.discard_enable_continuation(continuation);
                }

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
                self.start_verification_intent(manager, source, discoverable);
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
            I::TurnOff | I::ResumeDisable => {
                self.start_disable_operation().await?;
            }
            I::DismissNotice(CloudBackupNoticeKind::Heal) => {
                self.runtime.commit(|lifecycle| manager.dismiss_heal_notice(lifecycle));
            }
            // no last outcome is published on the screen yet, so there is nothing to dismiss
            I::DismissNotice(CloudBackupNoticeKind::LastOutcome) => {}
            // Google Drive authorization runs through the detail verification, as it did before
            // it had its own intent
            I::ReconnectDrive => {
                self.start_user_verification(
                    manager,
                    CloudBackupVerificationSource::CloudBackupDetail,
                    false,
                );
            }
            // no Turn Off review is held yet, so leaving the detail screen changes nothing
            I::LeaveDetail => {}
            I::RefreshDetail => {
                self.start_refresh_detail().await?;
            }
            I::EnterDetail => {
                self.start_enter_detail().await?;
            }
            I::PromptEnablePasskeyChoice(context) => manager.present_passkey_choice_prompt(
                CloudBackupPasskeyChoiceIntent::Enable(context, None),
            ),
            // admitted as the Enable it starts, so that Enable takes the slot in this same turn
            // and nothing queued behind the answer can take it first
            I::AcceptEnablePrompt(choice) => {
                if let Some(accepted) = manager.accept_enable_prompt(choice) {
                    match enable_of(accepted) {
                        (EnableIntent::Standard, context) => self.begin_enable_operation(context),
                        (EnableIntent::ForceNew, context) => {
                            self.begin_enable_force_new_operation(context);
                        }
                        (EnableIntent::NoDiscovery, context) => {
                            self.begin_enable_no_discovery_operation(context);
                        }
                    }
                }
            }
            // these answer their caller, so their own handlers admit them and reply
            I::SwitchDriveAccount
            | I::CancelDriveAccountSwitch
            | I::RetryDriveAccountSwitchRecovery
            | I::LocalReset => {
                warn!("Cloud Backup {intent:?} reached dispatch, which can't answer it");
            }
            // admission rejects these until their flows exist
            I::ConfirmDisable { .. }
            | I::CancelDisable
            | I::StopUsingOnThisDevice
            | I::RetryLoad
            | I::ResetOnThisDevice => {
                warn!("Cloud Backup admitted {intent:?}, which has no flow yet");
            }
        }

        Produces::ok(())
    }
}

/// What each answer to the Enable prompt on screen starts
///
/// The reducer holds the prompt, so dispatch and the screen projection both read it from there
pub(crate) fn enable_prompt_answers(state: &CloudBackupStateReducer) -> EnablePromptAnswers {
    let starts = |choice| state.enable_prompt_answer(choice).map(|accepted| enable_of(accepted).0);

    EnablePromptAnswers {
        use_existing: starts(CloudBackupEnablePromptChoice::UseExisting),
        create_new: starts(CloudBackupEnablePromptChoice::CreateNew),
    }
}

/// The Enable an accepted prompt answer starts, and the context it starts with
fn enable_of(
    accepted: CloudBackupAcceptedEnablePrompt,
) -> (EnableIntent, CloudBackupEnableContext) {
    match accepted {
        CloudBackupAcceptedEnablePrompt::Enable(context) => (EnableIntent::Standard, context),
        CloudBackupAcceptedEnablePrompt::ForceNew(context) => (EnableIntent::ForceNew, context),
        CloudBackupAcceptedEnablePrompt::NoDiscovery(context) => {
            (EnableIntent::NoDiscovery, context)
        }
    }
}
