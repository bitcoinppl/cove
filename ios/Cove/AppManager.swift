import MijickPopups
import Observation
import SwiftUI
import UIKit

private let walletModeChangeDelayMs = 250
private let sidebarNavigationDelayMs = 250
private let navigationSettleDelayMs = 800

@Observable final class AppManager: FfiReconcile {
    static let shared = makeShared()

    private let logger = Log(id: "AppManager")
    @ObservationIgnored
    private let navigationGenerations = GenerationTracker()
    @ObservationIgnored
    private var pendingSidebarNavigationTask: Task<Void, Never>?
    @ObservationIgnored
    private var navigationSettleTask: Task<Void, Never>?

    var rust: FfiApp
    var router: Router
    var database: Database
    var wallets: [WalletMetadata] = []
    var isSidebarVisible = false
    var isNavigationSettled = true
    var asyncRuntimeReady = false

    var alertState: TaggedItem<AppAlertState>? = .none
    var sheetState: TaggedItem<AppSheetState>? = .none

    /// tracks if current screen is scrolled past header for adaptive nav styling
    var isPastHeader = false

    var needsOnboarding = true
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
    /// Background time is tied to the cached wallet manager, not a route instance
    @ObservationIgnored
    private var initialScanBackgroundTask: UIBackgroundTaskIdentifier = .invalid
    @ObservationIgnored
    private weak var initialScanBackgroundTaskWalletManager: WalletManager?
    @ObservationIgnored
    private var initialScanBackgroundTaskAllowed = false

    private(set) var sendFlowManager: SendFlowManager?

    @ObservationIgnored
    weak var coinControlManager: CoinControlManager?

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
        needsOnboarding = rust.needsOnboarding()

        // set the cached prices and fees
        prices = try? rust.prices()
        fees = try? rust.fees()

        self.rust.listenForUpdates(updater: self)
    }

    func showInitialScanIncompleteAlert() {
        alertState = .init(.general(
            title: "Initial Scan Incomplete",
            message: "Can't send until initial scan completes."
        ))
    }

    func cachedWalletManager(id: WalletId) -> WalletManager? {
        guard let walletManager, walletManager.id == id else { return nil }
        return walletManager
    }

    func walletMetadata(id: WalletId) -> WalletMetadata? {
        if let walletManager = cachedWalletManager(id: id) {
            return walletManager.walletMetadata
        }

        return wallets.first(where: { $0.id == id })
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
        observeInitialScanLifecycle(for: walletManager)
        self.walletManager = walletManager
        return walletManager
    }

    private func observeInitialScanLifecycle(for walletManager: WalletManager) {
        walletManager.setInitialScanLifecycleChanged { [weak self, weak walletManager] in
            DispatchQueue.main.async { [weak self, weak walletManager] in
                guard let self, let walletManager else { return }
                guard self.walletManager === walletManager else { return }

                self.updateInitialScanBackgroundTask()
            }
        }
    }

    func beginInitialScanBackgroundTaskIfNeeded() {
        initialScanBackgroundTaskAllowed = true
        updateInitialScanBackgroundTask()
    }

    func endInitialScanBackgroundTask() {
        initialScanBackgroundTaskAllowed = false
        endInitialScanBackgroundTaskHandle()
    }

    private func updateInitialScanBackgroundTask() {
        guard let walletManager, walletManager.activeIncompleteInitialScan else {
            endInitialScanBackgroundTaskHandle()
            return
        }

        guard initialScanBackgroundTaskAllowed else {
            endInitialScanBackgroundTaskHandle()
            return
        }

        guard initialScanBackgroundTask == .invalid else {
            endInitialScanBackgroundTaskIfInactive()
            return
        }

        let backgroundTask = UIApplication.shared.beginBackgroundTask(
            withName: "Initial wallet scan"
        ) { [weak self] in
            DispatchQueue.main.async { [weak self] in
                Log.warn("Initial wallet scan background task expired")
                self?.endInitialScanBackgroundTask()
            }
        }

        guard backgroundTask != .invalid else {
            Log.warn("Unable to start initial wallet scan background task")
            return
        }

        initialScanBackgroundTask = backgroundTask
        initialScanBackgroundTaskWalletManager = walletManager
        logger.debug("Started initial wallet scan background task for wallet \(walletManager.id)")

        endInitialScanBackgroundTaskIfInactive()
    }

    private func endInitialScanBackgroundTaskHandle() {
        guard initialScanBackgroundTask != .invalid else { return }

        let backgroundTask = initialScanBackgroundTask
        initialScanBackgroundTask = .invalid
        initialScanBackgroundTaskWalletManager = nil

        UIApplication.shared.endBackgroundTask(backgroundTask)
        logger.debug("Ended initial wallet scan background task")
    }

    private func endInitialScanBackgroundTaskIfInactive() {
        guard initialScanBackgroundTask != .invalid else { return }
        guard let walletManager,
              let initialScanBackgroundTaskWalletManager,
              walletManager === initialScanBackgroundTaskWalletManager,
              walletManager.activeIncompleteInitialScan
        else {
            endInitialScanBackgroundTaskHandle()
            return
        }
    }

    func cachedSendFlowManager(id: WalletId) -> SendFlowManager? {
        guard let sendFlowManager, sendFlowManager.id == id else { return nil }
        return sendFlowManager
    }

    func ensureSendFlowManager(
        _ walletManager: WalletManager,
        presenter: SendFlowPresenter
    ) throws -> SendFlowManager {
        if let sendFlowManager = cachedSendFlowManager(id: walletManager.id) {
            logger.debug("found and using sendflow manager for \(walletManager.id)")
            sendFlowManager.presenter = presenter
            return sendFlowManager
        }

        logger.debug("did not find SendFlowManager for \(walletManager.id), creating new")
        clearSendFlowManager()

        let sendFlowManager = try SendFlowManager(
            walletManager.rust.newSendFlowManager(balance: walletManager.balance),
            presenter: presenter
        )
        self.sendFlowManager = sendFlowManager
        return sendFlowManager
    }

    public func setCoinControlManager(_ manager: CoinControlManager) {
        coinControlManager = manager
    }

    public func clearCoinControlManager(_ manager: CoinControlManager) {
        if coinControlManager === manager {
            coinControlManager = nil
        }
    }

    private func clearCoinControlManager() {
        guard let coinControlManager else { return }

        self.coinControlManager = nil
        coinControlManager.close()
    }

    private func reconcileCoinControlManagerOwnership() {
        guard coinControlManager != nil else { return }
        guard !router.containsCoinControlRoute else { return }

        clearCoinControlManager()
    }

    @MainActor
    public func reconcileAfterLabelsChanged(walletId: WalletId) {
        if let walletManager, walletManager.id == walletId {
            walletManager.reconcileAfterLabelsChanged()
        }

        if let coinControlManager, coinControlManager.id == walletId {
            Task { await coinControlManager.reloadLabels() }
        }

        if let sendFlowManager, sendFlowManager.id == walletId {
            sendFlowManager.reconcileAfterLabelsChanged()
        }
    }

    public var fullVersionId: String {
        let appVersion = Bundle.main.infoDictionary?["CFBundleShortVersionString"] as? String ?? ""
        let buildNumber = Bundle.main.infoDictionary?["CFBundleVersion"] as? String ?? ""
        return "v\(appVersion) (\(rust.gitShortHash())-\(buildNumber))"
    }

    func clearWalletManager(id: WalletId? = nil) {
        if id == nil {
            endInitialScanBackgroundTask()
            walletManager?.setInitialScanLifecycleChanged(nil)
            walletManager?.close()
            walletManager = nil
            clearSendFlowManager()
            return
        }

        if walletManager?.id == id {
            endInitialScanBackgroundTask()
            walletManager?.setInitialScanLifecycleChanged(nil)
            walletManager?.close()
            walletManager = nil
        }

        clearSendFlowManager(id: id)
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
        pendingSidebarNavigationTask?.cancel()
        pendingSidebarNavigationTask = nil
        navigationSettleTask?.cancel()
        navigationSettleTask = nil
        advanceNavigationGeneration()

        database = Database()
        needsOnboarding = rust.needsOnboarding()
        clearWalletManager()
        coinControlManager?.close()
        coinControlManager = nil

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

    private func isDuplicateTopRoute(_ route: Route) -> Bool {
        currentRoute.isSameNavigationDestination(routeToCheck: route)
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
        guard !isDuplicateTopRoute(route) else {
            isSidebarVisible = false
            return
        }

        advanceNavigationGeneration()
        pushRouteWithoutNavigationGeneration(route)
    }

    private func pushRouteWithoutNavigationGeneration(_ route: Route) {
        isSidebarVisible = false
        guard !isDuplicateTopRoute(route) else { return }

        router.routes.append(route)
        reconcileCoinControlManagerOwnership()
    }

    func pushRoutes(_ routes: [Route]) {
        advanceNavigationGeneration()
        pushRoutesWithoutNavigationGeneration(routes)
    }

    private func pushRoutesWithoutNavigationGeneration(_ routes: [Route]) {
        isSidebarVisible = false
        router.routes.append(contentsOf: routes)
        reconcileCoinControlManagerOwnership()
    }

    func popRoute() {
        advanceNavigationGeneration()

        if !router.routes.isEmpty {
            router.routes.removeLast()
            reconcileCoinControlManagerOwnership()
        }
    }

    func setRoute(_ routes: [Route]) {
        advanceNavigationGeneration()
        router.routes = routes
        reconcileCoinControlManagerOwnership()
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
    private func advanceNavigationGeneration() -> GenerationToken {
        let generation = navigationGenerations.advance()
        scheduleNavigationSettled(for: generation)
        return generation
    }

    private func scheduleNavigationSettledForCurrentGeneration() {
        scheduleNavigationSettled(for: navigationGenerations.capture())
    }

    private func scheduleNavigationSettled(for generation: GenerationToken) {
        navigationSettleTask?.cancel()
        isNavigationSettled = false

        navigationSettleTask = Task { @MainActor [weak self] in
            do {
                try await Task.sleep(for: .milliseconds(navigationSettleDelayMs))
            } catch {
                return
            }

            guard let self else { return }
            guard self.isNavigationGenerationCurrent(generation) else { return }

            self.isNavigationSettled = true
            self.navigationSettleTask = nil
        }
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
    func captureLoadAndResetGeneration() -> GenerationToken {
        navigationGenerations.capture()
    }

    @MainActor
    func startLoadAndResetTargetPrewarm(generation: GenerationToken, routes: [Route]) {
        Task { [weak self] in
            await self?.prewarmLoadAndResetTargetIfCurrent(generation: generation, routes: routes)
        }
    }

    @MainActor
    func prewarmLoadAndResetTargetIfCurrent(generation: GenerationToken, routes: [Route]) async {
        guard isNavigationGenerationCurrent(generation) else { return }
        guard case let .selectedWallet(id) = routes.first else { return }

        do {
            let manager = try ensureWalletManager(id: id)
            try await manager.startWalletScanIfNeeded()
        } catch {
            logger.error("Unable to prewarm selected wallet \(id): \(error)")
        }
    }

    @MainActor
    func resetAfterLoadingIfCurrent(generation: GenerationToken, route: Route, nextRoute: [Route]) {
        guard isNavigationGenerationCurrent(generation) else { return }
        guard router.default == route else { return }
        rust.resetAfterLoading(to: nextRoute)
    }

    private func isNavigationGenerationCurrent(_ generation: GenerationToken) -> Bool {
        navigationGenerations.isCurrent(capturedToken: generation)
    }

    func reconcile(message: AppStateReconcileMessage) {
        DispatchQueue.main.async { [weak self] in
            guard let self else { return }
            logger.debug("Update: \(message)")

            switch message {
            case let .routeUpdated(routes: routes):
                let didChangeRoute = router.routes != routes
                router.routes = routes
                reconcileCoinControlManagerOwnership()

                if didChangeRoute {
                    scheduleNavigationSettledForCurrentGeneration()
                }

            case let .pushedRoute(route):
                guard !isDuplicateTopRoute(route) else {
                    isSidebarVisible = false
                    return
                }

                router.routes.append(route)
                reconcileCoinControlManagerOwnership()
                scheduleNavigationSettledForCurrentGeneration()

            case .databaseUpdated:
                database = Database()
                needsOnboarding = rust.needsOnboarding()

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
                reconcileCoinControlManagerOwnership()
                scheduleNavigationSettledForCurrentGeneration()

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
        do {
            try rust.dispatch(action: action)
        } catch {
            logger.error("Unable to dispatch app action \(action), error: \(error)")
        }
    }
}

private extension Router {
    var containsCoinControlRoute: Bool {
        self.default.isCoinControlRoute || routes.contains { $0.isCoinControlRoute }
    }
}

private extension Route {
    var isCoinControlRoute: Bool {
        if case .coinControl = self { return true }
        return false
    }
}
