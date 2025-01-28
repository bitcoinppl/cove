import SwiftUI

enum UnlockMode {
    case main, decoy, wipe, locked
}

@Observable class AuthManager: AuthManagerReconciler {
    private let logger = Log(id: "AuthManager")
    var rust: RustAuthManager
    var type = Database().globalConfig().authType()
    var lockState: LockState = .locked
    var isWipeDataPinEnabled: Bool
    var isDecoyPinEnabled: Bool

    @MainActor
    var isUsingBiometrics: Bool = false

    public init() {
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
        checkUnlockMode(pin) != .locked
    }

    @MainActor
    public func checkUnlockMode(_ pin: String) -> UnlockMode {
        if AuthPin().check(pin: pin) {
            if Database().globalConfig().isInDecoyMode() {
                Database().switchToMainMode()
                let db = Database()

                let app = AppManaged.shared
                app.reset(db: db)

                if let selectedWalletId = db.globalConfig().selectedWallet() {
                    try? app.rust.selectWallet(id: selectedWalletId)
                } else {
                    app.loadAndReset(to: RouteFactory().newWalletSelect())
                }
            }

            return .main
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
            AppManager().reset()

            return .wipe
        }

        // check if the entered pin a the decoy pin, if so enter decoy mode
        if checkDecoyPin(pin) {
            // enter decoy mode if not already in decoy mode and reset app and router
            if Database().globalConfig().isInMainMode() {
                Database().switchToDecoyMode()
                let db = Database()

                let app = AppManager.shared
                app.reset(db: db)

                if let selectedWalletId = db.globalConfig().selectedWallet() {
                    try? app.rust.selectWallet(id: selectedWalletId)
                } else {
                    app.loadAndReset(to: RouteFactory().newWalletSelect())
                }
            }

            return .decoy
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
