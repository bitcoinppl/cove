import XCTest

/// Changes the wallet's display preferences while a Send screen is open, or just before it opens,
/// against cove-fake-node, then checks what the screen shows and what the app broadcast
///
/// The fake node prices bitcoin at 100,000 per BTC through `COVE_PRICE_URL`, so 0.0005 BTC is
/// 50.00 in fiat. Payment requests reach the app-wide scanner through the debug QR seam
/// (`COVE_UI_TEST_QR_SCRIPT`). Needs the fake node on the ports `COVE_UI_TEST_PORT_BASE` names,
/// which `xtask ios-ui` (`just ios-ui-background`) starts for every run, and a freshly erased
/// simulator
final class SendFlowDisplayPreferencesFullLaunchUITests: XCTestCase {
    private static let esploraUrl = UITestHostPorts.esploraUrl
    private static let maskedAmount = "**************"

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

    /// A payment request scanned while the wallet shows sats opens Send with its amount in sats
    func testScannedPaymentRequestOpensSendWithItsAmountInSats() throws {
        try launchFundedWallet()
        useSatsOnWalletScreen()

        openSendFromScannedPaymentRequest(btc: "0.0005")
        waitForLabel(unitSelector, equalTo: "sats", timeout: 10)
        waitForAmountText("50000", "0.0005 BTC requested didn't show as 50,000 sats")
        waitForSecondaryAmount(containing: "50.00", "50,000 sats never showed as 50.00 in fiat")

        dismissFeeStillLoadingAlert()
        app.tapNext()
        app.swipeToSend()

        let send = try node.waitForBroadcasts(count: 1)[0]
        XCTAssertEqual(sentToExternal(send), 50000, "a request for 0.0005 BTC must send 50,000 sats")
    }

    /// A payment request scanned while the wallet shows fiat opens Send with its amount in fiat
    func testScannedPaymentRequestOpensSendWithItsAmountInFiat() throws {
        try launchFundedWallet()
        useFiatOnWalletScreen()

        openSendFromScannedPaymentRequest(btc: "0.0005")
        XCTAssertTrue(amountField.waitForExistence(timeout: 10))
        XCTAssertFalse(unitSelector.exists, "fiat entry has no bitcoin unit selector")
        waitForAmountText(fiat: "50.00", "0.0005 BTC requested didn't show as 50.00 in fiat")
        waitForSecondaryAmount(containing: "0.0005", "the requested bitcoin amount didn't show under the fiat field")

        dismissFeeStillLoadingAlert()
        app.tapNext()
        app.swipeToSend()

        let send = try node.waitForBroadcasts(count: 1)[0]
        XCTAssertEqual(sentToExternal(send), 50000, "a request for 0.0005 BTC must send 50,000 sats")
    }

    /// Max in sats shows what a drain of every spendable coin leaves after the fee, and sends
    /// exactly that in a transaction with no change
    func testMaxInSatsSendsTheWholeBalanceLessTheFee() throws {
        try launchFundedWallet()
        useSatsOnWalletScreen()

        app.startSend(to: XCUIApplication.externalAddress)
        waitForLabel(unitSelector, equalTo: "sats", timeout: 10)
        app.tapMax()
        waitForAmountText("199023", "Max didn't show both coins less the fee in sats")
        waitForSecondaryAmount(containing: "199.02", "the Max amount never showed as 199.02 in fiat")

        app.tapNext()
        app.swipeToSend()

        let send = try node.waitForBroadcasts(count: 1)[0]
        assertDrainsBothCoins(send, sending: 199_023)
    }

    /// Max in fiat shows the drain priced in fiat, and still sends the exact drain rather than
    /// the rounded fiat amount converted back
    func testMaxInFiatSendsTheWholeBalanceLessTheFee() throws {
        try launchFundedWallet()
        useFiatOnWalletScreen()

        app.startSend(to: XCUIApplication.externalAddress)
        XCTAssertTrue(amountField.waitForExistence(timeout: 10))
        XCTAssertFalse(unitSelector.exists, "fiat entry has no bitcoin unit selector")
        app.tapMax()
        waitForAmountText(fiat: "199.02", "Max didn't show both coins less the fee in fiat")
        waitForSecondaryAmount(containing: "0.00199023", "the Max amount didn't show in btc under the fiat field")

        app.tapNext()
        app.swipeToSend()

        let send = try node.waitForBroadcasts(count: 1)[0]
        assertDrainsBothCoins(send, sending: 199_023)
    }

    /// With the wallet in sats, the confirm screen shows the amount, fee and total in sats, and
    /// the fee it shows is the one the broadcast pays
    func testConfirmScreenShowsTheFeeAndTotalInSats() throws {
        try launchFundedWallet()
        useSatsOnWalletScreen()

        app.startSend(to: XCUIApplication.externalAddress)
        waitForLabel(unitSelector, equalTo: "sats", timeout: 10)
        typeAmount("50000")
        app.tapNext()

        let amount = app.element("sendFlowConfirm.amount")
        XCTAssertTrue(amount.waitForExistence(timeout: 30), "the send didn't reach the confirm screen")
        XCTAssertEqual(amount.label.replacingOccurrences(of: ",", with: ""), "50000")
        waitUntil(
            app.buttons["sendFlowConfirm.unit"],
            matches: NSPredicate(format: "label BEGINSWITH %@", "sats"),
            timeout: 10,
            "the confirm screen's unit isn't sats"
        )

        let fee = try XCTUnwrap(confirmSats("sendFlowConfirm.fee"), "the fee isn't shown in sats")
        XCTAssertGreaterThan(fee, 0)
        XCTAssertEqual(confirmSats("sendFlowConfirm.receive"), 50000, "they'll receive isn't 50,000 sats")
        XCTAssertEqual(confirmSats("sendFlowConfirm.total"), 50000 + fee, "the total isn't 50,000 sats plus the fee")

        app.swipeToSend()

        let send = try node.waitForBroadcasts(count: 1)[0]
        XCTAssertEqual(sentToExternal(send), 50000, "50000 typed in sats must send 50,000 sats")

        // each funded coin holds 100,000 sats, so what the inputs hold beyond the outputs is the fee
        let outputs = send.outputs.reduce(0) { $0 + $1.sats }
        XCTAssertEqual(UInt64(send.inputs.count) * 100_000 - outputs, fee, "the broadcast doesn't pay the fee the confirm screen showed")
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

    /// One tap on the balance moves it from BTC to fiat, which Send then types in
    private func useFiatOnWalletScreen() {
        let balance = app.element("selectedWallet.balance")
        XCTAssertTrue(balance.waitForExistence(timeout: 15))
        tapBalance(balance, until: NSPredicate(format: "label CONTAINS %@", "$"), "the balance never showed fiat")
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

    /// Send opened at a requested amount checks that amount, with alerts, 700 ms after it shows,
    /// and alerts that the fee is still loading when the fee hasn't been computed by then, though
    /// it lands a moment later; the alert covers Next, so it is dismissed to reach the send
    private func dismissFeeStillLoadingAlert() {
        let alert = app.alerts["Fee Estimate Still Loading"]
        guard alert.waitForExistence(timeout: 5) else { return }

        alert.buttons["OK"].tap()
        XCTAssertTrue(alert.waitForNonExistence(timeout: 5), "the fee alert didn't close")
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
        amountField.typeTextKeyByKey(amount)
        waitForAmountText(amount, "the amount field didn't take \(amount)")
    }

    /// Deletes what the amount field shows, then types the new amount
    private func replaceAmount(with amount: String) {
        XCTAssertTrue(amountField.waitForExistence(timeout: 10))
        amountField.tap()

        let current = amountField.value as? String ?? ""
        let deletes = String(repeating: XCUIKeyboardKey.delete.rawValue, count: current.count + 1)
        amountField.typeTextKeyByKey(deletes + amount)
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

    /// Max drains every spendable coin to the address, so the broadcast spends both funded coins
    /// and pays the address alone, with no change
    private func assertDrainsBothCoins(_ broadcast: FakeNodeControl.Broadcast, sending sats: UInt64) {
        XCTAssertEqual(broadcast.inputs.count, 2, "Max must spend both funded coins")
        XCTAssertEqual(broadcast.outputs.count, 1, "Max must leave no change, got \(broadcast.outputs)")
        XCTAssertEqual(sentToExternal(broadcast), sats, "Max must send the \(sats) sats it showed")
    }

    // MARK: - Confirm screen

    /// Reads a confirm screen amount shown in sats, such as `977 sats`, or nil when it shows
    /// another unit
    private func confirmSats(_ identifier: String) -> UInt64? {
        let element = app.element(identifier)
        XCTAssertTrue(element.waitForExistence(timeout: 10), "the confirm screen has no \(identifier)")

        let parts = element.label.replacingOccurrences(of: ",", with: "").split(separator: " ")
        guard parts.count == 2, parts[1] == "sats" else {
            XCTFail("\(identifier) isn't in sats, got '\(element.label)'")
            return nil
        }

        return UInt64(parts[0])
    }
}
