@testable import Cove
import CoveCore
import os
import XCTest

final class CoinControlManagerTests: XCTestCase {
    @MainActor
    func testSelectAllUpdatesSelectionAndTotal() async {
        let sendFlowResolutionProbe = SendFlowResolutionProbe()
        let manager = CoinControlManager(
            RustCoinControlManager.previewNew(outputCount: 2, changeCount: 0),
            resolveSendFlowManager: sendFlowResolutionProbe.resolve
        )
        defer { manager.close() }

        XCTAssertEqual(sendFlowResolutionProbe.count, 0)
        manager.dispatch(.toggleSelectAll)

        let deadline = ContinuousClock.now + .seconds(2)
        while manager.selected.count != 2
            || manager.totalSelectedSats == 0
            || sendFlowResolutionProbe.count == 0
        {
            guard ContinuousClock.now < deadline else {
                XCTFail(
                    "select all did not update selection and total: "
                        + "selected=\(manager.selected.count) total=\(manager.totalSelectedSats) "
                        + "sendFlowResolutions=\(sendFlowResolutionProbe.count)"
                )
                return
            }

            await drainMainQueue()
        }

        let resolutionCount = sendFlowResolutionProbe.count
        manager.continuePressed()

        XCTAssertEqual(sendFlowResolutionProbe.count, resolutionCount + 1)
    }

    @MainActor
    func testContinueCancelsPendingSendFlowUpdateWhenManagerIsUnavailable() async {
        let sendFlowResolutionProbe = SendFlowResolutionProbe()
        let sleepStarted = XCTestExpectation(description: "send-flow update sleep started")
        let sleepCompleted = XCTestExpectation(description: "send-flow update sleep completed")
        var sleepContinuation: CheckedContinuation<Void, Error>?
        let manager = CoinControlManager(
            RustCoinControlManager.previewNew(outputCount: 2, changeCount: 0),
            resolveSendFlowManager: sendFlowResolutionProbe.resolve,
            sleep: { _ in
                try await withCheckedThrowingContinuation { continuation in
                    sleepContinuation = continuation
                    sleepStarted.fulfill()
                }
                sleepCompleted.fulfill()
            }
        )
        defer { manager.close() }

        manager.dispatch(.toggleSelectAll)
        let sleepStartedResult = await XCTWaiter.fulfillment(of: [sleepStarted], timeout: 2)
        XCTAssertEqual(sleepStartedResult, .completed)

        manager.continuePressed()
        XCTAssertEqual(sendFlowResolutionProbe.count, 1)

        sleepContinuation?.resume(returning: ())
        let sleepCompletedResult = await XCTWaiter.fulfillment(of: [sleepCompleted], timeout: 2)
        XCTAssertEqual(sleepCompletedResult, .completed)

        XCTAssertEqual(sendFlowResolutionProbe.count, 1)
    }

    @MainActor
    private func drainMainQueue() async {
        await withCheckedContinuation { continuation in
            DispatchQueue.main.async {
                continuation.resume()
            }
        }
    }
}

private final class SendFlowResolutionProbe: @unchecked Sendable {
    private let resolutionCount = OSAllocatedUnfairLock(initialState: 0)

    var count: Int {
        resolutionCount.withLock { $0 }
    }

    func resolve(_: WalletId) -> SendFlowManager? {
        resolutionCount.withLock { $0 += 1 }
        return nil
    }
}
