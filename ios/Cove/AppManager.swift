import MijickPopups
import Observation
import SwiftUI

private let walletModeChangeDelayMs = 250

enum LoadAndResetPreparationOutcome {
    case ready
    case redirected
}

enum WalletTargetPreparationOutcome {
    case ready(WalletManager)
    case redirected
}

struct WalletTransitionRecoveryPlan {
    private(set) var attemptedIds: [WalletId] = []

    mutating func recordAttempt(_ id: WalletId) {
        guard !attemptedIds.contains(id) else { return }
        attemptedIds.append(id)
    }

    func candidates(cachedWalletId: WalletId?, displayedIds: [WalletId]) -> [WalletId] {
        let orderedIds = [cachedWalletId].compactMap(\.self) + displayedIds

        return orderedIds.reduce(into: []) { candidates, id in
            guard !attemptedIds.contains(id), !candidates.contains(id) else { return }
            candidates.append(id)
        }
    }
}

@Observable final class AppManager: FfiReconcile {
    static let shared = makeShared()

    private let logger = Log(id: "AppManager")

    var rust: FfiApp
    var router: Router
    var database: Database
    var navigationCoordinator: NavigationCoordinator
    var managerCache: ManagerCache
    var wallets: [WalletMetadata] = []
    var isSidebarVisible = false
    var isNavigationSettled: Bool {
        navigationCoordinator.isNavigationSettled
    }

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
    var walletManager: WalletManager? {
        managerCache.walletManager
    }

    var sendFlowManager: SendFlowManager? {
        managerCache.sendFlowManager
    }

    var coinControlManager: CoinControlManager? {
        managerCache.coinControlManager
    }

    var keyTeleportManager: KeyTeleportManager? {
        managerCache.keyTeleportManager
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

    private static func makeShared() -> AppManager {
        requireBootstrapComplete()
        return AppManager()
    }

    private static func requireBootstrapComplete() {
        if ProcessInfo.processInfo.environment["XCODE_RUNNING_FOR_PREVIEWS"] == "1" {
            return
        }

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
        navigationCoordinator = NavigationCoordinator(routeClient: rust)
        managerCache = ManagerCache(backgroundScanTaskHandler: BackgroundScanTaskHandler())
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
        managerCache.cachedWalletManager(id: id)
    }

    func walletMetadata(id: WalletId) -> WalletMetadata? {
        managerCache.walletMetadata(id: id, wallets: wallets)
    }

    func ensureWalletManager(id: WalletId) throws -> WalletManager {
        try managerCache.ensureWalletManager(id: id, delegate: self)
    }

    @MainActor
    func ensureWalletManagerLoaded(
        id: WalletId,
        isCurrent: @MainActor () -> Bool = { true }
    ) async throws -> WalletManager {
        try await managerCache.ensureWalletManagerLoaded(
            id: id,
            delegate: self,
            isCurrent: isCurrent
        )
    }

    func beginInitialScanBackgroundTaskIfNeeded() {
        managerCache.beginInitialScanBackgroundTaskIfNeeded()
    }

    func endInitialScanBackgroundTask() {
        managerCache.endInitialScanBackgroundTask()
    }

    func cachedSendFlowManager(id: WalletId) -> SendFlowManager? {
        managerCache.cachedSendFlowManager(id: id)
    }

    func ensureSendFlowManager(
        _ walletManager: WalletManager,
        presenter: SendFlowPresenter
    ) throws -> SendFlowManager {
        try managerCache.ensureSendFlowManager(walletManager, presenter: presenter)
    }

    public func setCoinControlManager(_ manager: CoinControlManager) {
        managerCache.setCoinControlManager(manager)
    }

    public func clearCoinControlManager(_ manager: CoinControlManager) {
        managerCache.clearCoinControlManager(manager)
    }

    func ensureKeyTeleportManager() -> KeyTeleportManager {
        managerCache.ensureKeyTeleportManager(app: rust)
    }

    func clearKeyTeleportManager() {
        managerCache.clearKeyTeleportManager()
    }

    func canKeyTeleportSend(walletId: WalletId) -> Bool {
        rust.canKeyTeleportSend(walletId: walletId)
    }

    @MainActor
    public func reconcileAfterLabelsChanged(walletId: WalletId) {
        managerCache.reconcileAfterLabelsChanged(walletId: walletId)
    }

    public var fullVersionId: String {
        let appVersion = Bundle.main.infoDictionary?["CFBundleShortVersionString"] as? String ?? ""
        let buildNumber = Bundle.main.infoDictionary?["CFBundleVersion"] as? String ?? ""
        return "v\(appVersion) (\(rust.gitShortHash())-\(buildNumber))"
    }

    func clearWalletManager(id: WalletId? = nil) {
        managerCache.clearWalletManager(id: id)
    }

    func clearSendFlowManager(id: WalletId? = nil) {
        managerCache.clearSendFlowManager(id: id)
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
        navigationCoordinator.reset()

        database = Database()
        needsOnboarding = rust.needsOnboarding()
        clearWalletManager()
        managerCache.clearCoinControlManager()

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
            try selectWalletOrThrow(id)
        } catch {
            Log.error("Unable to select wallet \(id), error: \(error)")
        }
    }

    func selectWalletOrThrow(_ id: WalletId) throws {
        try navigationCoordinator.selectWallet(
            id,
            router: router,
            isSidebarVisible: &isSidebarVisible
        )
    }

    private func selectWalletWithoutNavigationGeneration(_ id: WalletId) throws {
        try navigationCoordinator.selectWallet(
            id,
            router: router,
            isSidebarVisible: &isSidebarVisible,
            advancesGeneration: false
        )
    }

    func trySelectLatestOrNewWallet() {
        do {
            try selectLatestOrNewWallet()
        } catch {
            Log.error("Unable to select latest wallet, error: \(error)")
        }
    }

    func selectLatestOrNewWallet() throws {
        try navigationCoordinator.selectLatestOrNewWallet(isSidebarVisible: &isSidebarVisible)
    }

    func toggleSidebar() {
        isSidebarVisible.toggle()
    }

    func loadWallets() {
        wallets = (try? database.wallets().all()) ?? []
    }

    func moveWallets(from source: IndexSet, to destination: Int) {
        var reordered = wallets
        reordered.move(fromOffsets: source, toOffset: destination)

        reorderWallets(walletIds: reordered.map(\.id))
    }

    func reorderWallets(walletIds: [WalletId]) {
        let walletsById = Dictionary(uniqueKeysWithValues: wallets.map { ($0.id, $0) })
        let currentWalletIds = Set(wallets.map(\.id))
        let requestedWalletIds = Set(walletIds)
        let reordered = walletIds.compactMap { walletsById[$0] }

        if walletIds.count == wallets.count, requestedWalletIds == currentWalletIds {
            wallets = reordered
        }

        do {
            wallets = try database.wallets().reorderWallets(walletIds: walletIds)
        } catch {
            logger.error("Unable to reorder wallets: \(error)")
            loadWallets()
        }
    }

    func pushRoute(_ route: Route) {
        navigationCoordinator.pushRoute(
            route,
            router: &router,
            isSidebarVisible: &isSidebarVisible
        ) { router in
            self.managerCache.reconcileRouteOwnedManagers(router: router)
        }
    }

    func pushRoutes(_ routes: [Route]) {
        navigationCoordinator.pushRoutes(
            routes,
            router: &router,
            isSidebarVisible: &isSidebarVisible
        ) { router in
            self.managerCache.reconcileRouteOwnedManagers(router: router)
        }
    }

    func popRoute() {
        navigationCoordinator.popRoute(router: &router) { router in
            self.managerCache.reconcileRouteOwnedManagers(router: router)
        }
    }

    func setRoute(_ routes: [Route]) {
        navigationCoordinator.setRoute(routes, router: &router) { router in
            self.managerCache.reconcileRouteOwnedManagers(router: router)
        }
    }

    func scanQr() {
        navigationCoordinator.advanceNavigationGeneration()
        sheetState = TaggedItem(.qr)
    }

    func scanNfc() {
        navigationCoordinator.scanNfc {
            self.nfcReader.scan()
        }
    }

    @MainActor
    func resetRoute(to routes: [Route]) {
        navigationCoordinator.resetRoute(to: routes)
    }

    func resetRoute(to route: Route) {
        navigationCoordinator.resetRoute(to: route)
    }

    @MainActor
    func loadAndReset(to route: Route) {
        navigationCoordinator.loadAndReset(to: route)
    }

    func closeSidebarAndSelectWallet(_ id: WalletId) {
        closeSidebarThenNavigate {
            do {
                try self.navigationCoordinator.selectWallet(
                    id,
                    router: self.router,
                    isSidebarVisible: &self.isSidebarVisible,
                    advancesGeneration: false
                )
            } catch {
                Log.error("Unable to select wallet \(id), error: \(error)")
            }
        }
    }

    func closeSidebarAndOpenNewWallet() {
        closeSidebarThenNavigate {
            if self.hasWallets {
                self.navigationCoordinator.pushRoute(
                    RouteFactory().newWalletSelect(),
                    router: &self.router,
                    isSidebarVisible: &self.isSidebarVisible,
                    advancesGeneration: false
                ) { router in
                    self.managerCache.reconcileRouteOwnedManagers(router: router)
                }
            } else {
                self.navigationCoordinator.resetRoute(
                    to: [RouteFactory().newWalletSelect()],
                    advancesGeneration: false
                )
            }
        }
    }

    func closeSidebarAndOpenSettings() {
        closeSidebarThenNavigate {
            self.navigationCoordinator.pushRoute(
                .settings(.main),
                router: &self.router,
                isSidebarVisible: &self.isSidebarVisible,
                advancesGeneration: false
            ) { router in
                self.managerCache.reconcileRouteOwnedManagers(router: router)
            }
        }
    }

    func closeSidebarAndOpenWalletSettings(_ id: WalletId) {
        closeSidebarThenNavigate {
            self.navigationCoordinator.pushRoutes(
                RouteFactory().nestedWalletSettings(id: id),
                router: &self.router,
                isSidebarVisible: &self.isSidebarVisible,
                advancesGeneration: false
            ) { router in
                self.managerCache.reconcileRouteOwnedManagers(router: router)
            }
        }
    }

    func closeSidebarAndScanNfc() {
        closeSidebarThenNavigate {
            self.navigationCoordinator.scanNfc(advancesGeneration: false) {
                self.nfcReader.scan()
            }
        }
    }

    private func closeSidebarThenNavigate(_ action: @escaping @MainActor () -> Void) {
        navigationCoordinator.closeSidebarThenNavigate(
            isSidebarVisible: &isSidebarVisible,
            action: action
        )
    }

    @MainActor
    func captureLoadAndResetGeneration() -> GenerationToken {
        navigationCoordinator.captureLoadAndResetGeneration()
    }

    @MainActor
    func prepareLoadAndResetTarget(
        generation: GenerationToken,
        routes: [Route]
    ) async throws -> LoadAndResetPreparationOutcome {
        guard case let .selectedWallet(id) = routes.first else { return .ready }

        switch try await prepareWalletTarget(id: id, generation: generation) {
        case .ready:
            return .ready
        case .redirected:
            return .redirected
        }
    }

    @MainActor
    func prepareSelectedWallet(
        id: WalletId,
        generation: GenerationToken
    ) async throws -> WalletTargetPreparationOutcome {
        try await prepareWalletTarget(id: id, generation: generation)
    }

    @MainActor
    private func prepareWalletTarget(
        id: WalletId,
        generation: GenerationToken
    ) async throws -> WalletTargetPreparationOutcome {
        var recoveryPlan = WalletTransitionRecoveryPlan()
        var candidateId = id

        while true {
            recoveryPlan.recordAttempt(candidateId)

            do {
                let manager = try await ensureWalletManagerLoaded(id: candidateId) {
                    self.navigationCoordinator.isNavigationGenerationCurrent(generation)
                }

                guard navigationCoordinator.isNavigationGenerationCurrent(generation) else {
                    throw CancellationError()
                }

                if candidateId == id {
                    return .ready(manager)
                }

                do {
                    try selectWalletWithoutNavigationGeneration(candidateId)
                    return .redirected
                } catch {
                    logger.error("Unable to select recovery wallet \(candidateId): \(error)")
                }
            } catch is CancellationError {
                throw CancellationError()
            } catch {
                handleWalletPreparationError(error, walletId: candidateId)
                guard walletPreparationFailureAllowsFallback(error) else { throw error }
            }

            guard let nextId = recoveryPlan.candidates(
                cachedWalletId: walletManager?.id,
                displayedIds: wallets.map(\.id)
            ).first else {
                recoverFromExhaustedWalletTransition(generation: generation)
                return .redirected
            }

            candidateId = nextId
        }
    }

    @MainActor
    private func handleWalletPreparationError(_ error: Error, walletId: WalletId) {
        if case let WalletManagerError.DatabaseCorruption(corruptedId, errorMessage) = error {
            logger.error("Wallet database corrupted for \(corruptedId): \(errorMessage)")
            alertState = TaggedItem(
                .walletDatabaseCorrupted(walletId: corruptedId, error: errorMessage)
            )
            return
        }

        logger.error("Unable to prepare wallet \(walletId): \(error)")
    }

    private func walletPreparationFailureAllowsFallback(_ error: Error) -> Bool {
        switch error {
        case WalletManagerError.WalletDoesNotExist,
             WalletManagerError.DatabaseCorruption:
            true
        default:
            false
        }
    }

    @MainActor
    private func recoverFromExhaustedWalletTransition(generation: GenerationToken) {
        guard navigationCoordinator.isNavigationGenerationCurrent(generation) else { return }

        do {
            try database.globalConfig().clearSelectedWallet()
        } catch {
            logger.error("Unable to clear selected wallet after recovery exhausted: \(error)")
        }

        clearWalletManager()
        resetRoute(to: .newWallet(.select))
    }

    @MainActor
    func resetAfterLoadingIfCurrent(generation: GenerationToken, route: Route, nextRoute: [Route]) {
        navigationCoordinator.resetAfterLoadingIfCurrent(
            generation: generation,
            route: route,
            nextRoute: nextRoute,
            router: router
        )
    }

    func reconcile(message: AppStateReconcileMessage) {
        DispatchQueue.main.async { [weak self] in
            guard let self else { return }
            logger.debug("Update: \(message)")

            switch message {
            case let .routeUpdated(routes: routes):
                navigationCoordinator.applyRouteUpdated(
                    routes: routes,
                    router: &router
                ) { router in
                    self.managerCache.reconcileRouteOwnedManagers(router: router)
                }

            case let .pushedRoute(route):
                navigationCoordinator.applyPushedRoute(
                    route,
                    router: &router,
                    isSidebarVisible: &isSidebarVisible
                ) { router in
                    self.managerCache.reconcileRouteOwnedManagers(router: router)
                }

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
                navigationCoordinator.applyDefaultRouteChanged(
                    route: route,
                    nestedRoutes: nestedRoutes,
                    router: &router,
                    routeId: &routeId
                ) { router in
                    self.managerCache.reconcileRouteOwnedManagers(router: router)
                }

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

extension AppManager: WalletManagerDelegate {
    func showWalletAlert(_ alertState: AppAlertState) {
        self.alertState = .init(alertState)
    }
}
