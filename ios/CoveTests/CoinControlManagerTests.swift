@testable import Cove
import CoveCore
import XCTest

final class CoinControlManagerTests: XCTestCase {
    private let maxSend = 435_000.0
    private let softMaxSend = 434_400.0

    private var smartSnapBand: CoinControlSmartSnapBand {
        CoinControlSmartSnapBand(
            softMaxSend: softMaxSend,
            maxSend: maxSend,
            step: 10
        )
    }

    func testSliderSynchronizationReplacesAmountDirectionAndPin() {
        var state = smartSnapBand.synchronize(amount: maxSend)
        state = smartSnapBand.snap(state: state, raw: maxSend - 5)
        state = smartSnapBand.synchronize(amount: 117_716)

        XCTAssertEqual(
            state,
            CoinControlSmartSnap(
                pinState: .none,
                amount: 117_716,
                previousRaw: 117_716
            )
        )
        XCTAssertEqual(
            smartSnapBand.snap(state: state, raw: 117_726),
            CoinControlSmartSnap(
                pinState: .none,
                amount: 117_726,
                previousRaw: 117_726
            )
        )
    }

    func testSliderSynchronizationRestoresSoftAndHardPins() {
        let soft = smartSnapBand.synchronize(amount: softMaxSend)
        XCTAssertEqual(
            smartSnapBand.snap(state: soft, raw: softMaxSend - 5),
            CoinControlSmartSnap(
                pinState: .soft,
                amount: softMaxSend,
                previousRaw: softMaxSend - 5
            )
        )

        let hard = smartSnapBand.synchronize(amount: maxSend)
        XCTAssertEqual(
            smartSnapBand.snap(state: hard, raw: maxSend - 5),
            CoinControlSmartSnap(
                pinState: .soft,
                amount: softMaxSend,
                previousRaw: maxSend - 5
            )
        )
    }

    func testSliderSynchronizationPrefersHardPinWhenLimitsAreEqual() {
        let band = CoinControlSmartSnapBand(
            softMaxSend: maxSend,
            maxSend: maxSend,
            step: 10
        )

        XCTAssertEqual(
            band.synchronize(amount: maxSend),
            CoinControlSmartSnap(
                pinState: .hard,
                amount: maxSend,
                previousRaw: maxSend
            )
        )
    }

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
