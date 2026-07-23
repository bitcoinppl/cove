@testable import Cove
import CoveCore
import XCTest

final class WalletTransitionTests: XCTestCase {
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
}
