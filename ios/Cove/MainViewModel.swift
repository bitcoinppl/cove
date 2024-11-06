import Observation
import SwiftUI

@Observable class MainViewModel: FfiReconcile {
    private let logger = Log(id: "MainViewModel")

    var rust: FfiApp
    var router: Router
    var database: Database
    var isSidebarVisible = false
    var asyncRuntimeReady = false

    var colorSchemeSelection = Database().globalConfig().colorScheme()
    var selectedNode = Database().globalConfig().selectedNode()

    var sheetState: TaggedItem<AppSheetState>? = .none
    var nfcReader = NFCReader()

    var prices: PriceResponse?

    // changed when route is reset, to clear lifecycle view state
    var routeId = UUID()

    @ObservationIgnored
    weak var walletViewModel: WalletViewModel?

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

    public func getWalletViewModel(id: WalletId) throws -> WalletViewModel {
        if let walletvm = walletViewModel, walletvm.id == id {
            logger.debug("found and using vm for \(id)")
            return walletvm
        }

        logger.debug("did not find vm for \(id), creating new vm: \(walletViewModel?.id ?? "none")")

        let walletvm = try WalletViewModel(id: id)
        walletViewModel = walletvm

        return walletViewModel!
    }

    public func updateWalletVm(_ vm: WalletViewModel) {
        walletViewModel = vm
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

                case let .selectedNodeChanged(node):
                    self.selectedNode = node

                case let .defaultRouteChanged(route):
                    // default changes, means root changes, set routes to []
                    self.router.routes = []
                    self.router.default = route
                    self.routeId = UUID()

                case let .fiatPricesChanged(prices):
                    self.prices = prices
                }
            }
        }
    }

    public func dispatch(action: AppAction) {
        logger.debug("dispatch \(action)")
        rust.dispatch(action: action)
    }
}
