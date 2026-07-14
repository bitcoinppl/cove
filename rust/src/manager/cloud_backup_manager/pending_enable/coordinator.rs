use cove_cspp::{MasterKeyPromotionActiveState, MasterKeyPromotionStatus, master_key::MasterKey};
use cove_device::keychain::Keychain;
use tracing::warn;

use crate::manager::cloud_backup_manager::wallets::{StagedPrfKey, UnpersistedPrfKey};
use crate::manager::cloud_backup_manager::{
    CloudBackupEnableContext, CloudBackupError, CloudBackupKeychain, PendingEnableJournal,
    PendingEnableJournalPhase, PendingEnableNamespaceOwnership, PendingEnablePasskeyMetadata,
};

#[derive(Clone, Debug)]
pub(crate) struct PendingEnableCoordinator(Keychain);

impl PendingEnableCoordinator {
    pub(crate) fn new(keychain: Keychain) -> Self {
        Self(keychain)
    }

    fn cspp(&self) -> cove_cspp::Cspp<Keychain> {
        cove_cspp::Cspp::new(self.0.clone())
    }

    fn cloud_keychain(&self) -> CloudBackupKeychain {
        CloudBackupKeychain::new(self.0.clone())
    }

    pub(crate) fn save_enable_recovery_master_key(
        &self,
        context: CloudBackupEnableContext,
        namespace_id: &str,
        master_key: &MasterKey,
        recovered_passkey: PendingEnablePasskeyMetadata,
    ) -> Result<(), CloudBackupError> {
        let cloud_keychain = self.cloud_keychain();
        let cspp = self.cspp();

        if let Some(mut journal) = cloud_keychain
            .load_pending_enable_journal()
            .map_err(|source| CloudBackupError::internal_context("load pending enable", source))?
        {
            let staged = cspp.load_staged_master_key().map_err(|source| {
                CloudBackupError::internal_context("load staged recovered master key", source)
            })?;
            let is_matching_recovery = journal.namespace_ownership()
                == PendingEnableNamespaceOwnership::RecoveredExisting
                && journal.namespace_id() == namespace_id
                && journal.context() == context
                && staged.as_ref().is_some_and(|staged| staged.as_bytes() == master_key.as_bytes());
            if !is_matching_recovery
                || !self.has_promotion_status(&[MasterKeyPromotionStatus::Staged])?
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
                return Err(CloudBackupError::Internal(
                    "unowned staged master key must be recovered before enabling Cloud Backup"
                        .into(),
                ));
            }
            MasterKeyPromotionStatus::Pending(_) => {
                return Err(CloudBackupError::Internal(
                    "a prior Cloud Backup master key promotion must be recovered first".into(),
                ));
            }
        }

        cspp.save_staged_master_key(master_key).map_err(|source| {
            CloudBackupError::internal_context("stage recovered master key", source)
        })?;
        let mut journal = PendingEnableJournal::staged(
            context,
            namespace_id.to_owned(),
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
        let cloud_keychain = self.cloud_keychain();
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
        if !matches!(
            journal.phase(),
            PendingEnableJournalPhase::PasskeyRegistered(_)
                | PendingEnableJournalPhase::RemoteWritesStarted(_)
        ) || !self.has_staged_namespace(journal.namespace_id())?
            || !matches!(
                self.promotion_status()?,
                MasterKeyPromotionStatus::Staged
                    | MasterKeyPromotionStatus::Pending(MasterKeyPromotionActiveState::Prior)
            )
        {
            return Err(CloudBackupError::Internal(
                "refusing to roll back mismatched recovered Cloud Backup evidence".into(),
            ));
        }

        self.cspp().rollback_master_key_promotion().map_err(|source| {
            CloudBackupError::internal_context("roll back recovered master key", source)
        })?;
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
        namespace_id: &str,
        expected_passkey: PendingEnablePasskeyMetadata,
    ) -> Result<(), CloudBackupError> {
        let cloud_keychain = self.cloud_keychain();
        let mut journal = cloud_keychain
            .load_pending_enable_journal()
            .map_err(|source| CloudBackupError::internal_context("load pending enable", source))?
            .ok_or_else(|| {
                CloudBackupError::Internal("pending recovered Cloud Backup state is missing".into())
            })?;
        if journal.namespace_ownership() != PendingEnableNamespaceOwnership::RecoveredExisting
            || journal.namespace_id() != namespace_id
            || journal.passkey() != Some(&expected_passkey)
            || !self.has_staged_namespace(namespace_id)?
            || !self.has_promotion_status(&[MasterKeyPromotionStatus::Staged])?
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
        let cloud_keychain = self.cloud_keychain();
        let cspp = self.cspp();

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
            if !self.has_promotion_status(&[MasterKeyPromotionStatus::Staged])? {
                return Err(CloudBackupError::Internal(
                    "pending Cloud Backup enable has mismatched promotion state".into(),
                ));
            }

            return Ok((staged, journal.context()));
        }

        match cspp.master_key_promotion_status().map_err(|source| {
            CloudBackupError::internal_context("inspect staged master key", source)
        })? {
            MasterKeyPromotionStatus::None => {}
            MasterKeyPromotionStatus::Staged => {
                return Err(CloudBackupError::Internal(
                    "unowned staged master key must be recovered before enabling Cloud Backup"
                        .into(),
                ));
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
        self.record_pending_enable_passkey_metadata(master_key, passkey.into())
    }

    pub(crate) fn record_pending_enable_passkey(
        &self,
        master_key: &MasterKey,
        passkey: &UnpersistedPrfKey,
    ) -> Result<(), CloudBackupError> {
        self.record_pending_enable_passkey_metadata(master_key, passkey.into())
    }

    fn record_pending_enable_passkey_metadata(
        &self,
        master_key: &MasterKey,
        passkey: PendingEnablePasskeyMetadata,
    ) -> Result<(), CloudBackupError> {
        let cloud_keychain = self.cloud_keychain();
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
        if !self.has_staged_namespace(journal.namespace_id())?
            || !self.has_promotion_status(&[MasterKeyPromotionStatus::Staged])?
        {
            return Err(CloudBackupError::Internal(
                "pending Cloud Backup enable passkey has mismatched staged material".into(),
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

        let cloud_keychain = self.cloud_keychain();
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
            passkey.into(),
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
        let cspp = self.cspp();
        let staged = cspp.load_staged_master_key().map_err(|source| {
            CloudBackupError::internal_context("load staged master key for promotion", source)
        })?;
        if staged.as_ref().is_none_or(|master_key| master_key.namespace_id() != namespace_id) {
            return Err(CloudBackupError::Internal(
                "pending Cloud Backup enable has mismatched staged master key ownership".into(),
            ));
        }
        if !self.has_promotion_status(&[
            MasterKeyPromotionStatus::Staged,
            MasterKeyPromotionStatus::Pending(MasterKeyPromotionActiveState::Prior),
        ])? {
            return Err(CloudBackupError::Internal(
                "pending Cloud Backup enable has mismatched promotion state".into(),
            ));
        }

        let cloud_keychain = self.cloud_keychain();
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
            let original = CloudBackupError::internal_context("promote staged master key", source);
            return Err(Self::retain_promotion_error(
                original,
                self.restore_pending_enable_local_promotion_for_retry(),
            ));
        }
        if let Err(source) = cloud_keychain.save_passkey_and_namespace(
            &expected_passkey.credential_id,
            expected_passkey.prf_salt,
            journal.namespace_id(),
        ) {
            let original =
                CloudBackupError::internal_context("promote Cloud Backup passkey metadata", source);
            return Err(Self::retain_promotion_error(
                original,
                self.restore_pending_enable_local_promotion_for_retry(),
            ));
        }

        Ok(())
    }

    fn retain_promotion_error(
        original: CloudBackupError,
        rollback: Result<(), CloudBackupError>,
    ) -> CloudBackupError {
        match rollback {
            Ok(()) => original,
            Err(rollback) => CloudBackupError::Internal(
                format!("{original}; pending enable rollback also failed: {rollback}").into(),
            ),
        }
    }

    pub(crate) fn restore_pending_enable_local_promotion_for_retry(
        &self,
    ) -> Result<(), CloudBackupError> {
        let cloud_keychain = self.cloud_keychain();
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

        if !self.has_staged_namespace(journal.namespace_id())?
            || !matches!(
                self.promotion_status()?,
                MasterKeyPromotionStatus::Pending(_) | MasterKeyPromotionStatus::Staged
            )
        {
            return Err(CloudBackupError::Internal(
                "pending Cloud Backup enable cannot restore mismatched promotion state".into(),
            ));
        }

        let cspp = self.cspp();
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
        let cloud_keychain = self.cloud_keychain();
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
        if !self.has_staged_namespace(journal.namespace_id())?
            || !self.has_promotion_status(&[MasterKeyPromotionStatus::Pending(
                MasterKeyPromotionActiveState::Staged,
            )])?
        {
            return Err(CloudBackupError::Internal(
                "pending Cloud Backup enable cannot commit mismatched promotion state".into(),
            ));
        }

        self.cspp().commit_master_key_promotion().map_err(|source| {
            CloudBackupError::internal_context("commit staged master key promotion", source)
        })?;
        cloud_keychain.delete_pending_enable_journal().map_err(|source| {
            CloudBackupError::internal_context("clear committed pending enable state", source)
        })
    }

    pub(crate) fn discard_pending_enable_local_state(
        &self,
        journal: &PendingEnableJournal,
    ) -> Result<(), CloudBackupError> {
        let persisted = self
            .cloud_keychain()
            .load_pending_enable_journal()
            .map_err(|source| CloudBackupError::internal_context("load pending enable", source))?
            .ok_or_else(|| {
                CloudBackupError::Internal("pending Cloud Backup enable state is missing".into())
            })?;
        if &persisted != journal || !self.has_staged_namespace(journal.namespace_id())? {
            return Err(CloudBackupError::Internal(
                "refusing to discard mismatched pending Cloud Backup evidence".into(),
            ));
        }

        let cspp = self.cspp();
        match journal.phase() {
            PendingEnableJournalPhase::LocalPromotionStarted(_) => {
                if !matches!(self.promotion_status()?, MasterKeyPromotionStatus::Pending(_)) {
                    return Err(CloudBackupError::Internal(
                        "refusing to roll back mismatched pending Cloud Backup promotion".into(),
                    ));
                }
                cspp.rollback_master_key_promotion().map_err(|error| {
                    CloudBackupError::internal_context(
                        "roll back pending Cloud Backup master key promotion",
                        error,
                    )
                })?;
            }
            PendingEnableJournalPhase::Staged
            | PendingEnableJournalPhase::PasskeyRegistered(_)
            | PendingEnableJournalPhase::RemoteWritesStarted(_) => {
                if !self.has_promotion_status(&[MasterKeyPromotionStatus::Staged])? {
                    return Err(CloudBackupError::Internal(
                        "refusing to discard mismatched pending Cloud Backup stage".into(),
                    ));
                }
                cspp.discard_staged_master_key().map_err(|error| {
                    CloudBackupError::internal_context(
                        "discard pending Cloud Backup staged master key",
                        error,
                    )
                })?;
            }
        }

        let cloud_keychain = self.cloud_keychain();
        cloud_keychain.restore_passkey_metadata(journal.previous_metadata()).map_err(|error| {
            CloudBackupError::internal_context("restore prior Cloud Backup passkey metadata", error)
        })?;
        cloud_keychain.delete_pending_enable_journal().map_err(|error| {
            CloudBackupError::internal_context("clear pending Cloud Backup enable state", error)
        })
    }

    pub(crate) fn discard_unpromoted_enable_stage(
        &self,
        context: &str,
    ) -> Result<(), CloudBackupError> {
        warn!("{context}: discarding isolated staged master key");
        let journal = self
            .cloud_keychain()
            .load_pending_enable_journal()
            .map_err(|source| CloudBackupError::internal_context("load pending enable", source))?
            .ok_or_else(|| {
                CloudBackupError::Internal("pending Cloud Backup enable state is missing".into())
            })?;
        if !matches!(journal.phase(), PendingEnableJournalPhase::Staged)
            || !self.has_staged_namespace(journal.namespace_id())?
            || !self.has_promotion_status(&[MasterKeyPromotionStatus::Staged])?
        {
            return Err(CloudBackupError::Internal(
                "refusing to discard mismatched pending Cloud Backup stage".into(),
            ));
        }

        let cspp = self.cspp();
        cspp.discard_staged_master_key().map_err(|source| {
            CloudBackupError::internal_context("discard staged master key", source)
        })?;
        self.cloud_keychain().delete_pending_enable_journal().map_err(|source| {
            CloudBackupError::internal_context("clear pending enable state", source)
        })
    }

    fn has_staged_namespace(&self, namespace_id: &str) -> Result<bool, CloudBackupError> {
        self.cspp()
            .load_staged_master_key()
            .map(|staged| {
                staged.is_some_and(|master_key| master_key.namespace_id() == namespace_id)
            })
            .map_err(|source| {
                CloudBackupError::internal_context("inspect staged master key", source)
            })
    }

    fn promotion_status(&self) -> Result<MasterKeyPromotionStatus, CloudBackupError> {
        self.cspp().master_key_promotion_status().map_err(|source| {
            CloudBackupError::internal_context("inspect master key promotion", source)
        })
    }

    fn has_promotion_status(
        &self,
        expected: &[MasterKeyPromotionStatus],
    ) -> Result<bool, CloudBackupError> {
        Ok(expected.contains(&self.promotion_status()?))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rollback_success_preserves_original_promotion_error() {
        let error = PendingEnableCoordinator::retain_promotion_error(
            CloudBackupError::Internal("original promotion failure".into()),
            Ok(()),
        );

        assert_eq!(error.to_string(), "internal error: original promotion failure");
    }

    #[test]
    fn rollback_failure_retains_both_errors() {
        let error = PendingEnableCoordinator::retain_promotion_error(
            CloudBackupError::Internal("original promotion failure".into()),
            Err(CloudBackupError::Internal("rollback failure".into())),
        );
        let message = error.to_string();

        assert!(message.contains("original promotion failure"));
        assert!(message.contains("rollback failure"));
    }
}
