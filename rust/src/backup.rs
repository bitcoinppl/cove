use parking_lot::Mutex;
use rand::RngExt as _;
use std::sync::Arc;
use zeroize::Zeroizing;

pub(crate) mod crypto;
mod error;
mod export;
pub(crate) mod import;
pub(crate) mod model;
pub(crate) mod recovery;
mod verify;

pub use error::BackupError;
pub use model::{BackupImportReport, BackupResult, BackupVerifyReport};

/// A decrypted and preflighted backup import that can be approved before it writes local state
#[derive(Debug, uniffi::Object)]
pub struct BackupImportPreparation {
    state: import::SharedImportPreparation,
}

impl BackupImportPreparation {
    fn new(state: import::ImportPreparationState) -> Arc<Self> {
        Arc::new(Self { state: Arc::new(Mutex::new(Some(state))) })
    }
}

#[uniffi::export]
impl BackupImportPreparation {
    /// Return the digest that binds an approval to this backup payload
    pub fn payload_digest(&self) -> String {
        self.state.lock().as_ref().map(|state| state.payload_digest.clone()).unwrap_or_default()
    }

    /// Return wallet ids found during preflight
    pub fn wallet_ids(&self) -> Vec<cove_types::WalletId> {
        self.state
            .lock()
            .as_ref()
            .map(|state| state.wallets.iter().map(|wallet| wallet.metadata.id.clone()).collect())
            .unwrap_or_default()
    }

    /// Return wallet ids whose existing artifacts need explicit cleanup approval
    pub fn markerless_conflict_wallet_ids(&self) -> Vec<cove_types::WalletId> {
        self.state
            .lock()
            .as_ref()
            .map(|state| {
                state
                    .wallets
                    .iter()
                    .filter(|wallet| wallet.snapshot.has_markerless_conflict())
                    .map(|wallet| wallet.metadata.id.clone())
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Return whether this import needs explicit cleanup approval
    pub fn requires_import_approval(&self) -> bool {
        !self.markerless_conflict_wallet_ids().is_empty()
    }
}

/// A one-use approval for the exact preflighted backup and artifact snapshots
#[derive(Debug, uniffi::Object)]
pub struct BackupImportApproval {
    state: import::SharedImportApproval,
}

#[derive(Debug, Clone, uniffi::Object)]
pub struct BackupManager;

#[uniffi::export]
impl BackupManager {
    #[uniffi::constructor]
    pub fn new() -> Self {
        Self
    }

    pub async fn export(&self, password: String) -> Result<BackupResult, BackupError> {
        export::export_all(password).await.map_err(|error| error.logged("export"))
    }

    /// Decrypt and preflight a backup without changing local wallet state
    #[uniffi::method(name = "prepareImport")]
    pub async fn prepare_import(
        &self,
        data: Vec<u8>,
        password: String,
    ) -> Result<Arc<BackupImportPreparation>, BackupError> {
        let state = import::prepare_import(data, password)
            .await
            .map_err(|error| error.logged("prepare import"))?;

        Ok(BackupImportPreparation::new(state))
    }

    /// Approve cleanup of markerless artifacts after rechecking the preflight snapshots
    #[uniffi::method(name = "approveImport")]
    pub async fn approve_import(
        &self,
        preparation: Arc<BackupImportPreparation>,
    ) -> Result<Arc<BackupImportApproval>, BackupError> {
        async move {
            let (payload_digest, requested_snapshots) = {
                let guard = preparation.state.lock();
                let state = guard.as_ref().ok_or(BackupError::ImportApprovalUsed)?;
                let requested_snapshots = state
                    .wallets
                    .iter()
                    .filter(|wallet| wallet.snapshot.has_markerless_conflict())
                    .map(|wallet| (wallet.metadata.id.clone(), wallet.snapshot.clone()))
                    .collect::<Vec<_>>();

                (state.payload_digest.clone(), requested_snapshots)
            };

            let snapshots = tokio::task::spawn_blocking(move || {
                let mut snapshots = std::collections::HashMap::new();
                for (wallet_id, expected) in requested_snapshots {
                    let validated = recovery::ValidatedRestoreWalletId::validate(&wallet_id)?;
                    let current = recovery::RestoreArtifactSnapshot::capture(&validated)?;
                    if current != expected {
                        return Err(BackupError::ImportApprovalStale(wallet_id));
                    }

                    snapshots.insert(wallet_id.as_str().to_string(), expected);
                }

                Ok::<_, BackupError>(snapshots)
            })
            .await
            .map_err(|error| {
                BackupError::Restore(format!("approval snapshot task failed: {error}"))
            })??;

            Ok(Arc::new(BackupImportApproval {
                state: Arc::new(Mutex::new(Some(import::ImportApprovalState {
                    payload_digest,
                    snapshots,
                }))),
            }))
        }
        .await
        .map_err(|error: BackupError| error.logged("approve import"))
    }

    /// Consume a preparation and optional one-use approval to import the backup
    #[uniffi::method(name = "importPrepared")]
    pub async fn import_prepared(
        &self,
        preparation: Arc<BackupImportPreparation>,
        approval: Option<Arc<BackupImportApproval>>,
    ) -> Result<BackupImportReport, BackupError> {
        async move {
            let (state, approval) = match approval {
                Some(approval) => {
                    let mut preparation_guard = preparation.state.lock();
                    let mut approval_guard = approval.state.lock();
                    let preparation_state =
                        preparation_guard.as_ref().ok_or(BackupError::ImportApprovalUsed)?;

                    let approval_state =
                        approval_guard.as_ref().ok_or(BackupError::ImportApprovalUsed)?;

                    import::validate_prepared_import(preparation_state, Some(approval_state))?;

                    let state = preparation_guard
                        .take()
                        .expect("preparation was checked as present while holding its mutex");

                    let approval = approval_guard
                        .take()
                        .expect("approval was checked as present while holding its mutex");
                    (state, Some(approval))
                }

                None => {
                    let mut preparation_guard = preparation.state.lock();
                    let preparation_state =
                        preparation_guard.as_ref().ok_or(BackupError::ImportApprovalUsed)?;

                    import::validate_prepared_import(preparation_state, None)?;

                    let state = preparation_guard
                        .take()
                        .expect("preparation was checked as present while holding its mutex");
                    (state, None)
                }
            };

            import::import_prepared(state, approval).await
        }
        .await
        .map_err(|error: BackupError| error.logged("import"))
    }

    #[uniffi::method(name = "verifyBackup")]
    pub async fn verify(
        &self,
        data: Vec<u8>,
        password: String,
    ) -> Result<BackupVerifyReport, BackupError> {
        verify::verify_backup(data, password).await.map_err(|error| error.logged("verify"))
    }

    /// Validate the file format without decrypting
    pub fn validate_format(&self, data: Vec<u8>) -> Result<(), BackupError> {
        crypto::validate_header(&data).map_err(|error| error.logged("validate format"))
    }

    /// Generate a 12-word BIP39 mnemonic to use as the backup password
    ///
    /// NOTE: the returned String is not zeroized — bip39::Mnemonic::to_string()
    /// allocates a plain String, and the value crosses FFI to Swift/Kotlin where
    /// we can't control deallocation. The entropy source is zeroized, limiting
    /// the exposure window
    pub fn generate_password(&self) -> String {
        let entropy = Zeroizing::new(rand::rng().random::<[u8; 16]>());
        let mnemonic = bip39::Mnemonic::from_entropy(&*entropy)
            .expect("16 bytes is valid entropy for 12 words");
        mnemonic.to_string()
    }

    /// Check whether a password meets backup requirements
    pub fn is_password_valid(&self, password: &str) -> bool {
        crypto::clean_password(password).is_ok()
    }

    /// Account name for saving backup passwords to the system credential store
    pub fn backup_account_name(&self) -> String {
        let now = jiff::Zoned::now();
        format!("Cove Backup - {}", now.strftime("%Y-%m-%d"))
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;

    #[test]
    fn valid_password() {
        let mgr = BackupManager::new();
        assert!(mgr.is_password_valid("abandonabilityableaboutaboveabsentabsorb"));
    }

    #[test]
    fn valid_password_with_spaces() {
        let mgr = BackupManager::new();
        assert!(mgr.is_password_valid("abandon ability able about above absent absorb"));
    }

    #[test]
    fn valid_password_with_tabs_and_newlines() {
        let mgr = BackupManager::new();
        assert!(mgr.is_password_valid("abandon\tability\nable\tabout\tabove\tabsent\tabsorb"));
    }

    #[test]
    fn too_short_password() {
        let mgr = BackupManager::new();
        assert!(!mgr.is_password_valid("tooshort"));
    }

    #[test]
    fn empty_password() {
        let mgr = BackupManager::new();
        assert!(!mgr.is_password_valid(""));
    }

    #[test]
    fn all_whitespace_password() {
        let mgr = BackupManager::new();
        assert!(!mgr.is_password_valid("                                    "));
    }

    #[test]
    fn exactly_32_chars_after_stripping() {
        let mgr = BackupManager::new();
        let password = "a".repeat(32);
        assert!(mgr.is_password_valid(&password));
    }

    #[test]
    fn short_after_stripping_whitespace() {
        let mgr = BackupManager::new();
        assert!(!mgr.is_password_valid("abc def ghi jkl mno"));
    }

    fn empty_payload() -> super::model::BackupPayload {
        super::model::BackupPayload {
            version: super::model::PAYLOAD_VERSION,
            created_at: 0,
            wallets: Vec::new(),
            settings: super::model::AppSettings {
                selected_network: None,
                selected_fiat_currency: None,
                color_scheme: None,
                selected_nodes: Vec::new(),
                custom_block_explorers: BTreeMap::new(),
            },
        }
    }

    fn empty_preparation(digest: &str) -> Arc<BackupImportPreparation> {
        BackupImportPreparation::new(import::ImportPreparationState {
            payload: Some(empty_payload()),
            payload_digest: digest.to_string(),
            wallets: Vec::new(),
        })
    }

    fn empty_approval(digest: &str) -> Arc<BackupImportApproval> {
        Arc::new(BackupImportApproval {
            state: Arc::new(Mutex::new(Some(import::ImportApprovalState {
                payload_digest: digest.to_string(),
                snapshots: std::collections::HashMap::new(),
            }))),
        })
    }

    fn init_import_test_state() {
        crate::database::test_support::delete_database();
        crate::app::reconcile::test_support::init_noop_updater();
        crate::test_support::init_test_keychain();
        crate::test_support::shared_mock_keychain().reset();
    }

    #[tokio::test(flavor = "current_thread")]
    async fn prepared_import_and_approval_are_one_use() {
        let _guard = crate::test_support::global_state_test_lock().lock().await;
        init_import_test_state();

        let manager = BackupManager::new();
        let preparation = empty_preparation("test-digest");
        let approval = empty_approval("test-digest");

        manager.import_prepared(preparation.clone(), Some(approval.clone())).await.unwrap();

        assert!(matches!(
            manager.import_prepared(preparation, None).await,
            Err(BackupError::ImportApprovalUsed)
        ));
        assert!(matches!(
            manager.import_prepared(empty_preparation("test-digest"), Some(approval)).await,
            Err(BackupError::ImportApprovalUsed)
        ));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn invalid_optional_approval_does_not_consume_preparation() {
        let _guard = crate::test_support::global_state_test_lock().lock().await;
        init_import_test_state();

        let manager = BackupManager::new();
        let preparation = empty_preparation("expected-digest");
        let stale_approval = empty_approval("stale-digest");

        assert!(matches!(
            manager.import_prepared(preparation.clone(), Some(stale_approval)).await,
            Err(BackupError::ImportApprovalStale(_))
        ));
        assert!(manager.import_prepared(preparation, None).await.is_ok());
    }

    // decrypted backup values that fail to parse must not reach logs, only serde's position
    #[tokio::test(flavor = "current_thread")]
    async fn malformed_decrypted_backup_values_stay_out_of_logs() {
        use crate::test_support::log_capture::LogCapture;

        let _guard = crate::test_support::global_state_test_lock().lock().await;
        init_import_test_state();

        let manager = BackupManager::new();
        let password = manager.generate_password();

        // verify and import strip whitespace from the password before deriving the key
        let encryption_password = crypto::clean_password(&password).unwrap();
        let secret_network = "private-user-text-network";
        let secret_created_at = "private-user-text-created-at";

        let encrypt = |payload: &serde_json::Value| {
            let json = serde_json::to_vec(payload).unwrap();
            crypto::encrypt(&crypto::compress(&json), &encryption_password).unwrap()
        };

        let metadata = crate::test_support::hot_wallet_metadata("Restored");
        let wallet = super::model::WalletBackup {
            metadata: serde_json::to_value(metadata).unwrap(),
            secret: super::model::WalletSecret::None,
            descriptors: None,
            xpub: None,
            labels_jsonl: None,
        };

        let mut payload = empty_payload();
        payload.wallets.push(wallet);
        let mut payload = serde_json::to_value(payload).unwrap();
        payload["wallets"][0]["metadata"]["network"] = serde_json::Value::from(secret_network);
        let bad_wallet_backup = encrypt(&payload);

        let mut payload = serde_json::to_value(empty_payload()).unwrap();
        payload["created_at"] = serde_json::Value::from(secret_created_at);
        let bad_payload_backup = encrypt(&payload);

        let capture = LogCapture::start();
        let errors = [
            manager.verify(bad_wallet_backup.clone(), password.clone()).await.unwrap_err(),
            manager.prepare_import(bad_wallet_backup, password.clone()).await.unwrap_err(),
            manager.verify(bad_payload_backup.clone(), password.clone()).await.unwrap_err(),
            manager.prepare_import(bad_payload_backup, password).await.unwrap_err(),
        ];
        let logs = capture.text();

        // failure messages name what was missing or leaked, never the captured text, since it
        // would carry the very values this test keeps out of logs
        for error in &errors {
            let kind = error.kind();
            assert!(matches!(error, BackupError::Deserialization(_)), "unexpected {kind} error");
        }

        assert!(logs.contains("backup verify failed"), "verify failure was not logged");
        assert!(logs.contains("backup prepare import failed"), "import failure was not logged");
        assert!(logs.contains("wallet metadata: data error"), "wallet error position missing");
        assert!(
            logs.contains("payload: data error at line 1 column"),
            "payload error position missing"
        );

        for secret in [secret_network, secret_created_at] {
            assert!(!logs.contains(secret), "logs quote a decrypted backup value");
            for error in &errors {
                let kind = error.kind();
                assert!(
                    !error.to_string().contains(secret),
                    "{kind} error quotes a decrypted backup value"
                );
            }
        }
    }

    // restore and keychain errors can name the wallet or quote key material, so only the kind
    // is logged
    #[test]
    fn free_text_backup_errors_log_only_their_kind() {
        use crate::test_support::log_capture::LogCapture;

        let wallet_name = "private wallet name";
        let capture = LogCapture::start();
        BackupError::Restore(format!("invalid xpub for {wallet_name}")).logged("import");
        BackupError::Keychain(format!("xpub for {wallet_name}: xpub6Cbad")).logged("import");
        let logs = capture.text();

        assert!(logs.contains("backup import failed kind=\"restore\""), "restore kind not logged");
        assert!(
            logs.contains("backup import failed kind=\"keychain\""),
            "keychain kind not logged"
        );

        assert!(!logs.contains(wallet_name), "logs quote the wallet name");
        assert!(!logs.contains("xpub6Cbad"), "logs quote the xpub");
    }
}
