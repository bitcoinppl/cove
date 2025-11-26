import Observation
import SwiftUI

@Observable final class AppManager: FfiReconcile {
    static let shared = AppManager()

    private let logger = Log(id: "AppManager")

    var rust: FfiApp
    var router: Router
    var database: Database
    var isSidebarVisible = false
    var asyncRuntimeReady = false

    var alertState: TaggedItem<AppAlertState>? = .none
    var sheetState: TaggedItem<AppSheetState>? = .none

    // tracks if current screen is scrolled past header for adaptive nav styling
    var isPastHeader = false

    var isTermsAccepted: Bool = Database().globalFlag().isTermsAccepted()
    var selectedNetwork = Database().globalConfig().selectedNetwork()

    var colorSchemeSelection = Database().globalConfig().colorScheme()
    var selectedNode = Database().globalConfig().selectedNode()
    var selectedFiatCurrency = Database().globalConfig().selectedFiatCurrency()

    var nfcReader = NFCReader()
    var nfcWriter = NFCWriter()
    var tapSignerNfc: TapSignerNFC?

    var prices: PriceResponse?
    var fees: FeeResponse?

    @MainActor
    var isLoading = false

    // changed when route is reset, to clear lifecycle view state
    var routeId = UUID()

    @ObservationIgnored
    var walletManager: WalletManager?

    @ObservationIgnored
    var sendFlowManager: SendFlowManager?

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

    private init() {
        logger.debug("Initializing AppManager")

        let rust = FfiApp()
        let state = rust.state()

        router = state.router
        self.rust = rust
        database = Database()

        // set the cached prices and fees
        prices = try? rust.prices()
        fees = try? rust.fees()

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

    public func getSendFlowManager(_ wm: WalletManager, presenter: SendFlowPresenter) -> SendFlowManager {
        let id = wm.id

        if let manager = sendFlowManager, wm.id == manager.id {
            logger.debug("found and using sendflow manager for \(wm.id)")
            manager.presenter = presenter
            return manager
        }

        let sendFlowManager = SendFlowManager(wm.rust.newSendFlowManager(), presenter: presenter)
        logger.debug("did not find SendFlowManager for \(id), creating new")

        self.sendFlowManager = sendFlowManager
        return sendFlowManager
    }

    public var fullVersionId: String {
        let appVersion = Bundle.main.infoDictionary?["CFBundleShortVersionString"] as? String ?? ""
        return "v\(appVersion) (\(rust.gitShortHash()))"
    }

    public func updateWalletVm(_ vm: WalletManager) {
        walletManager = vm
    }

    public func findTapSignerWallet(_ ts: TapSigner) -> WalletMetadata? {
        rust.findTapSignerWallet(tapSigner: ts)
    }

    public func getTapSignerBackup(_ ts: TapSigner) -> Data? {
        rust.getTapSignerBackup(tapSigner: ts)
    }

    public func saveTapSignerBackup(_ ts: TapSigner, _ backup: Data) -> Bool {
        rust.saveTapSignerBackup(tapSigner: ts, backup: backup)
    }

    /// Reset the manager state
    public func reset() {
        rust = FfiApp()
        database = Database()
        walletManager = nil

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

    // this will select the wallet and reset the route to the selectedWalletRoute
    func selectWallet(_ id: WalletId) {
        do {
            try rust.selectWallet(id: id)
            isSidebarVisible = false
        } catch {
            Log.error("Unabel to select wallet \(id), error: \(error)")
        }
    }

    func toggleSidebar() {
        isSidebarVisible.toggle()
    }

    func pushRoute(_ route: Route) {
        isSidebarVisible = false
        router.routes.append(route)
    }

    func pushRoutes(_ routes: [Route]) {
        isSidebarVisible = false
        router.routes.append(contentsOf: routes)
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

    func agreeToTerms() {
        self.dispatch(action: .acceptTerms)
        withAnimation { isTermsAccepted = true }
    }

    func reconcile(message: AppStateReconcileMessage) {
        Task {
            await MainActor.run {
                logger.debug("Update: \(message)")

                switch message {
                case let .routeUpdated(routes: routes):
                    self.router.routes = routes

                case let .pushedRoute(route):
                    self.router.routes.append(route)

                case .databaseUpdated:
                    self.database = Database()

                case let .colorSchemeChanged(colorSchemeSelection):
                    self.colorSchemeSelection = colorSchemeSelection

                case let .selectedNodeChanged(node):
                    self.selectedNode = node

                case let .selectedNetworkChanged(network):
                    self.selectedNetwork = network

                case let .defaultRouteChanged(route, nestedRoutes):
                    self.router.routes = nestedRoutes
                    self.router.default = route
                    self.routeId = UUID()

                case let .fiatPricesChanged(prices):
                    self.prices = prices

                case let .feesChanged(fees):
                    self.fees = fees

                case let .fiatCurrencyChanged(fiatCurrency):
                    self.selectedFiatCurrency = fiatCurrency

                    // refresh fiat values in the wallet manager
                    if let walletManager {
                        Task {
                            await walletManager.forceWalletScan()
                            await walletManager.updateWalletBalance()
                        }
                    }

                case .acceptedTerms:
                    self.isTermsAccepted = true

                case .walletModeChanged:
                    self.isLoading = true

                    Task {
                        try? await Task.sleep(for: .milliseconds(200))
                        await MainActor.run {
                            withAnimation { self.isLoading = false }
                        }
                    }
                }
            }
        }
    }

    public func dispatch(action: AppAction) {
        logger.debug("dispatch \(action)")
        rust.dispatch(action: action)
    }
}
