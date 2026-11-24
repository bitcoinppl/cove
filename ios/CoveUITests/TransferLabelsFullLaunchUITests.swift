import XCTest

/// Sends between two of the user's wallets in the real app against cove-fake-node and checks
/// that each wallet's transaction list labels the transfer with the other wallet's name
///
/// The first wallet is the funded test mnemonic and the second the empty legal winner phrase,
/// scanned in through the debug QR seam. Needs the fake node on the ports `COVE_UI_TEST_PORT_BASE`
/// names, which `xtask ios-ui` (`just ios-ui-background`) starts for every run
final class TransferLabelsFullLaunchUITests: XCTestCase {
    private static let esploraUrl = UITestHostPorts.esploraUrl

    /// The second mnemonic's first receive address
    private static let otherReceive0 = "tb1qjgx204hxfwuse548jc34fjzg6ffq8pvrqwlex4"

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

        // fee rates come from the fake node on every launch, so sends don't depend on mempool.space
        app.launchEnvironment["COVE_FEE_URL"] = "\(Self.esploraUrl)/api/v1/fees/recommended"
    }

    override func tearDown() {
        qr?.remove()
        super.tearDown()
    }

    /// A Max send from the first wallet to the second shows as "Transfer to" the second in the
    /// first wallet and, once mined, as "Transfer from" the first in the second
    func testSendBetweenWalletsIsLabelledInBoth() throws {
        try node.requireHealthy()
        try node.reset()
        _ = try node.fundWallet(mnemonic: XCUIApplication.testMnemonic, sats: [100_000])
        try node.mine()

        app.launchEnvironment["COVE_UI_TEST_ESPLORA_URL"] = Self.esploraUrl
        app.launch(expecting: app.staticTexts["Welcome to Cove"])
        app.importTestMnemonic()
        app.assertBalance(sats: 100_000, timeout: 120)
        let sender = app.selectedWalletName

        try app.importFromAddWallet(scanning: XCUIApplication.secondMnemonic, with: qr)
        app.assertBalance(sats: 0, timeout: 60)
        let receiver = app.selectedWalletName
        XCTAssertNotEqual(sender, receiver, "both wallets have the same name, so the labels can't tell them apart")

        app.selectWallet(named: sender)
        app.assertBalance(sats: 100_000, timeout: 60)
        app.startSend(to: Self.otherReceive0)
        app.tapMax()
        app.tapNext()
        app.swipeToSend()

        let send = try node.waitForBroadcasts(count: 1)[0]
        let received = try XCTUnwrap(send.outputs.first { $0.address == Self.otherReceive0 }, "the send didn't pay the second wallet")
        assertTransactionRow(labelled: "Transfer to \(receiver)")

        try node.mine()
        app.selectWallet(named: receiver)
        refreshUntilBalance(sats: received.sats)
        assertTransactionRow(labelled: "Transfer from \(sender)")
    }

    // MARK: - Steps

    /// A wallet only looks for new transactions now and then, so pull the list down until the
    /// transfer arrives
    private func refreshUntilBalance(sats: UInt64) {
        let balance = app.element("selectedWallet.balance")
        let deadline = Date().addingTimeInterval(90)
        while Date() < deadline {
            if balance.exists, satsFromAmountLabel(balance.label) == sats { return }

            let window = app.windows.firstMatch
            let top = window.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.35))
            let bottom = window.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.85))
            top.press(forDuration: 0.1, thenDragTo: bottom)
            Thread.sleep(forTimeInterval: 3)
        }

        app.assertBalance(sats: sats, timeout: 5)
    }

    private func assertTransactionRow(labelled label: String) {
        let row = app.descendants(matching: .any).matching(NSPredicate(format: "label CONTAINS %@", label)).firstMatch
        XCTAssertTrue(row.waitForExistence(timeout: 30), "the transaction list never showed \(label)")
    }
}
