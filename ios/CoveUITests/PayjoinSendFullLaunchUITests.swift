import XCTest

/// Pays a BIP 77 receiver's pj URI from the real send flow, with a real Payjoin session against the
/// directory, OHTTP relay and receiver the UI test host runs, then checks what the app broadcast
///
/// The host's relay reaches the app through the debug-only `COVE_UI_TEST_PAYJOIN_RELAY` seam, and
/// the receiver's coin and the wallet's are on cove-fake-node. `xtask ios-ui`
/// (`just ios-ui-background`) starts the host and the node for every run, on a freshly erased
/// simulator
final class PayjoinSendFullLaunchUITests: XCTestCase {
    private static let esploraUrl = UITestHostPorts.esploraUrl
    private static let walletSats: UInt64 = 100_000
    private static let paymentSats: UInt64 = 20000

    /// the session waits ten minutes, so a fresh countdown reads 10:00 or a few seconds under it
    private static let freshCountdown = #"(10:00|9:[0-5]\d) left"#

    private var app: XCUIApplication!
    private var node: FakeNodeControl!
    private let receiver = PayjoinReceiverControl()

    override func setUp() {
        super.setUp()
        continueAfterFailure = false

        app = XCUIApplication()
        node = FakeNodeControl()

        app.launchEnvironment["COVE_FEE_URL"] = "\(Self.esploraUrl)/api/v1/fees/recommended"
        app.launchEnvironment["COVE_UI_TEST_PAYJOIN_RELAY"] = UITestHostPorts.payjoinRelay
    }

    func testReceiverJoinsAPaymentToItsPjUri() throws {
        let walletCoin = try importFundedWallet()
        let session = try openConfirmScreen()

        // with fee help on, the screen names the most the receiver may take and the most the
        // payment may cost
        XCTAssertEqual(payjoinSwitch.value as? String, "1", "the Payjoin switch must start on")
        XCTAssertTrue(feeHelpLine.waitForExistence(timeout: 10), "a payment offering fee help shows no fee help line")
        let confirm = try readConfirmScreen()
        let feeHelp = try XCTUnwrap(confirm.feeHelp, "unreadable fee help line '\(feeHelpLine.label)'")
        XCTAssertGreaterThan(feeHelp, 0, "the payment offers no fee help: '\(feeHelpLine.label)'")
        XCTAssertTrue(confirm.totalLabel.hasPrefix("up to "), "the total must read as a maximum: '\(confirm.totalLabel)'")
        XCTAssertEqual(
            confirm.total,
            Self.paymentSats + confirm.fee + feeHelp,
            "the total '\(confirm.totalLabel)' must be the payment, the fee \(confirm.fee) and the fee help \(feeHelp)"
        )

        swipeAndWaitForReceiver()

        // nothing goes out while the wallet waits for the receiver
        XCTAssertEqual(try node.broadcasts().count, 0, "the payment was broadcast before the receiver answered")

        // with the proposal's broadcast held, the card is gone and nothing claims the payment went out
        try node.holdBroadcasts()
        try receiver.respond()
        try node.waitForHeldBroadcast()
        XCTAssertTrue(waitingCard.waitForNonExistence(timeout: 30), "the card stayed while the proposal went out")
        XCTAssertFalse(app.alerts["Sent!"].exists, "the payment was reported sent before its broadcast answered")
        try node.releaseBroadcasts()

        let sent = app.alerts["Sent!"]
        XCTAssertTrue(sent.waitForExistence(timeout: 60), "the receiver's proposal was not broadcast")
        let message = sentMessage(sent, startingWith: "Sent with Payjoin.")
        sent.buttons["OK"].tap()

        // the one broadcast is the receiver's proposal, spending its coin alongside the wallet's
        let proposal = try node.waitForBroadcasts(count: 1)[0]
        XCTAssertEqual(try node.broadcastRequests().count, 1, "the proposal was broadcast more than once")
        XCTAssertEqual(Set(proposal.inputs), [walletCoin.outpoint, session.coin])
        XCTAssertTrue(
            proposal.outputs.contains { $0.address == session.address },
            "the proposal must still pay the receiver: \(proposal.outputs)"
        )

        // the alert names what the wallet spent beyond the payment, and the part of it the
        // receiver took from the change for its coin, which can't pass what the payment offered
        let joined = #/Sent with Payjoin\. You paid (.+) in fees, including (.+) toward the receiver's coin\./#
        let match = try XCTUnwrap(message.wholeMatch(of: joined), "unexpected message for a joined payment: '\(message)'")
        let feePaid = try onlyAmount(in: String(match.1))
        let feeHelpTaken = try onlyAmount(in: String(match.2))
        let spent = try walletSpend(walletCoin, broadcast: proposal, paying: session)
        XCTAssertEqual(feePaid, spent, "the alert's fee in '\(message)' must be what the proposal \(proposal.outputs) cost the wallet")
        XCTAssertEqual(
            feeHelpTaken + confirm.fee,
            feePaid,
            "the fee help in '\(message)' must be the fee paid beyond the confirm screen's \(confirm.fee)"
        )
        XCTAssertTrue(
            (1 ... feeHelp).contains(feeHelpTaken),
            "the receiver took \(feeHelpTaken) where the payment offered up to \(feeHelp)"
        )

        // the payment's details show the fee it really paid, not the original's
        XCTAssertTrue(app.element("selectedWallet.balance").waitForExistence(timeout: 15))
        assertSentDetailsFee(feePaid)
    }

    func testSendWithoutPayjoinBroadcastsTheOriginalOnce() throws {
        let walletCoin = try importFundedWallet()
        let session = try openConfirmScreen()
        let confirm = try readConfirmScreen()
        swipeAndWaitForReceiver()

        // with the fallback's broadcast held, the card is gone and nothing claims the payment went out
        try node.holdBroadcasts()
        app.buttons["Send without Payjoin"].tap()
        try node.waitForHeldBroadcast()
        XCTAssertTrue(waitingCard.waitForNonExistence(timeout: 30), "the card stayed while the fallback went out")
        XCTAssertFalse(app.alerts["Sent!"].exists, "the payment was reported sent before its broadcast answered")
        try node.releaseBroadcasts()

        let sent = app.alerts["Sent!"]
        XCTAssertTrue(sent.waitForExistence(timeout: 60), "cancelling the wait did not send the payment")
        let message = sentMessage(sent, startingWith: "Sent without Payjoin.")
        sent.buttons["OK"].tap()

        // exactly one broadcast, the wallet's own payment as built
        let original = try node.waitForBroadcasts(count: 1)[0]
        XCTAssertEqual(try node.broadcastRequests().count, 1, "the fallback was broadcast more than once")
        XCTAssertEqual(original.inputs, [walletCoin.outpoint], "the fallback must spend only the wallet's coin")
        XCTAssertTrue(
            original.outputs.contains { $0.address == session.address && $0.sats == Self.paymentSats },
            "the fallback must pay the receiver the requested amount: \(original.outputs)"
        )

        // the fallback pays exactly the fee the confirm screen showed
        let fallback = #/Sent without Payjoin\. You paid (.+) in fees\./#
        let match = try XCTUnwrap(message.wholeMatch(of: fallback), "unexpected message for a fallback: '\(message)'")
        let feePaid = try onlyAmount(in: String(match.1))
        XCTAssertEqual(feePaid, confirm.fee, "the alert '\(message)' must name the confirm screen's fee \(confirm.fee)")
        XCTAssertEqual(
            try walletSpend(walletCoin, broadcast: original, paying: session),
            confirm.fee,
            "the original \(original.outputs) must cost the confirm screen's fee \(confirm.fee)"
        )
    }

    func testPayjoinSwitchOffSendsANormalPayment() throws {
        let walletCoin = try importFundedWallet()
        let session = try openConfirmScreen()
        let withPayjoin = try readConfirmScreen()
        XCTAssertNotNil(withPayjoin.feeHelp, "a payment offering fee help shows no fee help line")

        // switched off, the payment offers nothing and costs exactly the payment and its fee
        turnPayjoinOff()
        XCTAssertTrue(feeHelpLine.waitForNonExistence(timeout: 10), "the fee help line stayed with Payjoin off: '\(feeHelpLine.label)'")
        let confirm = try readConfirmScreen()
        XCTAssertFalse(confirm.totalLabel.contains("up to"), "the total must be exact with Payjoin off: '\(confirm.totalLabel)'")
        XCTAssertEqual(
            confirm.total,
            Self.paymentSats + confirm.fee,
            "the total '\(confirm.totalLabel)' must be the payment and the fee \(confirm.fee)"
        )

        // the payment goes out at once, with no wait for the receiver
        app.swipeToConfirm()
        let sent = app.alerts["Sent!"]
        let deadline = Date().addingTimeInterval(60)
        while !sent.exists, Date() < deadline {
            XCTAssertFalse(waitingCard.exists, "a payment sent with Payjoin off waited for the receiver")
            Thread.sleep(forTimeInterval: 0.25)
        }

        XCTAssertTrue(sent.exists, "the payment was not sent")
        XCTAssertTrue(
            sent.staticTexts["Transaction was successfully sent!"].exists,
            "a normal payment must show the plain message: \(sent.staticTexts.allElementsBoundByIndex.map(\.label))"
        )
        sent.buttons["OK"].tap()

        // exactly one broadcast, the wallet's own payment as built
        let payment = try node.waitForBroadcasts(count: 1)[0]
        XCTAssertEqual(try node.broadcastRequests().count, 1, "the payment was broadcast more than once")
        XCTAssertEqual(payment.inputs, [walletCoin.outpoint], "the payment must spend only the wallet's coin")
        XCTAssertTrue(
            payment.outputs.contains { $0.address == session.address && $0.sats == Self.paymentSats },
            "the payment must pay the receiver the requested amount: \(payment.outputs)"
        )
        XCTAssertEqual(
            try walletSpend(walletCoin, broadcast: payment, paying: session),
            confirm.fee,
            "the payment \(payment.outputs) must cost the confirm screen's fee \(confirm.fee)"
        )
    }

    func testDontHelpOffersTheReceiverNothing() throws {
        let walletCoin = try importFundedWallet()
        chooseDontHelp()
        let session = try openConfirmScreen()

        // the switch is still offered, but the payment offers no fee help and its total is exact
        XCTAssertEqual(payjoinSwitch.value as? String, "1", "the Payjoin switch must start on")
        let confirm = try readConfirmScreen()
        XCTAssertFalse(feeHelpLine.exists, "Don't help showed a fee help line: '\(feeHelpLine.label)'")
        XCTAssertFalse(confirm.totalLabel.contains("up to"), "the total must be exact without fee help: '\(confirm.totalLabel)'")
        XCTAssertEqual(
            confirm.total,
            Self.paymentSats + confirm.fee,
            "the total '\(confirm.totalLabel)' must be the payment and the fee \(confirm.fee)"
        )

        swipeAndWaitForReceiver()
        try receiver.respond()

        let sent = app.alerts["Sent!"]
        XCTAssertTrue(sent.waitForExistence(timeout: 60), "the receiver's proposal was not broadcast")
        let message = sentMessage(sent, startingWith: "Sent with Payjoin.")
        sent.buttons["OK"].tap()

        // the receiver joined and paid for its own coin, so the wallet paid just the fee shown
        let proposal = try node.waitForBroadcasts(count: 1)[0]
        XCTAssertEqual(try node.broadcastRequests().count, 1, "the proposal was broadcast more than once")
        XCTAssertEqual(Set(proposal.inputs), [walletCoin.outpoint, session.coin], "the receiver didn't join")

        let joined = #/Sent with Payjoin\. You paid (.+) in fees\./#
        let match = try XCTUnwrap(
            message.wholeMatch(of: joined),
            "a receiver that took nothing must leave fee help out of the message: '\(message)'"
        )
        let feePaid = try onlyAmount(in: String(match.1))
        XCTAssertEqual(feePaid, confirm.fee, "the alert '\(message)' must name the confirm screen's fee \(confirm.fee)")
        XCTAssertEqual(
            try walletSpend(walletCoin, broadcast: proposal, paying: session),
            feePaid,
            "the proposal \(proposal.outputs) must cost the wallet the fee in '\(message)'"
        )
    }

    func testFailedFallbackBroadcastShowsTheNodeError() throws {
        let message = "fake node refused the broadcast"
        _ = try startPayjoinPayment()

        try node.failBroadcasts(message: message)
        app.buttons["Send without Payjoin"].tap()

        let error = app.alerts["Error Broadcasting!"]
        XCTAssertTrue(error.waitForExistence(timeout: 60), "a refused fallback showed no error")
        XCTAssertTrue(
            error.staticTexts.matching(NSPredicate(format: "label CONTAINS %@", message)).firstMatch.exists,
            "the error must carry the node's reason"
        )
        XCTAssertFalse(app.alerts["Sent!"].exists, "a refused payment must not report sent")
        XCTAssertEqual(try node.broadcasts().count, 0, "the node logged a broadcast it refused")
        XCTAssertEqual(try node.broadcastRequests().count, 1, "the refused fallback was sent more than once")
    }

    func testWalletScreenResumesAWaitingPaymentAfterARelaunch() throws {
        let (walletCoin, session) = try startPayjoinPayment()

        // the app has posted the payment to the directory, so its session survives a relaunch
        try receiver.waitForOriginal()
        let secondsLeftBefore = try secondsLeft(in: countdown)
        let readBefore = Date()

        app.terminate()
        app.launchEnvironment.removeValue(forKey: "COVE_UI_TEST_ESPLORA_URL")
        app.launch(expecting: app.element("selectedWallet.balance"))

        let banner = app.staticTexts["Payjoin payment waiting for the receiver"]
        XCTAssertTrue(banner.waitForExistence(timeout: 30), "the wallet screen doesn't show the resumed payment")

        // the resumed session keeps its deadline instead of starting a new ten minutes
        XCTAssertTrue(countdown.waitForExistence(timeout: 10))
        let secondsLeftAfter = try secondsLeft(in: countdown)
        let expected = secondsLeftBefore - Int(Date().timeIntervalSince(readBefore))

        // the countdown lost just the time the relaunch took: a restarted ten minutes would read
        // more, and a deadline moved minutes earlier less. The lower slack is wide because a
        // loaded host once read a confirm screen countdown 13 s behind its redraw
        XCTAssertTrue(
            (expected - 30 ... expected + 2).contains(secondsLeftAfter),
            "the deadline moved over the relaunch: \(secondsLeftBefore) s, then \(secondsLeftAfter) s where \(expected) s was due"
        )
        XCTAssertEqual(try node.broadcasts().count, 0, "the relaunch broadcast the payment")

        // while the fallback's broadcast is held the banner names it, and goes once it is out
        try node.holdBroadcasts()
        app.buttons["Send without Payjoin"].tap()
        try node.waitForHeldBroadcast()
        XCTAssertTrue(
            app.staticTexts["Sending without Payjoin…"].waitForExistence(timeout: 30),
            "the banner doesn't say the fallback is going out"
        )
        try node.releaseBroadcasts()
        XCTAssertTrue(banner.waitForNonExistence(timeout: 60), "the banner stayed after the payment went out")
        XCTAssertTrue(
            app.staticTexts["Sending without Payjoin…"].waitForNonExistence(timeout: 30),
            "the banner stayed after the payment went out"
        )

        let original = try node.waitForBroadcasts(count: 1)[0]
        XCTAssertEqual(try node.broadcastRequests().count, 1, "the fallback was broadcast more than once")
        XCTAssertEqual(original.inputs, [walletCoin.outpoint], "the fallback must spend only the wallet's coin")
        XCTAssertTrue(original.outputs.contains { $0.address == session.address && $0.sats == Self.paymentSats })
    }

    // MARK: - Payment

    /// Imports the funded wallet, pays a new receiver's pj URI and waits on the confirm screen
    /// until it shows the wait for the receiver
    private func startPayjoinPayment() throws -> (FakeNodeControl.FundedOutput, PayjoinReceiverControl.Session) {
        let walletCoin = try importFundedWallet()
        let session = try openConfirmScreen()
        swipeAndWaitForReceiver()

        return (walletCoin, session)
    }

    /// Funds the test wallet's one coin, imports the wallet and waits for its scan to settle
    private func importFundedWallet() throws -> FakeNodeControl.FundedOutput {
        try node.requireHealthy()
        try node.reset()
        let walletCoin = try node.fundWallet(mnemonic: XCUIApplication.testMnemonic, sats: [Self.walletSats])[0]
        try node.mine()

        app.launchEnvironment["COVE_UI_TEST_ESPLORA_URL"] = Self.esploraUrl
        app.launch(expecting: app.staticTexts["Welcome to Cove"])
        app.importTestMnemonic()
        app.assertBalance(sats: Self.walletSats, timeout: 120)

        return walletCoin
    }

    /// Opens a new receiver's session and enters its pj URI from the wallet screen, stopping on
    /// the confirm screen before the swipe
    private func openConfirmScreen() throws -> PayjoinReceiverControl.Session {
        // the receiver's funding mines a block, so it comes after the wallet's scan has settled
        let session = try receiver.open(amountSats: Self.paymentSats)

        // the URI carries the amount, so the amount field fills in on its own
        app.startSend(pasting: session.pjUri)
        app.tapNext()
        XCTAssertTrue(payjoinSwitch.waitForExistence(timeout: 30), "paying a pj URI showed no Payjoin switch")
        XCTAssertFalse(waitingCard.exists, "the confirm screen waits before the payment was sent")

        return session
    }

    /// Swipes to send and waits until the confirm screen shows the wait for the receiver
    private func swipeAndWaitForReceiver() {
        app.swipeToConfirm()

        XCTAssertTrue(waitingCard.waitForExistence(timeout: 30), "paying a pj URI didn't wait for the receiver")
        XCTAssertTrue(countdown.waitForExistence(timeout: 5))
        XCTAssertTrue(
            countdown.label.range(of: Self.freshCountdown, options: .regularExpression) != nil,
            "unexpected countdown: \(countdown.label)"
        )
        XCTAssertTrue(app.buttons["Send without Payjoin"].isEnabled)
        XCTAssertFalse(app.alerts["Sent!"].exists, "a Payjoin payment must not report sent while it waits")
    }

    /// Turns the confirm screen's Payjoin switch off
    private func turnPayjoinOff() {
        // the row is a switch wrapping the control itself, which is what takes the tap; where the
        // control isn't its own element, the tap lands on the trailing edge the control draws at
        let control = payjoinSwitch.switches.firstMatch
        if control.exists {
            control.tap()
        } else {
            payjoinSwitch.coordinate(withNormalizedOffset: CGVector(dx: 0.95, dy: 0.5)).tap()
        }

        waitUntil(payjoinSwitch, matches: NSPredicate(format: "value == '0'"), timeout: 5, "the Payjoin switch didn't turn off")
    }

    /// Picks Don't help in Settings > Payjoin, then returns to the selected wallet
    private func chooseDontHelp() {
        app.tapButton(named: "app.sidebar.open")
        app.tapButton(named: "Settings")
        XCTAssertTrue(app.navigationBars["Settings"].waitForExistence(timeout: 10))

        let row = ScreenReachability(app: app)
            .assertReachable(app.buttons.matching(identifier: "settings.payjoin"), "settings: Payjoin", needsHit: true)
        row.tap()
        XCTAssertTrue(app.navigationBars["Payjoin"].waitForExistence(timeout: 10), "the Payjoin row didn't open its settings")

        let upToOneInput = app.buttons["payjoinSettings.feeHelp.upToOneInput"]
        let dontHelp = app.buttons["payjoinSettings.feeHelp.dontHelp"]
        XCTAssertTrue(dontHelp.waitForExistence(timeout: 10), "Settings > Payjoin offers no Don't help")
        XCTAssertTrue(upToOneInput.isSelected, "fee help must default to up to one input")

        dontHelp.tap()
        _ = dontHelp.wait(for: \.isSelected, toEqual: true, timeout: 5)
        XCTAssertTrue(dontHelp.isSelected, "Don't help was not selected")
        XCTAssertFalse(upToOneInput.isSelected, "up to one input stayed selected beside Don't help")

        app.goBack()
        XCTAssertTrue(app.navigationBars["Settings"].waitForExistence(timeout: 10))
        app.goBack()
        XCTAssertTrue(app.element("selectedWallet.balance").waitForExistence(timeout: 10))
    }

    // MARK: - Fees

    /// What the confirm screen says the payment costs, in sats
    private struct ConfirmFigures {
        /// The network fee row
        let fee: UInt64
        /// The most the receiver may take from the change, nil when the screen shows no fee help
        let feeHelp: UInt64?
        /// The You'll pay row
        let total: UInt64
        let totalLabel: String
    }

    /// Reads the fee, fee help and total the confirm screen shows
    private func readConfirmScreen() throws -> ConfirmFigures {
        let fee = app.staticTexts["sendConfirm.networkFee"]
        let total = app.staticTexts["sendConfirm.totalSpending"]
        XCTAssertTrue(fee.waitForExistence(timeout: 30), "the confirm screen shows no network fee")
        XCTAssertTrue(total.waitForExistence(timeout: 5), "the confirm screen shows no total")

        return try ConfirmFigures(
            fee: onlyAmount(in: fee.label),
            feeHelp: feeHelpLine.exists ? onlyAmount(in: feeHelpLine.label) : nil,
            total: onlyAmount(in: total.label),
            totalLabel: total.label
        )
    }

    /// The one amount `label` names, in sats
    private func onlyAmount(in label: String) throws -> UInt64 {
        let amounts = satsAmounts(in: label)
        XCTAssertEqual(amounts.count, 1, "expected one amount in '\(label)', read \(amounts)")
        return try XCTUnwrap(amounts.first, "no amount in '\(label)'")
    }

    /// What a broadcast cost the wallet beyond the payment: its coin less the payment and the
    /// change, the broadcast's one output that doesn't pay the receiver
    private func walletSpend(
        _ walletCoin: FakeNodeControl.FundedOutput,
        broadcast: FakeNodeControl.Broadcast,
        paying session: PayjoinReceiverControl.Session
    ) throws -> UInt64 {
        let change = broadcast.outputs.filter { $0.address != session.address }
        XCTAssertEqual(change.count, 1, "expected one change output in \(broadcast.outputs)")
        let changeSats = try XCTUnwrap(change.first, "no change output in \(broadcast.outputs)").sats

        return walletCoin.sats - Self.paymentSats - changeSats
    }

    /// The Sent alert's message, which must start with `prefix`
    private func sentMessage(_ alert: XCUIElement, startingWith prefix: String) -> String {
        let message = alert.staticTexts.matching(NSPredicate(format: "label BEGINSWITH %@", prefix)).firstMatch
        XCTAssertTrue(
            message.waitForExistence(timeout: 10),
            "the alert has no message starting '\(prefix)': \(alert.staticTexts.allElementsBoundByIndex.map(\.label))"
        )

        return message.label
    }

    /// Opens the sent payment from the wallet screen and checks its details show `sats` as the
    /// network fee
    private func assertSentDetailsFee(_ sats: UInt64) {
        // the wallet lists its broadcast as unconfirmed, which reads Sending while it has no label
        let row = app.staticTexts["Sending"]
        XCTAssertTrue(row.waitForExistence(timeout: 60), "the wallet screen doesn't list the payment")
        row.tap()

        // whether details are expanded is remembered per wallet
        let showDetails = app.buttons["transactionDetails.showDetails"]
        XCTAssertTrue(showDetails.waitForExistence(timeout: 15), "the payment's details didn't open")
        if showDetails.label == "Show Details" {
            showDetails.tap()
        }

        let fee = app.staticTexts["transactionDetails.networkFee"]
        XCTAssertTrue(fee.waitForExistence(timeout: 15), "the payment's details show no network fee")
        waitForAmount(fee, sats: sats, timeout: 15)
    }

    // MARK: - Queries

    private var payjoinSwitch: XCUIElement {
        app.switches.matching(identifier: "sendConfirm.payjoinToggle").firstMatch
    }

    private var feeHelpLine: XCUIElement {
        app.staticTexts["sendConfirm.payjoinFeeHelp"]
    }

    private var waitingCard: XCUIElement {
        app.staticTexts["Waiting for the receiver"]
    }

    private var countdown: XCUIElement {
        app.staticTexts.matching(NSPredicate(format: "label MATCHES %@", #"\d+:\d\d left"#)).firstMatch
    }

    private func secondsLeft(in countdown: XCUIElement) throws -> Int {
        let parts = countdown.label.replacingOccurrences(of: " left", with: "").split(separator: ":")
        guard parts.count == 2, let minutes = Int(parts[0]), let seconds = Int(parts[1]) else {
            throw ControlRequestError(description: "unreadable countdown: \(countdown.label)")
        }

        return minutes * 60 + seconds
    }
}

/// The Payjoin receiver the UI test host plays, see rust/xtask/src/ui_test_host/payjoin.rs
struct PayjoinReceiverControl {
    struct Session: Decodable {
        let pjUri: String
        /// The address the pj URI pays
        let address: String
        /// The receiver's own coin, which it adds to the payment when it joins
        let coin: String

        enum CodingKeys: String, CodingKey {
            case pjUri = "pj_uri"
            case address
            case coin
        }
    }

    private let baseUrl = UITestHostPorts.signerControl

    /// Funds the receiver's coin on the fake node and opens a session asking for `amountSats`
    func open(amountSats: UInt64) throws -> Session {
        let data = try controlRequest(baseUrl, "POST", "payjoin/receiver", body: ["amount_sats": amountSats])
        return try JSONDecoder().decode(Session.self, from: data)
    }

    /// Waits until the app's Original PSBT reaches the directory, leaving it unanswered
    func waitForOriginal() throws {
        _ = try controlRequest(baseUrl, "POST", "payjoin/original")
    }

    /// Joins the app's payment with the receiver's coin and sends the proposal back
    func respond() throws {
        _ = try controlRequest(baseUrl, "POST", "payjoin/respond")
    }
}
