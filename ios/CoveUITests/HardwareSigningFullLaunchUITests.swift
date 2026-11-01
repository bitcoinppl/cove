import XCTest

/// Signs sends from hardware wallets in the real app: animated UR and BBQr signatures over the QR
/// seam, signed PSBTs that match no draft, and TAPSIGNER signing over the NFC seam
///
/// Debug builds read QR frames from the file `COVE_UI_TEST_QR_SCRIPT` names instead of the camera,
/// and send TAPSIGNER commands to the card emulator at `COVE_UI_TEST_TAPSIGNER_BRIDGE` instead of an
/// NFC tag; docs/testing.md describes both seams. Needs the fake node and the hardware signer host,
/// which `xtask ios-ui` (`just ios-ui-background`) starts for every run, and a freshly erased
/// simulator, which xtask also provides
final class HardwareSigningFullLaunchUITests: XCTestCase {
    private static let esploraUrl = "http://127.0.0.1:39001"
    private static let feeUrl = "\(esploraUrl)/api/v1/fees/recommended"
    private static let fundedSats: [UInt64] = [100_000, 60000]

    /// Enough frames that the scanner shows progress before it completes
    private static let animatedParts = 5

    private var app: XCUIApplication!
    private var node: FakeNodeControl!
    private var signer: HardwareSignerControl!
    private var qr: QrScript!

    override func setUp() {
        super.setUp()
        continueAfterFailure = false

        app = XCUIApplication()
        node = FakeNodeControl()
        signer = HardwareSignerControl()
        qr = QrScript()

        // a fresh install on Signet against the fake node, pricing fees from it too
        app.launchEnvironment["COVE_UI_TEST_ESPLORA_URL"] = Self.esploraUrl
        app.launchEnvironment["COVE_FEE_URL"] = Self.feeUrl
        qr.configure(app)
    }

    override func tearDown() {
        qr.remove()
        super.tearDown()
    }

    // MARK: - QR signing

    func testAnimatedUrSignatureReachesConfirmationAndBroadcasts() throws {
        try signWithAnimatedQrAndBroadcast(.ur)
    }

    func testAnimatedBbqrSignatureReachesConfirmationAndBroadcasts() throws {
        try signWithAnimatedQrAndBroadcast(.bbqr)
    }

    private func signWithAnimatedQrAndBroadcast(_ format: QrFormat) throws {
        let funded = try fundTestWallet()
        importWatchOnlyWallet()
        app.assertBalance(sats: Self.fundedSats.reduce(0, +), timeout: 120)

        app.startSend(to: XCUIApplication.externalAddress)
        app.tapMax()
        app.tapNext()

        let unsigned = exportUnsignedPsbt(showing: format)
        let signed = try signer.sign(unsigned, minParts: Self.animatedParts)
        let frames = signed.frames(format)
        XCTAssertGreaterThan(frames.count, 1, "the signature must be an animated QR")

        // the camera can catch an animated QR at any frame, so feed them out of order
        openSignatureScanner()
        try qr.show(Array(frames.reversed()))

        assertConfirmationScreen()
        assertAdvancedDetails()
        app.swipeToSend()

        let broadcast = try node.waitForBroadcasts(count: 1)[0]
        XCTAssertEqual(Set(broadcast.inputs), Set(funded.map(\.outpoint)), "a Max send spends every UTXO")
        XCTAssertEqual(broadcast.outputs.map(\.address), [XCUIApplication.externalAddress])
    }

    func testPartialBbqrFramesShowProgressUntilTheRestArrive() throws {
        _ = try fundTestWallet()
        importWatchOnlyWallet()
        app.assertBalance(sats: Self.fundedSats.reduce(0, +), timeout: 120)

        app.startSend(to: XCUIApplication.externalAddress)
        app.tapMax()
        app.tapNext()

        let signed = try signer.sign(exportUnsignedPsbt(showing: .bbqr), minParts: Self.animatedParts)
        let frames = signed.bbqr
        let total = frames.count
        XCTAssertGreaterThanOrEqual(total, 3)

        // two frames in, the scanner shows how far it got and stays open
        openSignatureScanner()
        try qr.show(Array(frames.prefix(2)))
        XCTAssertTrue(app.staticTexts["Scanned 2 of \(total)"].waitForExistence(timeout: 15))
        XCTAssertTrue(app.staticTexts["\(total - 2) parts left"].exists)
        XCTAssertFalse(app.element("sendFlowConfirm.swipeToSend").exists)

        // a repeated frame doesn't count twice
        try qr.show([frames[1]])
        qr.waitUntilScanned(frameCount: 1)
        XCTAssertTrue(app.staticTexts["Scanned 2 of \(total)"].exists)

        try qr.show(Array(frames.dropFirst(2)))
        assertConfirmationScreen()
    }

    // MARK: - QR error paths

    func testSignaturesThatMatchNoDraftAreRejected() throws {
        _ = try fundTestWallet()
        importWatchOnlyWallet()
        app.assertBalance(sats: Self.fundedSats.reduce(0, +), timeout: 120)

        // sign a draft, then delete it
        app.startSend(to: XCUIApplication.externalAddress)
        app.tapMax()
        app.tapNext()
        let deleted = try signer.sign(exportUnsignedPsbt(showing: .ur), minParts: 1)
        deleteDraft()

        // a different draft, so the deleted one's signature can't match it
        startDraftOfHalfTheBalance()

        openSignatureScanner()
        try qr.show(deleted.ur)
        assertNoUnsignedTransactionAlert()

        openSignatureScanner()
        try qr.show(signer.unrelatedSignedPsbt(network: "signet", minParts: 3).ur)
        assertNoUnsignedTransactionAlert()

        // nothing checks a PSBT's network, so a mainnet one is just another unknown transaction
        openSignatureScanner()
        try qr.show(signer.unrelatedSignedPsbt(network: "bitcoin", minParts: 3).bbqr)
        assertNoUnsignedTransactionAlert()

        XCTAssertTrue(app.buttons["Import Signature"].exists, "the draft stays open after each rejection")
    }

    // MARK: - TAPSIGNER

    func testTapSignerImportRetriesAfterWrongPinAndDroppedSession() throws {
        try signer.startTapSigner()
        app.launchEnvironment["COVE_UI_TEST_TAPSIGNER_BRIDGE"] = HardwareSignerControl.tapSignerBridge
        app.launch(expecting: app.staticTexts["Welcome to Cove"])
        app.importTestMnemonic()

        startTapSignerImport()

        enterCvc("000000")
        tryAgainAfterWrongPin()

        // the card leaves the field partway through the import
        try signer.dropNextTapSignerSession(afterExchanges: 1)
        enterCvc(HardwareSignerControl.tapSignerCvc)
        retryInterruptedImport()

        finishTapSignerImport()
    }

    func testTapSignerSignsAfterWrongPinAndDroppedSession() throws {
        try node.requireHealthy()
        try node.reset()
        try signer.startTapSigner()

        app.launchEnvironment["COVE_UI_TEST_TAPSIGNER_BRIDGE"] = HardwareSignerControl.tapSignerBridge
        app.launch(expecting: app.staticTexts["Welcome to Cove"])
        app.importTestMnemonic()

        startTapSignerImport()
        enterCvc(HardwareSignerControl.tapSignerCvc)
        finishTapSignerImport()

        let funded = try node.fund(address: receiveAddress(), sats: [100_000])
        refreshUntilBalance(sats: 100_000)

        app.startSend(to: XCUIApplication.externalAddress)
        app.tapMax()
        app.tapNext()

        app.tapButton(named: "Sign using TAPSIGNER")
        enterCvc("000000")

        // rust-cktap rejects every BIP84 PSBT before it contacts the card, so even the wrong CVC
        // ends in the generic failure instead of Wrong PIN. Only that outcome is expected to fail;
        // once signing reaches the card, the whole flow runs
        let wrongPin = app.alerts["Wrong PIN"]
        let signingFailed = app.alerts["Signing Failed!"]
        let deadline = Date().addingTimeInterval(30)
        while Date() < deadline, !wrongPin.exists, !signingFailed.exists {
            Thread.sleep(forTimeInterval: 0.25)
        }

        if signingFailed.exists {
            XCTExpectFailure("TAPSIGNER signing fails with Invalid path, https://github.com/bitcoinppl/cove/issues/965") {
                XCTFail("the wrong CVC ended in Signing Failed! instead of Wrong PIN")
            }
            return
        }

        tryAgainAfterWrongPin()

        try signer.dropNextTapSignerSession(afterExchanges: 1)
        enterCvc(HardwareSignerControl.tapSignerCvc)
        retryInterruptedSigning()

        assertConfirmationScreen()
        app.swipeToSend()

        let broadcast = try node.waitForBroadcasts(count: 1)[0]
        XCTAssertEqual(broadcast.inputs, funded.map(\.outpoint))
        XCTAssertEqual(broadcast.outputs.map(\.address), [XCUIApplication.externalAddress])
    }

    // MARK: - Fixture

    /// Funds external indexes 0 and 1 of the test mnemonic, each its own UTXO
    private func fundTestWallet() throws -> [FakeNodeControl.FundedOutput] {
        try node.requireHealthy()
        try signer.requireHealthy()
        try node.reset()

        let outputs = try node.fundWallet(mnemonic: XCUIApplication.testMnemonic, sats: Self.fundedSats)
        try node.mine()

        XCTAssertEqual(outputs.map(\.sats), Self.fundedSats)
        return outputs
    }

    /// Imports the test mnemonic's account as a watch-only hardware wallet by scanning its export
    private func importWatchOnlyWallet() {
        app.launch(expecting: app.staticTexts["Welcome to Cove"])
        app.startImportingExistingWallet()
        app.tapButton(startingWith: "Hardware wallet", until: app.button(startingWith: "Scan export QR"))
        app.tapButton(startingWith: "Scan export QR")

        let descriptors: HardwareSignerControl.Descriptors
        do {
            descriptors = try signer.watchOnlyDescriptors()
            try qr.show(["\(descriptors.external)\n\(descriptors.internal)"])
        } catch {
            return XCTFail("couldn't show the wallet export: \(error)")
        }

        let notNow = app.buttons["Not Now"]
        XCTAssertTrue(app.staticTexts["Protect this hardware wallet with Cloud Backup?"].waitForExistence(timeout: 30))
        notNow.tap()
        app.acceptTermsIfNeeded()

        XCTAssertTrue(app.element("selectedWallet.balance").waitForExistence(timeout: 30))
    }

    // MARK: - Hardware screen

    /// Shows the draft as an animated QR in the format, then copies the unsigned PSBT
    private func exportUnsignedPsbt(showing format: QrFormat) -> String {
        app.tapButton(named: "Export Transaction")
        app.tapButton(named: "QR Code")
        XCTAssertTrue(app.staticTexts["Scan this QR"].waitForExistence(timeout: 15))

        let formatButton = app.segmentedControls.buttons[format.rawValue]
        XCTAssertTrue(formatButton.waitForExistence(timeout: 5))
        formatButton.tap()
        XCTAssertTrue(formatButton.isSelected)

        // the "Copied" popup shows for only two seconds, which a slow CI simulator's existence
        // checks can miss, so wait on the pasteboard itself
        let psbt: String
        do {
            let previous = try copiedText()
            app.tapButton(named: "Copy")
            psbt = try waitForCopiedText(changingFrom: previous, timeout: 15)
        } catch {
            XCTFail("couldn't read the copied PSBT: \(error)")
            return ""
        }

        XCTAssertFalse(psbt.isEmpty, "the Copy button copied nothing")

        // close the export sheet
        app.swipeDown(velocity: .fast)
        XCTAssertTrue(app.buttons["Import Signature"].waitForExistence(timeout: 10))

        return psbt
    }

    private func copiedText() throws -> String {
        try signer.simulatorPasteboard().trimmingCharacters(in: .whitespacesAndNewlines)
    }

    /// Polls the simulator pasteboard until it holds new text, returning empty text on timeout
    private func waitForCopiedText(changingFrom previous: String, timeout: TimeInterval) throws -> String {
        let deadline = Date().addingTimeInterval(timeout)
        while Date() < deadline {
            let text = try copiedText()
            if !text.isEmpty, text != previous { return text }

            Thread.sleep(forTimeInterval: 0.5)
        }

        return ""
    }

    private func openSignatureScanner() {
        app.tapButton(named: "Import Signature")

        // a tap that lands while the menu animates in does nothing and leaves the menu open
        let qr = app.buttons["QR"]
        XCTAssertTrue(qr.waitForExistence(timeout: 10))
        for _ in 0 ..< 3 where qr.exists {
            qr.tap()
            _ = qr.waitForNonExistence(timeout: 3)
        }

        XCTAssertFalse(qr.exists, "the QR menu item never opened the scanner")
    }

    private func deleteDraft() {
        let delete = app.navigationBars.buttons["Delete"]
        XCTAssertTrue(delete.waitForExistence(timeout: 10))
        delete.tap()

        // deleting returns to the send screen, which Back leaves for the wallet
        let balance = app.element("selectedWallet.balance")
        let address = app.textViews["sendFlow.address"]
        for _ in 0 ..< 3 where !balance.waitForExistence(timeout: 3) || address.exists {
            app.goBack()
        }

        XCTAssertTrue(balance.waitForExistence(timeout: 15))
        waitUntil(address, matches: NSPredicate(format: "exists == false"), timeout: 10, "the send screen is still showing")
    }

    private func startDraftOfHalfTheBalance() {
        let half = Double(Self.fundedSats.reduce(0, +)) / 2 / 100_000_000

        app.startSend(to: XCUIApplication.externalAddress)
        let amount = app.textFields["sendFlow.amount"]
        XCTAssertTrue(amount.waitForExistence(timeout: 10))
        amount.tap()
        amount.typeText(String(format: "%.8f", half))
        app.tapNext()

        XCTAssertTrue(app.buttons["Import Signature"].waitForExistence(timeout: 30))
    }

    private func assertNoUnsignedTransactionAlert() {
        let alert = app.alerts["No Unsigned Transaction Found"]
        XCTAssertTrue(alert.waitForExistence(timeout: 15))
        XCTAssertTrue(alert.staticTexts.matching(NSPredicate(format: "label BEGINSWITH %@", "No unsigned transaction found for transaction")).firstMatch.exists)
        alert.buttons["OK"].tap()
        XCTAssertTrue(alert.waitForNonExistence(timeout: 5))
    }

    private func assertConfirmationScreen() {
        XCTAssertTrue(app.element("sendFlowConfirm.swipeToSend").waitForExistence(timeout: 30), "the signature didn't reach the confirm screen")
    }

    private func assertAdvancedDetails() {
        let address = app.staticTexts["Address"]
        XCTAssertTrue(address.waitForExistence(timeout: 10))
        address.tap()

        XCTAssertTrue(app.staticTexts["Advanced Details"].waitForExistence(timeout: 10))
        XCTAssertTrue(app.staticTexts["UTXOs Used"].waitForExistence(timeout: 15))
        XCTAssertTrue(app.staticTexts["Sent To Address"].exists)
        XCTAssertTrue(app.staticTexts["Fee"].exists)

        // a Max send has no change output
        XCTAssertFalse(app.staticTexts["UTXO Change"].exists)

        app.buttons["Close"].tap()
        XCTAssertTrue(app.staticTexts["Advanced Details"].waitForNonExistence(timeout: 5))
    }

    // MARK: - TAPSIGNER screens

    /// Adds a wallet from an NFC tag read, which the seam answers from the emulated card
    private func startTapSignerImport() {
        app.tapButton(named: "app.sidebar.open")
        app.tapButton(named: "Add Wallet")
        scanTapSigner()
    }

    /// Scans the card from Add Wallet, which the seam answers from the emulated card
    private func scanTapSigner() {
        app.tapButton(named: "More wallet import options")
        app.tapButton(named: "NFC")

        let alert = app.alerts["Import TAPSIGNER?"]
        XCTAssertTrue(alert.waitForExistence(timeout: 15))
        alert.buttons["Yes"].tap()
    }

    private func enterCvc(_ cvc: String) {
        let field = app.secureTextFields["Enter CVC"]
        XCTAssertTrue(field.waitForExistence(timeout: 10))
        field.tap()
        field.typeText(cvc)
        app.tapButton(named: "Continue")
    }

    private func tryAgainAfterWrongPin() {
        let alert = app.alerts["Wrong PIN"]
        XCTAssertTrue(alert.waitForExistence(timeout: 30))
        alert.buttons["Try Again"].tap()
    }

    /// A session that drops after the card has started the import offers Retry, which resumes it;
    /// one that drops before then fails the import, which starts again from the NFC scan
    private func retryInterruptedImport() {
        let retry = app.button(startingWith: "Retry")
        let failed = app.alerts["TAPSIGNER Import Failed"]

        let deadline = Date().addingTimeInterval(30)
        while Date() < deadline, !retry.exists, !failed.exists {
            Thread.sleep(forTimeInterval: 0.5)
        }

        if retry.exists {
            retry.tap()
            return
        }

        XCTAssertTrue(failed.exists, "a dropped session should offer a retry or report the failed import")
        failed.buttons["OK"].tap()
        scanTapSigner()
        enterCvc(HardwareSignerControl.tapSignerCvc)
    }

    private func retryInterruptedSigning() {
        // signing has no retry screen; the failure alert sends the user back to sign again
        let failed = app.alerts["Signing Failed!"]
        XCTAssertTrue(failed.waitForExistence(timeout: 30), "a dropped session should report the failed signing")
        failed.buttons["OK"].tap()

        app.tapButton(named: "Sign using TAPSIGNER")
        enterCvc(HardwareSignerControl.tapSignerCvc)
    }

    private func finishTapSignerImport() {
        XCTAssertTrue(app.staticTexts["Import Complete"].waitForExistence(timeout: 30))
        app.tapButton(named: "Continue")
        XCTAssertTrue(app.element("selectedWallet.balance").waitForExistence(timeout: 30))
    }

    /// The wallet only looks for new transactions on its own now and then, so pull to refresh
    /// until the funding shows
    private func refreshUntilBalance(sats: UInt64) {
        let balance = app.element("selectedWallet.balance")
        XCTAssertTrue(balance.waitForExistence(timeout: 15))

        let deadline = Date().addingTimeInterval(90)
        while Date() < deadline, satsFromAmountLabel(balance.label) != sats {
            let top = app.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.4))
            let bottom = app.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.9))
            top.press(forDuration: 0.1, thenDragTo: bottom)
            Thread.sleep(forTimeInterval: 3)
        }

        app.assertBalance(sats: sats, timeout: 5)
    }

    private func receiveAddress() throws -> String {
        app.tapButton(named: "Receive")

        // Copy Address stays disabled until the address loads, and closes the sheet once tapped
        let copy = app.buttons["Copy Address"]
        XCTAssertTrue(copy.waitForExistence(timeout: 10))
        waitUntil(copy, matches: NSPredicate(format: "enabled == true"), timeout: 15, "the receive address never loaded")
        copy.tap()
        waitUntil(copy, matches: NSPredicate(format: "exists == false"), timeout: 10, "copying the address should close Receive")

        return try signer.simulatorPasteboard().trimmingCharacters(in: .whitespacesAndNewlines)
    }
}
