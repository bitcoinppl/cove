import XCTest

/// Opens Receive on a wallet whose receive address can't be opened, and checks that the app
/// reports the failure once the sheet has closed
///
/// The failure arrives while the Receive sheet is still sliding up, so the sheet closes and the
/// app raises its alert in the same update, which UIKit can't present until the sheet's dismissal
/// finishes. Opening Receive fails through the debug-only `COVE_UI_TEST_RECEIVE_OPEN_FAILS` seam,
/// set only for the launch that opens Receive. Needs the fake node, which `xtask ios-ui`
/// (`just ios-ui-background`) starts for every run
final class ReceiveFailureFullLaunchUITests: XCTestCase {
    private var app: XCUIApplication!

    override func setUp() {
        super.setUp()
        continueAfterFailure = false

        app = XCUIApplication()
        app.launchEnvironment["COVE_UI_TEST_ESPLORA_URL"] = UITestHostPorts.esploraUrl
    }

    func testReceiveThatCantOpenReportsTheFailure() {
        app.launch(expecting: app.staticTexts["Welcome to Cove"])
        app.importTestMnemonic()
        app.terminate()

        app.launchEnvironment["COVE_UI_TEST_RECEIVE_OPEN_FAILS"] = "1"
        app.launch(expecting: app.element("selectedWallet.balance"))
        app.tapButton(named: "Receive")

        let failure = app.alerts["Unable to Get Address"]
        XCTAssertTrue(failure.waitForExistence(timeout: 15), "Receive failed to open without reporting it")
        XCTAssertFalse(app.buttons["Copy Address"].exists, "Receive stayed open after it failed to open")
    }
}
