import XCTest

/// Restore All against the scripted fixture cloud, with the batch held inside a wallet's download
/// so the tests can read its progress, move around the app, cancel it or kill the app mid-batch
final class CloudBackupRestoreAllFullAppUITests: XCTestCase {
    static let walletTwoRecordId = "b1089e1017cd2b2a2e0085048b346f2e1c4a850a65b69a43b8d839cd60bd3f6b"
    static let walletThreeRecordId = "fa5eab18295a70b89491f347427099da2dd9d528e59d85b8934084fa01119914"

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

    /// Progress comes from the running batch: it names each wallet in turn, counts completed
    /// wallets, and moves each restored wallet out of the cloud-only rows as it finishes
    func testRunningBatchReportsSequentialProgress() throws {
        let control = try openDetailWithTwoCloudOnlyWallets()
        let screen = RestoreAllScreen(app: app, robot: robot)

        // one download per wallet happens while the batch prepares, before its restore read
        try control.holdWalletDownload(id: "wallet-two", recordId: Self.walletTwoRecordId, skippingCalls: 1)
        try control.holdWalletDownload(id: "wallet-three", recordId: Self.walletThreeRecordId, skippingCalls: 1)
        screen.tapRestoreAll(count: 2)

        control.waitForHold("wallet-two")
        screen.waitForProgress(containing: ["Restoring Fixture Wallet Two", "Completed 0 of 2"])
        XCTAssertTrue(screen.isNotOnDevice(Self.walletTwoRecordId))
        XCTAssertTrue(screen.isNotOnDevice(Self.walletThreeRecordId))

        control.release("wallet-two")
        control.waitForHold("wallet-three")
        screen.waitForProgress(containing: ["Restoring Fixture Wallet Three", "Completed 1 of 2"])
        screen.waitUntilOnDevice(Self.walletTwoRecordId)
        XCTAssertTrue(screen.isNotOnDevice(Self.walletThreeRecordId))

        control.release("wallet-three")
        XCTAssertTrue(screen.progress.waitForNonExistence(timeout: 30), "Restore All never finished")
        screen.waitUntilOnDevice(Self.walletThreeRecordId)
        XCTAssertFalse(robot.button(startingWith: "Restore All").exists)
        XCTAssertFalse(robot.button(startingWith: "Retry Remaining").exists)
    }

    /// Leaving the detail screen and the settings screen keeps the same batch running, and
    /// Cancel lets the current wallet finish and skips the next one
    func testLeavingAndReenteringKeepsTheBatchAndCancelSkipsTheNextWallet() throws {
        let control = try openDetailWithTwoCloudOnlyWallets()
        let screen = RestoreAllScreen(app: app, robot: robot)

        try control.holdWalletDownload(id: "wallet-two", recordId: Self.walletTwoRecordId, skippingCalls: 1)
        screen.tapRestoreAll(count: 2)
        control.waitForHold("wallet-two")
        screen.waitForProgress(containing: ["Restoring Fixture Wallet Two", "Completed 0 of 2"])

        // back out of Cloud Backup and Settings to the wallet, then come back
        robot.goBack()
        XCTAssertTrue(app.navigationBars["Settings"].waitForExistence(timeout: 10))
        robot.goBack()
        XCTAssertTrue(app.navigationBars["Settings"].waitForNonExistence(timeout: 10))
        robot.openSettings()
        robot.openCloudBackupDetail()

        screen.waitForProgress(containing: ["Restoring Fixture Wallet Two", "Completed 0 of 2"])
        XCTAssertEqual(control.state.enteredHolds, ["wallet-two"], "re-entering started another batch")

        let cancel = app.buttons["Cancel"]
        robot.scrollToHittable(cancel)
        cancel.tap()
        robot.scrollToHittable(robot.staticText(containing: "Cancel requested"))
        screen.waitForProgress(containing: ["Completed 0 of 2", "Cancel requested"])

        control.release("wallet-two")
        robot.scrollToHittable(robot.button(startingWith: "Retry Remaining (1)"), initialWait: 30)
        XCTAssertTrue(screen.progress.waitForNonExistence(timeout: 10))
        screen.waitUntilOnDevice(Self.walletTwoRecordId)
        XCTAssertTrue(screen.isNotOnDevice(Self.walletThreeRecordId), "cancel did not skip the next wallet")
        XCTAssertFalse(robot.button(startingWith: "Restore All").exists)
    }

    /// Killing the app partway through a batch, after one wallet finished, leaves Retry Remaining
    /// for only the wallet still in the cloud after a normal restart, without restoring anything or
    /// asking for the passkey on its own, and Retry Remaining then restores it
    func testRelaunchAfterKillOffersRetryRemainingWithoutAutomaticWork() throws {
        let control = try openDetailWithTwoCloudOnlyWallets()
        let screen = RestoreAllScreen(app: app, robot: robot)

        try control.holdWalletDownload(id: "wallet-three", recordId: Self.walletThreeRecordId, skippingCalls: 1)
        screen.tapRestoreAll(count: 2)
        control.waitForHold("wallet-three")
        screen.waitForProgress(containing: ["Restoring Fixture Wallet Three", "Completed 1 of 2"])

        robot.killAndRelaunch()
        robot.openSettings()
        robot.openCloudBackupDetail()

        robot.scrollToHittable(robot.button(startingWith: "Retry Remaining (1)"), initialWait: 30)
        screen.waitUntilOnDevice(Self.walletTwoRecordId)
        XCTAssertTrue(screen.isNotOnDevice(Self.walletThreeRecordId))
        let downloadsAtRetry = walletDownloads(control.state)

        // give any automatic restore or passkey prompt time to start
        Thread.sleep(forTimeInterval: 3)

        let state = control.state
        XCTAssertEqual(state.launches.count, 2, "expected one relaunch")
        XCTAssertEqual(state.countInLatestLaunch(.passkeyDiscover), 0, "relaunch asked for the passkey")
        XCTAssertEqual(state.countInLatestLaunch(.passkeyAuthenticate), 0, "relaunch asked for the passkey")
        XCTAssertEqual(state.countInLatestLaunch(.passkeyCreate), 0, "relaunch created a passkey")
        XCTAssertEqual(walletDownloads(state), downloadsAtRetry, "relaunch restored wallets on its own")
        XCTAssertFalse(screen.progress.exists, "relaunch resumed the batch on its own")

        let retry = robot.button(startingWith: "Retry Remaining (1)")
        robot.scrollToHittable(retry)
        retry.tap()
        screen.waitUntilOnDevice(Self.walletThreeRecordId, timeout: 30)
        XCTAssertTrue(screen.progress.waitForNonExistence(timeout: 30), "the retried batch never finished")
        XCTAssertFalse(robot.button(startingWith: "Retry Remaining").exists)
        XCTAssertEqual(control.state.count(.passkeyCreate), 0)
    }

    /// A cloud record that is another backup of a wallet already on the device shows as Already on
    /// this device, Restore All leaves it out of its count and its batch, and it stays that way
    func testRestoreAllSkipsACloudCopyOfAWalletAlreadyOnDevice() throws {
        let duplicate = CloudBackupUITestScript.duplicateWalletRecordId
        let control = try Self.openDetailWithTwoCloudOnlyWallets(
            app: app,
            robot: robot,
            alsoListing: [duplicate]
        )
        let screen = RestoreAllScreen(app: app, robot: robot)

        robot.scrollToHittable(screen.row(duplicate), initialWait: 15)
        XCTAssertTrue(screen.row(duplicate).label.contains("Already on this device"))
        XCTAssertFalse(robot.button(startingWith: "Restore All (3)").exists)

        try control.holdWalletDownload(id: "wallet-two", recordId: Self.walletTwoRecordId, skippingCalls: 1)
        screen.tapRestoreAll(count: 2)
        control.waitForHold("wallet-two")
        screen.waitForProgress(containing: ["Completed 0 of 2"])
        control.release("wallet-two")

        XCTAssertTrue(screen.progress.waitForNonExistence(timeout: 30), "Restore All never finished")
        screen.waitUntilOnDevice(Self.walletTwoRecordId)
        screen.waitUntilOnDevice(Self.walletThreeRecordId)
        screen.waitUntilOnDevice("24a462de41c1dcbfba825e87674d526b4fc0b7811fb0cee65232cb4f8209c11d")
        robot.scrollToHittable(screen.row(duplicate))
        XCTAssertTrue(screen.row(duplicate).label.contains("Already on this device"), "the copy was restored")
        XCTAssertFalse(robot.button(startingWith: "Retry Remaining").exists)
    }

    private func walletDownloads(_ state: CloudBackupUITestHostState) -> [Int] {
        [Self.walletTwoRecordId, Self.walletThreeRecordId].map {
            state.count(.walletDownload(recordId: $0))
        }
    }

    /// Restores Fixture Wallet One from onboarding while the cloud lists only it, then lists all
    /// three, so the detail screen shows wallets two and three as cloud-only with Restore All (2)
    private func openDetailWithTwoCloudOnlyWallets(
        arguments: [String] = []
    ) throws -> CloudBackupHostControl {
        try Self.openDetailWithTwoCloudOnlyWallets(app: app, robot: robot, arguments: arguments)
    }

    /// The shared setup behind `openDetailWithTwoCloudOnlyWallets()`; `alsoListing` adds records
    /// beyond the three fixture wallets to the later listing
    static func openDetailWithTwoCloudOnlyWallets(
        app _: XCUIApplication,
        robot: CloudBackupHostRobot,
        arguments: [String] = [],
        alsoListing extraRecordIds: [String] = []
    ) throws -> CloudBackupHostControl {
        let firstRecordId = "24a462de41c1dcbfba825e87674d526b4fc0b7811fb0cee65232cb4f8209c11d"
        let control = try CloudBackupHostControl(
            script: .init(cloud: .fixture, listedWalletRecordIds: [firstRecordId])
        )
        robot.launch(control: control, arguments: arguments)
        robot.restoreFromCloudBackup()

        try control.update {
            $0.listedWalletRecordIds = extraRecordIds.isEmpty
                ? nil
                : [firstRecordId, walletTwoRecordId, walletThreeRecordId] + extraRecordIds
        }
        robot.openSettings()
        robot.openCloudBackupDetail()
        robot.scrollToHittable(robot.button(startingWith: "Restore All (2)"), initialWait: 30)

        return control
    }
}

/// The Restore All parts of the Cloud Backup detail screen
struct RestoreAllScreen {
    let app: XCUIApplication
    let robot: CloudBackupHostRobot

    /// The running batch's progress, one element whose value holds the wallet and the counts
    var progress: XCUIElement {
        app.descendants(matching: .any)["Restore All progress"]
    }

    func tapRestoreAll(count: Int) {
        let restoreAll = robot.button(startingWith: "Restore All (\(count))")
        robot.scrollToHittable(restoreAll)
        restoreAll.tap()
    }

    func waitForProgress(
        containing fragments: [String],
        timeout: TimeInterval = 20,
        file: StaticString = #filePath,
        line: UInt = #line
    ) {
        robot.scrollToHittable(progress)

        let deadline = Date().addingTimeInterval(timeout)
        while Date() < deadline {
            if fragments.allSatisfy(progressValue.contains) { return }

            Thread.sleep(forTimeInterval: 0.2)
        }

        robot.attachScreen(named: "restore all progress")
        XCTFail("progress \"\(progressValue)\" never showed \(fragments)", file: file, line: line)
    }

    private var progressValue: String {
        guard progress.exists else { return "" }

        return progress.value as? String ?? ""
    }

    func row(_ recordId: String) -> XCUIElement {
        app.descendants(matching: .any)["cloudBackup.wallet.\(recordId)"]
    }

    /// Whether the wallet's row is in the cloud-only section, whose status reads Not on device
    func isNotOnDevice(_ recordId: String) -> Bool {
        let row = row(recordId)
        return row.waitForExistence(timeout: 5) && row.label.contains("Not on device")
    }

    /// Fixture wallet names, which each row's label starts with
    private static let walletNames = [
        "24a462de41c1dcbfba825e87674d526b4fc0b7811fb0cee65232cb4f8209c11d": "Fixture Wallet One",
        CloudBackupRestoreAllFullAppUITests.walletTwoRecordId: "Fixture Wallet Two",
        CloudBackupRestoreAllFullAppUITests.walletThreeRecordId: "Fixture Wallet Three",
    ]

    /// Statuses of a wallet that is on this device and backed up; Unknown and Failed are not proof
    /// that a restore landed
    private static let onDeviceStatuses = ["Confirmed", "Uploaded, confirming", "Uploading", "Dirty"]

    /// Waits until the wallet's row names it and shows an on-device status
    ///
    /// A missing row is not enough, since the list may be reloading or not have built the row
    func waitUntilOnDevice(
        _ recordId: String,
        timeout: TimeInterval = 20,
        file: StaticString = #filePath,
        line: UInt = #line
    ) {
        guard let name = Self.walletNames[recordId] else {
            return XCTFail("no fixture wallet name for \(recordId)", file: file, line: line)
        }

        let deadline = Date().addingTimeInterval(timeout)
        var label = ""
        while Date() < deadline {
            let row = row(recordId)
            label = row.exists ? row.label : ""
            if label.hasPrefix("\(name), "),
               !label.contains("Not on device"),
               Self.onDeviceStatuses.contains(where: label.contains)
            {
                return
            }

            Thread.sleep(forTimeInterval: 0.2)
        }

        robot.attachScreen(named: "wallet not on device")
        XCTFail("\(name) never showed as on this device; row label \"\(label)\"", file: file, line: line)
    }
}
