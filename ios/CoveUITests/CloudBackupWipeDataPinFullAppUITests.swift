import XCTest

/// A new hot wallet backed up by Cloud Backup needs no word verification before the Wipe Data PIN
/// can be turned on
///
/// The check runs after a relaunch, which rebuilds Cloud Backup's live state from what was
/// persisted, as it is whenever someone opens Settings in a later session
final class CloudBackupWipeDataPinFullAppUITests: XCTestCase {
    private static let pin = "135790"

    private var app: XCUIApplication!
    private var robot: CloudBackupHostRobot!

    override func setUp() {
        super.setUp()
        continueAfterFailure = false
        app = XCUIApplication()
        robot = CloudBackupHostRobot(app: app)
    }

    override func tearDown() {
        CloudBackupHostControl.tearDown(self, app: app)
        super.tearDown()
    }

    func testRelaunchedCloudBackupAllowsTheWipeDataPinForAnUnverifiedWallet() throws {
        let control = try CloudBackupHostControl(script: .init(cloud: .freshEnable, namespaceResults: [.empty]))
        robot.launch(control: control)
        robot.startFreshEnableFromOnboarding()
        robot.finishOnboardingAfterCloudBackupEnabled()

        control.releaseVisibility()
        control.waitForAppState("the fresh backup never confirmed", timeout: 60) {
            !$0.awaitingUploadConfirmation && $0.verification == "verified"
        }

        robot.killAndRelaunch()
        robot.openSettings()
        control.assertNextActions("the relaunched backup should still be on", available: ["turnOff"])
        enablePin()

        // the row is a switch wrapping the control itself, which is what takes the tap
        let wipeDataPin = app.switches.matching(identifier: "Enable Wipe Data PIN").firstMatch
        robot.scrollToHittable(wipeDataPin)
        wipeDataPin.switches.firstMatch.tap()

        // the unverified-wallets alert is checked first, so the confirmation proves it was skipped
        let confirm = app.alerts.buttons["Yes, Enable Wipe Data PIN"]
        let unverified = app.alerts["Can't Enable Wipe Data PIN"]
        XCTAssertTrue(
            confirm.waitForExistence(timeout: 10),
            unverified.exists
                ? "Cloud Backup covers the wallet, so it must not need verifying first"
                : "turning on the Wipe Data PIN should ask to confirm it"
        )
        XCTAssertFalse(unverified.exists, "Cloud Backup covers the wallet, so it must not need verifying first")

        app.alerts.buttons["Cancel"].tap()
    }

    /// Sets the app PIN from Settings, which the Wipe Data PIN row needs before it shows
    private func enablePin() {
        let pinToggle = app.switches.matching(identifier: "Enable PIN").firstMatch
        robot.scrollToHittable(pinToggle)
        pinToggle.switches.firstMatch.tap()

        XCTAssertTrue(app.staticTexts["Enter New PIN"].waitForExistence(timeout: 10))
        app.enterPin(Self.pin)
        XCTAssertTrue(app.staticTexts["Confirm New PIN"].waitForExistence(timeout: 5))
        app.enterPin(Self.pin)
        XCTAssertTrue(app.navigationBars["Settings"].waitForExistence(timeout: 10))
        XCTAssertEqual(pinToggle.value as? String, "1", "the PIN was not saved")
    }
}
