import XCTest

/// Opens Send with an amount already entered, from a payment request scanned with the app-wide
/// scanner, against cove-fake-node, then sends it
///
/// Payment requests reach the scanner through the debug QR seam (`COVE_UI_TEST_QR_SCRIPT`). Needs
/// the fake node on the ports `COVE_UI_TEST_PORT_BASE` names, which `xtask ios-ui`
/// (`just ios-ui-background`) starts for every run, and a freshly erased simulator
final class SendFlowPrefilledAmountFullLaunchUITests: XCTestCase {
    private static let esploraUrl = UITestHostPorts.esploraUrl

    private var app: XCUIApplication!
    private var node: FakeNodeControl!
    private var qr: QrScript!

    override func setUp() {
        super.setUp()
        continueAfterFailure = false

        app = XCUIApplication()
        node = FakeNodeControl()
        qr = QrScript()

        // fee rates and prices come from the fake node, so sends don't depend on mempool.space
        app.launchEnvironment["COVE_FEE_URL"] = "\(Self.esploraUrl)/api/v1/fees/recommended"
        app.launchEnvironment["COVE_PRICE_URL"] = "\(Self.esploraUrl)/api/v1/prices"
        qr.configure(app)
    }

    override func tearDown() {
        qr.remove()
        super.tearDown()
    }

    /// Send opened at a scanned amount in sats shows it in sats with its fiat price, checks that
    /// amount while its fee is still being calculated, and must not alert that the fee is still
    /// loading, since the fee shows a moment later and the alert only covers Next
    func testScannedPaymentRequestOpensSendWithoutAFeeStillLoadingAlert() throws {
        try app.launchFundedWallet(node: node)
        app.useSatsOnWalletScreen()

        app.openSendFromScannedPaymentRequest(btc: "0.0005", qr: qr)
        waitForLabel(app.sendUnitSelector, equalTo: "sats", timeout: 10)
        app.waitForSendAmountText("50000", "0.0005 BTC requested didn't show as 50,000 sats")

        // the fake node prices bitcoin at 100,000 per BTC
        let secondaryAmount = app.element("sendFlow.secondaryAmount")
        XCTAssertTrue(secondaryAmount.waitForExistence(timeout: 10))
        waitUntil(secondaryAmount, matches: NSPredicate(format: "label CONTAINS %@", "50.00"), timeout: 30, "50,000 sats never showed as 50.00 in fiat")

        // the screen checks a prefilled amount 700 ms after it shows, and the fee total lands
        // within a few seconds even on a loaded simulator
        let feeAlert = app.alerts["Fee Estimate Still Loading"]
        XCTAssertFalse(
            feeAlert.waitForExistence(timeout: 8),
            "Send opened at a scanned amount alerted that the fee was still loading"
        )

        app.tapNext()
        app.swipeToSend()

        let send = try node.waitForBroadcasts(count: 1)[0]
        XCTAssertEqual(send.sentToExternal, 50000, "a request for 0.0005 BTC must send 50,000 sats")
    }
}
