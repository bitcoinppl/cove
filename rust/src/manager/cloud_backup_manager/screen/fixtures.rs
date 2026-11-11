//! Named screens for the platform contract tests
//!
//! Each platform renders every fixture and checks that an available action has an enabled,
//! reachable control and an unavailable one is disabled or absent. Debug builds only: a release
//! build returns no fixtures, and `tools/release-seams/rust-markers.txt` lists the name prefix so
//! the release-seam check fails if the list ever reaches a release library

use super::types::CloudBackupScreenFixture;

/// The named screens the platform contract tests render; empty in release builds
#[uniffi::export]
pub fn cloud_backup_screen_fixtures() -> Vec<CloudBackupScreenFixture> {
    #[cfg(debug_assertions)]
    return debug::fixtures();

    #[cfg(not(debug_assertions))]
    Vec::new()
}

#[cfg(debug_assertions)]
mod debug {
    use cove_device::cloud_storage::CloudSyncHealth;

    use super::super::types::{
        CloudBackupActionAvailability, CloudBackupActionEntry, CloudBackupActions,
        CloudBackupConfiguredScreen, CloudBackupDeleteConfirmation, CloudBackupDisableConfirmation,
        CloudBackupDisableImpact, CloudBackupDisableReview, CloudBackupDisableScreen,
        CloudBackupDriveAccountSwitchRecovery, CloudBackupDriveAccountSwitchScreen,
        CloudBackupExtraCopiesSection, CloudBackupFinishingDisableScreen,
        CloudBackupFinishingDisableStage, CloudBackupHealNotice, CloudBackupInventoryAssessment,
        CloudBackupListedRecord, CloudBackupListedRow, CloudBackupNamedCount,
        CloudBackupNotOnDeviceSection, CloudBackupOperationKind, CloudBackupOwnRecord,
        CloudBackupOwnRow, CloudBackupRecoveryOperation, CloudBackupRejection,
        CloudBackupRowAction, CloudBackupRowActionKind, CloudBackupScreen, CloudBackupScreenAction,
        CloudBackupScreenBody, CloudBackupScreenFixture, CloudBackupSection,
        CloudBackupTurnOffFailure, CloudBackupVerificationView, CloudBackupWalletsSection,
    };
    use crate::manager::cloud_backup_manager::{
        CloudBackupEnableContext, CloudBackupEnableFlow, CloudBackupInventoryIncompleteReason,
        CloudBackupPasskeyChoiceIntent, CloudBackupPasskeyState,
        CloudBackupPendingEnableCleanupState, CloudBackupPendingEnableRecovery,
        CloudBackupRestoreAllState, CloudBackupRestoreFlow, CloudBackupRootPrompt,
        CloudBackupSettingsRowStatus, CloudBackupSyncState, CloudBackupVerificationPresentation,
        CloudBackupVerificationState, CloudBackupWalletItem, CloudBackupWalletStatus,
        CloudOnlyOperation, RecordId,
    };

    /// Prefixes every fixture name; the release-seam check looks for it
    const NAME_PREFIX: &str = "cloud-backup-screen-fixture:";

    const VERIFIED_AT: u64 = 1_700_000_000;

    use CloudBackupScreenAction as A;

    pub(super) fn fixtures() -> Vec<CloudBackupScreenFixture> {
        vec![
            fixture("disabled", disabled()),
            fixture("enabling", enabling()),
            fixture("enabling-existing-backup-prompt", existing_backup_prompt()),
            fixture("enabling-passkey-choice-existing-only", passkey_choice_existing_only()),
            fixture("restoring", restoring()),
            fixture("configured", configured(CloudBackupDisableScreen::Idle, None)),
            fixture("configured-passkey-missing", passkey_missing()),
            fixture("configured-verification-prompt", verification_prompt()),
            fixture("configured-verification-prompt-busy", verification_prompt_busy()),
            fixture("configured-operation-running", operation_running()),
            fixture("configured-heal-notice", heal_notice()),
            fixture("configured-rows-held", rows_held()),
            fixture("configured-extra-copy-only-copy", extra_copy_only_copy()),
            fixture("configured-extra-copy-gone", extra_copy_gone()),
            fixture("turn-off-review", turn_off_review()),
            fixture("finishing-disable", finishing_disable()),
            fixture("drive-account-switch", drive_account_switch()),
            fixture("pending-enable-recovery", pending_enable_recovery()),
            fixture("load-failed", load_failed()),
            fixture("corrupted", corrupted()),
        ]
    }

    fn fixture(name: &str, screen: CloudBackupScreen) -> CloudBackupScreenFixture {
        CloudBackupScreenFixture { name: format!("{NAME_PREFIX}{name}"), screen }
    }

    fn screen(
        body: CloudBackupScreenBody,
        settings_row: CloudBackupSettingsRowStatus,
        actions: CloudBackupActions,
    ) -> CloudBackupScreen {
        CloudBackupScreen {
            body,
            settings_row,
            heal_notice: None,
            last_outcome: None,
            root_prompt: CloudBackupRootPrompt::None,
            actions,
        }
    }

    fn available(actions: &[CloudBackupScreenAction]) -> CloudBackupActions {
        let entries = actions
            .iter()
            .map(|&action| CloudBackupActionEntry {
                action,
                availability: CloudBackupActionAvailability::Available,
            })
            .collect();

        CloudBackupActions { entries }
    }

    fn wallet(
        name: &str,
        record_id: &str,
        sync_status: CloudBackupWalletStatus,
    ) -> CloudBackupWalletItem {
        CloudBackupWalletItem {
            name: name.into(),
            network: None,
            wallet_mode: None,
            wallet_type: None,
            fingerprint: None,
            label_count: None,
            backup_updated_at: Some(VERIFIED_AT),
            sync_status,
            restore_failure: None,
            record_id: record_id.into(),
        }
    }

    fn row(
        record_id: &str,
        record: CloudBackupListedRecord,
        actions: Vec<CloudBackupRowActionKind>,
    ) -> CloudBackupListedRow {
        CloudBackupListedRow {
            record_id: RecordId::from(record_id.to_string()),
            record,
            actions: actions
                .into_iter()
                .map(|action| CloudBackupRowAction {
                    action,
                    availability: CloudBackupActionAvailability::Available,
                })
                .collect(),
        }
    }

    fn disabled() -> CloudBackupScreen {
        screen(
            CloudBackupScreenBody::Disabled,
            CloudBackupSettingsRowStatus::Disabled,
            available(&[A::Enable, A::RestoreFromCloudBackup]),
        )
    }

    fn enabling() -> CloudBackupScreen {
        screen(
            CloudBackupScreenBody::Enabling(CloudBackupEnableFlow::DiscoveringExistingBackup),
            CloudBackupSettingsRowStatus::SettingUp,
            CloudBackupActions::default(),
        )
    }

    /// Enable found a backup and asks whether to use its passkey or start over
    fn existing_backup_prompt() -> CloudBackupScreen {
        let context = CloudBackupEnableContext::settings_manual();
        let mut screen = screen(
            CloudBackupScreenBody::Enabling(CloudBackupEnableFlow::AwaitingForceNewConfirmation(
                context, None,
            )),
            CloudBackupSettingsRowStatus::SettingUp,
            available(&[
                A::DiscardPendingEnable,
                A::AcceptEnablePromptUseExisting,
                A::AcceptEnablePromptCreateNew,
            ]),
        );

        screen.root_prompt = CloudBackupRootPrompt::ExistingBackupFound(context, None);
        screen
    }

    /// The passkey choice for a backup that may already exist, which can only use its passkey
    fn passkey_choice_existing_only() -> CloudBackupScreen {
        let intent = CloudBackupPasskeyChoiceIntent::EnableExistingPasskeyOnly(
            CloudBackupEnableContext::settings_manual(),
            None,
        );
        let mut screen = screen(
            CloudBackupScreenBody::Enabling(CloudBackupEnableFlow::AwaitingPasskeyChoice(
                intent.clone(),
            )),
            CloudBackupSettingsRowStatus::SettingUp,
            available(&[A::DiscardPendingEnable, A::AcceptEnablePromptUseExisting]),
        );

        screen.root_prompt = CloudBackupRootPrompt::PasskeyChoice(intent);
        screen
    }

    fn restoring() -> CloudBackupScreen {
        screen(
            CloudBackupScreenBody::Restoring(CloudBackupRestoreFlow::Downloading {
                completed: 1,
                total: 3,
            }),
            CloudBackupSettingsRowStatus::Restoring,
            available(&[A::CancelRestore]),
        )
    }

    fn configured_actions() -> Vec<CloudBackupScreenAction> {
        vec![
            A::Verify,
            A::SyncUnsynced,
            A::RefreshDetail,
            A::RecreateManifest,
            A::ReinitializeBackup,
            A::SwitchDriveAccount,
            A::TurnOff,
            A::StartRestoreAll,
        ]
    }

    fn configured_sections(passkey: CloudBackupPasskeyState) -> Vec<CloudBackupSection> {
        let mut sections = Vec::new();
        if passkey != CloudBackupPasskeyState::Available {
            sections.push(CloudBackupSection::PasskeyCard(passkey));
        }

        sections.extend([
            CloudBackupSection::Verification(CloudBackupVerificationView {
                state: CloudBackupVerificationState::Verified {
                    report: None,
                    last_verified_at: Some(VERIFIED_AT),
                },
                presentation: CloudBackupVerificationPresentation::Hidden { source: None },
                recovery: CloudBackupRecoveryOperation::Idle,
            }),
            CloudBackupSection::Wallets(CloudBackupWalletsSection {
                up_to_date: vec![CloudBackupOwnRow {
                    wallet: wallet(
                        "Spending",
                        "record-spending",
                        CloudBackupWalletStatus::Confirmed,
                    ),
                    record: CloudBackupOwnRecord::Current,
                }],
                needs_sync: vec![CloudBackupOwnRow {
                    wallet: wallet("Savings", "record-savings", CloudBackupWalletStatus::Dirty),
                    record: CloudBackupOwnRecord::Pending,
                }],
                sync: CloudBackupSyncState::Idle,
            }),
            CloudBackupSection::NotOnDevice(CloudBackupNotOnDeviceSection {
                rows: vec![
                    row(
                        "record-cold",
                        CloudBackupListedRecord::NotOnDevice {
                            wallet: wallet(
                                "Cold Storage",
                                "record-cold",
                                CloudBackupWalletStatus::Confirmed,
                            ),
                        },
                        vec![
                            CloudBackupRowActionKind::Restore,
                            CloudBackupRowActionKind::Delete {
                                confirmation: CloudBackupDeleteConfirmation::MayBeOnlyCopy,
                            },
                        ],
                    ),
                    row(
                        "record-unchecked",
                        CloudBackupListedRecord::Unchecked,
                        vec![CloudBackupRowActionKind::CheckAgain],
                    ),
                ],
                restore_all: CloudBackupRestoreAllState::StartAvailable { wallet_count: 1 },
                operation: CloudOnlyOperation::Idle,
            }),
            CloudBackupSection::ExtraCopies(CloudBackupExtraCopiesSection {
                rows: vec![row(
                    "record-spending-old",
                    CloudBackupListedRecord::ExtraCopy {
                        wallet: wallet(
                            "Spending",
                            "record-spending-old",
                            CloudBackupWalletStatus::AlreadyOnDevice,
                        ),
                        covered_by: "Spending".into(),
                    },
                    vec![CloudBackupRowActionKind::Delete {
                        confirmation: CloudBackupDeleteConfirmation::Standard,
                    }],
                )],
            }),
            CloudBackupSection::TurnOff,
        ]);

        sections
    }

    fn configured_body(
        passkey: CloudBackupPasskeyState,
        disable: CloudBackupDisableScreen,
    ) -> CloudBackupScreenBody {
        CloudBackupScreenBody::Configured(CloudBackupConfiguredScreen {
            sections: configured_sections(passkey),
            last_sync: Some(VERIFIED_AT),
            sync_health: CloudSyncHealth::AllUploaded,
            disable,
        })
    }

    fn configured(
        disable: CloudBackupDisableScreen,
        extra_actions: Option<&[CloudBackupScreenAction]>,
    ) -> CloudBackupScreen {
        let mut actions = configured_actions();
        actions.extend(extra_actions.unwrap_or_default());

        screen(
            configured_body(CloudBackupPasskeyState::Available, disable),
            CloudBackupSettingsRowStatus::Active,
            available(&actions),
        )
    }

    fn passkey_missing() -> CloudBackupScreen {
        let mut actions = configured_actions();
        actions.push(A::RepairPasskey);

        screen(
            configured_body(CloudBackupPasskeyState::Missing, CloudBackupDisableScreen::Idle),
            CloudBackupSettingsRowStatus::PasskeyMissing,
            available(&actions),
        )
    }

    /// After an upload, the root prompt asks to verify the backup
    fn verification_prompt() -> CloudBackupScreen {
        let mut screen = configured(CloudBackupDisableScreen::Idle, None);
        screen.root_prompt = CloudBackupRootPrompt::Verification;
        screen
    }

    /// The verification prompt is up while a wallet restore runs, so its Verify waits
    fn verification_prompt_busy() -> CloudBackupScreen {
        let mut screen = verification_prompt();
        for entry in &mut screen.actions.entries {
            if entry.action == A::Verify {
                entry.availability = CloudBackupActionAvailability::Unavailable {
                    reason: CloudBackupRejection::OperationRunning {
                        operation: CloudBackupOperationKind::RestoreCloudWallet,
                    },
                };
            }
        }

        screen
    }

    /// A wallet restore runs, so every row control waits for it
    fn rows_held() -> CloudBackupScreen {
        let mut screen = configured(CloudBackupDisableScreen::Idle, None);
        let held = CloudBackupActionAvailability::Unavailable {
            reason: CloudBackupRejection::OperationRunning {
                operation: CloudBackupOperationKind::RestoreCloudWallet,
            },
        };

        for row in configured_rows(&mut screen) {
            for action in &mut row.actions {
                action.availability = held;
            }
        }

        screen
    }

    /// The Spending wallet left this device, so its old copy may now be the only one and its
    /// delete asks for the stronger confirmation
    fn extra_copy_only_copy() -> CloudBackupScreen {
        let mut screen = extra_copy_gone();
        let only_copy = row(
            "record-spending-old",
            CloudBackupListedRecord::NotOnDevice {
                wallet: wallet(
                    "Spending",
                    "record-spending-old",
                    CloudBackupWalletStatus::Confirmed,
                ),
            },
            vec![
                CloudBackupRowActionKind::Restore,
                CloudBackupRowActionKind::Delete {
                    confirmation: CloudBackupDeleteConfirmation::MayBeOnlyCopy,
                },
            ],
        );

        let CloudBackupScreenBody::Configured(configured) = &mut screen.body else {
            return screen;
        };

        for section in &mut configured.sections {
            if let CloudBackupSection::NotOnDevice(not_on_device) = section {
                not_on_device.rows.push(only_copy.clone());
            }
        }

        screen
    }

    /// The old copy of the Spending wallet was deleted elsewhere, so no row lists it
    fn extra_copy_gone() -> CloudBackupScreen {
        let mut screen = configured(CloudBackupDisableScreen::Idle, None);
        let CloudBackupScreenBody::Configured(configured) = &mut screen.body else {
            return screen;
        };

        configured
            .sections
            .retain(|section| !matches!(section, CloudBackupSection::ExtraCopies(_)));
        screen
    }

    fn configured_rows(screen: &mut CloudBackupScreen) -> Vec<&mut CloudBackupListedRow> {
        let CloudBackupScreenBody::Configured(configured) = &mut screen.body else {
            return Vec::new();
        };

        configured
            .sections
            .iter_mut()
            .flat_map(|section| match section {
                CloudBackupSection::NotOnDevice(section) => section.rows.as_mut_slice(),
                CloudBackupSection::ExtraCopies(section) => section.rows.as_mut_slice(),
                _ => &mut [],
            })
            .collect()
    }

    fn operation_running() -> CloudBackupScreen {
        let running = CloudBackupActionAvailability::Unavailable {
            reason: CloudBackupRejection::OperationRunning {
                operation: CloudBackupOperationKind::Verify,
            },
        };

        let entries = configured_actions()
            .into_iter()
            .map(|action| CloudBackupActionEntry {
                action,
                availability: match action {
                    A::SyncUnsynced | A::RefreshDetail => CloudBackupActionAvailability::Available,
                    _ => running,
                },
            })
            .collect();

        screen(
            configured_body(CloudBackupPasskeyState::Available, CloudBackupDisableScreen::Idle),
            CloudBackupSettingsRowStatus::Active,
            CloudBackupActions { entries },
        )
    }

    fn heal_notice() -> CloudBackupScreen {
        let mut screen = configured(CloudBackupDisableScreen::Idle, Some(&[A::DismissHealNotice]));
        screen.heal_notice =
            Some(CloudBackupHealNotice::StuckTurnOffHealed { healed_at: VERIFIED_AT });
        screen.settings_row = CloudBackupSettingsRowStatus::TurnedOnAgain;
        screen
    }

    fn turn_off_review() -> CloudBackupScreen {
        let review = CloudBackupDisableReview {
            review_id: 1,
            impact: CloudBackupDisableImpact {
                assessment: CloudBackupInventoryAssessment::Partial {
                    reason: CloudBackupInventoryIncompleteReason::ProviderSyncPending,
                },
                own_backups: 2,
                not_on_device: CloudBackupNamedCount {
                    names: vec!["Cold Storage".into()],
                    more: 0,
                },
                own_secret_missing_locally: CloudBackupNamedCount::default(),
                extra_copies: 1,
                unchecked: 1,
                unreadable: 0,
                unsupported: 0,
                other_backups: 0,
                confirmation: CloudBackupDisableConfirmation::Two,
            },
            changed_since_confirmation: false,
        };

        configured(
            CloudBackupDisableScreen::Reviewing(review),
            Some(&[A::ConfirmDisable, A::CancelDisable]),
        )
    }

    fn finishing_disable() -> CloudBackupScreen {
        screen(
            CloudBackupScreenBody::FinishingDisable(CloudBackupFinishingDisableScreen {
                stage: CloudBackupFinishingDisableStage::Failed {
                    reason: CloudBackupTurnOffFailure::Offline,
                },
            }),
            CloudBackupSettingsRowStatus::TurnOffIncomplete,
            available(&[A::ResumeDisable, A::StopUsingOnThisDevice]),
        )
    }

    fn drive_account_switch() -> CloudBackupScreen {
        screen(
            CloudBackupScreenBody::DriveAccountSwitch(CloudBackupDriveAccountSwitchScreen {
                recovery: Some(CloudBackupDriveAccountSwitchRecovery {
                    transition_id: 7,
                    message: "Cloud Backup could not finish switching Google accounts".into(),
                }),
            }),
            CloudBackupSettingsRowStatus::RecoveryRequired,
            available(&[A::RetryDriveAccountSwitchRecovery, A::CancelDriveAccountSwitch]),
        )
    }

    fn pending_enable_recovery() -> CloudBackupScreen {
        screen(
            CloudBackupScreenBody::PendingEnableRecovery(CloudBackupPendingEnableRecovery {
                support_code: "CB-0000".into(),
                cleanup: CloudBackupPendingEnableCleanupState::Available,
            }),
            CloudBackupSettingsRowStatus::RecoveryRequired,
            available(&[A::DiscardPendingEnable, A::ConfirmPendingEnableCleanup]),
        )
    }

    fn load_failed() -> CloudBackupScreen {
        screen(
            CloudBackupScreenBody::LoadFailed,
            CloudBackupSettingsRowStatus::RecoveryRequired,
            available(&[A::RetryLoad]),
        )
    }

    fn corrupted() -> CloudBackupScreen {
        screen(
            CloudBackupScreenBody::Corrupted,
            CloudBackupSettingsRowStatus::RecoveryRequired,
            available(&[A::ResetOnThisDevice]),
        )
    }
}
