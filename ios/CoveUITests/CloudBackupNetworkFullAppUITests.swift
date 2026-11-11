import XCTest

/// Drops and restores the scripted network, or holds the provider's inventory reads, around Cloud
/// Backup detail, with Fixture Wallet One restored from onboarding and wallets two and three left
/// in the cloud
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
        control.assertNextActions("offline, Restore All should wait for the inventory", unavailable: ["startRestoreAll"])

        XCTAssertTrue(element(CloudBackupUIIdentifier.inventoryIncomplete).waitForExistence(timeout: 10))
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
        XCTAssertEqual(control.appState?.notOnDeviceRecordIds, Self.cloudOnlyRecordIds, "the known rows changed offline")
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
        XCTAssertTrue(element(CloudBackupUIIdentifier.inventoryIncomplete).waitForExistence(timeout: 10))

        // the reconnect itself must refresh the open detail, so the test taps nothing; the
        // provider counts only listings it served, so a later count shows the inventory was read
        // again rather than the offline rows marked complete
        let listingsBeforeReconnect = walletListings(control.state)
        try control.setNetworkConnected(true)

        control.waitForAppState("reconnecting should complete the inventory with both cloud-only rows") {
            $0.inventoryLoaded && $0.inventoryError == nil
                && $0.notOnDeviceRecordIds == Self.cloudOnlyRecordIds
                && $0.restoreAll == "startAvailable(2)"
        }
        control.assertNextActions("reconnecting should offer Restore All again", available: ["startRestoreAll", "turnOff"])
        XCTAssertGreaterThan(walletListings(control.state), listingsBeforeReconnect, "the inventory was not read again")
        XCTAssertTrue(
            element(CloudBackupUIIdentifier.inventoryIncomplete).waitForNonExistence(timeout: 10),
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

    /// Reconnecting after an offline check checks the known inventory again, and while the
    /// provider holds that check every row the app still lists stays disabled; once the provider
    /// answers, a row opens both actions
    ///
    /// A check of a confirmed inventory keeps it ready while it runs, so the check starts from the
    /// failed offline one, which leaves the known rows retained but unconfirmed
    func testHeldReconnectCheckKeepsKnownRowsDisabledUntilTheProviderConfirmsThem() throws {
        let control = try openSettledDetail()

        robot.goBack()
        try control.setNetworkConnected(false)
        robot.openCloudBackupDetail()
        waitForOfflineInventory(control)

        // the refresh reads its snapshot first, and the sync-health check and any verification
        // list the cloud alongside it, so every one of them waits
        let holds = ["snapshot", "listing-1", "listing-2"]
        try control.holdNextWalletListSnapshot(id: "snapshot")
        try control.holdNextWalletList(id: "listing-1")
        try control.holdNextWalletList(id: "listing-2")
        try control.setNetworkConnected(true)
        control.waitForHold("snapshot")

        control.waitForAppState("the held check should keep both known rows without making them ready") {
            $0.inventory == "checking"
                && $0.notOnDeviceRecordIds == Self.cloudOnlyRecordIds
        }
        control.assertNextActions("the held check should hold Restore All", unavailable: ["startRestoreAll"])

        assertRowsWaitForTheInventory(control, phase: "while the reconnect check was held")

        for hold in holds {
            control.release(hold)
        }

        assertRowsOpenTheirActionsOnceConfirmed(control)
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
            $0.inventoryLoaded && $0.operation == "idle"
                && $0.verification == "verified"
                && $0.notOnDeviceRecordIds == Self.cloudOnlyRecordIds && $0.restoreAll == "startAvailable(2)"
        }
        XCTAssertGreaterThan(walletListings(control.state), listingsBeforeEntry, "opening the detail listed nothing")

        return control
    }

    private func waitForOfflineInventory(_ control: CloudBackupHostControl) {
        control.waitForAppState("reopening Cloud Backup offline should fail the check and keep both known rows") {
            $0.inventory == "incomplete" && $0.inventoryError != nil
                && $0.notOnDeviceRecordIds == Self.cloudOnlyRecordIds && $0.restoreAll == "startDisabled(2)"
        }
        let state = control.state
        offlineBaseline = (walletDownloads(state), passkeyCalls(state))
    }

    // MARK: - Assertions

    /// Restore All and every cloud-only row the app lists stay unavailable, and tapping them
    /// starts no passkey prompt, download or cloud-only operation
    private func assertRowsWaitForTheInventory(_ control: CloudBackupHostControl, phase: String) {
        let before = control.state
        let listed = control.appState?.notOnDeviceRecordIds ?? []
        XCTAssertFalse(listed.isEmpty, "no cloud-only row was listed \(phase)")

        // the form builds only rows near the screen, so a shown Restore All is scrolled to first
        let restoreAll = control.appState?.restoreAll ?? ""
        XCTAssertFalse(
            restoreAll.hasPrefix("startAvailable") || restoreAll.hasPrefix("retryAvailable"),
            "Restore All was available \(phase): \(restoreAll)"
        )

        if restoreAll.hasPrefix("startDisabled") {
            assertRestoreAllUnavailable(phase: phase)
        }

        for recordId in listed {
            assertRowOpensNoActions(recordId, phase: phase)
        }

        let after = control.state
        XCTAssertEqual(passkeyCalls(after), passkeyCalls(before), "a disabled row asked for a passkey \(phase)")
        XCTAssertEqual(walletDownloads(after), walletDownloads(before), "a disabled row downloaded a backup \(phase)")
        XCTAssertEqual(control.appState?.operation, "idle", "a disabled row started a cloud-only operation \(phase)")
    }

    /// Once the provider confirms the inventory, a cloud-only row's Restore and Delete are
    /// available, and showing them starts nothing
    private func assertRowsOpenTheirActionsOnceConfirmed(_ control: CloudBackupHostControl) {
        control.waitForAppState("the provider's answer should confirm both cloud-only rows") {
            $0.inventoryLoaded
                && $0.notOnDeviceRecordIds == Self.cloudOnlyRecordIds && $0.restoreAll == "startAvailable(2)"
        }

        let before = control.state
        let recordId = Self.cloudOnlyRecordIds[0]
        let restore = robot.rowAction(recordId, "restore")
        robot.scrollToHittable(restore)
        XCTAssertTrue(restore.isEnabled, "Restore is disabled on a confirmed row")
        XCTAssertTrue(robot.rowAction(recordId, "delete").isEnabled, "Delete is disabled on a confirmed row")

        XCTAssertEqual(walletDownloads(control.state), walletDownloads(before), "showing the row downloaded a backup")
        XCTAssertEqual(control.appState?.operation, "idle", "showing the row started a cloud-only operation")
    }

    /// Restore All shows its count as a disabled label, never as a button
    private func assertRestoreAllUnavailable(
        phase: String = "offline",
        file: StaticString = #filePath,
        line: UInt = #line
    ) {
        let label = robot.staticText(containing: "Restore All (2)")
        robot.scrollToHittable(label)
        let button = robot.button(startingWith: "Restore All")
        XCTAssertFalse(button.exists && button.isEnabled, "Restore All is available \(phase)", file: file, line: line)
    }

    /// The row's Restore and Delete are disabled, and tapping them starts nothing
    private func assertRowOpensNoActions(
        _ recordId: String,
        phase: String = "offline",
        file: StaticString = #filePath,
        line: UInt = #line
    ) {
        let row = walletRow(recordId)
        robot.scrollToHittable(row)
        XCTAssertTrue(row.label.contains("Not on device"), "\(recordId) is not a cloud-only row", file: file, line: line)

        for action in ["restore", "delete"] {
            let button = robot.rowAction(recordId, action)
            robot.scrollToHittable(button, initialWait: 2)
            XCTAssertTrue(button.exists, "\(recordId) has no \(action) \(phase)", file: file, line: line)
            XCTAssertFalse(button.isEnabled, "\(recordId) \(action) is enabled \(phase)", file: file, line: line)
            button.tap()
        }

        XCTAssertFalse(
            app.alerts.firstMatch.waitForExistence(timeout: 2),
            "\(recordId) opened a confirmation \(phase)",
            file: file,
            line: line
        )
    }

    // MARK: - Elements and counts

    private func element(_ identifier: String) -> XCUIElement {
        app.descendants(matching: .any)[identifier]
    }

    private func walletRow(_ recordId: String) -> XCUIElement {
        element(CloudBackupUIIdentifier.wallet(recordId))
    }

    private func walletRows(_ recordId: String) -> [XCUIElement] {
        app.descendants(matching: .any).matching(identifier: CloudBackupUIIdentifier.wallet(recordId)).allElementsBoundByIndex
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
