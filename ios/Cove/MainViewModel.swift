import SwiftUI

@Observable class MainViewModel: FfiUpdater {
    var rust: FfiApp
    var router: Router
    var database: Database
    var isSidebarVisible = false

    public let menuItems: [MenuItem] =
        [
            MenuItem(destination: RouteFactory().newWalletSelect(), title: "New Wallet", icon: "wallet.pass.fill"),
            MenuItem(destination: Route.listWallets, title: "Change Wallet", icon: "arrow.uturn.right.square.fill"),
        ]

    public init() {
        let rust = FfiApp()
        let state = rust.getState()

        router = state.router
        self.rust = rust
        database = Database()

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
                    // default changes, means root changes, set routes to []
                    self.router.default = route
                    self.router.routes = []
                }
            }
        }
    }

    public func dispatch(event: Event) {
        rust.dispatch(event: event)
    }
}
