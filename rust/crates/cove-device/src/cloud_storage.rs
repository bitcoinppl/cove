use std::sync::Arc;

use cove_cspp::backup_data::remote_layout;
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
    AuthorizationRequired(String),
    Unavailable,
}

#[derive(Debug, Clone, Hash, Eq, PartialEq, uniffi::Record)]
pub struct RemoteBackupLocation {
    pub relative_path: String,
}

impl RemoteBackupLocation {
    fn new(relative_path: String) -> Self {
        Self { relative_path }
    }
}

impl From<String> for RemoteBackupLocation {
    fn from(relative_path: String) -> Self {
        Self::new(relative_path)
    }
}

#[uniffi::export(callback_interface)]
#[async_trait::async_trait]
pub trait CloudStorageAccess: Send + Sync + std::fmt::Debug + 'static {
    async fn upload_master_key_backup(
        &self,
        namespace: String,
        location: RemoteBackupLocation,
        data: Vec<u8>,
        policy: CloudAccessPolicy,
    ) -> Result<(), CloudStorageError>;

    async fn upload_wallet_backup(
        &self,
        namespace: String,
        record_id: String,
        location: RemoteBackupLocation,
        data: Vec<u8>,
        policy: CloudAccessPolicy,
    ) -> Result<(), CloudStorageError>;

    async fn download_master_key_backup(
        &self,
        namespace: String,
        locations: Vec<RemoteBackupLocation>,
        policy: CloudAccessPolicy,
    ) -> Result<Vec<u8>, CloudStorageError>;

    async fn download_wallet_backup(
        &self,
        namespace: String,
        record_id: String,
        locations: Vec<RemoteBackupLocation>,
        policy: CloudAccessPolicy,
    ) -> Result<Vec<u8>, CloudStorageError>;

    async fn delete_wallet_backup(
        &self,
        namespace: String,
        record_id: String,
        locations: Vec<RemoteBackupLocation>,
        policy: CloudAccessPolicy,
    ) -> Result<(), CloudStorageError>;

    async fn delete_namespace(
        &self,
        namespace: String,
        policy: CloudAccessPolicy,
    ) -> Result<(), CloudStorageError>;

    /// List all namespace IDs (subdirectories of cspp-namespaces/)
    async fn list_namespaces(
        &self,
        policy: CloudAccessPolicy,
    ) -> Result<Vec<String>, CloudStorageError>;

    /// List backup locations within a namespace
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
        locations: Vec<RemoteBackupLocation>,
        policy: CloudAccessPolicy,
    ) -> Result<bool, CloudStorageError>;

    async fn overall_sync_health(&self, policy: CloudAccessPolicy) -> CloudSyncHealth;
}

static REF: OnceCell<CloudStorage> = OnceCell::new();

#[derive(Debug, Clone, uniffi::Object)]
pub struct CloudStorage(Arc<Box<dyn CloudStorageAccess>>);

#[derive(Debug, Clone)]
pub struct CloudStorageClient(CloudStorage, CloudAccessPolicy);

impl CloudStorage {
    pub fn global() -> &'static Self {
        REF.get().expect("cloud storage is not initialized")
    }

    pub fn global_explicit_client() -> CloudStorageClient {
        Self::global().client(CloudAccessPolicy::ConsentAllowed)
    }

    pub fn global_silent_client() -> CloudStorageClient {
        Self::global().client(CloudAccessPolicy::Silent)
    }

    fn client(&self, policy: CloudAccessPolicy) -> CloudStorageClient {
        CloudStorageClient(self.clone(), policy)
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
        Ok(!self.0.list_namespaces(policy).await?.is_empty())
    }
}

impl CloudStorageClient {
    pub async fn upload_master_key_backup(
        &self,
        namespace: String,
        data: Vec<u8>,
    ) -> Result<(), CloudStorageError> {
        self.0
            .0
            .upload_master_key_backup(
                namespace,
                remote_layout::master_key_upload_location().into(),
                data,
                self.1,
            )
            .await
    }

    pub async fn upload_wallet_backup(
        &self,
        namespace: String,
        record_id: String,
        data: Vec<u8>,
    ) -> Result<(), CloudStorageError> {
        let location = remote_layout::wallet_upload_location(&record_id).into();

        self.0.0.upload_wallet_backup(namespace, record_id, location, data, self.1).await
    }

    pub async fn download_master_key_backup(
        &self,
        namespace: String,
    ) -> Result<Vec<u8>, CloudStorageError> {
        self.0
            .0
            .download_master_key_backup(
                namespace,
                locations(remote_layout::master_key_read_locations()),
                self.1,
            )
            .await
    }

    pub async fn download_wallet_backup(
        &self,
        namespace: String,
        record_id: String,
    ) -> Result<Vec<u8>, CloudStorageError> {
        let read_locations = locations(remote_layout::wallet_read_locations(&record_id));

        self.0.0.download_wallet_backup(namespace, record_id, read_locations, self.1).await
    }

    pub async fn delete_wallet_backup(
        &self,
        namespace: String,
        record_id: String,
    ) -> Result<(), CloudStorageError> {
        let delete_locations = locations(remote_layout::locations_for_record_id(&record_id));

        self.0.0.delete_wallet_backup(namespace, record_id, delete_locations, self.1).await
    }

    pub async fn delete_namespace(&self, namespace: String) -> Result<(), CloudStorageError> {
        self.0.0.delete_namespace(namespace, self.1).await
    }

    pub async fn list_namespaces(&self) -> Result<Vec<String>, CloudStorageError> {
        self.0.0.list_namespaces(self.1).await
    }

    pub async fn list_wallet_files(
        &self,
        namespace: String,
    ) -> Result<Vec<String>, CloudStorageError> {
        self.0.0.list_wallet_files(namespace, self.1).await
    }

    pub async fn is_backup_uploaded(
        &self,
        namespace: String,
        record_id: String,
    ) -> Result<bool, CloudStorageError> {
        let upload_locations = locations(remote_layout::locations_for_record_id(&record_id));

        self.0.0.is_backup_uploaded(namespace, record_id, upload_locations, self.1).await
    }

    pub async fn overall_sync_health(&self) -> CloudSyncHealth {
        self.0.0.overall_sync_health(self.1).await
    }

    pub async fn list_wallet_backups(
        &self,
        namespace: String,
    ) -> Result<Vec<String>, CloudStorageError> {
        let filenames = self.0.0.list_wallet_files(namespace, self.1).await?;
        Ok(remote_layout::dedupe_wallet_record_ids(filenames.iter().map(String::as_str)))
    }

    pub async fn has_any_cloud_backup(&self) -> Result<bool, CloudStorageError> {
        Ok(!self.list_namespaces().await?.is_empty())
    }
}

fn locations(relative_paths: Vec<String>) -> Vec<RemoteBackupLocation> {
    relative_paths.into_iter().map(RemoteBackupLocation::from).collect()
}

#[uniffi::export]
pub fn cloud_backup_locations_sync_health(
    namespace_locations: Vec<Vec<String>>,
) -> CloudSyncHealth {
    if namespace_locations
        .iter()
        .all(|locations| !remote_layout::has_backup_location(locations.iter().map(String::as_str)))
    {
        return CloudSyncHealth::NoFiles;
    }

    if namespace_locations.iter().all(|locations| {
        remote_layout::has_master_key_location(locations.iter().map(String::as_str))
    }) {
        return CloudSyncHealth::AllUploaded;
    }

    CloudSyncHealth::Failed("cloud backup is incomplete".into())
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
        expected_policy: CloudAccessPolicy,
        expected_policy_used: Arc<AtomicBool>,
        wallet_files: Option<Vec<String>>,
    }

    #[async_trait::async_trait]
    impl CloudStorageAccess for TestCloudStorage {
        async fn upload_master_key_backup(
            &self,
            _namespace: String,
            _location: RemoteBackupLocation,
            _data: Vec<u8>,
            _policy: CloudAccessPolicy,
        ) -> Result<(), CloudStorageError> {
            panic!("unused in test")
        }

        async fn upload_wallet_backup(
            &self,
            _namespace: String,
            _record_id: String,
            _location: RemoteBackupLocation,
            _data: Vec<u8>,
            _policy: CloudAccessPolicy,
        ) -> Result<(), CloudStorageError> {
            panic!("unused in test")
        }

        async fn download_master_key_backup(
            &self,
            _namespace: String,
            _locations: Vec<RemoteBackupLocation>,
            _policy: CloudAccessPolicy,
        ) -> Result<Vec<u8>, CloudStorageError> {
            panic!("unused in test")
        }

        async fn download_wallet_backup(
            &self,
            _namespace: String,
            _record_id: String,
            _locations: Vec<RemoteBackupLocation>,
            _policy: CloudAccessPolicy,
        ) -> Result<Vec<u8>, CloudStorageError> {
            panic!("unused in test")
        }

        async fn delete_wallet_backup(
            &self,
            _namespace: String,
            _record_id: String,
            _locations: Vec<RemoteBackupLocation>,
            _policy: CloudAccessPolicy,
        ) -> Result<(), CloudStorageError> {
            panic!("unused in test")
        }

        async fn delete_namespace(
            &self,
            _namespace: String,
            _policy: CloudAccessPolicy,
        ) -> Result<(), CloudStorageError> {
            panic!("unused in test")
        }

        async fn list_namespaces(
            &self,
            policy: CloudAccessPolicy,
        ) -> Result<Vec<String>, CloudStorageError> {
            if policy == self.expected_policy {
                self.expected_policy_used.store(true, Ordering::Release);
                Ok(vec!["namespace-a".into()])
            } else {
                panic!("unexpected cloud access policy")
            }
        }

        async fn list_wallet_files(
            &self,
            _namespace: String,
            _policy: CloudAccessPolicy,
        ) -> Result<Vec<String>, CloudStorageError> {
            if let Some(files) = &self.wallet_files {
                return Ok(files.clone());
            }

            panic!("unused in test")
        }

        async fn is_backup_uploaded(
            &self,
            _namespace: String,
            _record_id: String,
            _locations: Vec<RemoteBackupLocation>,
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
    fn silent_client_forwards_silent_policy() {
        let expected_policy_used = Arc::new(AtomicBool::new(false));
        let cloud = CloudStorage(Arc::new(Box::new(TestCloudStorage {
            expected_policy: CloudAccessPolicy::Silent,
            expected_policy_used: expected_policy_used.clone(),
            wallet_files: None,
        })));

        assert!(
            block_on_ready(cloud.client(CloudAccessPolicy::Silent).has_any_cloud_backup())
                .expect("cloud check should succeed")
        );
        assert!(expected_policy_used.load(Ordering::Acquire));
    }

    #[test]
    fn explicit_client_forwards_consent_allowed_policy() {
        let expected_policy_used = Arc::new(AtomicBool::new(false));
        let cloud = CloudStorage(Arc::new(Box::new(TestCloudStorage {
            expected_policy: CloudAccessPolicy::ConsentAllowed,
            expected_policy_used: expected_policy_used.clone(),
            wallet_files: None,
        })));

        assert!(
            block_on_ready(cloud.client(CloudAccessPolicy::ConsentAllowed).has_any_cloud_backup())
                .expect("cloud check should succeed")
        );
        assert!(expected_policy_used.load(Ordering::Acquire));
    }

    #[test]
    fn list_wallet_backups_dedupes_legacy_and_kind_prefixed_locations() {
        let cloud = CloudStorage(Arc::new(Box::new(TestCloudStorage {
            expected_policy: CloudAccessPolicy::Silent,
            expected_policy_used: Arc::new(AtomicBool::new(false)),
            wallet_files: Some(vec![
                "wallet-record-a.json".into(),
                "wallets/wallet-record-a.json".into(),
                "wallet-record-b.json".into(),
                "master-key/masterkey-record.json".into(),
            ]),
        })));

        let record_ids = block_on_ready(
            cloud.client(CloudAccessPolicy::Silent).list_wallet_backups("namespace".into()),
        )
        .expect("list wallet backups");

        assert_eq!(record_ids, vec!["record-a".to_string(), "record-b".to_string()]);
    }

    #[test]
    fn backup_locations_sync_health_uses_remote_layout_policy() {
        assert_eq!(cloud_backup_locations_sync_health(Vec::new()), CloudSyncHealth::NoFiles);
        assert_eq!(
            cloud_backup_locations_sync_health(vec![vec![remote_layout::master_key_location()]]),
            CloudSyncHealth::AllUploaded,
        );
        assert_eq!(
            cloud_backup_locations_sync_health(vec![vec![
                remote_layout::wallet_location_from_record_id("record-a")
            ]]),
            CloudSyncHealth::Failed("cloud backup is incomplete".into()),
        );
    }
}
