import XCTest

/// Steps the send flow's full launch tests share, against cove-fake-node
extension XCUIApplication {
    var sendAmountField: XCUIElement {
        textFields["sendFlow.amount"]
    }

    var sendUnitSelector: XCUIElement {
        buttons["sendFlow.unit"]
    }

    /// Funds the test wallet with two 100,000 sat coins, then launches the app and imports it
    func launchFundedWallet(node: FakeNodeControl) throws {
        try node.requireHealthy()
        try node.reset()
        _ = try node.fundWallet(mnemonic: XCUIApplication.testMnemonic, sats: [100_000, 100_000])
        try node.mine()

        launchEnvironment["COVE_UI_TEST_ESPLORA_URL"] = UITestHostPorts.esploraUrl
        launch(expecting: staticTexts["Welcome to Cove"])
        importTestMnemonic()
        assertBalance(sats: 200_000, timeout: 120)
    }

    /// Picks SATS from the wallet balance's context menu
    func useSatsOnWalletScreen(waitForBalance: Bool = true) {
        let balance = element("selectedWallet.balance")
        XCTAssertTrue(balance.waitForExistence(timeout: 15))
        balance.press(forDuration: 1.2)

        let sats = buttons["SATS"]
        XCTAssertTrue(sats.waitForExistence(timeout: 5), "the balance's context menu has no SATS item")
        sats.tap()

        guard waitForBalance else { return }

        // a loaded simulator has taken over 10 seconds to redraw the balance after the menu closes
        waitUntil(balance, matches: NSPredicate(format: "label ENDSWITH %@", " SATS"), timeout: 20, "the balance never showed sats")
    }

    /// Scans a BIP21 request for the amount with Add Wallet's QR scanner, the app-wide scanner,
    /// and sends to it from the alert, which opens Send at the requested amount
    func openSendFromScannedPaymentRequest(btc amount: String, qr: QrScript) {
        tapButton(named: "app.sidebar.open")
        tapButton(named: "Add Wallet")
        tapButton(named: "More wallet import options", until: buttons["QR"])
        buttons["QR"].tap()

        let request = "bitcoin:\(XCUIApplication.externalAddress)?amount=\(amount)"
        XCTAssertNoThrow(try qr.show([request]))

        let sendTo = buttons["Send To Address"]
        XCTAssertTrue(sendTo.waitForExistence(timeout: 20), "the scanned request never offered Send To Address")
        sendTo.tap()
    }

    func typeSendAmount(_ amount: String) {
        XCTAssertTrue(sendAmountField.waitForExistence(timeout: 10))
        sendAmountField.tap()
        sendAmountField.typeTextKeyByKey(amount)
        waitForSendAmountText(amount, "the amount field didn't take \(amount)")
    }

    /// Waits for the amount field to show the text, ignoring the grouping commas sats get
    func waitForSendAmountText(_ text: String, _ message: String) {
        waitForSendAmount(message) { $0.replacingOccurrences(of: ",", with: "") == text }
    }

    func waitForSendAmount(_ message: String, _ matches: (String) -> Bool) {
        let deadline = Date().addingTimeInterval(10)
        while Date() < deadline {
            if let value = sendAmountField.value as? String, matches(value) { return }

            Thread.sleep(forTimeInterval: 0.25)
        }

        XCTFail("\(message), got '\(sendAmountField.value as? String ?? "")'")
    }

    func selectSendUnit(_ unit: String) {
        XCTAssertTrue(sendUnitSelector.waitForExistence(timeout: 10))
        sendUnitSelector.tap()

        let option = buttons["sendFlow.unit.\(unit)"]
        XCTAssertTrue(option.waitForExistence(timeout: 5), "the unit menu has no \(unit) option")
        option.tap()

        waitForLabel(sendUnitSelector, equalTo: unit, timeout: 10)
    }
}

extension FakeNodeControl.Broadcast {
    /// What the broadcast pays `XCUIApplication.externalAddress`
    var sentToExternal: UInt64? {
        outputs.first { $0.address == XCUIApplication.externalAddress }?.sats
    }
}
