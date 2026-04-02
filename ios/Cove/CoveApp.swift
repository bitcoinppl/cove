//
//  CoveApp.swift
//  Cove
//
//  Created by Praveen Perera  on 6/17/24.
//

@_exported import CoveCore
import MijickPopups
import Network
import SwiftUI

extension EnvironmentValues {
    @Entry var navigate: (Route) -> Void = { _ in }
}

struct SafeAreaInsetsKey: EnvironmentKey {
    static var defaultValue: EdgeInsets {
        #if os(iOS) || os(tvOS)
            let window = (UIApplication.shared.connectedScenes.first as? UIWindowScene)?.keyWindow
            guard let insets = window?.safeAreaInsets else { return EdgeInsets() }
            return EdgeInsets(
                top: insets.top, leading: insets.left, bottom: insets.bottom, trailing: insets.right
            )
        #else
            return EdgeInsets()
        #endif
    }
}

public extension EnvironmentValues {
    var safeAreaInsets: EdgeInsets {
        self[SafeAreaInsetsKey.self]
    }
}

@main
struct CoveApp: App {
    @UIApplicationDelegateAdaptor(CoveAppDelegate.self) var appDelegate
    enum StartupState {
        case loading
        case ready(AppManager, AuthManager)
        case onboarding(AppManager, AuthManager)
        case catastrophicError
        case fatalError(String)
    }

    @State private var startupState: StartupState = .loading
    @State private var bdkMigrationWarning: String?
    @State private var bootstrapRequestID = 0

    init() {
        _ = Keychain(keychain: KeychainAccessor())
        _ = Device(device: DeviceAccesor())
        _ = PasskeyAccess(provider: PasskeyProviderImpl())
        _ = CloudStorage(cloudStorage: CloudStorageAccessImpl())
        Self.excludeDataDirFromBackup(logFailure: false)
    }

    private static func excludeDataDirFromBackup(logFailure: Bool) {
        let path = rootDataDirPath()
        var url = URL(fileURLWithPath: path, isDirectory: true)
        do {
            var values = URLResourceValues()
            values.isExcludedFromBackup = true
            try url.setResourceValues(values)
        } catch {
            if logFailure {
                Log.error("Failed to set isExcludedFromBackup on data dir: \(error)")
            }
        }
    }

    var body: some Scene {
        WindowGroup {
            startupContent
                .task(id: bootstrapRequestID) {
                    await runBootstrap()
                }
                .alert(
                    "Encryption Migration Issue",
                    isPresented: Binding(
                        get: { bdkMigrationWarning != nil },
                        set: { if !$0 { bdkMigrationWarning = nil } }
                    )
                ) {
                    Button("OK") { bdkMigrationWarning = nil }
                } message: {
                    Text(
                        "Some wallet databases couldn't be encrypted. Your wallets still work and encryption will retry on next launch.\n\nIf this persists, please contact feedback@covebitcoinwallet.com"
                    )
                }
        }
    }
}

extension CoveApp {
    @MainActor
    private func runBootstrap() async {
        do {
            let warning = try await bootstrapWithTimeout()
            completeBootstrap(warning: warning)
        } catch {
            handleBootstrapError(error)
        }
    }

    @ViewBuilder
    private var startupContent: some View {
        switch startupState {
        case .loading:
            CoverView(errorMessage: nil)
        case let .ready(app, auth):
            CoveMainView(app: app, auth: auth)
        case let .onboarding(app, auth):
            OnboardingContainer(manager: OnboardingManager(app: app), auth: auth) {
                startupState = .ready(app, auth)
                startBackupIntegrityCheck()
            }
        case .catastrophicError:
            CatastrophicErrorView(
                onRestoreFromCloud: {
                    resetCatastrophicRecoveryStateAndRebootstrap()
                },
                onWipeOnly: {
                    resetCatastrophicRecoveryStateAndRebootstrap()
                }
            )
        case let .fatalError(message):
            CoverView(errorMessage: message)
        }
    }

    private func resetCatastrophicRecoveryStateAndRebootstrap() {
        startupState = .loading

        do {
            try resetLocalDataForCatastrophicRecovery()
            rebootstrap()
        } catch {
            startupState = .fatalError("Failed to reset local data: \(error.localizedDescription)")
        }
    }

    private func bootstrapWithTimeout() async throws -> String? {
        try await withThrowingTaskGroup(of: BootstrapResult.self) { group in
            group.addTask { try await .completed(warning: bootstrap()) }
            group.addTask { try await self.bootstrapWatchdog() }

            guard let result = try await group.next() else { throw BootstrapTimeoutError() }
            group.cancelAll()

            switch result {
            case let .completed(warning): return warning
            case .timedOut: throw BootstrapTimeoutError()
            }
        }
    }

    /// Adaptive timeout watchdog — extends timeout when migration is detected
    private func bootstrapWatchdog() async throws -> BootstrapResult {
        let startTime = ContinuousClock.now
        var migrationDetected = false

        while !Task.isCancelled {
            try await Task.sleep(for: .milliseconds(66))

            if !migrationDetected {
                let step = bootstrapProgress()
                if step.isMigrationInProgress() {
                    migrationDetected = true
                } else if let progress = activeMigration()?.progress(), progress.total > 0 {
                    migrationDetected = true
                }
            }

            let elapsed = ContinuousClock.now - startTime
            // shorter timeout since iOS hardware is more uniform
            let timeout: Duration = migrationDetected ? .seconds(20) : .seconds(10)
            if elapsed >= timeout {
                Log.warn("[STARTUP] watchdog firing after \(elapsed) (timeout=\(timeout), migration=\(migrationDetected))")
                cancelBootstrap()
                return .timedOut
            }
        }
        return .timedOut
    }

    private func handleBootstrapError(_ error: Error) {
        if error is BootstrapTimeoutError {
            let step = bootstrapProgress()
            if step == .complete {
                Log.warn("[STARTUP] bootstrap completed despite timeout — migration warning (if any) was lost and will retry on next launch")
                completeBootstrap()
            } else {
                Log.error("[STARTUP] bootstrap timed out, last step: \(step)")
                startupState = .fatalError(
                    "App startup timed out. Please force-quit and try again.\n\nPlease contact feedback@covebitcoinwallet.com"
                )
            }
        } else if error is CancellationError {
            Log.info("[STARTUP] bootstrap task cancelled (app lifecycle)")
        } else {
            let step = bootstrapProgress()
            if step == .complete {
                Log.warn("[STARTUP] bootstrap completed despite error — treating as success")
                completeBootstrap()
            } else if case AppInitError.DatabaseKeyMismatch = error {
                Log.error("[STARTUP] database encryption key mismatch")
                startupState = .catastrophicError
            } else if case AppInitError.AlreadyCalled = error {
                Log.error("[STARTUP] bootstrap already called at step: \(step)")
                startupState = .fatalError(
                    "App initialization error. Please force-quit and restart."
                )
            } else if case AppInitError.Cancelled = error {
                Log.error("[STARTUP] bootstrap cancelled at step: \(step)")
                startupState = .fatalError(
                    "App startup timed out. Please force-quit and try again.\n\nPlease contact feedback@covebitcoinwallet.com"
                )
            } else {
                Log.error("[STARTUP] bootstrap failed at step: \(step), error: \(error)")
                startupState = .fatalError(error.localizedDescription)
            }
        }
    }

    private func completeBootstrap(warning: String? = nil) {
        Log.info("[STARTUP] completeBootstrap called")

        // unconditional initialization — everything ready before any user interaction
        initializeApp()
        Self.excludeDataDirFromBackup(logFailure: true)
        let appManager = AppManager.shared
        appManager.asyncRuntimeReady = true
        CloudConnectivityMonitor.shared.start()
        CloudBackupManager.shared.rust.syncPersistedState()
        self.bdkMigrationWarning = warning
        startInitData(appManager)

        let needsOnboarding = !appManager.isTermsAccepted
            || shouldRunCloudRestoreCheck(appManager: appManager)

        if needsOnboarding {
            Log.info("[STARTUP] entering onboarding flow")
            self.startupState = .onboarding(appManager, AuthManager.shared)
        } else {
            Log.info("[STARTUP] going to ready state")
            self.startupState = .ready(appManager, AuthManager.shared)
            startBackupIntegrityCheck()
        }
    }

    private func shouldRunCloudRestoreCheck(appManager: AppManager) -> Bool {
        guard appManager.isTermsAccepted else { return false }
        guard case .disabled = CloudBackupManager.shared.status else { return false }
        do {
            guard try !appManager.database.wallets().hasAnyWallets() else { return false }
        } catch {
            Log.error("[STARTUP] failed to check for existing wallets before restore onboarding: \(error)")
            return false
        }
        guard FileManager.default.ubiquityIdentityToken != nil else { return false }
        return true
    }

    /// Re-bootstrap after recovery (Start Fresh / Wipe / Cloud Restore)
    private func rebootstrap() {
        resetBootstrapForRestore()
        bootstrapRequestID += 1
    }

    /// Non-blocking — initData preloads caches and prices but is not required for core functionality
    private func startInitData(_ appManager: AppManager) {
        Task {
            await appManager.rust.initData()
            Log.info("[STARTUP] initData completed")
        }
    }

    /// Background check that cloud backup files and keychain master key are intact
    private func startBackupIntegrityCheck() {
        Task {
            CloudBackupManager.shared.rust.resumePendingCloudUploadVerification()

            let isICloudAvailable = await MainActor.run { FileManager.default.ubiquityIdentityToken != nil }
            guard isICloudAvailable else { return }

            let warning = await Task.detached {
                CloudBackupManager.shared.rust.verifyBackupIntegrity()
            }.value
            if let warning { Log.error("[STARTUP] backup integrity warning: \(warning)") }
        }
    }
}

private enum BootstrapResult {
    case completed(warning: String?)
    case timedOut
}

private struct BootstrapTimeoutError: LocalizedError {
    var errorDescription: String? {
        "bootstrap timed out"
    }
}

final class CloudConnectivityMonitor: @unchecked Sendable {
    static let shared = CloudConnectivityMonitor()

    private let monitor = NWPathMonitor()
    private let queue = DispatchQueue(label: "cove.CloudConnectivityMonitor")
    private let lock = NSLock()
    private var started = false

    private init() {}

    func start() {
        lock.lock()
        defer { lock.unlock() }
        guard !started else { return }
        started = true

        monitor.pathUpdateHandler = { path in
            let hint: CloudConnectivityHint =
                if path.status == .satisfied {
                    .online
                } else if path.status == .unsatisfied {
                    .offline
                } else {
                    .unknown
                }

            CloudBackupManager.shared.rust.updateConnectivityHint(hint: hint)
        }

        monitor.start(queue: queue)
    }
}
