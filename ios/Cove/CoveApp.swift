//
//  CoveApp.swift
//  Cove
//
//  Created by Praveen Perera  on 6/17/24.
//

@_exported import CoveCore
import MijickPopups
import SwiftUI

extension EnvironmentValues {
    @Entry var navigate: (Route) -> Void = { _ in }
}

struct SafeAreaInsetsKey: EnvironmentKey {
    static var defaultValue: EdgeInsets {
        #if os(iOS) || os(tvOS)
            let window = (UIApplication.shared.connectedScenes.first as? UIWindowScene)?.keyWindow
            guard let insets = window?.safeAreaInsets else {
                return EdgeInsets()
            }
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
    @State private var app: AppManager?
    @State private var auth: AuthManager?
    @State private var bootstrapError: String?
    @State private var bdkMigrationWarning: String?

    init() {
        _ = Keychain(keychain: KeychainAccessor())
        _ = Device(device: DeviceAccesor())
    }

    var body: some Scene {
        WindowGroup {
            Group {
                if let app, let auth {
                    CoveMainView(app: app, auth: auth)
                } else {
                    CoverView(errorMessage: bootstrapError)
                }
            }
            .task {
                do {
                    let warning = try await bootstrapWithTimeout()
                    completeBootstrap(warning: warning)
                } catch {
                    handleBootstrapError(error)
                }
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
    private func bootstrapWithTimeout() async throws -> String? {
        try await withThrowingTaskGroup(of: BootstrapResult.self) { group in
            group.addTask { try await .completed(warning: bootstrap()) }
            group.addTask { try await self.bootstrapWatchdog() }

            guard let result = try await group.next() else {
                throw BootstrapTimeoutError()
            }
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
                bootstrapError =
                    "App startup timed out. Please force-quit and try again.\n\nPlease contact feedback@covebitcoinwallet.com"
            }
        } else if error is CancellationError {
            Log.info("[STARTUP] bootstrap task cancelled (app lifecycle)")
        } else {
            let step = bootstrapProgress()
            if step == .complete {
                Log.warn("[STARTUP] bootstrap completed despite error — treating as success")
                completeBootstrap()
            } else if case AppInitError.AlreadyCalled = error {
                Log.error("[STARTUP] bootstrap already called at step: \(step)")
                bootstrapError =
                    "App initialization error. Please force-quit and restart."
            } else if case AppInitError.Cancelled = error {
                Log.error("[STARTUP] bootstrap cancelled at step: \(step)")
                bootstrapError =
                    "App startup timed out. Please force-quit and try again.\n\nPlease contact feedback@covebitcoinwallet.com"
            } else {
                Log.error("[STARTUP] bootstrap failed at step: \(step), error: \(error)")
                bootstrapError = error.localizedDescription
            }
        }
    }

    private func completeBootstrap(warning: String? = nil) {
        let appManager = AppManager.shared
        appManager.asyncRuntimeReady = true
        self.app = appManager
        self.auth = AuthManager.shared
        self.bdkMigrationWarning = warning
        startInitData(appManager)
    }

    /// Non-blocking — initData preloads caches and prices but is not required for core functionality
    private func startInitData(_ appManager: AppManager) {
        Task {
            await appManager.rust.initData()
            Log.info("[STARTUP] initData completed")
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
