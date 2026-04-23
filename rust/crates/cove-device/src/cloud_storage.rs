use std::sync::Arc;

use cove_cspp::backup_data::wallet_record_id_from_filename;
use once_cell::sync::OnceCell;
use tracing::warn;

#[derive(Debug, Clone, Hash, Eq, PartialEq, uniffi::Error, thiserror::Error)]
#[uniffi::export(Display)]
pub enum CloudStorageError {
    #[error("not available: {0}")]
    NotAvailable(String),

    #[error("offline: {0}")]
    Offline(String),

    #[error("upload failed: {0}")]
    UploadFailed(String),

    #[error("download failed: {0}")]
    DownloadFailed(String),

    #[error("not found: {0}")]
    NotFound(String),

    #[error("quota exceeded")]
    QuotaExceeded,
}

#[derive(Debug, Clone, Hash, Eq, PartialEq, uniffi::Enum)]
pub enum CloudSyncHealth {
    Unknown,
    AllUploaded,
    Uploading,
    Failed(String),
    NoFiles,
    Unavailable,
}

#[uniffi::export(callback_interface)]
#[async_trait::async_trait]
pub trait CloudStorageAccess: Send + Sync + std::fmt::Debug + 'static {
    async fn upload_master_key_backup(
        &self,
        namespace: String,
        data: Vec<u8>,
    ) -> Result<(), CloudStorageError>;

    async fn upload_wallet_backup(
        &self,
        namespace: String,
        record_id: String,
        data: Vec<u8>,
    ) -> Result<(), CloudStorageError>;

    async fn download_master_key_backup(
        &self,
        namespace: String,
    ) -> Result<Vec<u8>, CloudStorageError>;

    async fn download_wallet_backup(
        &self,
        namespace: String,
        record_id: String,
    ) -> Result<Vec<u8>, CloudStorageError>;

    async fn delete_wallet_backup(
        &self,
        namespace: String,
        record_id: String,
    ) -> Result<(), CloudStorageError>;

    /// List all namespace IDs (subdirectories of cspp-namespaces/)
    async fn list_namespaces(&self) -> Result<Vec<String>, CloudStorageError>;

    /// List all namespace IDs without presenting consent UI
    async fn list_namespaces_non_interactive(&self) -> Result<Vec<String>, CloudStorageError>;

    /// List wallet backup filenames within a namespace for user-driven flows
    async fn list_wallet_files(&self, namespace: String) -> Result<Vec<String>, CloudStorageError>;

    /// List wallet backup filenames within a namespace without presenting consent UI
    async fn list_wallet_files_non_interactive(
        &self,
        namespace: String,
    ) -> Result<Vec<String>, CloudStorageError>;

    /// Check whether a blob has been fully uploaded to iCloud
    async fn is_backup_uploaded(
        &self,
        namespace: String,
        record_id: String,
    ) -> Result<bool, CloudStorageError>;

    async fn overall_sync_health(&self) -> CloudSyncHealth;
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

    /// Check if any cloud backup namespaces exist
    pub async fn has_any_cloud_backup(&self) -> Result<bool, CloudStorageError> {
        Ok(!self.list_namespaces_non_interactive().await?.is_empty())
    }
}

impl CloudStorage {
    pub async fn upload_master_key_backup(
        &self,
        namespace: String,
        data: Vec<u8>,
    ) -> Result<(), CloudStorageError> {
        self.0.upload_master_key_backup(namespace, data).await
    }

    pub async fn upload_wallet_backup(
        &self,
        namespace: String,
        record_id: String,
        data: Vec<u8>,
    ) -> Result<(), CloudStorageError> {
        self.0.upload_wallet_backup(namespace, record_id, data).await
    }

    pub async fn download_master_key_backup(
        &self,
        namespace: String,
    ) -> Result<Vec<u8>, CloudStorageError> {
        self.0.download_master_key_backup(namespace).await
    }

    pub async fn download_wallet_backup(
        &self,
        namespace: String,
        record_id: String,
    ) -> Result<Vec<u8>, CloudStorageError> {
        self.0.download_wallet_backup(namespace, record_id).await
    }

    pub async fn delete_wallet_backup(
        &self,
        namespace: String,
        record_id: String,
    ) -> Result<(), CloudStorageError> {
        self.0.delete_wallet_backup(namespace, record_id).await
    }

    pub async fn list_namespaces(&self) -> Result<Vec<String>, CloudStorageError> {
        self.0.list_namespaces().await
    }

    pub async fn list_namespaces_non_interactive(&self) -> Result<Vec<String>, CloudStorageError> {
        self.0.list_namespaces_non_interactive().await
    }

    pub async fn list_wallet_files(
        &self,
        namespace: String,
    ) -> Result<Vec<String>, CloudStorageError> {
        self.0.list_wallet_files(namespace).await
    }

    pub async fn list_wallet_files_non_interactive(
        &self,
        namespace: String,
    ) -> Result<Vec<String>, CloudStorageError> {
        self.0.list_wallet_files_non_interactive(namespace).await
    }

    pub async fn is_backup_uploaded(
        &self,
        namespace: String,
        record_id: String,
    ) -> Result<bool, CloudStorageError> {
        self.0.is_backup_uploaded(namespace, record_id).await
    }

    pub async fn overall_sync_health(&self) -> CloudSyncHealth {
        self.0.overall_sync_health().await
    }

    pub async fn list_wallet_backups(
        &self,
        namespace: String,
    ) -> Result<Vec<String>, CloudStorageError> {
        let filenames = self.0.list_wallet_files(namespace).await?;
        Ok(filenames
            .iter()
            .filter_map(|f| wallet_record_id_from_filename(f).map(String::from))
            .collect())
    }

    pub async fn list_wallet_backups_non_interactive(
        &self,
        namespace: String,
    ) -> Result<Vec<String>, CloudStorageError> {
        let filenames = self.0.list_wallet_files_non_interactive(namespace).await?;
        Ok(filenames
            .iter()
            .filter_map(|f| wallet_record_id_from_filename(f).map(String::from))
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use std::{
        future::Future,
        sync::{
            Arc,
            atomic::{AtomicBool, Ordering},
        },
        task::{Context, Poll, Waker},
    };

    use super::*;

    #[derive(Debug)]
    struct TestCloudStorage {
        interactive_namespaces_called: Arc<AtomicBool>,
        non_interactive_namespaces: Vec<String>,
    }

    #[async_trait::async_trait]
    impl CloudStorageAccess for TestCloudStorage {
        async fn upload_master_key_backup(
            &self,
            _namespace: String,
            _data: Vec<u8>,
        ) -> Result<(), CloudStorageError> {
            panic!("unused in test")
        }

        async fn upload_wallet_backup(
            &self,
            _namespace: String,
            _record_id: String,
            _data: Vec<u8>,
        ) -> Result<(), CloudStorageError> {
            panic!("unused in test")
        }

        async fn download_master_key_backup(
            &self,
            _namespace: String,
        ) -> Result<Vec<u8>, CloudStorageError> {
            panic!("unused in test")
        }

        async fn download_wallet_backup(
            &self,
            _namespace: String,
            _record_id: String,
        ) -> Result<Vec<u8>, CloudStorageError> {
            panic!("unused in test")
        }

        async fn delete_wallet_backup(
            &self,
            _namespace: String,
            _record_id: String,
        ) -> Result<(), CloudStorageError> {
            panic!("unused in test")
        }

        async fn list_namespaces(&self) -> Result<Vec<String>, CloudStorageError> {
            self.interactive_namespaces_called.store(true, Ordering::Release);
            panic!("interactive namespace listing should not be used")
        }

        async fn list_namespaces_non_interactive(&self) -> Result<Vec<String>, CloudStorageError> {
            Ok(self.non_interactive_namespaces.clone())
        }

        async fn list_wallet_files(
            &self,
            _namespace: String,
        ) -> Result<Vec<String>, CloudStorageError> {
            panic!("unused in test")
        }

        async fn list_wallet_files_non_interactive(
            &self,
            _namespace: String,
        ) -> Result<Vec<String>, CloudStorageError> {
            panic!("unused in test")
        }

        async fn is_backup_uploaded(
            &self,
            _namespace: String,
            _record_id: String,
        ) -> Result<bool, CloudStorageError> {
            panic!("unused in test")
        }

        async fn overall_sync_health(&self) -> CloudSyncHealth {
            panic!("unused in test")
        }
    }

    fn block_on_ready<F: Future>(future: F) -> F::Output {
        let waker = Waker::noop();
        let mut context = Context::from_waker(waker);
        let mut future = std::pin::pin!(future);
        match future.as_mut().poll(&mut context) {
            Poll::Ready(output) => output,
            Poll::Pending => panic!("future unexpectedly pending"),
        }
    }

    #[test]
    fn has_any_cloud_backup_uses_non_interactive_namespace_listing() {
        let interactive_namespaces_called = Arc::new(AtomicBool::new(false));
        let cloud = CloudStorage(Arc::new(Box::new(TestCloudStorage {
            interactive_namespaces_called: interactive_namespaces_called.clone(),
            non_interactive_namespaces: vec!["namespace-a".into()],
        })));

        assert!(block_on_ready(cloud.has_any_cloud_backup()).expect("cloud check should succeed"));
        assert!(!interactive_namespaces_called.load(Ordering::Acquire));
    }
}
