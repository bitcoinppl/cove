import XCTest

/// The recovery screen for an interrupted Cloud Backup setup at the largest accessibility text size
///
/// Not listed in the UI tests workflow's `IOS_UI_SUITES` until it passes in CI
final class CloudBackupIncompleteSetupFullAppUITests: XCTestCase {
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

    /// An interrupted setup whose staged key the keychain lost can't be resumed, so the relaunched
    /// app shows the recovery screen with its support code and cleanup action
    func testIncompleteSetupRecoveryActionsAreReachable() throws {
        let control = try CloudBackupHostControl(script: .init(
            cloud: .freshEnable,
            namespaceResults: [.empty],
            holds: [.init(id: "master-upload", point: .masterUploadReturn)]
        ))
        robot.launch(control: control, arguments: CloudBackupHostRobot.largestContentSizeArguments)
        robot.startFreshEnableFromOnboarding()
        control.waitForHold("master-upload")

        try robot.killAndRelaunch {
            let removed = try control.removeKeychainItems(containing: "staged_master_key")
            XCTAssertGreaterThan(removed, 0, "the interrupted setup staged no master key")
        }

        robot.scrollToHittable(app.staticTexts["Cloud Backup Needs Recovery"], initialWait: 30)
        robot.scrollToHittable(app.staticTexts["Support code"])
        robot.scrollToHittable(app.descendants(matching: .any)["cloudBackup.recovery.supportCode"])
        robot.scrollToHittable(robot.button(startingWith: "Copy Code"))
        robot.scrollToHittable(robot.button(startingWith: "Contact Support"))

        let removeSetupIdentifier = "cloudBackup.recovery.removeIncompleteSetup"
        let removeSetup = app.buttons[removeSetupIdentifier]
        robot.scrollToHittable(removeSetup)
        removeSetup.tap()

        // the screen's own button has the same label and sits under the dialog, so match the
        // dialog's button by leaving that one out
        let confirmRemoval = app.buttons.matching(NSPredicate(
            format: "label == %@ AND identifier != %@",
            "Remove Incomplete Setup",
            removeSetupIdentifier
        )).firstMatch
        XCTAssertTrue(confirmRemoval.waitForExistence(timeout: 10))
        XCTAssertTrue(confirmRemoval.isHittable, "the removal confirmation is not reachable")
    }
}
