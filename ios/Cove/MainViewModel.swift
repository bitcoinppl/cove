import SwiftUI

@Observable class MainViewModel: FfiUpdater {
    var rust: FfiApp
    var router: Router
    var database: Database

    public init() {
        let rust = FfiApp()
        let state = rust.getState()

        router = state.router
        self.rust = rust
        database = Database()

        self.rust.listenForUpdates(updater: self)
    }

    func pushRoute(_ route: Route) {
        router.routes.append(route)
    }

    func popRoute() {
        router.routes.removeLast()
    }

    func setRoute(_ routes: [Route]) {
        router.routes = routes
    }

    func update(update: Update) {
        Task {
            await MainActor.run {
                print("[SWIFT] Update: \(update)")

                switch update {
                case let .routerUpdate(router: router):
                    self.router = router
                case .databaseUpdate:
                    self.database = Database()
                }
            }
        }
    }

    public func dispatch(event: Event) {
        rust.dispatch(event: event)
    }
}
