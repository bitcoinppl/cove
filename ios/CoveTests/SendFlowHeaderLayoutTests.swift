@testable import Cove
import CoveCore
import SwiftUI
import XCTest

@MainActor
final class SendFlowHeaderLayoutTests: XCTestCase {
    /// Regression for #926: a fixed header height clipped the balance and unit at the
    /// largest accessibility text size instead of letting the header grow
    func testHeaderGrowsWithAccessibilityTextSize() {
        let manager = WalletManager(preview: .only)

        let defaultHeight = headerHeight(manager: manager, dynamicTypeSize: .large)
        let accessibilityHeight = headerHeight(manager: manager, dynamicTypeSize: .accessibility5)

        XCTAssertGreaterThan(
            accessibilityHeight,
            defaultHeight,
            "header stays \(accessibilityHeight)pt tall at accessibility5, so its balance row clips"
        )
    }

    private func headerHeight(manager: WalletManager, dynamicTypeSize: DynamicTypeSize) -> CGFloat {
        let header = SendFlowHeaderView(manager: manager, amount: Amount.fromSat(sats: 1_436_900))
            .environment(\.dynamicTypeSize, dynamicTypeSize)

        let hostingController = UIHostingController(rootView: header)
        let proposal = CGSize(width: 393, height: CGFloat.greatestFiniteMagnitude)

        return hostingController.sizeThatFits(in: proposal).height
    }
}
