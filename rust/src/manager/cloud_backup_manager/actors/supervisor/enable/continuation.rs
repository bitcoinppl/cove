use super::*;

/// An Enable or reinitialize that stopped before it finished
///
/// It holds the passkey material the work continues from and the lifecycle the work belongs to,
/// so it goes on only as the operation that paused it: a fresh Enable continues as Enable while
/// the backup is off, and a reinitialize continues as Reinitialize on the configured backup it
/// replaces. It only caches that material: admission reads the coordinator's view of the journal,
/// which records the pause whether or not the material is still in memory
#[derive(Debug)]
pub(crate) struct EnableContinuation {
    owner: ContinuationOwner,
    session: PendingEnableSession,
}

impl EnableContinuation {
    /// The continuation of the work `claim` runs
    pub(crate) fn of(
        claim: CloudBackupExclusiveOperationClaim,
        session: PendingEnableSession,
    ) -> Self {
        Self::new(ContinuationOwner::of(claim), session)
    }

    pub(crate) fn new(owner: ContinuationOwner, session: PendingEnableSession) -> Self {
        Self { owner, session }
    }

    pub(crate) fn owner(&self) -> ContinuationOwner {
        self.owner
    }

    pub(crate) fn session(&self) -> &PendingEnableSession {
        &self.session
    }

    pub(crate) fn into_session(self) -> PendingEnableSession {
        self.session
    }
}

/// What an accepted Discard ends
pub(crate) enum DiscardTarget {
    /// The existing-backup prompt, which holds no material, so closing it ends it
    Prompt,
    /// Paused work, with its material taken up from memory or the journal
    Work(EnableContinuation),
}

impl CloudBackupSupervisor {
    /// Takes the continuation if it is `owner`'s work, leaving any other
    pub(crate) fn take_enable_continuation(
        &mut self,
        owner: ContinuationOwner,
    ) -> Option<EnableContinuation> {
        self.enable_continuation.take_if(|continuation| continuation.owner == owner)
    }

    /// Whether `owner`'s work holds its material in memory
    pub(crate) fn has_enable_continuation(&self, owner: ContinuationOwner) -> bool {
        self.enable_continuation.as_ref().is_some_and(|continuation| continuation.owner == owner)
    }

    /// Takes up the paused work the coordinator's view records, as the owner it records
    ///
    /// The continuation only keeps the work's material in memory, so a failed run drops it while
    /// the journal stays; the material is rebuilt here from the journal, as a relaunch rebuilds
    /// it. A journal that can't be read is an error for the caller to show: the view keeps the
    /// pause, so the same answer works once the keychain reads again
    pub(crate) fn take_up_paused_work(
        &mut self,
        manager: &RustCloudBackupManager,
    ) -> Result<(), CloudBackupError> {
        let PendingWork::Paused(owner) = manager.pending_enable.pending_work() else {
            return Ok(());
        };
        if self.enable_continuation.is_none() {
            self.enable_continuation = Self::journaled_pause(owner)?;
        }

        Ok(())
    }

    /// Resolves an accepted Discard to what it ends; only paused work reads the journal
    pub(crate) fn discard_target(
        &mut self,
        manager: &RustCloudBackupManager,
    ) -> Result<DiscardTarget, CloudBackupError> {
        self.take_up_paused_work(manager)?;

        Ok(self.enable_continuation.take().map_or(DiscardTarget::Prompt, DiscardTarget::Work))
    }

    /// Shows why an answer to paused work did not run, next to the control that sent it
    pub(crate) fn answer_failed(
        &mut self,
        manager: &RustCloudBackupManager,
        action: CloudBackupScreenAction,
        error: &CloudBackupError,
    ) {
        warn!("Cloud Backup could not take up the paused Enable for {action:?}: {error}");
        if self.screen_outcome.action_failed(action, error) {
            manager.request_screen_publish();
        }
    }

    /// Keeps the material of the work `claim` runs, to continue from if that work stops
    pub(crate) fn keep_enable_continuation(
        &mut self,
        claim: CloudBackupExclusiveOperationClaim,
        session: PendingEnableSession,
    ) {
        self.enable_continuation = Some(EnableContinuation::of(claim, session));
    }
}
