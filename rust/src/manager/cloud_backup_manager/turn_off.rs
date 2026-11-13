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
use cove_device::keychain::Keychain;
use tracing::warn;

use crate::database::cloud_backup::CloudStorageIssue;
use crate::manager::cloud_backup_manager::admission::{RestoreTarget, ReviewExit, UserIntent};
use crate::manager::cloud_backup_manager::inventory::{
    CopyCover, InventoryImpactInputs, InventoryListedClass,
};
use crate::manager::cloud_backup_manager::owner::lifecycle::{
    CommittedDisable, DisableDisposition,
};
use crate::manager::cloud_backup_manager::screen::{
    CloudBackupDisableConfirmation, CloudBackupDisableImpact, CloudBackupDisableReview,
    CloudBackupInventoryAssessment, CloudBackupNamedCount, CloudBackupTurnOffFailure,
};
use crate::manager::cloud_backup_manager::wallets::LocalSecret;
use crate::manager::cloud_backup_manager::{
    ClassificationRequestId, CloudBackupExclusiveOperationClaim,
    CloudBackupInventoryIncompleteReason, CloudBackupVerificationSource, CloudCopySecret,
    InventoryReadAt, RecordId,
};
use crate::wallet::metadata::WalletMetadata;
use crate::wallet_identity::{
    BackupRecoveryMaterial, LocalWalletIdentities, local_wallets_hold_seed_of,
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

/// Check Again's classification request carries the id of the review that waits for it
impl From<ReviewId> for ClassificationRequestId {
    fn from(review_id: ReviewId) -> Self {
        Self::from(review_id.0)
    }
}

/// Turn Off between the tap and the user's consent
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum TurnOffReview {
    /// The evidence behind the review is being gathered
    Preparing {
        review_id: ReviewId,
        stage: PreparingStage,
    },
    /// The provider can't be reached, so the delete couldn't run either
    Offline,
    // boxed since the impact's record sets dwarf the other variants
    Reviewing(Box<DisableReview>),
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

/// Where a review being prepared is
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PreparingStage {
    /// Check Again asked the inventory for a fresh classification, and the review waits for it
    /// to settle or for the evidence deadline, whichever comes first
    Classifying,
    /// The silent listing and the read of this device's wallets are running
    Gathering,
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

/// Fresh evidence for an impact: what the namespace lists and this device's wallets with their
/// secrets, each read when the evidence was gathered
///
/// Gathered silently: listing never decrypts a record or asks for a passkey
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DisableEvidence {
    pub(crate) namespace_id: String,
    /// The wallet record ids the namespace lists now
    pub(crate) listing: ListingEvidence,
    /// This device's wallets and the secrets each holds now
    pub(crate) local: LocalEvidence,
    /// Namespaces made with another passkey; Turn Off leaves them in place
    pub(crate) other_backups: OtherBackupsCount,
}

/// What the namespace's wallet listing returned
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ListingEvidence {
    Listed(HashSet<String>),
    Failed(CloudStorageIssue),
    /// The provider didn't answer before the evidence deadline
    TimedOut,
}

impl ListingEvidence {
    /// Why the listing tells nothing, or `None` when it listed
    fn incomplete_reason(&self) -> Option<CloudBackupInventoryIncompleteReason> {
        match self {
            Self::Listed(_) => None,
            Self::Failed(issue) => Some((*issue).into()),
            Self::TimedOut => Some(CloudBackupInventoryIncompleteReason::ProviderUnavailable),
        }
    }
}

/// This device's wallets, read when the evidence was gathered
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum LocalEvidence {
    /// Read with the inventory at `read_at`, so a local wallet change or a new configuration the
    /// inventory applies later makes this read stale
    Read { read_at: InventoryReadAt, wallets: BTreeMap<String, LocalWallet> },
    /// The wallets couldn't be read
    Unread,
}

/// One of this device's wallets, by its own record id
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct LocalWallet {
    pub(crate) name: String,
    pub(crate) secret: LocalSecret,
    /// Whether the local wallets hold the seed this wallet's own backup could hold, which
    /// decides an own record the inventory hasn't read
    pub(crate) own_seed: OwnSeedCoverage,
}

/// Whether the local wallets hold the seed behind a wallet's own backup, by the wallet identity
/// coverage check a restore uses
///
/// A keychain read error is unknown, never held, and a public-only wallet never holds a seed
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum OwnSeedCoverage {
    /// A local hot wallet holds the seed
    Held,
    /// No local wallet holds the seed, as for a public-only wallet or a hot wallet whose secret
    /// is gone
    NotHeld,
    /// The local wallets or their keychain items couldn't be read
    Unknown,
}

impl OwnSeedCoverage {
    /// Reads `metadata`'s coverage from `identities`, the local wallets' identities read for this
    /// evidence; `None` when that read failed, which leaves every wallet unknown
    pub(crate) fn read(
        identities: Option<&LocalWalletIdentities>,
        metadata: &WalletMetadata,
        keychain: &Keychain,
    ) -> Self {
        let Some(identities) = identities else { return Self::Unknown };

        match local_wallets_hold_seed_of(identities, metadata, keychain) {
            Ok(true) => Self::Held,
            Ok(false) => Self::NotHeld,
            Err(error) => {
                let id = &metadata.id;
                warn!("Turn Off could not tell whether wallet {id} holds its seed: {error}");
                Self::Unknown
            }
        }
    }
}

/// How many namespaces another passkey made, which the dialog shows as information only
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum OtherBackupsCount {
    Counted(u32),
    /// The count failed, or hadn't answered when the listing was in
    Unknown,
}

impl OtherBackupsCount {
    /// The count the dialog shows; an unknown count leaves the line out
    fn shown(self) -> u32 {
        match self {
            Self::Counted(count) => count,
            Self::Unknown => 0,
        }
    }
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
    NotOnDevice {
        name: String,
    },
    /// A copy local wallets covered when the inventory read it
    ExtraCopy(CopyCover),
    Unsupported,
    Unreadable,
    Unchecked,
}

impl From<InventoryListedClass> for ListedClass {
    fn from(class: InventoryListedClass) -> Self {
        match class {
            InventoryListedClass::NotOnDevice { name } => Self::NotOnDevice { name },
            InventoryListedClass::ExtraCopy(cover) => Self::ExtraCopy(cover),
            InventoryListedClass::Unsupported => Self::Unsupported,
            InventoryListedClass::Unreadable => Self::Unreadable,
            InventoryListedClass::Unchecked => Self::Unchecked,
        }
    }
}

/// The inventory's facts an impact classifies the fresh listing and this device's wallets with
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ImpactFacts {
    listed: BTreeMap<String, ListedClass>,
    /// What each own record's cloud copy held when the inventory read it
    own_cloud_copies: BTreeMap<String, CloudCopySecret>,
    /// How complete the inventory behind these classes is
    assessment: CloudBackupInventoryAssessment,
    /// The configuration and the local wallet changes the inventory reflects; a local read from
    /// any other point is stale
    read_at: InventoryReadAt,
}

impl From<InventoryImpactInputs> for ImpactFacts {
    fn from(inputs: InventoryImpactInputs) -> Self {
        let InventoryImpactInputs { assessment, own_cloud_copies, listed, read_at } = inputs;
        let listed =
            listed.into_iter().map(|(record_id, class)| (record_id, class.into())).collect();

        Self { listed, own_cloud_copies, assessment, read_at }
    }
}

/// What Turn Off would delete, keyed by record id so a recheck can tell new risk from old
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DisableImpact {
    assessment: CloudBackupInventoryAssessment,
    own_backups: u32,
    /// The listed records whose wallet is on this device, so a recheck can tell a wallet that
    /// left since the review
    own_records: BTreeSet<String>,
    /// Record id to wallet name
    not_on_device: BTreeMap<String, String>,
    /// Local wallets whose secret this device lacks while a listed record holds it, their own
    /// backup or an extra copy, by the wallet's own record id, with names
    own_secret_missing_locally: BTreeMap<String, String>,
    extra_copies: BTreeSet<String>,
    /// Records whose risk couldn't be told: ones the inventory couldn't classify, own ones whose
    /// cloud copy wasn't read or whose secret this device couldn't read, and extra copies no
    /// wallet here is known to still cover
    unchecked: BTreeSet<String>,
    unreadable: BTreeSet<String>,
    unsupported: BTreeSet<String>,
    other_backups: u32,
    /// Present only when every listed record is known safe, which is all that asks once
    known_safe: Option<KnownSafe>,
}

/// What deleting one listed record risks, as the fresh evidence tells it
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RecordRisk {
    /// Read, and this device holds everything it can recover
    Safe,
    /// It holds what this device doesn't, or is a kind Cove can't recover from here
    AtRisk,
    /// What it holds or what this device holds couldn't be read
    Untold,
}

/// The proof that lets Turn Off ask once: a complete inventory, a current read of this device's
/// wallets, a namespace that lists at least one record, and every listed record read and known
/// safe to delete
///
/// Only [`KnownSafe::prove`] makes one, from the risks [`DisableImpact::assess`] told for every
/// listed record, so an empty backup, or one record at risk or untold, asks twice
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct KnownSafe(());

impl KnownSafe {
    fn prove(
        assessment: CloudBackupInventoryAssessment,
        risks: impl IntoIterator<Item = RecordRisk>,
    ) -> Option<Self> {
        if assessment != CloudBackupInventoryAssessment::Complete {
            return None;
        }

        let mut listed_any = false;
        for risk in risks {
            if risk != RecordRisk::Safe {
                return None;
            }

            listed_any = true;
        }

        listed_any.then_some(Self(()))
    }
}

impl DisableImpact {
    /// The impact of deleting the namespace `evidence` lists, classified by `facts`
    ///
    /// A failed, late or stale listing or local read never blocks: it makes the assessment
    /// unknown, which asks twice. The fresh listing and local read prove which records are this
    /// device's own backups and whether this device holds each one's secret now, and whether the
    /// wallets that covered each extra copy still hold what it can recover; only the inventory
    /// knows what the rest are, what each own record's cloud copy holds and which wallets cover
    /// each copy, so the assessment is the inventory's: an incomplete inventory asks twice even
    /// when every listed record is this device's. A record whose risk couldn't be told counts as
    /// unchecked and makes the assessment partial
    pub(crate) fn assess(evidence: &DisableEvidence, facts: &ImpactFacts) -> Self {
        let mut impact = Self {
            assessment: facts.assessment,
            own_backups: 0,
            own_records: BTreeSet::new(),
            not_on_device: BTreeMap::new(),
            own_secret_missing_locally: BTreeMap::new(),
            extra_copies: BTreeSet::new(),
            unchecked: BTreeSet::new(),
            unreadable: BTreeSet::new(),
            unsupported: BTreeSet::new(),
            other_backups: evidence.other_backups.shown(),
            known_safe: None,
        };

        // a read from before a wallet change the inventory applied since may miss a secret the
        // change removed, and one from another configuration describes other wallets, so either
        // proves nothing about the wallets now
        let local = match &evidence.local {
            LocalEvidence::Read { read_at, wallets } if *read_at == facts.read_at => Some(wallets),
            LocalEvidence::Read { .. } | LocalEvidence::Unread => None,
        };

        let (listing, local) = match (&evidence.listing, local) {
            (ListingEvidence::Listed(listing), Some(local)) => (listing, local),
            (ListingEvidence::Listed(listing), None) => {
                impact.assessment = CloudBackupInventoryAssessment::Unknown {
                    reason: CloudBackupInventoryIncompleteReason::Unknown,
                };
                impact.own_backups = count(listing);
                return impact;
            }
            (listing, local) => {
                let reason = listing
                    .incomplete_reason()
                    .unwrap_or(CloudBackupInventoryIncompleteReason::Unknown);
                impact.assessment = CloudBackupInventoryAssessment::Unknown { reason };
                impact.own_backups = local.map_or(0, |local| count(local.keys()));
                return impact;
            }
        };

        let risks = listing
            .iter()
            .map(|record_id| match local.get(record_id) {
                Some(wallet) => {
                    impact.own_backups += 1;
                    impact.own_records.insert(record_id.clone());
                    let copy = facts.own_cloud_copies.get(record_id);
                    impact.classify_own(record_id, wallet, copy)
                }
                None => impact.classify_listed(record_id, facts.listed.get(record_id), local),
            })
            .collect::<Vec<_>>();

        let untold = risks.contains(&RecordRisk::Untold);
        if untold && impact.assessment == CloudBackupInventoryAssessment::Complete {
            impact.assessment = CloudBackupInventoryAssessment::Partial {
                reason: CloudBackupInventoryIncompleteReason::Unknown,
            };
        }

        impact.known_safe = KnownSafe::prove(impact.assessment, risks);
        impact
    }

    /// Whether deleting an own record loses a secret this device doesn't hold now
    ///
    /// A record the inventory hasn't read yet, such as the one Turn Off's own drain just
    /// uploaded, is safe while the coverage check finds this device holds the wallet's seed: its
    /// record ID is derived from the wallet ID, so whatever it holds is this wallet's, and the
    /// seed stays here. A copy the inventory tried and couldn't read stays untold, since that
    /// failure is itself unexplained
    fn classify_own(
        &mut self,
        record_id: &str,
        wallet: &LocalWallet,
        copy: Option<&CloudCopySecret>,
    ) -> RecordRisk {
        match (wallet.secret, copy) {
            // this device keeps the secret, or the copy never held one
            (LocalSecret::Held, Some(CloudCopySecret::Held))
            | (_, Some(CloudCopySecret::WatchOnly)) => RecordRisk::Safe,

            (_, None) if wallet.own_seed == OwnSeedCoverage::Held => RecordRisk::Safe,

            (LocalSecret::Missing | LocalSecret::NeverHeld, Some(CloudCopySecret::Held)) => {
                self.own_secret_missing_locally.insert(record_id.to_owned(), wallet.name.clone());
                RecordRisk::AtRisk
            }

            (_, None | Some(CloudCopySecret::Unread))
            | (LocalSecret::Unreadable, Some(CloudCopySecret::Held)) => {
                self.unchecked.insert(record_id.to_owned());
                RecordRisk::Untold
            }
        }
    }

    /// What deleting a listed record that isn't this device's own backup risks
    fn classify_listed(
        &mut self,
        record_id: &str,
        class: Option<&ListedClass>,
        local: &BTreeMap<String, LocalWallet>,
    ) -> RecordRisk {
        let record_id = record_id.to_owned();
        match class {
            Some(ListedClass::NotOnDevice { name }) => {
                self.not_on_device.insert(record_id, name.clone());
                RecordRisk::AtRisk
            }
            Some(ListedClass::ExtraCopy(cover)) => self.classify_copy(record_id, cover, local),
            Some(ListedClass::Unsupported) => {
                self.unsupported.insert(record_id);
                RecordRisk::AtRisk
            }
            Some(ListedClass::Unreadable) => {
                self.unreadable.insert(record_id);
                RecordRisk::AtRisk
            }
            Some(ListedClass::Unchecked) => {
                self.unchecked.insert(record_id);
                RecordRisk::AtRisk
            }
            None => {
                self.unchecked.insert(record_id);
                RecordRisk::Untold
            }
        }
    }

    /// Whether this device still holds what an extra copy can recover, by the wallets that
    /// covered it when the inventory read it
    ///
    /// The inventory's coverage is from its own read, and a covering wallet can lose its secret
    /// without the inventory hearing of it, so the copy is judged by the same read of this
    /// device's wallets the own records are. A copy whose covering wallets all lack its secret
    /// names them, as the cloud may hold their only copy
    fn classify_copy(
        &mut self,
        record_id: String,
        cover: &CopyCover,
        local: &BTreeMap<String, LocalWallet>,
    ) -> RecordRisk {
        let covering = cover
            .covered_by
            .iter()
            .filter_map(|own_record_id| local.get_key_value(&String::from(own_record_id.clone())))
            .collect::<Vec<_>>();
        let holds = |wallet: &LocalWallet| match cover.material {
            // a wallet on this device holds its own public keys
            BackupRecoveryMaterial::PublicOnly => true,
            BackupRecoveryMaterial::Secret => wallet.secret == LocalSecret::Held,
        };

        if covering.iter().any(|(_, wallet)| holds(wallet)) {
            self.extra_copies.insert(record_id);
            return RecordRisk::Safe;
        }

        // no covering wallet is left here, or one whose secret couldn't be read may still hold it
        let unreadable =
            covering.iter().any(|(_, wallet)| wallet.secret == LocalSecret::Unreadable);
        if covering.is_empty() || unreadable {
            self.unchecked.insert(record_id);
            return RecordRisk::Untold;
        }

        for (own_record_id, wallet) in covering {
            self.own_secret_missing_locally.insert(own_record_id.clone(), wallet.name.clone());
        }

        RecordRisk::AtRisk
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

    /// One confirmation only with the proof that every listed record is known safe; anything
    /// that could be lost, anything Cove couldn't check, and an empty backup ask twice
    pub(crate) fn confirmation(&self) -> CloudBackupDisableConfirmation {
        match self.known_safe {
            Some(KnownSafe(())) => CloudBackupDisableConfirmation::One,
            None => CloudBackupDisableConfirmation::Two,
        }
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

        // a record whose wallet left this device since the review holds the only copy now, even
        // when it sits in a risk category it was already in, such as one unchecked both times
        let left_at_risk = confirmed
            .own_records
            .difference(&self.own_records)
            .any(|record_id| self.at_risk(record_id));

        left_at_risk
            || new_named(&self.not_on_device, &confirmed.not_on_device)
            || new_named(&self.own_secret_missing_locally, &confirmed.own_secret_missing_locally)
            || new_ids(&self.unchecked, &confirmed.unchecked)
            || new_ids(&self.unreadable, &confirmed.unreadable)
            || new_ids(&self.unsupported, &confirmed.unsupported)
            || assessment_rank(self.assessment) > assessment_rank(confirmed.assessment)
    }

    /// Whether a listed record that isn't this device's own is in a risk category
    fn at_risk(&self, record_id: &str) -> bool {
        self.not_on_device.contains_key(record_id)
            || self.unchecked.contains(record_id)
            || self.unreadable.contains(record_id)
            || self.unsupported.contains(record_id)
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

/// What committing a confirmed Turn Off did
///
/// Only the runtime makes one, as it writes the record and moves the body in one step, so a
/// commit always comes with the run that finishes it
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum TurnOffCommit {
    /// The record moved to Disabling and the body to the committed disable, where `run` deletes
    Committed { run: FinishRun, committed: CommittedDisable },
    /// The namespace changed after the review; nothing was written, and the Turn Off still runs
    StaleReview,
    /// Cloud Backup is no longer configured; nothing was written, and the Turn Off still runs
    NotConfigured,
    /// The claim no longer runs a Turn Off, so nothing was written
    NotCurrent,
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
            own_cloud_copies: BTreeMap::new(),
            assessment,
            read_at: InventoryReadAt::default(),
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
            listing: ListingEvidence::Listed(names.iter().map(|&name| record_id(name)).collect()),
            ..empty_evidence(namespace_id)
        };
        let facts = ImpactFacts {
            listed: names
                .iter()
                .map(|&name| (record_id(name), ListedClass::NotOnDevice { name: name.into() }))
                .collect(),
            own_cloud_copies: BTreeMap::new(),
            assessment: CloudBackupInventoryAssessment::Complete,
            read_at: InventoryReadAt::default(),
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
            listing: ListingEvidence::Listed(HashSet::new()),
            local: LocalEvidence::Read {
                read_at: InventoryReadAt::default(),
                wallets: BTreeMap::new(),
            },
            other_backups: OtherBackupsCount::Counted(0),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ids(ids: &[&str]) -> HashSet<String> {
        ids.iter().map(|id| (*id).to_owned()).collect()
    }

    /// Evidence listing `listing`, from a device whose wallets `local` each hold their secret
    fn evidence(listing: &[&str], local: &[&str]) -> DisableEvidence {
        let wallets = local.iter().map(|id| (*id, LocalSecret::Held)).collect::<Vec<_>>();
        evidence_with_secrets(listing, &wallets)
    }

    /// The coverage check's answer for a wallet whose keychain read gave `secret`
    fn own_seed_like(secret: LocalSecret) -> OwnSeedCoverage {
        match secret {
            LocalSecret::Held => OwnSeedCoverage::Held,
            LocalSecret::Unreadable => OwnSeedCoverage::Unknown,
            LocalSecret::Missing | LocalSecret::NeverHeld => OwnSeedCoverage::NotHeld,
        }
    }

    fn evidence_with_secrets(listing: &[&str], local: &[(&str, LocalSecret)]) -> DisableEvidence {
        let wallets = local
            .iter()
            .map(|(id, secret)| {
                let own_seed = own_seed_like(*secret);
                let wallet =
                    LocalWallet { name: format!("Wallet {id}"), secret: *secret, own_seed };
                ((*id).to_owned(), wallet)
            })
            .collect();

        DisableEvidence {
            namespace_id: "ns".into(),
            listing: ListingEvidence::Listed(ids(listing)),
            local: LocalEvidence::Read { read_at: InventoryReadAt::default(), wallets },
            other_backups: OtherBackupsCount::Counted(0),
        }
    }

    fn facts(listed: &[(&str, ListedClass)]) -> ImpactFacts {
        ImpactFacts {
            listed: listed.iter().map(|(id, class)| ((*id).to_owned(), class.clone())).collect(),
            own_cloud_copies: BTreeMap::new(),
            assessment: CloudBackupInventoryAssessment::Complete,
            read_at: InventoryReadAt::default(),
        }
    }

    /// Facts from an inventory that read each of `own` and found its cloud copy holds a secret
    fn facts_read(own: &[&str], listed: &[(&str, ListedClass)]) -> ImpactFacts {
        let mut facts = facts(listed);
        facts.own_cloud_copies =
            own.iter().map(|id| ((*id).to_owned(), CloudCopySecret::Held)).collect();

        facts
    }

    fn not_on_device(name: &str) -> ListedClass {
        ListedClass::NotOnDevice { name: name.into() }
    }

    /// A copy the wallet with own record `own` covered when the inventory read it
    fn extra_copy(own: &str, material: BackupRecoveryMaterial) -> ListedClass {
        ListedClass::ExtraCopy(CopyCover {
            covered_by: vec![RecordId::from(own.to_owned())],
            material,
        })
    }

    #[test]
    fn only_own_backups_and_extra_copies_ask_once() {
        let impact = DisableImpact::assess(
            &evidence(&["a", "b", "x"], &["a", "b"]),
            &facts_read(&["a", "b"], &[("x", extra_copy("a", BackupRecoveryMaterial::Secret))]),
        );

        let screen = impact.screen();
        assert_eq!((screen.own_backups, screen.extra_copies), (2, 1));
        assert_eq!(screen.confirmation, CloudBackupDisableConfirmation::One);
    }

    /// A backup Cove never read, and one with nothing in it, are not a known safe set, so each
    /// asks twice
    #[test]
    fn an_empty_backup_or_an_unread_own_record_asks_twice() {
        let empty = DisableImpact::assess(&evidence(&[], &["a"]), &facts(&[])).screen();
        assert_eq!(empty.assessment, CloudBackupInventoryAssessment::Complete);
        assert_eq!(empty.confirmation, CloudBackupDisableConfirmation::Two);

        // this device holds the seed, but the inventory couldn't read or doesn't support the copy
        let mut unread = facts(&[]);
        unread.own_cloud_copies.insert("a".into(), CloudCopySecret::Unread);
        let unread = DisableImpact::assess(&evidence(&["a"], &["a"]), &unread).screen();
        assert_eq!(unread.unchecked, 1);
        assert!(matches!(unread.assessment, CloudBackupInventoryAssessment::Partial { .. }));
        assert_eq!(unread.confirmation, CloudBackupDisableConfirmation::Two);
    }

    /// A record the inventory hasn't read yet, as Turn Off's own drain leaves one, is safe only
    /// while the coverage check finds this device holds its wallet's seed, whatever the plain
    /// keychain read said
    #[test]
    fn an_own_record_the_inventory_has_not_read_is_safe_only_with_the_seed_here() {
        let not_read = |own_seed| {
            let wallet =
                LocalWallet { name: "Wallet a".into(), secret: LocalSecret::Held, own_seed };
            let local = LocalEvidence::Read {
                read_at: InventoryReadAt::default(),
                wallets: BTreeMap::from([("a".to_owned(), wallet)]),
            };

            let evidence = DisableEvidence { local, ..evidence(&["a"], &[]) };
            DisableImpact::assess(&evidence, &facts(&[])).screen()
        };

        let held = not_read(OwnSeedCoverage::Held);
        assert_eq!((held.own_backups, held.unchecked), (1, 0));
        assert_eq!(held.assessment, CloudBackupInventoryAssessment::Complete);
        assert_eq!(held.confirmation, CloudBackupDisableConfirmation::One);

        // a public-only wallet, or a keychain the coverage check couldn't read
        for own_seed in [OwnSeedCoverage::NotHeld, OwnSeedCoverage::Unknown] {
            let untold = not_read(own_seed);
            assert_eq!(untold.unchecked, 1, "{own_seed:?}");
            assert!(matches!(untold.assessment, CloudBackupInventoryAssessment::Partial { .. }));
            assert_eq!(untold.confirmation, CloudBackupDisableConfirmation::Two, "{own_seed:?}");
        }
    }

    /// An extra copy's covering wallet is judged by the same keychain read as the own records,
    /// though the copy's record id isn't any local wallet's
    #[test]
    fn an_extra_copy_whose_covering_wallet_lost_its_seed_is_at_risk() {
        let seed_copy = facts(&[("x", extra_copy("w", BackupRecoveryMaterial::Secret))]);
        let assess = |secret, facts: &ImpactFacts| {
            DisableImpact::assess(&evidence_with_secrets(&["x"], &[("w", secret)]), facts)
        };

        let covered = assess(LocalSecret::Held, &seed_copy);
        assert_eq!(covered.screen().extra_copies, 1);
        assert_eq!(covered.confirmation(), CloudBackupDisableConfirmation::One);

        let lost = assess(LocalSecret::Missing, &seed_copy);
        assert_eq!(lost.screen().own_secret_missing_locally.names, ["Wallet w"]);
        assert_eq!(lost.screen().extra_copies, 0);
        assert_eq!(lost.confirmation(), CloudBackupDisableConfirmation::Two);
        assert!(lost.grew_since(&covered));

        let unreadable = assess(LocalSecret::Unreadable, &seed_copy).screen();
        assert_eq!((unreadable.unchecked, unreadable.extra_copies), (1, 0));

        // a watch-only copy needs only the wallet, not its seed
        let watch_only = facts(&[("x", extra_copy("w", BackupRecoveryMaterial::PublicOnly))]);
        let public = assess(LocalSecret::Missing, &watch_only);
        assert_eq!(public.confirmation(), CloudBackupDisableConfirmation::One);

        // no covering wallet is left on this device
        let gone = DisableImpact::assess(&evidence(&["x"], &[]), &seed_copy).screen();
        assert_eq!((gone.unchecked, gone.extra_copies), (1, 0));
        assert_eq!(gone.confirmation, CloudBackupDisableConfirmation::Two);
    }

    #[test]
    fn turn_off_asks_twice_until_the_inventory_is_complete() {
        let mut partial = facts(&[]);
        partial.assessment = CloudBackupInventoryAssessment::Partial {
            reason: CloudBackupInventoryIncompleteReason::Unknown,
        };

        let mut copy_unread = facts(&[]);
        copy_unread.own_cloud_copies.insert("a".into(), CloudCopySecret::Unread);

        // the listing proves "a" is this device's own, but not what its cloud copy holds
        let empty = DisableImpact::assess(&evidence(&[], &[]), &partial).screen();
        let own_only = DisableImpact::assess(&evidence(&["a"], &["a"]), &partial).screen();
        let missing = evidence_with_secrets(&["a"], &[("a", LocalSecret::Missing)]);
        let unchecked = DisableImpact::assess(&missing, &copy_unread).screen();
        let complete =
            DisableImpact::assess(&evidence(&["a"], &["a"]), &facts_read(&["a"], &[])).screen();

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
        let failed = DisableEvidence {
            listing: ListingEvidence::Failed(CloudStorageIssue::Unavailable),
            ..evidence(&[], &[])
        };
        let impact = DisableImpact::assess(&failed, &facts(&[]));

        let screen = impact.screen();
        assert!(matches!(screen.assessment, CloudBackupInventoryAssessment::Unknown { .. }));
        assert_eq!(screen.confirmation, CloudBackupDisableConfirmation::Two);
    }

    #[test]
    fn an_unclassified_record_is_unchecked_and_partial() {
        let impact =
            DisableImpact::assess(&evidence(&["a", "new"], &["a"]), &facts_read(&["a"], &[]));

        let screen = impact.screen();
        assert_eq!(screen.unchecked, 1);
        assert!(matches!(screen.assessment, CloudBackupInventoryAssessment::Partial { .. }));
        assert_eq!(screen.confirmation, CloudBackupDisableConfirmation::Two);
    }

    #[test]
    fn an_own_record_is_at_risk_by_the_secret_this_device_holds_now() {
        let mut facts = facts(&[]);
        facts.own_cloud_copies.insert("a".into(), CloudCopySecret::Held);
        let assess = |secret| {
            DisableImpact::assess(&evidence_with_secrets(&["a"], &[("a", secret)]), &facts).screen()
        };

        let held = assess(LocalSecret::Held);
        assert_eq!(held.confirmation, CloudBackupDisableConfirmation::One);

        // the cloud copy holds the seed the keychain lost after the inventory read it
        let missing = assess(LocalSecret::Missing);
        assert_eq!(missing.own_secret_missing_locally.names, ["Wallet a"]);
        assert_eq!(missing.confirmation, CloudBackupDisableConfirmation::Two);

        let unreadable = assess(LocalSecret::Unreadable);
        assert_eq!(unreadable.unchecked, 1);
        assert!(matches!(unreadable.assessment, CloudBackupInventoryAssessment::Partial { .. }));
    }

    #[test]
    fn a_local_read_from_before_a_wallet_change_is_not_current() {
        use crate::manager::cloud_backup_manager::{InventoryInput, InventoryOwner};

        let mut owner = InventoryOwner::default();
        owner.apply(InventoryInput::LocalWalletsChanged { local_record_ids: None });

        let mut facts = facts(&[]);
        facts.read_at = owner.read_at();

        let screen = DisableImpact::assess(&evidence(&["a"], &["a"]), &facts).screen();

        assert!(matches!(screen.assessment, CloudBackupInventoryAssessment::Unknown { .. }));
        assert_eq!(screen.confirmation, CloudBackupDisableConfirmation::Two);
    }

    #[test]
    fn an_own_record_whose_secret_left_turns_a_confirmed_review_into_growth() {
        let facts = facts_read(&["a"], &[]);
        let confirmed = DisableImpact::assess(&evidence(&["a"], &["a"]), &facts);
        let lost = DisableImpact::assess(
            &evidence_with_secrets(&["a"], &[("a", LocalSecret::Missing)]),
            &facts,
        );

        assert!(lost.grew_since(&confirmed));
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
        let confirmed = DisableImpact::assess(&evidence(&["a"], &["a"]), &facts_read(&["a"], &[]));
        let failed = DisableImpact::assess(
            &DisableEvidence {
                listing: ListingEvidence::Failed(CloudStorageIssue::Other),
                ..evidence(&[], &["a"])
            },
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
