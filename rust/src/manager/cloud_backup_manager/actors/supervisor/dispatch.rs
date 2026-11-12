use super::*;
use crate::manager::cloud_backup_manager::admission::{
    Admission, AdmissionSnapshot, CoalescibleIntent, ContinuationOwner, ContinuationStage,
    EnableIntent, EnablePromptAnswers, PausedContinuation, RestoreAllAdmission, UserIntent,
    evaluate_admission,
};
use crate::manager::cloud_backup_manager::delete_consent::DeleteConsents;
use crate::manager::cloud_backup_manager::inventory::InventoryFacts;
use crate::manager::cloud_backup_manager::turn_off::{
    AdmittedFinish, ConfirmedTurnOff, DeleteAuthority, ReviewId,
};
use crate::manager::cloud_backup_manager::{
    CloudBackupAcceptedEnablePrompt, CloudBackupEnableContext, CloudBackupEnablePromptChoice,
    CloudBackupNoticeKind, CloudBackupStateReducer,
};
use tracing::debug;

/// The start an accepted intent's admission produced
///
/// Turn Off's own intents allocate their operation as they are admitted, so what they start
/// holds its operation and is consumed once to start it; every other intent starts its own work
#[derive(Debug)]
enum Admitted {
    /// The work an intent starts by itself, or the work a review button names
    Work(UserIntent),
    /// Consent to the review on screen, which took the review and allocated the Turn Off
    Confirmed(ConfirmedTurnOff),
    /// A finishing run of the committed Turn Off, allocated on the idle finishing body
    Finish(AdmittedFinish),
}

/// Why an accepted intent produced no start
#[derive(Debug)]
enum Refused {
    /// The body no longer holds what admission accepted the intent on
    Rejected(Rejection),
    /// Stop Using on This Device couldn't save its choice, so nothing started
    SaveFailed(CloudBackupError),
}

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

        let started = match self.admit_start(&manager, &intent) {
            // a review button starts the work it names in this turn, attributed to that work's
            // own control, since the review it was drawn on ends with the start
            Some(Admitted::Work(work)) => self.start_and_attribute(manager, work).await,
            Some(Admitted::Confirmed(confirmed)) => {
                self.attribute(confirmed.claim, &intent);
                self.confirm_disable(&manager, confirmed);
                Produces::ok(())
            }
            Some(Admitted::Finish(finish)) => {
                self.attribute(finish.run.claim, &intent);
                self.begin_finish(&manager, finish);
                Produces::ok(())
            }
            None => Produces::ok(()),
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
        self.record_admission(manager, intent, admission);
        admission
    }

    /// Decides `intent`, and for an accepted one produces its start in the same step, then
    /// records the outcome
    ///
    /// An accepted Turn Off intent changes the body as it is admitted, so a body that no longer
    /// holds what admission read rejects it here, before anything starts
    fn admit_start(
        &mut self,
        manager: &RustCloudBackupManager,
        intent: &UserIntent,
    ) -> Option<Admitted> {
        let decided = self.decide(manager, intent);
        if decided != Admission::Accepted {
            self.record_admission(manager, intent, decided);

            // a coalesced intent starts nothing of its own, but the work it repeats takes it
            if let Admission::Coalesced(into) = decided {
                self.join_running(into);
            }

            return None;
        }

        match self.transition(manager, intent) {
            Ok(admitted) => {
                self.record_admission(manager, intent, Admission::Accepted);
                Some(admitted)
            }
            Err(Refused::Rejected(rejection)) => {
                self.record_admission(manager, intent, Admission::Rejected(rejection));
                None
            }
            Err(Refused::SaveFailed(error)) => {
                self.record_admission(manager, intent, Admission::Accepted);
                let action = CloudBackupScreenAction::StopUsingOnThisDevice;
                if self.screen_outcome.action_failed(action, &error) {
                    manager.request_screen_publish();
                }
                None
            }
        }
    }

    /// Hands a coalesced intent to the work it joins, which already shows on screen
    fn join_running(&mut self, into: CoalescibleIntent) {
        match into {
            // the load running may be silent, so the refresh after it carries the tap's authority
            // to ask the provider for consent
            CoalescibleIntent::RefreshDetail => {
                self.detail_workflow.join_refresh(DetailRefreshOrigin::Tap);
            }

            // the running sync and operation already do what the repeated tap asks
            CoalescibleIntent::SyncUnsynced | CoalescibleIntent::RepeatOfRunning(_) => {}
        }
    }

    /// The start an accepted `intent` produces on the current body
    fn transition(
        &mut self,
        manager: &RustCloudBackupManager,
        intent: &UserIntent,
    ) -> Result<Admitted, Refused> {
        use UserIntent as I;

        match intent {
            // a delete runs only on the evidence its confirmation showed
            I::DeleteCloudWallet(consent) => {
                self.consented_delete(manager, intent, |offered| offered.admits_row(consent))
            }
            I::DeleteUndecryptableWalletBackups(consent) => {
                self.consented_delete(manager, intent, |offered| {
                    offered.admits_undecryptable_batch(consent)
                })
            }
            I::ConfirmDisable { review_id } => self
                .runtime
                .admit_confirm(ReviewId::from(*review_id))
                .map(Admitted::Confirmed)
                .map_err(Refused::Rejected),
            I::LeaveReview { review_id, to } => self
                .runtime
                .leave_review(ReviewId::from(*review_id), *to)
                .map(Admitted::Work)
                .map_err(Refused::Rejected),
            I::ResumeDisable => self.start_finish(),
            // Reconnect on the committed Turn Off resumes it with the user's consent
            I::ReconnectDrive if matches!(*self.runtime, Runtime::FinishingDisable { .. }) => {
                self.start_finish()
            }
            // the choice is saved before the run starts, so no resume after a failure or a kill
            // can turn this finish back into a remote delete
            I::StopUsingOnThisDevice => {
                self.choose_local_only()?;
                self.start_finish()
            }
            intent => Ok(Admitted::Work(intent.clone())),
        }
    }

    /// Starts a delete whose consent is the one the inventory offers now
    ///
    /// The consent names the rows, category and generations the confirmation was drawn from, so
    /// any change since makes it stale, the way a changed review makes its Confirm stale; the
    /// screen published with the rejection carries the current consent
    fn consented_delete(
        &self,
        manager: &RustCloudBackupManager,
        intent: &UserIntent,
        admits: impl FnOnce(&DeleteConsents<'_>) -> bool,
    ) -> Result<Admitted, Refused> {
        // a namespace that can't be read can't match the one the records were listed in
        let namespace = manager
            .current_namespace_id()
            .inspect_err(|error| warn!("Read the namespace to check a delete's consent: {error}"))
            .ok();

        let view = manager.inventory_facts().view();
        let consents =
            view.delete_consents(self.runtime.generation()).in_namespace(namespace.as_deref());

        if !admits(&consents) {
            return Err(Refused::Rejected(Rejection::ReviewStale));
        }

        Ok(Admitted::Work(intent.clone()))
    }

    /// A finishing run the user tapped for, so the provider may ask for consent on the way
    fn start_finish(&mut self) -> Result<Admitted, Refused> {
        self.runtime
            .start_finish(DeleteAuthority::TryAgain)
            .map(Admitted::Finish)
            .map_err(Refused::Rejected)
    }

    /// Saves Stop Using on This Device in the committed record, as its local-only disposition
    fn choose_local_only(&mut self) -> Result<(), Refused> {
        let Runtime::FinishingDisable { committed, .. } = &*self.runtime else {
            return Err(Refused::Rejected(Rejection::NothingToContinue));
        };

        let committed = committed.clone();
        match self.runtime.commit(|lifecycle| lifecycle.choose_local_only(&committed)) {
            Ok(true) => Ok(()),
            // the record moved on to another attempt or out of Disabling
            Ok(false) => Err(Refused::Rejected(Rejection::NothingToContinue)),
            Err(error) => Err(Refused::SaveFailed(CloudBackupError::internal_context(
                "save Stop Using on This Device",
                error,
            ))),
        }
    }

    /// Records what `intent` got, and shows a rejection next to the control that sent it
    fn record_admission(
        &mut self,
        manager: &RustCloudBackupManager,
        intent: &UserIntent,
        admission: Admission,
    ) {
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

        if self.screen_outcome.admitted(intent, admission) {
            manager.request_screen_publish();
        }

        #[cfg(test)]
        {
            self.invariant_log.admissions += 1;
        }
    }

    /// What dispatch decides for `intent` now, on the body rebased onto the committed record
    pub(crate) fn decide(
        &mut self,
        manager: &RustCloudBackupManager,
        intent: &UserIntent,
    ) -> Admission {
        let inventory = manager.inventory_facts();
        evaluate_admission(intent, &self.admission_snapshot(manager, &inventory))
    }

    pub(crate) fn admission_snapshot(
        &self,
        manager: &RustCloudBackupManager,
        inventory: &InventoryFacts,
    ) -> AdmissionSnapshot {
        let body = self.admission_body(manager);
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

        // each read of the reducer is its own statement: a guard held across another read
        // deadlocks once a writer waits, since parking_lot's RwLock lets the writer go first
        let enable_prompt = enable_prompt_answers(&manager.state.read());
        let drive_account_switch = self.drive_account_switch_admission(manager);

        AdmissionSnapshot {
            body,
            running: self.runtime.claim().map(CloudBackupExclusiveOperationClaim::operation),
            sync_running: self.active_sync_request.is_some(),
            detail_inventory: inventory.readiness(),
            detail_checking: inventory.checking(),
            other_backups_loaded: manager.other_backups_inventory_is_ready(),
            restore_all,
            continuation: self.paused_continuation(manager),
            enable_prompt,
            turn_off: self.runtime.turn_off_admission(),
            drive_account_switch,
        }
    }

    /// The paused Enable or reinitialize admission decides Confirm Saved Passkey and Discard on
    ///
    /// The coordinator's view of the journal says which work is paused and whose it is, whether or
    /// not its material is in memory. An Enable waiting on one of its prompts, the existing-backup
    /// prompt or the passkey choice, is a paused Enable that holds no material; the reducer still
    /// holds those prompts until the screen projection owns them
    fn paused_continuation(&self, manager: &RustCloudBackupManager) -> Option<PausedContinuation> {
        if let PendingWork::Paused(owner) = manager.pending_enable.pending_work() {
            return Some(PausedContinuation {
                owner,
                stage: ContinuationStage::AwaitingSavedPasskey,
            });
        }

        manager.state.read().is_awaiting_enable_prompt().then_some(PausedContinuation {
            owner: ContinuationOwner::Enable,
            stage: ContinuationStage::EnablePrompt,
        })
    }

    /// The body admission decides on: the lifecycle's, unless journal evidence nobody has taken
    /// up stands in the way while nothing runs
    ///
    /// Unknown evidence is never a saved passkey. It is the load failure it is, so the screen
    /// offers Try Again, which runs the recovery again, and nothing replaces the configuration
    /// the evidence may still restore
    pub(crate) fn admission_body(&self, manager: &RustCloudBackupManager) -> AdmissionBody {
        let body = self.runtime.admission_body();
        let settled = self.runtime.claim().is_none()
            && !matches!(body, AdmissionBody::PendingEnableRecovery | AdmissionBody::Resetting);
        if settled && manager.pending_enable.pending_work() == PendingWork::Unavailable {
            return AdmissionBody::LoadFailed;
        }

        body
    }

    /// Starts an admitted intent, and remembers which control started the operation it claimed,
    /// so a failure is drawn next to that control
    async fn start_and_attribute(
        &mut self,
        manager: Arc<RustCloudBackupManager>,
        intent: UserIntent,
    ) -> ActorResult<()> {
        let before = self.runtime.claim();
        let started = self.start_admitted(manager.clone(), intent.clone()).await;

        if let Some(claim) = self.runtime.claim()
            && Some(claim) != before
        {
            self.attribute(claim, &intent);
        }

        started
    }

    /// Remembers that `intent`'s control started the operation `claim`
    fn attribute(&mut self, claim: CloudBackupExclusiveOperationClaim, intent: &UserIntent) {
        self.screen_outcome.started(claim, intent);
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
            I::DiscardPendingEnable => match self.discard_target(&manager) {
                Ok(DiscardTarget::Prompt) => manager.clear_enable_prompt(),
                Ok(DiscardTarget::Work(continuation)) => {
                    self.discard_enable_continuation(continuation);
                    manager.clear_enable_prompt();
                }
                Err(error) => {
                    self.answer_failed(
                        &manager,
                        CloudBackupScreenAction::DiscardPendingEnable,
                        &error,
                    );
                }
            },
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
            I::DeleteCloudWallet(consent) => {
                self.start_delete_cloud_wallet_operation(consent.record_id().clone().into())
                    .await?;
            }
            I::DeleteUndecryptableWalletBackups(consent) => {
                let record_ids = consent.record_ids().cloned().map(String::from).collect();
                self.start_delete_undecryptable_wallet_backups_operation(record_ids).await?;
            }
            // started in this turn, so the operation is allocated with its admission
            I::RecoverOtherBackups => self.begin_recover_other_backups_operation(),
            I::DeleteOtherBackups => self.begin_delete_other_backups_operation(),
            I::TurnOff => self.begin_turn_off_review(manager),
            I::CancelDisable => self.cancel_disable(&manager),
            I::DismissNotice(CloudBackupNoticeKind::Heal) => {
                self.runtime.commit(|lifecycle| manager.dismiss_heal_notice(lifecycle));
            }
            I::DismissNotice(CloudBackupNoticeKind::LastOutcome) => {
                if self.screen_outcome.dismiss() {
                    manager.request_screen_publish();
                }
            }
            // on a configured backup, Google Drive authorization runs through the detail
            // verification, as it did before it had its own intent
            I::ReconnectDrive => {
                self.start_verification_intent(
                    manager,
                    CloudBackupVerificationSource::CloudBackupDetail,
                    false,
                );
            }
            // the backstop for every dismissal path the platform might miss
            I::LeaveDetail => self.leave_turn_off_review(),
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
            I::SwitchDriveAccount => self.start_drive_account_switch(&manager),
            I::RetryDriveAccountSwitchRecovery => self.retry_drive_account_switch(&manager),
            I::CancelDriveAccountSwitch => self.cancel_drive_account_switch(&manager),
            // the reset answers its caller, so its own handler admits it and replies
            I::LocalReset => {
                warn!("Cloud Backup {intent:?} reached dispatch, which can't answer it");
            }
            // their admission allocated what they start, and dispatch starts it in their place
            I::LeaveReview { .. }
            | I::ConfirmDisable { .. }
            | I::ResumeDisable
            | I::StopUsingOnThisDevice => {
                warn!("Cloud Backup {intent:?} reached a start without its admission");
            }
            I::RetryLoad => self.retry_load(&manager),
            // the reset fences the supervisor through calls to it, so it runs off this handler
            I::ResetOnThisDevice => {
                cove_tokio::task::spawn(async move {
                    if let Err(error) = manager.reset_corrupted_on_this_device().await {
                        send!(manager.supervisor.reset_on_this_device_failed(error));
                    }
                });
            }
        }

        Produces::ok(())
    }

    /// Loads the lifecycle record again after a failed load, as a launch would
    ///
    /// The record's other owners reconcile from it the way they do at launch, and a committed
    /// Turn Off it reveals resumes as one would after a relaunch
    fn retry_load(&mut self, manager: &RustCloudBackupManager) {
        // the lifecycle loaded, but the journal could not be read, so its recovery runs again
        if self.runtime.admission_body() != AdmissionBody::LoadFailed {
            self.recover_journaled_enable(manager);
            manager.request_screen_publish();
            return;
        }

        if let Err(error) = self.runtime.retry_load() {
            warn!("Cloud Backup could not load its lifecycle record again: {error}");
            let error = CloudBackupError::internal_context("load cloud backup lifecycle", error);
            if self.screen_outcome.action_failed(CloudBackupScreenAction::RetryLoad, &error) {
                manager.request_screen_publish();
            }

            return;
        }

        info!("Cloud Backup loaded its lifecycle record on retry");
        manager.sync_persisted_state();

        // the journal's recovery waited for the record, so it runs now, as a launch runs it
        self.recover_journaled_enable(manager);
        manager.request_screen_publish();
        manager.resume_persisted_disable_if_needed();
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
