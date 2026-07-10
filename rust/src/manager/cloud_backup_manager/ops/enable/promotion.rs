use cove_cspp::{MasterKeyPromotionStatus, master_key::MasterKey};
use cove_device::keychain::Keychain;
use tracing::warn;

use super::RustCloudBackupManager;
use super::types::CloudBackupEnableRecoveryPreparation;
use crate::manager::cloud_backup_manager::wallets::{StagedPrfKey, UnpersistedPrfKey};
use crate::manager::cloud_backup_manager::{
    CloudBackupEnableContext, CloudBackupError, CloudBackupKeychain, PendingEnableJournal,
    PendingEnableJournalPhase, PendingEnableNamespaceOwnership, PendingEnablePasskeyMetadata,
};

impl RustCloudBackupManager {
    pub(crate) fn save_enable_recovery_master_key(
        &self,
        preparation: &CloudBackupEnableRecoveryPreparation,
    ) -> Result<(), CloudBackupError> {
        let keychain = Keychain::global().clone();
        let cloud_keychain = CloudBackupKeychain::new(keychain.clone());
        let cspp = cove_cspp::Cspp::new(keychain);
        let recovered_passkey = preparation.recovered_passkey_metadata();

        if let Some(mut journal) = cloud_keychain
            .load_pending_enable_journal()
            .map_err(|source| CloudBackupError::internal_context("load pending enable", source))?
        {
            let staged = cspp.load_staged_master_key().map_err(|source| {
                CloudBackupError::internal_context("load staged recovered master key", source)
            })?;
            let is_matching_recovery = journal.namespace_ownership()
                == PendingEnableNamespaceOwnership::RecoveredExisting
                && journal.namespace_id() == preparation.active_namespace_id
                && journal.context() == preparation.context
                && staged.as_ref().is_some_and(|master_key| {
                    master_key.as_bytes() == preparation.active_master_key.as_bytes()
                });
            if !is_matching_recovery
                || matches!(journal.phase(), PendingEnableJournalPhase::LocalPromotionStarted(_))
                || !journal.register_passkey(recovered_passkey)
            {
                return Err(CloudBackupError::Internal(
                    "a different pending Cloud Backup enable must be recovered first".into(),
                ));
            }

            return cloud_keychain.save_pending_enable_journal(&journal).map_err(|source| {
                CloudBackupError::internal_context("save recovered pending enable passkey", source)
            });
        }

        match cspp.master_key_promotion_status().map_err(|source| {
            CloudBackupError::internal_context("inspect recovered master key stage", source)
        })? {
            MasterKeyPromotionStatus::None => {}
            MasterKeyPromotionStatus::Staged => {
                cspp.discard_staged_master_key().map_err(|source| {
                    CloudBackupError::internal_context(
                        "discard unowned staged recovered master key",
                        source,
                    )
                })?;
            }
            MasterKeyPromotionStatus::Pending(_) => {
                return Err(CloudBackupError::Internal(
                    "a prior Cloud Backup master key promotion must be recovered first".into(),
                ));
            }
        }

        cspp.save_staged_master_key(&preparation.active_master_key).map_err(|source| {
            CloudBackupError::internal_context("stage recovered master key", source)
        })?;
        let mut journal = PendingEnableJournal::staged(
            preparation.context,
            preparation.active_namespace_id.clone(),
            PendingEnableNamespaceOwnership::RecoveredExisting,
            cloud_keychain.snapshot_passkey_metadata(),
        );
        if !journal.register_passkey(recovered_passkey) {
            let _ = cspp.discard_staged_master_key();

            return Err(CloudBackupError::Internal(
                "could not record recovered Cloud Backup passkey".into(),
            ));
        }
        if let Err(source) = cloud_keychain.save_pending_enable_journal(&journal) {
            let _ = cspp.discard_staged_master_key();

            return Err(CloudBackupError::internal_context(
                "save recovered pending enable",
                source,
            ));
        }

        Ok(())
    }

    pub(crate) fn rollback_enable_recovery_master_key(&self) -> Result<(), CloudBackupError> {
        warn!("Enable: rolling back recovered local master key after recovery failure");
        let cloud_keychain = CloudBackupKeychain::global();
        let journal = cloud_keychain
            .load_pending_enable_journal()
            .map_err(|source| CloudBackupError::internal_context("load pending enable", source))?
            .ok_or_else(|| {
                CloudBackupError::Internal("pending recovered Cloud Backup state is missing".into())
            })?;
        if journal.namespace_ownership() != PendingEnableNamespaceOwnership::RecoveredExisting {
            return Err(CloudBackupError::Internal(
                "refusing to roll back a non-recovery pending enable as recovered state".into(),
            ));
        }

        cove_cspp::Cspp::new(Keychain::global().clone()).rollback_master_key_promotion().map_err(
            |source| CloudBackupError::internal_context("roll back recovered master key", source),
        )?;
        cloud_keychain.restore_passkey_metadata(journal.previous_metadata()).map_err(|source| {
            CloudBackupError::internal_context(
                "restore prior Cloud Backup passkey metadata",
                source,
            )
        })?;
        cloud_keychain.delete_pending_enable_journal().map_err(|source| {
            CloudBackupError::internal_context("clear rolled back recovered enable state", source)
        })
    }

    pub(crate) fn mark_enable_recovery_remote_writes_started(
        &self,
        preparation: &CloudBackupEnableRecoveryPreparation,
    ) -> Result<(), CloudBackupError> {
        let cloud_keychain = CloudBackupKeychain::global();
        let expected_passkey = preparation.recovered_passkey_metadata();
        let mut journal = cloud_keychain
            .load_pending_enable_journal()
            .map_err(|source| CloudBackupError::internal_context("load pending enable", source))?
            .ok_or_else(|| {
                CloudBackupError::Internal("pending recovered Cloud Backup state is missing".into())
            })?;
        if journal.namespace_ownership() != PendingEnableNamespaceOwnership::RecoveredExisting
            || journal.namespace_id() != preparation.active_namespace_id
            || journal.passkey() != Some(&expected_passkey)
            || !journal.mark_remote_writes_started()
        {
            return Err(CloudBackupError::Internal(
                "pending recovered Cloud Backup state cannot start remote writes".into(),
            ));
        }

        cloud_keychain.save_pending_enable_journal(&journal).map_err(|source| {
            CloudBackupError::internal_context(
                "mark recovered pending enable remote writes",
                source,
            )
        })
    }

    pub(crate) fn stage_fresh_enable_master(
        &self,
        context: CloudBackupEnableContext,
    ) -> Result<(MasterKey, CloudBackupEnableContext), CloudBackupError> {
        let keychain = Keychain::global().clone();
        let cloud_keychain = CloudBackupKeychain::new(keychain.clone());
        let cspp = cove_cspp::Cspp::new(keychain);

        if let Some(journal) = cloud_keychain
            .load_pending_enable_journal()
            .map_err(|source| CloudBackupError::internal_context("load pending enable", source))?
        {
            if journal.namespace_ownership() != PendingEnableNamespaceOwnership::FreshOwned {
                return Err(CloudBackupError::Internal(
                    "an existing-backup recovery must be resumed before starting a new backup"
                        .into(),
                ));
            }
            if !matches!(journal.phase(), PendingEnableJournalPhase::Staged) {
                return Err(CloudBackupError::Internal(
                    "pending Cloud Backup enable must be resumed before starting another backup"
                        .into(),
                ));
            }
            let staged = cspp.load_staged_master_key().map_err(|source| {
                CloudBackupError::internal_context("load staged master key", source)
            })?;
            let Some(staged) = staged else {
                return Err(CloudBackupError::Internal(
                    "pending Cloud Backup enable is missing its staged master key".into(),
                ));
            };
            if staged.namespace_id() != journal.namespace_id() {
                return Err(CloudBackupError::Internal(
                    "pending Cloud Backup enable has mismatched staged ownership".into(),
                ));
            }

            return Ok((staged, journal.context()));
        }

        match cspp.master_key_promotion_status().map_err(|source| {
            CloudBackupError::internal_context("inspect staged master key", source)
        })? {
            MasterKeyPromotionStatus::None => {}
            MasterKeyPromotionStatus::Staged => {
                cspp.discard_staged_master_key().map_err(|source| {
                    CloudBackupError::internal_context("discard unowned staged master key", source)
                })?;
            }
            MasterKeyPromotionStatus::Pending(_) => {
                return Err(CloudBackupError::Internal(
                    "a prior Cloud Backup master key promotion must be recovered first".into(),
                ));
            }
        }

        let master_key = MasterKey::generate();
        cspp.save_staged_master_key(&master_key).map_err(|source| {
            CloudBackupError::internal_context("stage fresh master key", source)
        })?;
        let journal = PendingEnableJournal::staged(
            context,
            master_key.namespace_id(),
            PendingEnableNamespaceOwnership::FreshOwned,
            cloud_keychain.snapshot_passkey_metadata(),
        );
        if let Err(source) = cloud_keychain.save_pending_enable_journal(&journal) {
            let _ = cspp.discard_staged_master_key();
            return Err(CloudBackupError::internal_context("save pending enable", source));
        }

        Ok((master_key, context))
    }

    pub(crate) fn record_pending_enable_staged_passkey(
        &self,
        master_key: &MasterKey,
        passkey: &StagedPrfKey,
    ) -> Result<(), CloudBackupError> {
        self.record_pending_enable_passkey_metadata(
            master_key,
            PendingEnablePasskeyMetadata {
                credential_id: passkey.credential_id.clone(),
                prf_salt: passkey.prf_salt,
                provider_hint: passkey.provider_hint.clone(),
            },
        )
    }

    pub(crate) fn record_pending_enable_passkey(
        &self,
        master_key: &MasterKey,
        passkey: &UnpersistedPrfKey,
    ) -> Result<(), CloudBackupError> {
        self.record_pending_enable_passkey_metadata(
            master_key,
            PendingEnablePasskeyMetadata {
                credential_id: passkey.credential_id.clone(),
                prf_salt: passkey.prf_salt,
                provider_hint: passkey.provider_hint.clone(),
            },
        )
    }

    fn record_pending_enable_passkey_metadata(
        &self,
        master_key: &MasterKey,
        passkey: PendingEnablePasskeyMetadata,
    ) -> Result<(), CloudBackupError> {
        let cloud_keychain = CloudBackupKeychain::global();
        let mut journal = cloud_keychain
            .load_pending_enable_journal()
            .map_err(|source| CloudBackupError::internal_context("load pending enable", source))?
            .ok_or_else(|| {
                CloudBackupError::Internal("pending Cloud Backup enable state is missing".into())
            })?;
        if journal.namespace_id() != master_key.namespace_id() {
            return Err(CloudBackupError::Internal(
                "pending Cloud Backup enable has mismatched passkey ownership".into(),
            ));
        }
        if !journal.register_passkey(passkey) {
            return Err(CloudBackupError::Internal(
                "pending Cloud Backup enable passkey changed unexpectedly".into(),
            ));
        }

        cloud_keychain.save_pending_enable_journal(&journal).map_err(|source| {
            CloudBackupError::internal_context("save pending enable passkey", source)
        })
    }

    pub(crate) fn mark_pending_enable_remote_writes_started(
        &self,
        master_key: &MasterKey,
        passkey: &UnpersistedPrfKey,
    ) -> Result<(), CloudBackupError> {
        self.record_pending_enable_passkey(master_key, passkey)?;

        let cloud_keychain = CloudBackupKeychain::global();
        let mut journal = cloud_keychain
            .load_pending_enable_journal()
            .map_err(|source| CloudBackupError::internal_context("load pending enable", source))?
            .ok_or_else(|| {
                CloudBackupError::Internal("pending Cloud Backup enable state is missing".into())
            })?;
        if !journal.mark_remote_writes_started() {
            return Err(CloudBackupError::Internal(
                "pending Cloud Backup enable cannot start remote writes before passkey setup"
                    .into(),
            ));
        }

        cloud_keychain.save_pending_enable_journal(&journal).map_err(|source| {
            CloudBackupError::internal_context("mark pending enable remote writes", source)
        })
    }

    pub(crate) fn begin_pending_enable_local_promotion(
        &self,
        master_key: &MasterKey,
        passkey: &UnpersistedPrfKey,
    ) -> Result<(), CloudBackupError> {
        self.begin_pending_enable_local_promotion_with_metadata(
            &master_key.namespace_id(),
            PendingEnableNamespaceOwnership::FreshOwned,
            PendingEnablePasskeyMetadata {
                credential_id: passkey.credential_id.clone(),
                prf_salt: passkey.prf_salt,
                provider_hint: passkey.provider_hint.clone(),
            },
        )
    }

    pub(crate) fn begin_enable_recovery_local_promotion(
        &self,
        namespace_id: &str,
        credential_id: &[u8],
        prf_salt: [u8; 32],
    ) -> Result<(), CloudBackupError> {
        self.begin_pending_enable_local_promotion_with_metadata(
            namespace_id,
            PendingEnableNamespaceOwnership::RecoveredExisting,
            PendingEnablePasskeyMetadata {
                credential_id: credential_id.to_vec(),
                prf_salt,
                provider_hint: None,
            },
        )
    }

    fn begin_pending_enable_local_promotion_with_metadata(
        &self,
        namespace_id: &str,
        namespace_ownership: PendingEnableNamespaceOwnership,
        expected_passkey: PendingEnablePasskeyMetadata,
    ) -> Result<(), CloudBackupError> {
        let cspp = cove_cspp::Cspp::new(Keychain::global().clone());
        let staged = cspp.load_staged_master_key().map_err(|source| {
            CloudBackupError::internal_context("load staged master key for promotion", source)
        })?;
        if staged.as_ref().is_none_or(|master_key| master_key.namespace_id() != namespace_id) {
            return Err(CloudBackupError::Internal(
                "pending Cloud Backup enable has mismatched staged master key ownership".into(),
            ));
        }

        let cloud_keychain = CloudBackupKeychain::global();
        let mut journal = cloud_keychain
            .load_pending_enable_journal()
            .map_err(|source| CloudBackupError::internal_context("load pending enable", source))?
            .ok_or_else(|| {
                CloudBackupError::Internal("pending Cloud Backup enable state is missing".into())
            })?;
        if journal.namespace_ownership() != namespace_ownership
            || journal.namespace_id() != namespace_id
            || journal.passkey() != Some(&expected_passkey)
            || !journal.mark_local_promotion_started()
        {
            return Err(CloudBackupError::Internal(
                "pending Cloud Backup enable cannot promote unowned staged material".into(),
            ));
        }
        cloud_keychain.save_pending_enable_journal(&journal).map_err(|source| {
            CloudBackupError::internal_context("mark pending enable promotion", source)
        })?;

        if let Err(source) = cspp.promote_staged_master_key() {
            return self
                .restore_pending_enable_local_promotion_for_retry()
                .and(Err(CloudBackupError::internal_context("promote staged master key", source)));
        }
        if let Err(source) = cloud_keychain.save_passkey_and_namespace(
            &expected_passkey.credential_id,
            expected_passkey.prf_salt,
            journal.namespace_id(),
        ) {
            return self.restore_pending_enable_local_promotion_for_retry().and(Err(
                CloudBackupError::internal_context("promote Cloud Backup passkey metadata", source),
            ));
        }

        Ok(())
    }

    pub(crate) fn restore_pending_enable_local_promotion_for_retry(
        &self,
    ) -> Result<(), CloudBackupError> {
        let cloud_keychain = CloudBackupKeychain::global();
        let mut journal = cloud_keychain
            .load_pending_enable_journal()
            .map_err(|source| CloudBackupError::internal_context("load pending enable", source))?
            .ok_or_else(|| {
                CloudBackupError::Internal("pending Cloud Backup enable state is missing".into())
            })?;
        if !journal.roll_back_local_promotion() {
            return Err(CloudBackupError::Internal(
                "pending Cloud Backup enable has no local promotion to restore".into(),
            ));
        }

        let cspp = cove_cspp::Cspp::new(Keychain::global().clone());
        cspp.restore_prior_master_key_for_retry().map_err(|source| {
            CloudBackupError::internal_context("restore prior master key for retry", source)
        })?;
        cloud_keychain.restore_passkey_metadata(journal.previous_metadata()).map_err(|source| {
            CloudBackupError::internal_context(
                "restore prior Cloud Backup passkey metadata",
                source,
            )
        })?;
        cloud_keychain.save_pending_enable_journal(&journal).map_err(|source| {
            CloudBackupError::internal_context("restore pending enable retry state", source)
        })
    }

    pub(crate) fn commit_pending_enable_local_promotion(&self) -> Result<(), CloudBackupError> {
        let cloud_keychain = CloudBackupKeychain::global();
        let journal = cloud_keychain
            .load_pending_enable_journal()
            .map_err(|source| CloudBackupError::internal_context("load pending enable", source))?
            .ok_or_else(|| {
                CloudBackupError::Internal("pending Cloud Backup enable state is missing".into())
            })?;
        if !matches!(journal.phase(), PendingEnableJournalPhase::LocalPromotionStarted(_)) {
            return Err(CloudBackupError::Internal(
                "pending Cloud Backup enable local promotion is not ready to commit".into(),
            ));
        }

        cove_cspp::Cspp::new(Keychain::global().clone()).commit_master_key_promotion().map_err(
            |source| {
                CloudBackupError::internal_context("commit staged master key promotion", source)
            },
        )?;
        cloud_keychain.delete_pending_enable_journal().map_err(|source| {
            CloudBackupError::internal_context("clear committed pending enable state", source)
        })
    }

    pub(crate) fn discard_unpromoted_enable_stage(
        &self,
        context: &str,
    ) -> Result<(), CloudBackupError> {
        warn!("{context}: discarding isolated staged master key");
        let cspp = cove_cspp::Cspp::new(Keychain::global().clone());
        cspp.discard_staged_master_key().map_err(|source| {
            CloudBackupError::internal_context("discard staged master key", source)
        })?;
        CloudBackupKeychain::global().delete_pending_enable_journal().map_err(|source| {
            CloudBackupError::internal_context("clear pending enable state", source)
        })
    }
}
