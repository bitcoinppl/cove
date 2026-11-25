import XCTest

/// Sends in the unit picked on the wallet screen against cove-fake-node, then checks what the app
/// broadcast
///
/// Needs the fake node on the ports `COVE_UI_TEST_PORT_BASE` names, which `xtask ios-ui`
/// (`just ios-ui-background`) starts for every run, and a freshly erased simulator, which xtask
/// also provides
final class SendFlowUnitFullLaunchUITests: XCTestCase {
    private static let esploraUrl = UITestHostPorts.esploraUrl
    private static let feeUrl = "\(esploraUrl)/api/v1/fees/recommended"

    private var app: XCUIApplication!
    private var node: FakeNodeControl!

    override func setUp() {
        super.setUp()
        continueAfterFailure = false

        app = XCUIApplication()
        node = FakeNodeControl()

        // fee rates come from the fake node on every launch, so sends don't depend on mempool.space
        app.launchEnvironment["COVE_FEE_URL"] = Self.feeUrl
    }

    func testSatsPickedOnWalletScreenIsTheSendAmountUnit() throws {
        try node.requireHealthy()
        try node.reset()
        _ = try node.fundWallet(mnemonic: XCUIApplication.testMnemonic, sats: [100_000, 100_000])
        try node.mine()

        app.launchEnvironment["COVE_UI_TEST_ESPLORA_URL"] = Self.esploraUrl
        app.launch(expecting: app.staticTexts["Welcome to Cove"])
        app.importTestMnemonic()
        app.assertBalance(sats: 200_000, timeout: 120)

        // 1: show the balance in sats, then send an amount typed in sats
        useSatsOnWalletScreen()
        app.startSend(to: XCUIApplication.externalAddress)
        waitForLabel(unitSelector, equalTo: "sats", timeout: 10)
        typeAmount("50000")
        app.tapNext()
        app.swipeToSend()

        let satsSend = try node.waitForBroadcasts(count: 1)[0]
        XCTAssertEqual(sentToExternal(satsSend), 50000, "50000 typed in sats must send 50,000 sats")

        // 2: switching the send screen back to btc converts what was typed and sends in btc
        app.startSend(to: XCUIApplication.externalAddress)
        waitForLabel(unitSelector, equalTo: "sats", timeout: 10)
        typeAmount("20000")
        selectUnit("btc")
        waitForAmountText("0.0002", "20000 sats didn't convert to 0.0002 btc")
        app.tapNext()
        app.swipeToSend()

        let btcSend = try node.waitForBroadcasts(count: 2)[1]
        XCTAssertEqual(sentToExternal(btcSend), 20000, "0.0002 btc must send 20,000 sats")
    }

    /// A Send screen opened in btc, then left, keeps no hold on the unit when the wallet switches
    /// to sats and Send opens again straight away
    func testSatsPickedRightAfterABtcSendIsTheSendAmountUnit() throws {
        try node.requireHealthy()
        try node.reset()
        _ = try node.fundWallet(mnemonic: XCUIApplication.testMnemonic, sats: [100_000, 100_000])
        try node.mine()

        app.launchEnvironment["COVE_UI_TEST_ESPLORA_URL"] = Self.esploraUrl
        app.launch(expecting: app.staticTexts["Welcome to Cove"])
        app.importTestMnemonic()
        app.assertBalance(sats: 200_000, timeout: 120)

        // a send session in btc that the user backs out of
        app.startSend(to: XCUIApplication.externalAddress)
        waitForLabel(unitSelector, equalTo: "btc", timeout: 10)
        app.goBack()

        // switch to sats and open Send without waiting for the wallet screen to settle
        useSatsOnWalletScreen(waitForBalance: false)
        app.startSend(to: XCUIApplication.externalAddress)
        waitForLabel(unitSelector, equalTo: "sats", timeout: 10)
        typeAmount("50000")
        app.tapNext()
        app.swipeToSend()

        let send = try node.waitForBroadcasts(count: 1)[0]
        XCTAssertEqual(sentToExternal(send), 50000, "50000 typed in sats must send 50,000 sats")
    }

    // MARK: - Wallet screen

    private func useSatsOnWalletScreen(waitForBalance: Bool = true) {
        let balance = app.element("selectedWallet.balance")
        XCTAssertTrue(balance.waitForExistence(timeout: 15))
        balance.press(forDuration: 1.2)

        let sats = app.buttons["SATS"]
        XCTAssertTrue(sats.waitForExistence(timeout: 5), "the balance's context menu has no SATS item")
        sats.tap()

        guard waitForBalance else { return }
        waitUntil(balance, matches: NSPredicate(format: "label ENDSWITH %@", " SATS"), timeout: 10, "the balance never showed sats")
    }

    // MARK: - Send screen

    private var amountField: XCUIElement {
        app.textFields["sendFlow.amount"]
    }

    private var unitSelector: XCUIElement {
        app.buttons["sendFlow.unit"]
    }

    private func typeAmount(_ amount: String) {
        XCTAssertTrue(amountField.waitForExistence(timeout: 10))
        amountField.tap()
        amountField.typeTextOnceFocused(amount)
        waitForAmountText(amount, "the amount field didn't take \(amount)")
    }

    /// Waits for the amount field to show the text, ignoring the grouping commas sats get
    private func waitForAmountText(_ text: String, _ message: String) {
        let deadline = Date().addingTimeInterval(10)
        while Date() < deadline {
            let value = amountField.value as? String
            if value?.replacingOccurrences(of: ",", with: "") == text { return }

            Thread.sleep(forTimeInterval: 0.25)
        }

        XCTFail("\(message), got '\(amountField.value as? String ?? "")'")
    }

    private func selectUnit(_ unit: String) {
        XCTAssertTrue(unitSelector.waitForExistence(timeout: 10))
        unitSelector.tap()

        let option = app.buttons["sendFlow.unit.\(unit)"]
        XCTAssertTrue(option.waitForExistence(timeout: 5), "the unit menu has no \(unit) option")
        option.tap()

        waitForLabel(unitSelector, equalTo: unit, timeout: 10)
    }

    private func sentToExternal(_ broadcast: FakeNodeControl.Broadcast) -> UInt64? {
        broadcast.outputs.first { $0.address == XCUIApplication.externalAddress }?.sats
    }
}
