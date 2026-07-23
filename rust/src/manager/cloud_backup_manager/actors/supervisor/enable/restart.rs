use cove_cspp::{MasterKeyPromotionActiveState, MasterKeyPromotionStatus};
use cove_device::keychain::Keychain;
use zeroize::Zeroizing;

use super::*;
use crate::manager::cloud_backup_manager::wallets::StagedPrfKey;

enum PendingCompletionMatch {
    Matching(PendingVerificationCompletion),
    Missing,
    Conflicting,
}

#[cfg(test)]
mod recovery_code_tests {
    use super::*;
    use crate::manager::cloud_backup_manager::PendingEnablePasskeyMetadata;

    #[test]
    fn recovery_support_codes_are_stable_and_privacy_safe() {
        let cases = [
            (PendingEnableRecoveryReason::OwnershipJournalMissing, "CB-PE-001"),
            (PendingEnableRecoveryReason::JournalMaterialMismatch, "CB-PE-002"),
            (PendingEnableRecoveryReason::PromotionMismatch, "CB-PE-003"),
            (PendingEnableRecoveryReason::DurableCompletionMismatch, "CB-PE-004"),
            (PendingEnableRecoveryReason::UnreadableEvidence, "CB-PE-005"),
            (PendingEnableRecoveryReason::CleanupFailed, "CB-PE-006"),
        ];

        for (reason, expected) in cases {
            let state =
                PendingEnableRecoveryAssessment { reason, cleanup_plan: None }.public_state();

            assert_eq!(state.support_code, expected);
            assert!(!state.support_code.contains("namespace"));
            assert_eq!(state.cleanup, CloudBackupPendingEnableCleanupState::SupportOnly);
        }
    }

    #[test]
    fn cleanup_compatibility_rejects_incomplete_or_active_promotion_before_phase() {
        let registered =
            PendingEnableJournalPhase::PasskeyRegistered(PendingEnablePasskeyMetadata {
                credential_id: vec![1],
                prf_salt: [2; 32],
                provider_hint: None,
            });

        assert!(CloudBackupSupervisor::promotion_status_is_cleanup_compatible(
            &registered,
            MasterKeyPromotionStatus::Staged,
        ));
        assert!(!CloudBackupSupervisor::promotion_status_is_cleanup_compatible(
            &registered,
            MasterKeyPromotionStatus::Pending(MasterKeyPromotionActiveState::Staged),
        ));
        assert!(!CloudBackupSupervisor::promotion_status_is_cleanup_compatible(
            &registered,
            MasterKeyPromotionStatus::Pending(MasterKeyPromotionActiveState::Incomplete),
        ));
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PendingEnableRecoveryReason {
    OwnershipJournalMissing,
    JournalMaterialMismatch,
    PromotionMismatch,
    DurableCompletionMismatch,
    UnreadableEvidence,
    CleanupFailed,
}

impl PendingEnableRecoveryReason {
    fn support_code(self) -> &'static str {
        match self {
            Self::OwnershipJournalMissing => "CB-PE-001",
            Self::JournalMaterialMismatch => "CB-PE-002",
            Self::PromotionMismatch => "CB-PE-003",
            Self::DurableCompletionMismatch => "CB-PE-004",
            Self::UnreadableEvidence => "CB-PE-005",
            Self::CleanupFailed => "CB-PE-006",
        }
    }
}

enum PendingEnableCleanupPlan {
    UnownedStaged,
    Journaled { journal: Box<PendingEnableJournal>, promotion_status: MasterKeyPromotionStatus },
}

struct PendingEnableRecoveryAssessment {
    reason: PendingEnableRecoveryReason,
    cleanup_plan: Option<PendingEnableCleanupPlan>,
}

impl PendingEnableRecoveryAssessment {
    fn public_state(&self) -> CloudBackupPendingEnableRecovery {
        CloudBackupPendingEnableRecovery {
            support_code: self.reason.support_code().into(),
            cleanup: if self.cleanup_plan.is_some() {
                CloudBackupPendingEnableCleanupState::Available
            } else {
                CloudBackupPendingEnableCleanupState::SupportOnly
            },
        }
    }
}

impl CloudBackupSupervisor {
    pub async fn resume_pending_enable_after_restart(&mut self) -> ActorResult<()> {
        let Some(manager) = self.manager() else { return Produces::ok(()) };

        if let Err(error) = self.recover_pending_enable_after_restart(&manager) {
            error!("Failed to recover pending Cloud Backup enable state: {error}");
            let assessment = Self::pending_enable_recovery_assessment().unwrap_or_else(|| {
                Self::support_only(PendingEnableRecoveryReason::UnreadableEvidence)
            });
            manager.project_pending_enable_recovery(assessment.public_state());
        }

        Produces::ok(())
    }

    pub(crate) fn recover_pending_enable_after_restart(
        &mut self,
        manager: &RustCloudBackupManager,
    ) -> Result<(), CloudBackupError> {
        let cloud_keychain = CloudBackupKeychain::global();
        let journal = cloud_keychain.load_pending_enable_journal().map_err(|source| {
            CloudBackupError::internal_context("load pending enable during restart", source)
        })?;
        let cspp = cove_cspp::Cspp::new(Keychain::global().clone());
        if let Some(journal) = journal.as_ref()
            && matches!(journal.phase(), PendingEnableJournalPhase::Staging)
        {
            return Self::discard_staging_pending_enable(&cloud_keychain, &cspp);
        }

        let promotion_status = cspp.master_key_promotion_status().map_err(|source| {
            CloudBackupError::internal_context(
                "inspect staged master key during pending enable restart",
                source,
            )
        })?;
        let Some(journal) = journal else {
            return if promotion_status == MasterKeyPromotionStatus::None {
                Ok(())
            } else {
                Err(Self::pending_enable_restart_mismatch(
                    "CSPP staging or promotion material had no ownership journal",
                ))
            };
        };

        match journal.phase() {
            PendingEnableJournalPhase::Staging => {
                Self::discard_staging_pending_enable(&cloud_keychain, &cspp)
            }
            PendingEnableJournalPhase::Staged => Self::discard_staged_pending_enable(
                &cloud_keychain,
                &cspp,
                &journal,
                promotion_status,
            ),
            PendingEnableJournalPhase::PasskeyRegistered(_)
            | PendingEnableJournalPhase::RemoteWritesStarted(_) => {
                match journal.namespace_ownership() {
                    PendingEnableNamespaceOwnership::FreshOwned => self
                        .hydrate_pending_enable_confirmation(
                            manager,
                            &cspp,
                            &journal,
                            promotion_status,
                        ),
                    PendingEnableNamespaceOwnership::RecoveredExisting => self
                        .roll_back_recovered_existing_pending_enable(
                            manager,
                            &cloud_keychain,
                            &cspp,
                            &journal,
                            promotion_status,
                        ),
                }
            }
            PendingEnableJournalPhase::LocalPromotionStarted(_) => self
                .recover_pending_enable_local_promotion(
                    manager,
                    &cloud_keychain,
                    &cspp,
                    journal,
                    promotion_status,
                ),
        }
    }

    fn discard_staging_pending_enable(
        cloud_keychain: &CloudBackupKeychain,
        cspp: &cove_cspp::Cspp<Keychain>,
    ) -> Result<(), CloudBackupError> {
        cspp.discard_staged_master_key().map_err(|source| {
            CloudBackupError::internal_context(
                "discard interrupted pending enable staging during restart",
                source,
            )
        })?;
        cloud_keychain.delete_pending_enable_journal().map_err(|source| {
            CloudBackupError::internal_context(
                "clear interrupted pending enable ownership during restart",
                source,
            )
        })
    }

    fn discard_staged_pending_enable(
        cloud_keychain: &CloudBackupKeychain,
        cspp: &cove_cspp::Cspp<Keychain>,
        journal: &PendingEnableJournal,
        promotion_status: MasterKeyPromotionStatus,
    ) -> Result<(), CloudBackupError> {
        if promotion_status != MasterKeyPromotionStatus::Staged {
            return Err(Self::pending_enable_restart_mismatch(
                "staged journal did not have isolated staged CSPP material",
            ));
        }

        Self::validate_staged_namespace(cspp, journal.namespace_id())?;
        cspp.discard_staged_master_key().map_err(|source| {
            CloudBackupError::internal_context(
                "discard unregistered staged master key during restart",
                source,
            )
        })?;
        cloud_keychain.delete_pending_enable_journal().map_err(|source| {
            CloudBackupError::internal_context(
                "clear discarded pending enable state during restart",
                source,
            )
        })
    }

    fn hydrate_pending_enable_confirmation(
        &mut self,
        manager: &RustCloudBackupManager,
        cspp: &cove_cspp::Cspp<Keychain>,
        journal: &PendingEnableJournal,
        promotion_status: MasterKeyPromotionStatus,
    ) -> Result<(), CloudBackupError> {
        let valid_status = match journal.phase() {
            PendingEnableJournalPhase::PasskeyRegistered(_) => {
                promotion_status == MasterKeyPromotionStatus::Staged
            }
            PendingEnableJournalPhase::RemoteWritesStarted(_) => matches!(
                promotion_status,
                MasterKeyPromotionStatus::Staged
                    | MasterKeyPromotionStatus::Pending(MasterKeyPromotionActiveState::Prior)
            ),
            PendingEnableJournalPhase::Staging
            | PendingEnableJournalPhase::Staged
            | PendingEnableJournalPhase::LocalPromotionStarted(_) => false,
        };
        if !valid_status {
            return Err(Self::pending_enable_restart_mismatch(
                "registered journal did not have retryable staged CSPP material",
            ));
        }

        let master_key = Self::load_validated_staged_master_key(cspp, journal.namespace_id())?;
        let passkey = journal.passkey().ok_or_else(|| {
            Self::pending_enable_restart_mismatch(
                "registered journal did not contain passkey metadata",
            )
        })?;
        let passkey = StagedPrfKey {
            prf_salt: passkey.prf_salt,
            credential_id: passkey.credential_id.clone(),
            provider_hint: passkey.provider_hint.clone(),
        };

        self.pending_enable_session =
            Some(PendingEnableSession::awaiting_saved_passkey_confirmation(
                Zeroizing::new(master_key),
                Zeroizing::new(passkey),
                journal.context(),
            ));
        manager.project_enable_context_started(journal.context());
        manager.apply_enable_state(CloudBackupEnableState::AwaitingSavedPasskeyConfirmation(
            SavedPasskeyConfirmationMode::Manual,
        ));

        Ok(())
    }

    fn recover_pending_enable_local_promotion(
        &mut self,
        manager: &RustCloudBackupManager,
        cloud_keychain: &CloudBackupKeychain,
        cspp: &cove_cspp::Cspp<Keychain>,
        journal: PendingEnableJournal,
        promotion_status: MasterKeyPromotionStatus,
    ) -> Result<(), CloudBackupError> {
        match Self::pending_completion_match(&journal) {
            PendingCompletionMatch::Matching(completion) => {
                Self::finish_pending_enable_local_promotion(
                    manager,
                    cloud_keychain,
                    cspp,
                    &journal,
                    promotion_status,
                    completion,
                )
            }
            PendingCompletionMatch::Missing => {
                if journal.namespace_ownership()
                    == PendingEnableNamespaceOwnership::RecoveredExisting
                {
                    return self.roll_back_recovered_existing_pending_enable(
                        manager,
                        cloud_keychain,
                        cspp,
                        &journal,
                        promotion_status,
                    );
                }

                manager.pending_enable.restore_pending_enable_local_promotion_for_retry()?;
                let journal = cloud_keychain
                    .load_pending_enable_journal()
                    .map_err(|source| {
                        CloudBackupError::internal_context(
                            "reload rolled back pending enable during restart",
                            source,
                        )
                    })?
                    .ok_or_else(|| {
                        Self::pending_enable_restart_mismatch(
                            "rolled back pending enable journal disappeared",
                        )
                    })?;
                let promotion_status = cspp.master_key_promotion_status().map_err(|source| {
                    CloudBackupError::internal_context(
                        "inspect rolled back master key promotion during restart",
                        source,
                    )
                })?;

                self.hydrate_pending_enable_confirmation(manager, cspp, &journal, promotion_status)
            }
            PendingCompletionMatch::Conflicting => Err(Self::pending_enable_restart_mismatch(
                "pending verification completion did not match promoted enable namespace",
            )),
        }
    }

    fn roll_back_recovered_existing_pending_enable(
        &mut self,
        manager: &RustCloudBackupManager,
        cloud_keychain: &CloudBackupKeychain,
        cspp: &cove_cspp::Cspp<Keychain>,
        journal: &PendingEnableJournal,
        promotion_status: MasterKeyPromotionStatus,
    ) -> Result<(), CloudBackupError> {
        let valid_status = match journal.phase() {
            PendingEnableJournalPhase::PasskeyRegistered(_) => matches!(
                promotion_status,
                MasterKeyPromotionStatus::Staged | MasterKeyPromotionStatus::None
            ),
            PendingEnableJournalPhase::RemoteWritesStarted(_) => matches!(
                promotion_status,
                MasterKeyPromotionStatus::Staged
                    | MasterKeyPromotionStatus::Pending(MasterKeyPromotionActiveState::Prior)
                    | MasterKeyPromotionStatus::None
            ),
            PendingEnableJournalPhase::LocalPromotionStarted(_) => true,
            PendingEnableJournalPhase::Staging | PendingEnableJournalPhase::Staged => false,
        };
        if !valid_status {
            return Err(Self::pending_enable_restart_mismatch(
                "recovered-existing journal did not have rollback-compatible CSPP material",
            ));
        }

        if promotion_status == MasterKeyPromotionStatus::None {
            Self::validate_rolled_back_prior_namespace(cspp, journal)?;
        } else {
            Self::validate_staged_namespace(cspp, journal.namespace_id())?;
            cspp.rollback_master_key_promotion().map_err(|source| {
                CloudBackupError::internal_context(
                    "roll back recovered-existing master key during restart",
                    source,
                )
            })?;
        }

        cloud_keychain.restore_passkey_metadata(journal.previous_metadata()).map_err(|source| {
            CloudBackupError::internal_context(
                "restore prior recovered-existing passkey metadata during restart",
                source,
            )
        })?;
        cloud_keychain.delete_pending_enable_journal().map_err(|source| {
            CloudBackupError::internal_context(
                "clear rolled back recovered-existing enable state during restart",
                source,
            )
        })?;

        self.pending_enable_session = None;
        manager.clear_enable_progress_report();
        manager.sync_persisted_state();

        Ok(())
    }

    fn finish_pending_enable_local_promotion(
        manager: &RustCloudBackupManager,
        cloud_keychain: &CloudBackupKeychain,
        cspp: &cove_cspp::Cspp<Keychain>,
        journal: &PendingEnableJournal,
        promotion_status: MasterKeyPromotionStatus,
        completion: PendingVerificationCompletion,
    ) -> Result<(), CloudBackupError> {
        match promotion_status {
            MasterKeyPromotionStatus::Staged
            | MasterKeyPromotionStatus::Pending(
                MasterKeyPromotionActiveState::Prior
                | MasterKeyPromotionActiveState::Staged
                | MasterKeyPromotionActiveState::Incomplete,
            ) => {
                Self::validate_staged_namespace(cspp, journal.namespace_id())?;
                cspp.promote_staged_master_key().map_err(|source| {
                    CloudBackupError::internal_context(
                        "resume staged master key promotion during restart",
                        source,
                    )
                })?;
            }
            MasterKeyPromotionStatus::None => {
                let active = cspp.load_master_key_from_store().map_err(|source| {
                    CloudBackupError::internal_context(
                        "load committed master key during pending enable restart",
                        source,
                    )
                })?;
                if active.as_ref().map(cove_cspp::master_key::MasterKey::namespace_id).as_deref()
                    != Some(journal.namespace_id())
                {
                    return Err(Self::pending_enable_restart_mismatch(
                        "committed master key did not match pending enable namespace",
                    ));
                }
            }
        }

        let passkey = journal.passkey().ok_or_else(|| {
            Self::pending_enable_restart_mismatch(
                "promoted journal did not contain passkey metadata",
            )
        })?;
        cloud_keychain
            .save_passkey_and_namespace(
                &passkey.credential_id,
                passkey.prf_salt,
                journal.namespace_id(),
            )
            .map_err(|source| {
                CloudBackupError::internal_context(
                    "restore promoted passkey metadata during restart",
                    source,
                )
            })?;
        manager.activate_persisted_pending_verification_completion_for_source(
            completion,
            journal.context().verification_source,
        )?;
        cspp.commit_master_key_promotion().map_err(|source| {
            CloudBackupError::internal_context(
                "commit pending master key promotion during restart",
                source,
            )
        })?;
        cloud_keychain.delete_pending_enable_journal().map_err(|source| {
            CloudBackupError::internal_context(
                "clear committed pending enable state during restart",
                source,
            )
        })?;
        manager.sync_persisted_state();

        Ok(())
    }

    fn pending_completion_match(journal: &PendingEnableJournal) -> PendingCompletionMatch {
        let state = RustCloudBackupManager::load_persisted_state();
        match state {
            PersistedCloudBackupState::Configured(_) => {
                match state.pending_verification_completion().cloned() {
                    Some(completion) if completion.namespace_id() == journal.namespace_id() => {
                        PendingCompletionMatch::Matching(completion)
                    }
                    Some(_) => PendingCompletionMatch::Conflicting,
                    None => PendingCompletionMatch::Missing,
                }
            }
            PersistedCloudBackupState::Disabled => PendingCompletionMatch::Missing,
            PersistedCloudBackupState::ExclusiveTransition(_)
            | PersistedCloudBackupState::Corrupted { .. } => PendingCompletionMatch::Conflicting,
        }
    }

    fn load_validated_staged_master_key(
        cspp: &cove_cspp::Cspp<Keychain>,
        namespace_id: &str,
    ) -> Result<cove_cspp::master_key::MasterKey, CloudBackupError> {
        let master_key = cspp
            .load_staged_master_key()
            .map_err(|source| {
                CloudBackupError::internal_context(
                    "load staged master key during pending enable restart",
                    source,
                )
            })?
            .ok_or_else(|| {
                Self::pending_enable_restart_mismatch(
                    "pending enable journal had no staged master key",
                )
            })?;
        if master_key.namespace_id() != namespace_id {
            return Err(Self::pending_enable_restart_mismatch(
                "staged master key did not match pending enable namespace",
            ));
        }

        Ok(master_key)
    }

    fn validate_staged_namespace(
        cspp: &cove_cspp::Cspp<Keychain>,
        namespace_id: &str,
    ) -> Result<(), CloudBackupError> {
        Self::load_validated_staged_master_key(cspp, namespace_id).map(|_| ())
    }

    fn validate_rolled_back_prior_namespace(
        cspp: &cove_cspp::Cspp<Keychain>,
        journal: &PendingEnableJournal,
    ) -> Result<(), CloudBackupError> {
        let active_namespace = cspp
            .load_master_key_from_store()
            .map_err(|source| {
                CloudBackupError::internal_context(
                    "load prior master key during recovered-existing restart rollback",
                    source,
                )
            })?
            .map(|master_key| master_key.namespace_id());
        if active_namespace.as_deref() != journal.previous_metadata().namespace_id.as_deref() {
            return Err(Self::pending_enable_restart_mismatch(
                "rolled back master key did not match prior metadata namespace",
            ));
        }

        Ok(())
    }

    fn pending_enable_restart_mismatch(message: &str) -> CloudBackupError {
        CloudBackupError::Internal(
            format!("pending Cloud Backup enable restart state mismatch: {message}").into(),
        )
    }

    pub async fn confirm_pending_enable_cleanup(&mut self) -> ActorResult<()> {
        let Some(manager) = self.manager() else { return Produces::ok(()) };
        let Some(assessment) = Self::pending_enable_recovery_assessment() else {
            manager.sync_persisted_state();
            return Produces::ok(());
        };
        let public_assessment = assessment.public_state();
        let Some(plan) = assessment.cleanup_plan else {
            manager.project_pending_enable_recovery(public_assessment);
            return Produces::ok(());
        };

        manager.project_pending_enable_recovery(CloudBackupPendingEnableRecovery {
            support_code: assessment.reason.support_code().into(),
            cleanup: CloudBackupPendingEnableCleanupState::Cleaning,
        });

        if let Err(error) = Self::clean_pending_enable_local_evidence(&plan) {
            warn!("Pending Cloud Backup enable local cleanup failed: {error}");
            let cleanup = Self::pending_enable_recovery_assessment()
                .and_then(|assessment| assessment.cleanup_plan)
                .map_or(CloudBackupPendingEnableCleanupState::SupportOnly, |_| {
                    CloudBackupPendingEnableCleanupState::Available
                });
            manager.project_pending_enable_recovery(CloudBackupPendingEnableRecovery {
                support_code: PendingEnableRecoveryReason::CleanupFailed.support_code().into(),
                cleanup,
            });
            return Produces::ok(());
        }

        self.pending_enable_session = None;
        manager.clear_enable_progress_report();
        manager.sync_persisted_state();

        Produces::ok(())
    }

    fn clean_pending_enable_local_evidence(
        plan: &PendingEnableCleanupPlan,
    ) -> Result<(), CloudBackupError> {
        let cspp = cove_cspp::Cspp::new(Keychain::global().clone());
        let PendingEnableCleanupPlan::Journaled { journal, promotion_status } = plan else {
            let journal =
                CloudBackupKeychain::global().load_pending_enable_journal().map_err(|source| {
                    CloudBackupError::internal_context(
                        "revalidate pending enable journal before isolated stage cleanup",
                        source,
                    )
                })?;
            if journal.is_some()
                || cspp.master_key_promotion_status().map_err(|source| {
                    CloudBackupError::internal_context(
                        "revalidate isolated staged material before cleanup",
                        source,
                    )
                })? != MasterKeyPromotionStatus::Staged
            {
                return Err(CloudBackupError::Internal(
                    "pending enable isolated stage changed before cleanup".into(),
                ));
            }

            return cspp.discard_staged_master_key().map_err(|source| {
                CloudBackupError::internal_context(
                    "discard unowned pending enable staged material during local cleanup",
                    source,
                )
            });
        };

        match promotion_status {
            MasterKeyPromotionStatus::None => {}
            MasterKeyPromotionStatus::Staged => {
                cspp.discard_staged_master_key().map_err(|source| {
                    CloudBackupError::internal_context(
                        "discard pending enable staged material during local cleanup",
                        source,
                    )
                })?
            }
            MasterKeyPromotionStatus::Pending(_) => {
                cspp.rollback_master_key_promotion().map_err(|source| {
                    CloudBackupError::internal_context(
                        "restore prior master key during pending enable local cleanup",
                        source,
                    )
                })?
            }
        }

        let cloud_keychain = CloudBackupKeychain::global();
        cloud_keychain.restore_passkey_metadata(journal.previous_metadata()).map_err(|source| {
            CloudBackupError::internal_context(
                "restore prior metadata during pending enable local cleanup",
                source,
            )
        })?;
        cloud_keychain.delete_pending_enable_journal().map_err(|source| {
            CloudBackupError::internal_context(
                "clear pending enable journal during local cleanup",
                source,
            )
        })
    }

    fn pending_enable_recovery_assessment() -> Option<PendingEnableRecoveryAssessment> {
        let cloud_keychain = CloudBackupKeychain::global();
        let journal = match cloud_keychain.load_pending_enable_journal() {
            Ok(journal) => journal,
            Err(_) => {
                return Some(Self::support_only(PendingEnableRecoveryReason::UnreadableEvidence));
            }
        };
        let cspp = cove_cspp::Cspp::new(Keychain::global().clone());
        let Some(journal) = journal else {
            return match cspp.master_key_promotion_status() {
                Ok(MasterKeyPromotionStatus::None) => None,
                Ok(MasterKeyPromotionStatus::Staged) => Some(PendingEnableRecoveryAssessment {
                    reason: PendingEnableRecoveryReason::OwnershipJournalMissing,
                    cleanup_plan: Some(PendingEnableCleanupPlan::UnownedStaged),
                }),
                Err(_) => Some(Self::support_only(PendingEnableRecoveryReason::UnreadableEvidence)),
                Ok(MasterKeyPromotionStatus::Pending(_)) => {
                    Some(Self::support_only(PendingEnableRecoveryReason::OwnershipJournalMissing))
                }
            };
        };

        if !Self::metadata_snapshot_is_well_formed(journal.previous_metadata()) {
            return Some(Self::support_only(PendingEnableRecoveryReason::JournalMaterialMismatch));
        }

        let prior_namespace = journal.previous_metadata().namespace_id.as_deref();
        let evidence = match cspp
            .master_key_promotion_evidence(journal.namespace_id(), prior_namespace)
        {
            Ok(evidence) => evidence,
            Err(_) => {
                return Some(Self::support_only(PendingEnableRecoveryReason::UnreadableEvidence));
            }
        };
        if !evidence.prior_matches_expected
            || (evidence.status != MasterKeyPromotionStatus::None
                && !evidence.staged_matches_expected)
        {
            return Some(Self::support_only(PendingEnableRecoveryReason::JournalMaterialMismatch));
        }

        if matches!(
            evidence.status,
            MasterKeyPromotionStatus::Pending(MasterKeyPromotionActiveState::Incomplete)
        ) || !Self::promotion_status_is_cleanup_compatible(journal.phase(), evidence.status)
        {
            return Some(Self::support_only(PendingEnableRecoveryReason::PromotionMismatch));
        }

        let persisted = match Database::global().cloud_backup_state.get() {
            Ok(persisted) => persisted,
            Err(_) => {
                return Some(Self::support_only(PendingEnableRecoveryReason::UnreadableEvidence));
            }
        };
        if !Self::persisted_prior_state_matches(&persisted, journal.previous_metadata())
            || persisted.pending_verification_completion().is_some()
        {
            return Some(Self::support_only(
                PendingEnableRecoveryReason::DurableCompletionMismatch,
            ));
        }

        Some(PendingEnableRecoveryAssessment {
            reason: PendingEnableRecoveryReason::PromotionMismatch,
            cleanup_plan: Some(PendingEnableCleanupPlan::Journaled {
                journal: Box::new(journal),
                promotion_status: evidence.status,
            }),
        })
    }

    fn support_only(reason: PendingEnableRecoveryReason) -> PendingEnableRecoveryAssessment {
        PendingEnableRecoveryAssessment { reason, cleanup_plan: None }
    }

    fn metadata_snapshot_is_well_formed(snapshot: &PendingEnableLocalMetadataSnapshot) -> bool {
        match (&snapshot.credential_id, &snapshot.prf_salt, &snapshot.namespace_id) {
            (None, None, None) => true,
            (Some(credential_id), Some(prf_salt), Some(namespace_id)) => {
                !namespace_id.is_empty()
                    && hex::decode(credential_id).is_ok_and(|bytes| !bytes.is_empty())
                    && hex::decode(prf_salt).is_ok_and(|bytes| bytes.len() == 32)
            }
            _ => false,
        }
    }

    fn promotion_status_is_cleanup_compatible(
        phase: &PendingEnableJournalPhase,
        status: MasterKeyPromotionStatus,
    ) -> bool {
        match phase {
            PendingEnableJournalPhase::Staging
            | PendingEnableJournalPhase::Staged
            | PendingEnableJournalPhase::PasskeyRegistered(_)
            | PendingEnableJournalPhase::RemoteWritesStarted(_) => matches!(
                status,
                MasterKeyPromotionStatus::None
                    | MasterKeyPromotionStatus::Staged
                    | MasterKeyPromotionStatus::Pending(MasterKeyPromotionActiveState::Prior)
            ),
            PendingEnableJournalPhase::LocalPromotionStarted(_) => matches!(
                status,
                MasterKeyPromotionStatus::None
                    | MasterKeyPromotionStatus::Staged
                    | MasterKeyPromotionStatus::Pending(MasterKeyPromotionActiveState::Prior)
                    | MasterKeyPromotionStatus::Pending(MasterKeyPromotionActiveState::Staged)
            ),
        }
    }

    fn persisted_prior_state_matches(
        persisted: &PersistedCloudBackupState,
        previous_metadata: &PendingEnableLocalMetadataSnapshot,
    ) -> bool {
        match previous_metadata.namespace_id {
            Some(_) => persisted.is_configured(),
            None => matches!(persisted, PersistedCloudBackupState::Disabled),
        }
    }
}
