//! The in-memory view of the pending-enable journal's evidence
//!
//! The journal is one keychain item, and the three `CloudBackupKeychain` journal accessors are
//! the only I/O path to it, so they keep this view current for every writer: the Enable flow,
//! restart cleanup, reset, discard and switch rollback alike. Admission and the screen read the
//! view through the coordinator instead of reading the keychain

use parking_lot::Mutex;

use super::{PendingEnableJournal, PendingEnableJournalPhase, PendingEnableNamespaceOwnership};
use crate::manager::cloud_backup_manager::admission::ContinuationOwner;

/// What the journal holds, as this process last wrote or read it
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum JournalFact {
    /// No recovery has read the journal yet. The restart recovery is the supervisor's first
    /// message, and it waits only while the lifecycle record can't be read, when admission
    /// offers nothing this would change
    NotRead,
    /// No journal, or one that records no passkey the user saved
    NoSavedPasskey,
    /// A fresh Enable's registered passkey waits for the user's Confirm or Discard
    SavedPasskey,
    /// A local promotion began, or a recovered backup's work has no pause to go back to: only a
    /// running operation or the recovery may end it
    RecoveryOwned,
    /// The recovery could not read the journal
    Unreadable,
}

impl JournalFact {
    fn of(journal: Option<&PendingEnableJournal>) -> Self {
        match journal {
            Some(journal) if journal.awaits_saved_passkey() => Self::SavedPasskey,
            Some(journal)
                if matches!(
                    journal.phase(),
                    PendingEnableJournalPhase::LocalPromotionStarted(_)
                ) || journal.namespace_ownership()
                    == PendingEnableNamespaceOwnership::RecoveredExisting =>
            {
                Self::RecoveryOwned
            }
            Some(_) | None => Self::NoSavedPasskey,
        }
    }
}

#[derive(Debug)]
struct JournalView {
    fact: JournalFact,
    // the work the journal belongs to; the journal doesn't record it, so the supervisor names it
    // when that work starts and the recovery names it when it takes the journal up
    owner: Option<ContinuationOwner>,
}

static VIEW: Mutex<JournalView> =
    Mutex::new(JournalView { fact: JournalFact::NotRead, owner: None });

/// What the coordinator knows of pending Enable-flow work, without reading the keychain
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PendingWork {
    /// No journal records a passkey the user saved
    None,
    /// The journal records a passkey `owner`'s work registered, which Confirm or Discard answers
    Paused(ContinuationOwner),
    /// The journal could not be read, or records work no owner has taken up; the recovery takes
    /// it up on Try Again
    Unavailable,
}

pub(crate) fn pending_work() -> PendingWork {
    let view = VIEW.lock();
    match (view.fact, view.owner) {
        (JournalFact::NotRead | JournalFact::NoSavedPasskey, _) => PendingWork::None,
        (JournalFact::SavedPasskey, Some(owner)) => PendingWork::Paused(owner),
        (JournalFact::SavedPasskey, None)
        | (JournalFact::RecoveryOwned | JournalFact::Unreadable, _) => PendingWork::Unavailable,
    }
}

/// Names `owner` as the owner of the journal its work writes next; paused work keeps the owner
/// it has until its journal is gone
pub(crate) fn adopt(owner: ContinuationOwner) {
    let mut view = VIEW.lock();
    if view.fact != JournalFact::SavedPasskey || view.owner.is_none() {
        view.owner = Some(owner);
    }
}

pub(crate) fn mark_unreadable() {
    VIEW.lock().fact = JournalFact::Unreadable;
}

/// The journal now holds `journal`
pub(crate) fn journal_written(journal: &PendingEnableJournal) {
    VIEW.lock().fact = JournalFact::of(Some(journal));
}

/// A read found `journal`; a failed read changes nothing, since only this process writes it
///
/// The owner stays: a run adopts it before its first write, and that write reads first
pub(crate) fn journal_read(journal: Option<&PendingEnableJournal>) {
    VIEW.lock().fact = JournalFact::of(journal);
}

/// The journal is gone, and with it the work it belonged to
pub(crate) fn journal_deleted() {
    *VIEW.lock() = JournalView { fact: JournalFact::NoSavedPasskey, owner: None };
}
