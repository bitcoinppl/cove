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
        let (walletCoin, session) = try startPayjoinPayment()

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
        XCTAssertTrue(
            sent.staticTexts["Transaction was successfully sent!"].exists,
            "a joined payment must not say it went out without Payjoin"
        )
        sent.buttons["OK"].tap()

        // the one broadcast is the receiver's proposal, spending its coin alongside the wallet's
        let proposal = try node.waitForBroadcasts(count: 1)[0]
        XCTAssertEqual(try node.broadcastRequests().count, 1, "the proposal was broadcast more than once")
        XCTAssertEqual(Set(proposal.inputs), [walletCoin.outpoint, session.coin])
        XCTAssertTrue(
            proposal.outputs.contains { $0.address == session.address },
            "the proposal must still pay the receiver: \(proposal.outputs)"
        )
        XCTAssertTrue(app.element("selectedWallet.balance").waitForExistence(timeout: 15))
    }

    func testSendWithoutPayjoinBroadcastsTheOriginalOnce() throws {
        let (walletCoin, session) = try startPayjoinPayment()

        // with the fallback's broadcast held, the card is gone and nothing claims the payment went out
        try node.holdBroadcasts()
        app.buttons["Send without Payjoin"].tap()
        try node.waitForHeldBroadcast()
        XCTAssertTrue(waitingCard.waitForNonExistence(timeout: 30), "the card stayed while the fallback went out")
        XCTAssertFalse(app.alerts["Sent!"].exists, "the payment was reported sent before its broadcast answered")
        try node.releaseBroadcasts()

        let sent = app.alerts["Sent!"]
        XCTAssertTrue(sent.waitForExistence(timeout: 60), "cancelling the wait did not send the payment")
        XCTAssertTrue(
            sent.staticTexts["Transaction was successfully sent, without Payjoin."].exists,
            "the fallback must say it went out without Payjoin"
        )
        sent.buttons["OK"].tap()

        // exactly one broadcast, the wallet's own payment as built
        let original = try node.waitForBroadcasts(count: 1)[0]
        XCTAssertEqual(try node.broadcastRequests().count, 1, "the fallback was broadcast more than once")
        XCTAssertEqual(original.inputs, [walletCoin.outpoint], "the fallback must spend only the wallet's coin")
        XCTAssertTrue(
            original.outputs.contains { $0.address == session.address && $0.sats == Self.paymentSats },
            "the fallback must pay the receiver the requested amount: \(original.outputs)"
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
        try node.requireHealthy()
        try node.reset()
        let walletCoin = try node.fundWallet(mnemonic: XCUIApplication.testMnemonic, sats: [Self.walletSats])[0]
        try node.mine()

        app.launchEnvironment["COVE_UI_TEST_ESPLORA_URL"] = Self.esploraUrl
        app.launch(expecting: app.staticTexts["Welcome to Cove"])
        app.importTestMnemonic()
        app.assertBalance(sats: Self.walletSats, timeout: 120)

        // the receiver's funding mines a block, so it comes after the wallet's scan has settled
        let session = try receiver.open(amountSats: Self.paymentSats)

        // the URI carries the amount, so the amount field fills in on its own
        app.startSend(pasting: session.pjUri)
        app.tapNext()
        XCTAssertFalse(waitingCard.exists, "the confirm screen waits before the payment was sent")
        app.swipeToConfirm()

        XCTAssertTrue(waitingCard.waitForExistence(timeout: 30), "paying a pj URI didn't wait for the receiver")
        XCTAssertTrue(countdown.waitForExistence(timeout: 5))
        XCTAssertTrue(
            countdown.label.range(of: Self.freshCountdown, options: .regularExpression) != nil,
            "unexpected countdown: \(countdown.label)"
        )
        XCTAssertTrue(app.buttons["Send without Payjoin"].isEnabled)
        XCTAssertFalse(app.alerts["Sent!"].exists, "a Payjoin payment must not report sent while it waits")

        return (walletCoin, session)
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
