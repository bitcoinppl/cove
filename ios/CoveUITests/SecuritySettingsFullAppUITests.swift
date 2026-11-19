import XCTest

/// The PIN and decoy rules Settings follows, which Rust owns: which PIN can become the app PIN,
/// what the decoy PIN's Settings do, and what shows over the app in decoy mode
///
/// The XPRV-01 scenario restores a wallet with Cloud Backup on, so the main mode has a wallet and
/// a backup a decoy session must not give away
final class SecuritySettingsFullAppUITests: XCTestCase {
    private let walletName = "Fixture Private Key Wallet"
    private let mainPin = "135791"
    private let decoyPin = "246802"
    private let decoyWipeDataPin = "112233"

    private var app: XCUIApplication!
    private var robot: CloudBackupHostRobot!

    override func setUp() {
        super.setUp()
        continueAfterFailure = false
        app = XCUIApplication()
        robot = CloudBackupHostRobot(app: app)
    }

    /// Unlocking checks the app PIN first, so an app PIN equal to the decoy PIN would open the main
    /// wallets for anyone given the decoy PIN
    func testChangePinRefusesTheDecoyPin() {
        restoreWalletWithMainAndDecoyPins()

        // the row is a title with a tap gesture, not a button
        let changePin = app.staticTexts["Change PIN"]
        robot.scrollToHittable(changePin)
        changePin.tap()

        XCTAssertTrue(app.staticTexts["Enter Current PIN"].waitForExistence(timeout: 10))
        app.enterPin(mainPin)
        XCTAssertTrue(app.staticTexts["Enter new PIN"].waitForExistence(timeout: 5))
        app.enterPin(decoyPin)
        XCTAssertTrue(app.staticTexts["Confirm New PIN"].waitForExistence(timeout: 5))
        app.enterPin(decoyPin)

        let refusal = robot.staticText(containing: "same as your decoy PIN")
        if !refusal.waitForExistence(timeout: 10) {
            robot.attachScreen(named: "change PIN to the decoy PIN")
            XCTFail("Change PIN accepted the decoy PIN")
        }
        app.buttons["OK"].tap()

        // the app PIN is unchanged, so it still opens the main wallet
        lockByLeavingTheApp()
        app.verifyIdentity(mainPin)
        XCTAssertTrue(
            app.staticTexts.matching(NSPredicate(format: "label == %@", walletName)).firstMatch.waitForExistence(timeout: 20),
            "the main PIN no longer opens the main wallet"
        )
    }

    /// Decoy mode must look like main mode, so turning off a setting main mode guards with its PIN
    /// asks for the decoy PIN, and only the decoy's own toggles change
    func testDecoyModeAsksForTheDecoyPinToTurnSettingsOff() {
        restoreWalletWithMainAndDecoyPins()
        enterDecoyMode()
        robot.openSettings()

        let wipeToggle = app.switches["Enable Wipe Data PIN"]
        robot.scrollToHittable(wipeToggle)
        XCTAssertEqual(wipeToggle.value as? String, "0", "decoy mode starts with the wipe data PIN on")

        wipeToggle.switches.firstMatch.tap()
        app.tapButton(named: "Yes, Enable Wipe Data PIN")
        XCTAssertTrue(app.staticTexts["Enter Wipe Data PIN"].waitForExistence(timeout: 10))
        app.enterPin(decoyWipeDataPin)
        XCTAssertTrue(app.staticTexts["Confirm Wipe Data PIN"].waitForExistence(timeout: 5))
        app.enterPin(decoyWipeDataPin)
        XCTAssertTrue(app.navigationBars["Settings"].waitForExistence(timeout: 10))
        waitForValue(wipeToggle, "1", "turning the wipe data PIN on in decoy mode didn't show it on")

        wipeToggle.switches.firstMatch.tap()
        turnOffWithDecoyPin(wipeToggle, "the wipe data PIN")

        let decoyToggle = app.switches["Enable Decoy PIN"]
        robot.scrollToHittable(decoyToggle)
        decoyToggle.switches.firstMatch.tap()
        app.tapButton(named: "Yes, Enable Decoy PIN")
        XCTAssertTrue(app.staticTexts["Enter Decoy PIN"].waitForExistence(timeout: 10))
        app.enterPin(decoyWipeDataPin)
        XCTAssertTrue(app.staticTexts["Confirm Decoy PIN"].waitForExistence(timeout: 5))
        app.enterPin(decoyWipeDataPin)
        XCTAssertTrue(app.navigationBars["Settings"].waitForExistence(timeout: 10))
        waitForValue(decoyToggle, "1", "turning the decoy PIN on in decoy mode didn't show it on")

        decoyToggle.switches.firstMatch.tap()
        turnOffWithDecoyPin(decoyToggle, "the decoy PIN")

        let pinToggle = app.switches["Enable PIN"]
        robot.scrollToHittable(pinToggle)
        pinToggle.switches.firstMatch.tap()
        turnOffWithDecoyPin(pinToggle, "the PIN")

        // only the decoy's toggles changed, so the main PIN still opens the main wallet and the
        // decoy PIN is still set there
        lockByLeavingTheApp()
        app.verifyIdentity(mainPin)
        XCTAssertTrue(
            app.staticTexts.matching(NSPredicate(format: "label == %@", walletName)).firstMatch.waitForExistence(timeout: 20),
            "turning the PIN off in decoy mode removed the main PIN"
        )
        robot.openSettings()
        let mainDecoyToggle = app.switches["Enable Decoy PIN"]
        robot.scrollToHittable(mainDecoyToggle)
        XCTAssertEqual(mainDecoyToggle.value as? String, "1", "decoy mode turned off the saved decoy PIN")
    }

    /// A wallet change asks main mode to verify the Cloud Backup, but decoy mode must not show
    /// that the main wallets have a backup
    func testDecoyModeRaisesNoCloudBackupPrompt() {
        restoreWalletWithMainAndDecoyPins()
        enterDecoyMode()

        // a new wallet changes the backup, which raises the verify prompt in main mode
        app.importTestMnemonicFromAddWallet()
        XCTAssertTrue(app.element("selectedWallet.balance").waitForExistence(timeout: 30))

        let prompt = app.descendants(matching: .any)
            .matching(identifier: CloudBackupUIIdentifier.promptVerification).firstMatch
        if prompt.waitForExistence(timeout: 10) {
            robot.attachScreen(named: "cloud backup prompt in decoy mode")
            XCTFail("decoy mode showed the Cloud Backup verify prompt")
        }
    }

    private func restoreWalletWithMainAndDecoyPins() {
        robot.launch(scenario: "XPRV-01")
        robot.restoreFromCloudBackup()
        robot.openSettings()

        let pinToggle = app.switches["Enable PIN"]
        robot.scrollToHittable(pinToggle)
        pinToggle.switches.firstMatch.tap()

        XCTAssertTrue(app.staticTexts["Enter New PIN"].waitForExistence(timeout: 10))
        app.enterPin(mainPin)
        XCTAssertTrue(app.staticTexts["Confirm New PIN"].waitForExistence(timeout: 5))
        app.enterPin(mainPin)
        XCTAssertTrue(app.navigationBars["Settings"].waitForExistence(timeout: 10))

        let decoyToggle = app.switches["Enable Decoy PIN"]
        robot.scrollToHittable(decoyToggle)
        decoyToggle.switches.firstMatch.tap()
        app.tapButton(named: "Yes, Enable Decoy PIN")

        XCTAssertTrue(app.staticTexts["Enter Decoy PIN"].waitForExistence(timeout: 10))
        app.enterPin(decoyPin)
        XCTAssertTrue(app.staticTexts["Confirm Decoy PIN"].waitForExistence(timeout: 5))
        app.enterPin(decoyPin)
        XCTAssertTrue(app.navigationBars["Settings"].waitForExistence(timeout: 10))
        XCTAssertEqual(decoyToggle.value as? String, "1", "decoy PIN was not saved")
    }

    /// Locks the app and opens it with the decoy PIN, which shows no wallets yet
    private func enterDecoyMode() {
        lockByLeavingTheApp()
        app.verifyIdentity(decoyPin)
        XCTAssertTrue(
            app.staticTexts.matching(NSPredicate(format: "label == %@", walletName)).firstMatch
                .waitForNonExistence(timeout: 10),
            "the decoy PIN still shows the main wallet"
        )
    }

    /// Goes to the Home Screen for longer than the foreground grace, so the app locks, then comes
    /// back to the lock screen
    ///
    /// A relaunch would lock it too, but this scenario's test keychain doesn't survive one
    private func lockByLeavingTheApp() {
        // the app can go straight on to suspended, so any state but the foreground counts
        XCUIDevice.shared.press(.home)
        let left = NSPredicate { _, _ in self.app.state != .runningForeground }
        let leftForeground = XCTNSPredicateExpectation(predicate: left, object: nil)
        XCTAssertEqual(XCTWaiter.wait(for: [leftForeground], timeout: 10), .completed, "the app never left the foreground")
        Thread.sleep(forTimeInterval: 4)

        app.activate()
        XCTAssertTrue(app.wait(for: .runningForeground, timeout: 10), "the app never came back")
    }

    /// After a tap that turns `toggle` off in decoy mode, expects the PIN prompt main mode shows,
    /// opens it with the decoy PIN, and checks the toggle shows off
    private func turnOffWithDecoyPin(_ toggle: XCUIElement, _ setting: String) {
        let prompt = app.staticTexts["Enter Current PIN"]
        if !prompt.waitForExistence(timeout: 5) {
            robot.attachScreen(named: "turning off \(setting) in decoy mode")
            XCTFail("turning off \(setting) in decoy mode asked for no PIN")
        }

        app.enterPin(decoyPin)
        if !prompt.waitForNonExistence(timeout: 10) {
            robot.attachScreen(named: "decoy PIN for \(setting)")
            XCTFail("the decoy PIN didn't turn off \(setting) in decoy mode")
        }
        waitForValue(toggle, "0", "turning off \(setting) in decoy mode left it on")
    }

    private func waitForValue(_ toggle: XCUIElement, _ value: String, _ message: String) {
        waitUntil(toggle, matches: NSPredicate(format: "value == %@", value), timeout: 5, message)
    }
}
