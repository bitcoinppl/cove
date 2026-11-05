import XCTest

/// Deletes one cloud-only wallet backup from Cloud Backup detail through its action dialog and
/// confirmation alert, against the scripted host's cloud storage
///
/// The host writes every wallet backup it deletes to the file `COVE_CLOUD_BACKUP_UI_DELETE_LOG`
/// names, under `/tmp`, which the simulator shares with the test runner
final class CloudBackupDeleteFromCloudFullAppUITests: XCTestCase {
    private let localRecordId = "24a462de41c1dcbfba825e87674d526b4fc0b7811fb0cee65232cb4f8209c11d"
    private let keptRecordId = "b1089e1017cd2b2a2e0085048b346f2e1c4a850a65b69a43b8d839cd60bd3f6b"
    private let selectedRecordId = "fa5eab18295a70b89491f347427099da2dd9d528e59d85b8934084fa01119914"
    private let selectedWalletName = "Fixture Wallet Three"
    /// Test-only signal: the scripted host lists this spare private-key fixture wallet only after a
    /// delete, so its row shows only once a post-delete detail refresh was applied
    private let refreshMarkerRecordId = "042d85e772a6b6381c81acc04737bf48ae9e9151edea2114f567948f77dad5c4"
    private let localWalletName = "Fixture Wallet One"

    private var app: XCUIApplication!
    private var robot: CloudBackupHostRobot!
    private var deleteLogPath: String!

    override func setUp() {
        super.setUp()
        continueAfterFailure = false
        app = XCUIApplication()
        robot = CloudBackupHostRobot(app: app)
        deleteLogPath = "/tmp/cove-ui-test-cloud-delete-\(UUID().uuidString).json"
        app.launchEnvironment["COVE_CLOUD_BACKUP_UI_DELETE_LOG"] = deleteLogPath
    }

    override func tearDown() {
        try? FileManager.default.removeItem(atPath: deleteLogPath)
        super.tearDown()
    }

    func testCancelKeepsEveryBackupAndDeleteForeverRemovesOnlyTheSelectedOne() {
        // SC-U02 restores the first fixture wallet, then lists all three, so two are cloud-only
        robot.launch(scenario: "SC-U02")
        robot.restoreFromCloudBackup()
        robot.openSettings()

        let cloudBackup = app.staticTexts["Cloud Backup Enabled"]
        XCTAssertTrue(cloudBackup.waitForExistence(timeout: 15))
        cloudBackup.tap()
        XCTAssertTrue(app.navigationBars["Cloud Backup"].waitForExistence(timeout: 10))

        XCTAssertTrue(walletRow(recordId: keptRecordId).waitForExistence(timeout: 20))
        XCTAssertTrue(walletRow(recordId: selectedRecordId).exists)
        XCTAssertTrue(inventoryChecking.waitForNonExistence(timeout: 15), "the inventory check never finished")

        // the second cloud-only row, so a delete that took the first row's id would remove the wrong one
        let confirmation = openDeleteConfirmation()
        confirmation.buttons["Cancel"].tap()
        XCTAssertTrue(confirmation.waitForNonExistence(timeout: 5), "Cancel should close the confirmation")

        // gives a delete that Cancel would have started time to reach the provider
        let deadline = Date().addingTimeInterval(3)
        while Date() < deadline {
            XCTAssertEqual(deletedRecordIds(), [], "Cancel must not delete any backup")
            Thread.sleep(forTimeInterval: 0.2)
        }
        XCTAssertTrue(walletRow(recordId: selectedRecordId).exists, "Cancel should keep the selected row")
        XCTAssertTrue(walletRow(recordId: keptRecordId).exists, "Cancel should keep the other row")

        let deleteConfirmation = openDeleteConfirmation()
        deleteConfirmation.buttons["Delete Forever"].tap()

        XCTAssertTrue(
            waitForDeletedRecordIds(nonEmptyWithin: 20),
            "Delete Forever never reached the cloud provider"
        )
        XCTAssertEqual(deletedRecordIds(), [selectedRecordId], "only the selected backup should be deleted")

        // rust drops the row before its detail refresh lands, so wait for the refresh marker row,
        // which only a listing after the delete names, to prove the refreshed inventory was applied
        XCTAssertTrue(
            walletRow(recordId: refreshMarkerRecordId).waitForExistence(timeout: 20),
            "the refreshed inventory after the delete was never applied"
        )
        XCTAssertTrue(inventoryChecking.waitForNonExistence(timeout: 20), "the refresh after the delete never finished")
        XCTAssertFalse(walletRow(recordId: selectedRecordId).exists, "the deleted wallet should leave Not on This Device")
        XCTAssertTrue(walletRow(recordId: keptRecordId).exists, "the other cloud-only wallet should stay")
        XCTAssertTrue(walletRow(recordId: localRecordId).exists, "the local wallet's backup row should stay")
        XCTAssertEqual(deletedRecordIds(), [selectedRecordId], "only the selected backup should be deleted")

        assertLocalWalletStillOnDevice()
    }

    /// Leaves Cloud Backup for the wallet list in the sidebar, which lists only wallets on this
    /// device, since Cloud Backup's rows share one identifier across its on-device and cloud-only sections
    private func assertLocalWalletStillOnDevice() {
        let backToSettings = app.navigationBars["Cloud Backup"].buttons.firstMatch
        XCTAssertTrue(backToSettings.waitForExistence(timeout: 5))
        backToSettings.tap()

        let backToWallet = app.navigationBars["Settings"].buttons.firstMatch
        XCTAssertTrue(backToWallet.waitForExistence(timeout: 10))
        backToWallet.tap()

        let openSidebar = app.buttons["app.sidebar.open"]
        XCTAssertTrue(openSidebar.waitForExistence(timeout: 10))
        openSidebar.tap()

        let localWallet = app.buttons.matching(NSPredicate(format: "label CONTAINS %@", localWalletName)).firstMatch
        XCTAssertTrue(localWallet.waitForExistence(timeout: 10), "the local wallet should stay on this device")
        XCTAssertFalse(
            app.buttons.matching(NSPredicate(format: "label CONTAINS %@", selectedWalletName)).firstMatch.exists,
            "deleting the cloud backup must not add its wallet to this device"
        )
    }

    /// Opens the selected row's action dialog, checks it names the wallet, and returns the
    /// Delete Forever confirmation it leads to
    private func openDeleteConfirmation() -> XCUIElement {
        let row = walletRow(recordId: selectedRecordId)
        robot.scrollToHittable(row)
        row.tap()

        let deleteFromCloud = app.buttons["Delete from iCloud"]
        XCTAssertTrue(deleteFromCloud.waitForExistence(timeout: 10), "the wallet's action dialog never opened")
        XCTAssertTrue(
            app.staticTexts[selectedWalletName].exists,
            "the action dialog should be titled with the selected wallet"
        )
        deleteFromCloud.tap()

        let alert = app.alerts["Delete \(selectedWalletName)?"]
        XCTAssertTrue(alert.waitForExistence(timeout: 10), "the delete confirmation should name the selected wallet")
        XCTAssertTrue(alert.staticTexts["This wallet backup will be permanently removed from iCloud"].exists)
        XCTAssertTrue(alert.buttons["Delete Forever"].exists)
        XCTAssertTrue(alert.buttons["Cancel"].exists)

        return alert
    }

    private func deletedRecordIds() -> [String] {
        guard let data = FileManager.default.contents(atPath: deleteLogPath) else { return [] }

        do {
            return try JSONDecoder().decode([String].self, from: data)
        } catch {
            XCTFail("the delete log is not a JSON array of record ids: \(error)")
            return []
        }
    }

    private func waitForDeletedRecordIds(nonEmptyWithin timeout: TimeInterval) -> Bool {
        let deadline = Date().addingTimeInterval(timeout)
        while Date() < deadline {
            if !deletedRecordIds().isEmpty { return true }

            Thread.sleep(forTimeInterval: 0.2)
        }

        return !deletedRecordIds().isEmpty
    }

    private var inventoryChecking: XCUIElement {
        app.descendants(matching: .any)["cloudBackup.inventory.checking"]
    }

    private func walletRow(recordId: String) -> XCUIElement {
        app.descendants(matching: .any)["cloudBackup.wallet.\(recordId)"]
    }
}
