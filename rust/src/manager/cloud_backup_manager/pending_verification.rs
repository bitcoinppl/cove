use cove_cspp::backup_data::MASTER_KEY_RECORD_ID;

use crate::database::cloud_backup::{
    PersistedCloudBlobState, PersistedDeepVerificationReport,
    PersistedPendingVerificationCompletion, PersistedPendingVerificationUpload,
};

use super::{DeepVerificationReport, current_timestamp};

pub(crate) type PendingVerificationCompletion = PersistedPendingVerificationCompletion;
pub(crate) type PendingVerificationUpload = PersistedPendingVerificationUpload;

impl PendingVerificationCompletion {
    pub(crate) fn new(
        report: DeepVerificationReport,
        namespace_id: String,
        uploads: Vec<PendingVerificationUpload>,
    ) -> Self {
        Self {
            report: PersistedDeepVerificationReport::from_deep_verification_report(&report),
            namespace_id,
            uploads,
            created_at: Some(current_timestamp()),
        }
    }

    pub(crate) fn report(&self) -> DeepVerificationReport {
        self.report.to_deep_verification_report()
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
}

impl PersistedDeepVerificationReport {
    fn from_deep_verification_report(report: &DeepVerificationReport) -> Self {
        Self {
            master_key_wrapper_repaired: report.master_key_wrapper_repaired,
            local_master_key_repaired: report.local_master_key_repaired,
            credential_recovered: report.credential_recovered,
            wallets_verified: report.wallets_verified,
            wallets_failed: report.wallets_failed,
            wallets_unsupported: report.wallets_unsupported,
        }
    }

    fn to_deep_verification_report(&self) -> DeepVerificationReport {
        DeepVerificationReport {
            master_key_wrapper_repaired: self.master_key_wrapper_repaired,
            local_master_key_repaired: self.local_master_key_repaired,
            credential_recovered: self.credential_recovered,
            wallets_verified: self.wallets_verified,
            wallets_failed: self.wallets_failed,
            wallets_unsupported: self.wallets_unsupported,
            detail: None,
        }
    }
}
