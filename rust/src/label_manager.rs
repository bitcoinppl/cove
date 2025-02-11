use bip329::Labels;

use crate::{database::wallet_data::WalletDataDb, wallet::metadata::WalletId};

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

    pub fn import_labels(&self, labels: &str) -> Result<(), LabelManagerError> {
        let labels =
            Labels::try_from_str(labels).map_err(|e| LabelManagerError::Parse(e.to_string()))?;

        self.db
            .labels
            .insert_labels(labels)
            .map_err(|e| LabelManagerError::Save(e.to_string()))?;

        Ok(())
    }

    pub fn export_labels(&self) -> Result<String, LabelManagerError> {
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
