import SwiftUI

@Observable class MainViewModel: FfiUpdater {
    var rust: FfiApp
    var router: Router
    var database: Database

    public init() {
        let rust = FfiApp()
        let state = rust.getState()

        self.router = state.router
        self.rust = rust
        self.database = Database()

        self.rust.listenForUpdates(updater: self)
    }

    func pushRoute(_ route: Route) {
        self.router.routes.append(route)
    }

    func popRoute() {
        self.router.routes.removeLast()
    }

    func setRoute(_ routes: [Route]) {
        self.router.routes = routes
    }

    func update(update: Update) {
        Task {
            await MainActor.run {
                print("[swift] Update: \(update)")

                switch update {
                case .routerUpdate(router: let router):
                    self.router = router
                case .databaseUpdate:
                    self.database = Database()
                }
            }
        }
    }

    public func dispatch(event: Event) {
        self.rust.dispatch(event: event)
    }
}
