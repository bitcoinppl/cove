use cove_cspp::backup_data::MASTER_KEY_RECORD_ID;

use crate::database::cloud_backup::{
    PersistedCloudBlobState, PersistedDeepVerificationReport,
    PersistedPendingVerificationCompletion, PersistedPendingVerificationUpload,
};

use super::DeepVerificationReport;

#[derive(Debug, Clone)]
pub(crate) struct PendingVerificationCompletion {
    pub(crate) report: DeepVerificationReport,
    pub(crate) namespace_id: String,
    pub(crate) uploads: Vec<PendingVerificationUpload>,
    pub(crate) created_at: Option<u64>,
}

#[derive(Debug, Clone)]
pub(crate) enum PendingVerificationUpload {
    MasterKeyWrapper,
    Wallet { record_id: String, expected_revision: String },
}

impl PendingVerificationCompletion {
    pub(crate) fn new(
        report: DeepVerificationReport,
        namespace_id: String,
        uploads: Vec<PendingVerificationUpload>,
    ) -> Self {
        Self {
            report,
            namespace_id,
            uploads,
            created_at: Some(crate::manager::cloud_backup_manager::current_timestamp()),
        }
    }

    pub(crate) fn report(&self) -> &DeepVerificationReport {
        &self.report
    }

    pub(crate) fn namespace_id(&self) -> &str {
        &self.namespace_id
    }

    pub(crate) fn uploads(&self) -> &[PendingVerificationUpload] {
        &self.uploads
    }

    pub(crate) fn is_expired(&self, now: u64, ttl_seconds: u64) -> bool {
        let Some(created_at) = self.created_at else {
            // legacy persisted completions predate created_at and must be restarted
            return true;
        };

        if created_at > now {
            return true;
        }

        now.saturating_sub(created_at) >= ttl_seconds
    }

    pub(crate) fn persisted(&self) -> PersistedPendingVerificationCompletion {
        PersistedPendingVerificationCompletion {
            report: PersistedDeepVerificationReport::from(&self.report),
            namespace_id: self.namespace_id.clone(),
            created_at: self.created_at,
            uploads: self
                .uploads
                .iter()
                .cloned()
                .map(PersistedPendingVerificationUpload::from)
                .collect(),
        }
    }

    pub(crate) fn from_persisted(completion: PersistedPendingVerificationCompletion) -> Self {
        Self {
            report: DeepVerificationReport::from(completion.report),
            namespace_id: completion.namespace_id,
            created_at: completion.created_at,
            uploads: completion
                .uploads
                .into_iter()
                .map(PendingVerificationUpload::from_persisted)
                .collect(),
        }
    }
}

impl PendingVerificationUpload {
    pub(crate) fn new(record_id: String, expected_revision: String) -> Self {
        Self::Wallet { record_id, expected_revision }
    }

    pub(crate) fn master_key_wrapper() -> Self {
        Self::MasterKeyWrapper
    }

    pub(crate) fn record_id(&self) -> &str {
        match self {
            Self::MasterKeyWrapper => MASTER_KEY_RECORD_ID,
            Self::Wallet { record_id, .. } => record_id,
        }
    }

    pub(crate) fn expected_revision(&self) -> &str {
        match self {
            Self::MasterKeyWrapper => "master-key-wrapper",
            Self::Wallet { expected_revision, .. } => expected_revision,
        }
    }

    pub(crate) fn wallet_record_id(&self) -> Option<&str> {
        match self {
            Self::MasterKeyWrapper => None,
            Self::Wallet { record_id, .. } => Some(record_id),
        }
    }

    pub(crate) fn wallet_revision(&self) -> Option<&str> {
        match self {
            Self::MasterKeyWrapper => None,
            Self::Wallet { expected_revision, .. } => Some(expected_revision),
        }
    }

    pub(crate) fn target_revision(&self, sync_state: Option<&PersistedCloudBlobState>) -> String {
        let Self::Wallet { expected_revision, .. } = self else {
            return self.expected_revision().to_owned();
        };

        sync_state
            .and_then(PersistedCloudBlobState::revision_hash)
            .unwrap_or(expected_revision)
            .to_owned()
    }

    fn from_persisted(upload: PersistedPendingVerificationUpload) -> Self {
        match upload {
            PersistedPendingVerificationUpload::MasterKeyWrapper => Self::MasterKeyWrapper,
            PersistedPendingVerificationUpload::Wallet { record_id, expected_revision } => {
                Self::Wallet { record_id, expected_revision }
            }
        }
    }
}

impl DeepVerificationReport {
    fn from(report: PersistedDeepVerificationReport) -> Self {
        Self {
            master_key_wrapper_repaired: report.master_key_wrapper_repaired,
            local_master_key_repaired: report.local_master_key_repaired,
            credential_recovered: report.credential_recovered,
            wallets_verified: report.wallets_verified,
            wallets_failed: report.wallets_failed,
            wallets_unsupported: report.wallets_unsupported,
            detail: None,
        }
    }
}

impl From<&DeepVerificationReport> for PersistedDeepVerificationReport {
    fn from(report: &DeepVerificationReport) -> Self {
        Self {
            master_key_wrapper_repaired: report.master_key_wrapper_repaired,
            local_master_key_repaired: report.local_master_key_repaired,
            credential_recovered: report.credential_recovered,
            wallets_verified: report.wallets_verified,
            wallets_failed: report.wallets_failed,
            wallets_unsupported: report.wallets_unsupported,
        }
    }
}

impl From<PendingVerificationUpload> for PersistedPendingVerificationUpload {
    fn from(upload: PendingVerificationUpload) -> Self {
        match upload {
            PendingVerificationUpload::MasterKeyWrapper => Self::MasterKeyWrapper,
            PendingVerificationUpload::Wallet { record_id, expected_revision } => {
                Self::Wallet { record_id, expected_revision }
            }
        }
    }
}
