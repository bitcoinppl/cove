use std::sync::Arc;

use crate::{
    database::wallet_data::WalletDataDb, multi_format::Bip329Labels, wallet::metadata::WalletId,
};
use bip329::Labels;

#[derive(Debug, Clone, uniffi::Object)]
pub struct LabelManager {
    #[allow(dead_code)]
    id: WalletId,
    db: WalletDataDb,
}

#[derive(Debug, thiserror::Error, uniffi::Error)]
pub enum LabelManagerError {
    #[error("Failed to parse labels: {0}")]
    Parse(String),

    #[error("Failed to save labels: {0}")]
    Save(String),

    #[error("Failed to get labels: {0}")]
    Get(String),

    #[error("Failed to export labels: {0}")]
    Export(String),
}

pub type Error = LabelManagerError;
type Result<T, E = Error> = std::result::Result<T, E>;

#[uniffi::export]
impl LabelManager {
    #[uniffi::constructor]
    pub fn new(id: WalletId) -> Self {
        let db = WalletDataDb::new_or_existing(id.clone());
        Self { id, db }
    }

    pub fn export_default_file_name(&self, name: String) -> String {
        let name = name
            .replace(" ", "_")
            .replace(|c: char| !c.is_alphanumeric() || c == '_', "")
            .to_ascii_lowercase();

        format!("{name}-bip329-labels")
    }

    pub fn has_labels(&self) -> bool {
        self.db.labels.number_of_labels().unwrap_or(0) > 0
    }

    #[uniffi::method(name = "importLabels")]
    pub fn _import_labels(&self, labels: Arc<Bip329Labels>) -> Result<(), LabelManagerError> {
        let labels = Arc::unwrap_or_clone(labels);
        self.import_labels(labels.0)
    }

    pub fn import(&self, jsonl: &str) -> Result<(), LabelManagerError> {
        let labels =
            Labels::try_from_str(jsonl).map_err(|e| LabelManagerError::Parse(e.to_string()))?;

        self.import_labels(labels)
    }

    pub fn export(&self) -> Result<String, LabelManagerError> {
        let labels = self
            .db
            .labels
            .all_labels()
            .map_err(|e| LabelManagerError::Get(e.to_string()))?;

        let labels = labels
            .export()
            .map_err(|e| LabelManagerError::Export(e.to_string()))?;

        Ok(labels)
    }
}

impl LabelManager {
    pub fn import_labels(&self, labels: impl Into<Labels>) -> Result<(), LabelManagerError> {
        let labels = labels.into();
        self.db
            .labels
            .insert_labels(labels)
            .map_err(|e| LabelManagerError::Save(e.to_string()))?;

        Ok(())
    }
}
