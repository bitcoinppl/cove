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

    func testCleanupWarningUsesSingularVerbForOneWallet() {
        let message = backupCleanupWarningMessage(walletCount: 1)

        XCTAssertTrue(message.hasPrefix("1 wallet has "))
    }

    func testCleanupWarningUsesPluralVerbForMultipleWallets() {
        let message = backupCleanupWarningMessage(walletCount: 2)

        XCTAssertTrue(message.hasPrefix("2 wallets have "))
    }
}
