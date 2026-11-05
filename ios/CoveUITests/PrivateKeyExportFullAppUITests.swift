import XCTest

/// Export Private Key on a wallet that holds only an extended private key
///
/// The XPRV-01 scenario restores such a wallet from the scripted cloud backup, the same restore
/// a person runs after reinstalling, so no test-only path creates the wallet
final class PrivateKeyExportFullAppUITests: XCTestCase {
    /// BIP32 test vector 1 master key, the private key inside the XPRV-01 fixture wallet
    private let fixtureXprv =
        "xprv9s21ZrQH143K3QTDL4LXw2F7HEK3wJUD2nW2nRk4stbPy6cq3jPPqjiChkVvvNKmPGJxWUtg6LnF5kejMRNNU3TGtRBeJgk33yuGBxrMPHi"
    private let walletName = "Fixture Private Key Wallet"
    private let mainPin = "135791"
    private let decoyPin = "246802"
    private let wrongPin = "999999"

    private var app: XCUIApplication!
    private var robot: CloudBackupHostRobot!

    override func setUp() {
        super.setUp()
        continueAfterFailure = false
        app = XCUIApplication()
        robot = CloudBackupHostRobot(app: app)
    }

    func testOnlyAFreshMainPinRevealsThePrivateKey() {
        restorePrivateKeyWallet()
        robot.openSettings()
        enableMainAndDecoyPins()
        openWalletSettings()

        // cancelling the prompt reveals nothing
        startReveal()
        app.navigationBars["Verify Identity"].buttons["Cancel"].tap()
        XCTAssertTrue(verifyIdentity.waitForNonExistence(timeout: 5))
        assertPrivateKeyHidden()

        // a wrong PIN and the decoy PIN keep the prompt up without revealing
        startReveal()
        for pin in [wrongPin, decoyPin] {
            robot.enterPin(pin)

            // gives a reveal that would follow the PIN time to appear before asserting it never did
            XCTAssertFalse(
                app.navigationBars["Private Key"].waitForExistence(timeout: 3),
                "PIN \(pin) revealed the private key"
            )
            XCTAssertTrue(verifyIdentity.exists, "PIN \(pin) dismissed the prompt")
            assertPrivateKeyHidden()
        }

        robot.enterPin(mainPin)

        XCTAssertTrue(app.navigationBars["Private Key"].waitForExistence(timeout: 10))
        XCTAssertTrue(app.staticTexts[fixtureXprv].exists)
        app.buttons["Done"].tap()
        XCTAssertTrue(app.navigationBars["Private Key"].waitForNonExistence(timeout: 5))
    }

    /// Reveal's Copy puts the shown key on the pasteboard, and Continue with KeyTeleport asks for
    /// the main PIN afresh as Reveal does before it starts a send of this wallet
    ///
    /// The host reads the pasteboard, since reading it here shows the paste prompt
    func testRevealCopiesTheKeyAndKeyTeleportNeedsAFreshMainPin() throws {
        let signer = HardwareSignerControl()
        try signer.requireHealthy()

        restorePrivateKeyWallet()
        robot.openSettings()
        enableMainAndDecoyPins()
        openWalletSettings()

        startReveal()
        robot.enterPin(mainPin)
        XCTAssertTrue(app.navigationBars["Private Key"].waitForExistence(timeout: 10))
        XCTAssertTrue(app.staticTexts[fixtureXprv].exists)
        app.buttons["Copy for 2 Minutes"].tap()
        XCTAssertTrue(app.buttons["Copied for 2 Minutes"].waitForExistence(timeout: 5))
        XCTAssertEqual(
            try signer.simulatorPasteboard().trimmingCharacters(in: .whitespacesAndNewlines),
            fixtureXprv,
            "Copy put something other than the revealed key on the pasteboard"
        )
        app.buttons["Done"].tap()
        XCTAssertTrue(app.navigationBars["Private Key"].waitForNonExistence(timeout: 5))

        // the PIN Reveal took doesn't carry over, and neither a wrong PIN nor the decoy PIN starts a send
        startExport("Continue with KeyTeleport")
        for pin in [wrongPin, decoyPin] {
            robot.enterPin(pin)

            // gives a send that would follow the PIN time to start before asserting it never did
            XCTAssertFalse(keyTeleportSend.waitForExistence(timeout: 3), "PIN \(pin) started a KeyTeleport send")
            XCTAssertTrue(verifyIdentity.exists, "PIN \(pin) dismissed the prompt")
        }

        robot.enterPin(mainPin)
        XCTAssertTrue(keyTeleportSend.waitForExistence(timeout: 10), "the main PIN didn't start a KeyTeleport send")
        assertPrivateKeyHidden()
    }

    func testExportWithoutAppPinIsRefused() {
        restorePrivateKeyWallet()
        robot.openSettings()
        openWalletSettings()

        tapExportPrivateKey()
        app.buttons["Reveal and Copy"].tap()

        let alert = app.alerts["App Lock Required"]
        XCTAssertTrue(alert.waitForExistence(timeout: 5))
        XCTAssertTrue(
            alert.staticTexts["Enable a PIN or biometric app lock before exporting a private key."].exists
        )
        alert.buttons["OK"].tap()

        XCTAssertFalse(verifyIdentity.exists)
        assertPrivateKeyHidden()
    }

    private var verifyIdentity: XCUIElement {
        app.navigationBars["Verify Identity"]
    }

    /// The first KeyTeleport send screen, which waits for the receiving phone's request
    private var keyTeleportSend: XCUIElement {
        app.staticTexts["Scan Receiver Request"]
    }

    private func restorePrivateKeyWallet() {
        robot.launch(scenario: "XPRV-01")
        robot.restoreFromCloudBackup()
    }

    private func enableMainAndDecoyPins() {
        let pinToggle = app.switches["Enable PIN"]
        robot.scrollToHittable(pinToggle)
        pinToggle.switches.firstMatch.tap()

        XCTAssertTrue(app.staticTexts["Enter New PIN"].waitForExistence(timeout: 10))
        robot.enterPin(mainPin)
        XCTAssertTrue(app.staticTexts["Confirm New PIN"].waitForExistence(timeout: 5))
        robot.enterPin(mainPin)
        XCTAssertTrue(app.navigationBars["Settings"].waitForExistence(timeout: 10))

        let decoyToggle = app.switches["Enable Decoy PIN"]
        robot.scrollToHittable(decoyToggle)
        decoyToggle.switches.firstMatch.tap()

        let confirmDecoy = app.buttons["Yes, Enable Decoy PIN"]
        if !confirmDecoy.waitForExistence(timeout: 5) {
            robot.attachScreen(named: "decoy confirmation missing")
            XCTFail("the decoy PIN confirmation never appeared")
        }
        confirmDecoy.tap()

        XCTAssertTrue(app.staticTexts["Enter Decoy PIN"].waitForExistence(timeout: 10))
        robot.enterPin(decoyPin)
        XCTAssertTrue(app.staticTexts["Confirm Decoy PIN"].waitForExistence(timeout: 5))
        robot.enterPin(decoyPin)
        XCTAssertTrue(app.navigationBars["Settings"].waitForExistence(timeout: 10))
        XCTAssertEqual(decoyToggle.value as? String, "1", "decoy PIN was not saved")
    }

    private func openWalletSettings() {
        // settings rows are tappable text, and the closed sidebar also lists the wallet off
        // screen, so tap the copy Settings shows
        let walletRows = app.staticTexts.matching(NSPredicate(format: "label == %@", walletName))
        XCTAssertTrue(walletRows.firstMatch.waitForExistence(timeout: 10))

        var visibleRow: XCUIElement?
        for _ in 0 ..< 8 {
            visibleRow = walletRows.allElementsBoundByIndex.first(where: \.isHittable)
            if visibleRow != nil { break }
            app.swipeUp()
        }

        guard let visibleRow else { return XCTFail("no visible \(walletName) row in Settings") }
        visibleRow.tap()
    }

    private func tapExportPrivateKey() {
        let export = app.buttons["Export Private Key"]
        robot.scrollToHittable(export)
        export.tap()
    }

    private func startReveal() {
        startExport("Reveal and Copy")
    }

    /// Picks the export route from the warning and waits for the PIN prompt it opens
    private func startExport(_ route: String) {
        tapExportPrivateKey()

        let choice = app.buttons[route]
        XCTAssertTrue(choice.waitForExistence(timeout: 5))
        choice.tap()

        XCTAssertTrue(verifyIdentity.waitForExistence(timeout: 10))
        XCTAssertTrue(app.staticTexts["Enter Cove PIN"].waitForExistence(timeout: 5))
    }

    private func assertPrivateKeyHidden() {
        XCTAssertFalse(app.navigationBars["Private Key"].exists, "the private key sheet is showing")
        XCTAssertFalse(app.staticTexts[fixtureXprv].exists, "the private key is on screen")
    }
}
