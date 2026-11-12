//! The exported Cloud Backup screen: everything Swift and Kotlin draw, decided in Rust
//!
//! Platforms read only [`CloudBackupScreen`]. Which body and which sections are on screen, which
//! controls each section draws, and whether each control is enabled, come from here; a platform
//! never derives a control's presence or availability from the content these types carry

use cove_device::cloud_storage::CloudSyncHealth;

use crate::manager::cloud_backup_manager::{
    CloudBackupBatchDeleteConsent, CloudBackupRowDeleteConsent,
};
use crate::manager::cloud_backup_manager::{
    CloudBackupEnableFlow, CloudBackupEnablePromptChoice, CloudBackupInventoryIncompleteReason,
    CloudBackupOtherBackupsState, CloudBackupPasskeyHint, CloudBackupPasskeyState,
    CloudBackupPendingEnableRecovery, CloudBackupRestoreAllState, CloudBackupRestoreFlow,
    CloudBackupRootPrompt, CloudBackupSettingsRowStatus, CloudBackupSyncState,
    CloudBackupVerificationPresentation, CloudBackupVerificationState, CloudBackupWalletItem,
    CloudOnlyOperation, OtherBackupsOperation, RecordId,
};

/// Everything the Cloud Backup UI draws, projected by Rust from the facts it owns
///
/// `state()` returns the one last published as a [`CloudBackupPublishedScreen`]; every later
/// change arrives as one [`CloudBackupScreenDelta`], which each platform applies with
/// [`CloudBackupPublishedScreen::applying`] in a single assignment
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct CloudBackupScreen {
    pub body: CloudBackupScreenBody,
    pub settings_row: CloudBackupSettingsRowStatus,
    /// Persisted until the user dismisses it; shown on the Settings row and the detail screen
    pub heal_notice: Option<CloudBackupHealNotice>,
    /// The result of the last intent that did not simply succeed, drawn next to its control
    pub last_outcome: Option<CloudBackupLastOutcome>,
    pub root_prompt: CloudBackupRootPrompt,
    /// Actions that are not tied to one row; each row carries its own
    pub actions: CloudBackupActions,
}

/// Which screen Cloud Backup shows; each body names its exits in `actions`
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum CloudBackupScreenBody {
    Disabled,
    Enabling(CloudBackupEnableFlow),
    Restoring(CloudBackupRestoreFlow),
    Configured(CloudBackupConfiguredScreen),
    /// A committed Turn Off that still has to finish; it only goes forward
    FinishingDisable(CloudBackupFinishingDisableScreen),
    /// A Google Drive account switch is recorded; Android only
    DriveAccountSwitch(CloudBackupDriveAccountSwitchScreen),
    /// Work that registered a passkey waits for the user to confirm they saved it, whichever
    /// work it is; nothing runs until the user answers
    SavedPasskeyConfirmation(CloudBackupSavedPasskeyScreen),
    PendingEnableRecovery(CloudBackupPendingEnableRecovery),
    /// The stored record could not be loaded; Try Again runs the load again
    LoadFailed,
    /// The stored record can't be read as a backup; the exit resets Cloud Backup on this device
    Corrupted,
}

/// A configured backup, drawn as Rust's ordered sections
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct CloudBackupConfiguredScreen {
    /// In the order they are drawn; Turn Off is in every configured screen
    pub sections: Vec<CloudBackupSection>,
    /// Android's top-bar menu, in the order it lists them; iOS has no menu, and every entry is an
    /// Android-only action
    pub menu: Vec<CloudBackupMenuControl>,
    pub last_sync: Option<u64>,
    pub sync_health: CloudSyncHealth,
    /// Turn Off's presentation state, which drives its dialogs
    pub disable: CloudBackupDisableScreen,
    /// Where the passkey the passkey card asks for was created, drawn on that card
    ///
    /// Set only while the card shows a missing passkey and this device stored where that passkey
    /// was created
    pub passkey_hint: Option<CloudBackupPasskeyHint>,
}

/// One section of the configured screen
///
/// Each section names the controls it draws: a section with a fixed control draws it by its kind,
/// and every other section carries the list of its controls, so a platform draws exactly what the
/// section names
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum CloudBackupSection {
    /// Missing, unsupported or needs-repair passkey card, above the list; draws the passkey repair
    PasskeyCard(CloudBackupPasskeyState),
    Progress(CloudBackupProgressSection),
    Verification(CloudBackupVerificationSection),
    Wallets(CloudBackupWalletsSection),
    NotOnDevice(CloudBackupNotOnDeviceSection),
    ExtraCopies(CloudBackupExtraCopiesSection),
    OtherBackups(CloudBackupOtherBackupsSection),
    /// Draws Turn Off
    TurnOff,
}

/// What the progress section shows
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum CloudBackupProgressSection {
    /// The silent inventory is loading, inline above the rows it will fill
    InventoryChecking,
    /// The inventory could not be confirmed complete
    InventoryIncomplete {
        reason: CloudBackupInventoryIncompleteReason,
        /// In the order they are drawn
        controls: Vec<CloudBackupInventoryControl>,
    },
    /// A running verification in place of the rows; it draws no control
    VerificationCard(CloudBackupVerificationView),
    /// A running verification above the rows it will update; it draws no control
    VerificationInline(CloudBackupVerificationView),
}

/// A control the incomplete-inventory section draws
#[derive(Debug, Clone, Copy, Hash, PartialEq, Eq, uniffi::Enum)]
pub enum CloudBackupInventoryControl {
    /// Check Again: a silent inventory refresh
    CheckAgain,
    /// Android only: Google Drive needs authorizing before the inventory can load
    ReconnectDrive,
}

/// Verification status and the recovery work that hangs off it
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct CloudBackupVerificationView {
    pub state: CloudBackupVerificationState,
    pub presentation: CloudBackupVerificationPresentation,
    pub recovery: CloudBackupRecoveryOperation,
}

/// The verification section: the verification Rust reports and the controls drawn with it
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct CloudBackupVerificationSection {
    pub view: CloudBackupVerificationView,
    /// In the order they are drawn; empty while verification runs, while an upload waits to be
    /// confirmed with nothing to act on, and for a backup only a newer Cove can read
    pub controls: Vec<CloudBackupVerificationControl>,
}

/// A control the verification section draws
#[derive(Debug, Clone, Hash, PartialEq, Eq, uniffi::Enum)]
pub enum CloudBackupVerificationControl {
    /// Verify Now, Verify Again, or Try Again after a failure another verification can fix
    Verify,
    /// The rebuild a failed verification names as the only way to recover the backup
    Recover { kind: CloudBackupRecoveryKind },
    /// Deletes the wallet backups the verified key couldn't decrypt, drawn on the issue that
    /// counts them; its confirmation sends back `consent`, frozen to the records listed as
    /// undecryptable when it was drawn
    DeleteUnreadableBackups { consent: CloudBackupBatchDeleteConsent },
    /// Try Again on a latest upload Cove couldn't confirm
    RetryUpload,
    /// Android only: the latest upload waits on Google Drive access
    ReconnectDrive,
}

/// Which rebuild recovers a backup that verification can't use
#[derive(Debug, Clone, Copy, Hash, PartialEq, Eq, uniffi::Enum)]
pub enum CloudBackupRecoveryKind {
    /// The index is missing and the master key is intact: rebuild the index from this device
    RecreateManifest,
    /// No verified master key is left: replace the backup with this device's wallets
    ReinitializeBackup,
}

/// An entry in Android's top-bar menu
///
/// The rebuilds are not here: a rebuild is offered only as the recovery a failed verification
/// names, on both platforms
#[derive(Debug, Clone, Copy, Hash, PartialEq, Eq, uniffi::Enum)]
pub enum CloudBackupMenuControl {
    SwitchDriveAccount,
}

/// A manifest or backup rebuild in progress
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum CloudBackupRecoveryOperation {
    Idle,
    RecreatingManifest,
    ReinitializingBackup,
}

/// This device's wallets and the state of their own backup records
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct CloudBackupWalletsSection {
    pub up_to_date: Vec<CloudBackupOwnRow>,
    pub needs_sync: Vec<CloudBackupOwnRow>,
    pub sync: CloudBackupSyncState,
    /// In the order they are drawn
    pub controls: Vec<CloudBackupWalletsControl>,
}

/// A control the wallets section draws
#[derive(Debug, Clone, Copy, Hash, PartialEq, Eq, uniffi::Enum)]
pub enum CloudBackupWalletsControl {
    /// Uploads the wallets that need sync, under the list of them
    SyncNow,
}

/// A local wallet and its own cloud record
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct CloudBackupOwnRow {
    pub wallet: CloudBackupWalletItem,
    pub record: CloudBackupOwnRecord,
}

/// What the cloud holds for a local wallet's own record
#[derive(Debug, Clone, Copy, Hash, PartialEq, Eq, uniffi::Enum)]
pub enum CloudBackupOwnRecord {
    Current,
    Pending,
    Stale,
    Missing,
    /// The local wallet's secret is gone; its cloud record may be the only copy
    SecretMissingLocally,
}

/// Listed records that are not a local wallet's own backup, and the Restore All control
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct CloudBackupNotOnDeviceSection {
    /// `NotOnDevice`, `Unsupported`, `Unreadable` and `Unchecked` rows
    pub rows: Vec<CloudBackupListedRow>,
    pub restore_all: CloudBackupRestoreAllState,
    pub operation: CloudOnlyOperation,
}

/// Other backup records of wallets this device already holds
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct CloudBackupExtraCopiesSection {
    /// `ExtraCopy` rows only
    pub rows: Vec<CloudBackupListedRow>,
}

/// One listed cloud record that is not a local wallet's own backup
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct CloudBackupListedRow {
    pub record_id: RecordId,
    pub record: CloudBackupListedRecord,
    pub actions: Vec<CloudBackupRowAction>,
}

/// What Cove knows about a listed record
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum CloudBackupListedRecord {
    /// A wallet that is not on this device
    NotOnDevice {
        wallet: CloudBackupWalletItem,
    },
    /// Another record of a wallet this device holds; `covered_by` names that local wallet
    ExtraCopy {
        wallet: CloudBackupWalletItem,
        covered_by: String,
    },
    /// Made by a newer version of Cove
    Unsupported,
    Unreadable {
        kind: CloudBackupUnreadableKind,
        trust: CloudBackupKeyTrust,
    },
    /// No usable local master key, so Cove can't read it without the passkey
    Unchecked,
}

/// Why a listed record could not be read
#[derive(Debug, Clone, Copy, Hash, PartialEq, Eq, uniffi::Enum)]
pub enum CloudBackupUnreadableKind {
    Decrypt,
    Parse,
    NotDownloadable,
    Provider,
}

/// Whether the key that failed to read a record is this configuration's verified key
#[derive(Debug, Clone, Copy, Hash, PartialEq, Eq, uniffi::Enum)]
pub enum CloudBackupKeyTrust {
    VerifiedCurrentKey,
    Unverified,
}

/// Backups in namespaces that don't match this device
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct CloudBackupOtherBackupsSection {
    pub state: CloudBackupOtherBackupsState,
    pub operation: OtherBackupsOperation,
    /// In the order they are drawn
    pub controls: Vec<CloudBackupOtherBackupsControl>,
}

/// A control the other-backups section draws
#[derive(Debug, Clone, Copy, Hash, PartialEq, Eq, uniffi::Enum)]
pub enum CloudBackupOtherBackupsControl {
    Recover,
    Delete,
    /// Checks for other backups again after a check that failed
    CheckAgain,
}

/// Turn Off's presentation state inside a configured backup
///
/// The review holds no operation; `Draining` is the Turn Off operation before it commits, and
/// after the commit the body is `FinishingDisable`
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum CloudBackupDisableScreen {
    Idle,
    /// A silent refresh is building the review; Cancel is available
    Preparing,
    /// The provider can't be reached, and the delete can't run without it; one dialog with
    /// Try Again and Cancel
    Offline,
    /// The dialogs show while this holds, and come back after unlock
    Reviewing(CloudBackupDisableReview),
    /// Writes are draining before the commit; Cancel is still available
    Draining,
}

/// What Turn Off would delete, bound to the evidence it was computed from
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct CloudBackupDisableReview {
    /// `ConfirmDisable` carries this id; admission rejects one that is no longer current
    pub review_id: u64,
    pub impact: CloudBackupDisableImpact,
    /// The recheck after the drain found more at risk than the user confirmed
    pub changed_since_confirmation: bool,
}

/// The impact the Turn Off dialogs describe
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct CloudBackupDisableImpact {
    pub assessment: CloudBackupInventoryAssessment,
    /// This device's wallets whose backups are deleted
    pub own_backups: u32,
    pub not_on_device: CloudBackupNamedCount,
    /// Local wallets whose secret is gone; their cloud record may be the only copy
    pub own_secret_missing_locally: CloudBackupNamedCount,
    pub extra_copies: u32,
    pub unchecked: u32,
    pub unreadable: u32,
    pub unsupported: u32,
    /// Backups made with another passkey; Turn Off leaves them in place
    pub other_backups: u32,
    pub confirmation: CloudBackupDisableConfirmation,
}

/// How complete the evidence behind an inventory or a review is
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum CloudBackupInventoryAssessment {
    Complete,
    Partial { reason: CloudBackupInventoryIncompleteReason },
    Unknown { reason: CloudBackupInventoryIncompleteReason },
}

/// A count with at most a few names; Rust sets the cap so both apps agree
#[derive(Debug, Clone, PartialEq, Eq, Default, uniffi::Record)]
pub struct CloudBackupNamedCount {
    pub names: Vec<String>,
    /// How many more there are beyond `names`, for "and N more"
    pub more: u32,
}

impl CloudBackupNamedCount {
    /// How many there are in all, named or not
    pub fn total(&self) -> u32 {
        u32::try_from(self.names.len()).unwrap_or(u32::MAX).saturating_add(self.more)
    }
}

/// How many dialogs Turn Off asks through
#[derive(Debug, Clone, Copy, Hash, PartialEq, Eq, uniffi::Enum)]
pub enum CloudBackupDisableConfirmation {
    One,
    /// Something might be lost, or Cove couldn't check
    Two,
}

/// A committed Turn Off that still has to delete or clean up
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct CloudBackupFinishingDisableScreen {
    pub stage: CloudBackupFinishingDisableStage,
}

/// Where a committed Turn Off stands
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum CloudBackupFinishingDisableStage {
    /// Deleting the backups or cleaning up this device
    Resuming,
    /// The last attempt failed; Try Again, or after a failed Try Again stop using Cloud Backup on
    /// this device
    Failed { reason: CloudBackupTurnOffFailure },
}

/// Why a committed Turn Off could not finish
#[derive(Debug, Clone, Copy, Hash, PartialEq, Eq, uniffi::Enum)]
pub enum CloudBackupTurnOffFailure {
    Offline,
    /// Google Drive needs Reconnect, or iCloud Drive is off for Cove
    AuthorizationRequired,
    ProviderUnavailable,
    Other,
}

/// A recorded Google Drive account switch; Android only
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct CloudBackupDriveAccountSwitchScreen {
    pub stage: CloudBackupDriveAccountSwitchStage,
}

/// Where a Google Drive account switch stands for the user
///
/// A switch paused on the passkey its new backup registered is not a stage: the paused work owns
/// the screen as [`CloudBackupScreenBody::SavedPasskeyConfirmation`]
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum CloudBackupDriveAccountSwitchStage {
    /// Cove is moving Cloud Backup to the selected account; nothing waits on the user
    Switching,
    /// A step of the switch failed and the user must try again, or cancel while the previous
    /// account can still come back
    NeedsAttention(CloudBackupDriveAccountSwitchRecovery),
}

/// A registered passkey waiting for the user to confirm they saved it, and the work it continues
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct CloudBackupSavedPasskeyScreen {
    /// The work that registered the passkey and goes on once it is confirmed
    pub lifecycle: CloudBackupSavedPasskeyLifecycle,
    /// Written for the reader, for this lifecycle
    pub title: String,
    /// Written for the reader: what confirming does, and what leaving does where it can
    pub message: String,
    /// In the order they are drawn: the exits admission takes for the paused work
    pub controls: Vec<CloudBackupSavedPasskeyControl>,
}

/// The work a saved-passkey confirmation continues
#[derive(Debug, Clone, Copy, Hash, PartialEq, Eq, uniffi::Enum)]
pub enum CloudBackupSavedPasskeyLifecycle {
    /// Turning Cloud Backup on
    Enable,
    /// Replacing the configured backup with a new one under the new passkey
    Reinitialize,
    /// Moving Cloud Backup to a new Google account; Android only
    DriveAccountSwitch,
}

/// A control the saved-passkey confirmation draws
#[derive(Debug, Clone, Copy, Hash, PartialEq, Eq, uniffi::Enum)]
pub enum CloudBackupSavedPasskeyControl {
    /// The passkey was saved: the paused work goes on
    Confirm,
    /// Ends the paused work; a switch's reinitialize has none
    Discard,
}

/// A Google Drive account switch that needs the user to retry or cancel it
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct CloudBackupDriveAccountSwitchRecovery {
    /// What went wrong, written for the reader
    pub message: String,
}

/// A persisted notice the user must dismiss
#[derive(Debug, Clone, Copy, Hash, PartialEq, Eq, uniffi::Enum)]
pub enum CloudBackupHealNotice {
    /// A Turn Off from an older build never deleted anything, so Cloud Backup is on again
    StuckTurnOffHealed { healed_at: u64 },
}

/// Which notice a `DismissNotice` intent dismisses
#[derive(Debug, Clone, Copy, Hash, PartialEq, Eq, uniffi::Enum)]
pub enum CloudBackupNoticeKind {
    Heal,
    LastOutcome,
}

/// The result of an intent, tagged with the control it came from
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct CloudBackupLastOutcome {
    pub target: CloudBackupOutcomeTarget,
    pub outcome: CloudBackupOutcome,
}

/// The control a last outcome is drawn next to
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum CloudBackupOutcomeTarget {
    Action(CloudBackupScreenAction),
    Row { target: CloudBackupRowTarget },
}

/// What happened to an intent that did not simply succeed
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum CloudBackupOutcome {
    Rejected {
        reason: CloudBackupRejection,
    },
    /// Turn Off ended before it committed; nothing was deleted and backup stays on
    DisableNotStarted {
        reason: CloudBackupDisableNotStartedReason,
    },
    /// The passkey request did not complete; nothing changed
    PasskeyNotCompleted,
    Failed {
        message: String,
    },
}

/// Why Turn Off ended before it deleted anything
#[derive(Debug, Clone, Copy, Hash, PartialEq, Eq, uniffi::Enum)]
pub enum CloudBackupDisableNotStartedReason {
    Offline,
    AuthorizationRequired,
    ProviderUnavailable,
    /// The writes in flight did not drain
    WritesDidNotDrain,
    /// The commit could not be saved
    SaveFailed,
}

/// Why an intent can't run now; the platform shows it as the control's caption
#[derive(Debug, Clone, Copy, Hash, PartialEq, Eq, uniffi::Enum)]
pub enum CloudBackupRejection {
    CloudBackupOff,
    /// Enable or restore was asked for while Cloud Backup is already on
    AlreadyOn,
    /// The work needs Cloud Backup on, and the Enable or restore that turns it on hasn't finished
    SetupNotFinished,
    LoadFailed,
    /// Turn Off already deleted remote data, so it only goes forward
    TurnOffCommitted,
    OperationRunning {
        operation: CloudBackupOperationKind,
    },
    InventoryNotLoaded,
    OtherBackupsNotLoaded,
    RestoreAllUnavailable,
    NothingToCancel,
    NothingToContinue,
    /// What the confirmation answers is no longer current: the Turn Off review, or the rows and
    /// evidence a delete's consent names
    ReviewStale,
    /// A local reset is wiping Cloud Backup's local state
    Resetting,
    /// Offered only after a failed Try Again
    TryAgainFirst,
}

/// The running operation, as captions and outcomes name it
#[derive(Debug, Clone, Copy, Hash, PartialEq, Eq, uniffi::Enum)]
pub enum CloudBackupOperationKind {
    Enable,
    Restore,
    TurnOff,
    ResumeTurnOff,
    Verify,
    RepairPasskey,
    RestoreCloudWallet,
    RestoreAll,
    DeleteRecords,
    RecoverOtherBackups,
    DeleteOtherBackups,
    Reinitialize,
    RecreateManifest,
    DiscardPendingEnable,
    DriveAccountSwitch,
}

/// The screen's global actions; a missing action has no control on this screen
///
/// Derived from the controls the body and its sections name
/// ([`CloudBackupScreen::drawn_controls`]), one entry for each action however many surfaces draw
/// it; a platform reads an action's availability here and draws it where the shape names it
#[derive(Debug, Clone, PartialEq, Eq, Default, uniffi::Record)]
pub struct CloudBackupActions {
    pub entries: Vec<CloudBackupActionEntry>,
}

/// Where on screen a control is drawn
#[derive(Debug, Clone, Copy, Hash, PartialEq, Eq, uniffi::Enum)]
pub enum CloudBackupControlSurface {
    /// The content of a body other than a configured backup, whose exits it draws
    Body,
    HealNotice,
    PasskeyCard,
    InventoryIncomplete,
    Verification,
    Wallets,
    NotOnDevice,
    OtherBackups,
    TurnOff,
    /// The Turn Off dialogs, while a review or the work before it is open
    TurnOffDialogs,
    /// Android's top-bar menu
    Menu,
    /// The prompt over the screen: the Enable, passkey and verification prompts' choices
    RootPrompt,
}

/// One control the screen's shape draws, and where
#[derive(Debug, Clone, Copy, Hash, PartialEq, Eq, uniffi::Record)]
pub struct CloudBackupDrawnControl {
    pub action: CloudBackupScreenAction,
    pub surface: CloudBackupControlSurface,
}

/// One global action and whether it can run now
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct CloudBackupActionEntry {
    pub action: CloudBackupScreenAction,
    pub availability: CloudBackupActionAvailability,
}

/// Whether a drawn control is enabled
#[derive(Debug, Clone, Copy, Hash, PartialEq, Eq, uniffi::Enum)]
pub enum CloudBackupActionAvailability {
    Available,
    Unavailable { reason: CloudBackupRejection },
}

/// A control that dispatches one intent; the platform maps each to its `CloudBackupManagerAction`
///
/// Its name in lower camel case is its part of the control's accessibility identifier
#[derive(
    Debug, Clone, Copy, Hash, PartialEq, Eq, strum::EnumIter, strum::IntoStaticStr, uniffi::Enum,
)]
#[strum(serialize_all = "camelCase")]
pub enum CloudBackupScreenAction {
    Enable,
    RestoreFromCloudBackup,
    CancelRestore,
    ConfirmSavedPasskey,
    /// The Enable prompt's Use Existing Passkey, offered on every Enable prompt
    AcceptEnablePromptUseExisting,
    /// The Enable prompt's Create New Backup, offered only on a prompt that can start one
    AcceptEnablePromptCreateNew,
    DiscardPendingEnable,
    ConfirmPendingEnableCleanup,
    Verify,
    RepairPasskey,
    SyncUnsynced,
    /// Check Again: a silent inventory refresh
    RefreshDetail,
    RecreateManifest,
    ReinitializeBackup,
    StartRestoreAll,
    RetryRestoreAllRemaining,
    CancelRestoreAll,
    DeleteUnreadableBackups,
    RecoverOtherBackups,
    DeleteOtherBackups,
    TurnOff,
    ConfirmDisable,
    CancelDisable,
    /// Restore Them First on the Turn Off review, offered while it names wallets not on this
    /// device
    RestoreFirst,
    /// Verify First on the Turn Off review, offered while it has backups Cove couldn't check
    VerifyFirst,
    /// Try Again on a committed Turn Off
    ResumeDisable,
    StopUsingOnThisDevice,
    DismissHealNotice,
    /// Android only
    ReconnectDrive,
    /// Android only
    SwitchDriveAccount,
    /// Android only
    RetryDriveAccountSwitchRecovery,
    /// Android only
    CancelDriveAccountSwitch,
    /// Try Again on a failed load
    RetryLoad,
    /// Resets a corrupted Cloud Backup on this device
    ResetOnThisDevice,
}

impl CloudBackupScreenAction {
    /// The Enable prompt choice's own action
    pub(crate) fn accepting_enable_prompt(choice: CloudBackupEnablePromptChoice) -> Self {
        match choice {
            CloudBackupEnablePromptChoice::UseExisting => Self::AcceptEnablePromptUseExisting,
            CloudBackupEnablePromptChoice::CreateNew => Self::AcceptEnablePromptCreateNew,
        }
    }
}

/// One action on a listed row
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct CloudBackupRowAction {
    pub action: CloudBackupRowActionKind,
    pub availability: CloudBackupActionAvailability,
}

/// What a row control does
#[derive(Debug, Clone, Hash, PartialEq, Eq, uniffi::Enum)]
pub enum CloudBackupRowActionKind {
    Restore,
    /// Delete, with what the platform confirms and the consent its confirmation sends back
    Delete {
        confirmation: CloudBackupDeleteConfirmation,
        consent: CloudBackupRowDeleteConsent,
    },
    /// An extra copy whose replacement isn't confirmed yet: sync first
    BackUpFirst,
    CheckAgain,
    Verify,
}

/// Which control on a row, whatever it currently asks the user to confirm
///
/// The identity of a row control: its shape and availability are read from the row's current
/// [`CloudBackupRowAction`], so a change to either never makes it another control
#[derive(
    Debug, Clone, Copy, Hash, PartialEq, Eq, strum::EnumIter, strum::IntoStaticStr, uniffi::Enum,
)]
#[strum(serialize_all = "camelCase")]
pub enum CloudBackupRowControl {
    Restore,
    Delete,
    BackUpFirst,
    CheckAgain,
    Verify,
}

/// One control on one listed row
///
/// An open row sheet or delete confirmation holds only this, and a row outcome names it; each reads
/// the control's current action from the latest screen with [`CloudBackupScreen::row_action`], so
/// none keeps a copy of a decision Rust has since replaced
#[derive(Debug, Clone, Hash, PartialEq, Eq, uniffi::Record)]
pub struct CloudBackupRowTarget {
    pub record_id: RecordId,
    pub control: CloudBackupRowControl,
}

/// What the platform confirms before a row delete
#[derive(Debug, Clone, Copy, Hash, PartialEq, Eq, uniffi::Enum)]
pub enum CloudBackupDeleteConfirmation {
    Standard,
    /// This may be the only copy of the wallet
    MayBeOnlyCopy,
}

/// A screen as it was published, with the revision that orders it against every delta
///
/// The platform starts from the one `state()` returns and advances it only with
/// [`CloudBackupPublishedScreen::applying`]. Deltas sent before that snapshot was read can
/// still be queued for the platform; the revision is how they are told apart from the ones
/// that come after it
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct CloudBackupPublishedScreen {
    /// How many deltas the manager had published when this screen was current; the screen
    /// before anything is loaded is revision 0
    pub revision: u64,
    pub screen: CloudBackupScreen,
}

/// A change to the screen, with only the parts that changed
///
/// Each platform applies it with [`CloudBackupPublishedScreen::applying`] in one state
/// assignment, so no frame shows a half-applied update
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct CloudBackupScreenDelta {
    /// The revision of the screen this delta publishes; it changes the screen published at the
    /// revision before it
    pub revision: u64,
    pub body: Option<CloudBackupScreenBody>,
    pub settings_row: Option<CloudBackupSettingsRowStatus>,
    pub heal_notice: Option<CloudBackupHealNoticeUpdate>,
    pub last_outcome: Option<CloudBackupLastOutcomeUpdate>,
    pub root_prompt: Option<CloudBackupRootPrompt>,
    pub actions: Option<CloudBackupActions>,
}

/// The new heal notice, which may be none; Kotlin can't nest optionals
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Record)]
pub struct CloudBackupHealNoticeUpdate {
    pub notice: Option<CloudBackupHealNotice>,
}

/// The new last outcome, which may be none; Kotlin can't nest optionals
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct CloudBackupLastOutcomeUpdate {
    pub outcome: Option<CloudBackupLastOutcome>,
}

/// A named screen for the platform contract tests; debug builds only
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct CloudBackupScreenFixture {
    pub name: String,
    pub screen: CloudBackupScreen,
}
