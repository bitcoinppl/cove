import XCTest

/// Locks and unlocks UTXOs in the real app against cove-fake-node, then checks what the app broadcast
///
/// Needs the fake node on its fixed ports, which `xtask ios-ui` (`just ios-ui-background`) starts for
/// every run, and a freshly erased simulator, which xtask also provides
final class UtxoLockFullLaunchUITests: XCTestCase {
    private static let esploraUrl = "http://127.0.0.1:39001"
    private static let feeUrl = "\(esploraUrl)/api/v1/fees/recommended"

    private var app: XCUIApplication!
    private var node: FakeNodeControl!

    override func setUp() {
        super.setUp()
        continueAfterFailure = false

        app = XCUIApplication()
        node = FakeNodeControl()

        // fee rates come from the fake node on every launch, so sends don't depend on mempool.space
        app.launchEnvironment["COVE_FEE_URL"] = Self.feeUrl
    }

    func testLockedUtxosAreExcludedFromCoinControlAndSends() throws {
        let funded = try fundWallet()
        let coinA = funded[0]
        let coinB = funded[1]
        let coinC = funded[2]

        // 1 and 2: point a fresh install at the fake node on Signet and import the funded wallet
        app.launchEnvironment["COVE_UI_TEST_ESPLORA_URL"] = Self.esploraUrl
        app.launch(expecting: app.staticTexts["Welcome to Cove"])
        app.importTestMnemonic()
        app.assertBalance(sats: 190_000, timeout: 120)

        // 3: lock B from coin control; it can't be selected and Select All skips it
        openManageUtxos()
        assertRows([coinA, coinB, coinC])
        assertLockState(coinB, "unlocked")
        setLockFromContextMenu(coinB, menuItem: "Lock UTXO")
        assertLockState(coinB, "locked")

        assertLockedTapIsRejected(coinB)
        XCTAssertFalse(continueButton.isEnabled, "tapping a locked UTXO must not select it")

        selectAll()
        assertSelectedTotal(sats: 130_000)
        XCTAssertEqual(continueButton.label, "Continue (2)")

        assertLockedTapIsRejected(coinB)
        assertSelectedTotal(sats: 130_000)
        deselectAll()

        // 4: lock C from its receive transaction's details
        setLockFromTransactionDetails(coinC, from: "Lock", to: "Unlock")
        assertLockState(coinC, "locked")
        selectAll()
        assertSelectedTotal(sats: 100_000)
        XCTAssertEqual(continueButton.label, "Continue")
        deselectAll()

        // 5: both locks survive a cold relaunch
        app.terminate()
        app.launchEnvironment.removeValue(forKey: "COVE_UI_TEST_ESPLORA_URL")
        app.launch(expecting: app.element("selectedWallet.balance"))
        openManageUtxos()
        assertLockState(coinA, "unlocked")
        assertLockState(coinB, "locked")
        assertLockState(coinC, "locked")
        app.goBack()

        // 6: a Max send spends only the unlocked coin
        sendMaxFromWallet()

        let maxSend = try waitForBroadcasts(count: 1)[0]
        XCTAssertEqual(maxSend.inputs, [coinA.outpoint], "Max send must spend only the unlocked UTXO")
        XCTAssertTrue(maxSend.outputs.contains { $0.address == XCUIApplication.externalAddress })
        XCTAssertFalse(try node.outspend(coinB).spent)
        XCTAssertFalse(try node.outspend(coinC).spent)

        try node.mine()
        app.assertBalance(sats: 90000, timeout: 60)

        // 7: unlock B from coin control and C from its transaction details
        openManageUtxos()
        assertRows([coinB, coinC])
        setLockFromContextMenu(coinB, menuItem: "Unlock UTXO")
        assertLockState(coinB, "unlocked")
        setLockFromTransactionDetails(coinC, from: "Unlock", to: "Lock")
        assertLockState(coinC, "unlocked")

        selectAll()
        assertSelectedTotal(sats: 90000)
        XCTAssertEqual(continueButton.label, "Continue (2)")

        // 8: a coin control send spends exactly the coins that were unlocked
        continueButton.tap()
        sendToExternalAddressFromCoinControl()

        let coinControlSend = try waitForBroadcasts(count: 2)[1]
        XCTAssertEqual(Set(coinControlSend.inputs), [coinB.outpoint, coinC.outpoint])
        XCTAssertEqual(coinControlSend.inputs.count, 2)
        XCTAssertEqual(try node.outspend(coinB).spendingTxid, coinControlSend.txid)
        XCTAssertEqual(try node.outspend(coinC).spendingTxid, coinControlSend.txid)
    }

    // MARK: - Fixture

    /// Funds external indexes 0, 1 and 2 of the test wallet with A, B and C, each its own UTXO
    private func fundWallet() throws -> [FakeNodeControl.FundedOutput] {
        try node.requireHealthy()
        try node.reset()

        let outputs = try node.fundWallet(mnemonic: XCUIApplication.testMnemonic, sats: [100_000, 60000, 30000])
        try node.mine()

        XCTAssertEqual(outputs.map(\.sats), [100_000, 60000, 30000])
        return outputs
    }

    // MARK: - Coin control

    private var utxoRows: XCUIElementQuery {
        app.descendants(matching: .any).matching(NSPredicate(format: "identifier BEGINSWITH %@", "coinControl.utxo."))
    }

    private var continueButton: XCUIElement {
        app.buttons["coinControl.continue"]
    }

    private func row(_ coin: FakeNodeControl.FundedOutput) -> XCUIElement {
        app.element("coinControl.utxo.\(coin.outpoint)")
    }

    /// Checks coin control lists exactly these coins
    ///
    /// The list can still be adding rows when the first one appears, so a count taken then can
    /// come up short on a slow runner; waiting for each expected row first also names the coin
    /// that is missing when one is
    private func assertRows(_ coins: [FakeNodeControl.FundedOutput]) {
        for coin in coins {
            XCTAssertTrue(row(coin).waitForExistence(timeout: 10), "coin control is missing \(coin.outpoint)")
        }

        XCTAssertEqual(utxoRows.count, coins.count)
    }

    private func openManageUtxos() {
        let more = app.buttons["selectedWallet.more"]
        XCTAssertTrue(more.waitForExistence(timeout: 15))
        more.tap()

        let manageUtxos = app.buttons["selectedWallet.manageUtxos"]
        XCTAssertTrue(manageUtxos.waitForExistence(timeout: 10))
        manageUtxos.tap()

        XCTAssertTrue(app.navigationBars["Manage UTXOs"].waitForExistence(timeout: 15))
        XCTAssertTrue(utxoRows.firstMatch.waitForExistence(timeout: 30))
    }

    private func setLockFromContextMenu(_ coin: FakeNodeControl.FundedOutput, menuItem: String) {
        let row = row(coin)
        XCTAssertTrue(row.waitForExistence(timeout: 10))
        row.press(forDuration: 1.2)

        let action = app.buttons[menuItem]
        XCTAssertTrue(action.waitForExistence(timeout: 5), "missing context menu item '\(menuItem)'")
        action.tap()
    }

    /// Opens the coin's receive transaction from coin control, flips its lock and returns
    private func setLockFromTransactionDetails(_ coin: FakeNodeControl.FundedOutput, from: String, to: String) {
        let row = row(coin)
        XCTAssertTrue(row.waitForExistence(timeout: 10))
        row.press(forDuration: 1.2)

        let viewTransaction = app.buttons["View Transaction Details"]
        XCTAssertTrue(viewTransaction.waitForExistence(timeout: 5))
        viewTransaction.tap()

        // whether details are expanded is remembered per wallet
        let showDetails = app.buttons["transactionDetails.showDetails"]
        XCTAssertTrue(showDetails.waitForExistence(timeout: 15))
        if showDetails.label == "Show Details" {
            showDetails.tap()
        }

        let lockToggle = app.buttons["transactionDetails.lockToggle"]
        XCTAssertTrue(lockToggle.waitForExistence(timeout: 15))
        waitForLabel(lockToggle, equalTo: from, timeout: 15)
        lockToggle.tap()
        waitForLabel(lockToggle, equalTo: to, timeout: 15)

        app.goBack()
        XCTAssertTrue(app.navigationBars["Manage UTXOs"].waitForExistence(timeout: 15))
    }

    private func assertLockState(_ coin: FakeNodeControl.FundedOutput, _ state: String) {
        let row = row(coin)
        XCTAssertTrue(row.waitForExistence(timeout: 10), "missing UTXO row \(coin.outpoint)")
        waitUntil(row, matches: NSPredicate(format: "value == %@", state), timeout: 10, "UTXO \(coin.outpoint) is not \(state)")
    }

    private func assertLockedTapIsRejected(_ coin: FakeNodeControl.FundedOutput) {
        row(coin).tap()

        let alert = app.alerts["UTXO Locked"]
        XCTAssertTrue(alert.waitForExistence(timeout: 5), "tapping a locked UTXO should explain why it can't be selected")
        alert.buttons["OK"].tap()
        XCTAssertTrue(alert.waitForNonExistence(timeout: 5))
    }

    private func selectAll() {
        let selectAll = app.element("coinControl.selectAll")
        XCTAssertTrue(selectAll.waitForExistence(timeout: 10))
        XCTAssertEqual(selectAll.label, "Select All")
        selectAll.tap()
        waitForLabel(selectAll, equalTo: "Deselect All", timeout: 5)
    }

    private func deselectAll() {
        let selectAll = app.element("coinControl.selectAll")
        XCTAssertEqual(selectAll.label, "Deselect All")
        selectAll.tap()
        waitForLabel(selectAll, equalTo: "Select All", timeout: 5)
    }

    private func assertSelectedTotal(sats: UInt64) {
        let total = app.staticTexts["coinControl.selectedTotal"]
        XCTAssertTrue(total.waitForExistence(timeout: 10))
        waitForAmount(total, sats: sats, timeout: 10)
    }

    // MARK: - Sending

    private func sendMaxFromWallet() {
        app.startSend(to: XCUIApplication.externalAddress)
        app.tapMax()
        app.tapNext()
        app.swipeToSend()
    }

    private func sendToExternalAddressFromCoinControl() {
        app.enterAddress(XCUIApplication.externalAddress)
        app.tapNext()
        app.swipeToSend()
    }

    private func waitForBroadcasts(count: Int) throws -> [FakeNodeControl.Broadcast] {
        try node.waitForBroadcasts(count: count)
    }
}
