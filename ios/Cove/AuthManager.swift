import SwiftUI

@Observable class AuthManager: AuthManagerReconciler {
    private let logger = Log(id: "AuthManager")
    var rust: RustAuthManager
    var authType = Database().globalConfig().authType()
    var lockState: LockState = .locked

    @MainActor
    var isUsingBiometrics: Bool = false

    public init() {
        rust = RustAuthManager()
        rust.listenForUpdates(reconciler: self)
    }
    
    public func lock() {
        guard isAuthEnabled else { return }
        lockState = .locked
    }

    public var isAuthEnabled: Bool {
        authType != AuthType.none
    }

    public func checkPin(_ pin: String) -> Bool {
        if AuthPin().check(pin: pin) {
            return true
        }

        if self.checkWipeDataPin(pin) {
            // TODO: delete all data
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
                    self.authType = authType
                }
            }
        }
    }

    public func dispatch(action: AuthManagerAction) {
        logger.debug("dispatch: \(action)")
        rust.dispatch(action: action)
    }
}
