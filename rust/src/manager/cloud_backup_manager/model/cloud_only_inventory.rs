use std::collections::HashSet;

use crate::manager::cloud_backup_manager::CloudBackupWalletItem;

/// How an applied detail refresh relates to the provider listing it was built from
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum CloudOnlyListingUpdate {
    /// Built from a provider listing that found these cloud-only wallet record IDs
    Listed(HashSet<String>),
    /// Built without a listing, so loaded cloud-only rows cannot be matched and are reset
    Unlisted,
    /// Not related to the listing, loaded rows are kept as they are
    Retained,
}

/// Result of one cloud-only fetch, stamped with the listing generation it started under
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CloudOnlyFetch {
    pub(crate) wallets: Vec<CloudBackupWalletItem>,
    /// Listed records that could not be read, so `wallets` is not the whole cloud-only set
    pub(crate) unresolved_record_ids: Vec<String>,
    pub(crate) listing_generation: u64,
}

/// What the loaded cloud-only rows were fetched for
#[derive(Debug, Clone, PartialEq, Eq)]
struct LoadedFetch {
    listing_generation: u64,
    complete: bool,
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
    loaded: Option<LoadedFetch>,
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
        self.loaded = None;
    }

    pub(crate) fn record_loaded(&mut self, fetch: &CloudOnlyFetch) {
        self.loaded = Some(LoadedFetch {
            listing_generation: fetch.listing_generation,
            complete: fetch.unresolved_record_ids.is_empty(),
        });
    }

    pub(crate) fn clear_loaded(&mut self) {
        self.loaded = None;
    }

    /// Keeps loaded rows only when they are a complete read of exactly the listed wallets
    pub(crate) fn keep_loaded_for_listing(&mut self, loaded_ids: &HashSet<String>) -> bool {
        let complete = self.loaded.as_ref().is_some_and(|loaded| loaded.complete);
        if !complete || self.listed_record_ids.as_ref() != Some(loaded_ids) {
            return false;
        }

        // the rows match the latest listing, so they are current for it
        self.loaded = Some(LoadedFetch { listing_generation: self.generation, complete: true });
        true
    }

    /// Whether loaded rows are a complete read of exactly the latest listing
    pub(crate) fn loaded_is_complete_and_current(&self, loaded_ids: &HashSet<String>) -> bool {
        self.listed_record_ids.as_ref() == Some(loaded_ids)
            && self.loaded.as_ref().is_some_and(|loaded| {
                loaded.complete && loaded.listing_generation == self.generation
            })
    }
}
