//! What the user consents to when they delete listed cloud records
//!
//! The screen projects a consent with every delete it offers: each row's Delete, and the batch
//! delete of the backups the verified key couldn't decrypt. Each is minted here from the
//! inventory facts the screen draws and the configuration in force, and names the records it
//! covers, the category the inventory put them in and the generations it read them under. The
//! confirmation sends back the consent of the delete the user saw, unchanged
//!
//! Dispatch mints the same delete again from the facts at that moment, in the namespace in force,
//! and starts it only when the two are equal. A delete confirmed on evidence that has since moved,
//! such as an extra copy that is now the only copy, a new listing, a local wallet change, a new
//! configuration or another namespace, is rejected as stale instead of running under a
//! confirmation the user never gave
//!
//! A batch freezes its records when it is minted, so a record found after the confirmation is
//! never added to it

use super::RecordId;
use super::actors::write::ConfigGeneration;
use super::inventory::{InventoryView, ListedRecord};
use super::screen::{
    CloudBackupDeleteConfirmation, CloudBackupKeyTrust, CloudBackupListedRecord,
    CloudBackupUnreadableKind,
};

/// What a confirmed delete removes, which its records must still be when it runs
#[derive(Debug, Clone, Copy, Hash, PartialEq, Eq, uniffi::Enum)]
pub enum CloudBackupDeleteCategory {
    /// A copy of a wallet on this device whose own backup is current
    ExtraCopy,
    /// A wallet that isn't on this device, so this may be its only copy
    NotOnDevice,
    /// Written by a newer version of Cove
    Unsupported,
    /// The key of the configuration in force could not read it
    Unreadable { kind: CloudBackupUnreadableKind },
}

impl CloudBackupDeleteCategory {
    /// What the platform confirms before deleting a record of this category
    pub(crate) fn confirmation(self) -> CloudBackupDeleteConfirmation {
        match self {
            Self::ExtraCopy => CloudBackupDeleteConfirmation::Standard,
            Self::NotOnDevice | Self::Unsupported | Self::Unreadable { .. } => {
                CloudBackupDeleteConfirmation::MayBeOnlyCopy
            }
        }
    }

    /// The category a listed record is deleted as, or `None` when its row offers no Delete
    ///
    /// `copy_replaced` says a wallet an extra copy copies has its own current backup. The row
    /// rules: an extra copy goes only once its replacement is current, and an
    /// unreadable record only when the configuration's verified key is the one that failed
    fn of(record: &CloudBackupListedRecord, copy_replaced: bool) -> Option<Self> {
        use CloudBackupUnreadableKind::{Decrypt, NotDownloadable, Parse, Provider};

        match record {
            CloudBackupListedRecord::NotOnDevice { .. } => Some(Self::NotOnDevice),
            CloudBackupListedRecord::ExtraCopy { .. } => copy_replaced.then_some(Self::ExtraCopy),
            CloudBackupListedRecord::Unsupported => Some(Self::Unsupported),
            CloudBackupListedRecord::Unreadable {
                kind: kind @ (Decrypt | Parse),
                trust: CloudBackupKeyTrust::VerifiedCurrentKey,
            } => Some(Self::Unreadable { kind: *kind }),
            CloudBackupListedRecord::Unreadable {
                kind: Decrypt | Parse,
                trust: CloudBackupKeyTrust::Unverified,
            }
            | CloudBackupListedRecord::Unreadable { kind: NotDownloadable | Provider, .. }
            | CloudBackupListedRecord::Unchecked => None,
        }
    }
}

/// The inventory evidence a delete was offered on
///
/// Opaque to the platform. The generations are process-local counters, compared only with the
/// ones the same process mints
#[derive(Debug, Clone, Hash, PartialEq, Eq, uniffi::Record)]
pub struct CloudBackupDeleteEvidence {
    category: CloudBackupDeleteCategory,
    /// The namespace the records were listed in
    ///
    /// Carried apart from `configuration` because a reinitialize replaces the namespace inside
    /// one configuration generation
    namespace_id: String,
    /// The configuration in force
    configuration: u64,
    /// The configuration whose inventory scope the records were read under, which trails
    /// `configuration` while a new scope's inventory loads
    inventory_scope: Option<u64>,
    /// The cloud-only listing the records were classified under
    listing_generation: u64,
    /// How many local wallet changes the inventory had applied when it classified them
    local_wallets_generation: u64,
}

/// Consent to delete one listed record, as its row's Delete offered it
///
/// Opaque to the platform, which sends back the one its confirmation was opened with
#[derive(Debug, Clone, Hash, PartialEq, Eq, uniffi::Record)]
pub struct CloudBackupRowDeleteConsent {
    record_id: RecordId,
    evidence: CloudBackupDeleteEvidence,
}

impl CloudBackupRowDeleteConsent {
    /// The record the user confirmed deleting
    pub(crate) fn record_id(&self) -> &RecordId {
        &self.record_id
    }

    /// What the platform confirms before this delete
    pub(crate) fn confirmation(&self) -> CloudBackupDeleteConfirmation {
        self.evidence.category.confirmation()
    }
}

/// Consent to delete the backups the verified key couldn't decrypt, frozen to the records the
/// inventory listed as undecryptable when it was offered
///
/// Opaque to the platform, which sends back the one its confirmation was opened with
#[derive(Debug, Clone, Hash, PartialEq, Eq, uniffi::Record)]
pub struct CloudBackupBatchDeleteConsent {
    record_ids: Vec<RecordId>,
    evidence: CloudBackupDeleteEvidence,
}

impl CloudBackupBatchDeleteConsent {
    /// The records the user confirmed deleting, in listing order; nothing outside them is deleted
    pub(crate) fn record_ids(&self) -> impl Iterator<Item = &RecordId> {
        self.record_ids.iter()
    }
}

/// The deletes one inventory view offers under one configuration, in one namespace
///
/// The projection mints from it, and dispatch checks a returned consent against a fresh one, so
/// both decide from the same rule. With no namespace, nothing is listed and nothing is offered
pub(crate) struct DeleteConsents<'a> {
    view: &'a InventoryView,
    configuration: ConfigGeneration,
    namespace: Option<&'a str>,
}

impl InventoryView {
    /// The deletes this view offers while `configuration` is in force, in the namespace its
    /// records were listed in
    pub(crate) fn delete_consents(&self, configuration: ConfigGeneration) -> DeleteConsents<'_> {
        let namespace = self.rows.as_ref().and_then(|rows| rows.listed_namespace.as_deref());
        DeleteConsents { view: self, configuration, namespace }
    }
}

impl<'a> DeleteConsents<'a> {
    /// The same deletes minted in `namespace` instead, the one a delete would run in now, so a
    /// consent given on records listed in another namespace no longer matches
    pub(crate) fn in_namespace(self, namespace: Option<&'a str>) -> Self {
        Self { namespace, ..self }
    }

    /// The consent a listed row's Delete carries, or `None` when the row offers no Delete
    ///
    /// `copy_replaced` says a wallet an extra copy copies has its own current backup
    pub(crate) fn row(
        &self,
        listed: &ListedRecord,
        copy_replaced: bool,
    ) -> Option<CloudBackupRowDeleteConsent> {
        let category = CloudBackupDeleteCategory::of(&listed.record, copy_replaced)?;

        Some(CloudBackupRowDeleteConsent {
            record_id: listed.record_id.clone(),
            evidence: self.evidence(category)?,
        })
    }

    /// The batch delete of every listed record the key couldn't decrypt, in listing order, or
    /// `None` while no listing is loaded, since a batch can only name records listed in a
    /// namespace
    pub(crate) fn undecryptable_batch(&self) -> Option<CloudBackupBatchDeleteConsent> {
        let record_ids = self
            .view
            .rows
            .iter()
            .flat_map(|rows| &rows.listed)
            .filter(|listed| {
                matches!(
                    listed.record,
                    CloudBackupListedRecord::Unreadable {
                        kind: CloudBackupUnreadableKind::Decrypt,
                        ..
                    }
                )
            })
            .map(|listed| listed.record_id.clone())
            .collect();

        let category =
            CloudBackupDeleteCategory::Unreadable { kind: CloudBackupUnreadableKind::Decrypt };

        Some(CloudBackupBatchDeleteConsent { record_ids, evidence: self.evidence(category)? })
    }

    /// Whether `consent` is the Delete its row offers now
    pub(crate) fn admits_row(&self, consent: &CloudBackupRowDeleteConsent) -> bool {
        let Some(rows) = &self.view.rows else { return false };

        rows.listed
            .iter()
            .find(|listed| listed.record_id == consent.record_id)
            .and_then(|listed| self.row(listed, rows.copy_has_current_backup(listed)))
            .is_some_and(|current| current == *consent)
    }

    /// Whether `consent` is the batch the inventory offers now
    pub(crate) fn admits_undecryptable_batch(
        &self,
        consent: &CloudBackupBatchDeleteConsent,
    ) -> bool {
        self.undecryptable_batch().is_some_and(|current| current == *consent)
    }

    fn evidence(&self, category: CloudBackupDeleteCategory) -> Option<CloudBackupDeleteEvidence> {
        Some(CloudBackupDeleteEvidence {
            category,
            namespace_id: self.namespace?.to_owned(),
            configuration: self.configuration.into(),
            inventory_scope: self.view.read_at.scope().into(),
            listing_generation: self.view.listing_generation,
            local_wallets_generation: self.view.read_at.local_wallets(),
        })
    }
}

/// Consents for the named contract fixtures, which no inventory mints; debug builds only, like
/// the fixtures
#[cfg(debug_assertions)]
mod fixture_support {
    use super::{
        CloudBackupBatchDeleteConsent, CloudBackupDeleteCategory, CloudBackupDeleteEvidence,
        CloudBackupRowDeleteConsent, CloudBackupUnreadableKind, RecordId,
    };

    fn evidence(category: CloudBackupDeleteCategory) -> CloudBackupDeleteEvidence {
        CloudBackupDeleteEvidence {
            category,
            namespace_id: "fixture-namespace".into(),
            configuration: 0,
            inventory_scope: None,
            listing_generation: 0,
            local_wallets_generation: 0,
        }
    }

    impl CloudBackupRowDeleteConsent {
        pub(crate) fn fixture(record_id: &str, category: CloudBackupDeleteCategory) -> Self {
            Self { record_id: RecordId::from(record_id.to_owned()), evidence: evidence(category) }
        }
    }

    impl CloudBackupBatchDeleteConsent {
        pub(crate) fn fixture(record_ids: &[&str]) -> Self {
            let category =
                CloudBackupDeleteCategory::Unreadable { kind: CloudBackupUnreadableKind::Decrypt };

            Self {
                record_ids: record_ids.iter().map(|id| RecordId::from((*id).to_owned())).collect(),
                evidence: evidence(category),
            }
        }
    }
}

pub(crate) mod test_support {
    #![cfg(test)]

    use super::{CloudBackupDeleteCategory, CloudBackupListedRecord, CloudBackupRowDeleteConsent};

    impl CloudBackupRowDeleteConsent {
        /// Whether this consent names `record_id` as the kind of record `record` is
        pub(crate) fn names(
            &self,
            record_id: &super::RecordId,
            record: &CloudBackupListedRecord,
        ) -> bool {
            // a row offers Delete on an extra copy only once its replacement is current
            self.record_id == *record_id
                && CloudBackupDeleteCategory::of(record, true) == Some(self.evidence.category)
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::CloudBackupDeleteCategory;
    use crate::manager::cloud_backup_manager::actors::write::ConfigGeneration;
    use crate::manager::cloud_backup_manager::inventory::{
        CopyCover, InventoryListedClass, InventoryRows, InventoryView, ListedRecord,
    };
    use crate::manager::cloud_backup_manager::screen::{
        CloudBackupKeyTrust, CloudBackupListedRecord, CloudBackupOwnRecord, CloudBackupOwnRow,
        CloudBackupUnreadableKind,
    };
    use crate::manager::cloud_backup_manager::{
        CloudBackupOtherBackupsState, CloudBackupRestoreAllState, CloudBackupWalletItem,
        CloudBackupWalletStatus, CloudOnlyOperation, InventoryInput, InventoryOwner,
        InventoryScope, OtherBackupsOperation, RecordId,
    };
    use crate::wallet_identity::BackupRecoveryMaterial;

    const COPY: &str = "record-spending-old";
    const SPENDING: &str = "record-spending";
    const UNDECRYPTABLE: &str = "record-undecryptable";
    const NAMESPACE: &str = "namespace-listed";

    fn wallet(record_id: &str) -> CloudBackupWalletItem {
        CloudBackupWalletItem {
            name: "Spending".into(),
            network: None,
            wallet_mode: None,
            wallet_type: None,
            fingerprint: None,
            label_count: None,
            backup_updated_at: None,
            sync_status: CloudBackupWalletStatus::Confirmed,
            restore_failure: None,
            record_id: record_id.into(),
        }
    }

    fn id(record_id: &str) -> RecordId {
        RecordId::from(record_id.to_owned())
    }

    /// The Spending wallet with its own backup in `own`, an extra copy of it, and one record the
    /// key couldn't decrypt, read when the owner had applied `local_changes` wallet changes
    fn view(own: CloudBackupOwnRecord, local_changes: usize) -> InventoryView {
        let mut owner = InventoryOwner::default();
        for _ in 0..local_changes {
            owner.apply(InventoryInput::LocalWalletsChanged { local_record_ids: None });
        }

        let copy = ListedRecord {
            record_id: id(COPY),
            record: CloudBackupListedRecord::ExtraCopy {
                wallet: wallet(COPY),
                covered_by: "Spending".into(),
            },
            class: InventoryListedClass::ExtraCopy(CopyCover {
                covered_by: vec![id(SPENDING)],
                material: BackupRecoveryMaterial::Secret,
            }),
        };
        let undecryptable = ListedRecord {
            record_id: id(UNDECRYPTABLE),
            record: CloudBackupListedRecord::Unreadable {
                kind: CloudBackupUnreadableKind::Decrypt,
                trust: CloudBackupKeyTrust::Unverified,
            },
            class: InventoryListedClass::Unreadable,
        };

        InventoryView {
            classified: true,
            read_at: owner.read_at(),
            listing_generation: 1,
            progress: None,
            rows: Some(InventoryRows {
                up_to_date: vec![CloudBackupOwnRow { wallet: wallet(SPENDING), record: own }],
                needs_sync: Vec::new(),
                listed: vec![copy, undecryptable],
                listed_namespace: Some(NAMESPACE.into()),
                last_sync: None,
                own_cloud_copies: HashMap::new(),
            }),
            restore_all: CloudBackupRestoreAllState::NotShown,
            other_backups: CloudBackupOtherBackupsState::NotChecked,
            cloud_only_operation: CloudOnlyOperation::Idle,
            other_backups_operation: OtherBackupsOperation::Idle,
        }
    }

    fn listed<'a>(view: &'a InventoryView, record_id: &str) -> &'a ListedRecord {
        let rows = view.rows.as_ref().expect("rows");
        rows.listed.iter().find(|listed| listed.record_id == id(record_id)).expect("the row")
    }

    /// A consent minted on one inventory is admitted while nothing it names has moved, and
    /// refused once its record's category, the listing, the local wallets, the configuration or
    /// the namespace has
    #[test]
    fn a_row_consent_is_admitted_only_on_the_evidence_it_was_minted_on() {
        let n = ConfigGeneration::next();
        let minted_on = view(CloudBackupOwnRecord::Current, 0);
        let consent = minted_on
            .delete_consents(n)
            .row(listed(&minted_on, COPY), true)
            .expect("an extra copy with a current replacement offers Delete");
        assert_eq!(consent.evidence.category, CloudBackupDeleteCategory::ExtraCopy);

        // as dispatch checks it: minted again in the namespace in force
        let admits_in = |view: &InventoryView, configuration, namespace| {
            view.delete_consents(configuration).in_namespace(namespace).admits_row(&consent)
        };
        let admits =
            |view: &InventoryView, configuration| admits_in(view, configuration, Some(NAMESPACE));
        assert!(admits(&view(CloudBackupOwnRecord::Current, 0), n), "nothing moved");

        // a reinitialize replaced the namespace inside the same configuration generation
        let current = view(CloudBackupOwnRecord::Current, 0);
        assert!(!admits_in(&current, n, Some("namespace-reinitialized")), "namespace moved");
        assert!(!admits_in(&current, n, None), "namespace unreadable");

        // the records were listed again in another namespace
        let mut relisted_elsewhere = view(CloudBackupOwnRecord::Current, 0);
        if let Some(rows) = &mut relisted_elsewhere.rows {
            rows.listed_namespace = Some("namespace-reinitialized".into());
        }
        assert!(
            !admits_in(&relisted_elsewhere, n, Some("namespace-reinitialized")),
            "listed in another namespace"
        );

        // the replacement is no longer current, so the copy may be the only one
        assert!(!admits(&view(CloudBackupOwnRecord::Pending, 0), n), "category moved");

        // the wallet left this device, so the record is now the wallet's only backup
        let mut not_on_device = view(CloudBackupOwnRecord::Current, 0);
        if let Some(rows) = &mut not_on_device.rows {
            rows.up_to_date.clear();
            rows.listed[0].record = CloudBackupListedRecord::NotOnDevice { wallet: wallet(COPY) };
            rows.listed[0].class = InventoryListedClass::NotOnDevice { name: COPY.into() };
        }
        assert!(!admits(&not_on_device, n), "ExtraCopy became NotOnDevice");

        let mut relisted = view(CloudBackupOwnRecord::Current, 0);
        relisted.listing_generation += 1;
        assert!(!admits(&relisted, n), "listing moved");

        assert!(!admits(&view(CloudBackupOwnRecord::Current, 1), n), "local wallets moved");

        let mut rescoped = view(CloudBackupOwnRecord::Current, 0);
        let mut owner = InventoryOwner::default();
        owner.apply(InventoryInput::ScopeEntered(InventoryScope::of(n)));
        rescoped.read_at = owner.read_at();
        assert!(!admits(&rescoped, n), "read under another inventory scope");
        let reconfigured = ConfigGeneration::next();
        assert!(
            !admits(&view(CloudBackupOwnRecord::Current, 0), reconfigured),
            "new configuration"
        );

        let mut gone = view(CloudBackupOwnRecord::Current, 0);
        gone.rows = None;
        assert!(!admits(&gone, n), "no rows to delete from");
    }

    /// The batch names the undecryptable rows it was offered with, and is refused once another
    /// one is listed, so a record found after the confirmation is never deleted under it
    #[test]
    fn a_batch_consent_is_frozen_to_the_rows_it_was_minted_on() {
        let n = ConfigGeneration::next();
        let minted_on = view(CloudBackupOwnRecord::Current, 0);
        let batch = minted_on.delete_consents(n).undecryptable_batch().expect("rows are listed");
        assert_eq!(batch.record_ids().cloned().collect::<Vec<_>>(), vec![id(UNDECRYPTABLE)]);
        assert!(minted_on.delete_consents(n).admits_undecryptable_batch(&batch));

        let mut more = view(CloudBackupOwnRecord::Current, 0);
        if let Some(rows) = &mut more.rows {
            rows.listed.push(ListedRecord {
                record_id: id("record-found-later"),
                record: CloudBackupListedRecord::Unreadable {
                    kind: CloudBackupUnreadableKind::Decrypt,
                    trust: CloudBackupKeyTrust::Unverified,
                },
                class: InventoryListedClass::Unreadable,
            });
        }
        assert!(!more.delete_consents(n).admits_undecryptable_batch(&batch));

        let local_change = view(CloudBackupOwnRecord::Current, 1);
        assert!(!local_change.delete_consents(n).admits_undecryptable_batch(&batch));

        let reinitialized =
            minted_on.delete_consents(n).in_namespace(Some("namespace-reinitialized"));
        assert!(!reinitialized.admits_undecryptable_batch(&batch));

        // nothing listed, so there is no batch to consent to
        let mut unlisted = view(CloudBackupOwnRecord::Current, 0);
        unlisted.rows = None;
        assert!(unlisted.delete_consents(n).undecryptable_batch().is_none());
    }
}
