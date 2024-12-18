import SwiftUI

@Observable class AuthManager: AuthManagerReconciler {
    private let logger = Log(id: "AuthManager")
    var rust: RustAuthManager
    var type = Database().globalConfig().authType()
    var lockState: LockState = .locked
    var isWipeDataPinEnabled: Bool

    @MainActor
    var isUsingBiometrics: Bool = false

    public init() {
        let rust = RustAuthManager()
        self.rust = rust
        isWipeDataPinEnabled = rust.isWipeDataPinEnabled()

        rust.listenForUpdates(reconciler: self)
    }

    public func lock() {
        guard isAuthEnabled else { return }
        lockState = .locked
    }

    public var isAuthEnabled: Bool {
        type != AuthType.none
    }

    public func checkPin(_ pin: String) -> Bool {
        if AuthPin().check(pin: pin) {
            return true
        }

        if checkWipeDataPin(pin) {
            AppManager().rust.dangerousWipeAllData()

            // reset auth maanger
            rust = RustAuthManager()
            lockState = .unlocked
            type = .none

            // reset app manager
            AppManager().reset()

            return true
        }

        return false
    }

    public func checkWipeDataPin(_ pin: String) -> Bool {
        rust.checkWipeDataPin(pin: pin)
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
                }
            }
        }
    }

    public func dispatch(action: AuthManagerAction) {
        logger.debug("dispatch: \(action)")
        rust.dispatch(action: action)
    }
}
