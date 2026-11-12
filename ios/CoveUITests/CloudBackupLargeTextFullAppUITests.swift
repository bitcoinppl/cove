import XCTest

/// Cloud Backup screens at the largest accessibility text size keep their text whole and their
/// actions reachable by scrolling
///
/// XCUITest reports a text's whole label even when it draws truncated, so the screens' texts and
/// action labels carry the debug truncation probe (docs/testing.md), and rows and progress that
/// merge their texts into one element carry its report. The checks fail when a text or action
/// never lies wholly inside the window below the navigation bar, when an action takes no tap or
/// stays disabled, and when a probe reports its text truncated or past the side of the window
final class CloudBackupLargeTextFullAppUITests: XCTestCase {
    private let firstRecordId = "24a462de41c1dcbfba825e87674d526b4fc0b7811fb0cee65232cb4f8209c11d"
    private let secondRecordId = "b1089e1017cd2b2a2e0085048b346f2e1c4a850a65b69a43b8d839cd60bd3f6b"
    private let thirdRecordId = "fa5eab18295a70b89491f347427099da2dd9d528e59d85b8934084fa01119914"

    /// The message a passkey registration that fails after the system sheet showed leaves on the
    /// missing passkey screen, from `PASSKEY_ACCESS_RECOVERY_MESSAGE` in Rust
    private static let passkeyAccessRecoveryMessage = "Cove couldn't access your passkey. Check your connection and passkey account, then try again. If this keeps happening, choose another passkey provider or contact support."

    private var app: XCUIApplication!
    private var robot: CloudBackupHostRobot!
    private var layout: LargeTextLayout!
    /// The app state of a test launched into a named scenario, nil for one with a control
    private var scenarioState: CloudBackupScenarioAppState?

    override func setUp() {
        super.setUp()
        continueAfterFailure = false
        app = XCUIApplication()
        app.launchEnvironment[LargeTextLayout.probeEnvironmentKey] = "1"
        robot = CloudBackupHostRobot(app: app)
        layout = LargeTextLayout(app: app, robot: robot)
    }

    override func tearDown() {
        CloudBackupHostControl.tearDown(self, app: app)
        scenarioState?.remove()
        super.tearDown()
    }

    func testSetupDescriptionAcknowledgementsAndEnableAreReachable() {
        // MAN-03 starts with an empty cloud, so onboarding offers to enable Cloud Backup; the
        // native passkey it wires up is only used after Enable, which this test never taps
        let scenarioState = launchScenario("MAN-03")

        let getStarted = app.buttons["Get Started"]
        robot.scrollToHittable(getStarted)
        getStarted.tap()

        let newUser = robot.button(startingWith: "No, I")
        robot.scrollToHittable(newUser)
        newUser.tap()

        let enableFromBackup = robot.button(startingWith: "Enable")
        robot.scrollToHittable(enableFromBackup)
        enableFromBackup.tap()
        XCTAssertTrue(app.staticTexts["Cloud Backup"].waitForExistence(timeout: 10))

        robot.scrollToHittable(robot.staticText(containing: "end-to-end encrypted before it leaves your device"))
        robot.scrollToHittable(app.staticTexts["How It Works"])

        [
            "my passkey is required",
            "access to my iCloud account",
            "manually back up my 12 or 24 words",
        ].forEach { fragment in
            let acknowledgement = robot.staticText(containing: fragment)
            robot.scrollToHittable(acknowledgement)
            acknowledgement.tap()
        }

        let enable = app.buttons["Enable Cloud Backup"]
        robot.scrollToHittable(enable)
        XCTAssertTrue(enable.isEnabled, "Enable Cloud Backup stayed disabled after every acknowledgement")
        scenarioState.assertNextActions("onboarding should offer Enable", body: "disabled", available: ["enable"])
    }

    /// The inventory's header, its on-device and cloud-only rows, Restore All, and the verified
    /// result with its actions
    func testInventoryCloudOnlyRowsAndVerificationActionsAreReachable() {
        let scenarioState = openCloudBackupDetail(scenario: "SC-U02")

        layout.assertTextFits(layout.headerTitle, "inventory header title")
        // an on-device row only shows details, so it takes no taps
        layout.assertRowFits(recordId: firstRecordId, "on-device row", isAction: false)

        // the cloud-only rows appear once the inventory check finishes listing the cloud
        layout.assertTextFits(app.staticTexts["Not on This Device"], "cloud-only section header", initialWait: 20)
        let restoreAll = layout.assertActionFits(robot.button(startingWith: "Restore All (2)"), "Restore All")
        layout.assertLargestTextSize(restoreAll)
        layout.assertRowFits(recordId: secondRecordId, "cloud-only row two")
        layout.assertRowFits(recordId: thirdRecordId, "cloud-only row three")

        // restore can finish verification on its own, which turns Verify Now into Verify Again
        layout.assertActionFits(robot.button(startingWith: "Verify"), "Verify").tap()

        layout.assertTextFits(app.staticTexts["Backup verified"], "verified result", initialWait: 30)
        layout.assertTextFits(layout.staticText(endingWith: "wallet(s) verified"), "verified summary")
        layout.assertActionFits(robot.button(startingWith: "Verify Again"), "Verify Again")
        layout.assertActionFits(app.buttons[CloudBackupUIIdentifier.action("turnOff")], "Turn Off")
        scenarioState.assertNextActions(
            "the verified inventory should offer what the screen draws",
            available: ["verify", "startRestoreAll", "turnOff"]
        )
    }

    func testRestoreAllProgressKeepsCancelReachable() throws {
        let control = try CloudBackupRestoreAllFullAppUITests.openDetailWithTwoCloudOnlyWallets(
            app: app,
            robot: robot,
            arguments: CloudBackupHostRobot.largestContentSizeArguments
        )
        let screen = RestoreAllScreen(app: app, robot: robot)

        try control.holdWalletDownload(
            id: "wallet-two",
            recordId: CloudBackupRestoreAllFullAppUITests.walletTwoRecordId,
            skippingCalls: 1
        )
        screen.tapRestoreAll(count: 2)
        control.waitForHold("wallet-two")
        control.assertNextActions("a running batch should offer Cancel", available: ["cancelRestoreAll"])

        screen.waitForProgress(containing: ["Restoring Fixture Wallet Two", "Completed 0 of 2"])
        layout.assertReportFits(
            screen.progress,
            report: "cloudBackup.restoreAll.progress.truncation",
            "Restore All progress"
        )
        layout.assertActionFits(app.buttons["Cancel"], "Restore All Cancel")
        control.release("wallet-two")
    }

    /// A cancelled verification offers only Verify. A verification that finds no passkey and can't
    /// register a new one leaves the missing passkey card, whose Add Passkey shows the card again
    /// while its repair runs and, once that registration fails too, with the error and Add Passkey
    ///
    /// The card also shows for a backup whose saved state lost its passkey, which only older
    /// databases hold; both cases draw the same view
    func testMissingPasskeyRecoveryKeepsAddPasskeyReachable() throws {
        let control = try CloudBackupHostControl(
            script: .init(cloud: .fixture, listedWalletRecordIds: [firstRecordId])
        )
        robot.launch(control: control, arguments: CloudBackupHostRobot.largestContentSizeArguments)
        robot.restoreFromCloudBackup()
        robot.openSettings()
        robot.openCloudBackupDetail()
        control.waitForAppState("the restored backup never settled as verified", timeout: 60) {
            $0.inventoryLoaded && $0.verification == "verified"
                && !$0.awaitingUploadConfirmation
        }

        // a cancel shows nothing about the passkey being gone, so Verify is the only way on
        try control.answerNextPasskeyAssertions([.userCancelled])
        layout.assertActionFits(robot.button(startingWith: "Verify Again"), "Verify Again").tap()
        control.waitForAppState("cancelling the passkey should leave verification not completed") {
            $0.verification == "notCompleted"
        }
        control.assertNextActions(
            "a cancelled verification should offer Verify, not a passkey repair",
            available: ["verify", "turnOff"],
            unavailable: ["repairPasskey"]
        )

        layout.assertTextFits(app.staticTexts["Cloud Backup Not Verified"], "cancelled verification")
        let verifyNow = layout.assertActionFits(app.buttons["Verify Now"], "cancelled verification: Verify Now")
        XCTAssertFalse(app.buttons["Add New Passkey"].exists, "a cancelled verification offered a passkey repair")
        XCTAssertFalse(
            robot.staticText(containing: "If your passkey was deleted").exists,
            "a cancelled verification said the passkey may be deleted"
        )

        // neither the saved passkey nor discovery answers, so verification registers a new passkey
        // to protect the master key again, and the host fails it as a provider would after its
        // sheet showed
        let createsBeforeVerify = control.state.count(.passkeyCreate)
        try control.answerNextPasskeyAssertions([.noCredential, .noCredential])
        try control.update {
            $0.createResults = Array(repeating: .success, count: createsBeforeVerify) + [.postPresentationFailure]
        }
        verifyNow.tap()
        control.assertNextActions(
            "a verification that failed on the passkey should offer its repair",
            available: ["repairPasskey", "turnOff"]
        )
        assertMissingPasskeyScreenFits("verification failed on the passkey")

        // the card's own repair registers once more, held so the running card can be checked
        let createsBeforeRepair = control.state.count(.passkeyCreate)
        try control.update {
            $0.createResults = Array(repeating: .success, count: createsBeforeRepair) + [.postPresentationFailure]
            $0.holds.append(.init(id: "repair-create", point: .passkeyCreate, matchingCallsToSkip: createsBeforeRepair))
        }
        layout.assertActionFits(app.buttons["Add Passkey"], "missing passkey: Add Passkey").tap()
        control.waitForHold("repair-create")

        assertMissingPasskeyScreenFits("repair running")
        let opening = app.buttons.matching(NSPredicate(format: "label CONTAINS %@", "Opening Passkey Options")).firstMatch
        layout.assertActionFits(opening, "repair running: progress", enabled: false)

        control.release("repair-create")
        layout.assertTextFits(app.staticTexts.matching(NSPredicate(format: "label == %@", Self.passkeyAccessRecoveryMessage)).firstMatch, "repair failed: error", initialWait: 30)
        assertMissingPasskeyScreenFits("repair failed")
        layout.assertActionFits(app.buttons["Add Passkey"], "repair failed: Add Passkey")
        layout.assertActionFits(app.buttons[CloudBackupUIIdentifier.action("turnOff")], "repair failed: Turn Off")
    }

    /// A verification the provider can't serve the master key backup to fails with Try Again and
    /// no passkey card, since the passkey is fine; one that finds no passkey and can't register a
    /// new one offers the passkey repair
    func testFailedVerificationKeepsRecoveryActionsReachable() throws {
        let control = try CloudBackupHostControl(
            script: .init(cloud: .fixture, listedWalletRecordIds: [firstRecordId])
        )
        robot.launch(control: control, arguments: CloudBackupHostRobot.largestContentSizeArguments)
        robot.restoreFromCloudBackup()
        robot.openSettings()
        robot.openCloudBackupDetail()
        control.waitForAppState("the restored backup never settled as verified", timeout: 60) {
            $0.inventoryLoaded && $0.verification == "verified"
                && !$0.awaitingUploadConfirmation
        }

        let downloadsSoFar = control.state.count(.masterDownload)
        try control.update {
            $0.masterDownloadFailure = .init(callsToSkip: downloadsSoFar, error: .notAvailable)
        }
        layout.assertActionFits(robot.button(startingWith: "Verify Again"), "Verify Again").tap()
        control.waitForAppState("the unavailable master key backup should fail verification", timeout: 30) {
            $0.verification == "failed"
        }
        control.assertNextActions(
            "a failed download should offer Verify again, not a passkey repair",
            available: ["verify", "turnOff"],
            unavailable: ["repairPasskey"]
        )

        layout.assertTextFits(robot.staticText(containing: "temporarily unavailable"), "failed verification: error")
        let tryAgain = layout.assertActionFits(robot.button(startingWith: "Try Again"), "failed verification: Try Again")
        XCTAssertFalse(app.buttons["Add Passkey"].exists, "a failed download offered a passkey repair")

        // the download answers again, but neither the saved passkey nor discovery does, and the
        // replacement passkey fails as a provider would after its sheet showed
        let createsBeforeRetry = control.state.count(.passkeyCreate)
        try control.answerNextPasskeyAssertions([.noCredential, .noCredential])
        try control.update {
            $0.masterDownloadFailure = nil
            $0.createResults = Array(repeating: .success, count: createsBeforeRetry) + [.postPresentationFailure]
        }
        tryAgain.tap()
        control.assertNextActions(
            "a verification that failed on the passkey should offer its repair",
            available: ["repairPasskey", "turnOff"]
        )
        assertMissingPasskeyScreenFits("verification failed on the passkey")
        layout.assertActionFits(app.buttons["Add Passkey"], "missing passkey: Add Passkey")
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

        control.assertNextActions(
            "the relaunch should hold the interrupted setup for recovery",
            body: "pendingEnableRecovery",
            available: ["confirmPendingEnableCleanup"]
        )
        layout.assertTextFits(app.staticTexts["Cloud Backup Needs Recovery"], "recovery title", initialWait: 30)
        layout.assertTextFits(robot.staticText(containing: "setup was interrupted"), "recovery description")
        layout.assertTextFits(robot.staticText(containing: "incomplete local setup can be removed"), "recovery explanation")
        layout.assertTextFits(app.staticTexts["Support code"], "support code title")
        layout.assertTextFits(app.staticTexts[CloudBackupUIIdentifier.recoverySupportCode], "support code")
        layout.assertActionFits(robot.button(startingWith: "Copy Code"), "Copy Code")
        layout.assertActionFits(robot.button(startingWith: "Contact Support"), "Contact Support")
        layout.assertActionFits(app.buttons["Cancel"], "recovery Cancel")

        let removeSetupIdentifier = CloudBackupUIIdentifier.action("confirmPendingEnableCleanup")
        layout.assertActionFits(app.buttons[removeSetupIdentifier], "Remove Incomplete Setup").tap()

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

    /// The Turn Off review that asks twice, its confirmation and the offline dialog keep their
    /// text and every button reachable; iOS draws them as system alerts, which scroll their
    /// content at this size and which the truncation probe can't reach
    func testTurnOffDialogsKeepTheirActionsReachable() throws {
        let control = try restoreWalletOneAndOpenDetail(listingOthers: true)

        let turnOff = layout.assertActionFits(app.buttons[CloudBackupUIIdentifier.action("turnOff")], "Turn Off")
        turnOff.tap()
        let review = layout.assertAlertFits(app.alerts["Turn Off Cloud Backup?"], containing: "Fixture Wallet Two", "review")
        control.assertNextActions("the review should be confirmable", available: ["confirmDisable", "cancelDisable"])
        layout.assertAlertActionReachable(review.alertButton(CloudBackupUIIdentifier.action("restoreFirst")), in: review, "review: Restore Them First")
        layout.assertAlertActionReachable(review.alertButton(CloudBackupUIIdentifier.action("cancelDisable")), in: review, "review: Cancel")
        layout.assertAlertActionReachable(review.alertButton(CloudBackupUIIdentifier.turnOffContinue), in: review, "review: Continue").tap()

        let confirmation = layout.assertAlertFits(
            app.alerts["Delete These Backups and Turn Off Cloud Backup?"],
            containing: "can't be undone",
            "confirmation"
        )
        layout.assertAlertActionReachable(confirmation.alertButton(CloudBackupUIIdentifier.action("confirmDisable")), in: confirmation, "confirmation: Delete and Turn Off")
        layout.assertAlertActionReachable(confirmation.alertButton(CloudBackupUIIdentifier.action("cancelDisable")), in: confirmation, "confirmation: Cancel").tap()
        XCTAssertTrue(confirmation.waitForNonExistence(timeout: 10))

        try control.setNetworkConnected(false)
        layout.assertActionFits(app.buttons[CloudBackupUIIdentifier.action("turnOff")], "Turn Off offline").tap()
        let offline = layout.assertAlertFits(app.alerts["Can't Reach iCloud"], containing: "needs a connection", "offline")
        layout.assertAlertActionReachable(offline.alertButton(CloudBackupUIIdentifier.turnOffOfflineTryAgain), in: offline, "offline: Try Again")
        layout.assertAlertActionReachable(offline.alertButton(CloudBackupUIIdentifier.action("cancelDisable")), in: offline, "offline: Cancel").tap()
        XCTAssertEqual(control.state.count(.namespaceDelete), 0, "checking the dialogs deleted a backup")
    }

    /// The notice a healed legacy Turn Off leaves keeps its text whole and Turn Off and Dismiss
    /// reachable
    func testHealNoticeKeepsTurnOffAndDismissReachable() throws {
        let control = try restoreWalletOneAndOpenDetail(listingOthers: false)

        try control.requestLegacyStuckRecordOnNextLaunch()
        robot.killAndRelaunch()
        control.waitForLegacyStuckRecordSeeded()
        robot.openSettings()
        control.assertNextActions("the healed backup should offer Turn Off and Dismiss", available: ["turnOff", "dismissHealNotice"])

        layout.assertTextFits(app.staticTexts["Cloud Backup Turned On Again"], "Settings row: title", initialWait: 15)
        layout.assertTextFits(robot.staticText(containing: "backing up your wallets again"), "Settings row: message")
        robot.openCloudBackupDetailFromSettingsRow()

        // the title carries the notice's identifier, so it is found by its label
        layout.assertTextFits(robot.staticText(containing: "Cloud Backup is on again"), "heal notice: title", initialWait: 15)
        layout.assertTextFits(robot.staticText(containing: "An earlier Turn Off didn't finish"), "heal notice: description")
        layout.assertActionFits(app.buttons[CloudBackupUIIdentifier.healNoticeTurnOff], "heal notice: Turn Off")
        layout.assertActionFits(app.buttons[CloudBackupUIIdentifier.action("dismissHealNotice")], "heal notice: Dismiss")
    }

    /// A Turn Off whose delete failed after the confirmation keeps the Settings row's status and
    /// the panel's text whole, with Try Again reachable
    func testTurnOffIncompleteRowAndPanelKeepTryAgainReachable() throws {
        let control = try restoreWalletOneAndOpenDetail(listingOthers: false)
        try control.failNamespaceDelete(attempt: 1)

        layout.assertActionFits(app.buttons[CloudBackupUIIdentifier.action("turnOff")], "Turn Off").tap()
        let review = layout.assertAlertFits(app.alerts["Turn Off Cloud Backup?"], containing: "This deletes the backups", "review")
        layout.assertAlertActionReachable(review.alertButton(CloudBackupUIIdentifier.action("confirmDisable")), in: review, "review: Turn Off and Delete Backups").tap()
        control.assertNextActions(
            "the failed delete should leave Turn Off unfinished",
            body: "finishingDisable",
            available: ["resumeDisable"],
            timeout: 60
        )

        layout.assertTextFits(robot.staticText(containing: "Cloud Backup is being turned off"), "panel: title", initialWait: 15)
        layout.assertTextFits(robot.staticText(containing: "couldn't finish deleting your backups"), "panel: description")
        layout.assertActionFits(app.buttons[CloudBackupUIIdentifier.action("resumeDisable")], "panel: Try Again")

        robot.goBack()
        XCTAssertTrue(app.navigationBars["Settings"].waitForExistence(timeout: 10))
        layout.assertTextFits(app.staticTexts["Cloud Backup Is Being Turned Off"], "Settings row: title", initialWait: 15)
        layout.assertTextFits(robot.staticText(containing: "Open to finish turning off"), "Settings row: message")
    }

    /// Restores Fixture Wallet One at the largest text size and opens Cloud Backup once it is
    /// verified, with wallets two and three listed only in the cloud when `listingOthers`
    private func restoreWalletOneAndOpenDetail(listingOthers: Bool) throws -> CloudBackupHostControl {
        let control = try CloudBackupHostControl(
            script: .init(cloud: .fixture, listedWalletRecordIds: [firstRecordId])
        )
        robot.launch(control: control, arguments: CloudBackupHostRobot.largestContentSizeArguments)
        robot.restoreFromCloudBackup()
        if listingOthers {
            try control.update { $0.listedWalletRecordIds = nil }
        }

        let notOnDevice = listingOthers ? [secondRecordId, thirdRecordId] : []
        robot.openSettings()
        robot.openCloudBackupDetail()
        control.waitForAppState("the restored backup never settled as verified", timeout: 60) {
            $0.inventoryLoaded && $0.verification == "verified" && !$0.awaitingUploadConfirmation
                && $0.notOnDeviceRecordIds == notOnDevice
        }

        return control
    }

    private func assertMissingPasskeyScreenFits(_ screen: String) {
        layout.assertTextFits(app.staticTexts["Cloud Backup Passkey Missing"], "\(screen): title", initialWait: 15)
        layout.assertTextFits(robot.staticText(containing: "is not accessible until you use an existing passkey"), "\(screen): description")
        layout.assertTextFits(robot.staticText(containing: "to make your cloud backup accessible again"), "\(screen): help")
        layout.assertTextFits(robot.staticText(containing: "If your passkey was deleted"), "\(screen): deleted passkey help")
    }

    /// Launches the host into `scenario` at the largest text size, with its app state read from
    /// the file the scenario host writes
    private func launchScenario(_ scenario: String) -> CloudBackupScenarioAppState {
        let state = CloudBackupScenarioAppState(app: app)
        scenarioState = state
        robot.launch(scenario: scenario, arguments: CloudBackupHostRobot.largestContentSizeArguments)

        return state
    }

    private func openCloudBackupDetail(scenario: String) -> CloudBackupScenarioAppState {
        let state = launchScenario(scenario)
        robot.restoreFromCloudBackup()
        robot.openSettings()

        let cloudBackup = app.staticTexts["Cloud Backup Enabled"]
        robot.scrollToHittable(cloudBackup)
        cloudBackup.tap()
        XCTAssertTrue(app.navigationBars["Cloud Backup"].waitForExistence(timeout: 10))

        return state
    }
}

/// Scrolls a Cloud Backup screen as a user would and checks that its texts draw whole and its
/// actions can be reached
private struct LargeTextLayout {
    /// `UITestTruncationProbe.environmentKey` in the app
    static let probeEnvironmentKey = "COVE_UI_TEST_TRUNCATION_PROBE"

    /// The value a probed text or a truncation report has when everything in it draws whole
    private static let fitsValue = "fits"
    /// The values a probed text or a truncation report has when part of it doesn't draw whole
    private static let notWholeValues: Set<String> = ["truncated", "hyphenated", "offscreen"]

    /// An action's height at the default text size is at most 44 points, and a one-line label at
    /// the largest size is about 53 points tall before its padding
    private static let largestTextMinimumActionHeight: CGFloat = 60

    /// Inside the Form's 16 to 20 point side padding, so a drag there lands on no row's action
    private static let dragMargin: CGFloat = 8

    let app: XCUIApplication
    let robot: CloudBackupHostRobot

    /// The inventory header's title, which names the sync health the provider reports
    var headerTitle: XCUIElement {
        let titles = ["Cloud Backup Active", "Cloud Backup Syncing", "Checking Cloud Backup", "Cloud Backup Needs Attention"]
        return app.staticTexts.matching(NSPredicate(format: "label IN %@", titles)).firstMatch
    }

    func staticText(endingWith suffix: String) -> XCUIElement {
        app.staticTexts.matching(NSPredicate(format: "label ENDSWITH %@", suffix)).firstMatch
    }

    /// Scrolls the text wholly into view and fails unless its probe reports it draws whole
    func assertTextFits(_ element: XCUIElement, _ name: String, initialWait: TimeInterval = 10) {
        scrollWhollyIntoView(element, name, needsHit: false, initialWait: initialWait)
        assertProbeFits(element, name)
    }

    /// Scrolls the action wholly into view, checks it takes a tap there and, unless `enabled` is
    /// false, that it is enabled, and fails unless its label's probe reports it draws whole
    ///
    /// A button takes its label's accessibility value, so a button whose label dropped the probe
    /// fails here too
    @discardableResult
    func assertActionFits(_ element: XCUIElement, _ name: String, enabled: Bool = true, initialWait: TimeInterval = 10) -> XCUIElement {
        scrollWhollyIntoView(element, name, needsHit: true, initialWait: initialWait)
        if enabled {
            XCTAssertTrue(element.isEnabled, "\(name) is disabled")
        } else {
            XCTAssertFalse(element.isEnabled, "\(name) should be disabled while it runs")
        }

        assertProbeFits(element, name)
        return element
    }

    /// Scrolls a wallet row wholly into view, checks a row that is an action takes a tap there and
    /// is enabled, and fails unless every text in it draws whole
    func assertRowFits(recordId: String, _ name: String, isAction: Bool = true, initialWait: TimeInterval = 10) {
        let row = app.descendants(matching: .any)[CloudBackupUIIdentifier.wallet(recordId)]
        scrollWhollyIntoView(row, name, needsHit: isAction, initialWait: initialWait)
        if isAction {
            XCTAssertTrue(row.isEnabled, "\(name) is disabled")
        }

        assertProbeFits(app.descendants(matching: .any)["cloudBackup.wallet.\(recordId).truncation"], name)
    }

    /// Scrolls an element that merges its texts wholly into view and fails unless the truncation
    /// report laid over it says every one of them draws whole
    func assertReportFits(_ element: XCUIElement, report: String, _ name: String) {
        scrollWhollyIntoView(element, name, needsHit: false, initialWait: 10)
        assertProbeFits(app.descendants(matching: .any)[report], name)
    }

    /// Waits for a system alert and fails unless one of its texts contains `fragment`
    ///
    /// A system alert's text is drawn by iOS, which scrolls it rather than truncating it, so the
    /// text only has to be there
    @discardableResult
    func assertAlertFits(_ alert: XCUIElement, containing fragment: String, _ name: String) -> XCUIElement {
        XCTAssertTrue(alert.waitForExistence(timeout: 20), "\(name) never showed")
        let text = alert.staticTexts.matching(NSPredicate(format: "label CONTAINS %@", fragment)).firstMatch
        XCTAssertTrue(text.exists, "\(name) doesn't say '\(fragment)'")

        return alert
    }

    /// Scrolls a system alert's content until `button` takes a tap, and fails if it never does
    @discardableResult
    func assertAlertActionReachable(_ button: XCUIElement, in alert: XCUIElement, _ name: String) -> XCUIElement {
        XCTAssertTrue(button.waitForExistence(timeout: 5), "\(name) is missing")
        for _ in 0 ..< 6 where !button.isHittable {
            alert.swipeUp(velocity: .slow)
        }
        for _ in 0 ..< 6 where !button.isHittable {
            alert.swipeDown(velocity: .slow)
        }

        if !button.isHittable { robot.attachScreen(named: "\(name) unreachable") }
        XCTAssertTrue(button.isHittable, "\(name) takes no tap in its alert")
        XCTAssertTrue(button.isEnabled, "\(name) is disabled")

        return button
    }

    /// Fails when the action is no taller than at the default text size, so a launch that ignored
    /// the text size argument can't pass
    func assertLargestTextSize(_ action: XCUIElement) {
        XCTAssertGreaterThanOrEqual(
            action.frame.height,
            Self.largestTextMinimumActionHeight,
            "\(action) is \(action.frame.height)pt tall, so the largest text size didn't take effect"
        )
    }

    /// The probe measures after layout, so its value gets a moment to settle
    ///
    /// A button joins the values of everything in its label, so one that shows a spinner beside
    /// its probed text reads as `1, fits`; it fits when the probe's part says so and no part says
    /// it doesn't draw whole
    private func assertProbeFits(_ element: XCUIElement, _ name: String) {
        XCTAssertTrue(element.waitForExistence(timeout: 5), "\(name) has no truncation probe")

        let value = { element.value as? String ?? "" }
        let fits = {
            let parts = value().components(separatedBy: ", ")
            return parts.contains(Self.fitsValue) && Self.notWholeValues.isDisjoint(with: parts)
        }

        let settled = Date().addingTimeInterval(3)
        while !fits(), Date() < settled {
            RunLoop.current.run(until: Date().addingTimeInterval(0.25))
        }

        if !fits() { robot.attachScreen(named: "\(name) truncated") }
        XCTAssertTrue(fits(), "\(name) doesn't draw whole: its truncation probe reports '\(value())'")
    }

    /// Scrolls until the element lies wholly inside the window below the navigation bar, and for
    /// an action until it takes a tap there
    ///
    /// The robot's scroll stops once the element's center can take a tap, which leaves a tall
    /// row or a text half under the navigation bar, so short drags then move it the rest of the
    /// way. A text or a row that takes no taps is never hittable, so it scrolls until it shows
    /// instead
    private func scrollWhollyIntoView(_ element: XCUIElement, _ name: String, needsHit: Bool, initialWait: TimeInterval) {
        if needsHit {
            robot.scrollToHittable(element, initialWait: initialWait)
        } else {
            scrollOnScreen(element, initialWait: initialWait)
        }

        for _ in 0 ..< 6 where !isWhollyReadable(element) {
            drag(revealingContentBelow: element.frame.maxY > readableArea.maxY)
        }

        if !isWhollyReadable(element) { robot.attachScreen(named: "\(name) clipped") }
        XCTAssertTrue(
            isWhollyReadable(element),
            "\(name) never fit inside \(readableArea): it lies at \(element.frame)"
        )

        if needsHit {
            XCTAssertTrue(element.isHittable, "\(name) takes no tap where it is drawn")
        }
    }

    /// Swipes toward the element while its frame says where it is, and otherwise down the list
    /// and then back up, since a list only builds rows near the screen
    private func scrollOnScreen(_ element: XCUIElement, initialWait: TimeInterval, maxSwipes: Int = 20) {
        _ = element.waitForExistence(timeout: initialWait)

        var swipes = 0
        while !isOnScreen(element), swipes < maxSwipes * 2 {
            let revealBelow = element.exists && !element.frame.isEmpty
                ? element.frame.minY >= readableArea.maxY
                : swipes < maxSwipes

            if revealBelow {
                app.swipeUp(velocity: .slow)
            } else {
                app.swipeDown(velocity: .slow)
            }
            swipes += 1
        }
    }

    private func isOnScreen(_ element: XCUIElement) -> Bool {
        element.exists && !element.frame.isEmpty && readableArea.intersects(element.frame)
    }

    private func isWhollyReadable(_ element: XCUIElement) -> Bool {
        guard element.exists else { return false }

        let frame = element.frame
        return !frame.isEmpty && readableArea.insetBy(dx: -0.5, dy: -0.5).contains(frame)
    }

    /// The window below the top navigation bar, where content scrolled beneath the bar's
    /// translucent material no longer counts as shown
    private var readableArea: CGRect {
        let window = app.windows.firstMatch.frame
        let barBottom = app.navigationBars.allElementsBoundByIndex
            .map(\.frame)
            .filter { $0.minY >= 0 && $0.maxY < window.midY && $0.width > 0 }
            .map(\.maxY)
            .max() ?? window.minY

        return CGRect(x: window.minX, y: barBottom, width: window.width, height: window.maxY - barBottom)
    }

    /// Drags the screen by a fifth of its height along its leading margin
    private func drag(revealingContentBelow: Bool) {
        let window = app.windows.firstMatch
        let x = Self.dragMargin / max(window.frame.width, 1)
        let point = { (y: CGFloat) in window.coordinate(withNormalizedOffset: CGVector(dx: x, dy: y)) }

        let (start, end) = revealingContentBelow ? (point(0.6), point(0.4)) : (point(0.4), point(0.6))
        start.press(forDuration: 0.05, thenDragTo: end, withVelocity: .slow, thenHoldForDuration: 0.1)
    }
}
