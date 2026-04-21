import MijickPopups
import Observation
import SwiftUI

private let walletModeChangeDelayMs = 250

@Observable final class AppManager: FfiReconcile {
    static let shared = makeShared()

    private let logger = Log(id: "AppManager")

    var rust: FfiApp
    var router: Router
    var database: Database
    var wallets: [WalletMetadata] = []
    var isSidebarVisible = false
    var asyncRuntimeReady = false

    var alertState: TaggedItem<AppAlertState>? = .none
    var sheetState: TaggedItem<AppSheetState>? = .none

    /// tracks if current screen is scrolled past header for adaptive nav styling
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

    /// changed when route is reset, to clear lifecycle view state
    var routeId = UUID()

    /// AppManager is the sole owner of the live wallet manager used by wallet-backed routes.
    private(set) var walletManager: WalletManager?

    private(set) var sendFlowManager: SendFlowManager?

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

    private static func makeShared() -> AppManager {
        requireBootstrapComplete()
        return AppManager()
    }

    private static func requireBootstrapComplete() {
        if ProcessInfo.processInfo.environment["XCODE_RUNNING_FOR_PREVIEWS"] == "1" { return }

        let step = bootstrapProgress()
        guard step == .complete else {
            fatalError("AppManager initialized before bootstrap completed: \(step)")
        }
    }

    private init() {
        logger.debug("Initializing AppManager")

        let rust = FfiApp()
        let state = rust.state()

        router = state.router
        self.rust = rust
        database = Database()
        wallets = (try? database.wallets().all()) ?? []

        // set the cached prices and fees
        prices = try? rust.prices()
        fees = try? rust.fees()

        self.rust.listenForUpdates(updater: self)
    }

    func cachedWalletManager(id: WalletId) -> WalletManager? {
        guard let walletManager, walletManager.id == id else { return nil }
        return walletManager
    }

    func ensureWalletManager(id: WalletId) throws -> WalletManager {
        if let walletManager = cachedWalletManager(id: id) {
            logger.debug("found and using vm for \(id)")
            return walletManager
        }

        logger.debug(
            "did not find vm for \(id), creating new vm: \(walletManager?.id ?? "none")"
        )
        clearWalletManager()

        let walletManager = try WalletManager(id: id)
        self.walletManager = walletManager
        return walletManager
    }

    func cachedSendFlowManager(id: WalletId) -> SendFlowManager? {
        guard let sendFlowManager, sendFlowManager.id == id else { return nil }
        return sendFlowManager
    }

    func ensureSendFlowManager(
        _ walletManager: WalletManager,
        presenter: SendFlowPresenter
    ) -> SendFlowManager {
        if let sendFlowManager = cachedSendFlowManager(id: walletManager.id) {
            logger.debug("found and using sendflow manager for \(walletManager.id)")
            sendFlowManager.presenter = presenter
            return sendFlowManager
        }

        logger.debug("did not find SendFlowManager for \(walletManager.id), creating new")
        clearSendFlowManager()

        let sendFlowManager = SendFlowManager(
            walletManager.rust.newSendFlowManager(balance: walletManager.balance),
            presenter: presenter
        )
        self.sendFlowManager = sendFlowManager
        return sendFlowManager
    }

    public var fullVersionId: String {
        let appVersion = Bundle.main.infoDictionary?["CFBundleShortVersionString"] as? String ?? ""
        let buildNumber = Bundle.main.infoDictionary?["CFBundleVersion"] as? String ?? ""
        return "v\(appVersion) (\(rust.gitShortHash())-\(buildNumber))"
    }

    func clearWalletManager(id: WalletId? = nil) {
        if id == nil {
            walletManager = nil
            clearSendFlowManager()
            return
        }

        if walletManager?.id == id { walletManager = nil }
        clearSendFlowManager(id: id)
        return
    }

    func clearSendFlowManager(id: WalletId? = nil) {
        guard id == nil || sendFlowManager?.id == id else { return }
        sendFlowManager = nil
    }

    public func findTapSignerWallet(_ ts: TapSigner) -> WalletMetadata? {
        rust.findTapSignerWallet(tapSigner: ts)
    }

    public func getTapSignerBackup(_ ts: TapSigner) throws -> Data? {
        try rust.getTapSignerBackup(tapSigner: ts)
    }

    public func saveTapSignerBackup(_ ts: TapSigner, _ backup: Data) -> Bool {
        rust.saveTapSignerBackup(tapSigner: ts, backup: backup)
    }

    /// Reset the manager state
    public func reset() {
        rust = FfiApp()
        database = Database()
        clearWalletManager()

        let state = rust.state()
        router = state.router
    }

    /// Reload wallets from database (e.g. after cloud restore)
    func reloadWallets() {
        wallets = (try? database.wallets().all()) ?? []
        clearWalletManager()
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

    /// this will select the wallet and reset the route to the selectedWalletRoute
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

    func loadWallets() {
        wallets = (try? database.wallets().all()) ?? []
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
        DispatchQueue.main.async { [weak self] in
            guard let self else { return }
            logger.debug("Update: \(message)")

            switch message {
            case .routeUpdated(let routes):
                router.routes = routes

            case .pushedRoute(let route):
                router.routes.append(route)

            case .databaseUpdated:
                database = Database()

            case .colorSchemeChanged(let colorSchemeSelection):
                self.colorSchemeSelection = colorSchemeSelection

            case .selectedNodeChanged(let node):
                selectedNode = node

            case .selectedNetworkChanged(let network):
                selectedNetwork = network
                loadWallets()

            case .defaultRouteChanged(let route, let nestedRoutes):
                router.routes = nestedRoutes
                router.default = route
                routeId = UUID()

            case .fiatPricesChanged(let prices):
                self.prices = prices

            case .feesChanged(let fees):
                self.fees = fees

            case .fiatCurrencyChanged(let fiatCurrency):
                selectedFiatCurrency = fiatCurrency

                // refresh fiat values in the wallet manager
                if let walletManager {
                    Task {
                        await walletManager.forceWalletScan()
                        await walletManager.updateWalletBalance()
                    }
                }

            case .acceptedTerms:
                isTermsAccepted = true

            case .walletModeChanged:
                isLoading = true
                loadWallets()

                Task {
                    try? await Task.sleep(for: .milliseconds(walletModeChangeDelayMs))
                    await MainActor.run {
                        withAnimation { self.isLoading = false }
                    }
                }

            case .walletsChanged:
                wallets = (try? database.wallets().all()) ?? []

            case .clearCachedWalletManager(let walletId):
                clearWalletManager(id: walletId)

            case .showLoadingPopup:
                Task { await MiddlePopup(state: .loading).present() }

            case .hideLoadingPopup:
                Task { await PopupStack.dismissAllPopups() }
            }
        }
    }

    public func dispatch(action: AppAction) {
        logger.debug("dispatch \(action)")
        rust.dispatch(action: action)
    }
}
