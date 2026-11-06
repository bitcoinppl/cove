import XCTest

/// Deletes a cloud-backed wallet from this device through Wallet Settings and every confirmation
/// its count calls for, then checks that Cloud Backup lists the cloud copy as restorable and
/// restores it from its cloud-only row
///
/// Both scripted clouds drop a wallet backup the app deletes, so a stray cloud delete would leave
/// nothing to list or restore
final class CloudBackupLocalDeleteFullAppUITests: XCTestCase {
    private static let walletOneRecordId = "24a462de41c1dcbfba825e87674d526b4fc0b7811fb0cee65232cb4f8209c11d"

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

    /// The restored fixture wallet holds only public keys, so its deletion skips the Final Warning
    func testDeletingARestoredWalletKeepsARestorableCloudOnlyCopy() throws {
        let control = try CloudBackupHostControl(
            script: .init(cloud: .fixture, listedWalletRecordIds: [Self.walletOneRecordId])
        )
        robot.launch(control: control)
        robot.restoreFromCloudBackup()

        try deleteThroughWalletSettingsThenRestore(
            control,
            expectedConfirmations: 2,
            expectedRecordId: Self.walletOneRecordId
        )
    }

    /// A new hot wallet whose words were never verified needs the Final Warning, the only
    /// confirmation that tells the user the cloud copy survives
    func testDeletingAFreshlyBackedUpWalletKeepsItsCloudCopyRestorable() throws {
        let control = try CloudBackupHostControl(script: .init(cloud: .freshEnable, namespaceResults: [.empty]))
        robot.launch(control: control)
        robot.startFreshEnableFromOnboarding()
        robot.finishOnboardingAfterCloudBackupEnabled()

        control.releaseVisibility()
        control.waitForAppState("the fresh backup never confirmed", timeout: 60) {
            !$0.pendingUploadVerification && $0.verification == "verified"
        }

        try deleteThroughWalletSettingsThenRestore(control, expectedConfirmations: 3, expectedRecordId: nil)
    }

    // MARK: - Flow

    private func deleteThroughWalletSettingsThenRestore(
        _ control: CloudBackupHostControl,
        expectedConfirmations: Int,
        expectedRecordId: String?
    ) throws {
        robot.openSettings()
        robot.openCloudBackupDetail()
        control.waitForAppState("the provider should hold the wallet's backup and nothing should be cloud-only") {
            $0.detail == "complete" && $0.inventoryComplete && $0.cloudOnly.isEmpty && $0.wallets.count == 1
                && (expectedRecordId == nil || $0.wallets.first?.recordId == expectedRecordId)
        }

        let wallet = try XCTUnwrap(control.appState?.wallets.first)
        let cloudNamespace = control.cloudNamespaces.first
        let passkeyCreatesBefore = control.state.count(.passkeyCreate)

        openWalletSettingsFromCloudBackup()
        deleteWallet(expectedConfirmations: expectedConfirmations)

        // the fresh cloud keeps what the app wrote in the host's directory, so a stray delete
        // would show there before any listing could hide it
        if let cloudNamespace {
            XCTAssertTrue(
                control.cloudWalletRecordIds(namespace: cloudNamespace).contains(wallet.recordId),
                "deleting the local wallet deleted its cloud backup"
            )
        }

        robot.openSettings()
        robot.openCloudBackupDetail()
        control.waitForAppState("the provider should still list the deleted wallet's backup as restorable") {
            $0.detail == "complete" && $0.inventoryComplete && $0.inventoryReady && $0.operation == "idle"
                && $0.cloudOnly == [wallet.recordId] && $0.wallets.isEmpty
        }

        let row = app.descendants(matching: .any)["cloudBackup.wallet.\(wallet.recordId)"]
        robot.scrollToHittable(row)
        XCTAssertTrue(app.staticTexts["Not on This Device"].exists, "the cloud copy is not in Not on This Device")
        XCTAssertTrue(row.label.contains(wallet.name), "the cloud-only row is not the deleted wallet: \(row.label)")
        XCTAssertTrue(row.label.contains("Not on device"), "the cloud-only row is not restorable: \(row.label)")
        XCTAssertFalse(row.label.contains("Restore failed"), "the cloud-only row shows a failure: \(row.label)")

        row.tap()
        let restore = app.buttons["Restore to This Device"]
        XCTAssertTrue(restore.waitForExistence(timeout: 10), "the cloud-only row offered no restore")
        restore.tap()

        control.waitForAppState("restoring the cloud-only row should bring the wallet back") {
            $0.cloudOnly.isEmpty && $0.wallets.contains { $0.recordId == wallet.recordId && $0.name == wallet.name }
        }
        XCTAssertTrue(
            app.staticTexts["Not on This Device"].waitForNonExistence(timeout: 10),
            "the restored wallet should leave the Not on This Device section"
        )
        XCTAssertEqual(control.state.count(.passkeyCreate), passkeyCreatesBefore, "deleting or restoring created a passkey")
    }

    // MARK: - Steps

    /// Leaves Cloud Backup for the selected wallet, then opens its settings at the Danger Zone
    private func openWalletSettingsFromCloudBackup() {
        robot.goBack()
        let backToWallet = app.navigationBars["Settings"].buttons.firstMatch
        XCTAssertTrue(backToWallet.waitForExistence(timeout: 10))
        backToWallet.tap()

        app.tapButton(named: "selectedWallet.more")
        app.tapButton(named: "Wallet Settings")
        robot.scrollToHittable(app.buttons["Delete Wallet"], initialWait: 10)
    }

    /// Taps Delete Wallet and confirms every dialog the wallet's confirmation count shows, which
    /// is pinned so a wrong count would not pass both fixtures
    private func deleteWallet(expectedConfirmations: Int) {
        app.buttons["Delete Wallet"].tap()

        // the first confirmation is a confirmation dialog, which iOS 26 shows as a popover sheet
        // without its title, so its Delete button is the proof it opened
        let confirm = app.sheets.buttons["Delete"]
        XCTAssertTrue(confirm.waitForExistence(timeout: 10), "the first confirmation never showed")
        confirm.tap()
        var confirmations = 1

        for title in ["Confirm Deletion", "Final Warning"] {
            let alert = app.alerts[title]
            guard alert.waitForExistence(timeout: confirmations < expectedConfirmations ? 10 : 3) else { break }

            if title == "Final Warning" {
                XCTAssertTrue(
                    alert.staticTexts.matching(
                        NSPredicate(format: "label BEGINSWITH %@", "This wallet will be deleted from this device. You can recover it from the Cloud Backup screen")
                    ).firstMatch.exists,
                    "the final warning should point to the cloud copy"
                )
            }

            alert.buttons["Delete"].tap()
            confirmations += 1
            XCTAssertTrue(alert.waitForNonExistence(timeout: 10), "\(title) should close once confirmed")
        }

        XCTAssertEqual(confirmations, expectedConfirmations, "the wallet showed the wrong number of confirmations")
        XCTAssertFalse(
            app.alerts["Unable to Delete Wallet"].waitForExistence(timeout: 2),
            "wallet deletion should not fail"
        )
    }
}
