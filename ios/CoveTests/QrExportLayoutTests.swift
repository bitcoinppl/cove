@testable import Cove
import SwiftUI
import XCTest

@MainActor
final class QrExportLayoutTests: XCTestCase {
    func testManyQrFramesFitAvailableWidth() {
        let availableSize = CGSize(width: 260, height: 12)
        let controller = UIHostingController(
            rootView: QrExportProgressIndicator(qrCount: 250, currentIndex: 0)
        )

        let fittedSize = controller.sizeThatFits(in: availableSize)

        XCTAssertLessThanOrEqual(fittedSize.width, availableSize.width)
        XCTAssertEqual(fittedSize.height, availableSize.height)
    }
}
