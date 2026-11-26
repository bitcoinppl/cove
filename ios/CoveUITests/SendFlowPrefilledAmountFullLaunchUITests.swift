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

        // fee rates come from the fake node, so sends don't depend on mempool.space
        app.launchEnvironment["COVE_FEE_URL"] = "\(Self.esploraUrl)/api/v1/fees/recommended"
        qr.configure(app)
    }

    override func tearDown() {
        qr.remove()
        super.tearDown()
    }

    /// Send opened at a scanned amount checks that amount while its fee is still being
    /// calculated, and must not alert that the fee is still loading, since the fee shows a moment
    /// later and the alert only covers Next
    func testScannedPaymentRequestOpensSendWithoutAFeeStillLoadingAlert() throws {
        try launchFundedWallet()
        useSatsOnWalletScreen()

        openSendFromScannedPaymentRequest(btc: "0.0005")
        waitForLabel(unitSelector, equalTo: "sats", timeout: 10)
        waitForAmountText("50000", "0.0005 BTC requested didn't show as 50,000 sats")

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
        let sent = send.outputs.first { $0.address == XCUIApplication.externalAddress }?.sats
        XCTAssertEqual(sent, 50000, "a request for 0.0005 BTC must send 50,000 sats")
    }

    // MARK: - Setup

    private func launchFundedWallet() throws {
        try node.requireHealthy()
        try node.reset()
        _ = try node.fundWallet(mnemonic: XCUIApplication.testMnemonic, sats: [100_000, 100_000])
        try node.mine()

        app.launchEnvironment["COVE_UI_TEST_ESPLORA_URL"] = Self.esploraUrl
        app.launch(expecting: app.staticTexts["Welcome to Cove"])
        app.importTestMnemonic()
        app.assertBalance(sats: 200_000, timeout: 120)
    }

    private func useSatsOnWalletScreen() {
        let balance = app.element("selectedWallet.balance")
        XCTAssertTrue(balance.waitForExistence(timeout: 15))
        balance.press(forDuration: 1.2)

        let sats = app.buttons["SATS"]
        XCTAssertTrue(sats.waitForExistence(timeout: 5), "the balance's context menu has no SATS item")
        sats.tap()

        // a loaded simulator has taken over 10 seconds to redraw the balance after the menu closes
        waitUntil(balance, matches: NSPredicate(format: "label ENDSWITH %@", " SATS"), timeout: 20, "the balance never showed sats")
    }

    /// Scans a BIP21 request for the amount with Add Wallet's QR scanner, the app-wide scanner,
    /// and sends to it from the alert, which opens Send at the requested amount
    private func openSendFromScannedPaymentRequest(btc amount: String) {
        app.tapButton(named: "app.sidebar.open")
        app.tapButton(named: "Add Wallet")
        app.tapButton(named: "More wallet import options", until: app.buttons["QR"])
        app.buttons["QR"].tap()

        let request = "bitcoin:\(XCUIApplication.externalAddress)?amount=\(amount)"
        XCTAssertNoThrow(try qr.show([request]))

        let sendTo = app.buttons["Send To Address"]
        XCTAssertTrue(sendTo.waitForExistence(timeout: 20), "the scanned request never offered Send To Address")
        sendTo.tap()
    }

    // MARK: - Send screen

    private var amountField: XCUIElement {
        app.textFields["sendFlow.amount"]
    }

    private var unitSelector: XCUIElement {
        app.buttons["sendFlow.unit"]
    }

    /// Waits for the amount field to show the text, ignoring the grouping commas sats get
    private func waitForAmountText(_ text: String, _ message: String) {
        let deadline = Date().addingTimeInterval(10)
        while Date() < deadline {
            if let value = amountField.value as? String, value.replacingOccurrences(of: ",", with: "") == text {
                return
            }

            Thread.sleep(forTimeInterval: 0.25)
        }

        XCTFail("\(message), got '\(amountField.value as? String ?? "")'")
    }
}
