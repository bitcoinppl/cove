use serde::{Deserialize, Serialize};

use super::MASTER_KEY_RECORD_ID;

pub const REMOTE_PAYLOAD_SCHEMA_VERSION: u32 = 1;

pub const fn backup_envelope_version_v1() -> u32 {
    1
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RemotePayloadKind {
    MasterKey,
    Wallet,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RemotePayloadMetadata {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<RemotePayloadKind>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub schema_version: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub namespace_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub record_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wallet_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub generation: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created_at: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content_hash: Option<String>,
}

impl RemotePayloadMetadata {
    pub fn master_key(namespace_id: &str, updated_at: u64) -> Self {
        Self {
            kind: Some(RemotePayloadKind::MasterKey),
            schema_version: Some(REMOTE_PAYLOAD_SCHEMA_VERSION),
            namespace_id: Some(namespace_id.to_string()),
            record_id: Some(MASTER_KEY_RECORD_ID.to_string()),
            updated_at: Some(updated_at),
            ..Self::default()
        }
    }

    pub fn wallet(namespace_id: &str, record_id: &str, wallet_id: &str, updated_at: u64) -> Self {
        Self {
            kind: Some(RemotePayloadKind::Wallet),
            schema_version: Some(REMOTE_PAYLOAD_SCHEMA_VERSION),
            namespace_id: Some(namespace_id.to_string()),
            record_id: Some(record_id.to_string()),
            wallet_id: Some(wallet_id.to_string()),
            updated_at: Some(updated_at),
            ..Self::default()
        }
    }

    pub fn normalized_master_key(
        &self,
        namespace_id: &str,
    ) -> Result<NormalizedRemotePayloadMetadata, RemotePayloadError> {
        self.validate(RemotePayloadKind::MasterKey, namespace_id, MASTER_KEY_RECORD_ID, None)
    }

    pub fn normalized_wallet(
        &self,
        namespace_id: &str,
        record_id: &str,
        wallet_id: Option<&str>,
    ) -> Result<NormalizedRemotePayloadMetadata, RemotePayloadError> {
        self.validate(RemotePayloadKind::Wallet, namespace_id, record_id, wallet_id)
    }

    fn validate(
        &self,
        kind: RemotePayloadKind,
        namespace_id: &str,
        record_id: &str,
        wallet_id: Option<&str>,
    ) -> Result<NormalizedRemotePayloadMetadata, RemotePayloadError> {
        validate_optional_kind(self.kind, kind)?;
        validate_optional_schema_version(self.schema_version)?;
        validate_optional_string("namespace_id", self.namespace_id.as_deref(), namespace_id)?;
        validate_optional_string("record_id", self.record_id.as_deref(), record_id)?;

        if let Some(wallet_id) = wallet_id {
            validate_optional_string("wallet_id", self.wallet_id.as_deref(), wallet_id)?;
        }

        Ok(NormalizedRemotePayloadMetadata {
            kind,
            schema_version: self.schema_version.unwrap_or(REMOTE_PAYLOAD_SCHEMA_VERSION),
            namespace_id: self.namespace_id.clone().unwrap_or_else(|| namespace_id.to_string()),
            record_id: self.record_id.clone().unwrap_or_else(|| record_id.to_string()),
            wallet_id: self.wallet_id.clone().or_else(|| wallet_id.map(ToString::to_string)),
            generation: self.generation,
            created_at: self.created_at,
            updated_at: self.updated_at,
            content_hash: self.content_hash.clone(),
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NormalizedRemotePayloadMetadata {
    pub kind: RemotePayloadKind,
    pub schema_version: u32,
    pub namespace_id: String,
    pub record_id: String,
    pub wallet_id: Option<String>,
    pub generation: Option<u64>,
    pub created_at: Option<u64>,
    pub updated_at: Option<u64>,
    pub content_hash: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum RemotePayloadError {
    #[error("remote payload {field} mismatch: expected {expected}, got {actual}")]
    Mismatch { field: &'static str, expected: String, actual: String },
    #[error("unsupported remote payload schema version {0}")]
    UnsupportedSchemaVersion(u32),
}

fn validate_optional_kind(
    actual: Option<RemotePayloadKind>,
    expected: RemotePayloadKind,
) -> Result<(), RemotePayloadError> {
    let Some(actual) = actual else {
        return Ok(());
    };

    if actual == expected {
        return Ok(());
    }

    Err(RemotePayloadError::Mismatch {
        field: "kind",
        expected: format!("{expected:?}"),
        actual: format!("{actual:?}"),
    })
}

fn validate_optional_schema_version(version: Option<u32>) -> Result<(), RemotePayloadError> {
    if version.is_none_or(|version| version == REMOTE_PAYLOAD_SCHEMA_VERSION) {
        return Ok(());
    }

    Err(RemotePayloadError::UnsupportedSchemaVersion(version.unwrap()))
}

fn validate_optional_string(
    field: &'static str,
    actual: Option<&str>,
    expected: &str,
) -> Result<(), RemotePayloadError> {
    if actual.is_none_or(|actual| actual == expected) {
        return Ok(());
    }

    Err(RemotePayloadError::Mismatch {
        field,
        expected: expected.to_string(),
        actual: actual.unwrap().to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn old_wallet_payload_metadata_normalizes_from_location() {
        let normalized = RemotePayloadMetadata::default()
            .normalized_wallet("namespace-a", "record-a", Some("wallet-a"))
            .unwrap();

        assert_eq!(normalized.kind, RemotePayloadKind::Wallet);
        assert_eq!(normalized.schema_version, REMOTE_PAYLOAD_SCHEMA_VERSION);
        assert_eq!(normalized.namespace_id, "namespace-a");
        assert_eq!(normalized.record_id, "record-a");
        assert_eq!(normalized.wallet_id.as_deref(), Some("wallet-a"));
    }

    #[test]
    fn wallet_payload_metadata_rejects_conflicting_record_id() {
        let metadata = RemotePayloadMetadata {
            record_id: Some("record-b".to_string()),
            ..RemotePayloadMetadata::default()
        };

        let error =
            metadata.normalized_wallet("namespace-a", "record-a", Some("wallet-a")).unwrap_err();

        assert!(matches!(error, RemotePayloadError::Mismatch { field: "record_id", .. }));
    }

    #[test]
    fn wallet_payload_metadata_rejects_conflicting_kind_without_option_wrapper() {
        let metadata = RemotePayloadMetadata {
            kind: Some(RemotePayloadKind::MasterKey),
            ..RemotePayloadMetadata::default()
        };

        let error =
            metadata.normalized_wallet("namespace-a", "record-a", Some("wallet-a")).unwrap_err();

        assert_eq!(
            error,
            RemotePayloadError::Mismatch {
                field: "kind",
                expected: "Wallet".to_string(),
                actual: "MasterKey".to_string(),
            },
        );
    }

    #[test]
    fn master_key_payload_metadata_uses_logical_record_id() {
        let normalized =
            RemotePayloadMetadata::default().normalized_master_key("namespace-a").unwrap();

        assert_eq!(normalized.kind, RemotePayloadKind::MasterKey);
        assert_eq!(normalized.record_id, MASTER_KEY_RECORD_ID);
    }
}
