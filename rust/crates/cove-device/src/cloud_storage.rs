use std::sync::Arc;

use once_cell::sync::OnceCell;
use tracing::warn;

#[derive(Debug, Clone, Hash, Eq, PartialEq, uniffi::Error, thiserror::Error)]
#[uniffi::export(Display)]
pub enum CloudStorageError {
    #[error("not available: {0}")]
    NotAvailable(String),

    #[error("upload failed: {0}")]
    UploadFailed(String),

    #[error("download failed: {0}")]
    DownloadFailed(String),

    #[error("not found: {0}")]
    NotFound(String),

    #[error("quota exceeded")]
    QuotaExceeded,
}

#[uniffi::export(callback_interface)]
pub trait CloudStorageAccess: Send + Sync + std::fmt::Debug + 'static {
    fn upload_master_key_backup(&self, data: Vec<u8>) -> Result<(), CloudStorageError>;
    fn upload_wallet_backup(
        &self,
        record_id: String,
        data: Vec<u8>,
    ) -> Result<(), CloudStorageError>;

    fn download_master_key_backup(&self) -> Result<Vec<u8>, CloudStorageError>;
    fn download_wallet_backup(&self, record_id: String) -> Result<Vec<u8>, CloudStorageError>;

    fn upload_manifest(&self, data: Vec<u8>) -> Result<(), CloudStorageError>;
    fn download_manifest(&self) -> Result<Vec<u8>, CloudStorageError>;

    /// Check if a complete cloud backup exists by probing the manifest record
    ///
    /// Returns Ok(true) if manifest record exists (complete backup set),
    /// Ok(false) if definitely absent,
    /// Err for transient failures (network, iCloud unavailable)
    fn has_cloud_backup(&self) -> Result<bool, CloudStorageError>;
}

static REF: OnceCell<CloudStorage> = OnceCell::new();

#[derive(Debug, Clone, uniffi::Object)]
pub struct CloudStorage(Arc<Box<dyn CloudStorageAccess>>);

impl CloudStorage {
    pub fn global() -> &'static Self {
        REF.get().expect("cloud storage is not initialized")
    }
}

#[uniffi::export]
impl CloudStorage {
    #[uniffi::constructor]
    pub fn new(cloud_storage: Box<dyn CloudStorageAccess>) -> Self {
        if let Some(me) = REF.get() {
            warn!("cloud storage is already initialized");
            return me.clone();
        }

        let me = Self(Arc::new(cloud_storage));
        REF.set(me).expect("failed to set cloud storage");

        Self::global().clone()
    }
}

impl CloudStorage {
    pub fn upload_master_key_backup(&self, data: Vec<u8>) -> Result<(), CloudStorageError> {
        self.0.upload_master_key_backup(data)
    }

    pub fn upload_wallet_backup(
        &self,
        record_id: String,
        data: Vec<u8>,
    ) -> Result<(), CloudStorageError> {
        self.0.upload_wallet_backup(record_id, data)
    }

    pub fn download_master_key_backup(&self) -> Result<Vec<u8>, CloudStorageError> {
        self.0.download_master_key_backup()
    }

    pub fn download_wallet_backup(&self, record_id: String) -> Result<Vec<u8>, CloudStorageError> {
        self.0.download_wallet_backup(record_id)
    }

    pub fn upload_manifest(&self, data: Vec<u8>) -> Result<(), CloudStorageError> {
        self.0.upload_manifest(data)
    }

    pub fn download_manifest(&self) -> Result<Vec<u8>, CloudStorageError> {
        self.0.download_manifest()
    }

    pub fn has_cloud_backup(&self) -> Result<bool, CloudStorageError> {
        self.0.has_cloud_backup()
    }
}
