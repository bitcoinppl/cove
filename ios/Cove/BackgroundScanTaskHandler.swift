import UIKit

final class BackgroundScanTaskHandler {
    private let logger = Log(id: "BackgroundScanTaskHandler")

    private var initialScanBackgroundTask: UIBackgroundTaskIdentifier = .invalid
    private weak var initialScanBackgroundTaskWalletManager: WalletManager?
    private var initialScanBackgroundTaskAllowed = false

    func observeInitialScanLifecycle(
        for walletManager: WalletManager,
        currentWalletManager: @escaping () -> WalletManager?
    ) {
        walletManager.setInitialScanLifecycleChanged { [weak self, weak walletManager] in
            DispatchQueue.main.async { [weak self, weak walletManager] in
                guard let self, let walletManager else { return }
                guard currentWalletManager() === walletManager else { return }

                self.updateInitialScanBackgroundTask(walletManager: walletManager)
            }
        }
    }

    func beginInitialScanBackgroundTaskIfNeeded(walletManager: WalletManager?) {
        initialScanBackgroundTaskAllowed = true
        updateInitialScanBackgroundTask(walletManager: walletManager)
    }

    func endInitialScanBackgroundTask() {
        initialScanBackgroundTaskAllowed = false
        endInitialScanBackgroundTaskHandle()
    }

    func updateInitialScanBackgroundTask(walletManager: WalletManager?) {
        guard let walletManager, walletManager.activeIncompleteInitialScan else {
            endInitialScanBackgroundTaskHandle()
            return
        }

        guard initialScanBackgroundTaskAllowed else {
            endInitialScanBackgroundTaskHandle()
            return
        }

        guard initialScanBackgroundTask == .invalid else {
            endInitialScanBackgroundTaskIfInactive(walletManager: walletManager)
            return
        }

        let backgroundTask = UIApplication.shared.beginBackgroundTask(
            withName: "Initial wallet scan"
        ) { [weak self] in
            DispatchQueue.main.async { [weak self] in
                Log.warn("Initial wallet scan background task expired")
                self?.endInitialScanBackgroundTask()
            }
        }

        guard backgroundTask != .invalid else {
            Log.warn("Unable to start initial wallet scan background task")
            return
        }

        initialScanBackgroundTask = backgroundTask
        initialScanBackgroundTaskWalletManager = walletManager
        logger.debug("Started initial wallet scan background task for wallet \(walletManager.id)")

        endInitialScanBackgroundTaskIfInactive(walletManager: walletManager)
    }

    private func endInitialScanBackgroundTaskHandle() {
        guard initialScanBackgroundTask != .invalid else { return }

        let backgroundTask = initialScanBackgroundTask
        initialScanBackgroundTask = .invalid
        initialScanBackgroundTaskWalletManager = nil

        UIApplication.shared.endBackgroundTask(backgroundTask)
        logger.debug("Ended initial wallet scan background task")
    }

    private func endInitialScanBackgroundTaskIfInactive(walletManager: WalletManager?) {
        guard initialScanBackgroundTask != .invalid else { return }
        guard let walletManager,
              let initialScanBackgroundTaskWalletManager,
              walletManager === initialScanBackgroundTaskWalletManager,
              walletManager.activeIncompleteInitialScan
        else {
            endInitialScanBackgroundTaskHandle()
            return
        }
    }
}
