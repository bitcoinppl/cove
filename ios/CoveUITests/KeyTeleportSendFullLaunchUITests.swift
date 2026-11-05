import XCTest

/// Sends a wallet's recovery words by KeyTeleport in the real app, with the UI test host playing the
/// receiving phone
///
/// The host makes the receiver request and the debug QR seam shows it to the app's scanner. The test
/// reads the response off the screen as the receiving phone's camera would, by decoding the QR code
/// in a screenshot, and the password from its text, and the host decrypts them with the `keyteleport`
/// crate (docs/testing.md). Needs the hardware signer host, which `xtask ios-ui`
/// (`just ios-ui-background`) starts for every run, and a freshly erased simulator, which xtask
/// also provides
final class KeyTeleportSendFullLaunchUITests: XCTestCase {
    private static let esploraUrl = UITestHostPorts.esploraUrl
    private static let pin = "135791"

    private var app: XCUIApplication!
    private var peer: KeyTeleportPeer!
    private var signer: HardwareSignerControl!
    private var qr: QrScript!
    private var layout: ScreenReachability!

    override func setUp() {
        super.setUp()
        continueAfterFailure = false

        app = XCUIApplication()
        peer = KeyTeleportPeer()
        signer = HardwareSignerControl()
        qr = QrScript()
        layout = ScreenReachability(app: app)

        app.launchEnvironment["COVE_UI_TEST_ESPLORA_URL"] = Self.esploraUrl
        qr.configure(app)
    }

    override func tearDown() {
        qr.remove()
        super.tearDown()
    }

    /// The response screen shows the QR code and the password only one at a time, and the receiving
    /// phone decrypts the wallet's words from what it showed
    func testSentWordsShowTheQrCodeAndPasswordOneAtATime() throws {
        try signer.requireHealthy()
        app.launch(expecting: app.staticTexts["Welcome to Cove"])
        app.importTestMnemonic()
        enablePin()
        openSecretWords()
        startKeyTeleportSend()

        let receiver = try peer.startReceiving()
        layout.reachAndTap(app.buttons.matching(NSPredicate(format: "label == %@", "Scan QR")), "send: Scan QR")
        try qr.show([receiver.bbqr])

        let codeField = app.textFields["Receiver Code"]
        XCTAssertTrue(codeField.waitForExistence(timeout: 20), "the scanned request never asked for the receiver code")
        codeField.tap()
        codeField.typeText(receiver.code)
        layout.reachAndTap(button("Continue"), "receiver code: Continue")

        let (response, password) = readSenderResponse()
        let received = try peer.decode(response, session: receiver.session, password: password)
        XCTAssertEqual(received.kind, "mnemonic")
        XCTAssertEqual(received.mnemonic, XCUIApplication.testMnemonic, "the receiver should get this wallet's words")

        layout.reachAndTap(button("Done"), "response: Done")
        XCTAssertTrue(button("Tap to show password").firstMatch.waitForNonExistence(timeout: 10), "Done should close the response")
    }

    // MARK: - Steps

    private func enablePin() {
        app.tapButton(named: "app.sidebar.open")
        app.tapButton(named: "Settings")
        XCTAssertTrue(app.navigationBars["Settings"].waitForExistence(timeout: 10))

        // the row is a switch wrapping the control itself, which is what takes the tap
        let pinToggle = layout.assertReachable(app.switches.matching(identifier: "Enable PIN"), "settings: Enable PIN", needsHit: true)
        pinToggle.switches.firstMatch.tap()
        XCTAssertTrue(app.staticTexts["Enter New PIN"].waitForExistence(timeout: 10))
        enterPin()
        XCTAssertTrue(app.staticTexts["Confirm New PIN"].waitForExistence(timeout: 5))
        enterPin()
        XCTAssertTrue(app.navigationBars["Settings"].waitForExistence(timeout: 10))
        XCTAssertEqual(pinToggle.value as? String, "1", "the PIN was not saved")

        app.goBack()
        XCTAssertTrue(app.element("selectedWallet.balance").waitForExistence(timeout: 10))
    }

    /// Opens the wallet's recovery words, entering the PIN they ask for
    private func openSecretWords() {
        app.tapButton(named: "selectedWallet.more")
        app.tapButton(named: "Wallet Settings")
        layout.reachAndTap(button("View Secret Words"), "wallet settings: View Secret Words")
        app.tapButton(named: "Show Me")
        verifyIdentity()

        XCTAssertTrue(app.buttons["Secret words options"].waitForExistence(timeout: 10), "the recovery words never showed")
    }

    /// Starts KeyTeleport from the recovery words, which asks for the PIN again
    private func startKeyTeleportSend() {
        app.tapButton(named: "Secret words options")
        app.tapButton(named: "KeyTeleport")

        let confirm = app.buttons["Continue"]
        XCTAssertTrue(confirm.waitForExistence(timeout: 5), "KeyTeleport never asked to confirm")
        confirm.tap()
        verifyIdentity()

        XCTAssertTrue(app.staticTexts["Scan Receiver Request"].waitForExistence(timeout: 15), "the send screen never showed")
    }

    /// Reads the response QR code and the password off the screen, checking they show only one at
    /// a time
    private func readSenderResponse() -> (response: String, password: String) {
        XCTAssertTrue(button("Tap to show QR code").firstMatch.waitForExistence(timeout: 20), "the response never showed")
        guard let pair = layout.readRevealPair(
            qrHint: "Tap to show QR code",
            codeHint: "Tap to show password",
            code: shownPassword(),
            "response"
        ) else { return ("", "") }

        XCTAssertTrue(pair.qr.hasPrefix("B$2S"), "the QR code should hold a KeyTeleport sender response, got \(pair.qr.prefix(8))")
        return (pair.qr, pair.code)
    }

    // MARK: - Helpers

    /// Enters the PIN on the prompt a secret asks for
    private func verifyIdentity() {
        let prompt = app.staticTexts["Enter Cove PIN"]
        XCTAssertTrue(prompt.waitForExistence(timeout: 10), "the PIN prompt never showed")
        enterPin()
        XCTAssertTrue(prompt.waitForNonExistence(timeout: 10), "the PIN was not accepted")
    }

    private func enterPin() {
        for digit in Self.pin {
            let key = app.buttons[String(digit)]
            XCTAssertTrue(key.waitForExistence(timeout: 5), "number pad key \(digit) is missing")
            key.tap()
        }
    }

    /// The Teleport Password, eight Base32 characters grouped in pairs
    private func shownPassword() -> XCUIElement {
        app.staticTexts.matching(NSPredicate(format: "label MATCHES %@", "^[A-Z2-7]{2}( [A-Z2-7]{2}){3}$")).firstMatch
    }

    private func button(_ label: String) -> XCUIElementQuery {
        app.buttons.matching(NSPredicate(format: "label == %@", label))
    }
}
