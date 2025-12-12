import SwiftUI

enum UnlockMode {
    case main, decoy, wipe, locked
}

@Observable final class AuthManager: AuthManagerReconciler {
    static let shared = AuthManager()

    private let logger = Log(id: "AuthManager")
    var rust: RustAuthManager
    var type = Database().globalConfig().authType()
    var lockState: LockState = .locked
    var isWipeDataPinEnabled: Bool
    var isDecoyPinEnabled: Bool

    @ObservationIgnored
    var lockedAt: Date? {
        guard let lockedAt = rust.lockedAt() else { return nil }
        return Date(timeIntervalSince1970: Double(lockedAt))
    }

    @MainActor
    var isUsingBiometrics: Bool = false

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
    public func handleAndReturnUnlockMode(_ pin: String) -> UnlockMode {
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
                    try? app.rust.selectWallet(id: selectedWalletId)
                } else {
                    app.loadAndReset(to: RouteFactory().newWalletSelect())
                }
            }

            return .decoy
        }

        // check if the entered pin is a wipeDataPin
        // if so wipe the data
        if checkWipeDataPin(pin) {
            AppManager.shared.rust.dangerousWipeAllData()

            // reset auth maanger
            rust = RustAuthManager()
            unlock()

            type = .none

            // reset app manager
            AppManager.shared.reset()

            return .wipe
        }

        return .locked
    }

    @MainActor
    public func switchToMainMode() {
        rust.switchToMainMode()

        let app = AppManager.shared
        app.reset()
        app.isLoading = true

        let db = Database()
        if let selectedWalletId = db.globalConfig().selectedWallet() {
            try? app.rust.selectWallet(id: selectedWalletId)
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
