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
    private static let esploraUrl = UITestHostPorts.esploraUrl
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

    // MARK: - Large text

    /// Side by side at the largest text size, each sign action had room for only part of a word,
    /// which iOS split with an added hyphen
    func testSignActionsDrawWholeAtLargestTextSize() throws {
        _ = try fundTestWallet()
        importWatchOnlyWallet()
        app.assertBalance(sats: Self.fundedSats.reduce(0, +), timeout: 120)

        // at the largest size the keyboard covers the send screen's fields, so the draft is made
        // at the default size and reopened from the wallet's transactions after the relaunch
        app.startSend(to: XCUIApplication.externalAddress)
        app.tapMax()
        app.tapNext()
        XCTAssertTrue(app.buttons["Import Signature"].waitForExistence(timeout: 30))
        app.terminate()

        let (probeKey, probeValue) = ScreenReachability.truncationProbeEnvironment
        app.launchEnvironment[probeKey] = probeValue
        app.launchArguments += ["-UIPreferredContentSizeCategoryName", "UICTContentSizeCategoryAccessibilityXXXL"]
        app.launch(expecting: app.element("selectedWallet.balance"))

        // beside its icon and amount, the draft's row split "Pending Signature" with a hyphen; the
        // row takes taps through a gesture rather than a button, and XCUITest reports its texts
        // unhittable even on screen, so the tap goes to the text's center
        let screen = ScreenReachability(app: app)
        let draft = screen.assertReachable(
            app.staticTexts.matching(NSPredicate(format: "label == %@", "Pending Signature")),
            "wallet: the draft",
            needsHit: false
        )

        screen.assertDrawsWhole(draft, "wallet: the draft's Pending Signature")
        draft.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.5)).tap()
        XCTAssertTrue(app.buttons["Import Signature"].waitForExistence(timeout: 30))

        screen.assertScreenFits(
            "hardware sign",
            texts: ["Sign Transaction"],
            buttons: [signAction("Export Transaction"), signAction("Import Signature")]
        )
    }

    /// Buttons whose label starts with the title, since a label can also hold its icon's name
    private func signAction(_ title: String) -> XCUIElementQuery {
        app.buttons.matching(NSPredicate(format: "label BEGINSWITH %@", title))
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
        app.refreshUntilBalance(sats: 100_000)

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
            // every signing error shows the same alert, which under the bridge also names the
            // underlying error, so any other failure still fails the test
            let invalidPath = NSPredicate(format: "label CONTAINS %@", "PsbtSignError: Invalid path at index")
            guard signingFailed.staticTexts.matching(invalidPath).firstMatch.exists else {
                let shown = signingFailed.staticTexts.allElementsBoundByIndex.map(\.label)
                return XCTFail("the wrong CVC failed signing with an error other than #965: \(shown)")
            }

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
        app.openHardwareExportScanner()

        let descriptors: HardwareSignerControl.Descriptors
        do {
            descriptors = try signer.watchOnlyDescriptors()
            try qr.show(["\(descriptors.external)\n\(descriptors.internal)"])
        } catch {
            return XCTFail("couldn't show the wallet export: \(error)")
        }

        app.finishHardwareImport()
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

        closeExportSheet()
        return psbt
    }

    /// Swipes the export sheet away and waits until the hardware screen under it takes taps
    private func closeExportSheet() {
        // the "Copied" popup can dismiss on a tap outside it, so while it shows its window takes
        // every touch and a swipe never reaches the sheet
        XCTAssertTrue(app.staticTexts["Copied"].waitForNonExistence(timeout: 10), "the Copied popup never went away")

        // the sheet has no close button, and the Import Signature button behind it stays in the
        // accessibility tree while it shows, so wait for the sheet itself to go
        let sheetTitle = app.staticTexts["Scan this QR"]
        for _ in 0 ..< 3 where sheetTitle.exists {
            app.swipeDown(velocity: .fast)
            _ = sheetTitle.waitForNonExistence(timeout: 5)
        }

        XCTAssertFalse(sheetTitle.exists, "the export sheet never closed")

        let hittable = NSPredicate(format: "isHittable == true")
        waitUntil(app.buttons["Import Signature"], matches: hittable, timeout: 10, "Import Signature never became tappable")
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
        app.scanTapSignerFromAddWallet(expectingAlert: "Import TAPSIGNER?")
    }

    private func enterCvc(_ cvc: String) {
        app.enterTapSignerCvc(cvc)
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
        app.scanTapSigner(expectingAlert: "Import TAPSIGNER?")
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

    private func receiveAddress() throws -> String {
        try app.copyReceiveAddress(using: signer)
    }
}

// MARK: - Drawn export frames

/// Reads the unsigned export off the screen as a hardware wallet's camera would, instead of from
/// Copy, so a frame the sheet never draws or draws unreadably fails the send
extension HardwareSigningFullLaunchUITests {
    /// The fewest frames an export should cycle through, so the test reads an animated QR
    private static let drawnExportMinParts = 3

    /// Bounds the screenshots taken to see every part; a part that never shows fails within it
    private static let drawnExportTimeout: TimeInterval = 90

    /// Bounds the wait for the first frame of the split the frame counter shows
    private static let drawnExportSettleTimeout: TimeInterval = 5

    private struct DrawnExport {
        let psbt: String
        let frames: Int
        let captures: Int
        let duration: TimeInterval
    }

    func testDrawnUrExportJoinsToTheDraftAndItsSignatureBroadcasts() throws {
        try signDrawnExportAndBroadcast(.ur)
    }

    func testDrawnBbqrExportJoinsToTheDraftAndItsSignatureBroadcasts() throws {
        try signDrawnExportAndBroadcast(.bbqr)
    }

    private func signDrawnExportAndBroadcast(_ format: QrFormat) throws {
        let funded = try fundTestWallet()
        importWatchOnlyWallet()
        app.assertBalance(sats: Self.fundedSats.reduce(0, +), timeout: 120)

        app.startSend(to: XCUIApplication.externalAddress)
        app.tapMax()
        app.tapNext()

        let copied = exportUnsignedPsbt(showing: format)

        openExportSheet(showing: format)
        let shownParts = lowerDensityUntilAnimated()
        let stale = dropStaleFrames(format, shownParts: shownParts)
        let drawn = try readDrawnExport(format)
        closeExportSheet()

        print("drawn \(format.rawValue) export: \(drawn.frames) of \(shownParts) frames in \(drawn.captures) captures over \(String(format: "%.1f", drawn.duration)) s, after dropping \(stale) stale frames")
        XCTAssertEqual(drawn.frames, shownParts, "the frame counter and the drawn frames disagree on the part count")
        XCTAssertEqual(drawn.psbt, copied.lowercased(), "the drawn frames carry a different PSBT than Copy")

        let signed = try signer.sign(drawn.psbt, minParts: Self.animatedParts)
        openSignatureScanner()
        try qr.show(Array(signed.frames(format).reversed()))

        assertConfirmationScreen()
        app.swipeToSend()

        let broadcast = try node.waitForBroadcasts(count: 1)[0]
        XCTAssertEqual(Set(broadcast.inputs), Set(funded.map(\.outpoint)), "a Max send spends every UTXO")
        XCTAssertEqual(broadcast.outputs.map(\.address), [XCUIApplication.externalAddress])
    }

    private func openExportSheet(showing format: QrFormat) {
        app.tapButton(named: "Export Transaction")
        app.tapButton(named: "QR Code")
        XCTAssertTrue(app.staticTexts["Scan this QR"].waitForExistence(timeout: 15))

        let formatButton = app.segmentedControls.buttons[format.rawValue]
        XCTAssertTrue(formatButton.waitForExistence(timeout: 5))
        formatButton.tap()
        XCTAssertTrue(formatButton.isSelected)
    }

    /// Lowers the density until the export cycles through at least `drawnExportMinParts` frames,
    /// and returns how many the frame counter shows
    ///
    /// A small PSBT fits in one BBQr frame at the default density, which shows no frame counter
    private func lowerDensityUntilAnimated() -> Int {
        let counter = app.descendants(matching: .any)["QR frame"]
        let decrease = app.buttons["Decrease density"]
        XCTAssertTrue(decrease.waitForExistence(timeout: 10))

        var shown = counter.waitForExistence(timeout: 3) ? frameCount(counter) : 1
        for _ in 0 ..< 6 where shown < Self.drawnExportMinParts && decrease.isEnabled {
            decrease.tap()

            // the sheet regenerates its frames, and frames drawn before then would join a different
            // split, so wait until the counter shows the new count
            let previous = shown
            let deadline = Date().addingTimeInterval(5)
            while Date() < deadline, shown == previous {
                Thread.sleep(forTimeInterval: 0.2)
                shown = counter.exists ? frameCount(counter) : 1
            }
        }

        XCTAssertGreaterThanOrEqual(shown, Self.drawnExportMinParts, "the export never became an animated QR")
        return shown
    }

    /// The N of the counter's "i of N"
    private func frameCount(_ counter: XCUIElement) -> Int {
        let value = counter.value as? String ?? ""
        return value.split(separator: " ").last.flatMap { Int($0) } ?? 0
    }

    /// Decodes the sheet's QR until it shows a frame of the split the frame counter reports, and
    /// returns how many decoded frames of an earlier split it dropped first
    private func dropStaleFrames(_ format: QrFormat, shownParts: Int) -> Int {
        // the counter can update before the image is redrawn, so the first frames after a density
        // change can still belong to the split it replaced, which the host rightly won't join
        let prefix = Self.framePrefix(format)
        let deadline = Date().addingTimeInterval(Self.drawnExportSettleTimeout)
        var stale = 0

        while Date() < deadline {
            for code in app.visibleQrCodes where code.lowercased().hasPrefix(prefix.lowercased()) {
                if Self.partCount(code) == shownParts { return stale }
                stale += 1
            }

            Thread.sleep(forTimeInterval: .random(in: 0.05 ... 0.25))
        }

        XCTFail("no drawn frame showed the \(shownParts) parts the frame counter reports within \(Int(Self.drawnExportSettleTimeout)) s, after \(stale) stale frames")
        return stale
    }

    private static func framePrefix(_ format: QrFormat) -> String {
        switch format {
        case .ur: "ur:"
        case .bbqr: "B$"
        }
    }

    /// The part count in a frame's header: a BBQr's base 36 total, or the N of a UR's `i-N`
    private static func partCount(_ frame: String) -> Int? {
        if frame.hasPrefix("B$") {
            let header = Array(frame)
            guard header.count >= 6 else { return nil }
            return Int(String(header[4 ..< 6]), radix: 36)
        }

        // a multi-part UR's path is `ur:type/i-N/fragment`
        let path = frame.split(separator: "/")
        guard path.count == 3, let total = path[1].split(separator: "-").last else { return nil }
        return Int(total)
    }

    /// Screenshots the sheet and decodes its QR until the host can join every distinct frame seen
    private func readDrawnExport(_ format: QrFormat) throws -> DrawnExport {
        let prefix = Self.framePrefix(format)

        let started = Date()
        var frames: [String] = []
        var captures = 0
        var partsLeft: Int?

        while Date().timeIntervalSince(started) < Self.drawnExportTimeout {
            captures += 1
            for code in app.visibleQrCodes where code.lowercased().hasPrefix(prefix.lowercased()) && !frames.contains(code) {
                frames.append(code)

                let joined = try signer.joinExportFrames(frames)
                if let psbt = joined.psbt {
                    return DrawnExport(psbt: psbt, frames: frames.count, captures: captures, duration: Date().timeIntervalSince(started))
                }

                partsLeft = joined.partsLeft
            }

            // a random pause keeps the screenshots from falling into step with the frame timer
            Thread.sleep(forTimeInterval: .random(in: 0.05 ... 0.25))
        }

        XCTFail("decoded \(frames.count) distinct \(format.rawValue) frames in \(captures) captures, and \(partsLeft.map(String.init) ?? "some") parts never showed")
        return DrawnExport(psbt: "", frames: frames.count, captures: captures, duration: Date().timeIntervalSince(started))
    }
}
