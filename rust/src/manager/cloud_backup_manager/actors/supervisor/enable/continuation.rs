use super::*;
use crate::manager::cloud_backup_manager::admission::PausedContinuation;

/// An Enable or reinitialize that stopped before it finished
///
/// It holds the passkey material the work continues from and the lifecycle the work belongs to,
/// so it goes on only as the operation that paused it: a fresh Enable continues as Enable while
/// the backup is off, and a reinitialize continues as Reinitialize on the configured backup it
/// replaces. Admission reads it to decide Confirm Saved Passkey and Discard, so neither acts on
/// a continuation that is gone or belongs to another lifecycle
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

    fn stage(&self) -> ContinuationStage {
        match self.session {
            PendingEnableSession::RetryUpload(_) => ContinuationStage::RetryUpload,
            PendingEnableSession::AwaitingSavedPasskeyConfirmation(_) => {
                ContinuationStage::AwaitingSavedPasskey
            }
        }
    }

    /// What admission sees of this continuation
    pub(crate) fn paused(&self) -> PausedContinuation {
        PausedContinuation { owner: self.owner, stage: self.stage() }
    }
}

impl CloudBackupSupervisor {
    /// Takes the continuation if it is `owner`'s work stopped at `stage`, leaving any other
    pub(crate) fn take_enable_continuation(
        &mut self,
        owner: ContinuationOwner,
        stage: ContinuationStage,
    ) -> Option<EnableContinuation> {
        self.enable_continuation
            .take_if(|continuation| continuation.owner == owner && continuation.stage() == stage)
    }

    /// Whether `owner`'s work is stopped at `stage`
    pub(crate) fn has_enable_continuation(
        &self,
        owner: ContinuationOwner,
        stage: ContinuationStage,
    ) -> bool {
        self.enable_continuation.as_ref().is_some_and(|continuation| {
            continuation.owner == owner && continuation.stage() == stage
        })
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
