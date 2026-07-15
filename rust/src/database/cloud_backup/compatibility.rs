//! Compatibility serde for the alpha cloud backup persistence rollout
//!
//! Keep this module while alpha builds may still have redb JSON from flat cloud backup persistence structs
//!
//! Removal criteria:
//! - Ship one alpha release that reads legacy JSON and writes the versioned domain JSON
//! - Confirm active alpha testers have opened that release once, or intentionally reset alpha data
//! - Remove this module, the untagged legacy decoders, and the legacy JSON fixture tests together
//!
//! Why this exists:
//! - Table names stay stable during the domain model refactor
//! - Old installs can launch without an eager redb migration
//! - The next successful write naturally replaces legacy JSON with the new shape
//!
//! How to remove it:
//! - Derive Serialize and Deserialize directly on the persisted structs in `cloud_backup.rs`
//! - Keep only the new domain JSON shape if the persisted structs still differ from the runtime model
//! - Delete tests whose names mention legacy JSON

use cove_cspp::backup_data::MASTER_KEY_RECORD_ID;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use super::{
    CloudBackupRecordKey, PersistedBackupSyncState, PersistedBackupVerificationState,
    PersistedCloudBackupState, PersistedCloudBackupStatus, PersistedCloudBlobState,
    PersistedCloudBlobSyncState, PersistedConfiguredCloudBackup, PersistedDisablingCloudBackup,
    PersistedPasskeyState, PersistedPendingVerificationCompletion,
    PersistedPendingVerificationUpload,
};
use crate::wallet::metadata::WalletId;

impl<'de> Deserialize<'de> for PersistedCloudBackupState {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = serde_json::Value::deserialize(deserializer)?;
        if value.get("version").is_some() || value.get("backup").is_some() {
            let record = serde_json::from_value::<PersistedCloudBackupDomainRecord>(value)
                .map_err(serde::de::Error::custom)?;
            return record.into_state().map_err(serde::de::Error::custom);
        }

        serde_json::from_value::<PersistedLegacyCloudBackupState>(value)
            .map(Into::into)
            .map_err(serde::de::Error::custom)
    }
}

impl Serialize for PersistedCloudBackupState {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        PersistedCloudBackupDomainRecord::from_state(self).serialize(serializer)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
struct PersistedLegacyCloudBackupState {
    status: PersistedCloudBackupStatus,
    #[serde(default)]
    last_sync: Option<u64>,
    #[serde(default)]
    wallet_count: Option<u32>,
    #[serde(default)]
    last_verified_at: Option<u64>,
    #[serde(default)]
    last_verification_requested_at: Option<u64>,
    #[serde(default)]
    last_verification_dismissed_at: Option<u64>,
    #[serde(default)]
    pending_verification_completion: Option<PersistedPendingVerificationCompletion>,
}

impl From<PersistedLegacyCloudBackupState> for PersistedCloudBackupState {
    fn from(state: PersistedLegacyCloudBackupState) -> Self {
        if matches!(state.status, PersistedCloudBackupStatus::Disabled) {
            return Self::Disabled;
        }
        if matches!(state.status, PersistedCloudBackupStatus::Corrupted) {
            return Self::corrupted("legacy persisted cloud backup state was marked corrupted");
        }

        Self::Configured(PersistedConfiguredCloudBackup {
            passkey: legacy_passkey_state(state.status),
            verification: legacy_verification_state(
                state.status,
                state.last_verified_at,
                state.last_verification_requested_at,
                state.last_verification_dismissed_at,
            ),
            sync: PersistedBackupSyncState {
                last_sync: state.last_sync,
                wallet_count: state.wallet_count,
            },
            pending_verification_completion: state.pending_verification_completion,
            drive_account_switch: None,
        })
    }
}

fn legacy_passkey_state(status: PersistedCloudBackupStatus) -> PersistedPasskeyState {
    match status {
        PersistedCloudBackupStatus::PasskeyMissing => PersistedPasskeyState::Missing,
        PersistedCloudBackupStatus::Disabled
        | PersistedCloudBackupStatus::Enabled
        | PersistedCloudBackupStatus::Unverified
        | PersistedCloudBackupStatus::Disabling => PersistedPasskeyState::Available,
        PersistedCloudBackupStatus::Corrupted => PersistedPasskeyState::Missing,
    }
}

fn legacy_verification_state(
    status: PersistedCloudBackupStatus,
    last_verified_at: Option<u64>,
    requested_at: Option<u64>,
    dismissed_at: Option<u64>,
) -> PersistedBackupVerificationState {
    if matches!(status, PersistedCloudBackupStatus::Unverified) {
        return PersistedBackupVerificationState::Required {
            last_verified_at,
            requested_at,
            dismissed_at,
        };
    }

    match last_verified_at {
        Some(last_verified_at) => PersistedBackupVerificationState::Verified {
            last_verified_at,
            requested_at,
            dismissed_at,
        },
        None => PersistedBackupVerificationState::NotVerified { requested_at, dismissed_at },
    }
}

impl<'de> Deserialize<'de> for PersistedPendingVerificationUpload {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        enum TaggedUpload {
            MasterKeyWrapper,
            Wallet { record_id: String, expected_revision: String },
        }

        #[derive(Deserialize)]
        struct LegacyWalletUpload {
            record_id: String,
            expected_revision: String,
        }

        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Upload {
            Tagged(TaggedUpload),
            LegacyWallet(LegacyWalletUpload),
        }

        match Upload::deserialize(deserializer)? {
            Upload::Tagged(TaggedUpload::MasterKeyWrapper) => Ok(Self::MasterKeyWrapper),
            Upload::Tagged(TaggedUpload::Wallet { record_id, expected_revision })
            | Upload::LegacyWallet(LegacyWalletUpload { record_id, expected_revision }) => {
                Ok(Self::Wallet { record_id, expected_revision })
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
struct PersistedCloudBackupDomainRecord {
    version: u16,
    backup: PersistedBackupRecord,
}

impl<'de> Deserialize<'de> for PersistedCloudBackupDomainRecord {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct RawRecord {
            version: u16,
            backup: serde_json::Value,
        }

        let record = RawRecord::deserialize(deserializer)?;
        let backup = match record.version {
            1 => {
                let backup = serde_json::from_value::<PersistedBackupRecordV1>(record.backup)
                    .map_err(serde::de::Error::custom)?;
                backup.into_current()
            }
            2 => serde_json::from_value(record.backup).map_err(serde::de::Error::custom)?,
            version => {
                return Err(serde::de::Error::custom(format!(
                    "unsupported persisted cloud backup record version: {version}"
                )));
            }
        };

        Ok(Self { version: record.version, backup })
    }
}

impl PersistedCloudBackupDomainRecord {
    fn from_state(state: &PersistedCloudBackupState) -> Self {
        let backup = match state {
            PersistedCloudBackupState::Disabled => PersistedBackupRecord::Disabled,
            PersistedCloudBackupState::Configured(configured) => {
                PersistedBackupRecord::Configured(configured.clone())
            }
            PersistedCloudBackupState::Disabling(disabling) => {
                PersistedBackupRecord::Disabling(disabling.clone())
            }
            PersistedCloudBackupState::Corrupted { error } => {
                PersistedBackupRecord::Corrupted { error: error.clone() }
            }
        };

        Self { version: 2, backup }
    }

    fn into_state(self) -> Result<PersistedCloudBackupState, String> {
        if self.version != 1 && self.version != 2 {
            return Err(format!(
                "unsupported persisted cloud backup record version: {}",
                self.version
            ));
        }

        match self.backup {
            PersistedBackupRecord::Disabled => Ok(PersistedCloudBackupState::default()),
            PersistedBackupRecord::Configured(configured) => {
                Ok(PersistedCloudBackupState::Configured(configured))
            }
            PersistedBackupRecord::Disabling(disabling) => {
                Ok(PersistedCloudBackupState::Disabling(disabling))
            }
            PersistedBackupRecord::Corrupted { error } => {
                Ok(PersistedCloudBackupState::Corrupted { error })
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(tag = "state", content = "data")]
enum PersistedBackupRecordV1 {
    Disabled,
    Configured(PersistedConfiguredCloudBackup),
}

impl PersistedBackupRecordV1 {
    fn into_current(self) -> PersistedBackupRecord {
        match self {
            Self::Disabled => PersistedBackupRecord::Disabled,
            Self::Configured(configured) => PersistedBackupRecord::Configured(configured),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", content = "data")]
enum PersistedBackupRecord {
    Disabled,
    Configured(PersistedConfiguredCloudBackup),
    Disabling(PersistedDisablingCloudBackup),
    Corrupted { error: String },
}

impl Serialize for PersistedCloudBlobSyncState {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        PersistedCloudBlobSyncDomainRecord::from_sync_state(self).serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for PersistedCloudBlobSyncState {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum PersistedCloudBlobSyncStateShape {
            Domain(PersistedCloudBlobSyncDomainRecord),
            Legacy(PersistedLegacyCloudBlobSyncState),
        }

        match PersistedCloudBlobSyncStateShape::deserialize(deserializer)? {
            PersistedCloudBlobSyncStateShape::Domain(record) => {
                record.into_sync_state().map_err(serde::de::Error::custom)
            }
            PersistedCloudBlobSyncStateShape::Legacy(state) => {
                state.into_sync_state().map_err(serde::de::Error::custom)
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct PersistedCloudBlobSyncDomainRecord {
    version: u16,
    namespace_id: String,
    record_key: PersistedCloudBlobRecordKey,
    state: PersistedCloudBlobState,
}

impl PersistedCloudBlobSyncDomainRecord {
    fn from_sync_state(state: &PersistedCloudBlobSyncState) -> Self {
        Self {
            version: 1,
            namespace_id: state.namespace_id.clone(),
            record_key: PersistedCloudBlobRecordKey::from_record_key(state.record_key().clone()),
            state: state.state.clone(),
        }
    }

    fn into_sync_state(self) -> Result<PersistedCloudBlobSyncState, String> {
        if self.version != 1 {
            return Err(format!(
                "unsupported persisted cloud backup blob sync record version: {}",
                self.version
            ));
        }

        Ok(PersistedCloudBlobSyncState::from_record_key(
            self.namespace_id,
            self.record_key.into_record_key(),
            self.state,
        ))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct PersistedLegacyCloudBlobSyncState {
    namespace_id: String,
    wallet_id: Option<WalletId>,
    record_id: String,
    state: PersistedCloudBlobState,
}

impl From<PersistedLegacyCloudBlobSyncState> for PersistedCloudBlobSyncState {
    fn from(state: PersistedLegacyCloudBlobSyncState) -> Self {
        state.into_sync_state().unwrap_or_else(|error| panic!("{error}"))
    }
}

impl PersistedLegacyCloudBlobSyncState {
    fn into_sync_state(self) -> Result<PersistedCloudBlobSyncState, String> {
        if self.wallet_id.is_none() && self.record_id == MASTER_KEY_RECORD_ID {
            return Ok(PersistedCloudBlobSyncState::master_key_wrapper(
                self.namespace_id,
                self.state,
            ));
        }

        let Some(wallet_id) = self.wallet_id else {
            return Err(format!(
                "invalid legacy blob: missing wallet_id for record_id {}",
                self.record_id
            ));
        };

        let record_key = CloudBackupRecordKey::Wallet(wallet_id, self.record_id);

        Ok(PersistedCloudBlobSyncState::from_record_key(self.namespace_id, record_key, self.state))
    }
}

impl From<&PersistedCloudBlobSyncState> for PersistedLegacyCloudBlobSyncState {
    fn from(state: &PersistedCloudBlobSyncState) -> Self {
        let (wallet_id, record_id) = state.record_key().clone().into_parts();

        Self {
            namespace_id: state.namespace_id.clone(),
            wallet_id,
            record_id,
            state: state.state.clone(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind")]
enum PersistedCloudBlobRecordKey {
    MasterKeyWrapper,
    Wallet { wallet_id: WalletId, record_id: String },
    Corrupted { record_id: String },
}

impl PersistedCloudBlobRecordKey {
    fn from_record_key(key: CloudBackupRecordKey) -> Self {
        match key {
            CloudBackupRecordKey::MasterKeyWrapper => Self::MasterKeyWrapper,
            CloudBackupRecordKey::Wallet(wallet_id, record_id) => {
                Self::Wallet { wallet_id, record_id }
            }
            CloudBackupRecordKey::Corrupted(record_id) => Self::Corrupted { record_id },
        }
    }

    fn into_record_key(self) -> CloudBackupRecordKey {
        match self {
            Self::MasterKeyWrapper => CloudBackupRecordKey::MasterKeyWrapper,
            Self::Wallet { wallet_id, record_id } => {
                CloudBackupRecordKey::Wallet(wallet_id, record_id)
            }
            Self::Corrupted { record_id } => CloudBackupRecordKey::Corrupted(record_id),
        }
    }
}

#[cfg(test)]
mod tests {
    use cove_cspp::backup_data::MASTER_KEY_RECORD_ID;

    use super::super::{
        CloudBackupRecordKey, CloudBlobDirtyState, CloudBlobFailedState, PersistedBackupSyncState,
        PersistedBackupVerificationState, PersistedCloudBackupState, PersistedCloudBackupStatus,
        PersistedCloudBlobState, PersistedCloudBlobSyncState, PersistedConfiguredCloudBackup,
        PersistedDriveAccountSwitch, PersistedDriveAccountSwitchPhase, PersistedPasskeyState,
        PersistedPendingVerificationUpload,
    };

    fn configured_state(
        passkey: PersistedPasskeyState,
        verification: PersistedBackupVerificationState,
        last_sync: Option<u64>,
        wallet_count: Option<u32>,
    ) -> PersistedCloudBackupState {
        PersistedCloudBackupState::Configured(PersistedConfiguredCloudBackup {
            passkey,
            verification,
            sync: PersistedBackupSyncState { last_sync, wallet_count },
            pending_verification_completion: None,
            drive_account_switch: None,
        })
    }

    #[test]
    fn cloud_backup_state_accepts_legacy_status_timestamp_json() {
        let state: PersistedCloudBackupState = serde_json::from_value(serde_json::json!({
            "status": "Unverified",
            "last_sync": 10,
            "wallet_count": 2,
            "last_verified_at": 11,
            "last_verification_requested_at": 20,
            "last_verification_dismissed_at": 12
        }))
        .unwrap();

        assert_eq!(state.status(), PersistedCloudBackupStatus::Unverified);
        assert_eq!(state.last_sync(), Some(10));
        assert_eq!(state.wallet_count(), Some(2));
        assert_eq!(state.last_verified_at(), Some(11));
        assert_eq!(state.last_verification_requested_at(), Some(20));
        assert_eq!(state.last_verification_dismissed_at(), Some(12));
        assert!(state.should_prompt_verification());
    }

    #[test]
    fn cloud_backup_state_serializes_domain_shape() {
        let state = configured_state(
            PersistedPasskeyState::Missing,
            PersistedBackupVerificationState::Verified {
                last_verified_at: 11,
                requested_at: Some(20),
                dismissed_at: Some(12),
            },
            Some(10),
            Some(2),
        );

        let encoded = serde_json::to_value(&state).unwrap();

        assert_eq!(encoded["version"], 2);
        assert_eq!(encoded["backup"]["state"], "Configured");
        assert_eq!(encoded["backup"]["data"]["passkey"], "Missing");
        assert_eq!(encoded["backup"]["data"]["verification"]["state"], "Verified");
        assert_eq!(encoded["backup"]["data"]["sync"]["last_sync"], 10);
        assert_eq!(encoded["backup"]["data"]["sync"]["wallet_count"], 2);
        assert!(encoded.get("status").is_none());
        assert!(encoded.get("last_sync").is_none());
    }

    #[test]
    fn cloud_backup_state_round_trips_drive_account_switch_without_changing_record_version() {
        let mut state = configured_state(
            PersistedPasskeyState::Available,
            PersistedBackupVerificationState::NotVerified {
                requested_at: None,
                dismissed_at: None,
            },
            Some(10),
            Some(2),
        );
        state.set_drive_account_switch(PersistedDriveAccountSwitch {
            transition_id: 7,
            phase: PersistedDriveAccountSwitchPhase::Reinitializing,
        });

        let encoded = serde_json::to_value(&state).unwrap();
        let decoded: PersistedCloudBackupState = serde_json::from_value(encoded.clone()).unwrap();

        assert_eq!(encoded["version"], 2);
        assert_eq!(decoded, state);
    }

    #[test]
    fn cloud_backup_state_serializes_corrupted_domain_shape() {
        let state = PersistedCloudBackupState::corrupted("decode failed");

        let encoded = serde_json::to_value(&state).unwrap();

        assert_eq!(encoded["version"], 2);
        assert_eq!(encoded["backup"]["state"], "Corrupted");
        assert_eq!(encoded["backup"]["data"]["error"], "decode failed");
    }

    #[test]
    fn cloud_backup_state_accepts_corrupted_domain_json() {
        let state: PersistedCloudBackupState = serde_json::from_value(serde_json::json!({
            "version": 2,
            "backup": {
                "state": "Corrupted",
                "data": {
                    "error": "decode failed"
                }
            }
        }))
        .unwrap();

        assert_eq!(state, PersistedCloudBackupState::corrupted("decode failed"));
        assert_eq!(state.status(), PersistedCloudBackupStatus::Corrupted);
    }

    #[test]
    fn cloud_backup_state_accepts_v1_domain_json() {
        let state: PersistedCloudBackupState = serde_json::from_value(serde_json::json!({
            "version": 1,
            "backup": {
                "state": "Configured",
                "data": {
                    "passkey": "Available",
                    "verification": {
                        "state": "Required",
                        "data": {
                            "last_verified_at": 11,
                            "requested_at": 20,
                            "dismissed_at": 12
                        }
                    },
                    "sync": {
                        "last_sync": 10,
                        "wallet_count": 2
                    }
                }
            }
        }))
        .unwrap();

        assert_eq!(state.status(), PersistedCloudBackupStatus::Unverified);
        assert_eq!(state.last_sync(), Some(10));
        assert_eq!(state.wallet_count(), Some(2));
        assert_eq!(state.last_verified_at(), Some(11));
        assert_eq!(state.last_verification_requested_at(), Some(20));
        assert_eq!(state.last_verification_dismissed_at(), Some(12));
    }

    #[test]
    fn cloud_backup_state_accepts_v2_disabling_domain_json() {
        let state: PersistedCloudBackupState = serde_json::from_value(serde_json::json!({
            "version": 2,
            "backup": {
                "state": "Disabling",
                "data": {
                    "previous_configured": {
                        "passkey": "Available",
                        "verification": {
                            "state": "Verified",
                            "data": {
                                "last_verified_at": 11,
                                "requested_at": 20,
                                "dismissed_at": 12
                            }
                        },
                        "sync": {
                            "last_sync": 10,
                            "wallet_count": 2
                        }
                    },
                    "namespace_id": "namespace-1",
                    "disable_generation": 99,
                    "started_at": 30,
                    "delete_started_at": null,
                    "last_error": null,
                    "retry_after": null
                }
            }
        }))
        .unwrap();

        assert_eq!(state.status(), PersistedCloudBackupStatus::Disabling);
        assert_eq!(state.last_sync(), Some(10));
        assert_eq!(state.wallet_count(), Some(2));
    }

    #[test]
    fn cloud_backup_state_accepts_pending_verification_completion_without_created_at() {
        let state: PersistedCloudBackupState = serde_json::from_value(serde_json::json!({
            "version": 2,
            "backup": {
                "state": "Configured",
                "data": {
                    "passkey": "Available",
                    "verification": {
                        "state": "NotVerified",
                        "data": {}
                    },
                    "sync": {},
                    "pending_verification_completion": {
                        "report": {
                            "master_key_wrapper_repaired": false,
                            "local_master_key_repaired": false,
                            "credential_recovered": false,
                            "wallets_verified": 0,
                            "wallets_failed": 0,
                            "wallets_unsupported": 0
                        },
                        "namespace_id": "0123456789abcdef0123456789abcdef",
                        "uploads": ["MasterKeyWrapper"]
                    }
                }
            }
        }))
        .unwrap();

        let PersistedCloudBackupState::Configured(configured) = state else {
            panic!("expected configured cloud backup state");
        };
        let completion =
            configured.pending_verification_completion.expect("pending verification completion");

        assert_eq!(completion.created_at, None);
        assert_eq!(completion.uploads, vec![PersistedPendingVerificationUpload::MasterKeyWrapper]);
    }

    #[test]
    fn cloud_backup_state_rejects_v1_disabling_domain_json() {
        let error = serde_json::from_value::<PersistedCloudBackupState>(serde_json::json!({
            "version": 1,
            "backup": {
                "state": "Disabling",
                "data": {
                    "previous_configured": {
                        "passkey": "Available",
                        "verification": {
                            "state": "NotVerified",
                            "data": {}
                        },
                        "sync": {}
                    },
                    "namespace_id": "namespace-1",
                    "disable_generation": 99,
                    "started_at": 30
                }
            }
        }))
        .unwrap_err();

        assert!(error.to_string().contains("unknown variant"), "{error}");
    }

    #[test]
    fn cloud_backup_state_rejects_unsupported_domain_version() {
        let error = serde_json::from_value::<PersistedCloudBackupState>(serde_json::json!({
            "version": 3,
            "backup": {
                "state": "Disabled"
            }
        }))
        .unwrap_err();

        assert!(
            error.to_string().contains("unsupported persisted cloud backup record version: 3"),
            "{error}"
        );
    }

    #[test]
    fn blob_sync_state_accepts_legacy_master_key_json() {
        let state: PersistedCloudBlobSyncState = serde_json::from_value(serde_json::json!({
            "namespace_id": "ns-1",
            "wallet_id": null,
            "record_id": MASTER_KEY_RECORD_ID,
            "state": {
                "Dirty": {
                    "changed_at": 10
                }
            }
        }))
        .unwrap();

        assert_eq!(state.record_key(), &CloudBackupRecordKey::MasterKeyWrapper);
        assert!(state.is_master_key_wrapper());
    }

    #[test]
    fn blob_sync_state_accepts_legacy_wallet_json() {
        let state: PersistedCloudBlobSyncState = serde_json::from_value(serde_json::json!({
            "namespace_id": "ns-1",
            "wallet_id": "wallet-a",
            "record_id": "record-a",
            "state": {
                "Dirty": {
                    "changed_at": 10
                }
            }
        }))
        .unwrap();

        assert_eq!(
            state.record_key(),
            &CloudBackupRecordKey::Wallet("wallet-a".into(), "record-a".into())
        );
        assert!(state.is_wallet_record());
    }

    #[test]
    fn blob_sync_state_serializes_domain_shape() {
        let state = PersistedCloudBlobSyncState::wallet(
            "ns-1".into(),
            "wallet-a".into(),
            "record-a".into(),
            PersistedCloudBlobState::Dirty(CloudBlobDirtyState { changed_at: 10 }),
        );

        let encoded = serde_json::to_value(&state).unwrap();

        assert_eq!(encoded["version"], 1);
        assert_eq!(encoded["record_key"]["kind"], "Wallet");
        assert_eq!(encoded["record_key"]["wallet_id"], "wallet-a");
        assert_eq!(encoded["record_key"]["record_id"], "record-a");
        assert!(encoded.get("wallet_id").is_none());
        assert!(encoded.get("record_id").is_none());
    }

    #[test]
    fn blob_sync_state_serializes_corrupt_domain_shape() {
        let state = PersistedCloudBlobSyncState::corrupted("decode failed".into());

        let encoded = serde_json::to_value(&state).unwrap();

        assert_eq!(encoded["version"], 1);
        assert_eq!(encoded["record_key"]["kind"], "Corrupted");
        assert_eq!(
            encoded["record_key"]["record_id"],
            crate::database::cloud_backup::state::CORRUPT_BLOB_SYNC_RECORD_ID
        );
    }

    #[test]
    fn blob_sync_state_accepts_corrupt_domain_json() {
        let state: PersistedCloudBlobSyncState = serde_json::from_value(serde_json::json!({
            "version": 1,
            "namespace_id": "ns-1",
            "record_key": {
                "kind": "Corrupted",
                "record_id": "__corrupt_cloud_backup_blob_sync_state__"
            },
            "state": {
                "Failed": {
                    "revision_hash": null,
                    "retryable": false,
                    "issue": null,
                    "error": "decode failed",
                    "failed_at": 0
                }
            }
        }))
        .unwrap();

        assert!(state.is_corrupted());
        assert!(!state.is_master_key_wrapper());
    }

    #[test]
    fn blob_sync_state_accepts_domain_json() {
        let state: PersistedCloudBlobSyncState = serde_json::from_value(serde_json::json!({
            "version": 1,
            "namespace_id": "ns-1",
            "record_key": {
                "kind": "Wallet",
                "wallet_id": "wallet-a",
                "record_id": "record-a"
            },
            "state": {
                "Dirty": {
                    "changed_at": 10
                }
            }
        }))
        .unwrap();

        assert_eq!(state.namespace_id, "ns-1");
        assert_eq!(state.wallet_id(), Some(&"wallet-a".into()));
        assert_eq!(state.record_id(), "record-a");
        assert_eq!(
            state.record_key(),
            &CloudBackupRecordKey::Wallet("wallet-a".into(), "record-a".into())
        );
    }

    #[test]
    fn blob_sync_state_rejects_unsupported_domain_version() {
        let error = serde_json::from_value::<PersistedCloudBlobSyncState>(serde_json::json!({
            "version": 2,
            "namespace_id": "ns-1",
            "record_key": {
                "kind": "Wallet",
                "wallet_id": "wallet-a",
                "record_id": "record-a"
            },
            "state": {
                "Dirty": {
                    "changed_at": 10
                }
            }
        }))
        .unwrap_err();

        assert!(
            error
                .to_string()
                .contains("unsupported persisted cloud backup blob sync record version: 2"),
            "{error}"
        );
    }

    #[test]
    fn blob_sync_state_rejects_legacy_wallet_without_wallet_id() {
        let error = serde_json::from_value::<PersistedCloudBlobSyncState>(serde_json::json!({
            "namespace_id": "ns-1",
            "wallet_id": null,
            "record_id": "record-a",
            "state": {
                "Dirty": {
                    "changed_at": 10
                }
            }
        }))
        .unwrap_err();

        assert!(
            error
                .to_string()
                .contains("invalid legacy blob: missing wallet_id for record_id record-a"),
            "{error}"
        );
    }

    #[test]
    fn failed_blob_state_defaults_retryable_to_false() {
        let failed_state: CloudBlobFailedState = serde_json::from_value(serde_json::json!({
            "revision_hash": "rev-1",
            "error": "offline",
            "failed_at": 42
        }))
        .unwrap();

        assert!(!failed_state.retryable);
        assert_eq!(failed_state.issue, None);
    }

    #[test]
    fn pending_verification_upload_accepts_legacy_plain_wallet() {
        let upload: PersistedPendingVerificationUpload =
            serde_json::from_value(serde_json::json!({
                "record_id": "wallet-1",
                "expected_revision": "rev-1"
            }))
            .unwrap();

        assert_eq!(
            upload,
            PersistedPendingVerificationUpload::Wallet {
                record_id: "wallet-1".into(),
                expected_revision: "rev-1".into()
            }
        );
    }

    #[test]
    fn pending_verification_upload_accepts_tagged_variants() {
        let master: PersistedPendingVerificationUpload =
            serde_json::from_value(serde_json::json!("MasterKeyWrapper")).unwrap();
        let wallet: PersistedPendingVerificationUpload =
            serde_json::from_value(serde_json::json!({
                "Wallet": {
                    "record_id": "wallet-1",
                    "expected_revision": "rev-1"
                }
            }))
            .unwrap();

        assert_eq!(master, PersistedPendingVerificationUpload::MasterKeyWrapper);
        assert_eq!(
            wallet,
            PersistedPendingVerificationUpload::Wallet {
                record_id: "wallet-1".into(),
                expected_revision: "rev-1".into()
            }
        );
    }
}
