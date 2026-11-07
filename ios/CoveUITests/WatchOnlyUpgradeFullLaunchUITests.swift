import XCTest

/// Imports a funded watch-only wallet by scanning its export, then gives it its recovery words,
/// typed in Add Wallet or received by KeyTeleport, and checks the same wallet becomes a signing
/// wallet: it keeps its name, history and label, offers View Secret Words, no second wallet
/// appears, and a Max send from it is broadcast
///
/// The fake node checks every input's script before it logs a broadcast, so a logged spend of the
/// funded output proves the upgraded wallet signed it. The QR seam (`COVE_UI_TEST_QR_SCRIPT`,
/// docs/testing.md) stands in for the camera, and the UI test host plays the sending phone for
/// KeyTeleport. Needs the fake node and the hardware signer host, which `xtask ios-ui`
/// (`just ios-ui-background`) starts for every run
final class WatchOnlyUpgradeFullLaunchUITests: XCTestCase {
    private static let esploraUrl = UITestHostPorts.esploraUrl
    private static let feeUrl = "\(esploraUrl)/api/v1/fees/recommended"
    private static let fundedSats: UInt64 = 100_000
    private static let label = "Upgrade fixture deposit"

    /// The default names a recovery-words import and a KeyTeleport import give a new wallet, so a
    /// duplicate would match one of them
    private static let newWalletName = NSPredicate(
        format: "label MATCHES %@ OR label BEGINSWITH %@",
        "Wallet [0-9]+",
        "KeyTeleport "
    )

    private var app: XCUIApplication!
    private var node: FakeNodeControl!
    private var signer: HardwareSignerControl!
    private var peer: KeyTeleportPeer!
    private var qr: QrScript!
    private var layout: ScreenReachability!

    override func setUp() {
        super.setUp()
        continueAfterFailure = false

        app = XCUIApplication()
        node = FakeNodeControl()
        signer = HardwareSignerControl()
        peer = KeyTeleportPeer()
        qr = QrScript()
        layout = ScreenReachability(app: app)

        app.launchEnvironment["COVE_UI_TEST_ESPLORA_URL"] = Self.esploraUrl

        // fee rates come from the fake node, so the send doesn't depend on mempool.space
        app.launchEnvironment["COVE_FEE_URL"] = Self.feeUrl
        qr.configure(app)
    }

    override func tearDown() {
        qr.remove()
        super.tearDown()
    }

    func testRecoveryWordsTurnTheWatchOnlyWalletIntoASigningWallet() throws {
        let funding = try fundAndImportWatchOnlyWallet()
        try labelTransaction(funding.txid)
        let name = assertWatchOnly()

        app.importTestMnemonicFromAddWallet()
        assertKeepsHistoryAndLabel()

        assertUpgraded(name)
        try assertSendsMax(spending: funding)
    }

    func testKeyTeleportWordsTurnTheWatchOnlyWalletIntoASigningWallet() throws {
        let funding = try fundAndImportWatchOnlyWallet()
        try labelTransaction(funding.txid)
        let name = assertWatchOnly()

        openKeyTeleportReceive()
        try receive(.mnemonic(XCUIApplication.testMnemonic))
        XCTAssertTrue(app.staticTexts["Recovery words received"].waitForExistence(timeout: 30), "the words review never showed")
        layout.reachAndTap(button("Import Wallet"), "mnemonic review: Import Wallet")

        // the result names the wallet the words upgraded, not a new one
        XCTAssertTrue(app.staticTexts["Wallet imported"].waitForExistence(timeout: 30), "Import Wallet never finished")
        XCTAssertTrue(app.staticTexts["\(name) is ready to use in Cove."].exists, "the words didn't land on '\(name)'")
        layout.reachAndTap(button("Done"), "imported: Done")
        assertKeepsHistoryAndLabel()

        assertUpgraded(name)
        try assertSendsMax(spending: funding)
    }

    // MARK: - Steps

    /// Funds the test mnemonic's first address, imports its account as a watch-only hardware
    /// wallet by scanning its export, and waits for the funds to show
    private func fundAndImportWatchOnlyWallet() throws -> FakeNodeControl.FundedOutput {
        try node.requireHealthy()
        try signer.requireHealthy()
        try node.reset()

        let funding = try node.fundWallet(mnemonic: XCUIApplication.testMnemonic, sats: [Self.fundedSats])
        try node.mine()

        app.launch(expecting: app.staticTexts["Welcome to Cove"])
        app.openHardwareExportScanner()

        let descriptors = try signer.watchOnlyDescriptors()
        try qr.show(["\(descriptors.external)\n\(descriptors.internal)"])
        app.finishHardwareImport()
        app.assertBalance(sats: Self.fundedSats, timeout: 120)

        return funding[0]
    }

    /// Checks the wallet can't sign yet and returns its name
    private func assertWatchOnly() -> String {
        let name = walletNameInSidebar()
        XCTAssertFalse(hasSecretWordsInWalletSettings(), "the watch-only wallet already offers recovery words")
        return name
    }

    /// Checks the import landed on the wallet it upgraded, with the balance, history and label it had
    private func assertKeepsHistoryAndLabel() {
        app.assertBalance(sats: Self.fundedSats, timeout: 60)
        XCTAssertTrue(app.staticTexts[Self.label].waitForExistence(timeout: 15), "the transaction label was lost")
    }

    /// Checks the wallet the words reached is the one that was watched, now holding its words
    private func assertUpgraded(_ name: String) {
        XCTAssertEqual(walletNameInSidebar(), name, "the upgraded wallet was renamed or a second wallet appeared")
        XCTAssertTrue(hasSecretWordsInWalletSettings(), "the wallet still can't sign after receiving its recovery words")
    }

    /// Sends the whole balance to an outside address and checks the fake node accepted a spend of
    /// the funded output to it, which it does only with a valid signature
    private func assertSendsMax(spending funding: FakeNodeControl.FundedOutput) throws {
        app.startSend(to: XCUIApplication.externalAddress)
        app.tapMax()
        app.tapNext()
        app.swipeToSend()

        let sent = try node.waitForBroadcasts(count: 1)[0]
        XCTAssertEqual(sent.inputs, [funding.outpoint], "the send must spend the funded output")
        XCTAssertEqual(sent.outputs.map(\.address), [XCUIApplication.externalAddress], "Max must send everything to the address")
        XCTAssertTrue(try node.outspend(funding).spent, "the node doesn't show the funded output spent")
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

    // MARK: - KeyTeleport

    /// Opens Receive by KeyTeleport from Add Wallet
    private func openKeyTeleportReceive() {
        app.tapButton(named: "app.sidebar.open")
        app.tapButton(named: "Add Wallet")

        let moreOptions = app.buttons["More wallet import options"]
        XCTAssertTrue(moreOptions.waitForExistence(timeout: 10))
        moreOptions.tap()
        app.tapButton(named: "Key Teleport")
        XCTAssertTrue(button("Tap to show QR code").firstMatch.waitForExistence(timeout: 15), "the receive request never showed")
    }

    /// Reads the request off the screen as the sending phone's camera would, has the host answer
    /// it with the payload, scans the answer and unlocks it with the password
    private func receive(_ payload: KeyTeleportPeer.Payload) throws {
        // the KeyTeleportReceiveFullLaunchUITests suite checks the request's concealment; this test
        // only needs its contents, so it reads the code from the UI tree instead of with Vision
        layout.reachAndTap(button("Tap to show QR code"), "receive request: show QR code")
        let request = try XCTUnwrap(app.waitForVisibleQrCode(), "the receive request QR code never decoded")

        layout.reachAndTap(button("Tap to show receiver code"), "receive request: show receiver code")
        let code = app.staticTexts.matching(NSPredicate(format: "label MATCHES %@", "^[0-9]{2}( [0-9]{2}){3}$")).firstMatch
        XCTAssertTrue(code.waitForExistence(timeout: 5), "the receiver code never showed")

        let response = try peer.send(payload, to: request, code: code.label)
        layout.reachAndTap(button("Scan Sender Response"), "receive request: Scan Sender Response")
        try qr.show(response.parts)

        let field = app.secureTextFields.matching(identifier: "Teleport Password")
        XCTAssertTrue(field.firstMatch.waitForExistence(timeout: 20), "the scanned response never asked for the password")
        layout.reachAndTap(field, "password: field").typeText(response.password)
        layout.reachAndTap(button("Unlock"), "password: Unlock")
    }

    // MARK: - Queries

    /// The name of the only wallet the sidebar lists, failing when it lists none or several
    private func walletNameInSidebar() -> String {
        app.tapButton(named: "app.sidebar.open")

        let wallets = app.buttons.matching(NSPredicate(format: "label BEGINSWITH %@", "Imported "))
        XCTAssertTrue(wallets.firstMatch.waitForExistence(timeout: 10), "the sidebar lists no imported wallet")
        XCTAssertEqual(wallets.count, 1, "the sidebar lists \(wallets.count) imported wallets")
        XCTAssertEqual(app.buttons.matching(Self.newWalletName).count, 0, "the recovery words imported as a new wallet")
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

    private func button(_ label: String) -> XCUIElementQuery {
        app.buttons.matching(NSPredicate(format: "label == %@", label))
    }
}
