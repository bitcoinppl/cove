@testable import Cove
import UIKit
import XCTest

final class PresenterIdleTests: XCTestCase {
    @MainActor
    func testSettledSheetChainIsIdle() {
        let root = StubController()
        root.stubPresented = StubController()

        XCTAssertTrue(PresenterIdle.isIdle(from: root))
        XCTAssertTrue(PresenterIdle.isIdle(from: nil))
    }

    @MainActor
    func testAlertAnywhereInChainIsBusy() {
        let root = StubController()
        let sheet = StubController()
        root.stubPresented = sheet
        sheet.stubPresented = UIAlertController(title: nil, message: nil, preferredStyle: .alert)

        XCTAssertFalse(PresenterIdle.isIdle(from: root))
    }

    @MainActor
    func testDismissingSheetIsBusy() {
        let root = StubController()
        let sheet = StubController()
        sheet.stubBeingDismissed = true
        root.stubPresented = sheet

        XCTAssertFalse(PresenterIdle.isIdle(from: root))
    }

    func testSettleNeedsConsecutiveIdleChecks() {
        let now = ContinuousClock.now
        var settle = PresenterIdle.Settle(deadline: now + .seconds(1))

        let steps = [true, false, true, true].map { settle.record(idle: $0, at: now) }

        XCTAssertEqual(steps, [.poll, .poll, .poll, .settled])
    }

    func testSettleTimesOutWithoutConsecutiveIdleChecks() {
        let start = ContinuousClock.now
        var settle = PresenterIdle.Settle(deadline: start + .milliseconds(100))

        XCTAssertEqual(settle.record(idle: false, at: start), .poll)
        XCTAssertEqual(settle.record(idle: true, at: start + .milliseconds(100)), .timedOut)
    }
}

private final class StubController: UIViewController {
    var stubPresented: UIViewController?
    var stubBeingDismissed = false

    override var presentedViewController: UIViewController? {
        stubPresented
    }

    override var isBeingDismissed: Bool {
        stubBeingDismissed
    }
}
