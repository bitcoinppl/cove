//! The projection of the runtime, the reducer, the inventory facts and the notices onto the
//! exported screen
//!
//! The body and sections come from the reducer, the rows from the inventory facts, and the Turn
//! Off states from the runtime. Every control's availability comes from `evaluate_admission`, on
//! a snapshot rebuilt from the reducer, so the screen never offers what dispatch would refuse for
//! a reason the reducer can see

use cove_device::cloud_storage::CloudSyncHealth;

use super::notices::Notices;
use super::types::{
    CloudBackupActionAvailability, CloudBackupActionEntry, CloudBackupActions,
    CloudBackupConfiguredScreen, CloudBackupDeleteConfirmation,
    CloudBackupDriveAccountSwitchRecovery, CloudBackupDriveAccountSwitchScreen,
    CloudBackupExtraCopiesSection, CloudBackupFinishingDisableScreen, CloudBackupHealNotice,
    CloudBackupListedRecord, CloudBackupListedRow, CloudBackupNotOnDeviceSection,
    CloudBackupNoticeKind, CloudBackupOperationKind, CloudBackupOtherBackupsSection,
    CloudBackupRecoveryOperation, CloudBackupRejection, CloudBackupRowAction,
    CloudBackupRowActionKind, CloudBackupScreen, CloudBackupScreenAction, CloudBackupScreenBody,
    CloudBackupSection, CloudBackupVerificationView, CloudBackupWalletsSection,
};
use crate::manager::cloud_backup_manager::actors::supervisor::{Runtime, enable_prompt_answers};
use crate::manager::cloud_backup_manager::admission::{
    Admission, AdmissionBody, AdmissionSnapshot, CoalescibleIntent, ContinuationOwner,
    ContinuationStage, EnableIntent, PausedContinuation, Rejection, RestoreAllAdmission,
    UserIntent, evaluate_admission,
};
use crate::manager::cloud_backup_manager::inventory::{
    InventoryFacts, InventoryRows, InventoryView, ListedRecord,
};
use crate::manager::cloud_backup_manager::{
    CORRUPTED_CLOUD_BACKUP_STATE_MESSAGE, CloudBackupConfiguredState,
    CloudBackupDestructiveOperationState, CloudBackupEnableContext, CloudBackupEnableFlow,
    CloudBackupEnablePromptChoice, CloudBackupExclusiveOperation,
    CloudBackupExclusiveOperationClaim, CloudBackupLifecycle, CloudBackupOtherBackupsState,
    CloudBackupPasskeyState, CloudBackupPendingEnableCleanupState, CloudBackupRestoreAllState,
    CloudBackupRootPrompt, CloudBackupSettingsRowStatus, CloudBackupStateReducer,
    CloudBackupSyncState, CloudBackupVerificationSource, CloudBackupVerificationState,
};

/// The screen for the runtime, the reducer, the inventory and the notices as they stand
///
/// The reducer still holds the flows, passkey, verification and sync presentation; the runtime,
/// the inventory facts and the notices take over the rest
pub(crate) fn project(
    runtime: &Runtime,
    reducer: &CloudBackupStateReducer,
    inventory: &InventoryFacts,
    notices: &Notices,
) -> CloudBackupScreen {
    let state = reducer.public_state();
    let inventory = inventory.view();
    let heal_notice = notices.heal;
    let snapshot = admission_snapshot(reducer, &state.lifecycle, &inventory);

    let root_prompt = match &state.lifecycle {
        CloudBackupLifecycle::Configured(configured) => configured.root_prompt.clone(),
        _ => CloudBackupRootPrompt::None,
    };

    let actions = global_actions(&state.lifecycle, heal_notice, &snapshot, reducer, &inventory);
    let body = body(runtime, reducer, state.lifecycle, inventory, &snapshot);
    let settings_row = settings_row(&body, heal_notice, state.settings_row_status);

    CloudBackupScreen {
        body,
        settings_row,
        heal_notice,
        last_outcome: notices.last_outcome.clone(),
        root_prompt,
        actions,
    }
}

/// What admission reads, rebuilt from the reducer
///
/// The supervisor's runtime is the authority dispatch uses; the reducer mirrors its body and
/// running operation, which is what a control's availability needs
fn admission_snapshot(
    reducer: &CloudBackupStateReducer,
    lifecycle: &CloudBackupLifecycle,
    inventory: &InventoryView,
) -> AdmissionSnapshot {
    let body = match lifecycle {
        CloudBackupLifecycle::Disabled => AdmissionBody::Disabled,
        // a paused Enable holds no claim, so the supervisor's runtime is still Disabled
        CloudBackupLifecycle::Enabling(flow) if paused_stage(flow, reducer).is_some() => {
            AdmissionBody::Disabled
        }
        CloudBackupLifecycle::Enabling(_) => AdmissionBody::Enabling,
        CloudBackupLifecycle::Restoring(_) => AdmissionBody::Restoring,
        CloudBackupLifecycle::Configured(configured) if finishing_disable(configured) => {
            AdmissionBody::FinishingDisable
        }
        CloudBackupLifecycle::Configured(_) => AdmissionBody::Configured,
        CloudBackupLifecycle::PendingEnableRecovery(_) => AdmissionBody::PendingEnableRecovery,
        CloudBackupLifecycle::Failed(failure)
            if failure.message == CORRUPTED_CLOUD_BACKUP_STATE_MESSAGE =>
        {
            AdmissionBody::Disabled
        }
        CloudBackupLifecycle::Failed(_) => AdmissionBody::LoadFailed,
    };

    let (sync_running, restore_all) = match lifecycle {
        CloudBackupLifecycle::Configured(configured) => (
            configured.sync == CloudBackupSyncState::Syncing,
            restore_all_admission(&inventory.restore_all),
        ),
        _ => (false, RestoreAllAdmission::Unavailable),
    };

    AdmissionSnapshot {
        body,
        running: reducer.active_operation().map(CloudBackupExclusiveOperationClaim::operation),
        sync_running,
        detail_inventory: inventory.readiness,
        other_backups_loaded: matches!(
            inventory.other_backups,
            CloudBackupOtherBackupsState::Loaded { .. }
        ),
        restore_all,
        continuation: paused_continuation(lifecycle, reducer),
        enable_prompt: enable_prompt_answers(reducer),
    }
}

/// The paused Enable the reducer shows, as admission sees it
///
/// The supervisor's continuation is the authority dispatch reads. The reducer can't tell a paused
/// reinitialize from a paused Enable, so this mirror treats both as an Enable
fn paused_continuation(
    lifecycle: &CloudBackupLifecycle,
    reducer: &CloudBackupStateReducer,
) -> Option<PausedContinuation> {
    let CloudBackupLifecycle::Enabling(flow) = lifecycle else { return None };
    let stage = paused_stage(flow, reducer)?;

    Some(PausedContinuation { owner: ContinuationOwner::Enable, stage })
}

/// Where the Enable `flow` shows stopped, when it waits on the user with nothing running
fn paused_stage(
    flow: &CloudBackupEnableFlow,
    reducer: &CloudBackupStateReducer,
) -> Option<ContinuationStage> {
    if reducer.active_operation().is_some() {
        return None;
    }

    match flow {
        CloudBackupEnableFlow::AwaitingSavedPasskeyConfirmation(_) => {
            Some(ContinuationStage::AwaitingSavedPasskey)
        }
        CloudBackupEnableFlow::AwaitingForceNewConfirmation(..) => {
            Some(ContinuationStage::ExistingBackupPrompt)
        }
        CloudBackupEnableFlow::DiscoveringExistingBackup
        | CloudBackupEnableFlow::AwaitingPasskeyChoice(_)
        | CloudBackupEnableFlow::CreatingPasskey
        | CloudBackupEnableFlow::ConfirmingSavedPasskey
        | CloudBackupEnableFlow::UploadingInitialBackup { .. }
        | CloudBackupEnableFlow::RetryingUploadWithStagedMaterial { .. }
        | CloudBackupEnableFlow::WaitingForPasskeyAvailability => None,
    }
}

fn restore_all_admission(restore_all: &CloudBackupRestoreAllState) -> RestoreAllAdmission {
    match restore_all {
        CloudBackupRestoreAllState::StartAvailable { .. } => RestoreAllAdmission::StartAvailable,
        CloudBackupRestoreAllState::RetryAvailable { .. } => RestoreAllAdmission::RetryAvailable,
        CloudBackupRestoreAllState::Running { .. } => RestoreAllAdmission::Running,
        CloudBackupRestoreAllState::NotShown
        | CloudBackupRestoreAllState::StartDisabled { .. }
        | CloudBackupRestoreAllState::RetryDisabled { .. } => RestoreAllAdmission::Unavailable,
    }
}

/// A Turn Off that failed after it committed, so it can only go forward
fn finishing_disable(configured: &CloudBackupConfiguredState) -> bool {
    matches!(
        configured.destructive_operation,
        CloudBackupDestructiveOperationState::DisableFailed { can_keep_enabled: false, .. }
    )
}

fn body(
    runtime: &Runtime,
    reducer: &CloudBackupStateReducer,
    lifecycle: CloudBackupLifecycle,
    inventory: InventoryView,
    snapshot: &AdmissionSnapshot,
) -> CloudBackupScreenBody {
    if let Some(recovery) = reducer.drive_account_switch_recovery() {
        return drive_account_switch_body(recovery);
    }

    match lifecycle {
        CloudBackupLifecycle::Disabled => CloudBackupScreenBody::Disabled,
        CloudBackupLifecycle::Enabling(flow) => CloudBackupScreenBody::Enabling(flow),
        CloudBackupLifecycle::Restoring(flow) => CloudBackupScreenBody::Restoring(flow),
        CloudBackupLifecycle::PendingEnableRecovery(recovery) => {
            CloudBackupScreenBody::PendingEnableRecovery(recovery)
        }
        CloudBackupLifecycle::Failed(failure)
            if failure.message == CORRUPTED_CLOUD_BACKUP_STATE_MESSAGE =>
        {
            CloudBackupScreenBody::Corrupted
        }
        CloudBackupLifecycle::Failed(_) => CloudBackupScreenBody::LoadFailed,
        CloudBackupLifecycle::Configured(configured) if finishing_disable(&configured) => {
            CloudBackupScreenBody::FinishingDisable(CloudBackupFinishingDisableScreen {
                stage: runtime.finishing_disable_stage(),
            })
        }
        CloudBackupLifecycle::Configured(configured) => CloudBackupScreenBody::Configured(
            configured_screen(runtime, configured, inventory, snapshot),
        ),
    }
}

fn drive_account_switch_body(
    recovery: CloudBackupDriveAccountSwitchRecovery,
) -> CloudBackupScreenBody {
    CloudBackupScreenBody::DriveAccountSwitch(CloudBackupDriveAccountSwitchScreen {
        recovery: Some(recovery),
    })
}

fn configured_screen(
    runtime: &Runtime,
    configured: CloudBackupConfiguredState,
    inventory: InventoryView,
    snapshot: &AdmissionSnapshot,
) -> CloudBackupConfiguredScreen {
    let mut sections = Vec::new();

    if configured.passkey != CloudBackupPasskeyState::Available {
        sections.push(CloudBackupSection::PasskeyCard(configured.passkey.clone()));
    }

    if let Some(progress) = inventory.progress {
        sections.push(CloudBackupSection::Progress(progress));
    }

    sections.push(CloudBackupSection::Verification(CloudBackupVerificationView {
        state: configured.verification.clone(),
        presentation: configured.verification_presentation.clone(),
        recovery: recovery_operation(&configured.destructive_operation),
    }));

    let last_sync = inventory.rows.as_ref().and_then(|rows| rows.last_sync);
    if let Some(rows) = inventory.rows {
        let (not_on_device, extra_copies) = listed_rows(&rows, snapshot);
        sections.push(CloudBackupSection::Wallets(CloudBackupWalletsSection {
            up_to_date: rows.up_to_date,
            needs_sync: rows.needs_sync,
            sync: configured.sync.clone(),
        }));

        if !not_on_device.is_empty()
            || inventory.restore_all != CloudBackupRestoreAllState::NotShown
        {
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

    if inventory.other_backups != CloudBackupOtherBackupsState::NotChecked {
        sections.push(CloudBackupSection::OtherBackups(CloudBackupOtherBackupsSection {
            state: inventory.other_backups.clone(),
            operation: inventory.other_backups_operation.clone(),
        }));
    }

    sections.push(CloudBackupSection::TurnOff);

    CloudBackupConfiguredScreen {
        sections,
        last_sync,
        sync_health: configured.sync_health,
        disable: runtime.disable_screen(),
    }
}

fn recovery_operation(
    destructive_operation: &CloudBackupDestructiveOperationState,
) -> CloudBackupRecoveryOperation {
    match destructive_operation {
        CloudBackupDestructiveOperationState::RecreatingManifest => {
            CloudBackupRecoveryOperation::RecreatingManifest
        }
        CloudBackupDestructiveOperationState::ReinitializingBackup => {
            CloudBackupRecoveryOperation::ReinitializingBackup
        }
        CloudBackupDestructiveOperationState::Idle
        | CloudBackupDestructiveOperationState::Disabling
        | CloudBackupDestructiveOperationState::DisableFailed { .. } => {
            CloudBackupRecoveryOperation::Idle
        }
    }
}

/// The listed rows, split into Not on This Device and Extra Copies
fn listed_rows(
    rows: &InventoryRows,
    snapshot: &AdmissionSnapshot,
) -> (Vec<CloudBackupListedRow>, Vec<CloudBackupListedRow>) {
    let (extra_copies, not_on_device) = rows
        .listed
        .iter()
        .map(|listed| listed_row(listed, rows.copy_has_current_backup(listed), snapshot))
        .partition(|row| matches!(row.record, CloudBackupListedRecord::ExtraCopy { .. }));

    (not_on_device, extra_copies)
}

/// A listed row with its controls; `copy_replaced` says a covering wallet has its own current backup
fn listed_row(
    listed: &ListedRecord,
    copy_replaced: bool,
    snapshot: &AdmissionSnapshot,
) -> CloudBackupListedRow {
    let ListedRecord { record_id, record, .. } = listed.clone();
    let restore = UserIntent::RestoreCloudWallet(record_id.clone());
    let delete = UserIntent::DeleteCloudWallet(record_id.clone());
    let standard_delete =
        CloudBackupRowActionKind::Delete { confirmation: CloudBackupDeleteConfirmation::Standard };

    let actions = match &record {
        // an extra copy may go only once a wallet it copies has its own current backup
        CloudBackupListedRecord::ExtraCopy { .. } if copy_replaced => {
            vec![row_action(standard_delete, &delete, snapshot)]
        }
        CloudBackupListedRecord::ExtraCopy { .. } => vec![
            row_action(CloudBackupRowActionKind::BackUpFirst, &UserIntent::SyncUnsynced, snapshot),
            row_action(CloudBackupRowActionKind::CheckAgain, &UserIntent::RefreshDetail, snapshot),
        ],
        CloudBackupListedRecord::Unsupported => {
            vec![row_action(standard_delete, &delete, snapshot)]
        }
        CloudBackupListedRecord::Unchecked => vec![row_action(
            CloudBackupRowActionKind::CheckAgain,
            &UserIntent::RefreshDetail,
            snapshot,
        )],
        CloudBackupListedRecord::NotOnDevice { .. } => vec![
            row_action(CloudBackupRowActionKind::Restore, &restore, snapshot),
            row_action(
                CloudBackupRowActionKind::Delete {
                    confirmation: CloudBackupDeleteConfirmation::MayBeOnlyCopy,
                },
                &delete,
                snapshot,
            ),
        ],
        // the reducer lists no unreadable record, so none reaches here until the inventory owner
        // lists them and the projection gives them their controls
        CloudBackupListedRecord::Unreadable { .. } => Vec::new(),
    };

    CloudBackupListedRow { record_id, record, actions }
}

fn row_action(
    action: CloudBackupRowActionKind,
    intent: &UserIntent,
    snapshot: &AdmissionSnapshot,
) -> CloudBackupRowAction {
    CloudBackupRowAction { action, availability: availability(intent, snapshot) }
}

/// The global actions drawn for this lifecycle, each with what admission decides for it
fn global_actions(
    lifecycle: &CloudBackupLifecycle,
    heal_notice: Option<CloudBackupHealNotice>,
    snapshot: &AdmissionSnapshot,
    reducer: &CloudBackupStateReducer,
    inventory: &InventoryView,
) -> CloudBackupActions {
    use CloudBackupScreenAction as A;

    let mut drawn = if reducer.drive_account_switch_recovery().is_some() {
        vec![A::RetryDriveAccountSwitchRecovery, A::CancelDriveAccountSwitch]
    } else {
        lifecycle_actions(lifecycle, inventory)
    };

    if heal_notice.is_some() {
        drawn.push(A::DismissHealNotice);
    }

    let entries = drawn
        .into_iter()
        .map(|action| CloudBackupActionEntry {
            action,
            availability: availability(&intent_for(action), snapshot),
        })
        .collect();

    CloudBackupActions { entries }
}

fn lifecycle_actions(
    lifecycle: &CloudBackupLifecycle,
    inventory: &InventoryView,
) -> Vec<CloudBackupScreenAction> {
    use CloudBackupScreenAction as A;

    match lifecycle {
        CloudBackupLifecycle::Disabled => vec![A::Enable, A::RestoreFromCloudBackup],
        CloudBackupLifecycle::Enabling(
            CloudBackupEnableFlow::AwaitingSavedPasskeyConfirmation(_),
        ) => vec![A::ConfirmSavedPasskey],
        CloudBackupLifecycle::Enabling(
            CloudBackupEnableFlow::AwaitingForceNewConfirmation(..)
            | CloudBackupEnableFlow::AwaitingPasskeyChoice(_),
        ) => vec![A::AcceptEnablePrompt],
        CloudBackupLifecycle::Enabling(_) => Vec::new(),
        CloudBackupLifecycle::Restoring(_) => vec![A::CancelRestore],
        CloudBackupLifecycle::PendingEnableRecovery(recovery) => {
            let mut actions = vec![A::DiscardPendingEnable];
            if recovery.cleanup == CloudBackupPendingEnableCleanupState::Available {
                actions.push(A::ConfirmPendingEnableCleanup);
            }

            actions
        }
        CloudBackupLifecycle::Failed(failure)
            if failure.message == CORRUPTED_CLOUD_BACKUP_STATE_MESSAGE =>
        {
            vec![A::ResetOnThisDevice]
        }
        CloudBackupLifecycle::Failed(_) => vec![A::RetryLoad],
        CloudBackupLifecycle::Configured(configured) if finishing_disable(configured) => {
            vec![A::ResumeDisable, A::StopUsingOnThisDevice]
        }
        CloudBackupLifecycle::Configured(configured) => configured_actions(configured, inventory),
    }
}

fn configured_actions(
    configured: &CloudBackupConfiguredState,
    inventory: &InventoryView,
) -> Vec<CloudBackupScreenAction> {
    use CloudBackupScreenAction as A;

    let mut actions = vec![
        A::Verify,
        A::SyncUnsynced,
        A::RefreshDetail,
        A::RecreateManifest,
        A::ReinitializeBackup,
        A::SwitchDriveAccount,
        A::TurnOff,
    ];

    if configured.passkey != CloudBackupPasskeyState::Available {
        actions.push(A::RepairPasskey);
    }

    match inventory.restore_all {
        CloudBackupRestoreAllState::NotShown => {}
        CloudBackupRestoreAllState::StartDisabled { .. }
        | CloudBackupRestoreAllState::StartAvailable { .. } => actions.push(A::StartRestoreAll),
        CloudBackupRestoreAllState::RetryDisabled { .. }
        | CloudBackupRestoreAllState::RetryAvailable { .. } => {
            actions.push(A::RetryRestoreAllRemaining);
        }
        CloudBackupRestoreAllState::Running { .. } => actions.push(A::CancelRestoreAll),
    }

    if matches!(configured.verification, CloudBackupVerificationState::NeedsAttention { .. }) {
        actions.push(A::DeleteUnreadableBackups);
    }

    if let CloudBackupOtherBackupsState::Loaded { summary } = &inventory.other_backups
        && summary.namespace_count > 0
    {
        actions.push(A::RecoverOtherBackups);
        actions.push(A::DeleteOtherBackups);
    }

    if matches!(configured.sync_health, CloudSyncHealth::AuthorizationRequired(_)) {
        actions.push(A::ReconnectDrive);
    }

    actions
}

/// The intent a control dispatches, as admission sees it
///
/// Payloads that don't change admission take a fixed value: a review id, a prompt choice and a
/// verification source decide what the started work does, not whether it may start
fn intent_for(action: CloudBackupScreenAction) -> UserIntent {
    use CloudBackupScreenAction as A;
    use UserIntent as I;

    match action {
        A::Enable => I::Enable(EnableIntent::Standard, CloudBackupEnableContext::settings_manual()),
        A::RestoreFromCloudBackup => I::RestoreFromCloudBackup,
        A::CancelRestore => I::CancelRestore,
        A::ConfirmSavedPasskey => I::ConfirmSavedPasskey,
        A::AcceptEnablePrompt => I::AcceptEnablePrompt(CloudBackupEnablePromptChoice::UseExisting),
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
        A::ConfirmDisable => I::ConfirmDisable { review_id: 0 },
        A::CancelDisable => I::CancelDisable,
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
    snapshot: &AdmissionSnapshot,
) -> CloudBackupActionAvailability {
    match evaluate_admission(intent, snapshot) {
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

/// The Settings row: a committed Turn Off and the heal notice take the row over
fn settings_row(
    body: &CloudBackupScreenBody,
    heal_notice: Option<CloudBackupHealNotice>,
    row: CloudBackupSettingsRowStatus,
) -> CloudBackupSettingsRowStatus {
    match body {
        CloudBackupScreenBody::FinishingDisable(_) => {
            CloudBackupSettingsRowStatus::TurnOffIncomplete
        }
        CloudBackupScreenBody::Configured(_) if heal_notice.is_some() => {
            CloudBackupSettingsRowStatus::TurnedOnAgain
        }
        _ => row,
    }
}

impl From<Rejection> for CloudBackupRejection {
    fn from(rejection: Rejection) -> Self {
        match rejection {
            Rejection::CloudBackupOff => Self::CloudBackupOff,
            Rejection::NotConfigured => Self::NotConfigured,
            Rejection::LoadFailed => Self::LoadFailed,
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
