import XCTest

/// Bumps the fee of pending sends in the real app against cove-fake-node, then checks the node
/// holds the replacement
///
/// Needs the fake node and the hardware signer host on the ports `COVE_UI_TEST_PORT_BASE` names,
/// which `xtask ios-ui` (`just ios-ui-background`) starts for every run, and a freshly erased
/// simulator, which xtask also provides. With `COVE_UI_TEST_SCREENSHOT_DIR` set in the test
/// runner's environment (`TEST_RUNNER_COVE_UI_TEST_SCREENSHOT_DIR` for xcodebuild), each new
/// screen is saved there in light and dark
final class FeeBumpFullLaunchUITests: XCTestCase {
    private static let esploraUrl = UITestHostPorts.esploraUrl
    private static let feeUrl = "\(esploraUrl)/api/v1/fees/recommended"
    private static let fundedSats: [UInt64] = [100_000, 60000]

    /// What every test sends, small enough beside the funding that a bump never warns
    private static let sendAmountBtc = "0.00050000"

    private var app: XCUIApplication!
    private var node: FakeNodeControl!
    private var signer: HardwareSignerControl!
    private var qr: QrScript!
    private var screens: ScreenCapture!

    override func setUp() {
        super.setUp()
        continueAfterFailure = false

        app = XCUIApplication()
        node = FakeNodeControl()
        signer = HardwareSignerControl()
        qr = QrScript()
        screens = ScreenCapture(signer: signer)

        // a fresh install on Signet against the fake node, pricing fees from it too
        app.launchEnvironment["COVE_UI_TEST_ESPLORA_URL"] = Self.esploraUrl
        app.launchEnvironment["COVE_FEE_URL"] = Self.feeUrl
        qr.configure(app)
    }

    override func tearDown() {
        qr.remove()
        super.tearDown()
    }

    // MARK: - Hot wallet

    func testBumpFromDetailsReplacesTheSendAtMedium() throws {
        try fundTestWallet()
        launchAndImportHotWallet()

        let original = try sendSlowPayment()
        openDetails(of: original.txid)

        // a send that isn't late yet offers only the small button
        let bumpFee = app.buttons["transactionDetails.speedUp"]
        XCTAssertTrue(bumpFee.waitForExistence(timeout: 30), "a pending send's details should offer Speed Up")
        XCTAssertFalse(app.element("transactionDetails.delayedBanner").exists)
        screens.capture("details-pending")

        bumpFee.tap()
        assertBumpSheetSelects("medium")
        assertNote("speedUp.note", contains: "The amount and recipient stay the same.")
        screens.capture("bump-sheet")

        continueToConfirm()
        screens.capture("confirm-bump")
        swipeAndFinishFeeBump()

        let replacement = try assertReplaced(original)

        // the replacement's details strike through the fee it replaced
        openDetails(of: replacement.txid)
        expandDetails()
        XCTAssertTrue(app.element("transactionDetails.previousFee").waitForExistence(timeout: 15))
        let networkFee = app.element("transactionDetails.networkFee")
        XCTAssertTrue(networkFee.label.hasPrefix("New Network Fee"), "unexpected fee row '\(networkFee.label)'")
        XCTAssertTrue(app.element("transactionDetails.speedUpNote").exists)
        XCTAssertFalse(app.element("transactionDetails.feeHistory").exists, "one bump has no fee history to show")
        screens.capture("details-bumped")
    }

    func testMaxSendBumpTakesTheFeeFromTheAmountSent() throws {
        try fundTestWallet()
        launchAndImportHotWallet()
        app.assertBalance(sats: Self.fundedSats.reduce(0, +), timeout: 120)

        app.startSend(to: XCUIApplication.externalAddress)
        app.tapMax()
        app.tapNext()
        app.swipeToSend()

        let original = try node.waitForBroadcasts(count: 1)[0]
        XCTAssertEqual(original.outputs.count, 1, "a Max send has no change")
        openBumpSheetFromMenu(of: original.txid)

        // with no change to pay from, the recipient gets less by the extra fee
        assertBumpSheetSelects("medium")
        assertNote("speedUp.note", contains: "The recipient receives ")
        assertNote("speedUp.note", contains: " sats less.")
        screens.capture("bump-sheet-max-send")

        continueToConfirm()
        XCTAssertTrue(app.element("sendFlowConfirm.recipientReceivesLess").waitForExistence(timeout: 10))
        screens.capture("confirm-bump-max-send")
        swipeAndFinishFeeBump()

        let replacement = try assertReplaced(original)
        XCTAssertEqual(replacement.outputs.count, 1, "the bump adds no change")
        XCTAssertLessThan(replacement.outputs[0].sats, original.outputs[0].sats)
    }

    func testDelayedPopupBumpsInOneTap() throws {
        // waiting after 20 seconds and delayed after 40, since a Slow send pays under Medium
        app.launchEnvironment["COVE_UI_TEST_PENDING_SEND_DELAY_SECS"] = "40"

        try fundTestWallet()
        launchAndImportHotWallet()

        let original = try sendSlowPayment()
        let row = transactionRow(original.txid)
        waitUntil(row, matches: NSPredicate(format: "value == 'waiting'"), timeout: 60, "the send never started waiting")
        screens.capture("home-waiting")

        waitUntil(row, matches: NSPredicate(format: "value == 'delayed'"), timeout: 60, "the send never counted as delayed")
        screens.capture("home-delayed")

        // the delayed send's details lead with the banner
        openDetails(of: original.txid)
        XCTAssertTrue(app.element("transactionDetails.delayedBanner").waitForExistence(timeout: 30))
        XCTAssertTrue(app.buttons["transactionDetails.delayedBanner.speedUp"].exists)
        XCTAssertFalse(app.buttons["transactionDetails.speedUp"].exists)
        screens.capture("details-delayed")
        app.goBack()

        // the icon exists while the pop back to the wallet still animates, so wait for it to settle;
        // a coordinate tap covers the hit point quirk rows have, and the popup check below
        // confirms the tap landed
        let delayedIcon = row.buttons["transactionRow.delayedIcon"]
        XCTAssertTrue(delayedIcon.waitForExistence(timeout: 15))
        let settled = XCTNSPredicateExpectation(predicate: NSPredicate(format: "isHittable == true"), object: delayedIcon)
        _ = XCTWaiter.wait(for: [settled], timeout: 10)
        center(of: delayedIcon).tap()

        XCTAssertTrue(app.element("speedUpPopup").waitForExistence(timeout: 15), "the delayed icon should open the popup")
        XCTAssertTrue(app.staticTexts["Taking longer than usual"].exists)
        XCTAssertTrue(app.element("speedUpPopup.suggestedFee").waitForExistence(timeout: 30), "the popup never priced the bump")
        screens.capture("delayed-popup")

        app.buttons["speedUpPopup.speedUp"].tap()
        assertConfirmInBumpMode()
        swipeAndFinishFeeBump()

        try assertReplaced(original)
    }

    func testCustomRateBumpThenSecondBumpShowsFeeHistory() throws {
        try fundTestWallet()
        launchAndImportHotWallet()

        // the first bump picks its own rate from the long-press menu
        let original = try sendSlowPayment()
        openBumpSheetFromMenu(of: original.txid)

        let custom = app.element("speedUp.option.custom")
        XCTAssertTrue(custom.waitForExistence(timeout: 30))
        custom.tap()

        XCTAssertTrue(app.element("speedUp.customSheet").waitForExistence(timeout: 10))
        let slider = app.sliders["customFeeRate.slider"]
        XCTAssertTrue(slider.waitForExistence(timeout: 10))
        slider.adjust(toNormalizedSliderPosition: 0.15)
        waitUntil(app.element("customFeeRate.total"), matches: NSPredicate(format: "exists == true"), timeout: 15, "the custom rate was never priced")
        screens.capture("custom-rate")

        app.buttons["customFeeRate.done"].tap()
        XCTAssertTrue(app.element("speedUp.customSheet").waitForNonExistence(timeout: 10))
        waitUntil(custom, matches: NSPredicate(format: "selected == true"), timeout: 15, "the custom rate wasn't selected")

        continueToConfirm()
        swipeAndFinishFeeBump()
        let firstBump = try assertReplaced(original)

        // the second bump takes the sheet's default
        openBumpSheetFromMenu(of: firstBump.txid)
        XCTAssertTrue(app.buttons["speedUp.continue"].waitForExistence(timeout: 30))
        continueToConfirm()
        swipeAndFinishFeeBump()
        let secondBump = try assertReplaced(firstBump)

        // the fee history lists the original, the first bump and the current fee
        openDetails(of: secondBump.txid)
        expandDetails()

        let history = app.buttons["transactionDetails.feeHistory"]
        XCTAssertTrue(history.waitForExistence(timeout: 15), "two bumps should show the fee history")
        XCTAssertEqual(history.value as? String, "collapsed")
        history.tap()

        // a query on the expanded details takes seconds, so a short wait gets only one look
        waitUntil(history, matches: NSPredicate(format: "value == 'expanded'"), timeout: 15, "the fee history didn't open")

        for index in 0 ..< 3 {
            XCTAssertTrue(app.element("transactionDetails.feeHistory.\(index)").waitForExistence(timeout: 15), "fee history is missing entry \(index)")
        }

        XCTAssertFalse(app.element("transactionDetails.feeHistory.3").exists)
        screens.capture("details-fee-history")
    }

    func testReceivedPendingTransactionOffersNoBump() throws {
        try fundTestWallet()
        let received = try node.fundWallet(mnemonic: XCUIApplication.testMnemonic, sats: [25000], firstIndex: 2, confirm: false)[0]
        launchAndImportHotWallet()

        let row = transactionRow(received.txid)
        XCTAssertTrue(row.waitForExistence(timeout: 120), "the pending receive never showed")
        center(of: row).press(forDuration: 1.2)

        XCTAssertTrue(app.buttons["Copy Transaction ID"].waitForExistence(timeout: 5))
        XCTAssertFalse(bumpMenuItem.exists, "a received payment can't be bumped")
        screens.capture("menu-received")

        app.buttons["View Transaction Details"].tap()
        XCTAssertTrue(app.buttons["transactionDetails.showDetails"].waitForExistence(timeout: 30))
        XCTAssertFalse(app.buttons["transactionDetails.speedUp"].exists)
        XCTAssertFalse(app.element("transactionDetails.delayedBanner").exists)
    }

    // MARK: - Hardware wallet

    func testHardwareWalletBumpSignsTheReplacement() throws {
        try signer.requireHealthy()
        try fundTestWallet()
        app.launch(expecting: app.staticTexts["Welcome to Cove"])
        app.importWatchOnlyHardwareWallet(signer: signer, qr: qr)
        app.assertBalance(sats: Self.fundedSats.reduce(0, +), timeout: 120)

        startSlowPayment()
        try signOnHardware()
        app.swipeToSend()
        let original = try node.waitForBroadcasts(count: 1)[0]

        // bumping exports the replacement for the signer, which signs it like any draft
        openDetails(of: original.txid)
        let bumpFee = app.buttons["transactionDetails.speedUp"]
        XCTAssertTrue(bumpFee.waitForExistence(timeout: 30))
        bumpFee.tap()
        assertBumpSheetSelects("medium")
        app.buttons["speedUp.continue"].tap()

        XCTAssertTrue(app.buttons["Export Transaction"].waitForExistence(timeout: 30), "a hardware bump should export the replacement")
        try signOnHardware()
        assertConfirmInBumpMode()
        swipeAndFinishFeeBump()

        try assertReplaced(original)
    }

    // MARK: - Fixture

    /// Funds external indexes 0 and 1 of the test mnemonic, each its own confirmed UTXO
    private func fundTestWallet() throws {
        try node.requireHealthy()
        try node.reset()

        let outputs = try node.fundWallet(mnemonic: XCUIApplication.testMnemonic, sats: Self.fundedSats)
        try node.mine()
        XCTAssertEqual(outputs.map(\.sats), Self.fundedSats)
    }

    private func launchAndImportHotWallet() {
        app.launch(expecting: app.staticTexts["Welcome to Cove"])
        app.importTestMnemonic()
    }

    // MARK: - Sending

    /// Sends the test amount at Slow, which pays under Medium so the send can be bumped and
    /// counts as delayed once it is late, and returns its broadcast
    private func sendSlowPayment() throws -> FakeNodeControl.Broadcast {
        app.assertBalance(sats: Self.fundedSats.reduce(0, +), timeout: 120)
        startSlowPayment()
        app.swipeToSend()

        let original = try node.waitForBroadcasts(count: 1)[0]
        XCTAssertTrue(transactionRow(original.txid).waitForExistence(timeout: 60), "the send never showed in the wallet")
        return original
    }

    /// Fills the send screen with the test amount at Slow and taps Next
    private func startSlowPayment() {
        app.startSend(to: XCUIApplication.externalAddress)

        let amount = app.textFields["sendFlow.amount"]
        XCTAssertTrue(amount.waitForExistence(timeout: 10))
        amount.tap()
        amount.typeText(Self.sendAmountBtc)
        if app.keyboards.count > 0 {
            app.buttons["Done"].firstMatch.tap()
        }

        let changeSpeed = app.buttons["Change speed"]
        XCTAssertTrue(changeSpeed.waitForExistence(timeout: 30))
        changeSpeed.tap()

        let slow = app.element("sendFlow.feeOption.slow")
        XCTAssertTrue(slow.waitForExistence(timeout: 15))
        slow.tap()
        XCTAssertTrue(slow.waitForNonExistence(timeout: 10), "picking a speed should close the fee sheet")

        app.tapNext()
    }

    /// Shares the open draft with the signer and scans its signature back in
    private func signOnHardware() throws {
        let unsigned = app.exportUnsignedPsbt(showing: .ur, signer: signer)
        let signed = try signer.sign(unsigned, minParts: 1)

        app.openSignatureScanner()
        try qr.show(signed.ur)
    }

    // MARK: - Fee bump

    private var bumpMenuItem: XCUIElement {
        app.buttons.matching(NSPredicate(format: "label BEGINSWITH %@", "Speed Up")).firstMatch
    }

    private func openBumpSheetFromMenu(of txid: String) {
        let row = transactionRow(txid)
        XCTAssertTrue(row.waitForExistence(timeout: 60), "missing transaction row \(txid)")
        center(of: row).press(forDuration: 1.2)

        XCTAssertTrue(bumpMenuItem.waitForExistence(timeout: 5), "the long-press menu should offer Speed Up")
        XCTAssertTrue(app.buttons["Copy Address"].exists)
        XCTAssertTrue(app.buttons["Copy Transaction ID"].exists)
        XCTAssertTrue(app.buttons["View Transaction Details"].exists)
        screens.capture("menu-pending-send")

        bumpMenuItem.tap()
        XCTAssertTrue(app.element("speedUp.sheet").waitForExistence(timeout: 15))
    }

    /// Waits for the bump sheet's rates and checks which one it starts on
    private func assertBumpSheetSelects(_ speed: String) {
        let option = app.element("speedUp.option.\(speed)")
        XCTAssertTrue(option.waitForExistence(timeout: 30), "the bump sheet never showed its rates")
        waitUntil(option, matches: NSPredicate(format: "selected == true"), timeout: 10, "the bump sheet should start on \(speed)")
        XCTAssertTrue(app.element("speedUp.currentFee").exists)
        XCTAssertTrue(app.element("speedUp.newFee").exists)
        XCTAssertTrue(app.element("speedUp.additionalCost").exists)
    }

    private func assertNote(_ identifier: String, contains text: String) {
        let note = app.staticTexts[identifier]
        XCTAssertTrue(note.waitForExistence(timeout: 10), "missing \(identifier)")
        XCTAssertTrue(note.label.contains(text), "\(identifier) reads '\(note.label)'")
    }

    private func continueToConfirm() {
        let bump = app.buttons["speedUp.continue"]
        XCTAssertTrue(bump.waitForExistence(timeout: 15))
        XCTAssertTrue(bump.label.hasPrefix("Speed Up for "), "unexpected continue label '\(bump.label)'")
        bump.tap()

        assertConfirmInBumpMode()
    }

    private func assertConfirmInBumpMode() {
        let title = app.element("sendFlowConfirm.title")
        XCTAssertTrue(title.waitForExistence(timeout: 30), "the bump didn't reach the confirm screen")
        waitForLabel(title, equalTo: "Speed Up", timeout: 15)
        XCTAssertTrue(app.element("sendFlowConfirm.previousFee").waitForExistence(timeout: 10))
        XCTAssertTrue(app.staticTexts["New Network Fee"].exists)
        XCTAssertTrue(app.staticTexts["Swipe to confirm new fee"].exists)
    }

    /// Swipes the replacement out, then leaves the Sped Up screen for the wallet
    private func swipeAndFinishFeeBump() {
        app.swipeConfirmHandle()

        XCTAssertTrue(app.element("spedUp.title").waitForExistence(timeout: 30), "the bump didn't reach the Sped Up screen")
        XCTAssertFalse(app.alerts["Sent!"].exists, "a bump shows Sped Up instead of the Sent alert")
        XCTAssertTrue(app.element("spedUp.fee").exists)
        screens.capture("sped-up")

        app.buttons["spedUp.done"].tap()
        XCTAssertTrue(app.element("selectedWallet.balance").waitForExistence(timeout: 15))
    }

    /// Checks a new broadcast took the place of `replaced`, spending its inputs, and that the
    /// node now holds it for them, returning it
    @discardableResult
    private func assertReplaced(_ replaced: FakeNodeControl.Broadcast) throws -> FakeNodeControl.Broadcast {
        let replacement = try waitForReplacement(of: replaced)
        XCTAssertEqual(Set(replacement.inputs), Set(replaced.inputs), "a bump must spend the coins it replaces")
        XCTAssertTrue(replacement.outputs.contains { $0.address == XCUIApplication.externalAddress })

        for input in replaced.inputs {
            XCTAssertEqual(try node.outspend(outpoint: input).spendingTxid, replacement.txid, "the node should hold the replacement")
        }

        XCTAssertTrue(transactionRow(replacement.txid).waitForExistence(timeout: 60), "the replacement never showed in the wallet")
        return replacement
    }

    /// Polls the node for the broadcast that spends what `replaced` spent under another txid
    ///
    /// The node drops a replaced transaction from its broadcast log, so the log holds the latest
    /// version of a payment only
    private func waitForReplacement(of replaced: FakeNodeControl.Broadcast) throws -> FakeNodeControl.Broadcast {
        let deadline = Date().addingTimeInterval(15)
        while true {
            let replacement = try node.broadcasts().first { broadcast in
                broadcast.txid != replaced.txid && !Set(broadcast.inputs).isDisjoint(with: replaced.inputs)
            }

            if let replacement { return replacement }
            guard Date() < deadline else {
                throw try ControlRequestError(description: "nothing replaced \(replaced.txid): \(node.broadcasts())")
            }

            Thread.sleep(forTimeInterval: 0.5)
        }
    }

    // MARK: - Wallet screen

    private func transactionRow(_ txid: String) -> XCUIElement {
        app.element("transactionRow.\(txid)")
    }

    /// XCUITest at times finds no hit point on a row, which only contains other elements, and
    /// fails it as not hittable though nothing covers it, so rows take taps by coordinate; the
    /// step after each tap checks what it opened
    private func center(of row: XCUIElement) -> XCUICoordinate {
        row.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.5))
    }

    private func openDetails(of txid: String) {
        let row = transactionRow(txid)
        XCTAssertTrue(row.waitForExistence(timeout: 60), "missing transaction row \(txid)")
        center(of: row).tap()

        XCTAssertTrue(app.buttons["transactionDetails.showDetails"].waitForExistence(timeout: 30), "the details never opened")
    }

    /// Opens the details' fee rows, which stay open per wallet once shown
    private func expandDetails() {
        let showDetails = app.buttons["transactionDetails.showDetails"]
        XCTAssertTrue(showDetails.waitForExistence(timeout: 15))
        if showDetails.label == "Show Details" {
            showDetails.tap()
        }

        XCTAssertTrue(app.element("transactionDetails.networkFee").waitForExistence(timeout: 15))
    }
}

/// Saves what the screen shows in light and dark, when a run asks for screenshots
private struct ScreenCapture {
    let signer: HardwareSignerControl

    private let directory = ProcessInfo.processInfo.environment["COVE_UI_TEST_SCREENSHOT_DIR"].map(URL.init(fileURLWithPath:))

    func capture(_ name: String) {
        guard let directory else { return }

        do {
            try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
            try save(name, in: directory, dark: false)

            try signer.setSimulatorAppearance(dark: true)
            defer { try? signer.setSimulatorAppearance(dark: false) }
            try save(name, in: directory, dark: true)
        } catch {
            XCTFail("couldn't save the \(name) screenshot: \(error)")
        }
    }

    private func save(_ name: String, in directory: URL, dark: Bool) throws {
        // the app redraws a moment after the appearance changes
        Thread.sleep(forTimeInterval: 1.5)

        let file = directory.appendingPathComponent("\(name)-\(dark ? "dark" : "light").png")
        try XCUIScreen.main.screenshot().pngRepresentation.write(to: file)
    }
}
