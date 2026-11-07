import XCTest

/// Leaves the real app during an NFC scan with a PIN set, and checks the scan survives: no lock,
/// the scan's screen still up, and the tag it then reads still imports
///
/// An NFC scan's system sheet makes the app inactive, and leaving the foreground otherwise covers
/// the app, locks it and dismisses its sheets. The debug NFC seam keeps the reader's scan open
/// until the test writes the tag's NDEF message, so the test can send the app to the background
/// mid-scan. Needs the fake node and the hardware signer host, which `xtask ios-ui`
/// (`just ios-ui-background`) starts for every run
final class NfcScanPrivacyCoverFullLaunchUITests: XCTestCase {
    private static let esploraUrl = UITestHostPorts.esploraUrl
    private static let pin = "246810"

    /// Longer than any foreground grace, so a lock taken on the way out would still be up
    private static let timeAway: TimeInterval = 4

    private var app: XCUIApplication!
    private var node: FakeNodeControl!
    private var signer: HardwareSignerControl!
    private var qr: QrScript!
    private var nfc: NfcNdefScript!
    private var layout: ScreenReachability!

    override func setUp() {
        super.setUp()
        continueAfterFailure = false

        app = XCUIApplication()
        node = FakeNodeControl()
        signer = HardwareSignerControl()
        qr = QrScript()
        nfc = NfcNdefScript()
        layout = ScreenReachability(app: app)

        app.launchEnvironment["COVE_UI_TEST_ESPLORA_URL"] = Self.esploraUrl
        qr.configure(app)
        nfc.configure(app)
    }

    override func tearDown() {
        qr.remove()
        nfc.remove()
        super.tearDown()
    }

    /// With a PIN set, going to the Home Screen and back during Add Wallet's NFC scan keeps the
    /// app unlocked and the scan open, and the tag read afterwards imports its wallet
    func testLeavingDuringAnNfcScanKeepsTheAppUnlockedAndTheScanOpen() throws {
        try node.requireHealthy()
        try signer.requireHealthy()
        try node.reset()

        let first = try signer.publicKeyExport(.nativeSegwit)
        let export = try signer.publicKeyExport(.wrappedSegwit)

        // Add Wallet and Settings are in the sidebar once the app has a wallet
        app.launch(expecting: app.staticTexts["Welcome to Cove"])
        app.openHardwareExportScanner()
        try qr.show([first.key])
        app.finishHardwareImport()
        enablePin()

        app.tapButton(named: "app.sidebar.open")
        app.tapButton(named: "Add Wallet")
        app.scanNfcFromAddWallet()

        // the screen offers NFC help once the scan it started is open, the button's label also
        // carries its icon's name
        let nfcHelp = app.buttons.matching(NSPredicate(format: "label CONTAINS %@", "NFC Help")).firstMatch
        XCTAssertTrue(nfcHelp.waitForExistence(timeout: 10), "the NFC scan never started")

        // the app can go straight on to suspended, so any state but the foreground counts
        XCUIDevice.shared.press(.home)
        let left = NSPredicate { _, _ in self.app.state != .runningForeground }
        let leftForeground = XCTNSPredicateExpectation(predicate: left, object: nil)
        XCTAssertEqual(XCTWaiter.wait(for: [leftForeground], timeout: 10), .completed, "the app never left the foreground")
        Thread.sleep(forTimeInterval: Self.timeAway)
        app.activate()
        XCTAssertTrue(app.wait(for: .runningForeground, timeout: 10), "the app never came back")

        XCTAssertFalse(
            app.staticTexts["Enter Cove PIN"].waitForExistence(timeout: 3),
            "leaving during an NFC scan locked the app"
        )
        XCTAssertTrue(nfcHelp.isHittable, "the NFC scan's screen should still be up and uncovered")

        try nfc.showTextRecord(export.key)

        // ios sets its success alert while the import resets the route to the new wallet and the
        // alert doesn't survive that reset, so the imported wallet's first address is the check
        XCTAssertTrue(
            app.element("selectedWallet.balance").waitForExistence(timeout: 30),
            "the NFC scan left open never imported the tag's wallet"
        )

        let imported = app.alerts["Success"]
        if imported.waitForExistence(timeout: 2) {
            imported.buttons["OK"].tap()
        }

        app.openReceive()
        let derivation = app.staticTexts["Derivation: \(export.firstDerivationPath)"]
        XCTAssertTrue(derivation.waitForExistence(timeout: 10), "the selected wallet should be the one the tag held")
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

    private func enterPin() {
        for digit in Self.pin {
            let key = app.buttons[String(digit)]
            XCTAssertTrue(key.waitForExistence(timeout: 5), "number pad key \(digit) is missing")
            key.tap()
        }
    }
}
