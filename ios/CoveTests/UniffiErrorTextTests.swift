import CoveCore
import XCTest

/// Generated UniFFI errors default `localizedDescription` to a reflection dump such as
/// `CoveCore.AuthManagerError.DecoySet(...)`, and `uniffi_error_text.swift` routes it to the Rust text
final class UniffiErrorTextTests: XCTestCase {
    func testStructuredErrorShowsItsRustDisplayText() {
        let error: Error = AuthManagerError.DecoySet(.SameAsCurrentPin)

        XCTAssertEqual(
            error.localizedDescription,
            "Unable to set the decoy PIN, because it is the same as your current PIN"
        )
    }

    /// Regression for #923, where the hardware import alert showed the enum dump
    func testNestedErrorShowsTheInnerReason() {
        let error: Error = WalletError.ParseXpubError(
            .InvalidDescriptor(.InvalidDescriptor("no supported wallet export format matched"))
        )

        let message = error.localizedDescription

        XCTAssertFalse(message.contains("CoveCore."), "alert shows the raw enum: \(message)")
        XCTAssertTrue(message.contains("no supported wallet export format matched"), message)
    }

    func testFlatErrorShowsItsMessage() {
        let error: Error = DiagnosticsError.Submit(message: "Failed to submit diagnostics report: offline")

        XCTAssertEqual(error.localizedDescription, "Failed to submit diagnostics report: offline")
        XCTAssertEqual("\(error)", "Failed to submit diagnostics report: offline")
    }
}
