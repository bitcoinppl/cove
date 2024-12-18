import Observation
import SwiftUI

@Observable class AppManager: FfiReconcile {
    private let logger = Log(id: "AppManager")

    var rust: FfiApp
    var router: Router
    var database: Database
    var isSidebarVisible = false
    var asyncRuntimeReady = false

    var alertState: TaggedItem<AppAlertState>? = .none
    var sheetState: TaggedItem<AppSheetState>? = .none

    var colorSchemeSelection = Database().globalConfig().colorScheme()
    var selectedNode = Database().globalConfig().selectedNode()

    var nfcReader = NFCReader()

    var prices: PriceResponse?
    var fees: FeeResponse?

    // changed when route is reset, to clear lifecycle view state
    var routeId = UUID()

    @ObservationIgnored
    weak var walletManager: WalletManager?

    public var selectedNetwork: Network {
        rust.network()
    }

    public var colorScheme: ColorScheme? {
        switch colorSchemeSelection {
        case .light:
            .light
        case .dark:
            .dark
        case .system:
            nil
        }
    }

    public init() {
        logger.debug("Initializing AppManager")

        let rust = FfiApp()
        let state = rust.state()

        router = state.router
        self.rust = rust
        database = Database()

        self.rust.listenForUpdates(updater: self)
    }

    public func getWalletManager(id: WalletId) throws -> WalletManager {
        if let walletvm = walletManager, walletvm.id == id {
            logger.debug("found and using vm for \(id)")
            return walletvm
        }

        logger.debug("did not find vm for \(id), creating new vm: \(walletManager?.id ?? "none")")

        let walletvm = try WalletManager(id: id)
        walletManager = walletvm

        return walletManager!
    }

    public func updateWalletVm(_ vm: WalletManager) {
        walletManager = vm
    }

    /// Reset the manager state
    public func reset() {
        rust = FfiApp()
        database = Database()

        let state = rust.state()
        router = state.router
    }

    var currentRoute: Route {
        router.routes.last ?? router.default
    }

    var hasWallets: Bool {
        rust.hasWallets()
    }

    var numberOfWallets: Int {
        Int(rust.numWallets())
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

    func scanQr() {
        sheetState = TaggedItem(.qr)
    }

    @MainActor
    func resetRoute(to routes: [Route]) {
        guard routes.count > 1 else { return resetRoute(to: routes[0]) }
        rust.resetNestedRoutesTo(defaultRoute: routes[0], nestedRoutes: Array(routes[1...]))
    }

    func resetRoute(to route: Route) {
        rust.resetDefaultRouteTo(route: route)
    }

    @MainActor
    func loadAndReset(to route: Route) {
        rust.loadAndResetDefaultRoute(route: route)
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

                case let .selectedNodeChanged(node):
                    self.selectedNode = node

                case let .defaultRouteChanged(route, nestedRoutes):
                    self.router.routes = nestedRoutes
                    self.router.default = route
                    self.routeId = UUID()

                case let .fiatPricesChanged(prices):
                    self.prices = prices

                case let .feesChanged(fees):
                    self.fees = fees
                }
            }
        }
    }

    public func dispatch(action: AppAction) {
        logger.debug("dispatch \(action)")
        rust.dispatch(action: action)
    }
}
