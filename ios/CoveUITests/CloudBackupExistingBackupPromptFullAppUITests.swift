import XCTest

/// Cloud Backup enables against a cloud that also lists a namespace with no master key, as a
/// backup still syncing to the device or a setup that stopped before its upload leaves it
///
/// Such a namespace holds nothing a passkey could open, and nothing on the device tells the two
/// cases apart, so the enable explains that the files aren't available and lets the user wait or
/// create a new backup, instead of bringing the same passkey prompt back on every answer
final class CloudBackupExistingBackupPromptFullAppUITests: XCTestCase {
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

    /// Onboarding shows the files-unavailable prompt, whose Try Again says the files are still
    /// missing and whose Create New Backup enables beside the namespace without deleting it
    func testEnableWithNamespaceWithoutMasterKeyOffersTryAgainAndCreateNew() throws {
        let control = try CloudBackupHostControl(script: .init(
            cloud: .freshEnable,
            namespaceResults: [.staleNamespace]
        ))
        robot.launch(control: control)

        robot.startFreshEnableFromOnboarding()

        // onboarding sees backup metadata it can't read yet and says so instead of asking for a
        // passkey that has nothing to open
        let unavailable = tryAgainUntilFilesStillMissing()
        control.assertNextActions(
            "the files-unavailable prompt should offer Try Again and Create New Backup",
            body: "enabling",
            available: ["acceptEnablePromptUseExisting", "acceptEnablePromptCreateNew"]
        )
        tap(createNew, in: unavailable)
        robot.finishOnboardingAfterCloudBackupEnabled()

        // no passkey was asked to open the namespace, and the new backup sits beside it
        let state = control.state
        XCTAssertEqual(state.count(.passkeyCreate), 1)
        XCTAssertEqual(state.count(.passkeyDiscover), 0, "asked for a passkey for a namespace with no master key")
        XCTAssertEqual(state.count(.namespaceDelete), 0, "the enable deleted cloud data")
        XCTAssertEqual(control.cloudNamespaces.count, 1, "the enable should have uploaded one new backup")
        XCTAssertFalse(control.cloudNamespaces.contains(CloudBackupUITestControl.staleNamespace))
    }

    /// The Settings Enable sheet draws the files-unavailable prompt again after Try Again, and its
    /// Cancel leaves Cloud Backup off without deleting anything
    func testSettingsEnableWithNamespaceWithoutMasterKeyRetriesAndCancels() throws {
        let control = try CloudBackupHostControl(script: .init(cloud: .freshEnable, namespaceResults: [.empty]))
        robot.launch(control: control)
        robot.startFreshEnableFromOnboarding()
        robot.finishOnboardingAfterCloudBackupEnabled()

        control.releaseVisibility()
        control.waitForAppState("the fresh backup never confirmed", timeout: 60) {
            !$0.awaitingUploadConfirmation && $0.verification == "verified"
        }
        robot.openSettings()
        robot.openCloudBackupDetail()
        control.waitForAppState("the inventory never loaded") { $0.inventoryLoaded }
        turnOff(control)

        // the cloud now lists only a namespace whose files never arrived
        try control.update { $0.namespaceResults = [.staleNamespace] }
        let enableRow = app.staticTexts["Enable Cloud Backup"]
        XCTAssertTrue(enableRow.waitForExistence(timeout: 10), "Settings doesn't offer Enable")
        enableRow.tap()
        robot.acceptCloudBackupAcknowledgements()
        robot.tapEnableCloudBackup()

        let passkeyOptions = app.alerts["Passkey Options"]
        XCTAssertTrue(passkeyOptions.waitForExistence(timeout: 30))
        tap(useExisting, in: passkeyOptions)

        let unavailable = tryAgainUntilFilesStillMissing()
        let cancel = unavailable.buttons["Cancel"].firstMatch
        XCTAssertTrue(cancel.waitForExistence(timeout: 5))
        cancel.tap()

        control.assertNextActions(
            "Cancel should leave Cloud Backup off",
            body: "disabled",
            available: ["enable"],
            timeout: 30
        )
        XCTAssertTrue(app.alerts.firstMatch.waitForNonExistence(timeout: 10), "Cancel left a dialog up")
        let state = control.state
        XCTAssertEqual(state.count(.passkeyDiscover), 0, "asked for a passkey for a namespace with no master key")
        XCTAssertEqual(state.count(.passkeyCreate), 1, "Cancel created a passkey")
    }

    private var useExisting: String {
        CloudBackupUIIdentifier.action("acceptEnablePromptUseExisting")
    }

    private var createNew: String {
        CloudBackupUIIdentifier.action("acceptEnablePromptCreateNew")
    }

    /// Waits for the files-unavailable prompt, checks its first showing reports no retry, then
    /// taps Try Again and returns the prompt it brings back, which says the files are still missing
    private func tryAgainUntilFilesStillMissing() -> XCUIElement {
        let unavailable = app.alerts["Backup Files Not Available Yet"]
        XCTAssertTrue(
            unavailable.waitForExistence(timeout: 30),
            "the enable didn't explain that the backup's files aren't available"
        )
        XCTAssertFalse(app.alerts["Passkey Options"].exists, "a passkey prompt is still on screen")
        XCTAssertTrue(unavailable.alertButton(createNew).exists)
        XCTAssertFalse(
            alertText(unavailable, containing: "still aren't available").exists,
            "the first showing of the prompt already reported a failed retry"
        )

        // Try Again finds the files still missing and says so on the prompt it brings back
        tap(useExisting, in: unavailable)
        let retried = app.alerts["Backup Files Not Available Yet"]
        XCTAssertTrue(
            alertText(retried, containing: "still aren't available").waitForExistence(timeout: 30),
            "Try Again brought the prompt back without saying what it found"
        )
        return retried
    }

    private func turnOff(_ control: CloudBackupHostControl) {
        let turnOff = app.buttons[CloudBackupUIIdentifier.action("turnOff")]
        robot.scrollToHittable(turnOff)
        XCTAssertTrue(turnOff.isEnabled, "Turn Off is not available")
        turnOff.tap()

        let review = app.alerts["Turn Off Cloud Backup?"]
        XCTAssertTrue(review.waitForExistence(timeout: 20), "the Turn Off review never showed")
        review.alertButton(CloudBackupUIIdentifier.action("confirmDisable")).tap()
        control.assertNextActions(
            "Turn Off should finish with Cloud Backup off",
            body: "disabled",
            available: ["enable"],
            timeout: 60
        )
        XCTAssertTrue(app.navigationBars["Settings"].waitForExistence(timeout: 10), "Turn Off didn't return to Settings")
    }

    private func tap(_ identifier: String, in alert: XCUIElement) {
        let button = alert.alertButton(identifier)
        XCTAssertTrue(button.waitForExistence(timeout: 5), "\(identifier) is missing from \(alert.label)")
        XCTAssertTrue(button.isEnabled, "\(identifier) is disabled")
        button.tap()
    }

    private func alertText(_ alert: XCUIElement, containing fragment: String) -> XCUIElement {
        alert.staticTexts.matching(NSPredicate(format: "label CONTAINS %@", fragment)).firstMatch
    }
}
