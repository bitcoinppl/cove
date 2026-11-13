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
        CloudBackupDriveAccountSwitchStage, CloudBackupExtraCopiesSection,
        CloudBackupFinishingDisableScreen, CloudBackupFinishingDisableStage, CloudBackupHealNotice,
        CloudBackupInventoryAssessment, CloudBackupInventoryControl, CloudBackupKeyTrust,
        CloudBackupListedRecord, CloudBackupListedRow, CloudBackupMenuControl,
        CloudBackupNamedCount, CloudBackupNotOnDeviceSection, CloudBackupOperationKind,
        CloudBackupOtherBackupsControl, CloudBackupOtherBackupsSection, CloudBackupOwnRecord,
        CloudBackupOwnRow, CloudBackupProgressSection, CloudBackupRecoveryKind,
        CloudBackupRecoveryOperation, CloudBackupRejection, CloudBackupRowAction,
        CloudBackupRowActionKind, CloudBackupSavedPasskeyControl, CloudBackupSavedPasskeyLifecycle,
        CloudBackupSavedPasskeyScreen, CloudBackupScreen, CloudBackupScreenAction,
        CloudBackupScreenBody, CloudBackupScreenFixture, CloudBackupSection,
        CloudBackupTurnOffFailure, CloudBackupUnreadableKind, CloudBackupVerificationControl,
        CloudBackupVerificationSection, CloudBackupVerificationView, CloudBackupWalletsControl,
        CloudBackupWalletsSection,
    };
    use crate::manager::cloud_backup_manager::model::CloudBackupPasskeyRepairState;
    use crate::manager::cloud_backup_manager::ops::CLOUD_BACKUP_LOAD_FAILED_MESSAGE;
    use crate::manager::cloud_backup_manager::{
        CORRUPTED_CLOUD_BACKUP_STATE_MESSAGE, CloudBackupEnableContext, CloudBackupEnableFlow,
        CloudBackupInventoryIncompleteReason, CloudBackupOtherBackupsState,
        CloudBackupOtherBackupsSummary, CloudBackupPasskeyChoiceIntent, CloudBackupPasskeyState,
        CloudBackupPendingEnableCleanupState, CloudBackupPendingEnableRecovery,
        CloudBackupRestoreAllState, CloudBackupRestoreFlow, CloudBackupRootPrompt,
        CloudBackupSettingsRowStatus, CloudBackupSyncState, CloudBackupVerificationPresentation,
        CloudBackupVerificationState, CloudBackupWalletItem, CloudBackupWalletStatus,
        CloudBackupWalletVerificationIssues, CloudOnlyOperation, DeepVerificationFailure,
        DeepVerificationReport, GENERIC_CLOUD_BACKUP_ERROR_MESSAGE, OtherBackupsOperation,
        RecordId,
    };
    use crate::manager::cloud_backup_manager::{
        CloudBackupBatchDeleteConsent, CloudBackupDeleteCategory, CloudBackupRowDeleteConsent,
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
            fixture("configured-clean", clean()),
            fixture("configured-secret-missing-locally", secret_missing_locally()),
            fixture("configured-empty-inventory", empty_inventory()),
            fixture("configured-passkey-missing", passkey_missing()),
            fixture("configured-passkey-needs-repair", passkey_needs_repair()),
            fixture("configured-verification-not-completed", verification_not_completed()),
            fixture("configured-verification-failed", verification_failed()),
            fixture("configured-verification-unsupported-version", unsupported_version()),
            fixture("configured-verification-recreate-manifest", recreate_manifest()),
            fixture("configured-verification-reinitialize-backup", reinitialize_backup()),
            fixture("configured-needs-attention-undecryptable", undecryptable()),
            fixture("configured-verification-prompt", verification_prompt()),
            fixture("configured-verification-prompt-busy", verification_prompt_busy()),
            fixture("configured-inventory-incomplete", inventory_incomplete()),
            fixture("configured-other-backups", other_backups()),
            fixture("configured-other-backups-check-failed", other_backups_check_failed()),
            fixture("configured-verifying-inventory-incomplete", verifying_inventory_incomplete()),
            fixture("configured-operation-running", operation_running()),
            fixture("configured-heal-notice", heal_notice()),
            fixture("configured-rows-held", rows_held()),
            fixture("configured-extra-copy-only-copy", extra_copy_only_copy()),
            fixture("configured-extra-copy-gone", extra_copy_gone()),
            fixture("turn-off-review", turn_off_review()),
            fixture("turn-off-review-restore-all", turn_off_review_restore_all()),
            fixture("turn-off-review-restore-one", turn_off_review_restore_one()),
            fixture("turn-off-review-restore-unavailable", turn_off_review_restore_unavailable()),
            fixture("turn-off-review-verify-first", turn_off_review_verify_first()),
            fixture("finishing-disable", finishing_disable()),
            fixture(
                "finishing-disable-authorization-required",
                finishing_disable_authorization_required(),
            ),
            fixture("drive-account-switch", drive_account_switch()),
            fixture("saved-passkey-confirmation-enable", saved_passkey_confirmation_enable()),
            fixture(
                "saved-passkey-confirmation-reinitialize",
                saved_passkey_confirmation_reinitialize(),
            ),
            fixture(
                "saved-passkey-confirmation-drive-account-switch",
                saved_passkey_confirmation_drive_account_switch(),
            ),
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
        vec![A::Verify, A::SyncUnsynced, A::SwitchDriveAccount, A::TurnOff, A::StartRestoreAll]
    }

    fn configured_sections(passkey: CloudBackupPasskeyState) -> Vec<CloudBackupSection> {
        let mut sections = Vec::new();
        if passkey != CloudBackupPasskeyState::Available {
            sections.push(CloudBackupSection::PasskeyCard(passkey));
        }

        sections.extend([
            verification_section(
                CloudBackupVerificationState::Verified {
                    report: None,
                    last_verified_at: Some(VERIFIED_AT),
                },
                vec![CloudBackupVerificationControl::Verify],
            ),
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
                controls: vec![CloudBackupWalletsControl::SyncNow],
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
                                consent: CloudBackupRowDeleteConsent::fixture(
                                    "record-cold",
                                    CloudBackupDeleteCategory::NotOnDevice,
                                ),
                            },
                        ],
                    ),
                    row(
                        "record-unchecked",
                        CloudBackupListedRecord::Unchecked,
                        vec![CloudBackupRowActionKind::Verify],
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
                        consent: CloudBackupRowDeleteConsent::fixture(
                            "record-spending-old",
                            CloudBackupDeleteCategory::ExtraCopy,
                        ),
                    }],
                )],
            }),
            CloudBackupSection::TurnOff,
        ]);

        sections
    }

    fn verification_section(
        state: CloudBackupVerificationState,
        controls: Vec<CloudBackupVerificationControl>,
    ) -> CloudBackupSection {
        CloudBackupSection::Verification(CloudBackupVerificationSection {
            view: CloudBackupVerificationView {
                state,
                presentation: CloudBackupVerificationPresentation::Hidden { source: None },
                recovery: CloudBackupRecoveryOperation::Idle,
            },
            controls,
        })
    }

    /// Android's menu, which every configured screen has
    fn menu() -> Vec<CloudBackupMenuControl> {
        vec![CloudBackupMenuControl::SwitchDriveAccount]
    }

    fn configured_screen(sections: Vec<CloudBackupSection>) -> CloudBackupScreenBody {
        CloudBackupScreenBody::Configured(CloudBackupConfiguredScreen {
            sections,
            menu: menu(),
            last_sync: Some(VERIFIED_AT),
            sync_health: CloudSyncHealth::AllUploaded,
            disable: CloudBackupDisableScreen::Idle,
            passkey_hint: None,
        })
    }

    fn configured_body(
        passkey: CloudBackupPasskeyState,
        disable: CloudBackupDisableScreen,
    ) -> CloudBackupScreenBody {
        CloudBackupScreenBody::Configured(CloudBackupConfiguredScreen {
            sections: configured_sections(passkey),
            menu: menu(),
            last_sync: Some(VERIFIED_AT),
            sync_health: CloudSyncHealth::AllUploaded,
            disable,
            passkey_hint: None,
        })
    }

    /// The actions of a configured screen with Turn Off, Android's account switch and `extra`
    fn minimal_actions(extra: &[CloudBackupScreenAction]) -> CloudBackupActions {
        let mut actions = vec![A::SwitchDriveAccount, A::TurnOff];
        actions.extend(extra);
        available(&actions)
    }

    /// A verified backup whose wallets are all up to date and that holds nothing else: Verify
    /// Again and Turn Off, with no Sync Now and no rebuild
    fn clean() -> CloudBackupScreen {
        verified_with_one_wallet(CloudBackupOwnRecord::Current)
    }

    /// A verified backup holding one up-to-date wallet, with this device's record of it
    fn verified_with_one_wallet(record: CloudBackupOwnRecord) -> CloudBackupScreen {
        let sections = vec![
            verification_section(
                CloudBackupVerificationState::Verified {
                    report: None,
                    last_verified_at: Some(VERIFIED_AT),
                },
                vec![CloudBackupVerificationControl::Verify],
            ),
            CloudBackupSection::Wallets(CloudBackupWalletsSection {
                up_to_date: vec![CloudBackupOwnRow {
                    wallet: wallet(
                        "Spending",
                        "record-spending",
                        CloudBackupWalletStatus::Confirmed,
                    ),
                    record,
                }],
                needs_sync: Vec::new(),
                sync: CloudBackupSyncState::Idle,
                controls: Vec::new(),
            }),
            CloudBackupSection::TurnOff,
        ];

        screen(
            configured_screen(sections),
            CloudBackupSettingsRowStatus::Active,
            minimal_actions(&[A::Verify]),
        )
    }

    /// A wallet up to date in Cloud Backup whose secret this device no longer holds, so its row
    /// says the cloud copy is what restores it
    fn secret_missing_locally() -> CloudBackupScreen {
        verified_with_one_wallet(CloudBackupOwnRecord::SecretMissingLocally)
    }

    /// A loaded inventory with no wallets in it: no rows, so nothing to sync
    fn empty_inventory() -> CloudBackupScreen {
        let sections = vec![
            verification_section(
                CloudBackupVerificationState::NotVerified,
                vec![CloudBackupVerificationControl::Verify],
            ),
            CloudBackupSection::Wallets(CloudBackupWalletsSection {
                up_to_date: Vec::new(),
                needs_sync: Vec::new(),
                sync: CloudBackupSyncState::Idle,
                controls: Vec::new(),
            }),
            CloudBackupSection::TurnOff,
        ];

        screen(
            configured_screen(sections),
            CloudBackupSettingsRowStatus::Active,
            minimal_actions(&[A::Verify]),
        )
    }

    /// A backup only a newer Cove can read: the failure says to update, and nothing on the
    /// card can fix it, so it draws no Verify
    fn unsupported_version() -> CloudBackupScreen {
        let failure = DeepVerificationFailure::UnsupportedVersion {
            message: "This backup was made by a newer version of Cove".into(),
            detail: None,
        };
        let sections = vec![
            verification_section(CloudBackupVerificationState::Failed(failure), Vec::new()),
            CloudBackupSection::TurnOff,
        ];

        screen(
            configured_screen(sections),
            CloudBackupSettingsRowStatus::Active,
            minimal_actions(&[]),
        )
    }

    /// A failed verification whose only recovery is `kind`, drawn on its failure card
    fn recovery(kind: CloudBackupRecoveryKind) -> CloudBackupScreen {
        let message = "Cove couldn't use the cloud backup".to_string();
        let warning =
            "This replaces what is in the cloud with the wallets on this device".to_string();
        let failure = match kind {
            CloudBackupRecoveryKind::RecreateManifest => {
                DeepVerificationFailure::RecreateManifest { message, warning, detail: None }
            }
            CloudBackupRecoveryKind::ReinitializeBackup => {
                DeepVerificationFailure::ReinitializeBackup { message, warning, detail: None }
            }
        };
        let sections = vec![
            verification_section(
                CloudBackupVerificationState::Failed(failure),
                vec![CloudBackupVerificationControl::Recover { kind }],
            ),
            CloudBackupSection::TurnOff,
        ];

        screen(
            configured_screen(sections),
            CloudBackupSettingsRowStatus::Active,
            minimal_actions(&[kind.action()]),
        )
    }

    fn recreate_manifest() -> CloudBackupScreen {
        recovery(CloudBackupRecoveryKind::RecreateManifest)
    }

    fn reinitialize_backup() -> CloudBackupScreen {
        recovery(CloudBackupRecoveryKind::ReinitializeBackup)
    }

    /// A verification that found wallet backups the verified key can't decrypt: the issue that
    /// counts them offers to delete them, above Verify Again
    fn undecryptable() -> CloudBackupScreen {
        let report = DeepVerificationReport {
            master_key_wrapper_repaired: false,
            local_master_key_repaired: false,
            credential_recovered: false,
            wallets_verified: 1,
            wallet_issues: CloudBackupWalletVerificationIssues {
                decryption_failed: 2,
                ..CloudBackupWalletVerificationIssues::default()
            },
            detail: None,
        };
        let sections = vec![
            verification_section(
                CloudBackupVerificationState::NeedsAttention {
                    report,
                    checked_at: Some(VERIFIED_AT),
                },
                vec![
                    CloudBackupVerificationControl::DeleteUnreadableBackups {
                        consent: CloudBackupBatchDeleteConsent::fixture(&[
                            "record-undecryptable-1",
                            "record-undecryptable-2",
                        ]),
                    },
                    CloudBackupVerificationControl::Verify,
                ],
            ),
            CloudBackupSection::NotOnDevice(CloudBackupNotOnDeviceSection {
                rows: ["record-undecryptable-1", "record-undecryptable-2"]
                    .into_iter()
                    .map(|record_id| {
                        row(
                            record_id,
                            CloudBackupListedRecord::Unreadable {
                                kind: CloudBackupUnreadableKind::Decrypt,
                                trust: CloudBackupKeyTrust::Unverified,
                            },
                            vec![
                                CloudBackupRowActionKind::Verify,
                                CloudBackupRowActionKind::CheckAgain,
                            ],
                        )
                    })
                    .collect(),
                restore_all: CloudBackupRestoreAllState::NotShown,
                operation: CloudOnlyOperation::Idle,
            }),
            CloudBackupSection::TurnOff,
        ];

        let actions = minimal_actions(&[A::DeleteUnreadableBackups, A::Verify]);
        screen(configured_screen(sections), CloudBackupSettingsRowStatus::Active, actions)
    }

    /// Backups made with another passkey: Try Another Passkey and Delete These Backups
    fn other_backups() -> CloudBackupScreen {
        let summary = CloudBackupOtherBackupsSummary {
            namespace_count: 1,
            wallet_count: 2,
            passkey_hints: Vec::new(),
        };
        let section = other_backups_section(
            CloudBackupOtherBackupsState::Loaded { summary },
            vec![CloudBackupOtherBackupsControl::Recover, CloudBackupOtherBackupsControl::Delete],
        );

        let actions = minimal_actions(&[A::Verify, A::RecoverOtherBackups, A::DeleteOtherBackups]);
        screen(configured_screen(section), CloudBackupSettingsRowStatus::Active, actions)
    }

    /// A check for other backups that failed, with Check Again on its section
    fn other_backups_check_failed() -> CloudBackupScreen {
        let section = other_backups_section(
            CloudBackupOtherBackupsState::LoadFailed {
                reason: CloudBackupInventoryIncompleteReason::Offline,
            },
            vec![CloudBackupOtherBackupsControl::CheckAgain],
        );

        let actions = minimal_actions(&[A::Verify, A::RefreshDetail]);
        screen(configured_screen(section), CloudBackupSettingsRowStatus::Active, actions)
    }

    /// A verified backup's sections with an other-backups section in `state`
    fn other_backups_section(
        state: CloudBackupOtherBackupsState,
        controls: Vec<CloudBackupOtherBackupsControl>,
    ) -> Vec<CloudBackupSection> {
        vec![
            verification_section(
                CloudBackupVerificationState::Verified {
                    report: None,
                    last_verified_at: Some(VERIFIED_AT),
                },
                vec![CloudBackupVerificationControl::Verify],
            ),
            CloudBackupSection::OtherBackups(CloudBackupOtherBackupsSection {
                state,
                operation: OtherBackupsOperation::Idle,
                controls,
            }),
            CloudBackupSection::TurnOff,
        ]
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

    /// A verification failed on the passkey: its card offers Add New Passkey beside Verify
    fn passkey_needs_repair() -> CloudBackupScreen {
        let mut actions = configured_actions();
        actions.push(A::RepairPasskey);

        let passkey =
            CloudBackupPasskeyState::NeedsRepair { state: CloudBackupPasskeyRepairState::Idle };
        let mut screen = screen(
            configured_body(passkey, CloudBackupDisableScreen::Idle),
            CloudBackupSettingsRowStatus::PasskeyMissing,
            available(&actions),
        );

        set_verification_state(&mut screen, CloudBackupVerificationState::NotVerified);
        screen
    }

    /// The user dismissed the passkey sheet: the outcome shows beside Verify, with no repair
    fn verification_not_completed() -> CloudBackupScreen {
        let mut screen = configured(CloudBackupDisableScreen::Idle, None);
        set_verification_state(&mut screen, CloudBackupVerificationState::NotCompleted);
        screen
    }

    /// A verification failed off the passkey, such as offline: Verify again, with no repair
    fn verification_failed() -> CloudBackupScreen {
        let failure =
            DeepVerificationFailure::retry(GENERIC_CLOUD_BACKUP_ERROR_MESSAGE, None, None);
        let mut screen = configured(CloudBackupDisableScreen::Idle, None);
        set_verification_state(&mut screen, CloudBackupVerificationState::Failed(failure));
        screen
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

    /// Shows `state` in place of the verified one; each state these fixtures use draws Verify
    fn set_verification_state(screen: &mut CloudBackupScreen, state: CloudBackupVerificationState) {
        let CloudBackupScreenBody::Configured(configured) = &mut screen.body else { return };
        for section in &mut configured.sections {
            if let CloudBackupSection::Verification(verification) = section {
                verification.view.state = state.clone();
            }
        }
    }

    /// The inventory check failed, so its section offers Check Again above the rows it kept
    fn inventory_incomplete() -> CloudBackupScreen {
        let mut screen = configured(CloudBackupDisableScreen::Idle, Some(&[A::RefreshDetail]));
        if let CloudBackupScreenBody::Configured(configured) = &mut screen.body {
            configured.sections.insert(
                0,
                CloudBackupSection::Progress(CloudBackupProgressSection::InventoryIncomplete {
                    reason: CloudBackupInventoryIncompleteReason::ProviderSyncPending,
                    controls: vec![CloudBackupInventoryControl::CheckAgain],
                }),
            );
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
                    consent: CloudBackupRowDeleteConsent::fixture(
                        "record-spending-old",
                        CloudBackupDeleteCategory::NotOnDevice,
                    ),
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
                    A::SyncUnsynced => CloudBackupActionAvailability::Available,
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

    /// Verify runs while the inventory check is still incomplete: the verification holds the one
    /// progress section, inline above the rows, and the incomplete section's Check Again waits
    /// until it ends
    fn verifying_inventory_incomplete() -> CloudBackupScreen {
        let mut screen = configured(CloudBackupDisableScreen::Idle, None);
        screen.actions.entries.retain(|entry| entry.action != A::Verify);

        let CloudBackupScreenBody::Configured(configured) = &mut screen.body else {
            return screen;
        };

        let mut running = None;
        for section in &mut configured.sections {
            if let CloudBackupSection::Verification(verification) = section {
                verification.view.state = CloudBackupVerificationState::Running;
                verification.controls.clear();
                running = Some(verification.view.clone());
            }
        }

        if let Some(view) = running {
            configured.sections.insert(
                0,
                CloudBackupSection::Progress(CloudBackupProgressSection::VerificationInline(view)),
            );
        }

        screen
    }

    fn heal_notice() -> CloudBackupScreen {
        let mut screen = configured(CloudBackupDisableScreen::Idle, Some(&[A::DismissHealNotice]));
        screen.heal_notice =
            Some(CloudBackupHealNotice::StuckTurnOffHealed { healed_at: VERIFIED_AT });
        screen.settings_row = CloudBackupSettingsRowStatus::TurnedOnAgain;
        screen
    }

    /// A review that asks twice for `not_on_device` and `unchecked`, from a complete inventory
    fn review_impact(not_on_device: &[&str], unchecked: u32) -> CloudBackupDisableImpact {
        CloudBackupDisableImpact {
            assessment: CloudBackupInventoryAssessment::Complete,
            own_backups: 2,
            not_on_device: CloudBackupNamedCount {
                names: not_on_device.iter().map(|&name| name.into()).collect(),
                more: 0,
            },
            own_secret_missing_locally: CloudBackupNamedCount::default(),
            extra_copies: 0,
            unchecked,
            unreadable: 0,
            unsupported: 0,
            other_backups: 0,
            confirmation: CloudBackupDisableConfirmation::Two,
        }
    }

    fn reviewing(impact: CloudBackupDisableImpact, leave: &[A]) -> CloudBackupScreen {
        let review =
            CloudBackupDisableReview { review_id: 1, impact, changed_since_confirmation: false };
        let mut actions = vec![A::ConfirmDisable, A::CancelDisable];
        actions.extend(leave);

        configured(CloudBackupDisableScreen::Reviewing(review), Some(&actions))
    }

    fn turn_off_review() -> CloudBackupScreen {
        let impact = CloudBackupDisableImpact {
            assessment: CloudBackupInventoryAssessment::Partial {
                reason: CloudBackupInventoryIncompleteReason::ProviderSyncPending,
            },
            extra_copies: 1,
            ..review_impact(&["Cold Storage"], 1)
        };

        reviewing(impact, &[A::RestoreFirst, A::VerifyFirst])
    }

    /// Restore Them First restores every wallet not on this device through Restore All
    fn turn_off_review_restore_all() -> CloudBackupScreen {
        reviewing(review_impact(&["Cold Storage", "Spending"], 0), &[A::RestoreFirst])
    }

    /// Restore Them First restores the one wallet not on this device
    fn turn_off_review_restore_one() -> CloudBackupScreen {
        reviewing(review_impact(&["Cold Storage"], 0), &[A::RestoreFirst])
    }

    /// The listing is incomplete, so Restore All can't start and Restore Them First says why
    fn turn_off_review_restore_unavailable() -> CloudBackupScreen {
        let impact = CloudBackupDisableImpact {
            assessment: CloudBackupInventoryAssessment::Partial {
                reason: CloudBackupInventoryIncompleteReason::ProviderSyncPending,
            },
            ..review_impact(&["Cold Storage", "Spending"], 0)
        };

        let mut screen = reviewing(impact, &[A::RestoreFirst]);
        let unavailable = CloudBackupActionAvailability::Unavailable {
            reason: CloudBackupRejection::RestoreAllUnavailable,
        };
        for entry in &mut screen.actions.entries {
            if matches!(entry.action, A::RestoreFirst | A::StartRestoreAll) {
                entry.availability = unavailable;
            }
        }

        if let CloudBackupScreenBody::Configured(configured) = &mut screen.body {
            for section in &mut configured.sections {
                if let CloudBackupSection::NotOnDevice(not_on_device) = section {
                    not_on_device.restore_all =
                        CloudBackupRestoreAllState::StartDisabled { wallet_count: 2 };
                }
            }
        }

        screen
    }

    /// Verify First verifies the backups the review couldn't check
    fn turn_off_review_verify_first() -> CloudBackupScreen {
        reviewing(review_impact(&[], 2), &[A::VerifyFirst])
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

    /// Google Drive stopped the last run for consent, so Reconnect resumes it
    fn finishing_disable_authorization_required() -> CloudBackupScreen {
        screen(
            CloudBackupScreenBody::FinishingDisable(CloudBackupFinishingDisableScreen {
                stage: CloudBackupFinishingDisableStage::Failed {
                    reason: CloudBackupTurnOffFailure::AuthorizationRequired,
                },
            }),
            CloudBackupSettingsRowStatus::TurnOffIncomplete,
            available(&[A::ResumeDisable, A::ReconnectDrive, A::StopUsingOnThisDevice]),
        )
    }

    fn saved_passkey_confirmation(
        lifecycle: CloudBackupSavedPasskeyLifecycle,
        controls: &[CloudBackupSavedPasskeyControl],
        settings_row: CloudBackupSettingsRowStatus,
    ) -> CloudBackupScreen {
        let actions: Vec<_> = controls.iter().map(|control| control.action()).collect();

        screen(
            CloudBackupScreenBody::SavedPasskeyConfirmation(CloudBackupSavedPasskeyScreen {
                lifecycle,
                title: "Confirm your passkey".into(),
                message: "Your passkey was saved. Confirm it once to go on.".into(),
                controls: controls.to_vec(),
            }),
            settings_row,
            available(&actions),
        )
    }

    fn saved_passkey_confirmation_enable() -> CloudBackupScreen {
        saved_passkey_confirmation(
            CloudBackupSavedPasskeyLifecycle::Enable,
            &[CloudBackupSavedPasskeyControl::Confirm, CloudBackupSavedPasskeyControl::Discard],
            CloudBackupSettingsRowStatus::SettingUp,
        )
    }

    fn saved_passkey_confirmation_reinitialize() -> CloudBackupScreen {
        saved_passkey_confirmation(
            CloudBackupSavedPasskeyLifecycle::Reinitialize,
            &[CloudBackupSavedPasskeyControl::Confirm, CloudBackupSavedPasskeyControl::Discard],
            CloudBackupSettingsRowStatus::Active,
        )
    }

    /// A switch's reinitialize can't be discarded, so Confirm is its only control
    fn saved_passkey_confirmation_drive_account_switch() -> CloudBackupScreen {
        saved_passkey_confirmation(
            CloudBackupSavedPasskeyLifecycle::DriveAccountSwitch,
            &[CloudBackupSavedPasskeyControl::Confirm],
            CloudBackupSettingsRowStatus::Active,
        )
    }

    fn drive_account_switch() -> CloudBackupScreen {
        screen(
            CloudBackupScreenBody::DriveAccountSwitch(CloudBackupDriveAccountSwitchScreen {
                stage: CloudBackupDriveAccountSwitchStage::NeedsAttention(
                    CloudBackupDriveAccountSwitchRecovery {
                        message: "Cloud Backup could not finish switching Google accounts".into(),
                    },
                ),
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
            available(&[A::ConfirmPendingEnableCleanup]),
        )
    }

    fn load_failed() -> CloudBackupScreen {
        screen(
            CloudBackupScreenBody::LoadFailed,
            CloudBackupSettingsRowStatus::Error(CLOUD_BACKUP_LOAD_FAILED_MESSAGE.into()),
            available(&[A::RetryLoad]),
        )
    }

    fn corrupted() -> CloudBackupScreen {
        screen(
            CloudBackupScreenBody::Corrupted,
            CloudBackupSettingsRowStatus::Error(CORRUPTED_CLOUD_BACKUP_STATE_MESSAGE.into()),
            available(&[A::ResetOnThisDevice]),
        )
    }
}
