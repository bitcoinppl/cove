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
