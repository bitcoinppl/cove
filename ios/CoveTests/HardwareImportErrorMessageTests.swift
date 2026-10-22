@testable import Cove
import CoveCore
import XCTest

final class HardwareImportErrorMessageTests: XCTestCase {
    /// regression for #923: the alert showed the UniFFI enum dump instead of the Rust message
    func testInvalidKeyAlertShowsTheReasonInsteadOfTheEnumDump() {
        let error = WalletError.ParseXpubError(
            .InvalidDescriptor(.InvalidDescriptor("no supported wallet export format matched"))
        )

        let message = hardwareImportErrorMessage(error)

        XCTAssertFalse(message.contains("CoveCore."), "alert shows the raw enum: \(message)")
        XCTAssertTrue(message.contains("no supported wallet export format matched"), message)
    }
}
