import XCTest

/// Changes the wallet's display preferences while a Send screen is open, or just before it opens,
/// against cove-fake-node, then checks what the screen shows and what the app broadcast
///
/// The fake node prices bitcoin at 100,000 per BTC through `COVE_PRICE_URL`, so 0.0005 BTC is
/// 50.00 in fiat. Needs the fake node on the ports `COVE_UI_TEST_PORT_BASE` names, which `xtask
/// ios-ui` (`just ios-ui-background`) starts for every run, and a freshly erased simulator
final class SendFlowDisplayPreferencesFullLaunchUITests: XCTestCase {
    private static let esploraUrl = UITestHostPorts.esploraUrl
    private static let maskedAmount = "**************"

    private var app: XCUIApplication!
    private var node: FakeNodeControl!

    override func setUp() {
        super.setUp()
        continueAfterFailure = false

        app = XCUIApplication()
        node = FakeNodeControl()

        // fee rates and prices come from the fake node, so sends don't depend on mempool.space
        app.launchEnvironment["COVE_FEE_URL"] = "\(Self.esploraUrl)/api/v1/fees/recommended"
        app.launchEnvironment["COVE_PRICE_URL"] = "\(Self.esploraUrl)/api/v1/prices"
    }

    /// The Send header's eye hides the fiat amount under the amount field as well as the balance
    func testHidingAmountsFromTheSendHeaderMasksTheFiatAmount() throws {
        try launchFundedWallet()

        app.startSend(to: XCUIApplication.externalAddress)
        waitForLabel(unitSelector, equalTo: "btc", timeout: 10)
        typeAmount("0.0005")
        waitForSecondaryAmount(containing: "50.00", "0.0005 BTC never showed as 50.00 in fiat")

        tapHeaderVisibility()
        waitForSecondaryAmount(containing: Self.maskedAmount, "hiding amounts left the fiat amount showing")

        tapHeaderVisibility()
        waitForSecondaryAmount(containing: "50.00", "showing amounts again didn't bring back the fiat amount")
    }

    /// Switching the Send screen between bitcoin and fiat entry converts what was typed and sends
    /// the amount typed in fiat
    func testFiatEntryOnTheSendScreenSendsTheTypedAmount() throws {
        try launchFundedWallet()

        app.startSend(to: XCUIApplication.externalAddress)
        waitForLabel(unitSelector, equalTo: "btc", timeout: 10)
        typeAmount("0.0005")
        waitForSecondaryAmount(containing: "50.00", "0.0005 BTC never showed as 50.00 in fiat")

        // to fiat: the field shows the fiat amount and the row under it the bitcoin amount
        tapSecondaryAmount()
        waitForAmountText(fiat: "50.00", "switching to fiat didn't show 50.00 in the amount field")
        waitForSecondaryAmount(containing: "0.0005", "the bitcoin amount didn't move under the fiat field")

        replaceAmount(with: "30")
        waitForAmountText(fiat: "30", "the fiat field didn't take 30")
        waitForSecondaryAmount(containing: "0.0003", "30.00 in fiat didn't price as 0.0003 BTC")

        // back to bitcoin: the field shows what 30.00 bought, in btc
        tapSecondaryAmount()
        waitForLabel(unitSelector, equalTo: "btc", timeout: 10)
        waitForAmountText("0.0003", "switching back to btc didn't convert 30.00")

        app.tapNext()
        app.swipeToSend()

        let send = try node.waitForBroadcasts(count: 1)[0]
        XCTAssertEqual(sentToExternal(send), 30000, "30.00 typed in fiat must send 30,000 sats")
    }

    /// Tapping the wallet balance cycles BTC, fiat, SATS, fiat; Send then opens typing in fiat
    /// and shows bitcoin in sats
    func testSendOpensInTheFiatModeAndUnitTheWalletBalanceCycledTo() throws {
        try launchFundedWallet()

        let balance = app.element("selectedWallet.balance")
        XCTAssertTrue(balance.waitForExistence(timeout: 15))
        tapBalance(balance, until: NSPredicate(format: "label CONTAINS %@", "$"), "the balance never showed fiat")
        tapBalance(balance, until: NSPredicate(format: "label ENDSWITH %@", " SATS"), "the balance never showed sats")
        tapBalance(balance, until: NSPredicate(format: "label CONTAINS %@", "$"), "the balance never went back to fiat")

        app.startSend(to: XCUIApplication.externalAddress)
        XCTAssertTrue(amountField.waitForExistence(timeout: 10))
        XCTAssertFalse(unitSelector.exists, "fiat entry has no bitcoin unit selector")

        replaceAmount(with: "20")
        waitForAmountText(fiat: "20", "the fiat field didn't take 20")
        waitForSecondaryAmount(containing: "20,000", unit: "sats", "20.00 in fiat didn't show as 20,000 sats")

        tapSecondaryAmount()
        waitForLabel(unitSelector, equalTo: "sats", timeout: 10)
        waitForAmountText("20000", "switching to bitcoin entry didn't show 20,000 sats")

        app.tapNext()
        app.swipeToSend()

        let send = try node.waitForBroadcasts(count: 1)[0]
        XCTAssertEqual(sentToExternal(send), 20000, "20.00 typed in fiat must send 20,000 sats")
    }

    /// Coin control's set amount screen shows the amount in the unit picked there, with the fiat
    /// amount under it, and sends the amount it shows
    func testCoinControlSendsTheAmountShownInTheUnitPickedThere() throws {
        try launchFundedWallet()

        openManageUtxos()
        selectAllCoins()
        app.tapButton(named: "coinControl.continue", until: coinControlAmount)
        app.enterAddress(XCUIApplication.externalAddress)

        // both coins less the fee, so the amount itself is read back rather than assumed
        selectCoinControlUnit("btc")
        waitForCoinControlAmount("0.00199023", "both coins less the fee didn't show in btc")

        selectCoinControlUnit("sats")
        waitForCoinControlAmount("199023", "switching coin control to sats didn't convert the amount")
        waitUntil(
            app.element("sendFlow.coinControl.fiatAmount"),
            matches: NSPredicate(format: "label CONTAINS %@", "199.02"),
            timeout: 30,
            "199,023 sats never showed as 199.02 in fiat"
        )

        app.tapNext()
        app.swipeToSend()

        let send = try node.waitForBroadcasts(count: 1)[0]
        XCTAssertEqual(sentToExternal(send), 199_023, "coin control must send the 199,023 sats it showed")
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

    private func tapBalance(_ balance: XCUIElement, until predicate: NSPredicate, _ message: String) {
        balance.tap()
        waitUntil(balance, matches: predicate, timeout: 10, message)
    }

    // MARK: - Send screen

    private var amountField: XCUIElement {
        app.textFields["sendFlow.amount"]
    }

    private var unitSelector: XCUIElement {
        app.buttons["sendFlow.unit"]
    }

    private var secondaryAmount: XCUIElement {
        app.element("sendFlow.secondaryAmount")
    }

    private func typeAmount(_ amount: String) {
        XCTAssertTrue(amountField.waitForExistence(timeout: 10))
        amountField.tap()
        amountField.typeTextOnceFocused(amount)
        waitForAmountText(amount, "the amount field didn't take \(amount)")
    }

    /// Deletes what the amount field shows, then types the new amount
    private func replaceAmount(with amount: String) {
        XCTAssertTrue(amountField.waitForExistence(timeout: 10))
        amountField.tap()

        let current = amountField.value as? String ?? ""
        let deletes = String(repeating: XCUIKeyboardKey.delete.rawValue, count: current.count + 1)
        amountField.typeTextOnceFocused(deletes + amount)
    }

    /// Waits for the amount field to show the text, ignoring the grouping commas sats get
    private func waitForAmountText(_ text: String, _ message: String) {
        waitForAmountField(message) { $0.replacingOccurrences(of: ",", with: "") == text }
    }

    /// Waits for the fiat amount field to show the amount after its currency symbol, with or
    /// without cents
    private func waitForAmountText(fiat amount: String, _ message: String) {
        waitForAmountField(message) { value in
            let digits = value.filter { $0.isNumber || $0 == "." }
            return digits == amount || digits == "\(amount).00"
        }
    }

    private func waitForAmountField(_ message: String, _ matches: (String) -> Bool) {
        let deadline = Date().addingTimeInterval(10)
        while Date() < deadline {
            if let value = amountField.value as? String, matches(value) { return }

            Thread.sleep(forTimeInterval: 0.25)
        }

        XCTFail("\(message), got '\(amountField.value as? String ?? "")'")
    }

    private func waitForSecondaryAmount(containing text: String, _ message: String) {
        XCTAssertTrue(secondaryAmount.waitForExistence(timeout: 10))
        waitUntil(secondaryAmount, matches: NSPredicate(format: "label CONTAINS %@", text), timeout: 30, message)
    }

    /// Waits for the amount and its unit; the combined row's label joins them with a comma
    private func waitForSecondaryAmount(containing text: String, unit: String, _ message: String) {
        XCTAssertTrue(secondaryAmount.waitForExistence(timeout: 10))
        let predicate = NSPredicate(format: "label CONTAINS %@ AND label CONTAINS[c] %@", text, unit)
        waitUntil(secondaryAmount, matches: predicate, timeout: 30, message)
    }

    private func tapSecondaryAmount() {
        XCTAssertTrue(secondaryAmount.waitForExistence(timeout: 10))
        secondaryAmount.tap()
    }

    private func tapHeaderVisibility() {
        let eye = app.buttons["sendFlow.header.visibility"]
        XCTAssertTrue(eye.waitForExistence(timeout: 10))
        eye.tap()
    }

    // MARK: - Coin control

    private var coinControlAmount: XCUIElement {
        app.element("sendFlow.coinControl.amount")
    }

    private func openManageUtxos() {
        let more = app.buttons["selectedWallet.more"]
        XCTAssertTrue(more.waitForExistence(timeout: 15))
        more.tap()

        app.tapButton(named: "selectedWallet.manageUtxos", until: app.navigationBars["Manage UTXOs"])
    }

    private func selectAllCoins() {
        let selectAll = app.element("coinControl.selectAll")
        XCTAssertTrue(selectAll.waitForExistence(timeout: 30))
        waitForLabel(selectAll, equalTo: "Select All", timeout: 10)
        selectAll.tap()
        waitForLabel(selectAll, equalTo: "Deselect All", timeout: 5)
    }

    private func selectCoinControlUnit(_ unit: String) {
        let menu = app.element("sendFlow.coinControl.unit")
        XCTAssertTrue(menu.waitForExistence(timeout: 10))

        // the address keyboard can cover the menu
        if !menu.isHittable, app.keyboards.count > 0 {
            app.buttons["Done"].firstMatch.tap()
        }

        menu.tap()

        let option = app.buttons["sendFlow.coinControl.unit.\(unit)"]
        XCTAssertTrue(option.waitForExistence(timeout: 5), "the coin control unit menu has no \(unit) option")
        option.tap()

        waitUntil(menu, matches: NSPredicate(format: "label BEGINSWITH %@", unit), timeout: 10, "the unit menu never showed \(unit)")
    }

    /// Waits for coin control's amount to show the text, ignoring grouping commas
    private func waitForCoinControlAmount(_ text: String, _ message: String) {
        let deadline = Date().addingTimeInterval(10)
        while Date() < deadline {
            if coinControlAmount.label.replacingOccurrences(of: ",", with: "") == text { return }

            Thread.sleep(forTimeInterval: 0.25)
        }

        XCTFail("\(message), got '\(coinControlAmount.label)'")
    }

    private func sentToExternal(_ broadcast: FakeNodeControl.Broadcast) -> UInt64? {
        broadcast.outputs.first { $0.address == XCUIApplication.externalAddress }?.sats
    }
}
