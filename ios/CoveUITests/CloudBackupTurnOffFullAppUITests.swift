import XCTest

/// Turns Cloud Backup off through the review Rust builds from the inventory, against the scripted
/// cloud host
///
/// A fresh enable backs up only this device's wallets, so its Turn Off asks once; restoring
/// Fixture Wallet One leaves wallets two and three only in the cloud, so its Turn Off asks twice
final class CloudBackupTurnOffFullAppUITests: XCTestCase {
    static let walletOneRecordId = "24a462de41c1dcbfba825e87674d526b4fc0b7811fb0cee65232cb4f8209c11d"
    static let notOnDeviceRecordIds = [
        CloudBackupRestoreAllFullAppUITests.walletTwoRecordId,
        CloudBackupRestoreAllFullAppUITests.walletThreeRecordId,
    ]
    private static let heldDownload = "held-row-restore"

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

    // MARK: - Scenarios

    /// A wallet only in the cloud needs two confirmations; cancelling either deletes nothing, and
    /// Restore Them First restores them, after which Turn Off asks once and completes
    func testWalletsNotOnDeviceAskTwiceAndRestoreThemFirstRestoresThem() throws {
        let control = try restoreWalletOneAndOpenDetail()
        let namespaces = control.cloudNamespaces

        tapTurnOff(control)
        let review = reviewAlert()
        XCTAssertTrue(
            review.staticTexts.matching(NSPredicate(format: "label CONTAINS %@", "Fixture Wallet Two")).firstMatch.exists,
            "the review should name the wallets only in the cloud"
        )
        XCTAssertFalse(review.buttons["Turn Off and Delete Backups"].exists, "a wallet only in the cloud asked once")

        // the second dialog's Cancel leaves Turn Off as it was
        review.buttons["Continue"].tap()
        let confirmation = app.alerts["Delete these backups and turn off Cloud Backup?"]
        XCTAssertTrue(confirmation.waitForExistence(timeout: 10), "Continue never asked to confirm")
        confirmation.buttons["Cancel"].tap()
        assertTurnOffIdle(control, namespaces: namespaces, after: "cancelling the second dialog")

        // the first dialog's Cancel too
        tapTurnOff(control)
        reviewAlert().buttons["Cancel"].tap()
        assertTurnOffIdle(control, namespaces: namespaces, after: "cancelling the first dialog")

        tapTurnOff(control)
        reviewAlert().buttons["Restore Them First"].tap()
        control.waitForAppState("Restore Them First should restore the wallets only in the cloud", timeout: 60) {
            $0.notOnDeviceRecordIds.isEmpty && $0.wallets.count == 3
        }
        XCTAssertEqual(control.state.count(.namespaceDelete), 0, "Restore Them First deleted a backup")

        control.assertNextActions("once every wallet is on this device, Turn Off is offered", available: ["turnOff"])
        tapTurnOff(control)
        confirmOnce()
        waitForTurnedOff(control)
    }

    /// Backups of this device's wallets and an extra copy of one of them ask once, say the copy
    /// is deleted too, and Turn Off completes
    func testExtraCopiesOnlyAskOnceAndTurnOffCompletes() throws {
        let duplicate = CloudBackupUITestScript.duplicateWalletRecordId
        let control = try CloudBackupHostControl(script: .init(
            cloud: .fixture,
            listedWalletRecordIds: [Self.walletOneRecordId] + Self.notOnDeviceRecordIds
        ))
        robot.launch(control: control)
        robot.restoreFromCloudBackup()
        try control.update { $0.listedWalletRecordIds = [Self.walletOneRecordId] + Self.notOnDeviceRecordIds + [duplicate] }

        robot.openSettings()
        robot.openCloudBackupDetail()
        control.waitForAppState("the copy should list as the only extra copy, with every wallet on this device", timeout: 60) {
            $0.inventoryLoaded && $0.verification == "verified" && $0.wallets.count == 3
                && $0.rows.map(\.recordId) == [duplicate] && $0.rows.map(\.record) == ["extraCopy"]
        }
        control.assertNextActions("extra copies alone shouldn't hold Turn Off back", available: ["turnOff"])

        tapTurnOff(control)
        let review = reviewAlert()
        XCTAssertTrue(
            review.staticTexts.matching(NSPredicate(format: "label CONTAINS %@", "extra copy")).firstMatch.exists,
            "the review should say the extra copy is deleted too"
        )
        XCTAssertFalse(review.buttons["Restore Them First"].exists, "extra copies alone offered Restore Them First")
        confirmOnce()
        waitForTurnedOff(control)
    }

    /// Backups Cove can't check without the passkey ask twice; Verify First leaves the review and
    /// a passkey prompt the user doesn't finish says so neutrally, and Turn Off still completes
    /// through both dialogs
    func testUncheckedBackupsAskTwiceAndStillTurnOffAfterVerifyFirstIsNotCompleted() throws {
        let control = try restoreWalletOneAndOpenDetail()
        let namespaces = control.cloudNamespaces

        // without the local master key the records can only be checked with the passkey, which
        // the user never finishes
        try robot.killAndRelaunch {
            let removed = try control.removeKeychainItems(containing: "cspp::v1::master_key")
            XCTAssertGreaterThan(removed, 0, "the restore saved no master key")
            try control.answerNextPasskeyAssertions([.userCancelled])
        }
        robot.openSettings()
        robot.openCloudBackupDetailFromSettingsRow()
        control.waitForAppState("without the key the other backups should be unchecked", timeout: 60) {
            $0.rows.contains { $0.record == "unchecked" }
        }
        control.assertNextActions("unchecked backups shouldn't hold Turn Off back", available: ["turnOff"])

        tapTurnOff(control)
        let review = reviewAlert()
        XCTAssertTrue(
            review.staticTexts.matching(NSPredicate(format: "label CONTAINS %@", "without your passkey")).firstMatch.exists,
            "the review should say what Cove couldn't check"
        )
        XCTAssertTrue(review.buttons["Continue"].exists, "unchecked backups asked once")
        XCTAssertFalse(review.buttons["Turn Off and Delete Backups"].exists, "unchecked backups asked once")

        review.buttons["Verify First"].tap()
        control.waitForAppState("the unfinished prompt should leave verification not completed") {
            $0.verification == "notCompleted" && $0.screen.disable == "idle"
        }
        control.assertNextActions("Verify First should leave Turn Off and Verify offered", available: ["turnOff", "verify"])
        let notVerified = app.staticTexts["Cloud Backup Not Verified"]
        robot.scrollToHittable(notVerified)
        XCTAssertFalse(app.buttons["Add New Passkey"].exists, "an unfinished prompt offered a passkey repair")
        XCTAssertFalse(
            robot.staticText(containing: "If your passkey was deleted").exists,
            "an unfinished prompt said the passkey may be deleted"
        )
        XCTAssertEqual(control.state.count(.namespaceDelete), 0, "Verify First deleted a backup")
        XCTAssertEqual(control.cloudNamespaces, namespaces)

        tapTurnOff(control)
        reviewAlert().buttons["Continue"].tap()
        let confirmation = app.alerts["Delete these backups and turn off Cloud Backup?"]
        XCTAssertTrue(confirmation.waitForExistence(timeout: 10), "Continue never asked to confirm")
        confirmation.buttons["Delete and Turn Off"].tap()
        waitForTurnedOff(control)
    }

    /// A wallet that shows up only in the cloud between the review and the delete brings the
    /// review back, now asking twice and saying Cloud Backup changed, with nothing deleted
    func testImpactThatGrowsBeforeTheDeleteBringsTheReviewBack() throws {
        let control = try restoreWalletOneAndOpenDetail(listingOthers: false)
        let namespaces = control.cloudNamespaces

        tapTurnOff(control)
        let review = reviewAlert()
        XCTAssertFalse(review.buttons["Continue"].exists, "a backup of this device's wallet asked twice")

        // the recheck after the drain lists the cloud again and finds the two wallets
        try control.update { $0.listedWalletRecordIds = nil }
        review.buttons["Turn Off and Delete Backups"].tap()

        let changed = app.alerts.staticTexts.matching(
            NSPredicate(format: "label CONTAINS %@", "Cloud Backup changed since you checked")
        ).firstMatch
        XCTAssertTrue(changed.waitForExistence(timeout: 30), "the grown impact never brought the review back")
        let renewed = reviewAlert()
        // the recheck lists the cloud without reading the new records, so it counts the two
        // wallets only in the cloud as unchecked rather than naming them
        XCTAssertTrue(
            renewed.staticTexts.matching(NSPredicate(format: "label CONTAINS %@", "Cove can't check 2 backups")).firstMatch.exists,
            "the renewed review should count the wallets only in the cloud as unchecked"
        )
        XCTAssertTrue(renewed.buttons["Continue"].exists, "the renewed review should ask twice")
        control.assertNextActions(
            "the renewed review should be confirmable again",
            available: ["confirmDisable", "cancelDisable"]
        )
        XCTAssertEqual(control.state.count(.namespaceDelete), 0, "the grown impact was deleted anyway")
        XCTAssertEqual(control.cloudNamespaces, namespaces)

        renewed.buttons["Cancel"].tap()
        assertTurnOffIdle(control, namespaces: namespaces, after: "cancelling the renewed review")
    }

    /// Turn Off with no connection says so and deletes nothing; once reconnected, Try Again
    /// reviews and completes
    func testOfflineTurnOffAsksForAConnectionThenCompletesAfterReconnect() throws {
        let control = try freshEnableAndOpenDetail()
        let namespaces = control.cloudNamespaces

        try control.setNetworkConnected(false)
        tapTurnOff(control)

        let offline = app.alerts["Can't Reach iCloud"]
        XCTAssertTrue(offline.waitForExistence(timeout: 20), "Turn Off offline never said it needs a connection")
        control.waitForAppState("Turn Off offline should wait for a connection") { $0.screen.disable == "offline" }
        XCTAssertEqual(control.state.count(.namespaceDelete), 0, "Turn Off offline deleted a backup")
        XCTAssertEqual(control.cloudNamespaces, namespaces)

        try control.setNetworkConnected(true)
        control.assertNextActions("reconnecting should offer Turn Off again", available: ["turnOff"])
        offline.buttons["Try Again"].tap()

        confirmOnce()
        waitForTurnedOff(control)
    }

    /// A delete that fails after the confirmation leaves Turn Off unfinished across a relaunch:
    /// Settings says so, the panel works without the inventory, and Try Again finishes it
    func testFailedDeleteSurvivesRelaunchAndTryAgainFinishesTurnOff() throws {
        let control = try freshEnableAndOpenDetail()
        // a launch resumes a committed Turn Off at once, and a failed delete retries on its own
        // once the cloud is reachable, so every delete fails until the test taps Try Again
        try control.failNamespaceDeletes(1 ... 50)

        tapTurnOff(control)
        confirmOnce()
        control.assertNextActions(
            "a failed delete should leave Turn Off unfinished",
            body: "finishingDisable",
            available: ["resumeDisable"],
            timeout: 60
        )

        robot.killAndRelaunch()
        control.waitUntil("the relaunch should resume the committed Turn Off") {
            $0.launches.count == 2 && $0.launches[1].calls[CloudBackupUITestCall.namespaceDelete.key, default: 0] >= 1
        }

        robot.openSettings()
        // Stop Using is offered only after a failed Try Again, and nobody has tapped one yet
        control.assertNextActions(
            "the relaunch should still be finishing Turn Off",
            body: "finishingDisable",
            available: ["resumeDisable"],
            unavailable: ["turnOff", "stopUsingOnThisDevice"]
        )
        control.waitForAppState("Settings should say Turn Off didn't finish") { $0.settingsRow == "turnOffIncomplete" }

        robot.openCloudBackupDetail(rowTitle: "Cloud Backup Is Being Turned Off")
        XCTAssertTrue(app.descendants(matching: .any)["cloudBackup.finishingDisable"].waitForExistence(timeout: 10))
        let tryAgain = app.buttons["cloudBackup.action.resumeDisable"]
        robot.scrollToHittable(tryAgain)
        try control.stopFailingNamespaceDeletes()
        tryAgain.tap()

        waitForTurnedOff(control)
        XCTAssertTrue(app.navigationBars["Settings"].waitForExistence(timeout: 10), "finishing Turn Off didn't return to Settings")
        XCTAssertTrue(app.staticTexts["Enable Cloud Backup"].waitForExistence(timeout: 10), "Settings doesn't offer Enable")
    }

    /// A passkey prompt the user doesn't complete keeps the wallets and Turn Off, says so
    /// neutrally, and isn't asked again on relaunch
    func testPasskeyNotCompletedKeepsWalletsAndTurnOffWithoutPromptingOnRelaunch() throws {
        let control = try restoreWalletOneAndOpenDetail()
        try control.answerNextPasskeyAssertions([.userCancelled])

        let verify = app.buttons["cloudBackup.action.verify"]
        robot.scrollToHittable(verify)
        verify.tap()
        control.waitForAppState("a dismissed passkey prompt should leave verification not completed") {
            $0.verification == "notCompleted"
        }
        control.assertNextActions("a dismissed passkey prompt should keep Turn Off", available: ["turnOff", "verify"])
        XCTAssertEqual(control.appState?.wallets.map(\.recordId), [Self.walletOneRecordId], "the wallets changed")

        let passkeyCallsBeforeRelaunch = passkeyCalls(control.state)
        robot.killAndRelaunch()
        robot.openSettings()
        control.assertNextActions("the relaunch should keep Turn Off", available: ["turnOff"])
        XCTAssertEqual(passkeyCalls(control.state), passkeyCallsBeforeRelaunch, "the relaunch asked for a passkey")
    }

    /// A running row operation makes Turn Off unavailable with its reason; once it finishes, Turn
    /// Off is offered again
    func testRunningRowOperationHoldsTurnOffUntilItFinishes() throws {
        let control = try restoreWalletOneAndOpenDetail()
        let recordId = Self.notOnDeviceRecordIds[0]
        try control.holdWalletDownload(id: Self.heldDownload, recordId: recordId)

        let restore = robot.rowAction(recordId, "restore")
        robot.scrollToHittable(restore)
        restore.tap()
        control.waitForHold(Self.heldDownload)

        control.assertNextActions("a running restore should hold Turn Off", unavailable: ["turnOff"])
        XCTAssertNotNil(control.appState?.screen.unavailable["turnOff"], "Turn Off is hidden, not unavailable")
        let caption = app.descendants(matching: .any)["cloudBackup.action.turnOff.unavailable"]
        robot.scrollToHittable(caption)
        XCTAssertTrue(caption.exists, "unavailable Turn Off shows no reason")

        control.release(Self.heldDownload)
        control.assertNextActions("Turn Off should come back once the restore finishes", available: ["turnOff"], timeout: 60)
        let turnOff = app.buttons["cloudBackup.action.turnOff"]
        robot.scrollToHittable(turnOff)
        XCTAssertTrue(turnOff.isEnabled, "Turn Off stayed disabled after the restore finished")
    }

    /// The legacy stuck record heals on load into a working backup: the notice shows, no passkey
    /// is asked for, and Turn Off works from the notice
    func testHealedLegacyRecordShowsTheNoticeAndTurnsOffFromIt() throws {
        let control = try freshEnableAndOpenDetail()
        let passkeyCallsBefore = passkeyCalls(control.state)

        try control.requestLegacyStuckRecordOnNextLaunch()
        robot.killAndRelaunch()
        control.waitForLegacyStuckRecordSeeded()

        robot.openSettings()
        control.waitForAppState("the healed backup should say it is on again") {
            $0.settingsRow == "turnedOnAgain" && $0.screen.notices.contains("heal")
        }
        control.assertNextActions("the healed backup should offer Turn Off", available: ["turnOff", "dismissHealNotice"])

        robot.openCloudBackupDetail(rowTitle: "Cloud Backup Turned On Again")
        XCTAssertTrue(app.descendants(matching: .any)["cloudBackup.healNotice"].waitForExistence(timeout: 10))
        XCTAssertEqual(passkeyCalls(control.state), passkeyCallsBefore, "healing asked for a passkey")

        let turnOff = app.buttons["cloudBackup.healNotice.turnOff"]
        robot.scrollToHittable(turnOff)
        turnOff.tap()
        confirmOnce()
        waitForTurnedOff(control)
    }

    /// Leaving the app with the review up and coming back shows the same review, with nothing
    /// deleted, and Cancel still leaves Turn Off offered
    func testBackgroundingDuringTheReviewKeepsTheScreenCoherent() throws {
        let control = try freshEnableAndOpenDetail()
        let namespaces = control.cloudNamespaces

        tapTurnOff(control)
        XCTAssertTrue(app.alerts.buttons["Turn Off and Delete Backups"].waitForExistence(timeout: 20))

        XCUIDevice.shared.press(.home)
        XCTAssertTrue(app.wait(for: .runningBackground, timeout: 10), "the app never left the foreground")
        app.activate()
        XCTAssertTrue(app.wait(for: .runningForeground, timeout: 10), "the app never came back")

        let confirm = app.alerts.buttons["Turn Off and Delete Backups"]
        XCTAssertTrue(confirm.waitForExistence(timeout: 10), "coming back lost the review")
        control.waitForAppState("coming back should keep the review") { $0.screen.disable == "reviewing" }
        control.assertNextActions(
            "coming back should still offer the review's confirm and cancel",
            available: ["confirmDisable", "cancelDisable"]
        )
        XCTAssertEqual(control.state.count(.namespaceDelete), 0, "backgrounding deleted a backup")

        app.alerts.buttons["Cancel"].tap()
        assertTurnOffIdle(control, namespaces: namespaces, after: "cancelling the review after coming back")
    }

    // MARK: - Setup

    /// Restores Fixture Wallet One and opens Cloud Backup once the inventory lists wallets two
    /// and three as not on this device, or, without `listingOthers`, once it lists only wallet one
    private func restoreWalletOneAndOpenDetail(listingOthers: Bool = true) throws -> CloudBackupHostControl {
        let control = try CloudBackupHostControl(
            script: .init(cloud: .fixture, listedWalletRecordIds: [Self.walletOneRecordId])
        )
        robot.launch(control: control)
        robot.restoreFromCloudBackup()
        if listingOthers {
            try control.update { $0.listedWalletRecordIds = nil }
        }

        let notOnDevice = listingOthers ? Self.notOnDeviceRecordIds : []
        robot.openSettings()
        robot.openCloudBackupDetail()
        control.waitForAppState("the inventory never listed the wallets only in the cloud", timeout: 60) {
            $0.inventoryLoaded && $0.verification == "verified"
                && $0.notOnDeviceRecordIds == notOnDevice
        }
        control.assertNextActions("a settled backup should offer Turn Off", available: ["turnOff"])

        return control
    }

    /// Enables Cloud Backup from onboarding and opens it once the fresh backup is confirmed
    private func freshEnableAndOpenDetail() throws -> CloudBackupHostControl {
        let control = try CloudBackupHostControl(script: .init(cloud: .freshEnable, namespaceResults: [.empty]))
        robot.launch(control: control)
        robot.startFreshEnableFromOnboarding()
        robot.finishOnboardingAfterCloudBackupEnabled()

        control.releaseVisibility()
        control.waitForAppState("the fresh backup never confirmed", timeout: 60) {
            !$0.awaitingUploadConfirmation && $0.verification == "verified"
        }
        XCTAssertFalse(control.cloudNamespaces.isEmpty, "the enable wrote no backup")

        robot.openSettings()
        robot.openCloudBackupDetail()
        control.waitForAppState("the inventory never loaded") { $0.inventoryLoaded }
        control.assertNextActions("a confirmed backup should offer Turn Off", available: ["turnOff"])

        return control
    }

    // MARK: - Steps

    private func tapTurnOff(_: CloudBackupHostControl) {
        let turnOff = app.buttons["cloudBackup.action.turnOff"]
        robot.scrollToHittable(turnOff)
        XCTAssertTrue(turnOff.isEnabled, "Turn Off is not available")
        turnOff.tap()
    }

    /// The first dialog, which Rust opens once it has checked what Turn Off deletes
    ///
    /// Alert buttons are matched by label, since an alert action may not carry its identifier
    private func reviewAlert() -> XCUIElement {
        let review = app.alerts["Turn off Cloud Backup?"]
        XCTAssertTrue(review.waitForExistence(timeout: 20), "the Turn Off review never showed")
        return review
    }

    /// Confirms a review that asks once, for a backup of only this device's wallets
    private func confirmOnce() {
        let review = reviewAlert()
        XCTAssertFalse(review.buttons["Continue"].exists, "a backup of this device's wallets asked twice")
        review.buttons["Turn Off and Delete Backups"].tap()
    }

    private func waitForTurnedOff(_ control: CloudBackupHostControl) {
        control.assertNextActions(
            "Turn Off should finish with Cloud Backup off",
            body: "disabled",
            available: ["enable"],
            timeout: 60
        )
        XCTAssertTrue(control.cloudNamespaces.isEmpty, "Turn Off left a backup in the cloud")
    }

    /// Turn Off is offered again, the dialogs are gone, and the cloud still holds every backup
    private func assertTurnOffIdle(
        _ control: CloudBackupHostControl,
        namespaces: [String],
        after step: String,
        file: StaticString = #filePath,
        line: UInt = #line
    ) {
        XCTAssertTrue(app.alerts.firstMatch.waitForNonExistence(timeout: 10), "\(step) left a dialog up", file: file, line: line)
        control.assertNextActions("\(step) should offer Turn Off again", available: ["turnOff"], file: file, line: line)
        control.waitForAppState("\(step) should leave Turn Off idle", file: file, line: line) { $0.screen.disable == "idle" }
        XCTAssertEqual(control.state.count(.namespaceDelete), 0, "\(step) deleted a backup", file: file, line: line)
        XCTAssertEqual(control.cloudNamespaces, namespaces, "\(step) changed the cloud", file: file, line: line)
    }

    private func passkeyCalls(_ state: CloudBackupUITestHostState) -> [Int] {
        [state.count(.passkeyCreate), state.count(.passkeyAuthenticate), state.count(.passkeyDiscover)]
    }
}
