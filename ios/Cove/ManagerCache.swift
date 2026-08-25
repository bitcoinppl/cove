import Foundation
import Observation

enum WalletManagerCacheLoadDecision: Equatable {
    case installLoaded
    case useCached
    case cancelLoaded

    static func resolve(
        token: WalletManagerCacheLoadToken,
        currentState: WalletManagerCacheState,
        cachedWalletId: WalletId?,
        hasCurrentWaiter: Bool = true
    ) -> Self {
        guard hasCurrentWaiter else {
            return .cancelLoaded
        }

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

typealias SendFlowManagerFactory = (
    _ walletManager: WalletManager,
    _ presenter: SendFlowPresenter
) throws -> SendFlowManager

typealias WalletManagerLoadFactory = @MainActor (
    _ id: WalletId,
    _ delegate: WalletManagerDelegate
) async throws -> WalletManager

private final class WalletManagerLoadWaiter: @unchecked Sendable {
    private let lock = NSLock()
    private var isCancelled = false
    private let isCurrent: @MainActor () -> Bool

    init(isCurrent: @escaping @MainActor () -> Bool) {
        self.isCurrent = isCurrent
    }

    func cancel() {
        lock.withLock {
            isCancelled = true
        }
    }

    @MainActor
    func isCurrentWaiter() -> Bool {
        let isCancelled = lock.withLock { self.isCancelled }
        return !isCancelled && isCurrent()
    }
}

@MainActor
private final class WalletManagerLoadWaiterGroup {
    private var waiters: [UUID: WalletManagerLoadWaiter] = [:]

    func register(isCurrent: @escaping @MainActor () -> Bool) -> Registration {
        let id = UUID()
        let waiter = WalletManagerLoadWaiter(isCurrent: isCurrent)
        waiters[id] = waiter
        return Registration(group: self, id: id, waiter: waiter)
    }

    func hasCurrentWaiter() -> Bool {
        waiters.values.contains { $0.isCurrentWaiter() }
    }

    func unregister(id: UUID) {
        waiters.removeValue(forKey: id)
    }

    struct Registration {
        let group: WalletManagerLoadWaiterGroup
        let id: UUID
        let waiter: WalletManagerLoadWaiter
    }
}

private struct WalletManagerLoad {
    let id: UUID
    let waiters: WalletManagerLoadWaiterGroup
    let task: Task<WalletManager, Error>
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
    private let makeSendFlowManager: SendFlowManagerFactory
    @ObservationIgnored
    private let loadWalletManager: WalletManagerLoadFactory
    @ObservationIgnored
    private var walletManagerLoads: [WalletId: WalletManagerLoad] = [:]
    @ObservationIgnored
    private var walletManagerCacheState = WalletManagerCacheState()

    private(set) var walletManager: WalletManager?
    private(set) var sendFlowManager: SendFlowManager?
    private(set) var keyTeleportManager: KeyTeleportManager?
    @ObservationIgnored
    weak var coinControlManager: CoinControlManager?

    init(
        backgroundScanTaskHandler: BackgroundScanTaskHandler,
        makeSendFlowManager: @escaping SendFlowManagerFactory = { walletManager, presenter in
            try SendFlowManager(
                walletManager.newSendFlowManager(),
                presenter: presenter
            )
        },
        loadWalletManager: @escaping WalletManagerLoadFactory = { id, delegate in
            try await WalletManager.load(id: id, delegate: delegate)
        }
    ) {
        self.backgroundScanTaskHandler = backgroundScanTaskHandler
        self.makeSendFlowManager = makeSendFlowManager
        self.loadWalletManager = loadWalletManager
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

    @MainActor
    func ensureWalletManagerLoaded(
        id: WalletId,
        delegate: WalletManagerDelegate,
        isCurrent: @escaping @MainActor () -> Bool = { true }
    ) async throws -> WalletManager {
        guard isCurrent() else { throw CancellationError() }

        if let walletManager = cachedWalletManager(id: id) {
            logger.debug("found and using vm for \(id)")
            return walletManager
        }

        let load = walletManagerLoads[id] ?? startWalletManagerLoad(id: id, delegate: delegate)
        let waiter = load.waiters.register(isCurrent: isCurrent)
        defer {
            waiter.group.unregister(id: waiter.id)
        }

        logger.debug(
            "did not find vm for \(id), loading new vm: \(walletManager?.id ?? "none")"
        )

        let loadedWalletManager = try await withTaskCancellationHandler(
            operation: {
                try await load.task.value
            },
            onCancel: {
                waiter.waiter.cancel()
            }
        )
        try Task.checkCancellation()

        guard waiter.waiter.isCurrentWaiter() else {
            throw CancellationError()
        }

        guard let walletManager = cachedWalletManager(id: id),
              walletManager === loadedWalletManager
        else {
            throw CancellationError()
        }

        return walletManager
    }

    @MainActor
    private func startWalletManagerLoad(
        id: WalletId,
        delegate: WalletManagerDelegate
    ) -> WalletManagerLoad {
        let loadToken = walletManagerCacheState.loadToken(for: id)
        let loadId = UUID()
        let waiters = WalletManagerLoadWaiterGroup()
        let loadTask = Task { @MainActor [weak self] in
            guard let self else { throw CancellationError() }

            defer {
                if walletManagerLoads[id]?.id == loadId {
                    walletManagerLoads[id] = nil
                }
            }

            let loadedWalletManager = try await loadWalletManager(id, delegate)
            switch WalletManagerCacheLoadDecision.resolve(
                token: loadToken,
                currentState: walletManagerCacheState,
                cachedWalletId: walletManager?.id,
                hasCurrentWaiter: waiters.hasCurrentWaiter()
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

        let load = WalletManagerLoad(id: loadId, waiters: waiters, task: loadTask)
        walletManagerLoads[id] = load
        return load
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
            clearSendFlowManager()
            backgroundScanTaskHandler.endInitialScanBackgroundTask()
            walletManager?.setInitialScanLifecycleChanged(nil)
            let walletManager = self.walletManager
            self.walletManager = nil
            walletManager?.close()

            if hadWalletManager {
                walletManagerCacheState.managerChanged()
            }
            return
        }

        guard let id else { return }
        walletManagerCacheState.invalidate(.wallet(id))
        clearSendFlowManager(id: id)

        if walletManager?.id == id {
            backgroundScanTaskHandler.endInitialScanBackgroundTask()
            walletManager?.setInitialScanLifecycleChanged(nil)
            let walletManager = self.walletManager
            self.walletManager = nil
            walletManagerCacheState.managerChanged()
            walletManager?.close()
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

        let sendFlowManager = try makeSendFlowManager(walletManager, presenter)
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

    func ensureKeyTeleportManager(app: FfiApp) -> KeyTeleportManager {
        if let keyTeleportManager {
            return keyTeleportManager
        }

        let keyTeleportManager = KeyTeleportManager(app.newKeyTeleportManager())
        self.keyTeleportManager = keyTeleportManager
        return keyTeleportManager
    }

    func clearKeyTeleportManager() {
        guard let keyTeleportManager else { return }

        self.keyTeleportManager = nil
        keyTeleportManager.close()
    }

    func reconcileCoinControlManagerOwnership(router: Router) {
        guard coinControlManager != nil else { return }
        guard !router.containsCoinControlRoute else { return }

        clearCoinControlManager()
    }

    func reconcileKeyTeleportManagerOwnership(router: Router) {
        guard keyTeleportManager != nil else { return }
        guard !router.containsKeyTeleportRoute else { return }

        clearKeyTeleportManager()
    }

    func reconcileRouteOwnedManagers(router: Router) {
        reconcileCoinControlManagerOwnership(router: router)
        reconcileKeyTeleportManagerOwnership(router: router)
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

        let sendFlowManager = self.sendFlowManager
        self.sendFlowManager = nil
        sendFlowManager?.close()
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

    var containsKeyTeleportRoute: Bool {
        self.default.isKeyTeleportRoute || routes.contains { $0.isKeyTeleportRoute }
    }
}

private extension Route {
    var isCoinControlRoute: Bool {
        if case .coinControl = self {
            return true
        }
        return false
    }

    var isKeyTeleportRoute: Bool {
        if case .keyTeleport = self { return true }
        return false
    }
}
