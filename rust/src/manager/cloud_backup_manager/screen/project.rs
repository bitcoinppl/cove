//! The projection of the supervisor's runtime and facts onto the exported screen
//!
//! Which body is on screen comes from the runtime, and every control's availability from the
//! same [`AdmissionSnapshot`] dispatch decides on, which the supervisor takes once per publish.
//! The reducer is read only for presentation content it is the one writer of: the Enable and
//! restore flows, the passkey, verification and sync state, the root prompt and the progress of
//! row and other-backup work. It never decides a body or an action

use std::collections::HashSet;

use cove_device::cloud_storage::CloudSyncHealth;

use super::controls::drawn_controls;
use super::notices::Notices;
use super::types::{
    CloudBackupActionAvailability, CloudBackupActionEntry, CloudBackupActions,
    CloudBackupConfiguredScreen, CloudBackupDeleteConfirmation, CloudBackupDisableScreen,
    CloudBackupDriveAccountSwitchRecovery, CloudBackupDriveAccountSwitchScreen,
    CloudBackupDriveAccountSwitchStage, CloudBackupExtraCopiesSection,
    CloudBackupFinishingDisableScreen, CloudBackupInventoryControl, CloudBackupKeyTrust,
    CloudBackupListedRecord, CloudBackupListedRow, CloudBackupMenuControl,
    CloudBackupNotOnDeviceSection, CloudBackupOperationKind, CloudBackupOtherBackupsControl,
    CloudBackupOtherBackupsSection, CloudBackupProgressSection, CloudBackupRecoveryKind,
    CloudBackupRecoveryOperation, CloudBackupRejection, CloudBackupRowAction,
    CloudBackupRowActionKind, CloudBackupScreen, CloudBackupScreenAction, CloudBackupScreenBody,
    CloudBackupSection, CloudBackupUnreadableKind, CloudBackupVerificationControl,
    CloudBackupVerificationSection, CloudBackupVerificationView, CloudBackupWalletsControl,
    CloudBackupWalletsSection,
};
use crate::manager::cloud_backup_manager::actors::supervisor::Runtime;
use crate::manager::cloud_backup_manager::admission::{
    Admission, AdmissionBody, AdmissionSnapshot, CoalescibleIntent, ContinuationOwner,
    ContinuationStage, EnableIntent, Rejection, ReviewExit, SwitchAdmission, UserIntent,
    evaluate_admission,
};
use crate::manager::cloud_backup_manager::inventory::{
    InventoryFacts, InventoryProgress, InventoryRows, InventoryView, ListedRecord,
};
use crate::manager::cloud_backup_manager::ops::CLOUD_BACKUP_LOAD_FAILED_MESSAGE;
use crate::manager::cloud_backup_manager::{
    CORRUPTED_CLOUD_BACKUP_STATE_MESSAGE, CloudBackupConfiguredState, CloudBackupEnableContext,
    CloudBackupEnableFlow, CloudBackupEnablePromptChoice, CloudBackupExclusiveOperation,
    CloudBackupInventoryIncompleteReason, CloudBackupNoticeKind, CloudBackupOtherBackupsState,
    CloudBackupPasskeyState, CloudBackupRestoreAllState, CloudBackupRestoreFlow,
    CloudBackupRootPrompt, CloudBackupSettingsRowStatus, CloudBackupStateReducer,
    CloudBackupSyncState, CloudBackupVerificationSource, CloudBackupVerificationState,
    DeepVerificationFailure,
};

/// The screen for the runtime, its admission snapshot, the presentation, the inventory and the
/// notices as they stand
///
/// Pure: the same inputs give the same screen, so the harness can compare a fresh projection with
/// what the platform was sent
pub(crate) fn project(
    runtime: &Runtime,
    admission: &AdmissionSnapshot,
    presentation: &CloudBackupStateReducer,
    inventory: &InventoryFacts,
    notices: &Notices,
) -> CloudBackupScreen {
    let body = body(runtime, admission, presentation, inventory);
    let settings_row = settings_row(&body, notices, presentation);
    let root_prompt = match &body {
        CloudBackupScreenBody::Enabling(_) | CloudBackupScreenBody::Configured(_) => {
            presentation.root_prompt()
        }
        _ => CloudBackupRootPrompt::None,
    };
    let actions = global_actions(&body, &root_prompt, notices, admission);

    CloudBackupScreen {
        body,
        settings_row,
        heal_notice: notices.heal,
        last_outcome: notices.last_outcome.clone(),
        root_prompt,
        actions,
    }
}

/// The body the runtime is in, drawn with the reducer's content for it
fn body(
    runtime: &Runtime,
    admission: &AdmissionSnapshot,
    presentation: &CloudBackupStateReducer,
    inventory: &InventoryFacts,
) -> CloudBackupScreenBody {
    use AdmissionBody as B;

    // a switch step that failed needs the user whatever the record says, since a failed
    // finalize comes after the record is gone
    if let Some(recovery) = presentation.drive_account_switch_recovery() {
        return CloudBackupScreenBody::DriveAccountSwitch(CloudBackupDriveAccountSwitchScreen {
            stage: CloudBackupDriveAccountSwitchStage::NeedsAttention(recovery),
        });
    }

    match admission.body {
        B::Disabled => match presentation.enable_flow() {
            Some(flow) if enable_waits_on_user(flow, admission) => {
                CloudBackupScreenBody::Enabling(flow.clone())
            }
            _ => CloudBackupScreenBody::Disabled,
        },
        // the wipe leaves Cloud Backup off, and every control says why it can't run meanwhile
        B::Resetting => CloudBackupScreenBody::Disabled,
        B::Enabling => enabling(presentation),
        B::Restoring => restoring(presentation),
        B::PendingEnableRecovery => match admission.running {
            Some(operation) if is_enable(operation) => enabling(presentation),
            Some(CloudBackupExclusiveOperation::Restore) => restoring(presentation),
            // the admission body came from this runtime, so it holds the recovery
            _ => runtime.pending_enable_recovery().cloned().map_or(
                CloudBackupScreenBody::Disabled,
                CloudBackupScreenBody::PendingEnableRecovery,
            ),
        },
        B::DriveAccountSwitch => {
            CloudBackupScreenBody::DriveAccountSwitch(CloudBackupDriveAccountSwitchScreen {
                stage: drive_account_switch_stage(admission),
            })
        }
        B::Configured => CloudBackupScreenBody::Configured(configured_screen(
            runtime,
            admission,
            presentation,
            inventory,
        )),
        B::FinishingDisable => {
            CloudBackupScreenBody::FinishingDisable(CloudBackupFinishingDisableScreen {
                stage: runtime.finishing_disable_stage(),
            })
        }
        B::LoadFailed => CloudBackupScreenBody::LoadFailed,
        B::Corrupted => CloudBackupScreenBody::Corrupted,
    }
}

/// An Enable that stopped for the user with no operation running: a prompt the reducer holds,
/// or a saved passkey the supervisor's paused Enable waits to have confirmed
fn enable_waits_on_user(flow: &CloudBackupEnableFlow, admission: &AdmissionSnapshot) -> bool {
    match flow {
        CloudBackupEnableFlow::AwaitingForceNewConfirmation(..)
        | CloudBackupEnableFlow::AwaitingPasskeyChoice(_) => true,
        CloudBackupEnableFlow::AwaitingSavedPasskeyConfirmation(_) => admission
            .continuation
            .is_some_and(|paused| paused.stage == ContinuationStage::AwaitingSavedPasskey),
        CloudBackupEnableFlow::DiscoveringExistingBackup
        | CloudBackupEnableFlow::CreatingPasskey
        | CloudBackupEnableFlow::ConfirmingSavedPasskey
        | CloudBackupEnableFlow::UploadingInitialBackup { .. }
        | CloudBackupEnableFlow::RetryingUploadWithStagedMaterial { .. }
        | CloudBackupEnableFlow::WaitingForPasskeyAvailability => false,
    }
}

/// A recorded switch with no failed step: paused on its saved passkey while its reinitialize
/// holds a continuation and nothing runs, stopped when admission says it needs the user anyway,
/// otherwise moving on its own
fn drive_account_switch_stage(admission: &AdmissionSnapshot) -> CloudBackupDriveAccountSwitchStage {
    let paused = admission.running.is_none()
        && admission.continuation.is_some_and(|paused| {
            matches!(paused.owner, ContinuationOwner::DriveAccountSwitch(_))
                && paused.stage == ContinuationStage::AwaitingSavedPasskey
        });
    if paused {
        return CloudBackupDriveAccountSwitchStage::AwaitingSavedPasskeyConfirmation;
    }

    match admission.drive_account_switch {
        SwitchAdmission::NeedsAttention { .. } => {
            CloudBackupDriveAccountSwitchStage::NeedsAttention(
                CloudBackupDriveAccountSwitchRecovery {
                    message: DRIVE_ACCOUNT_SWITCH_STOPPED_MESSAGE.into(),
                },
            )
        }
        SwitchAdmission::None => CloudBackupDriveAccountSwitchStage::Switching,
    }
}

const DRIVE_ACCOUNT_SWITCH_STOPPED_MESSAGE: &str =
    "Cloud Backup stopped while changing Google accounts. Try again to finish.";

fn is_enable(operation: CloudBackupExclusiveOperation) -> bool {
    use CloudBackupExclusiveOperation as Op;

    matches!(operation, Op::Enable | Op::EnableForceNew | Op::EnableNoDiscovery)
}

/// A running Enable, with the step the reducer last heard of, or the first one before it has
fn enabling(presentation: &CloudBackupStateReducer) -> CloudBackupScreenBody {
    let flow = presentation
        .enable_flow()
        .cloned()
        .unwrap_or(CloudBackupEnableFlow::DiscoveringExistingBackup);

    CloudBackupScreenBody::Enabling(flow)
}

/// A running restore, with the progress the reducer last heard of, or the first step before it has
fn restoring(presentation: &CloudBackupStateReducer) -> CloudBackupScreenBody {
    let flow = presentation.restore_flow().cloned().unwrap_or(CloudBackupRestoreFlow::Finding);
    CloudBackupScreenBody::Restoring(flow)
}

fn configured_screen(
    runtime: &Runtime,
    admission: &AdmissionSnapshot,
    presentation: &CloudBackupStateReducer,
    inventory: &InventoryFacts,
) -> CloudBackupConfiguredScreen {
    let configured = presentation.configured_presentation();
    let inventory = inventory.view();
    let view = CloudBackupVerificationView {
        state: configured.verification.clone(),
        presentation: configured.verification_presentation.clone(),
        recovery: recovery_operation(admission.running),
    };

    let mut sections = Vec::new();
    if configured.passkey != CloudBackupPasskeyState::Available {
        sections.push(CloudBackupSection::PasskeyCard(configured.passkey.clone()));
    }

    // the screen has one progress section, and a running verification takes it over the
    // inventory's: inline above rows it will update, or a card in their place
    let rows_visible = inventory.rows.as_ref().is_some_and(InventoryRows::has_rows);
    let progress = match configured.verification {
        CloudBackupVerificationState::Running if rows_visible => {
            Some(CloudBackupProgressSection::VerificationInline(view.clone()))
        }
        CloudBackupVerificationState::Running => {
            Some(CloudBackupProgressSection::VerificationCard(view.clone()))
        }
        _ => inventory.progress.map(|progress| inventory_progress(progress, &configured)),
    };

    if let Some(progress) = progress {
        sections.push(CloudBackupSection::Progress(progress));
    }

    let controls = verification_controls(&configured);
    sections
        .push(CloudBackupSection::Verification(CloudBackupVerificationSection { view, controls }));

    let last_sync = inventory.rows.as_ref().and_then(|rows| rows.last_sync);
    if let Some(rows) = &inventory.rows {
        push_inventory_sections(&mut sections, rows, &inventory, &configured, admission);
    }

    if inventory.other_backups != CloudBackupOtherBackupsState::NotChecked {
        let controls = other_backups_controls(&inventory.other_backups);
        sections.push(CloudBackupSection::OtherBackups(CloudBackupOtherBackupsSection {
            state: inventory.other_backups.clone(),
            operation: inventory.other_backups_operation.clone(),
            controls,
        }));
    }

    sections.push(CloudBackupSection::TurnOff);

    CloudBackupConfiguredScreen {
        sections,
        menu: menu(),
        last_sync,
        sync_health: configured.sync_health,
        disable: runtime.disable_screen(),
    }
}

/// The inventory's loading or failed section, with Reconnect where Drive access is what failed
fn inventory_progress(
    progress: InventoryProgress,
    configured: &CloudBackupConfiguredState,
) -> CloudBackupProgressSection {
    use CloudBackupInventoryControl as C;

    let reason = match progress {
        InventoryProgress::Checking => return CloudBackupProgressSection::InventoryChecking,
        InventoryProgress::Incomplete(reason) => reason,
    };

    let reconnect = reason == CloudBackupInventoryIncompleteReason::AuthorizationRequired
        || drive_authorization_required(configured);
    let controls =
        if reconnect { vec![C::ReconnectDrive, C::CheckAgain] } else { vec![C::CheckAgain] };

    CloudBackupProgressSection::InventoryIncomplete { reason, controls }
}

/// The controls the verification section draws for the state it shows
///
/// A failure names the one recovery it allows: another verification, or a rebuild; a backup only
/// a newer Cove can read has none here
fn verification_controls(
    configured: &CloudBackupConfiguredState,
) -> Vec<CloudBackupVerificationControl> {
    use CloudBackupVerificationControl as C;
    use CloudBackupVerificationState as S;

    match &configured.verification {
        S::NotVerified | S::Required | S::Verified { .. } | S::NotCompleted => vec![C::Verify],
        S::NeedsAttention { report, .. } if report.wallet_issues.decryption_failed > 0 => {
            vec![C::DeleteUnreadableBackups, C::Verify]
        }
        S::NeedsAttention { .. } => vec![C::Verify],
        // the progress section draws a running verification
        S::Running => Vec::new(),
        // the wait for the upload's confirmation offers what unblocks it, if anything does
        S::AwaitingUploadConfirmation => {
            let retry = matches!(configured.sync, CloudBackupSyncState::Failed(_));
            let reconnect = drive_authorization_required(configured);
            [(retry, C::RetryUpload), (reconnect, C::ReconnectDrive)]
                .into_iter()
                .filter_map(|(drawn, control)| drawn.then_some(control))
                .collect()
        }
        S::Failed(failure) => match failure {
            DeepVerificationFailure::Retry { .. } => vec![C::Verify],
            DeepVerificationFailure::RecreateManifest { .. } => {
                vec![C::Recover { kind: CloudBackupRecoveryKind::RecreateManifest }]
            }
            DeepVerificationFailure::ReinitializeBackup { .. } => {
                vec![C::Recover { kind: CloudBackupRecoveryKind::ReinitializeBackup }]
            }
            // the backup is newer than this Cove: verifying again can't read it either
            DeepVerificationFailure::UnsupportedVersion { .. } => Vec::new(),
        },
    }
}

/// Whether Google Drive has asked to be authorized again; the iCloud provider never reports it
fn drive_authorization_required(configured: &CloudBackupConfiguredState) -> bool {
    matches!(configured.sync_health, CloudSyncHealth::AuthorizationRequired(_))
}

/// Recover and Delete while there are other backups to act on, and Check Again after a check
/// that failed
fn other_backups_controls(
    state: &CloudBackupOtherBackupsState,
) -> Vec<CloudBackupOtherBackupsControl> {
    use CloudBackupOtherBackupsControl as C;

    match state {
        CloudBackupOtherBackupsState::Loaded { summary } if summary.namespace_count > 0 => {
            vec![C::Recover, C::Delete]
        }
        CloudBackupOtherBackupsState::LoadFailed { .. } => vec![C::CheckAgain],
        CloudBackupOtherBackupsState::NotChecked
        | CloudBackupOtherBackupsState::Checking
        | CloudBackupOtherBackupsState::Loaded { .. } => Vec::new(),
    }
}

/// Android's top-bar menu: the account switch, for any configured backup
///
/// A rebuild is not a menu entry: on a backup that verifies, recreating the index or
/// reinitializing replaces what the cloud holds with this device's wallets, so it is offered only
/// as the recovery a failed verification names
fn menu() -> Vec<CloudBackupMenuControl> {
    vec![CloudBackupMenuControl::SwitchDriveAccount]
}

/// The wallets, Not on This Device and Extra Copies sections, in that order
fn push_inventory_sections(
    sections: &mut Vec<CloudBackupSection>,
    rows: &InventoryRows,
    inventory: &InventoryView,
    configured: &CloudBackupConfiguredState,
    admission: &AdmissionSnapshot,
) {
    let (extra_copies, not_on_device): (Vec<_>, Vec<_>) = rows
        .listed
        .iter()
        .map(|listed| listed_row(listed, rows.copy_has_current_backup(listed), admission))
        .partition(|row| matches!(row.record, CloudBackupListedRecord::ExtraCopy { .. }));

    // the list of wallets waiting to upload offers to upload them
    let controls = if rows.needs_sync.is_empty() {
        Vec::new()
    } else {
        vec![CloudBackupWalletsControl::SyncNow]
    };

    sections.push(CloudBackupSection::Wallets(CloudBackupWalletsSection {
        up_to_date: rows.up_to_date.clone(),
        needs_sync: rows.needs_sync.clone(),
        sync: configured.sync.clone(),
        controls,
    }));

    if !not_on_device.is_empty() || inventory.restore_all != CloudBackupRestoreAllState::NotShown {
        sections.push(CloudBackupSection::NotOnDevice(CloudBackupNotOnDeviceSection {
            rows: not_on_device,
            restore_all: inventory.restore_all.clone(),
            operation: inventory.cloud_only_operation.clone(),
        }));
    }

    if !extra_copies.is_empty() {
        sections.push(CloudBackupSection::ExtraCopies(CloudBackupExtraCopiesSection {
            rows: extra_copies,
        }));
    }
}

/// The rebuild a verification recovery runs, read from the running operation
fn recovery_operation(
    running: Option<CloudBackupExclusiveOperation>,
) -> CloudBackupRecoveryOperation {
    use CloudBackupExclusiveOperation as Op;

    match running {
        Some(Op::RecreateManifest) => CloudBackupRecoveryOperation::RecreatingManifest,
        Some(Op::ReinitializeBackup | Op::VerifyReinitializedBackup) => {
            CloudBackupRecoveryOperation::ReinitializingBackup
        }
        _ => CloudBackupRecoveryOperation::Idle,
    }
}

/// A listed row with the controls its kind allows; `copy_replaced` says a wallet an
/// extra copy copies has its own current backup
fn listed_row(
    listed: &ListedRecord,
    copy_replaced: bool,
    admission: &AdmissionSnapshot,
) -> CloudBackupListedRow {
    use CloudBackupRowActionKind as R;

    let ListedRecord { record_id, record, .. } = listed.clone();
    let restore = UserIntent::RestoreCloudWallet(record_id.clone());
    let delete = UserIntent::DeleteCloudWallet(record_id.clone());
    let only_copy_delete = R::Delete { confirmation: CloudBackupDeleteConfirmation::MayBeOnlyCopy };
    let verify = UserIntent::StartVerification {
        source: CloudBackupVerificationSource::CloudBackupDetail,
        discoverable: false,
    };

    let controls = match &record {
        CloudBackupListedRecord::NotOnDevice { .. } => {
            vec![(R::Restore, restore), (only_copy_delete, delete)]
        }
        // an extra copy may go only once a wallet it copies has its own current backup
        CloudBackupListedRecord::ExtraCopy { .. } if copy_replaced => {
            vec![(R::Delete { confirmation: CloudBackupDeleteConfirmation::Standard }, delete)]
        }
        CloudBackupListedRecord::ExtraCopy { .. } => {
            vec![
                (R::BackUpFirst, UserIntent::SyncUnsynced),
                (R::CheckAgain, UserIntent::RefreshDetail),
            ]
        }
        CloudBackupListedRecord::Unsupported => vec![(only_copy_delete, delete)],
        CloudBackupListedRecord::Unreadable {
            kind: CloudBackupUnreadableKind::Decrypt | CloudBackupUnreadableKind::Parse,
            trust: CloudBackupKeyTrust::VerifiedCurrentKey,
        } => vec![(only_copy_delete, delete), (R::CheckAgain, UserIntent::RefreshDetail)],
        CloudBackupListedRecord::Unreadable {
            kind: CloudBackupUnreadableKind::Decrypt | CloudBackupUnreadableKind::Parse,
            trust: CloudBackupKeyTrust::Unverified,
        } => vec![(R::Verify, verify), (R::CheckAgain, UserIntent::RefreshDetail)],
        CloudBackupListedRecord::Unreadable {
            kind: CloudBackupUnreadableKind::NotDownloadable | CloudBackupUnreadableKind::Provider,
            ..
        } => vec![(R::CheckAgain, UserIntent::RefreshDetail)],
        CloudBackupListedRecord::Unchecked => vec![(R::Verify, verify)],
    };

    let actions = controls
        .into_iter()
        .map(|(action, intent)| CloudBackupRowAction {
            action,
            availability: availability(&intent, admission),
        })
        .collect();

    CloudBackupListedRow { record_id, record, actions }
}

/// One entry for each action the body, its sections, the root prompt and the heal notice draw,
/// with what admission decides for it
fn global_actions(
    body: &CloudBackupScreenBody,
    root_prompt: &CloudBackupRootPrompt,
    notices: &Notices,
    admission: &AdmissionSnapshot,
) -> CloudBackupActions {
    let mut seen = HashSet::new();
    let entries = drawn_controls(body, root_prompt, notices.heal.as_ref())
        .into_iter()
        .map(|drawn| drawn.action)
        .filter(|action| seen.insert(*action))
        .map(|action| CloudBackupActionEntry {
            action,
            availability: availability(&intent_for(action, admission), admission),
        })
        .collect();

    CloudBackupActions { entries }
}

/// The intent a control dispatches, as admission sees it
///
/// Payloads that don't change admission take a fixed value: a verification source decides what
/// the started work does, not whether it may start. A prompt choice and a review id do decide, so
/// each Enable prompt choice has its own action, and Confirm carries the review under review
fn intent_for(action: CloudBackupScreenAction, admission: &AdmissionSnapshot) -> UserIntent {
    use CloudBackupScreenAction as A;
    use UserIntent as I;

    match action {
        A::Enable => I::Enable(EnableIntent::Standard, CloudBackupEnableContext::settings_manual()),
        A::RestoreFromCloudBackup => I::RestoreFromCloudBackup,
        A::CancelRestore => I::CancelRestore,
        A::ConfirmSavedPasskey => I::ConfirmSavedPasskey,
        A::AcceptEnablePromptUseExisting => {
            I::AcceptEnablePrompt(CloudBackupEnablePromptChoice::UseExisting)
        }
        A::AcceptEnablePromptCreateNew => {
            I::AcceptEnablePrompt(CloudBackupEnablePromptChoice::CreateNew)
        }
        A::DiscardPendingEnable => I::DiscardPendingEnable,
        A::ConfirmPendingEnableCleanup => I::ConfirmPendingEnableCleanup,
        A::Verify => I::StartVerification {
            source: CloudBackupVerificationSource::CloudBackupDetail,
            discoverable: false,
        },
        A::RepairPasskey => I::RepairPasskey { no_discovery: false },
        A::SyncUnsynced => I::SyncUnsynced,
        A::RefreshDetail => I::RefreshDetail,
        A::RecreateManifest => I::RecreateManifest,
        A::ReinitializeBackup => I::ReinitializeBackup,
        A::StartRestoreAll => I::StartRestoreAll,
        A::RetryRestoreAllRemaining => I::RetryRestoreAllRemaining,
        A::CancelRestoreAll => I::CancelRestoreAll,
        A::DeleteUnreadableBackups => I::DeleteUndecryptableWalletBackups,
        A::RecoverOtherBackups => I::RecoverOtherBackups,
        A::DeleteOtherBackups => I::DeleteOtherBackups,
        A::TurnOff => I::TurnOff,
        A::ConfirmDisable => {
            I::ConfirmDisable { review_id: admission.turn_off.review_id().unwrap_or_default() }
        }
        A::CancelDisable => I::CancelDisable,
        A::RestoreFirst => I::LeaveReview {
            review_id: admission.turn_off.review_id().unwrap_or_default(),
            to: ReviewExit::RestoreFirst,
        },
        A::VerifyFirst => I::LeaveReview {
            review_id: admission.turn_off.review_id().unwrap_or_default(),
            to: ReviewExit::VerifyFirst,
        },
        A::ResumeDisable => I::ResumeDisable,
        A::StopUsingOnThisDevice => I::StopUsingOnThisDevice,
        A::DismissHealNotice => I::DismissNotice(CloudBackupNoticeKind::Heal),
        A::ReconnectDrive => I::ReconnectDrive,
        A::SwitchDriveAccount => I::SwitchDriveAccount,
        A::RetryDriveAccountSwitchRecovery => I::RetryDriveAccountSwitchRecovery,
        A::CancelDriveAccountSwitch => I::CancelDriveAccountSwitch,
        A::RetryLoad => I::RetryLoad,
        A::ResetOnThisDevice => I::ResetOnThisDevice,
    }
}

/// Whether a control is enabled: a tap that would coalesce into the same running work shows that
/// work as the reason, and a tap that joins a refresh or sync stays enabled
fn availability(
    intent: &UserIntent,
    admission: &AdmissionSnapshot,
) -> CloudBackupActionAvailability {
    match evaluate_admission(intent, admission) {
        Admission::Accepted
        | Admission::Coalesced(
            CoalescibleIntent::RefreshDetail | CoalescibleIntent::SyncUnsynced,
        ) => CloudBackupActionAvailability::Available,
        Admission::Coalesced(CoalescibleIntent::RepeatOfRunning(operation)) => {
            CloudBackupActionAvailability::Unavailable {
                reason: CloudBackupRejection::OperationRunning { operation: operation.into() },
            }
        }
        Admission::Rejected(rejection) => {
            CloudBackupActionAvailability::Unavailable { reason: rejection.into() }
        }
    }
}

/// The Settings row for each body
fn settings_row(
    body: &CloudBackupScreenBody,
    notices: &Notices,
    presentation: &CloudBackupStateReducer,
) -> CloudBackupSettingsRowStatus {
    use CloudBackupSettingsRowStatus as Row;

    match body {
        CloudBackupScreenBody::Disabled => Row::Disabled,
        CloudBackupScreenBody::Enabling(_) => Row::SettingUp,
        CloudBackupScreenBody::Restoring(_) => Row::Restoring,
        CloudBackupScreenBody::Configured(_) if notices.heal.is_some() => Row::TurnedOnAgain,
        CloudBackupScreenBody::Configured(screen)
            if screen.disable == CloudBackupDisableScreen::Draining =>
        {
            Row::Disabling
        }
        CloudBackupScreenBody::Configured(_)
        | CloudBackupScreenBody::DriveAccountSwitch(CloudBackupDriveAccountSwitchScreen {
            stage:
                CloudBackupDriveAccountSwitchStage::Switching
                | CloudBackupDriveAccountSwitchStage::AwaitingSavedPasskeyConfirmation,
        }) => presentation.configured_settings_row(),
        CloudBackupScreenBody::DriveAccountSwitch(_)
        | CloudBackupScreenBody::PendingEnableRecovery(_) => Row::RecoveryRequired,
        CloudBackupScreenBody::FinishingDisable(_) => Row::TurnOffIncomplete,
        CloudBackupScreenBody::LoadFailed => Row::Error(CLOUD_BACKUP_LOAD_FAILED_MESSAGE.into()),
        CloudBackupScreenBody::Corrupted => Row::Error(CORRUPTED_CLOUD_BACKUP_STATE_MESSAGE.into()),
    }
}

impl From<Rejection> for CloudBackupRejection {
    fn from(rejection: Rejection) -> Self {
        match rejection {
            Rejection::CloudBackupOff => Self::CloudBackupOff,
            Rejection::NotConfigured => Self::NotConfigured,
            // the exported reasons name no corrupted record; both say the record can't be used
            Rejection::LoadFailed | Rejection::Corrupted => Self::LoadFailed,
            Rejection::TurnOffCommitted => Self::TurnOffCommitted,
            Rejection::OperationRunning(operation) => {
                Self::OperationRunning { operation: operation.into() }
            }
            Rejection::DetailInventoryNotLoaded => Self::InventoryNotLoaded,
            Rejection::OtherBackupsNotLoaded => Self::OtherBackupsNotLoaded,
            Rejection::RestoreAllUnavailable => Self::RestoreAllUnavailable,
            Rejection::NothingToCancel => Self::NothingToCancel,
            Rejection::NothingToContinue => Self::NothingToContinue,
            Rejection::Resetting => Self::Resetting,
            Rejection::ReviewStale => Self::ReviewStale,
            Rejection::TryAgainFirst => Self::TryAgainFirst,
            Rejection::DriveAccountSwitch => {
                Self::OperationRunning { operation: CloudBackupOperationKind::DriveAccountSwitch }
            }
        }
    }
}

impl From<CloudBackupExclusiveOperation> for CloudBackupOperationKind {
    fn from(operation: CloudBackupExclusiveOperation) -> Self {
        use CloudBackupExclusiveOperation as Op;

        match operation {
            Op::Enable | Op::EnableForceNew | Op::EnableNoDiscovery => Self::Enable,
            Op::Restore => Self::Restore,
            Op::Disable => Self::TurnOff,
            Op::RecreateManifest => Self::RecreateManifest,
            Op::ReinitializeBackup | Op::VerifyReinitializedBackup => Self::Reinitialize,
            Op::RepairPasskey => Self::RepairPasskey,
            Op::Verify => Self::Verify,
            Op::RecoverOtherBackups => Self::RecoverOtherBackups,
            Op::DeleteOtherBackups => Self::DeleteOtherBackups,
            Op::RestoreCloudWallet => Self::RestoreCloudWallet,
            Op::RestoreAllCloudWallets => Self::RestoreAll,
            Op::DeleteCloudWallet | Op::DeleteUndecryptableWalletBackups => Self::DeleteRecords,
            Op::DiscardPendingEnable => Self::DiscardPendingEnable,
        }
    }
}
