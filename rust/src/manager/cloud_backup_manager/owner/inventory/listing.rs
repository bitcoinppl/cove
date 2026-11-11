use std::collections::{BTreeSet, HashSet};

use crate::manager::cloud_backup_manager::actors::write::ConfigGeneration;

use crate::manager::cloud_backup_manager::inventory::{CopyCover, InventoryListedClass};
use crate::manager::cloud_backup_manager::screen::{
    CloudBackupKeyTrust, CloudBackupListedRecord, CloudBackupUnreadableKind,
};
use crate::manager::cloud_backup_manager::{CloudBackupWalletItem, CloudBackupWalletStatus};
use crate::wallet_identity::CoveringWallets;

/// One listed record of the active namespace that is not a local wallet's own backup
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CloudOnlyRow {
    pub(crate) record_id: String,
    pub(crate) record: CloudOnlyRecord,
}

/// What Cove knows about a listed record
///
/// Like the exported [`CloudBackupListedRecord`], except an extra copy keeps the identities of
/// the local wallets that cover it, so it is matched to their own backups by wallet ID
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum CloudOnlyRecord {
    NotOnDevice(CloudBackupWalletItem),
    ExtraCopy { wallet: CloudBackupWalletItem, covered_by: CoveringWallets },
    Unsupported,
    Unreadable { kind: CloudBackupUnreadableKind, trust: CloudBackupKeyTrust },
    Unchecked,
}

impl CloudOnlyRow {
    /// A record no local wallet covers, so restoring it adds what it holds
    pub(crate) fn not_on_device(mut wallet: CloudBackupWalletItem) -> Self {
        wallet.sync_status = CloudBackupWalletStatus::DeletedFromDevice;

        Self { record_id: wallet.record_id.clone(), record: CloudOnlyRecord::NotOnDevice(wallet) }
    }

    /// A record whose recovery material the local wallets `covered_by` already hold
    pub(crate) fn extra_copy(
        mut wallet: CloudBackupWalletItem,
        covered_by: CoveringWallets,
    ) -> Self {
        wallet.sync_status = CloudBackupWalletStatus::AlreadyOnDevice;
        wallet.restore_failure = None;

        Self {
            record_id: wallet.record_id.clone(),
            record: CloudOnlyRecord::ExtraCopy { wallet, covered_by },
        }
    }

    pub(crate) fn unsupported(record_id: String) -> Self {
        Self { record_id, record: CloudOnlyRecord::Unsupported }
    }

    /// A record that could not be read; the key is unverified until verification proves it
    pub(crate) fn unreadable(record_id: String, kind: CloudBackupUnreadableKind) -> Self {
        let trust = CloudBackupKeyTrust::Unverified;

        Self { record_id, record: CloudOnlyRecord::Unreadable { kind, trust } }
    }

    /// A record left unread because no usable local master key exists
    pub(crate) fn unchecked(record_id: String) -> Self {
        Self { record_id, record: CloudOnlyRecord::Unchecked }
    }

    /// The wallet a restore would add, for a record no local wallet covers
    pub(crate) fn restorable_wallet(&self) -> Option<&CloudBackupWalletItem> {
        match &self.record {
            CloudOnlyRecord::NotOnDevice(wallet) => Some(wallet),
            _ => None,
        }
    }

    pub(crate) fn restorable_wallet_mut(&mut self) -> Option<&mut CloudBackupWalletItem> {
        match &mut self.record {
            CloudOnlyRecord::NotOnDevice(wallet) => Some(wallet),
            _ => None,
        }
    }

    /// What Turn Off's impact reads this row as, keeping an extra copy's covering wallets and
    /// what it holds, so Turn Off can check this device still holds it
    pub(crate) fn impact_class(&self) -> InventoryListedClass {
        match &self.record {
            CloudOnlyRecord::NotOnDevice(wallet) => {
                InventoryListedClass::NotOnDevice { name: wallet.name.clone() }
            }
            CloudOnlyRecord::ExtraCopy { covered_by, .. } => {
                InventoryListedClass::ExtraCopy(CopyCover::from(covered_by))
            }
            CloudOnlyRecord::Unsupported => InventoryListedClass::Unsupported,
            CloudOnlyRecord::Unreadable { .. } => InventoryListedClass::Unreadable,
            CloudOnlyRecord::Unchecked => InventoryListedClass::Unchecked,
        }
    }

    /// Whether the record was read, so the row says what it holds
    ///
    /// An unsupported record was read far enough to know a newer Cove wrote it, so it counts
    pub(crate) fn is_read(&self) -> bool {
        match &self.record {
            CloudOnlyRecord::NotOnDevice(_)
            | CloudOnlyRecord::ExtraCopy { .. }
            | CloudOnlyRecord::Unsupported => true,
            CloudOnlyRecord::Unreadable { .. } | CloudOnlyRecord::Unchecked => false,
        }
    }

    /// The record as the screen shows it
    pub(crate) fn listed_record(&self) -> CloudBackupListedRecord {
        match &self.record {
            CloudOnlyRecord::NotOnDevice(wallet) => {
                CloudBackupListedRecord::NotOnDevice { wallet: wallet.clone() }
            }
            CloudOnlyRecord::ExtraCopy { wallet, covered_by } => {
                CloudBackupListedRecord::ExtraCopy {
                    wallet: wallet.clone(),
                    covered_by: covered_by.display_name().to_owned(),
                }
            }
            CloudOnlyRecord::Unsupported => CloudBackupListedRecord::Unsupported,
            CloudOnlyRecord::Unreadable { kind, trust } => {
                CloudBackupListedRecord::Unreadable { kind: *kind, trust: *trust }
            }
            CloudOnlyRecord::Unchecked => CloudBackupListedRecord::Unchecked,
        }
    }
}

/// How an applied detail refresh relates to the provider listing it was built from
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum CloudOnlyListingUpdate {
    /// Built from a provider listing that found these cloud-only wallet record IDs
    Listed(HashSet<String>),
    /// Built without a listing, so loaded cloud-only rows cannot be matched and are reset
    Unlisted,
}

/// Result of one cloud-only fetch, stamped with the listing generation it started under
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CloudOnlyFetch {
    /// One row for every cloud-only record the fetch's own provider listing found
    pub(crate) rows: Vec<CloudOnlyRow>,
    pub(crate) listing_generation: u64,
    /// The namespace the fetch listed the records in
    pub(crate) namespace_id: String,
}

impl CloudOnlyFetch {
    /// The cloud-only record IDs the fetch's own provider listing found
    fn listed_record_ids(&self) -> HashSet<String> {
        self.rows.iter().map(|row| row.record_id.clone()).collect()
    }

    /// Whether every listed record was read
    fn read_every_record(&self) -> bool {
        self.rows.iter().all(CloudOnlyRow::is_read)
    }
}

/// Identity of the latest provider listing and proof of what the loaded cloud-only rows cover
///
/// Owned by the reducer next to the rows, so applying a detail and applying a fetch are decided
/// against the same listing under one lock
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct CloudOnlyInventory {
    /// Bumped whenever the listed cloud-only IDs change, so a fetch started under an older
    /// listing can be told apart from one that matches the current listing
    generation: u64,
    listed_record_ids: Option<HashSet<String>>,
    /// Listing generation the loaded rows completely read, unset when nothing complete is loaded
    complete_fetch_generation: Option<u64>,
    /// Listing generation the loaded rows were read or kept under, complete or not, unset when
    /// no rows are loaded
    rows_generation: Option<u64>,
    /// The records the last verified read could not decrypt, while its listing is current
    verified_unreadable: Option<UnreadableRead>,
}

/// Records a key this configuration verified failed to decrypt, read under one listing
///
/// The reader takes the listing generation before it reads the local wallets, so a wallet
/// change during the read moves the generation and the read is dropped instead of naming a
/// record that became a local wallet's own
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct UnreadableRead {
    pub(crate) listing_generation: u64,
    /// The configuration whose verified key failed to decrypt the records
    pub(crate) configuration: ConfigGeneration,
    pub(crate) record_ids: BTreeSet<String>,
}

impl CloudOnlyInventory {
    pub(crate) fn generation(&self) -> u64 {
        self.generation
    }

    pub(crate) fn record_listing(&mut self, record_ids: &HashSet<String>) {
        if self.listed_record_ids.as_ref() != Some(record_ids) {
            self.generation += 1;
            self.listed_record_ids = Some(record_ids.clone());
        }
    }

    /// Forgets the listing and the loaded proof, so every fetch started before is stale
    pub(crate) fn invalidate(&mut self) {
        self.generation += 1;
        self.listed_record_ids = None;
        self.complete_fetch_generation = None;
        self.verified_unreadable = None;
    }

    /// Records a fetch applied under the current generation as the loaded proof, but only when
    /// it read every record of exactly the latest listing
    ///
    /// A fetch lists the provider again, so it can find a different set than the detail listing
    /// did, and then it proves nothing until a new listing settles which one is current
    pub(crate) fn record_loaded(&mut self, fetch: &CloudOnlyFetch) {
        let fetch_listing = fetch.listed_record_ids();

        // the listing was forgotten before this fetch started, so the fetch's own listing is the
        // latest one
        if self.listed_record_ids.is_none() {
            self.listed_record_ids = Some(fetch_listing.clone());
        }

        let complete =
            fetch.read_every_record() && self.listed_record_ids.as_ref() == Some(&fetch_listing);
        self.complete_fetch_generation = complete.then_some(fetch.listing_generation);
        self.rows_generation = Some(fetch.listing_generation);
    }

    /// Drops a record that stopped being cloud-only, because it was restored or deleted, so the
    /// listing keeps matching the rows that remain
    pub(crate) fn forget_record(&mut self, record_id: &str) {
        if let Some(listed_record_ids) = &mut self.listed_record_ids {
            listed_record_ids.remove(record_id);
        }
    }

    pub(crate) fn clear_loaded(&mut self) {
        self.complete_fetch_generation = None;
        self.rows_generation = None;
    }

    fn covers_listing(&self, loaded_ids: &HashSet<String>) -> bool {
        self.listed_record_ids.as_ref() == Some(loaded_ids)
    }

    /// Keeps loaded rows only when they are a complete read of exactly the listed wallets
    pub(crate) fn keep_loaded_for_listing(&mut self, loaded_ids: &HashSet<String>) -> bool {
        if self.complete_fetch_generation.is_none() || !self.covers_listing(loaded_ids) {
            return false;
        }

        // the rows match the latest listing, so they are current for it
        self.complete_fetch_generation = Some(self.generation);
        self.rows_generation = Some(self.generation);
        true
    }

    /// Whether the loaded rows were read under the latest listing, so a delete can rest on them
    ///
    /// Unlike completeness, a partial read counts: each row it holds still says what its record
    /// is, but rows read under a listing since replaced or forgotten say nothing
    pub(crate) fn loaded_rows_are_current(&self) -> bool {
        self.rows_generation == Some(self.generation)
    }

    /// Keeps a verified read made under the current listing, returning whether it was kept
    pub(crate) fn record_unreadable(&mut self, read: UnreadableRead) -> bool {
        if read.listing_generation != self.generation {
            return false;
        }

        self.verified_unreadable = Some(read);
        true
    }

    /// The last verified read, while the listing it was made under is still current
    pub(crate) fn verified_unreadable(&self) -> Option<&UnreadableRead> {
        self.verified_unreadable.as_ref().filter(|read| read.listing_generation == self.generation)
    }

    /// Whether loaded rows are a complete read of exactly the latest listing
    ///
    /// The one completeness check behind both the exported `incomplete` flag and the Restore All
    /// marker proof
    pub(crate) fn loaded_is_complete_and_current(&self, loaded_ids: &HashSet<String>) -> bool {
        self.covers_listing(loaded_ids) && self.complete_fetch_generation == Some(self.generation)
    }
}
