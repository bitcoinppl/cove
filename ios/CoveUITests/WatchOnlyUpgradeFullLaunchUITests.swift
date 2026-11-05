import XCTest

/// Imports a funded watch-only wallet by scanning its export, labels its funding transaction, then
/// imports the matching recovery words and checks that the same wallet now offers View Secret Words
/// and keeps its name, history and label instead of a second wallet appearing. It doesn't sign a
/// spend
///
/// The QR seam (`COVE_UI_TEST_QR_SCRIPT`, docs/testing.md) stands in for the camera. Needs the fake
/// node and the hardware signer host, which `xtask ios-ui` (`just ios-ui-background`) starts for
/// every run
final class WatchOnlyUpgradeFullLaunchUITests: XCTestCase {
    private static let esploraUrl = "http://127.0.0.1:39001"
    private static let fundedSats: UInt64 = 100_000
    private static let label = "Upgrade fixture deposit"

    /// The default name a recovery-words import gives a new wallet, so a duplicate would match it
    private static let newWalletName = NSPredicate(format: "label MATCHES %@", "Wallet [0-9]+")

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

        app.launchEnvironment["COVE_UI_TEST_ESPLORA_URL"] = Self.esploraUrl
        qr.configure(app)
    }

    override func tearDown() {
        qr.remove()
        super.tearDown()
    }

    func testRecoveryWordsTurnTheWatchOnlyWalletIntoASigningWallet() throws {
        try node.requireHealthy()
        try signer.requireHealthy()
        try node.reset()

        let funding = try node.fundWallet(mnemonic: XCUIApplication.testMnemonic, sats: [Self.fundedSats])
        try node.mine()

        try importWatchOnlyWallet()
        app.assertBalance(sats: Self.fundedSats, timeout: 120)
        try labelTransaction(funding[0].txid)

        let name = walletNameInSidebar()
        XCTAssertFalse(hasSecretWordsInWalletSettings(), "the watch-only wallet already offers recovery words")

        importTestMnemonicFromAddWallet()

        // the import lands on the wallet it upgraded, with the history and label it had
        app.assertBalance(sats: Self.fundedSats, timeout: 60)
        XCTAssertTrue(app.staticTexts[Self.label].waitForExistence(timeout: 15), "the transaction label was lost")

        XCTAssertEqual(walletNameInSidebar(), name, "the upgraded wallet was renamed or a second wallet appeared")
        XCTAssertEqual(app.buttons.matching(Self.newWalletName).count, 0, "the recovery words imported as a new wallet")
        XCTAssertTrue(hasSecretWordsInWalletSettings(), "the wallet still can't sign after importing its recovery words")
    }

    // MARK: - Steps

    /// Imports the test mnemonic's account as a watch-only hardware wallet by scanning its export
    private func importWatchOnlyWallet() throws {
        app.launch(expecting: app.staticTexts["Welcome to Cove"])
        app.openHardwareExportScanner()

        let descriptors = try signer.watchOnlyDescriptors()
        try qr.show(["\(descriptors.external)\n\(descriptors.internal)"])
        app.finishHardwareImport()
    }

    /// Scans a BIP329 label for the transaction from the wallet screen's QR scanner
    private func labelTransaction(_ txid: String) throws {
        let record = try JSONSerialization.data(withJSONObject: ["type": "tx", "ref": txid, "label": Self.label])
        let scan = app.buttons.matching(NSPredicate(format: "label ==[c] %@ OR identifier == %@", "QR Code", "qrcode")).firstMatch
        XCTAssertTrue(scan.waitForExistence(timeout: 10), "the wallet screen's QR scanner button is missing")
        scan.tap()

        try qr.show([String(decoding: record, as: UTF8.self)])

        let imported = app.alerts["Success"]
        XCTAssertTrue(imported.waitForExistence(timeout: 15), "the scanned label was not imported")
        imported.buttons.firstMatch.tap()
        XCTAssertTrue(app.staticTexts[Self.label].waitForExistence(timeout: 15), "the transaction doesn't show its label")
    }

    /// Walks Add Wallet to the 12-word import and enters the test mnemonic
    private func importTestMnemonicFromAddWallet() {
        app.tapButton(named: "app.sidebar.open")
        app.tapButton(named: "Add Wallet")
        app.tapButton(startingWith: "On This Device", until: app.buttons["Import existing wallet"])
        app.tapButton(startingWith: "Import existing wallet", until: app.buttons["12 Words"])
        app.tapButton(startingWith: "12 Words", until: app.navigationBars["Import Wallet"])
        app.enterTestMnemonicAndImport()

        XCTAssertTrue(app.element("selectedWallet.balance").waitForExistence(timeout: 30))
    }

    /// The name of the only wallet the sidebar lists, failing when it lists none or several
    private func walletNameInSidebar() -> String {
        app.tapButton(named: "app.sidebar.open")

        let wallets = app.buttons.matching(NSPredicate(format: "label BEGINSWITH %@", "Imported "))
        XCTAssertTrue(wallets.firstMatch.waitForExistence(timeout: 10), "the sidebar lists no imported wallet")
        XCTAssertEqual(wallets.count, 1, "the sidebar lists \(wallets.count) imported wallets")
        let name = wallets.firstMatch.label

        // tapping the wallet closes the sidebar and keeps it selected
        wallets.firstMatch.tap()
        XCTAssertTrue(app.element("selectedWallet.balance").waitForExistence(timeout: 15))
        return name
    }

    /// Opens the wallet's settings and reports whether its Danger Zone offers the recovery words,
    /// which only a wallet holding them does, then returns to the wallet
    private func hasSecretWordsInWalletSettings() -> Bool {
        app.tapButton(named: "selectedWallet.more")
        app.tapButton(named: "Wallet Settings")

        let delete = app.buttons["Delete Wallet"]
        for _ in 0 ..< 10 where !(delete.exists && delete.isHittable) {
            app.swipeUp(velocity: .slow)
        }

        XCTAssertTrue(delete.isHittable, "wallet settings never showed the Danger Zone")
        let hasSecretWords = app.buttons["View Secret Words"].exists

        app.goBack()
        XCTAssertTrue(app.element("selectedWallet.balance").waitForExistence(timeout: 15))
        return hasSecretWords
    }
}
