import XCTest

/// Checks that the alert for an address scanned from the wallet screen names which of the user's
/// wallets owns it, in the real app against cove-fake-node
///
/// The first wallet is the test mnemonic and the second the legal winner phrase, scanned in
/// through the debug QR seam, which also stands in for the camera when the addresses are scanned.
/// Needs the fake node on the ports `COVE_UI_TEST_PORT_BASE` names, which `xtask ios-ui`
/// (`just ios-ui-background`) starts for every run
final class AddressOwnershipScanFullLaunchUITests: XCTestCase {
    private static let esploraUrl = UITestHostPorts.esploraUrl

    /// The test mnemonic's first receive address
    private static let firstReceive0 = "tb1q6rz28mcfaxtmd6v789l9rrlrusdprr9pqcpvkl"

    /// The second mnemonic's first change address
    private static let secondChange0 = "tb1qmapywu8v8uztsqk4j8f5sarjzm2j9r04n8lauv"

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
    }

    override func tearDown() {
        qr?.remove()
        super.tearDown()
    }

    /// Scanned from the second wallet, its own change address says it is this wallet's without
    /// offering Go to Wallet, an address no wallet owns shows the alert without a note, and the
    /// first wallet's address names the first wallet and offers Go to Wallet, which opens it
    func testFoundAddressAlertNamesTheWalletThatOwnsTheAddress() throws {
        try node.requireHealthy()
        try node.reset()

        app.launchEnvironment["COVE_UI_TEST_ESPLORA_URL"] = Self.esploraUrl
        app.launch(expecting: app.staticTexts["Welcome to Cove"])
        app.importTestMnemonic()
        app.assertBalance(sats: 0, timeout: 120)
        let first = app.selectedWalletName

        try app.importFromAddWallet(scanning: XCUIApplication.secondMnemonic, with: qr)
        app.assertBalance(sats: 0, timeout: 60)
        let second = app.selectedWalletName
        XCTAssertNotEqual(second, first, "both wallets have the same name, so the alert can't tell them apart")

        let ownAddress = try scanFoundAddress(Self.secondChange0)
        assertMessage(ownAddress, contains: "This address belongs to this wallet, “\(second)”")
        XCTAssertFalse(ownAddress.buttons["Go to Wallet"].exists, "the open wallet's own address offered Go to Wallet, which leads back here")
        dismiss(ownAddress)

        // the alert waits for the lookup, so it shows without a note once the lookup found no owner
        let strangersAddress = try scanFoundAddress(XCUIApplication.externalAddress)
        let note = strangersAddress.staticTexts.matching(NSPredicate(format: "label CONTAINS %@", "belongs to")).firstMatch
        XCTAssertFalse(note.exists, "the alert named an owner for an address no wallet owns: \(note.label)")
        XCTAssertFalse(strangersAddress.buttons["Go to Wallet"].exists, "an address no wallet owns offered Go to Wallet")
        dismiss(strangersAddress)

        let firstsAddress = try scanFoundAddress(Self.firstReceive0)
        assertMessage(firstsAddress, contains: "This address belongs to your wallet “\(first)”")
        firstsAddress.buttons["Go to Wallet"].tap()
        XCTAssertTrue(firstsAddress.waitForNonExistence(timeout: 10), "Go to Wallet left the alert up")
        waitForLabel(app.element("selectedWallet.title"), equalTo: first, timeout: 30)
    }

    // MARK: - Steps

    /// Opens the wallet screen's scanner, scans `address` and returns the alert it raises
    private func scanFoundAddress(_ address: String) throws -> XCUIElement {
        // the toolbar's scan button is an icon, labelled on some iOS versions and only identified
        // by its symbol on others, as in WatchOnlyUpgradeFullLaunchUITests
        let scan = app.buttons.matching(NSPredicate(format: "label ==[c] %@ OR identifier == %@", "QR Code", "qrcode")).firstMatch
        XCTAssertTrue(scan.waitForExistence(timeout: 10), "the wallet screen's QR scanner button is missing")

        // the simulator's scanner shows its zoom button over the simulated camera, and a tap while
        // the last alert closes can land on nothing, so tap again until it shows
        let zoom = app.buttons["1x"]
        for _ in 0 ..< 3 where !zoom.exists {
            if scan.isHittable { scan.tap() }
            _ = zoom.waitForExistence(timeout: 5)
        }

        XCTAssertTrue(zoom.exists, "the QR scanner never opened")
        try qr.show([address])

        let alert = app.alerts["Found Address"]
        XCTAssertTrue(alert.waitForExistence(timeout: 20), "scanning \(address) never showed the Found Address alert")
        return alert
    }

    private func assertMessage(_ alert: XCUIElement, contains expected: String) {
        let message = alert.staticTexts.matching(NSPredicate(format: "label CONTAINS %@", expected)).firstMatch
        XCTAssertTrue(message.waitForExistence(timeout: 5), "expected the alert to say \(expected): \(alert.debugDescription)")
    }

    private func dismiss(_ alert: XCUIElement) {
        alert.buttons["Cancel"].tap()
        XCTAssertTrue(alert.waitForNonExistence(timeout: 10), "Cancel left the alert up")
    }
}
