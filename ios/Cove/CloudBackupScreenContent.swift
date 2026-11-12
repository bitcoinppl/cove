import Foundation

// Readers for the content Rust put on the Cloud Backup screen
//
// These pick a value out of the screen; none decides whether a control is enabled, which only the
// screen's `actions` and each row's own actions say

extension CloudBackupScreen {
    /// The configured screen, when the body is a configured backup
    var configured: CloudBackupConfiguredScreen? {
        guard case let .configured(configured) = body else { return nil }
        return configured
    }

    /// The Enable flow in progress, when the body is Enabling
    var enableFlow: CloudBackupEnableFlow? {
        guard case let .enabling(flow) = body else { return nil }
        return flow
    }

    /// The Enable paused on the passkey it registered, when that pause owns the screen
    var pausedEnable: CloudBackupSavedPasskeyScreen? {
        guard case let .savedPasskeyConfirmation(paused) = body, paused.lifecycle == .enable else {
            return nil
        }

        return paused
    }

    /// The interrupted Enable, when the body is the pending-enable recovery
    var pendingEnableRecovery: CloudBackupPendingEnableRecovery? {
        guard case let .pendingEnableRecovery(recovery) = body else { return nil }
        return recovery
    }

    /// The verification the configured screen shows, wherever Rust placed it
    var verification: CloudBackupVerificationView? {
        configured?.verification
    }

    /// The last outcome when it answers `action`, so it is drawn next to that control
    func lastOutcome(for action: CloudBackupScreenAction) -> CloudBackupOutcome? {
        guard let lastOutcome, case .action(action) = lastOutcome.target else { return nil }
        return lastOutcome.outcome
    }

    /// The last outcome when it answers a control on the Enable sheet
    var enableSheetOutcome: CloudBackupOutcome? {
        guard let lastOutcome, case let .action(action) = lastOutcome.target else { return nil }

        switch action {
        case .enable, .acceptEnablePromptUseExisting, .acceptEnablePromptCreateNew, .confirmSavedPasskey,
             .discardPendingEnable, .confirmPendingEnableCleanup:
            return lastOutcome.outcome
        default:
            return nil
        }
    }

    /// Whether iOS draws a control for `action`: Rust names one on a surface iOS draws
    func drawsOnIOS(_ action: CloudBackupScreenAction) -> Bool {
        drawnControls().contains { $0.action == action && !$0.isAndroidOnly() }
    }

    /// The last outcome when no drawn control answers it, so it is drawn at the top instead
    var unplacedOutcome: CloudBackupOutcome? {
        guard let lastOutcome else { return nil }

        switch lastOutcome.target {
        case let .action(action):
            return drawsOnIOS(action) ? nil : lastOutcome.outcome
        case let .row(target):
            return listedRow(recordId: target.recordId) == nil ? lastOutcome.outcome : nil
        }
    }
}

extension CloudBackupConfiguredScreen {
    /// The verification view, from its own section or from the progress section that shows it
    /// running
    var verification: CloudBackupVerificationView? {
        for section in sections {
            switch section {
            case let .verification(section):
                return section.view
            case let .progress(.verificationCard(view)),
                 let .progress(.verificationInline(view)):
                return view
            default:
                continue
            }
        }

        return nil
    }

    /// This device's wallets and their sync state, when the wallets section is drawn
    var wallets: CloudBackupWalletsSection? {
        for case let .wallets(wallets) in sections {
            return wallets
        }

        return nil
    }

    /// The passkey card's state, when the configured screen shows one
    var passkeyCard: CloudBackupPasskeyState? {
        for case let .passkeyCard(state) in sections {
            return state
        }

        return nil
    }
}

extension CloudBackupEnableFlow {
    /// Whether Cove is working on this step, rather than waiting for the user to answer a prompt
    var isWorking: Bool {
        switch self {
        case .awaitingForceNewConfirmation, .awaitingPasskeyChoice,
             .awaitingSavedPasskeyConfirmation(.manual):
            false
        case .discoveringExistingBackup, .creatingPasskey, .awaitingSavedPasskeyConfirmation(.automatic),
             .confirmingSavedPasskey, .uploadingInitialBackup, .retryingUploadWithStagedMaterial,
             .waitingForPasskeyAvailability:
            true
        }
    }
}

extension CloudBackupActionAvailability {
    var isAvailable: Bool {
        self == .available
    }

    /// Why the control is disabled, to show as its caption
    var unavailableReason: CloudBackupRejection? {
        guard case let .unavailable(reason) = self else { return nil }
        return reason
    }
}

extension CloudBackupActionAvailability? {
    /// Whether a drawn control is enabled; a missing entry draws no control
    var isAvailable: Bool {
        self == .available
    }
}

extension CloudBackupRejection {
    /// The caption shown under a control this rejection disables
    var caption: String {
        switch self {
        case .cloudBackupOff:
            "Cloud Backup is off."
        case .notConfigured:
            "Cloud Backup is already on."
        case .loadFailed:
            "Cove couldn't load Cloud Backup."
        case .turnOffCommitted:
            "Cloud Backup is being turned off."
        case let .operationRunning(operation):
            "Waiting for \(operation.displayName) to finish."
        case .inventoryNotLoaded:
            "Waiting for Cove to finish checking iCloud."
        case .otherBackupsNotLoaded:
            "Waiting for Cove to finish checking for other backups."
        case .restoreAllUnavailable:
            "Restore All isn't available right now."
        case .nothingToCancel:
            "Nothing to cancel."
        case .nothingToContinue:
            "Nothing to continue."
        case .reviewStale:
            "Cloud Backup changed since you checked."
        case .resetting:
            "Cove is resetting Cloud Backup on this device."
        case .tryAgainFirst:
            "Available after you try again."
        }
    }
}

extension CloudBackupOperationKind {
    /// The running work as a caption names it
    var displayName: String {
        switch self {
        case .enable: "Cloud Backup setup"
        case .restore: "the restore"
        case .turnOff, .resumeTurnOff: "Turn Off"
        case .verify: "verification"
        case .repairPasskey: "the passkey repair"
        case .restoreCloudWallet: "the wallet restore"
        case .restoreAll: "Restore All"
        case .deleteRecords: "the backup delete"
        case .recoverOtherBackups: "the other-backup recovery"
        case .deleteOtherBackups: "the other-backup delete"
        case .reinitialize: "the backup reinitialize"
        case .recreateManifest: "the backup index rebuild"
        case .discardPendingEnable: "the setup cleanup"
        case .driveAccountSwitch: "the account switch"
        }
    }
}

extension CloudBackupOutcome {
    /// The message drawn next to the control the outcome answers
    var message: String {
        switch self {
        case let .rejected(reason):
            reason.caption
        case let .disableNotStarted(reason):
            "Cloud Backup is still on. \(reason.message)"
        case .passkeyNotCompleted:
            "The passkey request didn't finish. Nothing changed."
        case let .failed(message):
            message
        }
    }
}

extension CloudBackupDisableNotStartedReason {
    var message: String {
        switch self {
        case .offline:
            "Cove can't reach iCloud right now."
        case .authorizationRequired:
            "Turn on iCloud Drive for Cove, then try again."
        case .providerUnavailable:
            "iCloud Drive isn't available right now."
        case .writesDidNotDrain:
            "Cove couldn't finish saving recent changes first."
        case .saveFailed:
            "Cove couldn't save the change on this device."
        }
    }
}

extension CloudBackupInventoryIncompleteReason {
    /// Why Cove couldn't confirm the iCloud inventory, with what to do next
    var inventoryMessage: String {
        switch self {
        case .providerSyncPending:
            "iCloud Drive is still loading Cove backup files. Keep Cove open, then check again."
        case .offline:
            "This device is offline. Connect to the internet, then check iCloud Drive again."
        case .authorizationRequired:
            "Cove cannot access iCloud Drive. Turn on iCloud Drive for Cove, then check again."
        case .providerUnavailable:
            "iCloud Drive is not available now. Try again later."
        case .unknown:
            "Cove could not load Cloud Backup details. Try again."
        }
    }

    /// Why Cove couldn't check for backups made with another key
    var otherBackupsMessage: String {
        switch self {
        case .providerSyncPending:
            "Cove could not check for backups with another key because iCloud Drive is still syncing. Check again when syncing is complete."
        case .offline:
            "Cove could not check for backups with another key because this device is offline. Connect to the internet, then check again."
        case .authorizationRequired:
            "Cove cannot check for backups with another key. Turn on iCloud Drive for Cove, then check again."
        case .providerUnavailable:
            "Cove cannot check for backups with another key because iCloud Drive is not available now. Try again later."
        case .unknown:
            "Cove could not check for backups with another key. Try again."
        }
    }
}

extension CloudBackupSection {
    /// A stable identity for the section, since each kind appears at most once
    var kind: String {
        switch self {
        case .passkeyCard: "passkeyCard"
        case .progress: "progress"
        case .verification: "verification"
        case .wallets: "wallets"
        case .notOnDevice: "notOnDevice"
        case .extraCopies: "extraCopies"
        case .otherBackups: "otherBackups"
        case .turnOff: "turnOff"
        }
    }
}

extension CloudBackupNamedCount {
    /// The names Rust chose to show, then how many more, such as "A, B, C and 2 more"
    var listText: String {
        var parts = names
        if more > 0 { parts.append("\(more) more") }

        guard parts.count > 1, let last = parts.last else { return parts.first ?? "" }
        return "\(parts.dropLast().joined(separator: ", ")) and \(last)"
    }

    /// The number of wallets the names stand for
    var total: UInt32 {
        UInt32(names.count) + more
    }
}

extension CloudBackupRowActionKind {
    var title: String {
        switch self {
        case .restore: "Restore"
        case .delete: "Delete"
        case .backUpFirst: "Back Up First"
        case .checkAgain: "Check Again"
        case .verify: "Verify"
        }
    }

    var systemImage: String {
        switch self {
        case .restore: "icloud.and.arrow.down"
        case .delete: "trash"
        case .backUpFirst: "icloud.and.arrow.up"
        case .checkAgain: "arrow.clockwise"
        case .verify: "checkmark.shield"
        }
    }
}

extension CloudBackupListedRecord {
    /// The wallet the record holds, when Cove could read which wallet it is
    var wallet: CloudBackupWalletItem? {
        switch self {
        case let .notOnDevice(wallet), let .extraCopy(wallet, _):
            wallet
        case .unsupported, .unreadable, .unchecked:
            nil
        }
    }

    /// The row's title when Cove can't name the wallet
    var unnamedTitle: String {
        switch self {
        case .notOnDevice, .extraCopy:
            "Wallet backup"
        case .unsupported:
            "Backup from a newer version of Cove"
        case .unreadable:
            "Backup Cove can't read"
        case .unchecked:
            "Backup Cove couldn't check"
        }
    }

    /// What the row is, under its title
    var caption: String? {
        switch self {
        case .notOnDevice:
            nil
        case .extraCopy:
            "Already on this device under another backup"
        case .unsupported:
            "Update Cove to restore it."
        case let .unreadable(kind, _):
            switch kind {
            case .decrypt: "It can't be decrypted with this backup key."
            case .parse: "Its data is damaged."
            case .notDownloadable: "It couldn't be downloaded from iCloud."
            case .provider: "iCloud couldn't open it."
            }
        case .unchecked:
            "Cove needs your passkey to check it. It may be a wallet that isn't on this device."
        }
    }
}
