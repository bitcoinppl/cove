import XCTest

/// Receives a KeyTeleport response through the app-wide QR scanner rather than the receive
/// screen's own Scan Sender Response
///
/// A person can show the request, leave the receive screen, and scan the sender's response later
/// from Add Wallet's scanner. That scan goes through `ScanManager`, which has to recognize the
/// response, find the stored receive session and reopen the receive screen at the password. The
/// UI test host plays the sending phone and the debug QR seam stands in for the camera, as in
/// `KeyTeleportReceiveFullLaunchUITests`
final class KeyTeleportAppScannerFullLaunchUITests: XCTestCase {
    /// A BIP39 test vector, a different wallet from the one onboarding imports
    private static let sentWords = "legal winner thank year wave sausage worth useful legal winner thank yellow"

    private var app: XCUIApplication!
    private var peer: KeyTeleportPeer!
    private var signer: HardwareSignerControl!
    private var qr: QrScript!

    override func setUp() {
        super.setUp()
        continueAfterFailure = false

        app = XCUIApplication()
        peer = KeyTeleportPeer()
        signer = HardwareSignerControl()
        qr = QrScript()

        app.launchEnvironment["COVE_UI_TEST_ESPLORA_URL"] = UITestHostPorts.esploraUrl
        qr.configure(app)
    }

    override func tearDown() {
        qr.remove()
        super.tearDown()
    }

    /// A multi-part response scanned out of order from Add Wallet, after leaving the receive
    /// screen, reopens the receive at the password and unlocks to the review
    func testResponseScannedFromTheAppScannerReopensTheReceiveAtThePassword() throws {
        try signer.requireHealthy()
        app.launch(expecting: app.staticTexts["Welcome to Cove"])
        app.importTestMnemonic()

        openAddWallet()
        app.buttons["More wallet import options"].tap()
        app.tapButton(named: "Key Teleport")
        let (request, code) = readReceiverRequest()
        let response = try peer.send(.mnemonic(Self.sentWords), to: request, code: code, minParts: 3)
        XCTAssertGreaterThanOrEqual(response.parts.count, 3, "the response must be a multi-part BBQr")

        // leaving the receive screen keeps its request, so a later scan can still answer it
        app.goBack()
        let moreOptions = app.buttons["More wallet import options"]
        XCTAssertTrue(moreOptions.waitForExistence(timeout: 10), "Back should return to Add Wallet")
        XCTAssertFalse(app.secureTextFields["Teleport Password"].exists)

        moreOptions.tap()
        app.tapButton(named: "QR")
        try qr.show(Array(response.parts.reversed()))

        let field = app.secureTextFields["Teleport Password"]
        XCTAssertTrue(field.waitForExistence(timeout: 20), "the scanned response never reopened the receive at the password")
        field.tap()
        field.typeText(response.password)
        app.tapButton(named: "Unlock")
        XCTAssertTrue(
            app.staticTexts["Recovery words received"].waitForExistence(timeout: 30),
            "the password should unlock the response against the stored request"
        )
    }

    // MARK: - Helpers

    /// Opens Add Wallet from the sidebar unless it is already showing
    private func openAddWallet() {
        let moreOptions = app.buttons["More wallet import options"]
        if !moreOptions.waitForExistence(timeout: 2) {
            app.tapButton(named: "app.sidebar.open")
            app.tapButton(named: "Add Wallet")
        }

        XCTAssertTrue(moreOptions.waitForExistence(timeout: 10))
    }

    /// Reads the receiver code, then the request QR code, off the screen, one revealed at a time
    private func readReceiverRequest() -> (request: String, code: String) {
        let codeHint = app.buttons.matching(NSPredicate(format: "label == %@", "Tap to show receiver code")).firstMatch
        XCTAssertTrue(codeHint.waitForExistence(timeout: 15), "the receive request never showed")
        codeHint.tap()

        let code = app.staticTexts.matching(NSPredicate(format: "label MATCHES %@", "^[0-9]{2}( [0-9]{2}){3}$")).firstMatch
        XCTAssertTrue(code.waitForExistence(timeout: 5), "the receiver code never showed")
        let shownCode = code.label

        app.buttons.matching(NSPredicate(format: "label == %@", "Tap to show QR code")).firstMatch.tap()
        let request = app.waitForVisibleQrCode() ?? ""
        XCTAssertTrue(request.hasPrefix("B$2R"), "the QR code should hold a KeyTeleport receiver request, got \(request.prefix(8))")

        return (request, shownCode)
    }
}
