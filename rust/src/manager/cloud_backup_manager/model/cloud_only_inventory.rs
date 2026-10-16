use std::collections::HashSet;

use crate::manager::cloud_backup_manager::CloudBackupWalletItem;

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
    pub(crate) wallets: Vec<CloudBackupWalletItem>,
    /// Listed records that could not be read, so `wallets` is not the whole cloud-only set
    pub(crate) unresolved_record_ids: Vec<String>,
    /// Listed records whose wallet identity already exists on this device under another record
    /// ID, so they are accounted for without being restorable rows
    pub(crate) duplicate_record_ids: HashSet<String>,
    pub(crate) listing_generation: u64,
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
    /// Listed records accounted for by a local wallet with the same identity, which never show as
    /// loaded rows but still count toward covering the listing
    duplicate_record_ids: HashSet<String>,
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
        self.duplicate_record_ids.clear();
    }

    pub(crate) fn record_loaded(&mut self, fetch: &CloudOnlyFetch) {
        self.complete_fetch_generation =
            fetch.unresolved_record_ids.is_empty().then_some(fetch.listing_generation);
        self.duplicate_record_ids = fetch.duplicate_record_ids.clone();
    }

    /// Accounts for a record that finished as a duplicate of a local wallet, so removing its row
    /// does not leave the loaded rows short of the listing
    pub(crate) fn record_duplicate(&mut self, record_id: String) {
        self.duplicate_record_ids.insert(record_id);
    }

    pub(crate) fn clear_loaded(&mut self) {
        self.complete_fetch_generation = None;
        self.duplicate_record_ids.clear();
    }

    fn covers_listing(&self, loaded_ids: &HashSet<String>) -> bool {
        let Some(listed) = &self.listed_record_ids else { return false };

        listed.len() == loaded_ids.union(&self.duplicate_record_ids).count()
            && listed
                .iter()
                .all(|id| loaded_ids.contains(id) || self.duplicate_record_ids.contains(id))
    }

    /// Keeps loaded rows only when they are a complete read of exactly the listed wallets
    pub(crate) fn keep_loaded_for_listing(&mut self, loaded_ids: &HashSet<String>) -> bool {
        if self.complete_fetch_generation.is_none() || !self.covers_listing(loaded_ids) {
            return false;
        }

        // the rows match the latest listing, so they are current for it
        self.complete_fetch_generation = Some(self.generation);
        true
    }

    /// Whether loaded rows are a complete read of exactly the latest listing
    pub(crate) fn loaded_is_complete_and_current(&self, loaded_ids: &HashSet<String>) -> bool {
        self.covers_listing(loaded_ids) && self.complete_fetch_generation == Some(self.generation)
    }
}
