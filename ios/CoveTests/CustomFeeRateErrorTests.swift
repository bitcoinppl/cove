@testable import Cove
import XCTest

final class CustomFeeRateErrorTests: XCTestCase {
    func testTopLevelInsufficientFundsIsTooHighCustomFeeError() {
        XCTAssertTrue(isTooHighCustomFeeError(SendFlowError.InsufficientFunds))
    }

    func testWrappedWalletInsufficientFundsIsTooHighCustomFeeError() {
        let error = SendFlowError.WalletManager(.InsufficientFunds("not enough funds"))

        XCTAssertTrue(isTooHighCustomFeeError(error))
    }

    func testOtherWalletErrorsAreNotTooHighCustomFeeErrors() {
        let error = SendFlowError.WalletManager(.LockedOutputsSelected)

        XCTAssertFalse(isTooHighCustomFeeError(error))
    }
}
