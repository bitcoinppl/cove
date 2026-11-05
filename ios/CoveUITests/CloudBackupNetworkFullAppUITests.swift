import XCTest

/// Drops and restores the scripted network around Cloud Backup detail, with Fixture Wallet One
/// restored from onboarding and wallets two and three left in the cloud
///
/// The host's network switch fails every provider call while it is down and tells Rust the device
/// is offline, as a real dropped connection would, without touching the simulator's network
final class CloudBackupNetworkFullAppUITests: XCTestCase {
    private static let walletOneRecordId = "24a462de41c1dcbfba825e87674d526b4fc0b7811fb0cee65232cb4f8209c11d"
    private static let cloudOnlyRecordIds = [
        CloudBackupRestoreAllFullAppUITests.walletTwoRecordId,
        CloudBackupRestoreAllFullAppUITests.walletThreeRecordId,
    ]

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

    /// Reopening Cloud Backup offline keeps the wallets it already knew, shows the check as
    /// incomplete instead of an empty cloud, and lets neither Restore All nor any cloud-only row
    /// restore or delete a wallet
    func testOfflineInventoryKeepsKnownRowsAndDisablesTheirActions() throws {
        let control = try openSettledDetail()

        robot.goBack()
        try control.setNetworkConnected(false)
        robot.openCloudBackupDetail()
        waitForOfflineInventory(control)

        XCTAssertTrue(element("cloudBackup.inventory.incomplete").waitForExistence(timeout: 10))
        XCTAssertFalse(app.staticTexts["No iCloud backup files uploaded yet"].exists, "offline showed an empty cloud")
        assertRestoreAllUnavailable()

        for recordId in Self.cloudOnlyRecordIds {
            assertRowOpensNoActions(recordId)
        }

        // the taps above started nothing: no passkey prompt, cloud-only operation or download
        let state = control.state
        XCTAssertEqual(passkeyCalls(state), offlineBaseline.passkeyCalls, "a disabled row asked for a passkey")
        XCTAssertEqual(walletDownloads(state), offlineBaseline.walletDownloads, "a disabled row downloaded a backup")
        XCTAssertEqual(control.appState?.operation, "idle", "a disabled row started a cloud-only operation")
        XCTAssertEqual(control.appState?.cloudOnly, Self.cloudOnlyRecordIds, "the known rows changed offline")
    }

    /// Reconnecting alone refreshes the open detail: the inventory completes with each wallet
    /// listed once, Restore All comes back, and nothing creates or asks for a passkey
    func testReconnectCompletesInventoryWithoutDuplicatesOrPasskey() throws {
        let control = try openSettledDetail()
        let passkeyCallsBeforeOffline = passkeyCalls(control.state)
        let masterUploadsBeforeOffline = control.state.count(.masterUpload)

        robot.goBack()
        try control.setNetworkConnected(false)
        robot.openCloudBackupDetail()
        waitForOfflineInventory(control)
        XCTAssertTrue(element("cloudBackup.inventory.incomplete").waitForExistence(timeout: 10))

        // the reconnect itself must refresh the open detail, so the test taps nothing; the
        // provider counts only listings it served, so a later count shows the inventory was read
        // again rather than the offline rows marked complete
        let listingsBeforeReconnect = walletListings(control.state)
        try control.setNetworkConnected(true)

        control.waitForAppState("reconnecting should complete the inventory with both cloud-only rows") {
            $0.detail == "complete" && $0.inventoryComplete && $0.inventoryError == nil
                && $0.cloudOnly == Self.cloudOnlyRecordIds && !$0.cloudOnlyIncomplete
                && $0.restoreAll == "startAvailable(2)"
        }
        XCTAssertGreaterThan(walletListings(control.state), listingsBeforeReconnect, "the inventory was not read again")
        XCTAssertTrue(
            element("cloudBackup.inventory.incomplete").waitForNonExistence(timeout: 10),
            "the incomplete notice should clear once the inventory completes"
        )

        robot.scrollToHittable(robot.button(startingWith: "Restore All (2)"))
        XCTAssertTrue(robot.button(startingWith: "Restore All (2)").isEnabled)
        for recordId in [Self.walletOneRecordId] + Self.cloudOnlyRecordIds {
            let row = walletRow(recordId)
            robot.scrollToHittable(row)
            XCTAssertEqual(walletRows(recordId).count, 1, "\(recordId) is listed more than once")
        }

        let state = control.state
        XCTAssertEqual(passkeyCalls(state), passkeyCallsBeforeOffline, "the reconnect asked for a passkey")
        XCTAssertEqual(state.count(.passkeyCreate), 0, "the reconnect created a passkey")
        XCTAssertEqual(state.count(.masterUpload), masterUploadsBeforeOffline, "the reconnect wrote a new master key")
        XCTAssertEqual(Set(control.cloudNamespaces).count, control.cloudNamespaces.count)
        XCTAssertLessThanOrEqual(control.cloudNamespaces.count, 1, "the reconnect made a second backup namespace")
    }

    // MARK: - Setup

    /// Provider and passkey calls counted once the offline inventory settled
    private var offlineBaseline: (walletDownloads: [Int], passkeyCalls: [Int]) = ([], [])

    /// Restores Fixture Wallet One, lists all three wallets, and opens Cloud Backup detail once the
    /// entry refresh has listed the cloud and the restore's verification has settled
    private func openSettledDetail() throws -> CloudBackupHostControl {
        let control = try CloudBackupHostControl(
            script: .init(cloud: .fixture, listedWalletRecordIds: [Self.walletOneRecordId])
        )
        robot.launch(control: control)
        robot.restoreFromCloudBackup()
        try control.update { $0.listedWalletRecordIds = nil }

        robot.openSettings()
        let listingsBeforeEntry = walletListings(control.state)
        robot.openCloudBackupDetail()

        // the detail held from the restore can already look complete, so wait for the entry's own
        // listing and for verification to settle too: a verification result that lands after the
        // network drops replaces the detail without a listing and clears the cloud-only rows
        control.waitForAppState("the provider never confirmed both cloud-only rows", timeout: 60) {
            $0.detail == "complete" && $0.inventoryComplete && $0.operation == "idle"
                && $0.verification == "verified"
                && $0.cloudOnly == Self.cloudOnlyRecordIds && $0.restoreAll == "startAvailable(2)"
        }
        XCTAssertGreaterThan(walletListings(control.state), listingsBeforeEntry, "opening the detail listed nothing")

        return control
    }

    private func waitForOfflineInventory(_ control: CloudBackupHostControl) {
        control.waitForAppState("reopening Cloud Backup offline should fail the check and keep both known rows") {
            ["failed", "checking"].contains($0.detail) && !$0.inventoryComplete && $0.inventoryError != nil
                && $0.cloudOnly == Self.cloudOnlyRecordIds && $0.restoreAll == "startDisabled(2)"
        }
        let state = control.state
        offlineBaseline = (walletDownloads(state), passkeyCalls(state))
    }

    // MARK: - Assertions

    /// Restore All shows its count as a disabled label, never as a button
    private func assertRestoreAllUnavailable(file: StaticString = #filePath, line: UInt = #line) {
        let label = robot.staticText(containing: "Restore All (2)")
        robot.scrollToHittable(label)
        let button = robot.button(startingWith: "Restore All")
        XCTAssertFalse(button.exists && button.isEnabled, "Restore All is available offline", file: file, line: line)
    }

    /// The row is disabled, and tapping it opens no Restore or Delete action
    private func assertRowOpensNoActions(_ recordId: String, file: StaticString = #filePath, line: UInt = #line) {
        let row = walletRow(recordId)
        robot.scrollToHittable(row)
        XCTAssertTrue(row.label.contains("Not on device"), "\(recordId) is not a cloud-only row", file: file, line: line)
        XCTAssertFalse(row.isEnabled, "\(recordId) is enabled offline", file: file, line: line)

        row.tap()
        XCTAssertFalse(
            app.buttons["Restore to This Device"].waitForExistence(timeout: 2),
            "\(recordId) opened Restore offline",
            file: file,
            line: line
        )
        XCTAssertFalse(app.buttons["Delete from iCloud"].exists, "\(recordId) opened Delete offline", file: file, line: line)
    }

    // MARK: - Elements and counts

    private func element(_ identifier: String) -> XCUIElement {
        app.descendants(matching: .any)[identifier]
    }

    private func walletRow(_ recordId: String) -> XCUIElement {
        element("cloudBackup.wallet.\(recordId)")
    }

    private func walletRows(_ recordId: String) -> [XCUIElement] {
        app.descendants(matching: .any).matching(identifier: "cloudBackup.wallet.\(recordId)").allElementsBoundByIndex
    }

    private func walletListings(_ state: CloudBackupUITestHostState) -> Int {
        state.count(.walletList) + state.count(.walletListSnapshot)
    }

    private func walletDownloads(_ state: CloudBackupUITestHostState) -> [Int] {
        Self.cloudOnlyRecordIds.map { state.count(.walletDownload(recordId: $0)) }
    }

    private func passkeyCalls(_ state: CloudBackupUITestHostState) -> [Int] {
        [state.count(.passkeyCreate), state.count(.passkeyAuthenticate), state.count(.passkeyDiscover)]
    }
}
