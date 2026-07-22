import Observation

enum WalletManagerCacheLoadDecision: Equatable {
    case installLoaded
    case useCached
    case cancelLoaded

    static func resolve(
        token: WalletManagerCacheLoadToken,
        currentState: WalletManagerCacheState,
        cachedWalletId: WalletId?
    ) -> Self {
        if cachedWalletId == token.targetId {
            return .useCached
        }

        if currentState.invalidated(token) {
            return .cancelLoaded
        }

        if currentState.managerGeneration != token.managerGeneration,
           cachedWalletId != nil
        {
            return .cancelLoaded
        }

        return .installLoaded
    }
}

enum WalletManagerCacheInvalidationScope: Equatable {
    case all
    case wallet(WalletId)
}

struct WalletManagerCacheLoadToken: Equatable {
    let targetId: WalletId
    let managerGeneration: UInt64
    fileprivate let allInvalidationGeneration: UInt64
    fileprivate let targetInvalidationGeneration: UInt64
}

struct WalletManagerCacheState: Equatable {
    private(set) var managerGeneration: UInt64 = 0
    private var allInvalidationGeneration: UInt64 = 0
    private var walletInvalidationGenerations: [WalletId: UInt64] = [:]

    func loadToken(for targetId: WalletId) -> WalletManagerCacheLoadToken {
        WalletManagerCacheLoadToken(
            targetId: targetId,
            managerGeneration: managerGeneration,
            allInvalidationGeneration: allInvalidationGeneration,
            targetInvalidationGeneration: walletInvalidationGenerations[targetId, default: 0]
        )
    }

    func invalidated(_ token: WalletManagerCacheLoadToken) -> Bool {
        allInvalidationGeneration != token.allInvalidationGeneration
            || walletInvalidationGenerations[token.targetId, default: 0]
            != token.targetInvalidationGeneration
    }

    mutating func managerChanged() {
        managerGeneration &+= 1
    }

    mutating func invalidate(_ scope: WalletManagerCacheInvalidationScope) {
        switch scope {
        case .all:
            allInvalidationGeneration &+= 1
            walletInvalidationGenerations.removeAll(keepingCapacity: true)
        case let .wallet(id):
            walletInvalidationGenerations[id, default: 0] &+= 1
        }
    }
}

@Observable final class ManagerCache {
    private let logger = Log(id: "ManagerCache")

    @ObservationIgnored
    private let backgroundScanTaskHandler: BackgroundScanTaskHandler
    @ObservationIgnored
    private var walletManagerCacheState = WalletManagerCacheState()

    private(set) var walletManager: WalletManager?
    private(set) var sendFlowManager: SendFlowManager?
    @ObservationIgnored
    weak var coinControlManager: CoinControlManager?

    init(backgroundScanTaskHandler: BackgroundScanTaskHandler) {
        self.backgroundScanTaskHandler = backgroundScanTaskHandler
    }

    func cachedWalletManager(id: WalletId) -> WalletManager? {
        guard let walletManager, walletManager.id == id else { return nil }
        return walletManager
    }

    func walletMetadata(id: WalletId, wallets: [WalletMetadata]) -> WalletMetadata? {
        if let walletManager = cachedWalletManager(id: id) {
            return walletManager.walletMetadata
        }

        return wallets.first(where: { $0.id == id })
    }

    func ensureWalletManager(
        id: WalletId,
        delegate: WalletManagerDelegate
    ) throws -> WalletManager {
        if let walletManager = cachedWalletManager(id: id) {
            logger.debug("found and using vm for \(id)")
            return walletManager
        }

        logger.debug(
            "did not find vm for \(id), creating new vm: \(walletManager?.id ?? "none")"
        )

        let walletManager = try WalletManager(id: id, delegate: delegate)
        return installWalletManager(walletManager)
    }

    @MainActor
    func ensureWalletManagerLoaded(
        id: WalletId,
        delegate: WalletManagerDelegate,
        isCurrent: @MainActor () -> Bool = { true }
    ) async throws -> WalletManager {
        guard isCurrent() else { throw CancellationError() }

        if let walletManager = cachedWalletManager(id: id) {
            logger.debug("found and using vm for \(id)")
            return walletManager
        }

        let loadToken = walletManagerCacheState.loadToken(for: id)
        logger.debug(
            "did not find vm for \(id), loading new vm: \(walletManager?.id ?? "none")"
        )

        let loadedWalletManager = try await WalletManager.load(id: id, delegate: delegate)
        do {
            try Task.checkCancellation()
        } catch {
            loadedWalletManager.close()
            throw error
        }

        guard isCurrent() else {
            loadedWalletManager.close()
            throw CancellationError()
        }

        switch WalletManagerCacheLoadDecision.resolve(
            token: loadToken,
            currentState: walletManagerCacheState,
            cachedWalletId: walletManager?.id
        ) {
        case .installLoaded:
            return installWalletManager(loadedWalletManager)
        case .useCached:
            loadedWalletManager.close()
            guard let walletManager = cachedWalletManager(id: id) else {
                throw CancellationError()
            }

            return walletManager
        case .cancelLoaded:
            loadedWalletManager.close()
            throw CancellationError()
        }
    }

    private func installWalletManager(_ walletManager: WalletManager) -> WalletManager {
        if let existing = self.walletManager {
            if existing === walletManager {
                return walletManager
            }
            if existing.id == walletManager.id {
                walletManager.close()
                return existing
            }
        }

        let previousManager = self.walletManager
        backgroundScanTaskHandler.endInitialScanBackgroundTask()
        previousManager?.setInitialScanLifecycleChanged(nil)
        clearSendFlowManager()

        backgroundScanTaskHandler.observeInitialScanLifecycle(for: walletManager) { [weak self] in
            self?.walletManager
        }
        self.walletManager = walletManager
        walletManagerCacheState.managerChanged()
        previousManager?.close()

        return walletManager
    }

    func clearWalletManager(id: WalletId? = nil) {
        if id == nil {
            walletManagerCacheState.invalidate(.all)

            let hadWalletManager = walletManager != nil
            backgroundScanTaskHandler.endInitialScanBackgroundTask()
            walletManager?.setInitialScanLifecycleChanged(nil)
            walletManager?.close()
            walletManager = nil
            clearSendFlowManager()

            if hadWalletManager {
                walletManagerCacheState.managerChanged()
            }
            return
        }

        guard let id else { return }
        walletManagerCacheState.invalidate(.wallet(id))

        if walletManager?.id == id {
            backgroundScanTaskHandler.endInitialScanBackgroundTask()
            walletManager?.setInitialScanLifecycleChanged(nil)
            walletManager?.close()
            walletManager = nil
            walletManagerCacheState.managerChanged()
        }

        clearSendFlowManager(id: id)
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

    func clearCoinControlManager() {
        guard let coinControlManager else { return }

        self.coinControlManager = nil
        coinControlManager.close()
    }

    func reconcileCoinControlManagerOwnership(router: Router) {
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

    func clearSendFlowManager(id: WalletId? = nil) {
        guard id == nil || sendFlowManager?.id == id else { return }
        sendFlowManager = nil
    }

    func beginInitialScanBackgroundTaskIfNeeded() {
        backgroundScanTaskHandler.beginInitialScanBackgroundTaskIfNeeded(walletManager: walletManager)
    }

    func endInitialScanBackgroundTask() {
        backgroundScanTaskHandler.endInitialScanBackgroundTask()
    }
}

extension Router {
    var containsCoinControlRoute: Bool {
        self.default.isCoinControlRoute || routes.contains { $0.isCoinControlRoute }
    }
}

private extension Route {
    var isCoinControlRoute: Bool {
        if case .coinControl = self {
            return true
        }
        return false
    }
}
