import SwiftUI

enum UnlockMode {
    case main, decoy, wipe, locked
}

enum WipePresentationState: Equatable {
    case idle
    case running
    case shutdownBlocked(ShutdownAttemptId)
    case failed

    // the wipe PIN can be entered under duress, so this copy must not reveal that a wipe ran
    static let failureTitle = "Unable to Open Cove"
    static let failureMessage = "Please try again."
}

/// A request for the user to open the app lock with a credential before secret content loads
///
/// Captures the credential unlock count and the main or decoy mode at the time of the request, so
/// only a later PIN, decoy PIN, or biometric unlock in the same mode satisfies it. Create one with
/// `AuthManager.requestCredentialUnlock()` and check it with `AuthManager.credentialUnlockStatus(_:)`
struct CredentialUnlockRequest: Equatable {
    fileprivate let generation: UInt64
    fileprivate let isInDecoyMode: Bool
}

/// Where a `CredentialUnlockRequest` stands against the current app lock
enum CredentialUnlockStatus: Equatable {
    /// The app lock is still waiting for a credential
    case pending

    /// The user opened the app lock with a credential after the request, in the same mode, and the
    /// app has not locked since
    case satisfied

    /// The app locked again after the credential and opened without one, such as the grace-period
    /// release after a brief app switch
    case unlockedWithoutCredential

    /// The app switched between main and decoy mode, so the request no longer applies
    case superseded
}

private enum WipeCallResult: Sendable {
    case success(FullWipeCompletion)
    case failure(AppError)
    case unexpectedFailure(String)
}

@Observable final class AuthManager: AuthManagerReconciler {
    static let shared = makeShared()

    private let logger = Log(id: "AuthManager")
    var rust: RustAuthManager
    var type = Database().globalConfig().authType()
    private(set) var lockState: LockState
    var isWipeDataPinEnabled: Bool
    var isDecoyPinEnabled: Bool
    var wipePresentationState = WipePresentationState.idle

    /// Counts app lock unlocks made with a PIN, decoy PIN, or biometric credential
    ///
    /// Grace-period releases and disabled authentication never increment it, so screens can tell
    /// a presented credential apart from the lock simply opening
    private(set) var credentialUnlockGeneration: UInt64 = 0

    /// Counts every time the app lock engages
    ///
    /// Lets a screen tell that the lock engaged even when a grace-period release has already opened
    /// it again before the screen observed the lock, as when SwiftUI skips updates in the background
    private(set) var lockGeneration: UInt64 = 0

    /// The lock count when the last credential opened the app, so a credential that a later lock
    /// has outlived cannot satisfy a request
    @ObservationIgnored
    private var lastCredentialLockGeneration: UInt64 = 0

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

        lockState = rust.isLocked() ? .locked : .unlocked
        isWipeDataPinEnabled = rust.isWipeDataPinEnabled()
        isDecoyPinEnabled = rust.isDecoyPinEnabled()

        rust.listenForUpdates(reconciler: self)
    }

    public func isInDecoyMode() -> Bool {
        rust.isInDecoyMode()
    }

    /// Lock the app until the user authenticates
    public func lock(reason: AppLockReason) {
        Log.debug("[AUTH] locking for \(reason)")
        lockGeneration += 1
        setLocked(rust.lock(reason: reason))
    }

    /// Lock the app for a send confirmation screen, the returned token releases only this lock
    public func lockForSendConfirmation() -> SendConfirmationLock? {
        guard let lock = rust.lockForSendConfirmation() else { return nil }
        lockGeneration += 1
        lockState = .locked
        return lock
    }

    /// Release the lock a send confirmation screen took, other locks stay in place
    public func releaseSendConfirmationLock(_ lock: SendConfirmationLock) {
        setLocked(rust.releaseSendConfirmationLock(lock: lock))
    }

    /// Skip authentication when the app comes back right after leaving the foreground
    public func releaseBackgroundLockOnForeground() {
        setLocked(rust.releaseBackgroundLockOnForeground())
    }

    /// Unlock when no authentication is set up
    public func unlockIfAuthDisabled() {
        setLocked(rust.unlockIfAuthDisabled())
    }

    /// Unlock after the lock screen verified the user with biometrics
    private func completeBiometricAuthentication() {
        unlockAuthenticated()
    }

    /// Lock state binding for the app lock screen
    ///
    /// The lock screen only reports unlocked after verifying a PIN or biometrics. A PIN has already
    /// unlocked through `handleAndReturnUnlockMode`, so only a still-locked state completes the
    /// biometric unlock here
    var appLockState: Binding<LockState> {
        Binding(
            get: { self.lockState },
            set: { newValue in
                guard newValue == .unlocked, self.lockState == .locked else { return }
                self.completeBiometricAuthentication()
            }
        )
    }

    /// Lock the app with `.reauthentication` before showing secret content
    ///
    /// Returns nil when the app stays unlocked because no app lock is set up, so there is no
    /// credential to ask for
    func requestCredentialUnlock() -> CredentialUnlockRequest? {
        let request = CredentialUnlockRequest(
            generation: credentialUnlockGeneration,
            isInDecoyMode: isInDecoyMode()
        )

        // rust decides whether the lock applies, so an unreadable auth type still asks for a credential
        lock(reason: .reauthentication)
        return lockState == .locked ? request : nil
    }

    func credentialUnlockStatus(_ request: CredentialUnlockRequest) -> CredentialUnlockStatus {
        if isInDecoyMode() != request.isInDecoyMode { return .superseded }
        if lockState == .locked { return .pending }
        if credentialUnlockGeneration > request.generation, lastCredentialLockGeneration == lockGeneration {
            return .satisfied
        }

        return .unlockedWithoutCredential
    }

    /// Only called after the user authenticated with a PIN, biometrics, or a trick PIN
    private func unlockAuthenticated() {
        rust.unlockAuthenticated()

        // count the credential before publishing the unlock so observers never see the lock open without it
        lastCredentialLockGeneration = lockGeneration
        credentialUnlockGeneration += 1
        lockState = .unlocked
    }

    private func setLocked(_ isLocked: Bool) {
        lockState = isLocked ? .locked : .unlocked
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
            unlockAuthenticated()
            return .main
        }

        // check if the entered pin a the decoy pin, if so enter decoy mode
        if checkDecoyPin(pin) {
            // enter decoy mode if not already in decoy mode and reset app and router
            if Database().globalConfig().isInMainMode() {
                rust.switchToDecoyMode()
                unlockAuthenticated()

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
    private func apply(_ settings: AuthSettings) {
        type = settings.authType
        isWipeDataPinEnabled = settings.isWipeDataPinEnabled
        isDecoyPinEnabled = settings.isDecoyPinEnabled
        isUsingBiometrics = false
    }

    @MainActor
    private func finishWipe(call: WipeCall) async -> UnlockMode {
        let rustApp = AppManager.shared.rust
        let result = await Task.detached(priority: .userInitiated) {
            do {
                let completion = switch call {
                case .initial:
                    try rustApp.dangerousWipeAllData()
                case let .retry(attemptId):
                    try rustApp.retryDangerousWipeAllData(attemptId: attemptId)
                }

                return WipeCallResult.success(completion)
            } catch let error as AppError {
                return WipeCallResult.failure(error)
            } catch {
                return WipeCallResult.unexpectedFailure(error.localizedDescription)
            }
        }.value

        switch result {
        case let .success(completion):
            await AppManager.shared.applyWipeCompletion(completion)
            apply(completion.auth)
            unlockAuthenticated()
            wipePresentationState = .idle
            return .wipe

        case let .failure(.WalletLifecycle(.shutdownBlocked(attemptId, _, _))):
            wipePresentationState = .shutdownBlocked(attemptId)
            return .locked

        case let .failure(error):
            logger.error("Failed to wipe all data: \(error)")
            wipePresentationState = .failed
            return .locked

        case let .unexpectedFailure(message):
            logger.error("Failed to wipe all data: \(message)")
            wipePresentationState = .failed
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

            case let .wipeDataPinChanged(enabled):
                isWipeDataPinEnabled = enabled

            case let .decoyPinChanged(enabled):
                isDecoyPinEnabled = enabled
            }
        }
    }

    public func dispatch(action: AuthManagerAction) {
        logger.debug("dispatch: \(action)")
        rust.dispatch(action: action)
    }
}
