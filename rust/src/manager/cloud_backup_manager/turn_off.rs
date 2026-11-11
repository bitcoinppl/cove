//! Turn Off before it commits: the review the user consents to and the impact it describes, and
//! where a committed Turn Off stands
//!
//! The review is presentation the configured body holds, not an operation: it takes no slot, and
//! any operation that starts ends it. Its impact is computed by [`DisableImpact::assess`] from
//! [`DisableEvidence`], a fresh listing and this device's wallets, classified by the inventory's
//! facts. The same function runs for the review and for the recheck after the drain, so a commit
//! only goes ahead on evidence that puts nothing more at risk than what the user confirmed

use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::sync::atomic::{AtomicU64, Ordering};

use cove_device::cloud_storage::{CloudStorage, CloudStorageClient};

use crate::database::cloud_backup::CloudStorageIssue;
use crate::manager::cloud_backup_manager::admission::{RestoreTarget, ReviewExit, UserIntent};
use crate::manager::cloud_backup_manager::inventory::{
    InventoryImpactInputs, InventoryListedClass,
};
use crate::manager::cloud_backup_manager::owner::lifecycle::{
    CommittedDisable, DisableDisposition,
};
use crate::manager::cloud_backup_manager::screen::{
    CloudBackupDisableConfirmation, CloudBackupDisableImpact, CloudBackupDisableReview,
    CloudBackupInventoryAssessment, CloudBackupNamedCount, CloudBackupTurnOffFailure,
};
use crate::manager::cloud_backup_manager::{
    CloudBackupExclusiveOperationClaim, CloudBackupInventoryIncompleteReason,
    CloudBackupVerificationSource, RecordId,
};

/// The most wallet names a dialog lists before "and N more"; set here so both apps agree
pub(crate) const DISABLE_IMPACT_NAME_CAP: usize = 3;

static NEXT_REVIEW_ID: AtomicU64 = AtomicU64::new(1);

/// Identifies one review, so a confirmation answers exactly the review it was shown
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct ReviewId(u64);

impl ReviewId {
    pub(crate) fn next() -> Self {
        Self(NEXT_REVIEW_ID.fetch_add(1, Ordering::Relaxed))
    }

    pub(crate) fn get(self) -> u64 {
        self.0
    }
}

impl From<u64> for ReviewId {
    fn from(id: u64) -> Self {
        Self(id)
    }
}

/// Turn Off between the tap and the user's consent
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum TurnOffReview {
    /// The silent listing behind the review is running
    Preparing {
        review_id: ReviewId,
    },
    /// The provider can't be reached, so the delete couldn't run either
    Offline,
    Reviewing(DisableReview),
}

impl TurnOffReview {
    /// The id of the review on screen, once it has one to consent to
    pub(crate) fn reviewing_id(&self) -> Option<ReviewId> {
        match self {
            Self::Reviewing(review) => Some(review.review_id),
            Self::Preparing { .. } | Self::Offline => None,
        }
    }
}

/// What Turn Off would delete, bound to the evidence it was computed from
///
/// The configured body holds it, and any operation start or body change drops it, so a review
/// never outlives the configuration it read; the commit also checks its namespace
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DisableReview {
    pub(crate) review_id: ReviewId,
    pub(crate) namespace_id: String,
    pub(crate) impact: DisableImpact,
    /// The recheck after the drain found more at risk than the user confirmed
    pub(crate) changed_since_confirmation: bool,
}

impl DisableReview {
    pub(crate) fn screen(&self) -> CloudBackupDisableReview {
        CloudBackupDisableReview {
            review_id: self.review_id.get(),
            impact: self.impact.screen(),
            changed_since_confirmation: self.changed_since_confirmation,
        }
    }
}

/// Fresh evidence for an impact: what the namespace lists and which wallets this device has
///
/// Gathered silently: listing never decrypts a record or asks for a passkey
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DisableEvidence {
    pub(crate) namespace_id: String,
    /// The wallet record ids the namespace lists now
    pub(crate) listing: Result<HashSet<String>, CloudStorageIssue>,
    /// The record ids of this device's wallets, or `None` when they couldn't be read
    pub(crate) local: Option<HashSet<String>>,
    /// Namespaces made with another passkey; Turn Off leaves them in place
    pub(crate) other_backups: u32,
}

/// Why evidence for an impact couldn't be gathered at all
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum EvidenceUnavailable {
    /// The provider can't be reached, so the delete couldn't run either
    Offline,
    /// This device's own configuration couldn't be read
    Local(String),
}

/// What each listed record is, as the inventory classified it
#[derive(Debug, Clone, PartialEq, Eq)]
enum ListedClass {
    NotOnDevice { name: String },
    ExtraCopy,
    Unsupported,
    Unreadable,
    Unchecked,
}

impl From<InventoryListedClass> for ListedClass {
    fn from(class: InventoryListedClass) -> Self {
        match class {
            InventoryListedClass::NotOnDevice { name } => Self::NotOnDevice { name },
            InventoryListedClass::ExtraCopy => Self::ExtraCopy,
            InventoryListedClass::Unsupported => Self::Unsupported,
            InventoryListedClass::Unreadable => Self::Unreadable,
            InventoryListedClass::Unchecked => Self::Unchecked,
        }
    }
}

/// The inventory's facts an impact classifies the fresh listing with
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ImpactFacts {
    listed: BTreeMap<String, ListedClass>,
    /// Local wallets whose secret is gone, by record id, with their names
    secret_missing_locally: BTreeMap<String, String>,
    /// Local wallets whose cloud copy couldn't be read to tell, by record id, with their names
    secret_unchecked: BTreeMap<String, String>,
    /// How complete the inventory behind these classes and secret checks is
    assessment: CloudBackupInventoryAssessment,
}

impl From<InventoryImpactInputs> for ImpactFacts {
    fn from(inputs: InventoryImpactInputs) -> Self {
        let InventoryImpactInputs { assessment, secret_missing_locally, secret_unchecked, listed } =
            inputs;
        let listed =
            listed.into_iter().map(|(record_id, class)| (record_id, class.into())).collect();

        Self { listed, secret_missing_locally, secret_unchecked, assessment }
    }
}

/// What Turn Off would delete, keyed by record id so a recheck can tell new risk from old
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DisableImpact {
    assessment: CloudBackupInventoryAssessment,
    own_backups: u32,
    /// Record id to wallet name
    not_on_device: BTreeMap<String, String>,
    /// Record id to wallet name
    own_secret_missing_locally: BTreeMap<String, String>,
    extra_copies: BTreeSet<String>,
    unchecked: BTreeSet<String>,
    unreadable: BTreeSet<String>,
    unsupported: BTreeSet<String>,
    other_backups: u32,
}

impl DisableImpact {
    /// The impact of deleting the namespace `evidence` lists, classified by `facts`
    ///
    /// A failed listing or local read never blocks: it makes the assessment unknown, which
    /// asks twice. The fresh listing proves which records are this device's own backups, but
    /// only the inventory knows what the rest are and whether an own record's cloud copy holds
    /// a secret this device lacks, so the assessment is always the inventory's: an incomplete
    /// inventory asks twice even when every listed record is this device's. An own record with
    /// an unchecked secret, or a listed record the inventory hasn't classified, counts as
    /// unchecked; an unclassified record also makes the assessment partial, since it may be a
    /// wallet that isn't on this device
    pub(crate) fn assess(evidence: &DisableEvidence, facts: &ImpactFacts) -> Self {
        let mut impact = Self {
            assessment: facts.assessment,
            own_backups: 0,
            not_on_device: BTreeMap::new(),
            own_secret_missing_locally: BTreeMap::new(),
            extra_copies: BTreeSet::new(),
            unchecked: BTreeSet::new(),
            unreadable: BTreeSet::new(),
            unsupported: BTreeSet::new(),
            other_backups: evidence.other_backups,
        };

        let (listing, local) = match (&evidence.listing, &evidence.local) {
            (Ok(listing), Some(local)) => (listing, local),
            (Err(issue), local) => {
                impact.assessment =
                    CloudBackupInventoryAssessment::Unknown { reason: (*issue).into() };
                impact.own_backups = local.as_ref().map_or(0, count);
                return impact;
            }
            (Ok(listing), None) => {
                impact.assessment = CloudBackupInventoryAssessment::Unknown {
                    reason: CloudBackupInventoryIncompleteReason::Unknown,
                };
                impact.own_backups = count(listing);
                return impact;
            }
        };

        let mut unclassified = false;
        for record_id in listing {
            if local.contains(record_id) {
                impact.own_backups += 1;
                if let Some(name) = facts.secret_missing_locally.get(record_id) {
                    impact.own_secret_missing_locally.insert(record_id.clone(), name.clone());
                }

                if facts.secret_unchecked.contains_key(record_id) {
                    impact.unchecked.insert(record_id.clone());
                }

                continue;
            }

            match facts.listed.get(record_id) {
                Some(ListedClass::NotOnDevice { name }) => {
                    impact.not_on_device.insert(record_id.clone(), name.clone());
                }
                Some(ListedClass::ExtraCopy) => {
                    impact.extra_copies.insert(record_id.clone());
                }
                Some(ListedClass::Unsupported) => {
                    impact.unsupported.insert(record_id.clone());
                }
                Some(ListedClass::Unreadable) => {
                    impact.unreadable.insert(record_id.clone());
                }
                Some(ListedClass::Unchecked) => {
                    impact.unchecked.insert(record_id.clone());
                }
                None => {
                    unclassified = true;
                    impact.unchecked.insert(record_id.clone());
                }
            }
        }

        if unclassified && impact.assessment == CloudBackupInventoryAssessment::Complete {
            impact.assessment = CloudBackupInventoryAssessment::Partial {
                reason: CloudBackupInventoryIncompleteReason::Unknown,
            };
        }

        impact
    }

    /// What Restore Them First restores: the one wallet not on this device, or all of them
    pub(crate) fn restore_target(&self) -> Option<RestoreTarget> {
        match self.not_on_device.len() {
            0 => None,
            1 => Some(RestoreTarget::OneWallet),
            _ => Some(RestoreTarget::AllWallets),
        }
    }

    /// The work a review button leaves this review for, picked here so the platform never
    /// composes a Cancel and a second intent
    pub(crate) fn exit_intent(&self, exit: ReviewExit) -> Option<UserIntent> {
        match exit {
            ReviewExit::VerifyFirst => Some(UserIntent::StartVerification {
                source: CloudBackupVerificationSource::CloudBackupDetail,
                discoverable: false,
            }),
            ReviewExit::RestoreFirst => match self.restore_target()? {
                RestoreTarget::OneWallet => self.not_on_device.keys().next().map(|record_id| {
                    UserIntent::RestoreCloudWallet(RecordId::from(record_id.clone()))
                }),
                RestoreTarget::AllWallets => Some(UserIntent::StartRestoreAll),
            },
        }
    }

    /// Whether anything could be lost, or Cove couldn't check
    pub(crate) fn confirmation(&self) -> CloudBackupDisableConfirmation {
        let at_risk = !self.not_on_device.is_empty()
            || !self.own_secret_missing_locally.is_empty()
            || !self.unchecked.is_empty()
            || !self.unreadable.is_empty()
            || !self.unsupported.is_empty();

        if at_risk || self.assessment != CloudBackupInventoryAssessment::Complete {
            return CloudBackupDisableConfirmation::Two;
        }

        CloudBackupDisableConfirmation::One
    }

    /// Whether this impact, from a recheck, puts more at risk than `confirmed`
    ///
    /// Any record in a risk category that the confirmed impact didn't have there counts, so a
    /// record that left a category can't hide one that joined it
    pub(crate) fn grew_since(&self, confirmed: &Self) -> bool {
        let new_named = |now: &BTreeMap<String, String>, then: &BTreeMap<String, String>| {
            now.keys().any(|record_id| !then.contains_key(record_id))
        };
        let new_ids = |now: &BTreeSet<String>, then: &BTreeSet<String>| !now.is_subset(then);

        new_named(&self.not_on_device, &confirmed.not_on_device)
            || new_named(&self.own_secret_missing_locally, &confirmed.own_secret_missing_locally)
            || new_ids(&self.unchecked, &confirmed.unchecked)
            || new_ids(&self.unreadable, &confirmed.unreadable)
            || new_ids(&self.unsupported, &confirmed.unsupported)
            || assessment_rank(self.assessment) > assessment_rank(confirmed.assessment)
    }

    pub(crate) fn screen(&self) -> CloudBackupDisableImpact {
        CloudBackupDisableImpact {
            assessment: self.assessment,
            own_backups: self.own_backups,
            not_on_device: named_count(&self.not_on_device),
            own_secret_missing_locally: named_count(&self.own_secret_missing_locally),
            extra_copies: count(&self.extra_copies),
            unchecked: count(&self.unchecked),
            unreadable: count(&self.unreadable),
            unsupported: count(&self.unsupported),
            other_backups: self.other_backups,
            confirmation: self.confirmation(),
        }
    }
}

/// Worse evidence ranks higher
fn assessment_rank(assessment: CloudBackupInventoryAssessment) -> u8 {
    match assessment {
        CloudBackupInventoryAssessment::Complete => 0,
        CloudBackupInventoryAssessment::Partial { .. } => 1,
        CloudBackupInventoryAssessment::Unknown { .. } => 2,
    }
}

fn count<T>(items: impl IntoIterator<Item = T>) -> u32 {
    u32::try_from(items.into_iter().count()).unwrap_or(u32::MAX)
}

/// At most [`DISABLE_IMPACT_NAME_CAP`] names in name order, and how many more there are
fn named_count(records: &BTreeMap<String, String>) -> CloudBackupNamedCount {
    let mut names = records.values().cloned().collect::<Vec<_>>();
    names.sort();

    let more = count(names.iter().skip(DISABLE_IMPACT_NAME_CAP));
    names.truncate(DISABLE_IMPACT_NAME_CAP);
    CloudBackupNamedCount { names, more }
}

/// What woke an automatic resume of a committed Turn Off
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AutomaticResume {
    /// Once per process, so it can't loop against a provider that keeps failing
    Launch,
    /// Connectivity came back or the cloud changed, which can repeat, so it waits for the last
    /// failure's `retry_after`
    CloudReachable,
}

/// Who started a finishing run that deletes the namespace, which decides whether the provider
/// may ask the user for consent on the way
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DeleteAuthority {
    /// The user confirmed the review, and the committed Turn Off goes on to delete
    Commit,
    /// A launch, a reconnect or a cloud change resumed it with nobody tapping anything
    Automatic,
    /// The user tapped Try Again, or Reconnect after the provider asked for consent
    TryAgain,
}

impl DeleteAuthority {
    /// The provider client the run deletes with
    ///
    /// A run nobody tapped for only gets a silent client, so the provider can't show its consent
    /// sheet: an automatic resume that needs consent fails with `AuthorizationRequired` and waits
    /// for the user's Reconnect or Try Again
    pub(crate) fn cloud_client(self) -> CloudStorageClient {
        match self {
            Self::Automatic => CloudStorage::global_silent_client(),
            Self::Commit | Self::TryAgain => CloudStorage::global_explicit_client(),
        }
    }
}

/// How a committed Turn Off's finishing run started
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FinishStart {
    /// Delete the namespace, then clean up this device
    Delete(DeleteAuthority),
    /// The user chose to stop using Cloud Backup on this device after a failed Try Again: the
    /// local cleanup runs and the remote backups stay
    LocalOnly,
}

impl FinishStart {
    /// How a run started with `authority` finishes a committed disable whose record holds
    /// `disposition`
    ///
    /// The disposition outranks the authority: once the user chose to keep the remote backups,
    /// no resume or retry deletes them
    pub(crate) fn of(disposition: DisableDisposition, authority: DeleteAuthority) -> Self {
        match disposition {
            DisableDisposition::DeleteRemote => Self::Delete(authority),
            DisableDisposition::LocalOnly => Self::LocalOnly,
        }
    }
}

/// The one automatic retry a committed Turn Off keeps armed for a reconnect that came before
/// its last failure's `retry_after`
///
/// The finishing body holds at most one, for its attempt and deadline: a finishing run that
/// starts clears it, and a body that moves to another attempt or out of `FinishingDisable` drops
/// it, so a wake that fires after either finds nothing to resume
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct RetryWake {
    pub(crate) attempt_id: u64,
    /// The `retry_after` it waits for, in Unix seconds
    pub(crate) at: u64,
}

/// A confirmed Turn Off, as admission produced it: the review the user consented to, taken out of
/// the idle configured body, and the drain operation allocated in its place
#[derive(Debug)]
pub(crate) struct ConfirmedTurnOff {
    pub(crate) claim: CloudBackupExclusiveOperationClaim,
    pub(crate) review: DisableReview,
}

/// A finishing run of the committed Turn Off, as admission or an automatic resume produced it:
/// the committed state the idle finishing body held, and the run allocated on it
#[derive(Debug)]
pub(crate) struct AdmittedFinish {
    pub(crate) run: FinishRun,
    pub(crate) committed: CommittedDisable,
}

/// One finishing run of a committed Turn Off: its operation and how it started
///
/// Only the runtime makes one, as it allocates the run's operation, and every completion of the
/// run carries it, so each step reads how the run started from the run itself
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct FinishRun {
    pub(crate) claim: CloudBackupExclusiveOperationClaim,
    pub(crate) start: FinishStart,
}

/// Why the last finishing run of a committed Turn Off stopped
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FinishFailure {
    Failed(CloudBackupTurnOffFailure),
    /// A Try Again has failed since the commit, so the user may stop using Cloud Backup on
    /// this device; later failures keep this
    TryAgainFailed(CloudBackupTurnOffFailure),
}

impl FinishFailure {
    /// The failure after a run that started as `start` stopped for `reason`
    pub(crate) fn after(
        previous: Option<Self>,
        start: FinishStart,
        reason: CloudBackupTurnOffFailure,
    ) -> Self {
        match (previous, start) {
            (Some(Self::TryAgainFailed(_)), _)
            | (_, FinishStart::Delete(DeleteAuthority::TryAgain) | FinishStart::LocalOnly) => {
                Self::TryAgainFailed(reason)
            }
            (_, FinishStart::Delete(DeleteAuthority::Commit | DeleteAuthority::Automatic)) => {
                Self::Failed(reason)
            }
        }
    }

    /// Whether a Try Again the user tapped has failed, which offers Stop Using on This Device
    pub(crate) fn try_again_failed(self) -> bool {
        matches!(self, Self::TryAgainFailed(_))
    }

    pub(crate) fn reason(self) -> CloudBackupTurnOffFailure {
        match self {
            Self::Failed(reason) | Self::TryAgainFailed(reason) => reason,
        }
    }
}

impl From<CloudStorageIssue> for CloudBackupTurnOffFailure {
    fn from(issue: CloudStorageIssue) -> Self {
        match issue {
            CloudStorageIssue::Offline => Self::Offline,
            CloudStorageIssue::AuthorizationRequired => Self::AuthorizationRequired,
            CloudStorageIssue::Unavailable
            | CloudStorageIssue::SyncPending
            | CloudStorageIssue::QuotaExceeded => Self::ProviderUnavailable,
            CloudStorageIssue::NotFound | CloudStorageIssue::Other => Self::Other,
        }
    }
}

#[cfg(test)]
pub(crate) mod test_support {
    use super::*;

    /// A review of an empty backup under `namespace_id`, from a complete inventory
    pub(crate) fn empty_review(namespace_id: &str) -> DisableReview {
        empty_review_assessed(namespace_id, CloudBackupInventoryAssessment::Complete)
    }

    /// A review of an empty backup under `namespace_id`, from an inventory as complete as
    /// `assessment`
    pub(crate) fn empty_review_assessed(
        namespace_id: &str,
        assessment: CloudBackupInventoryAssessment,
    ) -> DisableReview {
        let evidence = empty_evidence(namespace_id);
        let facts = ImpactFacts {
            listed: BTreeMap::new(),
            secret_missing_locally: BTreeMap::new(),
            secret_unchecked: BTreeMap::new(),
            assessment,
        };

        DisableReview {
            review_id: ReviewId::next(),
            namespace_id: namespace_id.to_owned(),
            impact: DisableImpact::assess(&evidence, &facts),
            changed_since_confirmation: false,
        }
    }

    /// A review of a backup under `namespace_id` that lists one wallet not on this device for
    /// each of `names`, from a complete inventory
    pub(crate) fn review_not_on_device(namespace_id: &str, names: &[&str]) -> DisableReview {
        let record_id = |name: &str| format!("record-{name}");
        let evidence = DisableEvidence {
            listing: Ok(names.iter().map(|&name| record_id(name)).collect()),
            ..empty_evidence(namespace_id)
        };
        let facts = ImpactFacts {
            listed: names
                .iter()
                .map(|&name| (record_id(name), ListedClass::NotOnDevice { name: name.into() }))
                .collect(),
            secret_missing_locally: BTreeMap::new(),
            secret_unchecked: BTreeMap::new(),
            assessment: CloudBackupInventoryAssessment::Complete,
        };

        DisableReview {
            review_id: ReviewId::next(),
            namespace_id: namespace_id.to_owned(),
            impact: DisableImpact::assess(&evidence, &facts),
            changed_since_confirmation: false,
        }
    }

    /// Evidence of an empty backup under `namespace_id` and a device with no wallets
    pub(crate) fn empty_evidence(namespace_id: &str) -> DisableEvidence {
        DisableEvidence {
            namespace_id: namespace_id.to_owned(),
            listing: Ok(HashSet::new()),
            local: Some(HashSet::new()),
            other_backups: 0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ids(ids: &[&str]) -> HashSet<String> {
        ids.iter().map(|id| (*id).to_owned()).collect()
    }

    fn evidence(listing: &[&str], local: &[&str]) -> DisableEvidence {
        DisableEvidence {
            namespace_id: "ns".into(),
            listing: Ok(ids(listing)),
            local: Some(ids(local)),
            other_backups: 0,
        }
    }

    fn facts(listed: &[(&str, ListedClass)]) -> ImpactFacts {
        ImpactFacts {
            listed: listed.iter().map(|(id, class)| ((*id).to_owned(), class.clone())).collect(),
            secret_missing_locally: BTreeMap::new(),
            secret_unchecked: BTreeMap::new(),
            assessment: CloudBackupInventoryAssessment::Complete,
        }
    }

    fn not_on_device(name: &str) -> ListedClass {
        ListedClass::NotOnDevice { name: name.into() }
    }

    #[test]
    fn only_own_backups_and_extra_copies_ask_once() {
        let impact = DisableImpact::assess(
            &evidence(&["a", "b", "x"], &["a", "b"]),
            &facts(&[("x", ListedClass::ExtraCopy)]),
        );

        let screen = impact.screen();
        assert_eq!((screen.own_backups, screen.extra_copies), (2, 1));
        assert_eq!(screen.confirmation, CloudBackupDisableConfirmation::One);
    }

    #[test]
    fn turn_off_asks_twice_until_the_inventory_is_complete() {
        let mut partial = facts(&[]);
        partial.assessment = CloudBackupInventoryAssessment::Partial {
            reason: CloudBackupInventoryIncompleteReason::Unknown,
        };

        let mut secret_unchecked = facts(&[]);
        secret_unchecked.secret_unchecked.insert("a".into(), "Wallet A".into());

        // the listing proves "a" is this device's own, but not what its cloud copy holds
        let empty = DisableImpact::assess(&evidence(&[], &[]), &partial).screen();
        let own_only = DisableImpact::assess(&evidence(&["a"], &["a"]), &partial).screen();
        let unchecked =
            DisableImpact::assess(&evidence(&["a"], &["a"]), &secret_unchecked).screen();
        let complete = DisableImpact::assess(&evidence(&["a"], &["a"]), &facts(&[])).screen();

        assert!(matches!(empty.assessment, CloudBackupInventoryAssessment::Partial { .. }));
        assert_eq!(empty.confirmation, CloudBackupDisableConfirmation::Two);
        assert!(matches!(own_only.assessment, CloudBackupInventoryAssessment::Partial { .. }));
        assert_eq!(own_only.confirmation, CloudBackupDisableConfirmation::Two);
        assert_eq!(
            (unchecked.unchecked, unchecked.confirmation),
            (1, CloudBackupDisableConfirmation::Two)
        );
        assert_eq!(complete.assessment, CloudBackupInventoryAssessment::Complete);
        assert_eq!(complete.confirmation, CloudBackupDisableConfirmation::One);
    }

    #[test]
    fn names_are_capped_with_a_count_of_the_rest() {
        let listed = ["w1", "w2", "w3", "w4", "w5"];
        let classes = listed
            .iter()
            .map(|id| (*id, not_on_device(&format!("Wallet {id}"))))
            .collect::<Vec<_>>();
        let impact = DisableImpact::assess(&evidence(&listed, &[]), &facts(&classes));

        let screen = impact.screen();
        assert_eq!(screen.not_on_device.names, ["Wallet w1", "Wallet w2", "Wallet w3"]);
        assert_eq!(screen.not_on_device.more, 2);
        assert_eq!(screen.confirmation, CloudBackupDisableConfirmation::Two);
    }

    #[test]
    fn a_failed_listing_with_no_rows_asks_twice() {
        let failed =
            DisableEvidence { listing: Err(CloudStorageIssue::Unavailable), ..evidence(&[], &[]) };
        let impact = DisableImpact::assess(&failed, &facts(&[]));

        let screen = impact.screen();
        assert!(matches!(screen.assessment, CloudBackupInventoryAssessment::Unknown { .. }));
        assert_eq!(screen.confirmation, CloudBackupDisableConfirmation::Two);
    }

    #[test]
    fn an_unclassified_record_is_unchecked_and_partial() {
        let impact = DisableImpact::assess(&evidence(&["a", "new"], &["a"]), &facts(&[]));

        let screen = impact.screen();
        assert_eq!(screen.unchecked, 1);
        assert!(matches!(screen.assessment, CloudBackupInventoryAssessment::Partial { .. }));
        assert_eq!(screen.confirmation, CloudBackupDisableConfirmation::Two);
    }

    #[test]
    fn an_own_record_whose_secret_is_missing_is_at_risk() {
        let mut facts = facts(&[]);
        facts.secret_missing_locally.insert("a".into(), "Savings".into());
        let impact = DisableImpact::assess(&evidence(&["a"], &["a"]), &facts);

        let screen = impact.screen();
        assert_eq!(screen.own_secret_missing_locally.names, ["Savings"]);
        assert_eq!(screen.confirmation, CloudBackupDisableConfirmation::Two);
    }

    #[test]
    fn a_swapped_record_in_a_risk_category_counts_as_growth() {
        let facts = facts(&[("x", not_on_device("X")), ("y", not_on_device("Y"))]);
        let confirmed = DisableImpact::assess(&evidence(&["x"], &[]), &facts);
        let same = DisableImpact::assess(&evidence(&["x"], &[]), &facts);
        let swapped = DisableImpact::assess(&evidence(&["y"], &[]), &facts);
        let fewer = DisableImpact::assess(&evidence(&[], &[]), &facts);

        assert!(!same.grew_since(&confirmed));
        assert!(swapped.grew_since(&confirmed));
        assert!(!fewer.grew_since(&confirmed));
    }

    #[test]
    fn worse_evidence_counts_as_growth() {
        let confirmed = DisableImpact::assess(&evidence(&["a"], &["a"]), &facts(&[]));
        let failed = DisableImpact::assess(
            &DisableEvidence { listing: Err(CloudStorageIssue::Other), ..evidence(&[], &["a"]) },
            &facts(&[]),
        );

        assert!(failed.grew_since(&confirmed));
        assert!(!confirmed.grew_since(&failed));
    }

    #[test]
    fn a_failed_try_again_stays_failed_after_try_again() {
        use CloudBackupTurnOffFailure as Reason;

        let start = |authority| FinishStart::Delete(authority);
        let first = FinishFailure::after(None, start(DeleteAuthority::Commit), Reason::Offline);
        assert_eq!(first, FinishFailure::Failed(Reason::Offline));

        let tapped =
            FinishFailure::after(Some(first), start(DeleteAuthority::TryAgain), Reason::Other);
        assert_eq!(tapped, FinishFailure::TryAgainFailed(Reason::Other));

        let later =
            FinishFailure::after(Some(tapped), start(DeleteAuthority::Automatic), Reason::Offline);
        assert_eq!(later, FinishFailure::TryAgainFailed(Reason::Offline));
    }
}
