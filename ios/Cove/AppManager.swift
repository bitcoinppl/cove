import MijickPopups
import Observation
import SwiftUI

private let walletModeChangeDelayMs = 250
private let sidebarNavigationDelayMs = 250

@Observable final class AppManager: FfiReconcile {
    static let shared = makeShared()

    private let logger = Log(id: "AppManager")
    @ObservationIgnored
    private var navigationGeneration: UInt64 = 0
    @ObservationIgnored
    private var pendingSidebarNavigationTask: Task<Void, Never>?

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

    /// Multiple screens within the same wallet (send, coin control, tx details, settings)
    /// call getWalletManager, this avoids recreating the actor and reconciler each time
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

        let sendFlowManager = SendFlowManager(wm.rust.newSendFlowManager(balance: wm.balance), presenter: presenter)
        logger.debug("did not find SendFlowManager for \(id), creating new")

        self.sendFlowManager = sendFlowManager
        return sendFlowManager
    }

    public var fullVersionId: String {
        let appVersion = Bundle.main.infoDictionary?["CFBundleShortVersionString"] as? String ?? ""
        let buildNumber = Bundle.main.infoDictionary?["CFBundleVersion"] as? String ?? ""
        return "v\(appVersion) (\(rust.gitShortHash())-\(buildNumber))"
    }

    public func updateWalletVm(_ vm: WalletManager) {
        walletManager = vm
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
        pendingSidebarNavigationTask?.cancel()
        pendingSidebarNavigationTask = nil
        advanceNavigationGeneration()

        database = Database()
        walletManager = nil

        let state = rust.state()
        router = state.router
    }

    /// Reload wallets from database (e.g. after cloud restore)
    func reloadWallets() {
        wallets = (try? database.wallets().all()) ?? []
        walletManager = nil
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
            try selectWalletOrThrow(id)
        } catch {
            Log.error("Unable to select wallet \(id), error: \(error)")
        }
    }

    func selectWalletOrThrow(_ id: WalletId) throws {
        advanceNavigationGeneration()
        try selectWalletWithoutNavigationGeneration(id)
    }

    private func selectWalletWithoutNavigationGeneration(_ id: WalletId) throws {
        try rust.dispatch(action: .selectWallet(id: id))
        isSidebarVisible = false
    }

    func trySelectLatestOrNewWallet() {
        do {
            try selectLatestOrNewWallet()
        } catch {
            Log.error("Unable to select latest wallet, error: \(error)")
        }
    }

    func selectLatestOrNewWallet() throws {
        advanceNavigationGeneration()
        try rust.dispatch(action: .selectLatestOrNewWallet)
        isSidebarVisible = false
    }

    func toggleSidebar() {
        isSidebarVisible.toggle()
    }

    func loadWallets() {
        wallets = (try? database.wallets().all()) ?? []
    }

    func pushRoute(_ route: Route) {
        advanceNavigationGeneration()
        pushRouteWithoutNavigationGeneration(route)
    }

    private func pushRouteWithoutNavigationGeneration(_ route: Route) {
        isSidebarVisible = false
        router.routes.append(route)
    }

    func pushRoutes(_ routes: [Route]) {
        advanceNavigationGeneration()
        pushRoutesWithoutNavigationGeneration(routes)
    }

    private func pushRoutesWithoutNavigationGeneration(_ routes: [Route]) {
        isSidebarVisible = false
        router.routes.append(contentsOf: routes)
    }

    func popRoute() {
        advanceNavigationGeneration()

        if !router.routes.isEmpty {
            router.routes.removeLast()
        }
    }

    func setRoute(_ routes: [Route]) {
        advanceNavigationGeneration()
        router.routes = routes
    }

    func scanQr() {
        advanceNavigationGeneration()
        sheetState = TaggedItem(.qr)
    }

    func scanNfc() {
        advanceNavigationGeneration()
        scanNfcWithoutNavigationGeneration()
    }

    private func scanNfcWithoutNavigationGeneration() {
        nfcReader.scan()
    }

    @MainActor
    func resetRoute(to routes: [Route]) {
        advanceNavigationGeneration()
        resetRouteWithoutNavigationGeneration(to: routes)
    }

    @MainActor
    private func resetRouteWithoutNavigationGeneration(to routes: [Route]) {
        if routes.count > 1 {
            rust.resetNestedRoutesTo(defaultRoute: routes[0], nestedRoutes: Array(routes[1...]))
        } else if let route = routes.first {
            rust.resetDefaultRouteTo(route: route)
        }
    }

    func resetRoute(to route: Route) {
        advanceNavigationGeneration()
        resetRouteWithoutNavigationGeneration(to: route)
    }

    private func resetRouteWithoutNavigationGeneration(to route: Route) {
        rust.resetDefaultRouteTo(route: route)
    }

    @MainActor
    func loadAndReset(to route: Route) {
        advanceNavigationGeneration()
        rust.loadAndResetDefaultRoute(route: route)
    }

    @discardableResult
    private func advanceNavigationGeneration() -> UInt64 {
        navigationGeneration &+= 1

        return navigationGeneration
    }

    func closeSidebarAndSelectWallet(_ id: WalletId) {
        closeSidebarThenNavigate {
            do {
                try self.selectWalletWithoutNavigationGeneration(id)
            } catch {
                Log.error("Unable to select wallet \(id), error: \(error)")
            }
        }
    }

    func closeSidebarAndOpenNewWallet() {
        closeSidebarThenNavigate {
            if self.hasWallets {
                self.pushRouteWithoutNavigationGeneration(RouteFactory().newWalletSelect())
            } else {
                self.resetRouteWithoutNavigationGeneration(to: [RouteFactory().newWalletSelect()])
            }
        }
    }

    func closeSidebarAndOpenSettings() {
        closeSidebarThenNavigate {
            self.pushRouteWithoutNavigationGeneration(.settings(.main))
        }
    }

    func closeSidebarAndOpenWalletSettings(_ id: WalletId) {
        closeSidebarThenNavigate {
            self.pushRoutesWithoutNavigationGeneration(RouteFactory().nestedWalletSettings(id: id))
        }
    }

    func closeSidebarAndScanNfc() {
        closeSidebarThenNavigate {
            self.scanNfcWithoutNavigationGeneration()
        }
    }

    private func closeSidebarThenNavigate(_ action: @escaping @MainActor () -> Void) {
        pendingSidebarNavigationTask?.cancel()
        let generation = advanceNavigationGeneration()

        withAnimation(.spring(response: 0.3, dampingFraction: 0.8)) {
            isSidebarVisible = false
        }

        pendingSidebarNavigationTask = Task { @MainActor in
            do {
                try await Task.sleep(for: .milliseconds(sidebarNavigationDelayMs))
            } catch {
                return
            }

            guard isNavigationGenerationCurrent(generation) else { return }
            action()
        }
    }

    @MainActor
    func captureLoadAndResetGeneration() -> UInt64 {
        navigationGeneration
    }

    @MainActor
    func resetAfterLoadingIfCurrent(generation: UInt64, route: Route, nextRoute: [Route]) {
        guard isNavigationGenerationCurrent(generation) else { return }
        guard router.default == route else { return }
        rust.resetAfterLoading(to: nextRoute)
    }

    private func isNavigationGenerationCurrent(_ generation: UInt64) -> Bool {
        generation == navigationGeneration
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
            case let .routeUpdated(routes: routes):
                router.routes = routes

            case let .pushedRoute(route):
                router.routes.append(route)

            case .databaseUpdated:
                database = Database()

            case let .colorSchemeChanged(colorSchemeSelection):
                self.colorSchemeSelection = colorSchemeSelection

            case let .selectedNodeChanged(node):
                selectedNode = node

            case let .selectedNetworkChanged(network):
                selectedNetwork = network
                loadWallets()

            case let .defaultRouteChanged(route, nestedRoutes):
                router.routes = nestedRoutes
                router.default = route
                routeId = UUID()

            case let .fiatPricesChanged(prices):
                self.prices = prices

            case let .feesChanged(fees):
                self.fees = fees

            case let .fiatCurrencyChanged(fiatCurrency):
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

            case let .clearCachedWalletManager(walletId):
                if walletManager?.id == walletId { walletManager = nil }

            case .showLoadingPopup:
                Task { await MiddlePopup(state: .loading).present() }

            case .hideLoadingPopup:
                Task { await PopupStack.dismissAllPopups() }
            }
        }
    }

    public func dispatch(action: AppAction) {
        logger.debug("dispatch \(action)")
        do {
            try rust.dispatch(action: action)
        } catch {
            logger.error("Unable to dispatch app action \(action), error: \(error)")
        }
    }
}
