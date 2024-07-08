import SwiftUI

@Observable class MainViewModel: FfiUpdater {
    private let logger = Log(id: "MainViewModel")

    var rust: FfiApp
    var router: Router
    var database: Database
    var isSidebarVisible = false

    public var selectedNetwork: Network {
        rust.network()
    }

    public let menuItems: [MenuItem] =
        [
            MenuItem(destination: RouteFactory().newWalletSelect(), title: "New Wallet", icon: "wallet.pass.fill"),
            MenuItem(destination: Route.listWallets, title: "Change Wallet", icon: "arrow.uturn.right.square.fill"),
        ]

    public init() {
        logger.debug("Initializing MainViewModel")

        let rust = FfiApp()
        let state = rust.state()

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

    func resetRoute(to route: Route) {
        router.routes = []
        rust.resetDefaultRouteTo(route: route)
    }

    func reconcile(message _: AppStateReconcileMessage) {
        Task {
            await MainActor.run {
                logger.debug("Update: \(update)")
                print("update \(update)")

                switch update {
                case let .routeUpdate(routes: routes):
                    self.router.routes = routes

                case .databaseUpdate:
                    self.database = Database()

                case let .defaultRouteChanged(route):
                    // default changes, means root changes, set routes to []
                    self.router.routes = []
                    self.router.default = route
                }
            }
        }
    }

    public func dispatch(action: AppAction) {
        logger.debug("dispatch \(action)")
        rust.dispatch(action: action)
    }
}
