import SwiftUI

@Observable class MainViewModel: FfiReconcile {
    private let logger = Log(id: "MainViewModel")

    var rust: FfiApp
    var router: Router
    var database: Database
    var isSidebarVisible = false
    var colorSchemeSelection = Database().globalConfig().colorScheme()

    public var selectedNetwork: Network {
        rust.network()
    }

    public var colorScheme: ColorScheme? {
        switch colorSchemeSelection {
        case .light:
            return .light
        case .dark:
            return .dark
        case .system:
            return nil
        }
    }

    public init() {
        logger.debug("Initializing MainViewModel")

        let rust = FfiApp()
        let state = rust.state()

        router = state.router
        self.rust = rust
        database = Database()

        self.rust.listenForUpdates(updater: self)
    }

    var currentRoute: Route {
        router.routes.last ?? router.default
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
        rust.resetDefaultRouteTo(route: route)
    }

    func reconcile(message: AppStateReconcileMessage) {
        Task {
            await MainActor.run {
                logger.debug("Update: \(message)")

                switch message {
                case let .routeUpdated(routes: routes):
                    self.router.routes = routes

                case .databaseUpdated:
                    self.database = Database()

                case let .colorSchemeChanged(colorSchemeSelection):
                    self.colorSchemeSelection = colorSchemeSelection

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
