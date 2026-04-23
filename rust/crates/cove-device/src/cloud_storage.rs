use std::sync::Arc;

use cove_cspp::backup_data::wallet_record_id_from_filename;
use once_cell::sync::OnceCell;
use tracing::warn;

#[derive(Debug, Clone, Hash, Eq, PartialEq, uniffi::Error, thiserror::Error)]
#[uniffi::export(Display)]
pub enum CloudStorageError {
    #[error("authorization required: {0}")]
    AuthorizationRequired(String),

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

#[derive(Debug, Clone, Copy, Hash, Eq, PartialEq, uniffi::Enum)]
pub enum CloudAccessPolicy {
    ConsentAllowed,
    Silent,
}

#[derive(Debug, Clone, Hash, Eq, PartialEq, uniffi::Enum)]
pub enum CloudSyncHealth {
    Unknown,
    AllUploaded,
    Uploading,
    Failed(String),
    NoFiles,
    AuthorizationRequired,
    Unavailable,
}

#[uniffi::export(callback_interface)]
#[async_trait::async_trait]
pub trait CloudStorageAccess: Send + Sync + std::fmt::Debug + 'static {
    async fn upload_master_key_backup(
        &self,
        namespace: String,
        data: Vec<u8>,
        policy: CloudAccessPolicy,
    ) -> Result<(), CloudStorageError>;

    async fn upload_wallet_backup(
        &self,
        namespace: String,
        record_id: String,
        data: Vec<u8>,
        policy: CloudAccessPolicy,
    ) -> Result<(), CloudStorageError>;

    async fn download_master_key_backup(
        &self,
        namespace: String,
        policy: CloudAccessPolicy,
    ) -> Result<Vec<u8>, CloudStorageError>;

    async fn download_wallet_backup(
        &self,
        namespace: String,
        record_id: String,
        policy: CloudAccessPolicy,
    ) -> Result<Vec<u8>, CloudStorageError>;

    async fn delete_wallet_backup(
        &self,
        namespace: String,
        record_id: String,
        policy: CloudAccessPolicy,
    ) -> Result<(), CloudStorageError>;

    /// List all namespace IDs (subdirectories of cspp-namespaces/)
    async fn list_namespaces(
        &self,
        policy: CloudAccessPolicy,
    ) -> Result<Vec<String>, CloudStorageError>;

    /// List wallet backup filenames within a namespace
    async fn list_wallet_files(
        &self,
        namespace: String,
        policy: CloudAccessPolicy,
    ) -> Result<Vec<String>, CloudStorageError>;

    /// Check whether a blob has been fully uploaded to iCloud
    async fn is_backup_uploaded(
        &self,
        namespace: String,
        record_id: String,
        policy: CloudAccessPolicy,
    ) -> Result<bool, CloudStorageError>;

    async fn overall_sync_health(&self, policy: CloudAccessPolicy) -> CloudSyncHealth;
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
    pub async fn has_any_cloud_backup(
        &self,
        policy: CloudAccessPolicy,
    ) -> Result<bool, CloudStorageError> {
        Ok(!self.list_namespaces(policy).await?.is_empty())
    }
}

impl CloudStorage {
    pub async fn upload_master_key_backup(
        &self,
        namespace: String,
        data: Vec<u8>,
        policy: CloudAccessPolicy,
    ) -> Result<(), CloudStorageError> {
        self.0.upload_master_key_backup(namespace, data, policy).await
    }

    pub async fn upload_wallet_backup(
        &self,
        namespace: String,
        record_id: String,
        data: Vec<u8>,
        policy: CloudAccessPolicy,
    ) -> Result<(), CloudStorageError> {
        self.0.upload_wallet_backup(namespace, record_id, data, policy).await
    }

    pub async fn download_master_key_backup(
        &self,
        namespace: String,
        policy: CloudAccessPolicy,
    ) -> Result<Vec<u8>, CloudStorageError> {
        self.0.download_master_key_backup(namespace, policy).await
    }

    pub async fn download_wallet_backup(
        &self,
        namespace: String,
        record_id: String,
        policy: CloudAccessPolicy,
    ) -> Result<Vec<u8>, CloudStorageError> {
        self.0.download_wallet_backup(namespace, record_id, policy).await
    }

    pub async fn delete_wallet_backup(
        &self,
        namespace: String,
        record_id: String,
        policy: CloudAccessPolicy,
    ) -> Result<(), CloudStorageError> {
        self.0.delete_wallet_backup(namespace, record_id, policy).await
    }

    pub async fn list_namespaces(
        &self,
        policy: CloudAccessPolicy,
    ) -> Result<Vec<String>, CloudStorageError> {
        self.0.list_namespaces(policy).await
    }

    pub async fn list_wallet_files(
        &self,
        namespace: String,
        policy: CloudAccessPolicy,
    ) -> Result<Vec<String>, CloudStorageError> {
        self.0.list_wallet_files(namespace, policy).await
    }

    pub async fn is_backup_uploaded(
        &self,
        namespace: String,
        record_id: String,
        policy: CloudAccessPolicy,
    ) -> Result<bool, CloudStorageError> {
        self.0.is_backup_uploaded(namespace, record_id, policy).await
    }

    pub async fn overall_sync_health(&self, policy: CloudAccessPolicy) -> CloudSyncHealth {
        self.0.overall_sync_health(policy).await
    }

    pub async fn list_wallet_backups(
        &self,
        namespace: String,
        policy: CloudAccessPolicy,
    ) -> Result<Vec<String>, CloudStorageError> {
        let filenames = self.0.list_wallet_files(namespace, policy).await?;
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
        consent_allowed_namespaces_called: Arc<AtomicBool>,
        silent_namespaces: Vec<String>,
    }

    #[async_trait::async_trait]
    impl CloudStorageAccess for TestCloudStorage {
        async fn upload_master_key_backup(
            &self,
            _namespace: String,
            _data: Vec<u8>,
            _policy: CloudAccessPolicy,
        ) -> Result<(), CloudStorageError> {
            panic!("unused in test")
        }

        async fn upload_wallet_backup(
            &self,
            _namespace: String,
            _record_id: String,
            _data: Vec<u8>,
            _policy: CloudAccessPolicy,
        ) -> Result<(), CloudStorageError> {
            panic!("unused in test")
        }

        async fn download_master_key_backup(
            &self,
            _namespace: String,
            _policy: CloudAccessPolicy,
        ) -> Result<Vec<u8>, CloudStorageError> {
            panic!("unused in test")
        }

        async fn download_wallet_backup(
            &self,
            _namespace: String,
            _record_id: String,
            _policy: CloudAccessPolicy,
        ) -> Result<Vec<u8>, CloudStorageError> {
            panic!("unused in test")
        }

        async fn delete_wallet_backup(
            &self,
            _namespace: String,
            _record_id: String,
            _policy: CloudAccessPolicy,
        ) -> Result<(), CloudStorageError> {
            panic!("unused in test")
        }

        async fn list_namespaces(
            &self,
            policy: CloudAccessPolicy,
        ) -> Result<Vec<String>, CloudStorageError> {
            match policy {
                CloudAccessPolicy::ConsentAllowed => {
                    self.consent_allowed_namespaces_called.store(true, Ordering::Release);
                    panic!("consent-allowed namespace listing should not be used")
                }
                CloudAccessPolicy::Silent => Ok(self.silent_namespaces.clone()),
            }
        }

        async fn list_wallet_files(
            &self,
            _namespace: String,
            _policy: CloudAccessPolicy,
        ) -> Result<Vec<String>, CloudStorageError> {
            panic!("unused in test")
        }

        async fn is_backup_uploaded(
            &self,
            _namespace: String,
            _record_id: String,
            _policy: CloudAccessPolicy,
        ) -> Result<bool, CloudStorageError> {
            panic!("unused in test")
        }

        async fn overall_sync_health(&self, _policy: CloudAccessPolicy) -> CloudSyncHealth {
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
    fn has_any_cloud_backup_uses_silent_namespace_listing() {
        let consent_allowed_namespaces_called = Arc::new(AtomicBool::new(false));
        let cloud = CloudStorage(Arc::new(Box::new(TestCloudStorage {
            consent_allowed_namespaces_called: consent_allowed_namespaces_called.clone(),
            silent_namespaces: vec!["namespace-a".into()],
        })));

        assert!(
            block_on_ready(cloud.has_any_cloud_backup(CloudAccessPolicy::Silent))
                .expect("cloud check should succeed")
        );
        assert!(!consent_allowed_namespaces_called.load(Ordering::Acquire));
    }
}
