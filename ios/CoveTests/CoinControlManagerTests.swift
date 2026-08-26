@testable import Cove
import CoveCore
import XCTest

final class CoinControlManagerTests: XCTestCase {
    @MainActor
    func testSelectAllUpdatesSelectionAndTotal() async {
        let manager = CoinControlManager(
            RustCoinControlManager.previewNew(outputCount: 2, changeCount: 0)
        )
        defer { manager.close() }

        manager.dispatch(.toggleSelectAll)

        let deadline = ContinuousClock.now + .seconds(2)
        while manager.selected.count != 2 || manager.totalSelectedSats == 0 {
            guard ContinuousClock.now < deadline else {
                XCTFail(
                    "select all did not update selection and total: "
                        + "selected=\(manager.selected.count) total=\(manager.totalSelectedSats)"
                )
                return
            }

            await drainMainQueue()
        }
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
