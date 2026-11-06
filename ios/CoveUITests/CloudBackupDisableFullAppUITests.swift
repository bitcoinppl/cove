import XCTest

/// Disables Cloud Backup through the detail's production Disable action while the cloud holds a
/// wallet upload mid-write
///
/// The fresh-enable cloud keeps every write in the host's directory and stores a held upload only
/// once it is released, so a disable that deleted the backup before the write drained would leave
/// that wallet behind in the cloud
final class CloudBackupDisableFullAppUITests: XCTestCase {
    private static let heldUpload = "new-wallet-upload"

    /// How long the disable must keep waiting on the held write before the test releases it
    private static let heldWriteObservation: TimeInterval = 3

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

    func testDisableWaitsForAHeldUploadThenDeletesTheWholeBackup() throws {
        let control = try CloudBackupHostControl(script: .init(cloud: .freshEnable, namespaceResults: [.empty]))
        robot.launch(control: control)
        robot.startFreshEnableFromOnboarding()
        robot.finishOnboardingAfterCloudBackupEnabled()

        control.releaseVisibility()
        control.waitForAppState("the fresh backup never confirmed", timeout: 60) {
            !$0.pendingUploadVerification && $0.verification == "verified"
        }
        let namespace = try XCTUnwrap(control.cloudNamespaces.first, "the enable wrote no backup")

        // adding a wallet uploads it on its own, and the host parks that upload before the cloud
        // stores it
        try control.holdNextWalletUpload(id: Self.heldUpload)
        let uploadsBeforeHeld = control.state.count(.walletUpload)
        app.importTestMnemonicFromAddWallet()
        control.waitForHold(Self.heldUpload)

        robot.openSettings()
        openCloudBackupDetailWhateverItsStatus()
        control.waitForAppState("the detail never confirmed its inventory, which Disable needs") {
            $0.detail == "complete" && $0.inventoryComplete
        }
        disableThroughBothConfirmations()

        XCTAssertTrue(
            app.staticTexts["Deleting cloud backups..."].waitForExistence(timeout: 10),
            "the disable never started"
        )
        control.waitForAppState("Cloud Backup should be disabling while the write is held") {
            $0.settingsRow == "disabling"
        }
        assertBackupKeptWhileWriteIsHeld(control, namespace: namespace)

        control.release(Self.heldUpload)

        control.waitForAppState("the disable should finish once the held write drains", timeout: 60) {
            $0.lifecycle == "disabled"
        }
        XCTAssertEqual(control.state.count(.namespaceDelete), 1, "the disable should delete the backup once")
        XCTAssertTrue(control.cloudNamespaces.isEmpty, "the disable left a master key in the cloud")
        XCTAssertEqual(
            control.cloudWalletRecordIds(namespace: namespace),
            [],
            "a write that finished after the delete left a wallet in the cloud"
        )
        XCTAssertEqual(
            control.state.count(.walletUpload),
            uploadsBeforeHeld + 1,
            "the disable let another wallet upload through"
        )
    }

    // MARK: - Steps

    /// Opens Cloud Backup detail from Settings by the configured row, whose title follows the
    /// backup's status, such as Syncing while the wallet's upload is held
    private func openCloudBackupDetailWhateverItsStatus() {
        let row = app.buttons.matching(NSPredicate(format: "label BEGINSWITH %@", "Cloud Backup")).firstMatch
        robot.scrollToHittable(row, initialWait: 15)
        row.tap()
        XCTAssertTrue(app.navigationBars["Cloud Backup"].waitForExistence(timeout: 10))
    }

    private func disableThroughBothConfirmations() {
        let disable = app.buttons["Disable Cloud Backup"]
        robot.scrollToHittable(disable)
        XCTAssertTrue(disable.isEnabled, "Disable Cloud Backup is not available")
        disable.tap()

        // iOS 26 shows the first confirmation dialog as a popover sheet without its title
        let proceed = app.sheets.buttons["Continue"]
        XCTAssertTrue(proceed.waitForExistence(timeout: 10), "the first disable confirmation never showed")
        proceed.tap()

        let finalConfirmation = app.alerts["Delete Cloud Backups?"]
        XCTAssertTrue(finalConfirmation.waitForExistence(timeout: 10), "the final disable confirmation never showed")
        finalConfirmation.buttons["Delete Cloud Backups and Disable"].tap()
    }

    /// The disable keeps waiting on the held write for a while, deleting nothing
    private func assertBackupKeptWhileWriteIsHeld(_ control: CloudBackupHostControl, namespace: String) {
        let deadline = Date().addingTimeInterval(Self.heldWriteObservation)
        while Date() < deadline {
            XCTAssertEqual(control.state.count(.namespaceDelete), 0, "the disable deleted the backup before the write drained")
            XCTAssertEqual(control.cloudNamespaces, [namespace], "the backup disappeared while the write was held")
            XCTAssertNotEqual(control.appState?.lifecycle, "disabled", "the disable finished before the write drained")

            Thread.sleep(forTimeInterval: 0.2)
        }
    }
}
