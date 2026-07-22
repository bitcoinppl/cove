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

    func testCacheRaceUsesMatchingWinnerAndRejectsSupersedingReplacement() {
        XCTAssertEqual(
            WalletManagerCacheLoadDecision.resolve(
                targetId: "wallet-b",
                capturedGeneration: 1,
                currentGeneration: 2,
                cachedWalletId: "wallet-b"
            ),
            .useCached
        )
        XCTAssertEqual(
            WalletManagerCacheLoadDecision.resolve(
                targetId: "wallet-b",
                capturedGeneration: 1,
                currentGeneration: 2,
                cachedWalletId: "wallet-c"
            ),
            .cancelLoaded
        )
        XCTAssertEqual(
            WalletManagerCacheLoadDecision.resolve(
                targetId: "wallet-b",
                capturedGeneration: 1,
                currentGeneration: 1,
                cachedWalletId: "wallet-a"
            ),
            .installLoaded
        )
    }

    func testCacheRaceInstallsTargetAfterUnrelatedClear() {
        XCTAssertEqual(
            WalletManagerCacheLoadDecision.resolve(
                targetId: "wallet-b",
                capturedGeneration: 1,
                currentGeneration: 2,
                cachedWalletId: nil
            ),
            .installLoaded
        )
    }
}
