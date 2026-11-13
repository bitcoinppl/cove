//! The inventory view row deletes are checked against, at dispatch and again at execution
//!
//! Read from the inventory owner under one lock, through the consent rule the screen draws its
//! Delete controls with, so a row the screen offers is a row this view deletes

use std::collections::BTreeMap;

use crate::manager::cloud_backup_manager::actors::write::ConfigGeneration;
use crate::manager::cloud_backup_manager::inventory::InventoryView;
use crate::manager::cloud_backup_manager::owner::delete_authorization::{
    DeleteView, ListingGeneration,
};
use crate::manager::cloud_backup_manager::{CloudBackupError, RustCloudBackupManager};

impl RustCloudBackupManager {
    /// The rows of the namespace in force as row deletes see them, under `configuration`
    pub(crate) fn delete_view(
        &self,
        configuration: ConfigGeneration,
    ) -> Result<DeleteView, CloudBackupError> {
        let namespace = self.current_namespace_id()?;
        Ok(self.inventory_facts().view().delete_view(configuration, namespace))
    }
}

impl InventoryView {
    /// The deletable rows as a delete in `namespace` under `configuration` sees them
    pub(crate) fn delete_view(
        &self,
        configuration: ConfigGeneration,
        namespace: String,
    ) -> DeleteView {
        let consents = self.delete_consents(configuration).in_namespace(Some(&namespace));

        // rows listed in another namespace aren't records a delete there could reach
        let rows: BTreeMap<_, _> = self
            .rows
            .iter()
            .filter(|rows| rows.listed_namespace.as_deref() == Some(namespace.as_str()))
            .flat_map(|rows| {
                rows.listed.iter().filter_map(|listed| {
                    let consent = consents.row(listed, rows.copy_has_current_backup(listed))?;
                    Some((listed.record_id.as_str().to_owned(), consent.evidence().category()))
                })
            })
            .collect();

        DeleteView {
            namespace,
            listing: ListingGeneration(self.listing_generation),
            read_at: self.read_at,
            configuration,
            rows,
        }
    }
}
