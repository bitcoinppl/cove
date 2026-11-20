import XCTest

/// A wallet counts as backed up only once Cloud Backup confirms its copy, not as soon as Cloud
/// Backup is on
///
/// The fresh cloud accepts the new backup but keeps it unreadable until the visibility gate is
/// released, so until then the new wallet's upload stays unconfirmed
final class CloudBackupWalletRecoveryFullAppUITests: XCTestCase {
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

    func testAWalletIsBackedUpOnlyOnceCloudBackupConfirmsIt() throws {
        let control = try CloudBackupHostControl(script: .init(cloud: .freshEnable, namespaceResults: [.empty]))
        robot.launch(control: control)
        robot.startFreshEnableFromOnboarding()
        robot.finishOnboardingAfterCloudBackupEnabled()
        control.waitForAppState("the fresh backup should wait for its confirmation") {
            $0.awaitingUploadConfirmation
        }

        XCTAssertTrue(
            backupReminder.waitForExistence(timeout: 10),
            "an unconfirmed backup leaves the wallet unprotected, so the reminder must show"
        )
        assertFinalDeleteWarning(
            startingWith: "This wallet is not backed up",
            button: "Delete Forever"
        )

        control.releaseVisibility()
        control.waitForAppState("the fresh backup never confirmed", timeout: 60) {
            !$0.awaitingUploadConfirmation && $0.verification == "verified"
        }
        control.assertNextActions("the confirmed backup should stay on", available: ["turnOff"])

        XCTAssertTrue(
            backupReminder.waitForNonExistence(timeout: 10),
            "a confirmed backup protects the wallet, so the reminder must go"
        )
        assertFinalDeleteWarning(
            startingWith: "This wallet will be deleted from this device. You can recover it from the Cloud Backup screen",
            button: "Delete"
        )
    }

    // MARK: - Steps

    private var backupReminder: XCUIElement {
        app.buttons.matching(NSPredicate(format: "label CONTAINS[c] %@", "backup your wallet")).firstMatch
    }

    /// Opens Wallet Settings, confirms Delete up to the Final Warning, checks it, and cancels
    ///
    /// A new hot wallet whose words were never verified reaches the Final Warning, the only
    /// confirmation that says whether Cloud Backup can bring it back
    private func assertFinalDeleteWarning(
        startingWith message: String,
        button: String,
        file: StaticString = #filePath,
        line: UInt = #line
    ) {
        app.tapButton(named: "selectedWallet.more")
        app.tapButton(named: "Wallet Settings")
        let deleteWallet = app.buttons["Delete Wallet"]
        robot.scrollToHittable(deleteWallet, initialWait: 10)
        deleteWallet.tap()

        // the first confirmation is a confirmation dialog, which iOS 26 shows as a popover sheet
        // without its title, so its Delete button is the proof it opened
        let confirm = app.sheets.buttons["Delete"]
        XCTAssertTrue(confirm.waitForExistence(timeout: 10), "the first confirmation never showed", file: file, line: line)
        confirm.tap()

        let second = app.alerts["Confirm Deletion"]
        XCTAssertTrue(second.waitForExistence(timeout: 10), "Confirm Deletion never showed", file: file, line: line)
        second.buttons["Delete"].tap()

        let final = app.alerts["Final Warning"]
        XCTAssertTrue(final.waitForExistence(timeout: 10), "the Final Warning never showed", file: file, line: line)
        XCTAssertTrue(
            final.staticTexts.matching(NSPredicate(format: "label BEGINSWITH %@", message)).firstMatch.exists,
            "the Final Warning should start with \"\(message)\"",
            file: file,
            line: line
        )
        XCTAssertTrue(final.buttons[button].exists, "the Final Warning's button should read \"\(button)\"", file: file, line: line)

        final.buttons["Cancel"].tap()
        XCTAssertTrue(final.waitForNonExistence(timeout: 10), "cancelling should close the Final Warning", file: file, line: line)

        // back from Wallet Settings to the wallet
        let back = app.navigationBars.buttons.firstMatch
        XCTAssertTrue(back.waitForExistence(timeout: 10), file: file, line: line)
        back.tap()
        XCTAssertTrue(app.buttons["selectedWallet.more"].waitForExistence(timeout: 10), file: file, line: line)
    }
}
