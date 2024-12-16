import SwiftUI

@Observable class AuthManager: AuthManagerReconciler {
    private let logger = Log(id: "AuthManager")
    var rust: RustAuthManager
    var authType = Database().globalConfig().authType()

    @MainActor
    var isUsingBiometrics: Bool = false

    public init() {
        rust = RustAuthManager()
        rust.listenForUpdates(reconciler: self)
    }

    public var isAuthEnabled: Bool {
        authType != AuthType.none
    }

    public func checkPin(_ pin: String) -> Bool {
        AuthPin().check(pin: pin)
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
