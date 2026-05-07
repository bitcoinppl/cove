use crate::manager::cloud_backup_manager::{
    CloudBackupDetail, CloudBackupVerificationPresentation, CloudBackupVerificationReason,
    CloudBackupVerificationSource, DeepVerificationFailure, DeepVerificationReport,
    PendingUploadVerificationState, RecoveryState, VerificationState,
};

#[derive(Debug, Clone, Default)]
pub(crate) struct CloudBackupVerificationEffect {
    pub(crate) presentation: Option<CloudBackupVerificationPresentation>,
    pub(crate) verification: Option<VerificationState>,
    pub(crate) pending_upload_verification: Option<PendingUploadVerificationState>,
    pub(crate) recovery: Option<RecoveryState>,
    pub(crate) detail: Option<CloudBackupDetail>,
    pub(crate) refresh_sync_health: bool,
}

#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct CloudBackupVerificationCoordinator;

impl CloudBackupVerificationCoordinator {
    fn hidden(
        source: Option<CloudBackupVerificationSource>,
    ) -> CloudBackupVerificationPresentation {
        CloudBackupVerificationPresentation::Hidden { source }
    }

    pub(crate) fn begin_manual(
        source: CloudBackupVerificationSource,
    ) -> CloudBackupVerificationEffect {
        CloudBackupVerificationEffect {
            presentation: Some(CloudBackupVerificationPresentation::ManualVerifying { source }),
            verification: Some(VerificationState::Verifying),
            ..CloudBackupVerificationEffect::default()
        }
    }

    pub(crate) fn begin_manual_presentation(
        source: CloudBackupVerificationSource,
    ) -> CloudBackupVerificationEffect {
        CloudBackupVerificationEffect {
            presentation: Some(CloudBackupVerificationPresentation::ManualVerifying { source }),
            ..CloudBackupVerificationEffect::default()
        }
    }

    pub(crate) fn begin_background_confirmation(
        source: CloudBackupVerificationSource,
    ) -> CloudBackupVerificationEffect {
        CloudBackupVerificationEffect {
            presentation: Some(CloudBackupVerificationPresentation::BackgroundConfirming(source)),
            pending_upload_verification: Some(PendingUploadVerificationState::Confirming),
            verification: Some(VerificationState::Idle),
            ..CloudBackupVerificationEffect::default()
        }
    }

    pub(crate) fn block_background_on_authorization(
        source: CloudBackupVerificationSource,
    ) -> CloudBackupVerificationEffect {
        CloudBackupVerificationEffect {
            presentation: Some(
                CloudBackupVerificationPresentation::BackgroundBlockedOnAuthorization(source),
            ),
            pending_upload_verification: Some(
                PendingUploadVerificationState::BlockedOnAuthorization,
            ),
            ..CloudBackupVerificationEffect::default()
        }
    }

    pub(crate) fn pending_upload_state(
        pending: PendingUploadVerificationState,
        source: CloudBackupVerificationSource,
    ) -> CloudBackupVerificationEffect {
        match pending {
            PendingUploadVerificationState::Idle => CloudBackupVerificationEffect {
                presentation: Some(Self::hidden(Some(source))),
                pending_upload_verification: Some(PendingUploadVerificationState::Idle),
                ..CloudBackupVerificationEffect::default()
            },
            PendingUploadVerificationState::Confirming => {
                Self::begin_background_confirmation(source)
            }
            PendingUploadVerificationState::BlockedOnAuthorization => {
                Self::block_background_on_authorization(source)
            }
        }
    }

    pub(crate) fn complete(
        source: CloudBackupVerificationSource,
        report: DeepVerificationReport,
    ) -> CloudBackupVerificationEffect {
        CloudBackupVerificationEffect {
            presentation: Some(CloudBackupVerificationPresentation::Completed { source }),
            verification: Some(VerificationState::Verified(report.clone())),
            recovery: Some(RecoveryState::Idle),
            detail: report.detail.clone(),
            refresh_sync_health: true,
            ..CloudBackupVerificationEffect::default()
        }
    }

    pub(crate) fn fail(
        source: CloudBackupVerificationSource,
        failure: DeepVerificationFailure,
    ) -> CloudBackupVerificationEffect {
        CloudBackupVerificationEffect {
            presentation: Some(CloudBackupVerificationPresentation::Failed {
                source,
                message: failure.message(),
            }),
            verification: Some(VerificationState::Failed(failure.clone())),
            recovery: Some(RecoveryState::Idle),
            detail: failure.detail().cloned(),
            refresh_sync_health: true,
            ..CloudBackupVerificationEffect::default()
        }
    }

    pub(crate) fn needs_decision(
        reason: CloudBackupVerificationReason,
        source: CloudBackupVerificationSource,
    ) -> CloudBackupVerificationEffect {
        CloudBackupVerificationEffect {
            presentation: Some(CloudBackupVerificationPresentation::NeedsDecision {
                reason,
                source,
            }),
            ..CloudBackupVerificationEffect::default()
        }
    }

    pub(crate) fn dismiss_decision(
        source: CloudBackupVerificationSource,
    ) -> CloudBackupVerificationEffect {
        CloudBackupVerificationEffect {
            presentation: Some(Self::hidden(Some(source))),
            ..CloudBackupVerificationEffect::default()
        }
    }

    pub(crate) fn current_source(
        presentation: &CloudBackupVerificationPresentation,
    ) -> CloudBackupVerificationSource {
        match presentation {
            CloudBackupVerificationPresentation::ManualVerifying { source }
            | CloudBackupVerificationPresentation::Completed { source }
            | CloudBackupVerificationPresentation::Failed { source, .. } => *source,
            CloudBackupVerificationPresentation::BackgroundConfirming(source)
            | CloudBackupVerificationPresentation::BackgroundBlockedOnAuthorization(source) => {
                *source
            }
            CloudBackupVerificationPresentation::Hidden { source } => {
                source.unwrap_or(CloudBackupVerificationSource::Settings)
            }
            CloudBackupVerificationPresentation::NeedsDecision { source, .. } => *source,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn default_report() -> DeepVerificationReport {
        DeepVerificationReport {
            master_key_wrapper_repaired: false,
            local_master_key_repaired: false,
            credential_recovered: false,
            wallets_verified: 0,
            wallets_failed: 0,
            wallets_unsupported: 0,
            detail: None,
        }
    }

    #[test]
    fn manual_verification_preserves_source() {
        let effect = CloudBackupVerificationCoordinator::begin_manual(
            CloudBackupVerificationSource::Onboarding,
        );

        assert_eq!(
            effect.presentation,
            Some(CloudBackupVerificationPresentation::ManualVerifying {
                source: CloudBackupVerificationSource::Onboarding
            })
        );
        assert_eq!(effect.verification, Some(VerificationState::Verifying));
    }

    #[test]
    fn background_confirmation_clears_interactive_verification() {
        let effect = CloudBackupVerificationCoordinator::begin_background_confirmation(
            CloudBackupVerificationSource::Onboarding,
        );

        assert_eq!(
            effect.presentation,
            Some(CloudBackupVerificationPresentation::BackgroundConfirming(
                CloudBackupVerificationSource::Onboarding,
            ))
        );
        assert_eq!(
            effect.pending_upload_verification,
            Some(PendingUploadVerificationState::Confirming)
        );
        assert_eq!(effect.verification, Some(VerificationState::Idle));
    }

    #[test]
    fn authorization_block_uses_background_presentation() {
        let effect = CloudBackupVerificationCoordinator::block_background_on_authorization(
            CloudBackupVerificationSource::RootPrompt,
        );

        assert_eq!(
            effect.presentation,
            Some(CloudBackupVerificationPresentation::BackgroundBlockedOnAuthorization(
                CloudBackupVerificationSource::RootPrompt,
            ))
        );
        assert_eq!(
            effect.pending_upload_verification,
            Some(PendingUploadVerificationState::BlockedOnAuthorization)
        );
    }

    #[test]
    fn completion_preserves_source() {
        let effect = CloudBackupVerificationCoordinator::complete(
            CloudBackupVerificationSource::CloudBackupDetail,
            default_report(),
        );

        assert_eq!(
            effect.presentation,
            Some(CloudBackupVerificationPresentation::Completed {
                source: CloudBackupVerificationSource::CloudBackupDetail
            })
        );
        assert!(matches!(effect.verification, Some(VerificationState::Verified(_))));
    }

    #[test]
    fn failed_preserves_source() {
        let failure = DeepVerificationFailure::retry("verification failed", None, None);
        let effect = CloudBackupVerificationCoordinator::fail(
            CloudBackupVerificationSource::Settings,
            failure,
        );

        assert_eq!(
            effect.presentation,
            Some(CloudBackupVerificationPresentation::Failed {
                source: CloudBackupVerificationSource::Settings,
                message: "verification failed".into()
            })
        );
        assert!(matches!(effect.verification, Some(VerificationState::Failed(_))));
        assert_eq!(effect.recovery, Some(RecoveryState::Idle));
    }

    #[test]
    fn idle_pending_upload_hides_presentation() {
        let effect = CloudBackupVerificationCoordinator::pending_upload_state(
            PendingUploadVerificationState::Idle,
            CloudBackupVerificationSource::Onboarding,
        );

        assert_eq!(effect.pending_upload_verification, Some(PendingUploadVerificationState::Idle));
        assert_eq!(
            effect.presentation,
            Some(CloudBackupVerificationPresentation::Hidden {
                source: Some(CloudBackupVerificationSource::Onboarding),
            })
        );
    }

    #[test]
    fn decision_transitions_preserve_source() {
        let decision = CloudBackupVerificationCoordinator::needs_decision(
            CloudBackupVerificationReason::BackupChanged,
            CloudBackupVerificationSource::RootPrompt,
        );

        assert_eq!(
            decision.presentation,
            Some(CloudBackupVerificationPresentation::NeedsDecision {
                reason: CloudBackupVerificationReason::BackupChanged,
                source: CloudBackupVerificationSource::RootPrompt,
            })
        );

        let hidden = CloudBackupVerificationCoordinator::dismiss_decision(
            CloudBackupVerificationSource::RootPrompt,
        );

        assert_eq!(
            hidden.presentation,
            Some(CloudBackupVerificationPresentation::Hidden {
                source: Some(CloudBackupVerificationSource::RootPrompt),
            })
        );
    }

    #[test]
    fn current_source_preserves_background_source() {
        assert_eq!(
            CloudBackupVerificationCoordinator::current_source(
                &CloudBackupVerificationPresentation::BackgroundConfirming(
                    CloudBackupVerificationSource::Onboarding,
                ),
            ),
            CloudBackupVerificationSource::Onboarding
        );
    }

    #[test]
    fn current_source_uses_embedded_or_default_source() {
        assert_eq!(
            CloudBackupVerificationCoordinator::current_source(
                &CloudBackupVerificationPresentation::Hidden {
                    source: Some(CloudBackupVerificationSource::CloudBackupDetail),
                },
            ),
            CloudBackupVerificationSource::CloudBackupDetail
        );
        assert_eq!(
            CloudBackupVerificationCoordinator::current_source(
                &CloudBackupVerificationPresentation::Hidden { source: None },
            ),
            CloudBackupVerificationSource::Settings
        );
        assert_eq!(
            CloudBackupVerificationCoordinator::current_source(
                &CloudBackupVerificationPresentation::NeedsDecision {
                    reason: CloudBackupVerificationReason::BackupChanged,
                    source: CloudBackupVerificationSource::Onboarding,
                },
            ),
            CloudBackupVerificationSource::Onboarding
        );
    }
}
