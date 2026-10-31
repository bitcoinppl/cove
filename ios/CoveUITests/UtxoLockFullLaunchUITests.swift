import XCTest

/// Locks and unlocks UTXOs in the real app against cove-fake-node, then checks what the app broadcast
///
/// Needs the fake node on its fixed ports, which `xtask ios-ui` (`just ios-ui-background`) starts for
/// every run, and a freshly erased simulator, which xtask also provides
final class UtxoLockFullLaunchUITests: XCTestCase {
    private static let esploraUrl = "http://127.0.0.1:39001"
    private static let feeUrl = "\(esploraUrl)/api/v1/fees/recommended"
    private static let mnemonic = (Array(repeating: "abandon", count: 11) + ["about"]).joined(separator: " ")

    /// BIP173 test vector: the P2WPKH address of private key 1, which is not in the test wallet
    private static let externalAddress = "tb1qw508d6qejxtdg4y5r3zarvary0c5xw7kxpjzsx"

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
        launchApp(expecting: app.staticTexts["Welcome to Cove"])
        importFundedWallet()
        assertBalance(sats: 190_000, timeout: 120)

        // 3: lock B from coin control; it can't be selected and Select All skips it
        openManageUtxos()
        XCTAssertEqual(utxoRows.count, 3)
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
        launchApp(expecting: element("selectedWallet.balance"))
        openManageUtxos()
        assertLockState(coinA, "unlocked")
        assertLockState(coinB, "locked")
        assertLockState(coinC, "locked")
        goBack()

        // 6: a Max send spends only the unlocked coin
        sendMaxFromWallet()

        let maxSend = try waitForBroadcasts(count: 1)[0]
        XCTAssertEqual(maxSend.inputs, [coinA.outpoint], "Max send must spend only the unlocked UTXO")
        XCTAssertTrue(maxSend.outputs.contains { $0.address == Self.externalAddress })
        XCTAssertFalse(try node.outspend(coinB).spent)
        XCTAssertFalse(try node.outspend(coinC).spent)

        try node.mine()
        assertBalance(sats: 90000, timeout: 60)

        // 7: unlock B from coin control and C from its transaction details
        openManageUtxos()
        XCTAssertEqual(utxoRows.count, 2)
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

        let outputs = try node.fundWallet(mnemonic: Self.mnemonic, sats: [100_000, 60000, 30000])
        try node.mine()

        XCTAssertEqual(outputs.map(\.sats), [100_000, 60000, 30000])
        return outputs
    }

    // MARK: - Launch and onboarding

    /// Launches the app, relaunching once if the bootstrap watchdog fires on a cold simulator
    private func launchApp(expecting readyElement: XCUIElement) {
        app.launch()

        guard waitForReadyOrStartupTimeout(readyElement) == .startupTimedOut else { return }

        app.terminate()
        app.launch()

        // xtask retries the test on a clean simulator when it sees this message
        if waitForReadyOrStartupTimeout(readyElement) == .startupTimedOut {
            XCTFail("App startup timed out on relaunch")
        }
    }

    private enum LaunchOutcome {
        case ready
        case startupTimedOut
        case unknown
    }

    private func waitForReadyOrStartupTimeout(_ readyElement: XCUIElement) -> LaunchOutcome {
        let startupTimedOut = staticText(containing: "App startup timed out")

        // the watchdog allows up to 20 seconds while a migration runs
        let deadline = Date().addingTimeInterval(30)
        while Date() < deadline {
            if readyElement.exists { return .ready }
            if startupTimedOut.exists { return .startupTimedOut }

            Thread.sleep(forTimeInterval: 0.5)
        }

        return .unknown
    }

    private func importFundedWallet() {
        tapButton(named: "Get Started")
        acceptTermsIfNeeded()
        tapButton(startingWith: "Yes, I have Bitcoin")

        let useAnotherWallet = button(startingWith: "Use another wallet")
        if useAnotherWallet.waitForExistence(timeout: 2) {
            useAnotherWallet.tap()
        }

        tapButton(startingWith: "Software wallet", until: button(startingWith: "Enter recovery words"))
        tapButton(startingWith: "Enter recovery words", until: button(startingWith: "12 words"))
        tapButton(startingWith: "12 words", until: app.navigationBars["Import Wallet"])

        for (index, word) in Self.mnemonic.split(separator: " ").enumerated() {
            let field = app.textFields["hotWalletImport.word.\(index + 1)"]
            XCTAssertTrue(field.waitForExistence(timeout: 10))
            field.tap()
            field.typeText(word == "about" ? "abou" : "aba")
        }

        app.buttons["hotWalletImport.import"].tap()

        XCTAssertTrue(app.staticTexts["Protect this wallet with Cloud Backup?"].waitForExistence(timeout: 20))
        app.buttons["Not Now"].tap()
        acceptTermsIfNeeded()

        // only native segwit is funded, but keep it if other address types are offered anyway
        let keepCurrent = button(startingWith: "Keep Current")
        if keepCurrent.waitForExistence(timeout: 5) {
            keepCurrent.tap()
        }

        XCTAssertTrue(element("selectedWallet.balance").waitForExistence(timeout: 30))
    }

    private func acceptTermsIfNeeded() {
        guard app.staticTexts["Terms & Conditions"].waitForExistence(timeout: 3) else { return }

        [
            "onboarding.terms.check.backup",
            "onboarding.terms.check.legal",
            "onboarding.terms.check.financial",
            "onboarding.terms.check.recovery",
            "onboarding.terms.check.agreement",
        ].forEach { app.buttons[$0].tap() }

        app.buttons["onboarding.terms.agree"].tap()
    }

    // MARK: - Coin control

    private var utxoRows: XCUIElementQuery {
        app.descendants(matching: .any).matching(NSPredicate(format: "identifier BEGINSWITH %@", "coinControl.utxo."))
    }

    private var continueButton: XCUIElement {
        app.buttons["coinControl.continue"]
    }

    private func row(_ coin: FakeNodeControl.FundedOutput) -> XCUIElement {
        element("coinControl.utxo.\(coin.outpoint)")
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

        goBack()
        XCTAssertTrue(app.navigationBars["Manage UTXOs"].waitForExistence(timeout: 15))
    }

    private func assertLockState(_ coin: FakeNodeControl.FundedOutput, _ state: String) {
        let row = row(coin)
        XCTAssertTrue(row.waitForExistence(timeout: 10), "missing UTXO row \(coin.outpoint)")
        wait(for: row, predicate: NSPredicate(format: "value == %@", state), timeout: 10, "UTXO \(coin.outpoint) is not \(state)")
    }

    private func assertLockedTapIsRejected(_ coin: FakeNodeControl.FundedOutput) {
        row(coin).tap()

        let alert = app.alerts["UTXO Locked"]
        XCTAssertTrue(alert.waitForExistence(timeout: 5), "tapping a locked UTXO should explain why it can't be selected")
        alert.buttons["OK"].tap()
        XCTAssertTrue(alert.waitForNonExistence(timeout: 5))
    }

    private func selectAll() {
        let selectAll = element("coinControl.selectAll")
        XCTAssertTrue(selectAll.waitForExistence(timeout: 10))
        XCTAssertEqual(selectAll.label, "Select All")
        selectAll.tap()
        waitForLabel(selectAll, equalTo: "Deselect All", timeout: 5)
    }

    private func deselectAll() {
        let selectAll = element("coinControl.selectAll")
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
        let send = app.buttons.matching(NSPredicate(format: "label == 'Send' OR label ENDSWITH ', Send'")).firstMatch
        XCTAssertTrue(send.waitForExistence(timeout: 10))
        send.tap()

        enterExternalAddress()

        let amount = app.textFields["sendFlow.amount"]
        XCTAssertTrue(amount.waitForExistence(timeout: 10))
        amount.tap()

        let max = app.buttons["sendFlow.max"]
        XCTAssertTrue(max.waitForExistence(timeout: 10))
        max.tap()

        confirmAndSwipeToSend()
    }

    private func sendToExternalAddressFromCoinControl() {
        enterExternalAddress()
        confirmAndSwipeToSend()
    }

    private func enterExternalAddress() {
        let address = app.textViews["sendFlow.address"]
        XCTAssertTrue(address.waitForExistence(timeout: 15))
        address.tap()
        address.typeText(Self.externalAddress)
    }

    private func confirmAndSwipeToSend() {
        // Next shows once the address is valid and fee rates have loaded; both screens drop the
        // keyboard that covers it once the address and amount are valid, Max included
        let next = app.buttons["sendFlow.next"]
        XCTAssertTrue(next.waitForExistence(timeout: 60))
        wait(for: next, predicate: NSPredicate(format: "hittable == true"), timeout: 10, "Next is covered")
        next.tap()

        let swipe = element("sendFlowConfirm.swipeToSend")
        XCTAssertTrue(swipe.waitForExistence(timeout: 30), "the send didn't reach the confirm screen")

        // drag the handle from the leading edge all the way across
        let frame = swipe.frame
        let start = swipe.coordinate(withNormalizedOffset: CGVector(dx: frame.height / 2 / frame.width, dy: 0.5))
        let end = swipe.coordinate(withNormalizedOffset: CGVector(dx: 0.99, dy: 0.5))
        start.press(forDuration: 0.2, thenDragTo: end, withVelocity: .slow, thenHoldForDuration: 0.2)

        let sent = app.alerts["Sent!"]
        XCTAssertTrue(sent.waitForExistence(timeout: 30), "the send was not broadcast")
        sent.buttons["OK"].tap()

        XCTAssertTrue(element("selectedWallet.balance").waitForExistence(timeout: 15))
    }

    private func waitForBroadcasts(count: Int) throws -> [FakeNodeControl.Broadcast] {
        let deadline = Date().addingTimeInterval(15)
        var broadcasts = try node.broadcasts()
        while broadcasts.count < count, Date() < deadline {
            Thread.sleep(forTimeInterval: 0.5)
            broadcasts = try node.broadcasts()
        }

        XCTAssertEqual(broadcasts.count, count, "unexpected broadcasts: \(broadcasts)")
        return broadcasts
    }

    // MARK: - Amounts

    private func assertBalance(sats: UInt64, timeout: TimeInterval) {
        let balance = element("selectedWallet.balance")
        XCTAssertTrue(balance.waitForExistence(timeout: 15))
        waitForAmount(balance, sats: sats, timeout: timeout)
    }

    /// Waits for the element to show the amount, in whichever unit the wallet displays
    private func waitForAmount(_ element: XCUIElement, sats: UInt64, timeout: TimeInterval) {
        let deadline = Date().addingTimeInterval(timeout)
        while Date() < deadline {
            if element.exists, Self.sats(fromAmountLabel: element.label) == sats { return }

            Thread.sleep(forTimeInterval: 0.5)
        }

        let current = element.exists ? "'\(element.label)'" : "missing"
        XCTFail("expected \(sats) sats, got \(current)")
    }

    /// Parses an amount label such as `190,000 SATS` or `0.0019 BTC` into sats
    private static func sats(fromAmountLabel label: String) -> UInt64? {
        let parts = label.replacingOccurrences(of: ",", with: "").split(separator: " ")
        guard parts.count == 2, let value = Decimal(string: String(parts[0]), locale: Locale(identifier: "en_US_POSIX")) else {
            return nil
        }

        let sats: Decimal
        switch parts[1] {
        case "SATS": sats = value
        case "BTC": sats = value * 100_000_000
        default: return nil
        }

        let whole = NSDecimalNumber(decimal: sats).uint64Value
        return Decimal(whole) == sats ? whole : nil
    }

    // MARK: - Queries and waiting

    private func element(_ identifier: String) -> XCUIElement {
        app.descendants(matching: .any)[identifier].firstMatch
    }

    private func button(startingWith labelPrefix: String) -> XCUIElement {
        app.buttons.matching(NSPredicate(format: "label BEGINSWITH %@", labelPrefix)).firstMatch
    }

    private func staticText(containing text: String) -> XCUIElement {
        app.staticTexts.matching(NSPredicate(format: "label CONTAINS %@", text)).firstMatch
    }

    private func tapButton(named name: String) {
        let button = app.buttons[name]
        XCTAssertTrue(button.waitForExistence(timeout: 10), "Timed out waiting for button '\(name)'")
        button.tap()
    }

    private func tapButton(startingWith labelPrefix: String) {
        let button = button(startingWith: labelPrefix)
        XCTAssertTrue(button.waitForExistence(timeout: 10), "Timed out waiting for button starting with '\(labelPrefix)'")
        button.tap()
    }

    /// On CI a tap can land while the screen settles or a system banner passes over it and do
    /// nothing, so tap again while the button is still showing
    private func tapButton(startingWith labelPrefix: String, until target: XCUIElement) {
        let button = button(startingWith: labelPrefix)
        XCTAssertTrue(button.waitForExistence(timeout: 10), "Timed out waiting for button starting with '\(labelPrefix)'")

        for _ in 0 ..< 3 {
            if button.exists, button.isHittable {
                button.tap()
            }

            if target.waitForExistence(timeout: 5) { return }
        }

        XCTFail("Tapping the button starting with '\(labelPrefix)' never showed \(target)")
    }

    private func goBack() {
        app.navigationBars.buttons.element(boundBy: 0).tap()
    }

    private func waitForLabel(_ element: XCUIElement, equalTo label: String, timeout: TimeInterval) {
        wait(for: element, predicate: NSPredicate(format: "label == %@", label), timeout: timeout, "expected label '\(label)'")
    }

    private func wait(for element: XCUIElement, predicate: NSPredicate, timeout: TimeInterval, _ message: String) {
        let expectation = XCTNSPredicateExpectation(predicate: predicate, object: element)
        let result = XCTWaiter().wait(for: [expectation], timeout: timeout)
        let current = element.exists ? "label '\(element.label)', value '\(element.value ?? "nil")'" : "missing"
        XCTAssertEqual(result, .completed, "\(message), got \(current)")
    }
}

/// Synchronous client for the cove-fake-node control API, see rust/crates/cove-fake-node/README.md
private struct FakeNodeControl {
    struct FundedOutput: Decodable {
        let txid: String
        let vout: UInt32
        let sats: UInt64

        var outpoint: String {
            "\(txid):\(vout)"
        }
    }

    struct Broadcast: Decodable {
        struct Output: Decodable {
            let address: String?
            let sats: UInt64
        }

        let txid: String
        let inputs: [String]
        let outputs: [Output]
    }

    struct Outspend: Decodable {
        let spent: Bool
        let spendingTxid: String?

        enum CodingKeys: String, CodingKey {
            case spent
            case spendingTxid = "spending_txid"
        }
    }

    private struct Funded: Decodable {
        let outputs: [FundedOutput]
    }

    private struct ErrorBody: Decodable {
        let error: String
    }

    struct RequestError: Error, CustomStringConvertible {
        let description: String
    }

    /// the simulator shares the host's loopback, where xtask runs the node
    private let baseUrl = URL(string: "http://127.0.0.1:39000")!

    func requireHealthy() throws {
        do {
            _ = try request("GET", "health")
        } catch {
            throw RequestError(
                description: "cove-fake-node is not reachable at \(baseUrl), run this test through `just ios-ui-background`: \(error)"
            )
        }
    }

    func reset() throws {
        _ = try request("POST", "reset")
    }

    func fundWallet(mnemonic: String, sats: [UInt64]) throws -> [FundedOutput] {
        let outputs = sats.enumerated().map { index, sats in ["index": index, "sats": sats] as [String: Any] }
        let body: [String: Any] = ["mnemonic": mnemonic, "outputs": outputs, "confirm": true]

        return try decode(Funded.self, request("POST", "fund-wallet", body: body)).outputs
    }

    func mine() throws {
        _ = try request("POST", "mine", body: ["blocks": 1])
    }

    func broadcasts() throws -> [Broadcast] {
        try decode([Broadcast].self, request("GET", "broadcasts"))
    }

    func outspend(_ output: FundedOutput) throws -> Outspend {
        try decode(Outspend.self, request("GET", "outspend/\(output.txid)/\(output.vout)"))
    }

    private func decode<T: Decodable>(_ type: T.Type, _ data: Data) throws -> T {
        try JSONDecoder().decode(type, from: data)
    }

    private func request(_ method: String, _ path: String, body: [String: Any]? = nil) throws -> Data {
        var request = URLRequest(url: baseUrl.appendingPathComponent(path), timeoutInterval: 30)
        request.httpMethod = method

        if let body {
            request.httpBody = try JSONSerialization.data(withJSONObject: body)
            request.setValue("application/json", forHTTPHeaderField: "Content-Type")
        }

        let response = Response()
        let done = DispatchSemaphore(value: 0)
        URLSession.shared.dataTask(with: request) { data, urlResponse, error in
            response.data = data
            response.status = (urlResponse as? HTTPURLResponse)?.statusCode
            response.error = error
            done.signal()
        }.resume()
        done.wait()

        if let error = response.error { throw error }

        let data = response.data ?? Data()
        guard response.status == 200 else {
            let message = (try? decode(ErrorBody.self, data))?.error ?? String(decoding: data, as: UTF8.self)
            throw RequestError(description: "\(method) /\(path) returned \(response.status ?? 0): \(message)")
        }

        return data
    }

    private final class Response: @unchecked Sendable {
        var data: Data?
        var status: Int?
        var error: Error?
    }
}
