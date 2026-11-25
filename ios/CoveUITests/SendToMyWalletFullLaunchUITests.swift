import XCTest

/// Sends from the funded test wallet to another of the user's wallets through "Send between
/// wallets", and checks the Your Wallets sheet, the picked wallet's card and its Change link,
/// editing the address after a pick, the TO WALLET card on Confirm with its hardware wallet note,
/// and the address the broadcast pays
///
/// There are two other wallets. One is a hardware wallet imported from a second mnemonic's
/// watch-only descriptors through the debug NFC seam. The descriptors carry the key's fingerprint,
/// so it imports as a hardware wallet rather than an xpub-only one, and the signer host derives
/// the account's first receive address, which is the one a wallet with no history hands out next.
/// The other is an emulated TAPSIGNER, which has no screen to check an address on. Needs the fake
/// node, the hardware signer host and its TAPSIGNER emulator, which `xtask ios-ui`
/// (`just ios-ui-background`) starts for every run
final class SendToMyWalletFullLaunchUITests: XCTestCase {
    private static let esploraUrl = UITestHostPorts.esploraUrl
    private static let feeUrl = "\(esploraUrl)/api/v1/fees/recommended"
    private static let fundedSats: UInt64 = 100_000

    /// A BIP39 test vector, so the destination has a fingerprint of its own; a key from the test
    /// mnemonic would be refused as a duplicate of the sending wallet
    private static let destinationMnemonic = (Array(repeating: "zoo", count: 11) + ["wrong"]).joined(separator: " ")

    private static let checkOnDeviceNote = "Check this address on your hardware wallet before you send."

    private var app: XCUIApplication!
    private var node: FakeNodeControl!
    private var signer: HardwareSignerControl!
    private var nfc: NfcNdefScript!

    override func setUp() {
        super.setUp()
        continueAfterFailure = false

        app = XCUIApplication()
        node = FakeNodeControl()
        signer = HardwareSignerControl()
        nfc = NfcNdefScript()

        // a fresh install on Signet against the fake node, with fee rates from it too
        app.launchEnvironment["COVE_UI_TEST_ESPLORA_URL"] = Self.esploraUrl
        app.launchEnvironment["COVE_FEE_URL"] = Self.feeUrl
        nfc.configure(app)
    }

    override func tearDown() {
        nfc.remove()
        super.tearDown()
    }

    func testPickingAnotherWalletFillsItsNextAddressAndSendsThere() throws {
        try node.requireHealthy()
        try signer.requireHealthy()
        try node.reset()
        try signer.startTapSigner()

        let funded = try node.fundWallet(mnemonic: XCUIApplication.testMnemonic, sats: [Self.fundedSats])
        try node.mine()
        let descriptors = try signer.watchOnlyDescriptors(mnemonic: Self.destinationMnemonic)
        let fingerprint = try XCTUnwrap(Self.fingerprint(in: descriptors.external), "the descriptor names no fingerprint")

        // the same BIP84 account as the descriptors, for the address the host derives at /0/0
        let destination = try signer.publicKeyExport(.nativeSegwit, mnemonic: Self.destinationMnemonic)

        try importWallets(hardware: descriptors)

        // 4: Send shows the address card and the row that opens the other wallets
        app.openSend()
        XCTAssertTrue(app.staticTexts["Send to"].exists, "the address section should be headed Send to")
        XCTAssertTrue(app.staticTexts["Paste or scan address"].exists, "the empty address card should say what to do")
        XCTAssertTrue(app.buttons["sendFlow.sendBetweenWallets"].exists, "Send should offer the other wallets")

        // 5: the sheet lists both other wallets, the hardware wallet by its fingerprint, without the
        // sending wallet or any note
        openYourWallets(from: "sendFlow.sendBetweenWallets")
        let rows = app.buttons.matching(NSPredicate(format: "identifier BEGINSWITH %@", "sendFlow.transferDestination."))
        XCTAssertEqual(rows.count, 2, "the sheet should list only the two other wallets")

        // the hardware wallet's name can hold its fingerprint too, so this reads the row's detail
        let coldRow = rows.matching(NSPredicate(format: "value == %@", fingerprint)).firstMatch
        let tapSignerRow = rows.matching(NSPredicate(format: "value != %@", fingerprint)).firstMatch
        XCTAssertTrue(coldRow.exists, "the hardware wallet's row should show its fingerprint \(fingerprint), rows: \(rows.allElementsBoundByIndex.map { "\($0.label): \($0.value ?? "")" })")
        XCTAssertFalse(app.staticText(containing: "hardware wallet").exists, "the sheet should carry no hardware wallet note")

        // 6: picking the TAPSIGNER shows its card with its name and address
        pick(tapSignerRow)
        let recipientName = app.staticTexts["sendFlow.transferRecipient.name"]
        XCTAssertTrue(recipientName.waitForExistence(timeout: 30), "picking a wallet should show its card")
        let tapSignerName = recipientName.label
        let recipientAddress = app.staticTexts["sendFlow.transferRecipient.address"]
        XCTAssertTrue(Self.unspaced(recipientAddress.label).hasPrefix("tb1"), "the card should show the picked address, it shows '\(recipientAddress.label)'")

        // 7: tapping the address goes back to editing it, and an edit drops the pick for good: once the
        // amount takes the focus back, the card would show again for a pick that was kept
        revealAboveKeyboard(recipientAddress)
        recipientAddress.tap()
        let field = app.textViews["sendFlow.address"]
        XCTAssertTrue(field.waitForExistence(timeout: 10), "tapping the card should go back to the address field")
        field.typeTextOnceFocused("x")
        app.textFields["sendFlow.amount"].tap()
        XCTAssertTrue(field.waitForExistence(timeout: 10), "an edited address should stay in the field")
        XCTAssertFalse(recipientName.waitForExistence(timeout: 3), "an edited address should drop the picked wallet")

        // 8: Confirm names the picked TAPSIGNER, with no note, since it has no screen
        openYourWallets(from: "sendFlow.sendBetweenWallets")
        pick(tapSignerRow)
        waitForLabel(recipientName, tapSignerName, "picking the TAPSIGNER again should show its card")
        app.tapMax()
        app.tapNext()
        assertConfirmRecipient(named: tapSignerName)
        XCTAssertFalse(app.element("sendFlowConfirm.transferRecipient.checkOnDevice").exists, "a TAPSIGNER has no screen to check the address on")
        XCTAssertFalse(app.staticTexts[Self.checkOnDeviceNote].exists)

        // 9: back on Set Amount the pick stays, and Change swaps it for the hardware wallet, whose next
        // address it fills in
        app.goBack()
        waitForLabel(recipientName, tapSignerName, "going back should keep the picked wallet")
        openYourWallets(from: "sendFlow.transferRecipient.change")
        pick(coldRow)
        waitForLabel(recipientName, { $0 != tapSignerName }, "Change should swap the picked wallet")
        let coldName = recipientName.label
        waitForAddressText(destination.firstAddress, "the hardware wallet's card should show")

        // 10: Confirm names the hardware wallet with the check-on-device note, and the broadcast
        // pays its address
        app.tapNext()
        assertConfirmRecipient(named: coldName)
        XCTAssertTrue(app.staticTexts[Self.checkOnDeviceNote].exists, "a hardware wallet with a screen should come with the check-on-device note")
        waitForAddressText(destination.firstAddress, "Confirm should show the picked address")
        app.swipeToSend()
        try assertBroadcast(spending: funded[0].outpoint, paying: destination.firstAddress)
    }

    // MARK: - Steps

    /// Imports the hardware wallet, the TAPSIGNER, and then the funded wallet, which opens as the
    /// selected wallet
    private func importWallets(hardware descriptors: HardwareSignerControl.Descriptors) throws {
        // 1: the hardware wallet comes first, from onboarding's NFC scan; the sheet offers a wallet
        // only once its first scan has finished
        app.launch(expecting: app.staticTexts["Welcome to Cove"])
        app.startHardwareNfcScan()
        try nfc.showTextRecord("\(descriptors.external)\n\(descriptors.internal)")
        app.finishHardwareImport()
        app.assertBalance(sats: 0, timeout: 60)

        // 2: the TAPSIGNER bridge answers every NFC scan once it is set, so it waits for a relaunch
        app.terminate()
        app.launchEnvironment["COVE_UI_TEST_TAPSIGNER_BRIDGE"] = HardwareSignerControl.tapSignerBridge
        app.launch(expecting: app.element("selectedWallet.balance"))
        app.scanTapSignerFromAddWallet(expectingAlert: "Import TAPSIGNER?")
        app.enterTapSignerCvc(HardwareSignerControl.tapSignerCvc)
        XCTAssertTrue(app.staticTexts["Import Complete"].waitForExistence(timeout: 30))
        app.tapButton(named: "Continue")
        app.assertBalance(sats: 0, timeout: 60)

        // 3: the funded wallet, imported from Add Wallet, opens as the selected wallet
        app.importTestMnemonicFromAddWallet()
        app.assertBalance(sats: Self.fundedSats, timeout: 120)
    }

    private func assertBroadcast(spending outpoint: String, paying address: String) throws {
        let sent = try node.waitForBroadcasts(count: 1)[0]
        XCTAssertEqual(sent.inputs, [outpoint])
        XCTAssertTrue(
            sent.outputs.contains { $0.address == address },
            "the send should pay \(address), it paid \(sent.outputs)"
        )
    }

    private func openYourWallets(from opener: String) {
        revealAboveKeyboard(app.buttons[opener])
        app.tapButton(named: opener, until: app.staticTexts["sendFlow.transferDestinations.title"])
        XCTAssertEqual(app.staticTexts["sendFlow.transferDestinations.title"].label, "Your Wallets")
    }

    private func pick(_ row: XCUIElement) {
        XCTAssertTrue(row.waitForExistence(timeout: 10), "the sheet lost the wallet's row")
        row.tap()
        XCTAssertTrue(
            app.staticTexts["sendFlow.transferDestinations.title"].waitForNonExistence(timeout: 10),
            "picking a wallet should close the sheet"
        )
    }

    /// Scrolls Set Amount until the element sits above the keyboard and its toolbar
    ///
    /// The amount field takes the focus when Send opens and after a pick, and its keyboard covers the
    /// Send to section. The toolbar's Done only closes it once the amount and address are valid, so
    /// the test scrolls instead, dragging from the section's header, which is plain text
    private func revealAboveKeyboard(_ element: XCUIElement) {
        XCTAssertTrue(element.waitForExistence(timeout: 10), "\(element) never showed")

        for _ in 0 ..< 6 {
            guard let top = keyboardTop(), element.frame.maxY > top - 8 else { return }

            let header = app.staticTexts["Send to"]
            let start = header.isHittable
                ? header.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.5))
                : app.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.5))
            start.press(forDuration: 0.05, thenDragTo: start.withOffset(CGVector(dx: 0, dy: -150)))
        }

        XCTFail("the keyboard kept covering \(element)")
    }

    /// The top of the keyboard or of the toolbar above it, whichever is higher, while one shows
    private func keyboardTop() -> CGFloat? {
        let keyboard = app.keyboards.firstMatch
        guard keyboard.exists else { return nil }

        // the screen's own toolbar container spans the whole window
        let toolbarTops = app.toolbars.allElementsBoundByIndex
            .map(\.frame)
            .filter { $0.height < 100 && $0.minY > 0 }
            .map(\.minY)

        return ([keyboard.frame.minY] + toolbarTops).min()
    }

    private func assertConfirmRecipient(named name: String) {
        XCTAssertTrue(app.element("sendFlowConfirm.swipeToSend").waitForExistence(timeout: 30), "the send didn't reach Confirm")
        XCTAssertTrue(app.staticText(containing: "TO WALLET").exists, "Confirm should show the TO WALLET card")
        waitForLabel(app.staticTexts["sendFlowConfirm.transferRecipient.name"], name, "Confirm should name the picked wallet")
    }

    private func waitForLabel(_ element: XCUIElement, _ label: String, _ message: String) {
        waitForLabel(element, { $0 == label }, "\(message): '\(label)'")
    }

    private func waitForLabel(_ element: XCUIElement, _ matches: @escaping (String) -> Bool, _ message: String) {
        let deadline = Date().addingTimeInterval(30)
        while Date() < deadline {
            if element.exists, matches(element.label) { return }

            Thread.sleep(forTimeInterval: 0.5)
        }

        XCTFail("\(message), it shows '\(element.exists ? element.label : "nothing")'")
    }

    /// Waits for a text showing the address, which the app spaces out in groups
    private func waitForAddressText(_ address: String, _ message: String, timeout: TimeInterval = 30) {
        let candidates = app.staticTexts.matching(NSPredicate(format: "label BEGINSWITH %@", String(address.prefix(4))))
        let deadline = Date().addingTimeInterval(timeout)
        while Date() < deadline {
            if candidates.allElementsBoundByIndex.contains(where: { Self.unspaced($0.label) == address }) { return }

            Thread.sleep(forTimeInterval: 0.5)
        }

        let shown = candidates.allElementsBoundByIndex.map(\.label)
        XCTFail("\(message) \(address), it shows \(shown)")
    }

    /// The master fingerprint a descriptor's key origin names, uppercased as the app shows it
    private static func fingerprint(in descriptor: String) -> String? {
        guard let range = descriptor.range(of: #"\[[0-9a-fA-F]{8}"#, options: .regularExpression) else { return nil }
        return String(descriptor[range].dropFirst()).uppercased()
    }

    private static func unspaced(_ text: String) -> String {
        text.filter { !$0.isWhitespace }
    }
}
