import XCTest

/// Checks the note under the send address field that names which of the user's wallets owns the
/// address, on Set Amount and on coin control's Set Amount, in the real app against cove-fake-node
///
/// The first wallet is the funded test mnemonic and the second the empty legal winner phrase,
/// scanned in through the debug QR seam. Needs the fake node on the ports `COVE_UI_TEST_PORT_BASE`
/// names, which `xtask ios-ui` (`just ios-ui-background`) starts for every run
final class AddressOwnershipSendFullLaunchUITests: XCTestCase {
    private static let esploraUrl = UITestHostPorts.esploraUrl

    /// The test mnemonic's receive address at index 2, which no send reveals
    private static let senderReceive2 = "tb1qxdyjf6h5d6qxap4n2dap97q4j5ps6ua8sll0ct"

    /// The second mnemonic's first receive address
    private static let otherReceive0 = "tb1qjgx204hxfwuse548jc34fjzg6ffq8pvrqwlex4"

    /// The second mnemonic's first change address
    private static let otherChange0 = "tb1qmapywu8v8uztsqk4j8f5sarjzm2j9r04n8lauv"

    private var app: XCUIApplication!
    private var node: FakeNodeControl!
    private var qr: QrScript!

    override func setUp() {
        super.setUp()
        continueAfterFailure = false

        app = XCUIApplication()
        node = FakeNodeControl()
        qr = QrScript()
        qr.configure(app)

        // fee rates come from the fake node, so Set Amount doesn't depend on mempool.space
        app.launchEnvironment["COVE_FEE_URL"] = "\(Self.esploraUrl)/api/v1/fees/recommended"
    }

    override func tearDown() {
        qr?.remove()
        super.tearDown()
    }

    /// The note names the sending wallet for its own address and the other wallet by name for
    /// one of its addresses, follows the address as it is edited, and stays away for an address
    /// no wallet owns
    func testSetAmountNamesTheWalletThatOwnsTheAddress() throws {
        try node.requireHealthy()
        try node.reset()
        _ = try node.fundWallet(mnemonic: XCUIApplication.testMnemonic, sats: [100_000, 60000])
        try node.mine()

        let (sender, other) = try importBothWallets()

        // Set Amount, from the sender's own address to the other wallet's, then to a stranger's
        // with no amount a whole address moves the focus to the amount, and the number pad then
        // covers the note, so set one first and each whole address closes the keyboard instead
        app.startSend(to: Self.senderReceive2)
        app.tapMax()
        assertOwnershipNote("This is an address in this wallet (receive, index 2)")

        replaceAddress(with: Self.otherReceive0)
        assertOwnershipNote("This is your address in “\(other)” (receive, index 0)")

        replaceAddress(with: XCUIApplication.externalAddress)
        assertNoOwnershipNote()

        // coin control's own Set Amount screen
        leaveToWallet(named: sender)
        openManageUtxos()
        app.tapButton(named: "coinControl.selectAll")
        app.tapButton(named: "coinControl.continue")
        app.enterAddress(Self.otherChange0)
        assertOwnershipNote("This is your address in “\(other)” (change, index 0)")
    }

    // MARK: - Steps

    /// Imports the funded test mnemonic, then the empty second wallet, and returns to the first;
    /// returns both wallets' names
    private func importBothWallets() throws -> (sender: String, other: String) {
        app.launchEnvironment["COVE_UI_TEST_ESPLORA_URL"] = Self.esploraUrl
        app.launch(expecting: app.staticTexts["Welcome to Cove"])
        app.importTestMnemonic()
        app.assertBalance(sats: 160_000, timeout: 120)
        let sender = app.selectedWalletName

        try app.importFromAddWallet(scanning: XCUIApplication.secondMnemonic, with: qr)
        app.assertBalance(sats: 0, timeout: 60)
        let other = app.selectedWalletName
        XCTAssertNotEqual(sender, other, "both wallets have the same name, so the note can't tell them apart")

        app.selectWallet(named: sender)
        app.assertBalance(sats: 160_000, timeout: 60)
        return (sender, other)
    }

    private func openManageUtxos() {
        app.tapButton(named: "selectedWallet.more")
        app.tapButton(named: "selectedWallet.manageUtxos", until: app.navigationBars["Manage UTXOs"])
    }

    /// Backs out of the send screens to the wallet
    private func leaveToWallet(named name: String) {
        let title = app.element("selectedWallet.title")
        for _ in 0 ..< 3 where !title.exists {
            if app.keyboards.count > 0 { app.buttons["Done"].firstMatch.tap() }
            app.goBack()
            _ = title.waitForExistence(timeout: 5)
        }

        waitForLabel(title, equalTo: name, timeout: 10)
    }

    /// Deletes the address in the field and types another, as a person correcting it would
    private func replaceAddress(with address: String) {
        let field = app.textViews["sendFlow.address"]
        XCTAssertTrue(field.waitForExistence(timeout: 10))
        if app.keyboards.count > 0, !field.isHittable {
            app.buttons["Done"].firstMatch.tap()
        }

        field.tap()
        let current = (field.value as? String) ?? ""
        let deletes = String(repeating: XCUIKeyboardKey.delete.rawValue, count: max(current.count, 64))
        field.typeTextOnceFocused(deletes)
        waitUntil(field, matches: NSPredicate(format: "value == ''"), timeout: 5, "the address field didn't clear")
        field.typeText(address)
    }

    // MARK: - Checks

    private var ownershipNote: XCUIElement {
        app.element("sendFlow.addressOwnership")
    }

    private func assertOwnershipNote(_ expected: String) {
        waitForLabel(ownershipNote, equalTo: expected, timeout: 20)
    }

    /// The lookup for an address no wallet owns ends quietly, so give it time to finish before
    /// checking nothing showed
    private func assertNoOwnershipNote() {
        XCTAssertTrue(ownershipNote.waitForNonExistence(timeout: 10), "a note stayed for an address no wallet owns")
        Thread.sleep(forTimeInterval: 3)
        XCTAssertFalse(ownershipNote.exists, "a note showed for an address no wallet owns: \(ownershipNote.label)")
    }
}
