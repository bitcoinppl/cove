import XCTest

/// Cloud Backup screens at the largest accessibility text size keep their text and actions
/// reachable by scrolling
final class CloudBackupLargeTextFullAppUITests: XCTestCase {
    private let secondRecordId = "b1089e1017cd2b2a2e0085048b346f2e1c4a850a65b69a43b8d839cd60bd3f6b"
    private let thirdRecordId = "fa5eab18295a70b89491f347427099da2dd9d528e59d85b8934084fa01119914"

    private var app: XCUIApplication!
    private var robot: CloudBackupHostRobot!

    override func setUp() {
        super.setUp()
        continueAfterFailure = false
        app = XCUIApplication()
        robot = CloudBackupHostRobot(app: app)
    }

    func testSetupDescriptionAcknowledgementsAndEnableAreReachable() {
        // MAN-03 starts with an empty cloud, so onboarding offers to enable Cloud Backup; the
        // native passkey it wires up is only used after Enable, which this test never taps
        robot.launch(scenario: "MAN-03", arguments: CloudBackupHostRobot.largestContentSizeArguments)

        let getStarted = app.buttons["Get Started"]
        robot.scrollToHittable(getStarted)
        getStarted.tap()

        let newUser = robot.button(startingWith: "No, I")
        robot.scrollToHittable(newUser)
        newUser.tap()

        let enableFromBackup = robot.button(startingWith: "Enable")
        robot.scrollToHittable(enableFromBackup)
        enableFromBackup.tap()
        XCTAssertTrue(app.staticTexts["Cloud Backup"].waitForExistence(timeout: 10))

        robot.scrollToHittable(robot.staticText(containing: "end-to-end encrypted before it leaves your device"))
        robot.scrollToHittable(app.staticTexts["How It Works"])

        [
            "my passkey is required",
            "access to my iCloud account",
            "manually back up my 12 or 24 words",
        ].forEach { fragment in
            let acknowledgement = robot.staticText(containing: fragment)
            robot.scrollToHittable(acknowledgement)
            acknowledgement.tap()
        }

        let enable = app.buttons["Enable Cloud Backup"]
        robot.scrollToHittable(enable)
        XCTAssertTrue(enable.isEnabled, "Enable Cloud Backup stayed disabled after every acknowledgement")
    }

    func testInventoryCloudOnlyRowsAndVerificationActionsAreReachable() {
        openCloudBackupDetail(scenario: "SC-U02")

        // the cloud-only rows appear once the inventory check finishes listing the cloud
        for recordId in [secondRecordId, thirdRecordId] {
            let row = app.descendants(matching: .any)["cloudBackup.wallet.\(recordId)"]
            robot.scrollToHittable(row, initialWait: 20)
        }

        robot.scrollToHittable(robot.button(startingWith: "Restore All (2)"))

        // restore can finish verification on its own, which turns Verify Now into Verify Again
        let verify = robot.button(startingWith: "Verify")
        robot.scrollToHittable(verify)
        verify.tap()

        robot.scrollToHittable(app.staticTexts["Backup verified"], initialWait: 30)
        robot.scrollToHittable(robot.button(startingWith: "Verify Again"))
        robot.scrollToHittable(app.buttons["Disable Cloud Backup"])
    }

    private func openCloudBackupDetail(scenario: String) {
        robot.launch(scenario: scenario, arguments: CloudBackupHostRobot.largestContentSizeArguments)
        robot.restoreFromCloudBackup()
        robot.openSettings()

        let cloudBackup = app.staticTexts["Cloud Backup Enabled"]
        robot.scrollToHittable(cloudBackup)
        cloudBackup.tap()
        XCTAssertTrue(app.navigationBars["Cloud Backup"].waitForExistence(timeout: 10))
    }
}
