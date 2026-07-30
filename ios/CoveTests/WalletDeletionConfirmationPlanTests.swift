@testable import Cove
import XCTest

final class WalletDeletionConfirmationPlanTests: XCTestCase {
    func testRequiredConfirmationCountMapsToSupportedPlan() {
        XCTAssertEqual(WalletDeletionConfirmationPlan(requiredConfirmations: 0), .oneStep)
        XCTAssertEqual(WalletDeletionConfirmationPlan(requiredConfirmations: 1), .oneStep)
        XCTAssertEqual(WalletDeletionConfirmationPlan(requiredConfirmations: 2), .twoSteps)
        XCTAssertEqual(WalletDeletionConfirmationPlan(requiredConfirmations: 3), .threeSteps)
        XCTAssertEqual(WalletDeletionConfirmationPlan(requiredConfirmations: .max), .threeSteps)
    }
}
