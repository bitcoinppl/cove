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
        super.tearDown()
    }

    func testSetupDescriptionAcknowledgementsAndEnableAreReachable() {
        // MAN-03 starts with an empty cloud, so onboarding offers to enable Cloud Backup; the
        // native passkey it wires up is only used after Enable, which this test never taps
        robot.launch(scenario: "MAN-03", arguments: CloudBackupHostRobot.largestContentSizeArguments)

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
    }

    /// The inventory's header, its on-device and cloud-only rows, Restore All, and the verified
    /// result with its actions
    func testInventoryCloudOnlyRowsAndVerificationActionsAreReachable() {
        openCloudBackupDetail(scenario: "SC-U02")

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
        layout.assertActionFits(app.buttons["Disable Cloud Backup"], "Disable Cloud Backup")
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

        screen.waitForProgress(containing: ["Restoring Fixture Wallet Two", "Completed 0 of 2"])
        layout.assertReportFits(
            screen.progress,
            report: "cloudBackup.restoreAll.progress.truncation",
            "Restore All progress"
        )
        layout.assertActionFits(app.buttons["Cancel"], "Restore All Cancel")
        control.release("wallet-two")
    }

    /// A passkey repair started from a cancelled verification shows the missing passkey screen
    /// while it runs and, once the new passkey's registration fails, again with the error and Add
    /// Passkey to try once more
    ///
    /// The screen also shows for a backup whose saved state lost its passkey, which only older
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
            $0.detail == "complete" && $0.inventoryComplete && $0.verification == "verified"
                && !$0.pendingUploadVerification
        }

        try control.answerNextPasskeyAssertions([.userCancelled])
        layout.assertActionFits(robot.button(startingWith: "Verify Again"), "Verify Again").tap()
        control.waitForAppState("cancelling the passkey should leave verification cancelled") {
            $0.verification == "cancelled"
        }

        layout.assertTextFits(app.staticTexts["Cloud Backup Not Verified"], "cancelled verification")
        layout.assertTextFits(robot.staticText(containing: "If your passkey was deleted"), "cancelled verification help")
        layout.assertActionFits(app.buttons["Verify Now"], "cancelled verification: Verify Now")

        // the repair registers a new passkey, which the host holds and then fails as a provider
        // would after its sheet showed
        let createsSoFar = control.state.count(.passkeyCreate)
        try control.update {
            $0.createResults = Array(repeating: .success, count: createsSoFar) + [.postPresentationFailure]
            $0.holds.append(.init(id: "repair-create", point: .passkeyCreate, matchingCallsToSkip: createsSoFar))
        }
        layout.assertActionFits(app.buttons["Add New Passkey"], "cancelled verification: Add New Passkey").tap()
        control.waitForHold("repair-create")

        assertMissingPasskeyScreenFits("repair running")
        let opening = app.buttons.matching(NSPredicate(format: "label CONTAINS %@", "Opening Passkey Options")).firstMatch
        layout.assertActionFits(opening, "repair running: progress", enabled: false)

        control.release("repair-create")
        layout.assertTextFits(app.staticTexts.matching(NSPredicate(format: "label == %@", Self.passkeyAccessRecoveryMessage)).firstMatch, "repair failed: error", initialWait: 30)
        assertMissingPasskeyScreenFits("repair failed")
        layout.assertActionFits(app.buttons["Add Passkey"], "repair failed: Add Passkey")
        layout.assertActionFits(app.buttons["Disable Cloud Backup"], "repair failed: Disable Cloud Backup")
    }

    /// A verification the provider can't serve the master key backup to fails with Try Again and
    /// a passkey repair
    func testFailedVerificationKeepsRecoveryActionsReachable() throws {
        let control = try CloudBackupHostControl(
            script: .init(cloud: .fixture, listedWalletRecordIds: [firstRecordId])
        )
        robot.launch(control: control, arguments: CloudBackupHostRobot.largestContentSizeArguments)
        robot.restoreFromCloudBackup()
        robot.openSettings()
        robot.openCloudBackupDetail()
        control.waitForAppState("the restored backup never settled as verified", timeout: 60) {
            $0.detail == "complete" && $0.inventoryComplete && $0.verification == "verified"
                && !$0.pendingUploadVerification
        }

        let downloadsSoFar = control.state.count(.masterDownload)
        try control.update {
            $0.masterDownloadFailure = .init(callsToSkip: downloadsSoFar, error: .notAvailable)
        }
        layout.assertActionFits(robot.button(startingWith: "Verify Again"), "Verify Again").tap()
        control.waitForAppState("the unavailable master key backup should fail verification", timeout: 30) {
            $0.verification == "failed"
        }

        layout.assertTextFits(robot.staticText(containing: "temporarily unavailable"), "failed verification: error")
        layout.assertActionFits(robot.button(startingWith: "Try Again"), "failed verification: Try Again")
        layout.assertActionFits(app.buttons["Add New Passkey"], "failed verification: Add New Passkey")
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

        control.waitForAppState("the relaunch should hold the interrupted setup for recovery") {
            $0.lifecycle == "pendingEnableRecovery"
        }
        layout.assertTextFits(app.staticTexts["Cloud Backup Needs Recovery"], "recovery title", initialWait: 30)
        layout.assertTextFits(robot.staticText(containing: "setup was interrupted"), "recovery description")
        layout.assertTextFits(robot.staticText(containing: "incomplete local setup can be removed"), "recovery explanation")
        layout.assertTextFits(app.staticTexts["Support code"], "support code title")
        layout.assertTextFits(app.staticTexts["cloudBackup.recovery.supportCode"], "support code")
        layout.assertActionFits(robot.button(startingWith: "Copy Code"), "Copy Code")
        layout.assertActionFits(robot.button(startingWith: "Contact Support"), "Contact Support")
        layout.assertActionFits(app.buttons["Cancel"], "recovery Cancel")

        let removeSetupIdentifier = "cloudBackup.recovery.removeIncompleteSetup"
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

    private func assertMissingPasskeyScreenFits(_ screen: String) {
        layout.assertTextFits(app.staticTexts["Cloud Backup Passkey Missing"], "\(screen): title", initialWait: 15)
        layout.assertTextFits(robot.staticText(containing: "is not accessible until you use an existing passkey"), "\(screen): description")
        layout.assertTextFits(robot.staticText(containing: "to make your cloud backup accessible again"), "\(screen): help")
    }

    private func openCloudBackupDetail(scenario: String) {
        robot.launch(scenario: scenario, arguments: CloudBackupHostRobot.largestContentSizeArguments)
        robot.restoreFromCloudBackup()
        robot.openSettings()

        let cloudBackup = app.staticTexts["Cloud Backup Enabled"]
        robot.scrollToHittable(cloudBackup)
        cloudBackup.tap()
        XCTAssertTrue(app.navigationBars["Cloud Backup"].waitForExistence(timeout: 10))
    }
}

/// Scrolls a Cloud Backup screen as a user would and checks that its texts draw whole and its
/// actions can be reached
private struct LargeTextLayout {
    /// `UITestTruncationProbe.environmentKey` in the app
    static let probeEnvironmentKey = "COVE_UI_TEST_TRUNCATION_PROBE"

    /// The value a probed text or a truncation report has when everything in it draws whole
    private static let fitsValue = "fits"
    private static let truncatedValue = "truncated"

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
        let row = app.descendants(matching: .any)["cloudBackup.wallet.\(recordId)"]
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
    /// truncated
    private func assertProbeFits(_ element: XCUIElement, _ name: String) {
        XCTAssertTrue(element.waitForExistence(timeout: 5), "\(name) has no truncation probe")

        let value = { element.value as? String ?? "" }
        let fits = {
            let parts = value().components(separatedBy: ", ")
            return parts.contains(Self.fitsValue) && !parts.contains(Self.truncatedValue)
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
