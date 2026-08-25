@testable import Cove
import CoveCore
import XCTest

final class WalletTransitionTests: XCTestCase {
    @MainActor
    private func makeSendFlowManager() throws -> (WalletManager, SendFlowManager) {
        let walletManager = WalletManager(preview: .only)
        let presenter = SendFlowPresenter(
            routing: TestSendFlowRouting(),
            manager: walletManager
        )
        let sendFlowManager = SendFlowManager(
            TestSendFlowRustManager(walletId: walletManager.id),
            presenter: presenter
        )

        return (walletManager, sendFlowManager)
    }

    private func drainMainQueue() async {
        await withCheckedContinuation { continuation in
            DispatchQueue.main.async {
                continuation.resume()
            }
        }
    }

    func testRecoveryPrioritizesCachedWalletThenDisplayOrder() {
        var plan = WalletTransitionRecoveryPlan()
        plan.recordAttempt("wallet-b")

        XCTAssertEqual(
            plan.candidates(
                cachedWalletId: "wallet-a",
                displayedIds: ["wallet-c", "wallet-a", "wallet-b", "wallet-d"]
            ),
            ["wallet-a", "wallet-c", "wallet-d"]
        )
    }

    func testCacheRaceUsesMatchingWinnerEvenAfterInvalidation() {
        var state = WalletManagerCacheState()
        let token = state.loadToken(for: "wallet-b")
        state.invalidate(.wallet("wallet-b"))
        state.invalidate(.all)

        XCTAssertEqual(
            WalletManagerCacheLoadDecision.resolve(
                token: token,
                currentState: state,
                cachedWalletId: "wallet-b"
            ),
            .useCached
        )
    }

    func testCacheRaceRejectsSupersedingReplacement() {
        var state = WalletManagerCacheState()
        let token = state.loadToken(for: "wallet-b")
        state.managerChanged()

        XCTAssertEqual(
            WalletManagerCacheLoadDecision.resolve(
                token: token,
                currentState: state,
                cachedWalletId: "wallet-c"
            ),
            .cancelLoaded
        )
    }

    func testCacheRaceInstallsOverUnchangedDifferentWallet() {
        let state = WalletManagerCacheState()
        let token = state.loadToken(for: "wallet-b")

        XCTAssertEqual(
            WalletManagerCacheLoadDecision.resolve(
                token: token,
                currentState: state,
                cachedWalletId: "wallet-a"
            ),
            .installLoaded
        )
    }

    func testCacheRaceInstallsTargetAfterUnrelatedClear() {
        var state = WalletManagerCacheState()
        let token = state.loadToken(for: "wallet-b")
        state.invalidate(.wallet("wallet-a"))

        XCTAssertEqual(
            WalletManagerCacheLoadDecision.resolve(
                token: token,
                currentState: state,
                cachedWalletId: nil
            ),
            .installLoaded
        )
    }

    func testCacheRaceCancelsTargetAfterTargetedClearWithoutCachedManager() {
        var state = WalletManagerCacheState()
        let token = state.loadToken(for: "wallet-b")
        state.invalidate(.wallet("wallet-b"))

        XCTAssertEqual(
            WalletManagerCacheLoadDecision.resolve(
                token: token,
                currentState: state,
                cachedWalletId: nil
            ),
            .cancelLoaded
        )
    }

    func testCacheRaceCancelsTargetAfterClearAllWithoutCachedManager() {
        var state = WalletManagerCacheState()
        let token = state.loadToken(for: "wallet-b")
        state.invalidate(.all)

        XCTAssertEqual(
            WalletManagerCacheLoadDecision.resolve(
                token: token,
                currentState: state,
                cachedWalletId: nil
            ),
            .cancelLoaded
        )
    }

    func testCacheRaceRejectsPublicationWhenAllWaitersAreStale() {
        let state = WalletManagerCacheState()
        let token = state.loadToken(for: "wallet-b")

        XCTAssertEqual(
            WalletManagerCacheLoadDecision.resolve(
                token: token,
                currentState: state,
                cachedWalletId: nil,
                hasCurrentWaiter: false
            ),
            .cancelLoaded
        )
    }

    @MainActor
    func testConcurrentWalletLoadsShareThePublishedManager() async throws {
        let expectedManager = WalletManager(preview: .only)
        let loadStarted = expectation(description: "wallet load starts")
        var resumeLoad: CheckedContinuation<Void, Never>?
        var loadCount = 0
        let cache = ManagerCache(
            backgroundScanTaskHandler: BackgroundScanTaskHandler(),
            loadWalletManager: { _, _ in
                loadCount += 1
                loadStarted.fulfill()
                await withCheckedContinuation { (continuation: CheckedContinuation<Void, Never>) in
                    resumeLoad = continuation
                }

                return expectedManager
            }
        )
        let delegate = TestWalletManagerDelegate()

        let firstLoad = Task { @MainActor in
            try await cache.ensureWalletManagerLoaded(id: expectedManager.id, delegate: delegate)
        }
        await fulfillment(of: [loadStarted], timeout: 1)

        let secondLoad = Task { @MainActor in
            try await cache.ensureWalletManagerLoaded(id: expectedManager.id, delegate: delegate)
        }
        await Task.yield()
        XCTAssertEqual(loadCount, 1)

        resumeLoad?.resume()
        let firstManager = try await firstLoad.value
        let secondManager = try await secondLoad.value

        XCTAssertTrue(firstManager === expectedManager)
        XCTAssertTrue(secondManager === expectedManager)
        XCTAssertTrue(cache.cachedWalletManager(id: expectedManager.id) === expectedManager)

        cache.clearWalletManager()
    }

    @MainActor
    func testStaleWalletLoadClosesWithoutPublishing() async throws {
        let expectedManager = WalletManager(preview: .only)
        let loadStarted = expectation(description: "wallet load starts")
        var resumeLoad: CheckedContinuation<Void, Never>?
        var isCurrent = true
        let cache = ManagerCache(
            backgroundScanTaskHandler: BackgroundScanTaskHandler(),
            loadWalletManager: { _, _ in
                loadStarted.fulfill()
                await withCheckedContinuation { (continuation: CheckedContinuation<Void, Never>) in
                    resumeLoad = continuation
                }

                return expectedManager
            }
        )
        let delegate = TestWalletManagerDelegate()

        let load = Task { @MainActor in
            try await cache.ensureWalletManagerLoaded(
                id: expectedManager.id,
                delegate: delegate,
                isCurrent: { isCurrent }
            )
        }
        await fulfillment(of: [loadStarted], timeout: 1)

        isCurrent = false
        resumeLoad?.resume()

        do {
            _ = try await load.value
            XCTFail("stale wallet load must be cancelled")
        } catch is CancellationError {}

        XCTAssertNil(cache.cachedWalletManager(id: expectedManager.id))
        XCTAssertFalse(expectedManager.canApplyReconcileMessages)
    }

    @MainActor
    func testCurrentWalletLoadReceivesWinnerWhenAnotherWaiterIsStale() async throws {
        let expectedManager = WalletManager(preview: .only)
        let loadStarted = expectation(description: "wallet load starts")
        var resumeLoad: CheckedContinuation<Void, Never>?
        var staleIsCurrent = true
        var loadCount = 0
        let cache = ManagerCache(
            backgroundScanTaskHandler: BackgroundScanTaskHandler(),
            loadWalletManager: { _, _ in
                loadCount += 1
                loadStarted.fulfill()
                await withCheckedContinuation { (continuation: CheckedContinuation<Void, Never>) in
                    resumeLoad = continuation
                }

                return expectedManager
            }
        )
        let delegate = TestWalletManagerDelegate()

        let staleLoad = Task { @MainActor in
            try await cache.ensureWalletManagerLoaded(
                id: expectedManager.id,
                delegate: delegate,
                isCurrent: { staleIsCurrent }
            )
        }
        await fulfillment(of: [loadStarted], timeout: 1)

        let currentLoad = Task { @MainActor in
            try await cache.ensureWalletManagerLoaded(id: expectedManager.id, delegate: delegate)
        }
        await Task.yield()
        XCTAssertEqual(loadCount, 1)

        staleIsCurrent = false
        resumeLoad?.resume()

        do {
            _ = try await staleLoad.value
            XCTFail("stale wallet waiter must be cancelled")
        } catch is CancellationError {}

        let winner = try await currentLoad.value
        XCTAssertTrue(winner === expectedManager)
        XCTAssertTrue(cache.cachedWalletManager(id: expectedManager.id) === expectedManager)
        XCTAssertTrue(expectedManager.canApplyReconcileMessages)

        cache.clearWalletManager()
    }

    func testRepeatedInvalidationAdvancesWhenCacheIsEmpty() {
        var state = WalletManagerCacheState()
        state.invalidate(.wallet("wallet-b"))
        let targetedToken = state.loadToken(for: "wallet-b")
        state.invalidate(.wallet("wallet-b"))

        XCTAssertTrue(state.invalidated(targetedToken))

        state.invalidate(.all)
        let allToken = state.loadToken(for: "wallet-c")
        state.invalidate(.all)

        XCTAssertTrue(state.invalidated(allToken))
    }

    func testClosingFailureRetriesOnlyTheRequestedWallet() {
        let error = WalletManagerError.WalletLifecycle(
            .managerClosing(walletId: "wallet-a")
        )

        XCTAssertTrue(walletManagerIsClosing(error, walletId: "wallet-a"))
        XCTAssertFalse(walletManagerIsClosing(error, walletId: "wallet-b"))
    }

    func testConstructionInProgressRetriesOnlyTheRequestedWallet() {
        let error = WalletManagerError.WalletLifecycle(
            .constructionInProgress(walletId: "wallet-a")
        )

        XCTAssertTrue(walletManagerConstructionIsInProgress(error, walletId: "wallet-a"))
        XCTAssertFalse(walletManagerConstructionIsInProgress(error, walletId: "wallet-b"))
        XCTAssertFalse(walletManagerIsClosing(error, walletId: "wallet-a"))
    }

    func testCommittedAddressSwitchRecoveryIsNotAPreCommitFailure() {
        let committed = WalletManagerError.AddressTypeSwitchCommittedWithRecoveryPending(
            addressType: .legacy,
            failures: []
        )

        XCTAssertEqual(
            committedAddressTypeSwitchResult(committed, requestedType: .legacy),
            .committedWithRecoveryPending
        )
        XCTAssertNil(
            committedAddressTypeSwitchResult(committed, requestedType: .wrappedSegwit)
        )
        XCTAssertNil(
            committedAddressTypeSwitchResult(
                WalletManagerError.UnableToSwitch(.legacy, "not committed"),
                requestedType: .legacy
            )
        )
    }

    func testRetainedWalletManagerRejectsRustAccessAfterClose() async {
        let manager = WalletManager(preview: .only)
        manager.close()

        XCTAssertFalse(manager.canApplyReconcileMessages)
        XCTAssertFalse(manager.hasRecoveryWords())

        do {
            try await manager.validateMetadata()
            XCTFail("closed manager must reject later Rust access")
        } catch {
            XCTAssertEqual(error.localizedDescription, "Wallet manager is closed")
        }

        manager.close()
        XCTAssertFalse(manager.canApplyReconcileMessages)
    }

    @MainActor
    func testRetainedSendFlowManagerRejectsWorkAndReconciliationAfterClose() async throws {
        let (walletManager, manager) = try makeSendFlowManager()
        defer { walletManager.close() }

        manager.enteringBtcAmount = "before-close"
        manager.debouncedDispatch(.refreshWalletBalance, for: .seconds(10))
        manager.presenter.alertState = .init(.general(title: "Old", message: "Old"))
        manager.apply(
            .setAlert(.general(title: "Queued", message: "Must not appear"))
        )

        manager.close()
        manager.close()

        XCTAssertFalse(manager.canApplyReconcileMessages)
        XCTAssertFalse(manager.validateAmount())
        XCTAssertFalse(manager.validateAddress())
        XCTAssertFalse(manager.amountExceedsBalance())
        XCTAssertNil(
            manager.sanitizeBtcEnteringAmount(oldValue: "1", newValue: "2")
        )
        XCTAssertNil(manager.utxos())
        let initializedAfterClose = await manager.waitForInit()
        XCTAssertFalse(initializedAfterClose)

        manager.dispatch(.refreshWalletBalance)
        manager.debouncedDispatch(.refreshWalletBalance, for: .milliseconds(1))
        manager.apply(.updateEnteringBtcAmount("stale-direct-update"))
        manager.reconcile(message: .updateEnteringBtcAmount("stale-update"))
        await drainMainQueue()
        try await Task.sleep(for: .milliseconds(700))

        XCTAssertEqual(manager.enteringBtcAmount, "before-close")
        XCTAssertNil(manager.presenter.alertState)

        do {
            _ = try await manager.getNewCustomFeeRateWithTotal(
                feeRate: FeeRate.fromSatPerVb(satPerVb: 1),
                feeSpeed: .slow
            )
            XCTFail("closed manager must reject later Rust access")
        } catch {
            XCTAssertEqual(error.localizedDescription, "Send flow manager is closed")
        }
    }

    @MainActor
    func testWalletCacheClearInvalidatesMatchingRetainedSendFlowManagers() throws {
        let (walletManager, retainedManager) = try makeSendFlowManager()
        defer { walletManager.close() }

        let cache = ManagerCache(
            backgroundScanTaskHandler: BackgroundScanTaskHandler(),
            makeSendFlowManager: { walletManager, presenter in
                SendFlowManager(
                    TestSendFlowRustManager(walletId: walletManager.id),
                    presenter: presenter
                )
            }
        )
        let cachedManager = try cache.ensureSendFlowManager(
            walletManager,
            presenter: retainedManager.presenter
        )

        retainedManager.close()
        XCTAssertFalse(retainedManager.canApplyReconcileMessages)
        XCTAssertTrue(cache.cachedSendFlowManager(id: walletManager.id) === cachedManager)

        cache.clearWalletManager(id: "different-wallet")
        XCTAssertTrue(cachedManager.canApplyReconcileMessages)

        cache.clearWalletManager(id: walletManager.id)
        XCTAssertNil(cache.cachedSendFlowManager(id: walletManager.id))
        XCTAssertFalse(cachedManager.canApplyReconcileMessages)

        let replacement = try cache.ensureSendFlowManager(
            walletManager,
            presenter: retainedManager.presenter
        )
        cache.clearWalletManager()

        XCTAssertNil(cache.cachedSendFlowManager(id: walletManager.id))
        XCTAssertFalse(replacement.canApplyReconcileMessages)
    }
}

@MainActor
private final class TestSendFlowRouting: SendFlowRouting {
    func popRoute() {}

    func loadAndReset(to _: Route) {}
}

private final class TestSendFlowRustManager: SendFlowRustManaging {
    private let id: WalletId

    init(walletId: WalletId) {
        self.id = walletId
    }

    func walletId() -> WalletId { id }
    func enteringFiatAmount() -> String { "" }
    func sendAmountFiat() -> String { "" }
    func sendAmountBtc() -> String { "" }
    func totalSpentInFiat() -> String { "" }
    func totalSpentInBtc() -> String { "" }
    func totalFeeString() -> String? { nil }
    func listenForUpdates(reconciler _: SendFlowManagerReconciler) {}
    func validateAddress(displayAlert _: Bool) -> Bool { false }
    func validateAmount(displayAlert _: Bool) -> Bool { false }

    func getCustomFeeOption(
        feeRate _: FeeRate,
        feeSpeed _: FeeSpeed
    ) async throws -> FeeRateOptionWithTotalFee {
        throw TestSendFlowRustManagerError.unexpectedCall
    }

    func waitForInit() async -> Bool { false }
    func amountExceedsBalance() -> Bool { false }
    func sanitizeBtcEnteringAmount(oldValue _: String, newValue _: String) -> String? { nil }
    func sanitizeFiatEnteringAmount(oldValue _: String, newValue _: String) -> String? { nil }
    func utxos() -> [Utxo]? { nil }
    func maxSendMinusFees() -> Amount? { nil }
    func maxSendMinusFeesAndSmallUtxo() -> Amount? { nil }
    func dispatch(action _: SendFlowManagerAction) {}
}

@MainActor
private final class TestWalletManagerDelegate: WalletManagerDelegate {
    func reconcileAfterLabelsChanged(walletId _: WalletId) {}

    func showWalletAlert(_: AppAlertState) {}
}

private enum TestSendFlowRustManagerError: Error {
    case unexpectedCall
}
