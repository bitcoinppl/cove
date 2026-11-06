import CoveCore
import Foundation
import Observation

/// Writes the cloud backup manager's state whenever it changes, so a UI test can check what the
/// app holds rather than what the lazily built Cloud Backup form happens to have on screen
///
/// A scenario test that names `COVE_CLOUD_BACKUP_UI_DELETE_LOG` gets it in
/// `<COVE_CLOUD_BACKUP_UI_DELETE_LOG>.state`, and a test with a control directory gets it in that
/// directory's `app-state.json`
@MainActor
enum CloudBackupUITestStateProbe {
    static func start() async {
        guard let path = outputPath() else { return }

        // creating CloudBackupManager.shared spawns rust actors, so wait until bootstrap has set
        // its last step (rust/src/bootstrap.rs:132); the app's runBootstrap then resumes on the
        // main actor and its completeBootstrap creates the shared manager (ios/Cove/CoveApp.swift:171
        // and :346), which the pause leaves it time to do first
        do {
            while bootstrapProgress() != .complete {
                try await Task.sleep(for: .milliseconds(100))
            }
            try await Task.sleep(for: .seconds(1))
        } catch {
            return
        }

        observe(path: path)

        // rewrites the latest state on a slow timer too, so a failed write doesn't leave the file
        // stale until the next manager change
        while !Task.isCancelled {
            try? await Task.sleep(for: .milliseconds(500))
            write(snapshot(CloudBackupManager.shared), to: path)
        }
    }

    private static func outputPath() -> String? {
        let environment = ProcessInfo.processInfo.environment
        if let logPath = environment["COVE_CLOUD_BACKUP_UI_DELETE_LOG"] {
            return logPath + ".state"
        }

        guard let directory = environment[CloudBackupUITestControl.environmentKey], !directory.isEmpty else {
            return nil
        }

        return (directory as NSString).appendingPathComponent(CloudBackupUITestControl.appStateFileName)
    }

    private static func observe(path: String) {
        let state = withObservationTracking {
            snapshot(CloudBackupManager.shared)
        } onChange: {
            // onChange fires before the new value is set, so read it once the change has landed
            Task { @MainActor in observe(path: path) }
        }

        write(state, to: path)
    }

    private static func write(_ state: CloudBackupUITestAppState, to path: String) {
        // atomic, since the test polls the file while the host writes it
        do {
            try JSONEncoder().encode(state).write(to: URL(fileURLWithPath: path), options: .atomic)
        } catch {
            print("failed to write the cloud backup UI test state: \(error)")
        }
    }

    private static func snapshot(_ manager: CloudBackupManager) -> CloudBackupUITestAppState {
        let detail = switch manager.configuredState?.detail {
        case nil: "none"
        case .notLoaded: "notLoaded"
        case .checking: "checking"
        case .complete: manager.isDetailInventoryComplete ? "complete" : "completeUnconfirmed"
        case .failed: "failed"
        }

        let operation = switch manager.cloudOnlyOperation {
        case .idle: "idle"
        case .operating: "operating"
        case .warning: "warning"
        case .failed: "failed"
        }

        var cloudOnly: [String] = []
        var cloudOnlyIncomplete = false
        if case let .loaded(wallets, incomplete) = manager.cloudOnly {
            cloudOnly = wallets.map(\.recordId)
            cloudOnlyIncomplete = incomplete
        }

        return CloudBackupUITestAppState(
            detail: detail,
            operation: operation,
            cloudOnly: cloudOnly,
            cloudOnlyIncomplete: cloudOnlyIncomplete,
            inventoryReady: manager.isDetailInventoryReady,
            inventoryComplete: manager.isDetailInventoryComplete,
            inventoryError: manager.detailError,
            restoreAll: restoreAllDescription(manager.restoreAllState),
            verification: verificationDescription(manager.verificationState),
            pendingUploadVerification: manager.hasPendingUploadVerification,
            sync: syncDescription(manager.syncState),
            syncError: manager.syncError,
            wallets: walletRows(manager.detail),
            lifecycle: lifecycleDescription(manager.lifecycle),
            settingsRow: settingsRowDescription(manager.settingsRowStatus)
        )
    }

    private static func walletRows(_ detail: CloudBackupDetail?) -> [CloudBackupUITestAppState.Wallet] {
        guard let detail else { return [] }

        let upToDate = detail.upToDate.map { walletRow($0, needsSync: false) }
        let needsSync = detail.needsSync.map { walletRow($0, needsSync: true) }
        return upToDate + needsSync
    }

    private static func walletRow(_ item: CloudBackupWalletItem, needsSync: Bool) -> CloudBackupUITestAppState.Wallet {
        let status = switch item.syncStatus {
        case .dirty: "dirty"
        case .uploading: "uploading"
        case .uploadedPendingConfirmation: "uploadedPendingConfirmation"
        case .confirmed: "confirmed"
        case .failed: "failed"
        case .deletedFromDevice: "deletedFromDevice"
        case .alreadyOnDevice: "alreadyOnDevice"
        case .unsupportedVersion: "unsupportedVersion"
        case .remoteStateUnknown: "remoteStateUnknown"
        }

        return .init(recordId: item.recordId, name: item.name, status: status, needsSync: needsSync)
    }

    private static func lifecycleDescription(_ lifecycle: CloudBackupLifecycle) -> String {
        switch lifecycle {
        case .disabled: "disabled"
        case .enabling: "enabling"
        case .restoring: "restoring"
        case .pendingEnableRecovery: "pendingEnableRecovery"
        case .failed: "failed"
        case .configured: "configured"
        }
    }

    private static func settingsRowDescription(_ status: CloudBackupSettingsRowStatus) -> String {
        switch status {
        case .disabled: "disabled"
        case .disabling: "disabling"
        case .settingUp: "settingUp"
        case .restoring: "restoring"
        case .active: "active"
        case .passkeyMissing: "passkeyMissing"
        case .passkeyProviderUnsupported: "passkeyProviderUnsupported"
        case .unverified: "unverified"
        case .confirming: "confirming"
        case .verificationRecommended: "verificationRecommended"
        case .checkingSync: "checkingSync"
        case .syncing: "syncing"
        case .noFiles: "noFiles"
        case .driveUnavailable: "driveUnavailable"
        case .recoveryRequired: "recoveryRequired"
        case .error: "error"
        case .authorizationRequired: "authorizationRequired"
        }
    }

    private static func syncDescription(_ state: CloudBackupSyncState?) -> String {
        switch state {
        case nil: "none"
        case .idle: "idle"
        case .syncing: "syncing"
        case .blocked: "blocked"
        case .failed: "failed"
        }
    }

    private static func verificationDescription(_ state: CloudBackupVerificationState?) -> String {
        switch state {
        case nil: "none"
        case .notVerified: "notVerified"
        case .verified: "verified"
        case .needsAttention: "needsAttention"
        case .required: "required"
        case .running: "running"
        case .awaitingUploadConfirmation: "awaitingUploadConfirmation"
        case .cancelled: "cancelled"
        case .failed: "failed"
        }
    }

    private static func restoreAllDescription(_ state: CloudBackupRestoreAllState) -> String {
        switch state {
        case .notShown: "notShown"
        case let .startDisabled(count): "startDisabled(\(count))"
        case let .startAvailable(count): "startAvailable(\(count))"
        case let .retryDisabled(count): "retryDisabled(\(count))"
        case let .retryAvailable(count): "retryAvailable(\(count))"
        case let .running(completed, total, _, _): "running(\(completed)/\(total))"
        }
    }
}
