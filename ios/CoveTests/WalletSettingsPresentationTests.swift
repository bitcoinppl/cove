@testable import Cove
import XCTest

final class WalletSettingsPresentationTests: XCTestCase {
    private struct XprvLoadError: Error {}

    @MainActor
    func testXprvRevealLoadsOnceAndClearsAtEnd() {
        let presentation = WalletSettingsXprvRevealPresentation()
        var loadCount = 0
        let loadXprv = {
            loadCount += 1
            return "sensitive xprv"
        }

        presentation.load(using: loadXprv)
        presentation.load(using: loadXprv)

        XCTAssertEqual(loadCount, 1)
        guard case let .loaded(xprv) = presentation.state else {
            return XCTFail("Expected the private key to load")
        }
        XCTAssertEqual(xprv, "sensitive xprv")

        presentation.end()
        presentation.load(using: loadXprv)

        XCTAssertEqual(loadCount, 1)
        guard case .ended = presentation.state else {
            return XCTFail("Expected the private key to be cleared")
        }
    }

    @MainActor
    func testEndedXprvRevealDoesNotLoadAgainAfterFailure() {
        let presentation = WalletSettingsXprvRevealPresentation()
        var loadCount = 0
        let loadXprv: () throws -> String = {
            loadCount += 1
            throw XprvLoadError()
        }

        presentation.load(using: loadXprv)

        XCTAssertEqual(loadCount, 1)
        guard case .failed = presentation.state else {
            return XCTFail("Expected the private key load to fail")
        }

        presentation.end()
        presentation.load(using: loadXprv)

        XCTAssertEqual(loadCount, 1)
        guard case .ended = presentation.state else {
            return XCTFail("Expected the failed presentation to stay ended")
        }
    }
}
