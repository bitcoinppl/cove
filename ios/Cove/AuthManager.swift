import SwiftUI

enum UnlockMode {
    case main, decoy, wipe, locked
}

enum WipePresentationState: Equatable {
    case idle
    case running
    case shutdownBlocked(ShutdownAttemptId)
    case failed(String)
}

private enum WipeCallResult: Sendable {
    case success
    case failure(AppError)
    case unexpectedFailure(String)
}

@Observable final class AuthManager: AuthManagerReconciler {
    static let shared = makeShared()

    private let logger = Log(id: "AuthManager")
    var rust: RustAuthManager
    var type = Database().globalConfig().authType()
    var lockState: LockState = .locked
    var isWipeDataPinEnabled: Bool
    var isDecoyPinEnabled: Bool
    var wipePresentationState = WipePresentationState.idle

    @ObservationIgnored
    var lockedAt: Date? {
        guard let lockedAt = rust.lockedAt() else { return nil }
        return Date(timeIntervalSince1970: Double(lockedAt))
    }

    @MainActor
    var isUsingBiometrics: Bool = false

    private static func makeShared() -> AuthManager {
        requireBootstrapComplete()
        return AuthManager()
    }

    private static func requireBootstrapComplete() {
        if ProcessInfo.processInfo.environment["XCODE_RUNNING_FOR_PREVIEWS"] == "1" {
            return
        }

        let step = bootstrapProgress()
        guard step == .complete else {
            fatalError("AuthManager initialized before bootstrap completed: \(step)")
        }
    }

    private init() {
        Log.debug("Initializing AuthManager")

        let rust = RustAuthManager()
        self.rust = rust

        isWipeDataPinEnabled = rust.isWipeDataPinEnabled()
        isDecoyPinEnabled = rust.isDecoyPinEnabled()

        rust.listenForUpdates(reconciler: self)
    }

    public func isInDecoyMode() -> Bool {
        rust.isInDecoyMode()
    }

    public func lock() {
        guard isAuthEnabled else { return }
        let now = UInt64(Date.now.timeIntervalSince1970)
        Log.debug("[AUTH] locking at \(now)")
        lockState = .locked
        try? rust.setLockedAt(lockedAt: now)
    }

    public func unlock() {
        lockState = .unlocked
        try? rust.setLockedAt(lockedAt: 0)
    }

    public var isAuthEnabled: Bool {
        type != AuthType.none
    }

    @MainActor
    public func checkPin(_ pin: String) -> Bool {
        AuthPin().check(pin: pin)
    }

    @MainActor
    public func handleAndReturnUnlockMode(_ pin: String) async -> UnlockMode {
        if AuthPin().check(pin: pin) {
            if Database().globalConfig().isInDecoyMode() { switchToMainMode() }
            unlock()
            return .main
        }

        // check if the entered pin a the decoy pin, if so enter decoy mode
        if checkDecoyPin(pin) {
            // enter decoy mode if not already in decoy mode and reset app and router
            if Database().globalConfig().isInMainMode() {
                rust.switchToDecoyMode()
                unlock()

                let app = AppManager.shared
                app.reset()
                app.isLoading = true

                let db = Database()
                if let selectedWalletId = db.globalConfig().selectedWallet() {
                    do {
                        try app.selectWalletOrThrow(selectedWalletId)
                    } catch {
                        logger.error("Failed to select decoy wallet after auth fallback: \(error)")
                        app.isLoading = false
                        app.loadAndReset(to: RouteFactory().newWalletSelect())
                    }
                } else {
                    app.loadAndReset(to: RouteFactory().newWalletSelect())
                }
            }

            return .decoy
        }

        // check if the entered pin is a wipeDataPin
        // if so wipe the data
        if checkWipeDataPin(pin) {
            wipePresentationState = .running
            return await finishWipe(call: .initial)
        }

        return .locked
    }

    @MainActor
    func retryWipe(_ attemptId: ShutdownAttemptId) async {
        wipePresentationState = .running
        _ = await finishWipe(call: .retry(attemptId))
    }

    @MainActor
    func cancelWipe(_ attemptId: ShutdownAttemptId) {
        AppManager.shared.rust.cancelDangerousWipe(attemptId: attemptId)
        wipePresentationState = .idle
    }

    @MainActor
    func clearWipeFailure() {
        wipePresentationState = .idle
    }

    private enum WipeCall: Sendable {
        case initial
        case retry(ShutdownAttemptId)
    }

    @MainActor
    private func finishWipe(call: WipeCall) async -> UnlockMode {
        let app = AppManager.shared.rust
        let result = await Task.detached(priority: .userInitiated) {
            do {
                switch call {
                case .initial:
                    try app.dangerousWipeAllData()
                case let .retry(attemptId):
                    try app.retryDangerousWipeAllData(attemptId: attemptId)
                }

                return WipeCallResult.success
            } catch let error as AppError {
                return WipeCallResult.failure(error)
            } catch {
                return WipeCallResult.unexpectedFailure(error.localizedDescription)
            }
        }.value

        switch result {
        case .success:
            rust = RustAuthManager()
            unlock()
            type = .none
            wipePresentationState = .idle
            AppManager.shared.reset()
            return .wipe

        case let .failure(.WalletLifecycle(.shutdownBlocked(attemptId, _, _))):
            wipePresentationState = .shutdownBlocked(attemptId)
            return .locked

        case let .failure(error):
            logger.error("Failed to wipe all data: \(error)")
            wipePresentationState = .failed(error.localizedDescription)
            return .locked

        case let .unexpectedFailure(message):
            logger.error("Failed to wipe all data: \(message)")
            wipePresentationState = .failed(message)
            return .locked
        }
    }

    @MainActor
    public func switchToMainMode() {
        rust.switchToMainMode()

        let app = AppManager.shared
        app.reset()
        app.isLoading = true

        let db = Database()
        if let selectedWalletId = db.globalConfig().selectedWallet() {
            do {
                try app.selectWalletOrThrow(selectedWalletId)
            } catch {
                logger.error("Failed to select main wallet after auth fallback: \(error)")
                app.isLoading = false
                app.loadAndReset(to: RouteFactory().newWalletSelect())
            }
        } else {
            app.loadAndReset(to: RouteFactory().newWalletSelect())
        }
    }

    public func checkWipeDataPin(_ pin: String) -> Bool {
        rust.checkWipeDataPin(pin: pin)
    }

    public func checkDecoyPin(_ pin: String) -> Bool {
        rust.checkDecoyPin(pin: pin)
    }

    func reconcile(message: AuthManagerReconcileMessage) {
        DispatchQueue.main.async { [weak self] in
            guard let self else { return }
            logger.debug("reconcile: \(message)")

            switch message {
            case let .authTypeChanged(authType):
                type = authType

            case .wipeDataPinChanged:
                isWipeDataPinEnabled = rust.isWipeDataPinEnabled()

            case .decoyPinChanged:
                isDecoyPinEnabled = rust.isDecoyPinEnabled()
            }
        }
    }

    public func dispatch(action: AuthManagerAction) {
        logger.debug("dispatch: \(action)")
        rust.dispatch(action: action)
    }
}
