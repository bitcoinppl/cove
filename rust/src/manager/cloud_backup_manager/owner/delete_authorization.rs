//! Row delete authorization: the only proof that lets the write lane delete a wallet record
//!
//! A [`DeleteAuthorization`] is built from the consent the user confirmed, field by field: the
//! records it names, the namespace they were listed in, the listing, inventory read and
//! configuration they were classified under, and their delete category. Only a
//! [`RowDeleteAuthority`] can build one. Dispatch checks it against the [`DeleteView`] before the
//! delete starts, and the write lane checks it again at the head of its queue, just before the
//! remote call, against the view current at that moment; a mismatch is a [`StaleTarget`], never a
//! delete
//!
//! The record ids are frozen when the consent is minted, so a batch never grows: a record found
//! after the user confirmed needs a new consent. The row rules live with the consent
//! (`delete_consent.rs`), so the screen and this check decide a row the same way

use std::collections::{BTreeMap, BTreeSet};

use tracing::info;

use crate::manager::cloud_backup_manager::actors::write::ConfigGeneration;
use crate::manager::cloud_backup_manager::delete_consent::{
    CloudBackupBatchDeleteConsent, CloudBackupDeleteCategory, CloudBackupDeleteEvidence,
    CloudBackupRowDeleteConsent,
};
use crate::manager::cloud_backup_manager::owner::inventory::{
    InventoryReadAt, RecordRead, RecordReads,
};
use crate::manager::cloud_backup_manager::screen::CloudBackupUnreadableKind;

/// Generation of the cloud-only listing a row was classified under
///
/// The inventory owner moves it whenever the listed records change or the listing is forgotten,
/// which includes every local wallet change, so loaded rows from an older listing never match
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct ListingGeneration(pub(crate) u64);

/// The inventory's deletable rows with the generations they were classified under
///
/// Read from the inventory owner under one lock, through the same consent rule the screen draws
/// its Delete controls with. An unreadable row is deletable here only when the configuration in
/// force verified the key that failed
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DeleteView {
    /// The namespace in force, which a delete would run in now
    pub(crate) namespace: String,
    pub(crate) listing: ListingGeneration,
    /// The configuration scope and local wallet changes the inventory was read under
    pub(crate) read_at: InventoryReadAt,
    pub(crate) configuration: ConfigGeneration,
    /// The category each listed record would be deleted as now, keyed by record ID; a row that
    /// offers no Delete, or one listed in another namespace, has no entry
    pub(crate) rows: BTreeMap<String, CloudBackupDeleteCategory>,
}

impl DeleteView {
    /// The category `record_id` would be deleted as now, or `None` when its row offers no Delete
    pub(crate) fn category_of(&self, record_id: &str) -> Option<CloudBackupDeleteCategory> {
        self.rows.get(record_id).copied()
    }

    /// Whether the view deletes `record_id` as one the configuration's verified key failed to
    /// decrypt, which only a read with that key can make it
    fn still_undecryptable(&self, record_id: &str) -> bool {
        let undecryptable =
            CloudBackupDeleteCategory::Unreadable { kind: CloudBackupUnreadableKind::Decrypt };

        self.category_of(record_id) == Some(undecryptable)
    }
}

/// Why an authorization no longer holds at execution; nothing was deleted
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum StaleTarget {
    /// The command names a record the authorization does not
    NotAuthorized,
    NamespaceChanged,
    /// The listing the rows were classified under was replaced or forgotten
    ListingChanged,
    /// A local wallet changed, or the inventory was read under another configuration scope
    InventoryReadChanged,
    /// The configuration it was minted under is no longer in force, so neither is the key an
    /// unreadable row's trust rests on
    ConfigurationChanged,
    /// The record left the inventory or is no longer in the authorized category
    CategoryChanged,
    /// The lane could not read the inventory, so it cannot prove the authorization holds
    InventoryUnavailable,
}

/// A confirmed delete of a frozen set of records, valid only against the inventory it came from
///
/// Fields are private and only [`RowDeleteAuthority`] builds one, from a consent
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DeleteAuthorization {
    namespace: String,
    record_ids: BTreeSet<String>,
    listing: ListingGeneration,
    read_at: InventoryReadAt,
    configuration: ConfigGeneration,
    category: CloudBackupDeleteCategory,
}

impl DeleteAuthorization {
    /// The authorization `evidence` describes, for `record_ids`
    fn of(
        evidence: &CloudBackupDeleteEvidence,
        record_ids: impl IntoIterator<Item = String>,
    ) -> Self {
        Self {
            namespace: evidence.namespace_id().to_owned(),
            record_ids: record_ids.into_iter().collect(),
            listing: ListingGeneration(evidence.listing_generation()),
            read_at: evidence.read_at(),
            configuration: evidence.configuration(),
            category: evidence.category(),
        }
    }

    pub(crate) fn namespace(&self) -> &str {
        &self.namespace
    }

    /// The records frozen at confirmation, in a stable order
    pub(crate) fn record_ids(&self) -> impl Iterator<Item = &str> {
        self.record_ids.iter().map(String::as_str)
    }

    /// `read` of `record_id`, made after this authorization held, for the inventory owner
    ///
    /// Stamped with the listing and configuration it held under, so the owner drops the read
    /// once either moved
    pub(crate) fn read_of(&self, record_id: String, read: RecordRead) -> RecordReads {
        RecordReads {
            listing_generation: self.listing.0,
            configuration: self.configuration,
            reads: BTreeMap::from([(record_id, read)]),
        }
    }

    /// Whether every record it names may be deleted now, against `current`
    pub(crate) fn holds(&self, current: &DeleteView) -> Result<(), StaleTarget> {
        self.record_ids.iter().try_for_each(|record_id| self.check(record_id, current))
    }

    /// Whether `record_id` may be deleted now, against the view current at execution
    pub(crate) fn check(&self, record_id: &str, current: &DeleteView) -> Result<(), StaleTarget> {
        if !self.record_ids.contains(record_id) {
            return Err(StaleTarget::NotAuthorized);
        }

        if current.namespace != self.namespace {
            return Err(StaleTarget::NamespaceChanged);
        }

        if current.listing != self.listing {
            return Err(StaleTarget::ListingChanged);
        }

        if current.read_at != self.read_at {
            return Err(StaleTarget::InventoryReadChanged);
        }

        if current.configuration != self.configuration {
            return Err(StaleTarget::ConfigurationChanged);
        }

        if current.category_of(record_id) != Some(self.category) {
            return Err(StaleTarget::CategoryChanged);
        }

        Ok(())
    }
}

/// Proof that the holder owns the inventory, and so may build delete authorizations
///
/// The private field means only this module can create one, and
/// [`RowDeleteAuthority::bootstrap`] is the only place that does
#[derive(Debug)]
pub(crate) struct RowDeleteAuthority(());

impl RowDeleteAuthority {
    /// Creates the authority the inventory owner holds for the life of the manager
    ///
    /// Takes the lifecycle writer as proof that the caller is the bootstrap phase, the only code
    /// that holds one before it moves into the supervisor
    pub(crate) fn bootstrap(_owner: &super::lifecycle::LifecycleWriter) -> Self {
        Self(())
    }

    /// The authorization to delete the one record `consent` names, as the user confirmed it
    pub(crate) fn authorize_row(
        &self,
        consent: &CloudBackupRowDeleteConsent,
    ) -> DeleteAuthorization {
        DeleteAuthorization::of(consent.evidence(), [consent.record_id().as_str().to_owned()])
    }

    /// The authorization to delete those records of the confirmed batch the verified read in
    /// `view` still holds as undecryptable
    ///
    /// Deleting a subset of what the user confirmed never deletes anything they didn't agree to.
    /// A confirmed record the read no longer holds has gone already or reads again, so it is
    /// skipped; when none is left, the authorization names no record and nothing is deleted
    pub(crate) fn authorize_batch(
        &self,
        consent: &CloudBackupBatchDeleteConsent,
        view: &DeleteView,
    ) -> DeleteAuthorization {
        let confirmed = consent.record_ids().count();
        let record_ids: Vec<String> = consent
            .record_ids()
            .filter(|record_id| view.still_undecryptable(record_id.as_str()))
            .map(|record_id| record_id.as_str().to_owned())
            .collect();

        let skipped = confirmed - record_ids.len();
        if skipped > 0 {
            info!(
                "delete undecryptable: skipped={skipped} of confirmed={confirmed}, no longer listed as unreadable"
            );
        }

        DeleteAuthorization::of(consent.evidence(), record_ids)
    }
}

pub(crate) mod test_support {
    #![cfg(test)]

    use super::{CloudBackupDeleteCategory, DeleteAuthorization, DeleteView, RowDeleteAuthority};

    /// An authority for tests that drive a write lane or a worker without a supervisor
    pub(crate) fn delete_authority() -> RowDeleteAuthority {
        RowDeleteAuthority(())
    }

    /// The authorization a consent given on `view` to delete `record_id` as `category` builds,
    /// for tests that drive a write lane without a screen to confirm on
    pub(crate) fn authorization_on(
        view: &DeleteView,
        record_id: &str,
        category: CloudBackupDeleteCategory,
    ) -> DeleteAuthorization {
        DeleteAuthorization {
            namespace: view.namespace.clone(),
            record_ids: [record_id.to_owned()].into(),
            listing: view.listing,
            read_at: view.read_at,
            configuration: view.configuration,
            category,
        }
    }
}
