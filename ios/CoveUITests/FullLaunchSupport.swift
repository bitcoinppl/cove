import XCTest

/// Launching, onboarding, querying and waiting helpers the full-launch UI tests share
extension XCUIApplication {
    /// The test mnemonic every full-launch test funds and imports
    static let testMnemonic = (Array(repeating: "abandon", count: 11) + ["about"]).joined(separator: " ")

    /// Launches the app, relaunching once if the bootstrap watchdog fires on a cold simulator
    func launch(expecting readyElement: XCUIElement) {
        launch()

        guard waitForReadyOrStartupTimeout(readyElement) == .startupTimedOut else { return }

        terminate()
        launch()

        // xtask retries the test, or a cloud backup class's tests that didn't pass, on a clean
        // simulator when it sees this message
        if waitForReadyOrStartupTimeout(readyElement) == .startupTimedOut {
            XCTFail("App startup timed out on relaunch")
        }
    }

    private enum LaunchOutcome {
        case ready
        case startupTimedOut
        case unknown
    }

    private func waitForReadyOrStartupTimeout(_ readyElement: XCUIElement) -> LaunchOutcome {
        let startupTimedOut = staticText(containing: "App startup timed out")

        // the watchdog allows up to 20 seconds while a migration runs
        let deadline = Date().addingTimeInterval(30)
        while Date() < deadline {
            if readyElement.exists { return .ready }
            if startupTimedOut.exists { return .startupTimedOut }

            Thread.sleep(forTimeInterval: 0.5)
        }

        return .unknown
    }

    // MARK: - Onboarding

    /// Walks onboarding from the welcome screen to the choice of how the user stores bitcoin
    func startImportingExistingWallet() {
        // a tap right after launch can land while the welcome screen settles and do nothing
        let next = staticTexts.matching(NSPredicate(format: "label IN %@", ["Terms & Conditions", "Do you already have Bitcoin?"]))
        tapButton(startingWith: "Get Started", until: next.firstMatch)
        acceptTermsIfNeeded()
        tapButton(startingWith: "Yes, I have Bitcoin")

        let useAnotherWallet = button(startingWith: "Use another wallet")
        if useAnotherWallet.waitForExistence(timeout: 2) {
            useAnotherWallet.tap()
        }
    }

    /// Imports the test mnemonic as a hot wallet and lands on its wallet screen
    func importTestMnemonic() {
        startImportingExistingWallet()
        tapButton(startingWith: "Software wallet", until: button(startingWith: "Enter recovery words"))
        tapButton(startingWith: "Enter recovery words", until: button(startingWith: "12 words"))
        tapButton(startingWith: "12 words", until: navigationBars["Import Wallet"])
        enterTestMnemonicAndImport()

        XCTAssertTrue(staticTexts["Protect this wallet with Cloud Backup?"].waitForExistence(timeout: 20))
        buttons["Not Now"].tap()
        acceptTermsIfNeeded()

        // only native segwit is funded, but keep it if other address types are offered anyway
        let keepCurrent = button(startingWith: "Keep Current")
        if keepCurrent.waitForExistence(timeout: 5) {
            keepCurrent.tap()
        }

        XCTAssertTrue(element("selectedWallet.balance").waitForExistence(timeout: 30))
    }

    /// Types the test mnemonic into the open Import Wallet screen and taps Import
    func enterTestMnemonicAndImport() {
        for (index, word) in Self.testMnemonic.split(separator: " ").enumerated() {
            let field = textFields["hotWalletImport.word.\(index + 1)"]
            XCTAssertTrue(field.waitForExistence(timeout: 10))
            field.tap()
            field.typeText(word == "about" ? "abou" : "aba")
        }

        buttons["hotWalletImport.import"].tap()
    }

    func acceptTermsIfNeeded() {
        guard staticTexts["Terms & Conditions"].waitForExistence(timeout: 3) else { return }

        [
            "onboarding.terms.check.backup",
            "onboarding.terms.check.legal",
            "onboarding.terms.check.financial",
            "onboarding.terms.check.recovery",
            "onboarding.terms.check.agreement",
        ].forEach { buttons[$0].tap() }

        buttons["onboarding.terms.agree"].tap()
    }

    /// Walks Add Wallet from the sidebar to the 12-word import, enters the test mnemonic and
    /// lands on the new wallet
    func importTestMnemonicFromAddWallet() {
        tapButton(named: "app.sidebar.open")
        tapButton(named: "Add Wallet")
        tapButton(startingWith: "On This Device", until: buttons["Import existing wallet"])
        tapButton(startingWith: "Import existing wallet", until: buttons["12 Words"])
        tapButton(startingWith: "12 Words", until: navigationBars["Import Wallet"])
        enterTestMnemonicAndImport()

        // keeps native segwit when the node shows activity on other address types too
        let keepCurrent = button(startingWith: "Keep Current")
        let balance = element("selectedWallet.balance")
        let deadline = Date().addingTimeInterval(30)
        while Date() < deadline, !keepCurrent.exists {
            if balance.exists, !keepCurrent.waitForExistence(timeout: 3) { return }

            Thread.sleep(forTimeInterval: 0.2)
        }

        XCTAssertTrue(keepCurrent.exists || balance.exists, "the import never reached the new wallet")
        if keepCurrent.exists {
            keepCurrent.tap()
            XCTAssertTrue(keepCurrent.waitForNonExistence(timeout: 10))
        }
    }

    // MARK: - Sending

    /// BIP173 test vector: the P2WPKH address of private key 1, which is not in the test wallet
    static let externalAddress = "tb1qw508d6qejxtdg4y5r3zarvary0c5xw7kxpjzsx"

    /// Opens Send from the wallet screen and enters the address
    func startSend(to address: String) {
        let send = buttons.matching(NSPredicate(format: "label == 'Send' OR label ENDSWITH ', Send'")).firstMatch
        XCTAssertTrue(send.waitForExistence(timeout: 10))
        send.tap()

        enterAddress(address)
    }

    func enterAddress(_ address: String) {
        let field = textViews["sendFlow.address"]
        XCTAssertTrue(field.waitForExistence(timeout: 15))

        // the send screen can open with the amount focused, and the keyboard's toolbar then covers
        // the address field
        if !field.isHittable, keyboards.count > 0 {
            buttons["Done"].firstMatch.tap()
        }

        waitUntil(field, matches: NSPredicate(format: "hittable == true"), timeout: 5, "the address field is covered")
        field.tap()
        field.typeText(address)
    }

    func tapMax() {
        let amount = textFields["sendFlow.amount"]
        XCTAssertTrue(amount.waitForExistence(timeout: 10))
        amount.tap()

        let max = buttons["sendFlow.max"]
        XCTAssertTrue(max.waitForExistence(timeout: 10))
        max.tap()
    }

    func tapNext() {
        // Next shows once the address is valid and fee rates have loaded; Max drops the keyboard
        // that covers it, but a typed amount leaves it up
        let next = buttons["sendFlow.next"]
        XCTAssertTrue(next.waitForExistence(timeout: 60))
        if !next.isHittable, keyboards.count > 0 {
            buttons["Done"].firstMatch.tap()
        }

        waitUntil(next, matches: NSPredicate(format: "hittable == true"), timeout: 10, "Next is covered")
        next.tap()
    }

    /// Swipes the confirm screen's handle across, accepts the Sent alert and returns to the wallet
    func swipeToSend() {
        let swipe = element("sendFlowConfirm.swipeToSend")
        XCTAssertTrue(swipe.waitForExistence(timeout: 30), "the send didn't reach the confirm screen")

        // drag the handle from the leading edge all the way across
        let frame = swipe.frame
        let start = swipe.coordinate(withNormalizedOffset: CGVector(dx: frame.height / 2 / frame.width, dy: 0.5))
        let end = swipe.coordinate(withNormalizedOffset: CGVector(dx: 0.99, dy: 0.5))
        start.press(forDuration: 0.2, thenDragTo: end, withVelocity: .slow, thenHoldForDuration: 0.2)

        let sent = alerts["Sent!"]
        XCTAssertTrue(sent.waitForExistence(timeout: 30), "the send was not broadcast")
        sent.buttons["OK"].tap()

        XCTAssertTrue(element("selectedWallet.balance").waitForExistence(timeout: 15))
    }

    // MARK: - Amounts

    /// Waits for the balance, then for the initial scan to finish
    ///
    /// The scan can publish the full balance while it still checks unused addresses, and coin control
    /// and Send refuse to open until it finishes. The wallet shows "Checking wallet history" until then
    func assertBalance(sats: UInt64, timeout: TimeInterval) {
        let balance = element("selectedWallet.balance")
        XCTAssertTrue(balance.waitForExistence(timeout: 15))
        waitForAmount(balance, sats: sats, timeout: timeout)

        let checkingHistory = staticTexts["Checking wallet history"]
        XCTAssertTrue(checkingHistory.waitForNonExistence(timeout: timeout), "the initial scan didn't finish")
    }

    // MARK: - App lock

    /// Turns on the app PIN from Settings, then returns to the selected wallet
    func enablePin(_ pin: String) {
        tapButton(named: "app.sidebar.open")
        tapButton(named: "Settings")
        XCTAssertTrue(navigationBars["Settings"].waitForExistence(timeout: 10))

        // the row is a switch wrapping the control itself, which is what takes the tap
        let pinToggle = ScreenReachability(app: self)
            .assertReachable(switches.matching(identifier: "Enable PIN"), "settings: Enable PIN", needsHit: true)
        pinToggle.switches.firstMatch.tap()
        XCTAssertTrue(staticTexts["Enter New PIN"].waitForExistence(timeout: 10))
        enterPin(pin)
        XCTAssertTrue(staticTexts["Confirm New PIN"].waitForExistence(timeout: 5))
        enterPin(pin)
        XCTAssertTrue(navigationBars["Settings"].waitForExistence(timeout: 10))
        XCTAssertEqual(pinToggle.value as? String, "1", "the PIN was not saved")

        goBack()
        XCTAssertTrue(element("selectedWallet.balance").waitForExistence(timeout: 10))
    }

    /// Enters the PIN on the prompt a secret asks for
    func verifyIdentity(_ pin: String) {
        let prompt = staticTexts["Enter Cove PIN"]
        XCTAssertTrue(prompt.waitForExistence(timeout: 10), "the PIN prompt never showed")
        enterPin(pin)
        XCTAssertTrue(prompt.waitForNonExistence(timeout: 10), "the PIN was not accepted")
    }

    func enterPin(_ pin: String) {
        for digit in pin {
            let key = buttons[String(digit)]
            XCTAssertTrue(key.waitForExistence(timeout: 5), "number pad key \(digit) is missing")
            key.tap()
        }
    }

    // MARK: - Queries

    func element(_ identifier: String) -> XCUIElement {
        descendants(matching: .any)[identifier].firstMatch
    }

    func button(startingWith labelPrefix: String) -> XCUIElement {
        buttons.matching(NSPredicate(format: "label BEGINSWITH %@", labelPrefix)).firstMatch
    }

    func staticText(containing text: String) -> XCUIElement {
        staticTexts.matching(NSPredicate(format: "label CONTAINS %@", text)).firstMatch
    }

    func tapButton(named name: String) {
        let button = buttons[name]
        XCTAssertTrue(button.waitForExistence(timeout: 10), "Timed out waiting for button '\(name)'")
        button.tap()
    }

    func tapButton(startingWith labelPrefix: String) {
        let button = button(startingWith: labelPrefix)
        XCTAssertTrue(button.waitForExistence(timeout: 10), "Timed out waiting for button starting with '\(labelPrefix)'")
        button.tap()
    }

    /// On CI a tap can land while the screen settles or a system banner passes over it and do
    /// nothing, so tap again while the button is still showing
    func tapButton(startingWith labelPrefix: String, until target: XCUIElement) {
        let button = button(startingWith: labelPrefix)
        XCTAssertTrue(button.waitForExistence(timeout: 10), "Timed out waiting for button starting with '\(labelPrefix)'")

        for _ in 0 ..< 3 {
            if button.exists, button.isHittable {
                button.tap()
            } else if button.exists, !button.frame.isEmpty {
                // a layer drawn over a button, as on Add Wallet in the cloud backup host, can make
                // XCUITest report it unhittable though a tap at its center lands
                button.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.5)).tap()
            }

            if target.waitForExistence(timeout: 5) { return }
        }

        XCTFail("Tapping the button starting with '\(labelPrefix)' never showed \(target)")
    }

    func goBack() {
        navigationBars.buttons.element(boundBy: 0).tap()
    }
}

// MARK: - Waiting

func waitForLabel(_ element: XCUIElement, equalTo label: String, timeout: TimeInterval) {
    waitUntil(element, matches: NSPredicate(format: "label == %@", label), timeout: timeout, "expected label '\(label)'")
}

func waitUntil(_ element: XCUIElement, matches predicate: NSPredicate, timeout: TimeInterval, _ message: String) {
    let expectation = XCTNSPredicateExpectation(predicate: predicate, object: element)
    let result = XCTWaiter().wait(for: [expectation], timeout: timeout)
    let current = element.exists ? "label '\(element.label)', value '\(element.value ?? "nil")'" : "missing"
    XCTAssertEqual(result, .completed, "\(message), got \(current)")
}

/// Waits for the element to show the amount, in whichever unit the wallet displays
func waitForAmount(_ element: XCUIElement, sats: UInt64, timeout: TimeInterval) {
    let deadline = Date().addingTimeInterval(timeout)
    while Date() < deadline {
        if element.exists, satsFromAmountLabel(element.label) == sats { return }

        Thread.sleep(forTimeInterval: 0.5)
    }

    let current = element.exists ? "'\(element.label)'" : "missing"
    XCTFail("expected \(sats) sats, got \(current)")
}

/// Parses an amount label such as `190,000 SATS` or `0.0019 BTC` into sats
func satsFromAmountLabel(_ label: String) -> UInt64? {
    let parts = label.replacingOccurrences(of: ",", with: "").split(separator: " ")
    guard parts.count == 2, let value = Decimal(string: String(parts[0]), locale: Locale(identifier: "en_US_POSIX")) else {
        return nil
    }

    let sats: Decimal
    switch parts[1] {
    case "SATS": sats = value
    case "BTC": sats = value * 100_000_000
    default: return nil
    }

    let whole = NSDecimalNumber(decimal: sats).uint64Value
    return Decimal(whole) == sats ? whole : nil
}
