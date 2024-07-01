import SwiftUI

@Observable class MainViewModel: FfiUpdater {
    var rust: FfiApp
    var router: Router
    var database: Database
    var isSidebarVisible = false
    var defaultRoute: Route

    public init() {
        let rust = FfiApp()
        let state = rust.getState()

        router = state.router
        self.rust = rust
        database = Database()

        defaultRoute = state.defaultRoute

        self.rust.listenForUpdates(updater: self)
    }

    func toggleSidebar() {
        isSidebarVisible.toggle()
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
                case let .routeUpdate(routes: routes):
                    self.router.routes = routes
                case .databaseUpdate:
                    self.database = Database()
                case let .defaultRouteChanged(route):
                    self.defaultRoute = route
                }
            }
        }
    }

    public func dispatch(event: Event) {
        rust.dispatch(event: event)
    }
}
