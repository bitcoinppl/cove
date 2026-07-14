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

    func testAtomicCreationFailureLeavesCachedManagerInstalled() {
        let cached = TestWalletManager(id: "wallet-a")
        var installed = cached

        XCTAssertThrowsError(
            try getOrCreateWalletManagerAtomically(
                cachedManager: cached,
                requestedId: "wallet-b",
                id: { $0.id },
                create: { throw TestError.creationFailed },
                install: {
                    installed = $0
                    return $0
                }
            )
        )
        XCTAssertTrue(installed === cached)
    }

    func testAtomicCreationInstallsOnlyAfterSuccess() throws {
        let cached = TestWalletManager(id: "wallet-a")
        let candidate = TestWalletManager(id: "wallet-b")
        var events: [String] = []

        let result = try getOrCreateWalletManagerAtomically(
            cachedManager: cached,
            requestedId: candidate.id,
            id: { $0.id },
            create: {
                events.append("created")
                return candidate
            },
            install: {
                events.append("installed")
                return $0
            }
        )

        XCTAssertTrue(result === candidate)
        XCTAssertEqual(events, ["created", "installed"])
    }

    func testCacheRaceUsesMatchingWinnerAndRejectsUnrelatedReplacement() {
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
}

private final class TestWalletManager {
    let id: WalletId

    init(id: WalletId) {
        self.id = id
    }
}

private enum TestError: Error {
    case creationFailed
}
