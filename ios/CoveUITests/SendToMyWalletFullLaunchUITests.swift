import XCTest

/// Sends from the funded test wallet to another of the user's wallets through the send screen's
/// wallet picker, and checks the picker's list, the address a pick fills in, the recipient Confirm
/// shows and the address the broadcast pays
///
/// The other wallet is a hardware wallet imported from a second mnemonic's watch-only descriptors
/// through the debug NFC seam. The descriptors carry the key's fingerprint, so it imports as a
/// hardware wallet rather than an xpub-only one, and the signer host derives the account's first
/// receive address, which is the one a wallet with no history hands out next. Needs the fake node and the hardware signer host, which
/// `xtask ios-ui` (`just ios-ui-background`) starts for every run
final class SendToMyWalletFullLaunchUITests: XCTestCase {
    private static let esploraUrl = UITestHostPorts.esploraUrl
    private static let feeUrl = "\(esploraUrl)/api/v1/fees/recommended"
    private static let fundedSats: UInt64 = 100_000

    /// A BIP39 test vector, so the destination has a fingerprint of its own; a key from the test
    /// mnemonic would be refused as a duplicate of the sending wallet
    private static let destinationMnemonic = (Array(repeating: "zoo", count: 11) + ["wrong"]).joined(separator: " ")

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

        let funded = try node.fundWallet(mnemonic: XCUIApplication.testMnemonic, sats: [Self.fundedSats])
        try node.mine()
        let descriptors = try signer.watchOnlyDescriptors(mnemonic: Self.destinationMnemonic)

        // the same BIP84 account as the descriptors, for the address the host derives at /0/0
        let destination = try signer.publicKeyExport(.nativeSegwit, mnemonic: Self.destinationMnemonic)

        // 1: the destination comes first, from onboarding's NFC scan; the picker offers a wallet
        // only once its first scan has finished
        app.launch(expecting: app.staticTexts["Welcome to Cove"])
        app.startHardwareNfcScan()
        try nfc.showTextRecord("\(descriptors.external)\n\(descriptors.internal)")
        app.finishHardwareImport()
        app.assertBalance(sats: 0, timeout: 60)

        // 2: the funded wallet, imported from Add Wallet, opens as the selected wallet
        app.importTestMnemonicFromAddWallet()
        app.assertBalance(sats: Self.fundedSats, timeout: 120)

        // 3: the picker lists the other wallet as a hardware wallet, and not the sending wallet
        app.openSend()
        app.tapButton(named: "sendFlow.sendToMyWallet", until: app.navigationBars["Send to My Wallet"])

        let rows = app.buttons.matching(NSPredicate(format: "identifier BEGINSWITH %@", "sendFlow.transferDestination."))
        XCTAssertTrue(rows.firstMatch.waitForExistence(timeout: 10), "the picker listed no wallet")
        XCTAssertEqual(rows.count, 1, "the picker should list only the other wallet")
        XCTAssertTrue(app.staticTexts["Hardware wallet"].exists, "the destination should be marked as a hardware wallet")
        XCTAssertTrue(
            app.staticText(containing: "Check the address on the device").exists,
            "a hardware wallet destination should come with the check-on-device note"
        )

        rows.firstMatch.tap()
        XCTAssertTrue(app.navigationBars["Send to My Wallet"].waitForNonExistence(timeout: 10), "picking a wallet should close the picker")

        // 4: the pick fills in the destination's next receive address; the unfocused field shows it
        // as grouped text over an empty editor
        waitForAddressText(destination.firstAddress, "the pick should fill in")

        // 5: Confirm names that address as the recipient, and the broadcast pays it
        app.tapMax()
        app.tapNext()
        assertConfirmRecipient(destination.firstAddress)
        app.swipeToSend()

        let sent = try node.waitForBroadcasts(count: 1)[0]
        XCTAssertEqual(sent.inputs, [funded[0].outpoint])
        XCTAssertTrue(
            sent.outputs.contains { $0.address == destination.firstAddress },
            "the send should pay \(destination.firstAddress), it paid \(sent.outputs)"
        )
    }

    /// Waits for Confirm to show the address as the recipient
    private func assertConfirmRecipient(_ address: String) {
        XCTAssertTrue(app.element("sendFlowConfirm.swipeToSend").waitForExistence(timeout: 30), "the send didn't reach Confirm")
        waitForAddressText(address, "Confirm should name the recipient")
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

    private static func unspaced(_ text: String) -> String {
        text.filter { !$0.isWhitespace }
    }
}
