import SwiftUI

enum UnlockMode {
    case main, decoy, wipe, locked
}

@Observable class AuthManager: AuthManagerReconciler {
    static let shared = AuthManager()

    private let logger = Log(id: "AuthManager")
    var rust: RustAuthManager
    var type = Database().globalConfig().authType()
    var lockState: LockState = .locked
    var isWipeDataPinEnabled: Bool
    var isDecoyPinEnabled: Bool

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
        lockState = .locked
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
            if Database().globalConfig().isInDecoyMode() {
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

            return .main
        }

        // check if the entered pin a the decoy pin, if so enter decoy mode
        if checkDecoyPin(pin) {
            // enter decoy mode if not already in decoy mode and reset app and router
            if Database().globalConfig().isInMainMode() {
                rust.switchToDecoyMode()
                lockState = .unlocked

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
            lockState = .unlocked
            type = .none

            // reset app manager
            AppManager.shared.reset()

            return .wipe
        }

        return .locked
    }

    public func checkWipeDataPin(_ pin: String) -> Bool {
        rust.checkWipeDataPin(pin: pin)
    }

    public func checkDecoyPin(_ pin: String) -> Bool {
        rust.checkDecoyPin(pin: pin)
    }

    func reconcile(message: AuthManagerReconcileMessage) {
        logger.debug("reconcile: \(message)")

        Task {
            await MainActor.run {
                switch message {
                case let .authTypeChanged(authType):
                    self.type = authType

                case .wipeDataPinChanged:
                    self.isWipeDataPinEnabled = self.rust.isWipeDataPinEnabled()

                case .decoyPinChanged:
                    self.isDecoyPinEnabled = self.rust.isDecoyPinEnabled()
                }
            }
        }
    }

    public func dispatch(action: AuthManagerAction) {
        logger.debug("dispatch: \(action)")
        rust.dispatch(action: action)
    }
}
