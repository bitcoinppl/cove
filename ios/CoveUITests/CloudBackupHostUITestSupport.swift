import XCTest

/// Drives the scripted Cloud Backup host app through the steps several UI test classes share
///
/// The host app swaps in scripted cloud storage and passkey providers, so restore and enable run
/// without iCloud or a real passkey. Each launch wipes local data first
struct CloudBackupHostRobot {
    let app: XCUIApplication

    /// Largest accessibility text size, as set in Settings > Accessibility > Display & Text Size
    static let largestContentSizeArguments = [
        "-UIPreferredContentSizeCategoryName",
        "UICTContentSizeCategoryAccessibilityXXXL",
    ]

    /// Launches the host into onboarding, relaunching once if the bootstrap watchdog fires on a
    /// cold simulator
    func launch(scenario: String?, arguments: [String] = []) {
        app.launchEnvironment["COVE_CLOUD_BACKUP_UI_RESET"] = "1"
        app.launchEnvironment["COVE_CLOUD_BACKUP_UI_SCENARIO"] = scenario
        app.launchArguments += arguments
        app.launch()

        guard waitForOnboardingOrStartupTimeout() == .startupTimedOut else { return }

        app.terminate()
        app.launch()

        // xtask retries the test, or a cloud backup class's tests that didn't pass, on a clean
        // simulator when it sees this message
        if waitForOnboardingOrStartupTimeout() == .startupTimedOut {
            XCTFail("App startup timed out on relaunch")
        }
    }

    /// Launches the host driven by `control`'s script, after wiping local data
    func launch(control: CloudBackupHostControl, arguments: [String] = []) {
        control.configure(app)
        launch(scenario: nil, arguments: arguments)
    }

    /// Kills the app and starts a new process on the same data, keychain and cloud, as a person
    /// force-quitting and reopening Cove would
    ///
    /// `whileStopped` runs after the app has stopped and before the new process starts
    func killAndRelaunch(whileStopped: () throws -> Void) rethrows {
        app.terminate()
        XCTAssertTrue(app.wait(for: .notRunning, timeout: 10), "the app did not stop")
        try whileStopped()

        // the launch environment carries over to the next launch, and the reset would wipe the
        // data this relaunch must keep
        app.launchEnvironment.removeValue(forKey: "COVE_CLOUD_BACKUP_UI_RESET")
        app.launch()

        XCTAssertTrue(app.wait(for: .runningForeground, timeout: 20), "the relaunched app did not start")
        if staticText(containing: "App startup timed out").waitForExistence(timeout: 3) {
            XCTFail("App startup timed out on relaunch")
        }
    }

    func killAndRelaunch() {
        killAndRelaunch(whileStopped: {})
    }

    /// Starts a fresh Cloud Backup enable as a new user from onboarding, through the passkey
    /// acknowledgements and Enable Cloud Backup
    func startFreshEnableFromOnboarding() {
        let getStarted = app.buttons["Get Started"]
        XCTAssertTrue(getStarted.waitForExistence(timeout: 15))
        scrollToHittable(getStarted)
        getStarted.tap()

        if app.staticTexts["Terms & Conditions"].waitForExistence(timeout: 2) {
            acceptTerms()
        }

        let newUser = button(startingWith: "No, I")
        scrollToHittable(newUser)
        newUser.tap()

        openCloudBackupFromBackupStep()
        acceptCloudBackupAcknowledgements()
        tapEnableCloudBackup()
    }

    /// Opens the Cloud Backup details from onboarding's backup step through its Enable button
    func openCloudBackupFromBackupStep() {
        let enableFromBackup = button(startingWith: "Enable")
        scrollToHittable(enableFromBackup, initialWait: 20)
        enableFromBackup.tap()
        XCTAssertTrue(app.staticTexts["Cloud Backup"].waitForExistence(timeout: 10))
    }

    func acceptCloudBackupAcknowledgements() {
        [
            "my passkey is required",
            "access to my iCloud account",
            "manually back up my 12 or 24 words",
        ].forEach { fragment in
            let acknowledgement = staticText(containing: fragment)
            scrollToHittable(acknowledgement)
            acknowledgement.tap()
        }
    }

    func tapEnableCloudBackup() {
        let enable = app.buttons["Enable Cloud Backup"]
        scrollToHittable(enable)
        XCTAssertTrue(enable.isEnabled, "Enable Cloud Backup is disabled")
        enable.tap()
    }

    /// Waits for onboarding's Cloud Backup success and continues into the app
    func finishOnboardingAfterCloudBackupEnabled() {
        XCTAssertTrue(app.staticTexts["Cloud Backup enabled"].waitForExistence(timeout: 30))
        let continueButton = app.buttons["Continue"]
        scrollToHittable(continueButton)
        continueButton.tap()

        // the success step returns to the backup step, which now shows Cloud Backup as Enabled
        // and lets a new wallet continue to the terms without saving its words first
        let backupStep = app.staticTexts.matching(
            NSPredicate(format: "label BEGINSWITH %@", "Back up your wallet")
        ).firstMatch
        XCTAssertTrue(backupStep.waitForExistence(timeout: 15))
        XCTAssertTrue(app.buttons["Enabled"].waitForExistence(timeout: 5))
        let continueFromBackup = app.buttons["Continue"]
        scrollToHittable(continueFromBackup)
        continueFromBackup.tap()

        XCTAssertTrue(app.staticTexts["Terms & Conditions"].waitForExistence(timeout: 10))
        acceptTerms()
    }

    /// Opens Cloud Backup detail from Settings, whose row title follows the backup's status
    func openCloudBackupDetail(rowTitle: String = "Cloud Backup Enabled") {
        let row = app.staticTexts[rowTitle]
        scrollToHittable(row, initialWait: 15)
        row.tap()
        XCTAssertTrue(app.navigationBars["Cloud Backup"].waitForExistence(timeout: 10))
    }

    /// Opens Cloud Backup detail from its Settings row whatever status the row shows, for a test
    /// whose scenario leaves the status to Rust
    func openCloudBackupDetailFromSettingsRow() {
        let row = app.buttons["settings.cloudBackup"]
        scrollToHittable(row, initialWait: 15)
        row.tap()
        XCTAssertTrue(app.navigationBars["Cloud Backup"].waitForExistence(timeout: 10))
    }

    /// Goes back one screen with the navigation bar's back button
    func goBack() {
        let back = app.navigationBars.buttons.element(boundBy: 0)
        XCTAssertTrue(back.waitForExistence(timeout: 5))
        back.tap()
    }

    private enum LaunchOutcome {
        case onboarding
        case startupTimedOut
        case unknown
    }

    /// Waits for onboarding, which every launch lands on since each wipes local data first
    private func waitForOnboardingOrStartupTimeout() -> LaunchOutcome {
        let welcome = app.staticTexts["Welcome to Cove"]
        let restore = app.buttons["Restore with Passkey"]
        let startupTimedOut = staticText(containing: "App startup timed out")

        // the watchdog allows up to 20 seconds while a migration runs
        let deadline = Date().addingTimeInterval(30)
        while Date() < deadline {
            if welcome.exists || restore.exists { return .onboarding }
            if startupTimedOut.exists { return .startupTimedOut }

            Thread.sleep(forTimeInterval: 0.5)
        }

        return .unknown
    }

    /// Restores the scenario's cloud wallets from onboarding and lands on the selected wallet
    func restoreFromCloudBackup() {
        let restore = app.buttons["Restore with Passkey"]
        if !restore.waitForExistence(timeout: 10) {
            XCTAssertTrue(app.staticTexts["Welcome to Cove"].waitForExistence(timeout: 10))
            let getStarted = app.buttons["Get Started"]
            scrollToHittable(getStarted)
            getStarted.tap()

            if !restore.waitForExistence(timeout: 15) {
                let manualRestore = button(startingWith: "Restore from Cove backup")
                XCTAssertTrue(manualRestore.waitForExistence(timeout: 10))
                scrollToHittable(manualRestore)
                manualRestore.tap()
                XCTAssertTrue(restore.waitForExistence(timeout: 10))
            }
        }
        scrollToHittable(restore)
        restore.tap()

        XCTAssertTrue(app.staticTexts["You’re all set"].waitForExistence(timeout: 30), "restore did not complete")
        let done = app.buttons["Done"]
        scrollToHittable(done)
        done.tap()

        if app.staticTexts["Terms & Conditions"].waitForExistence(timeout: 10) {
            acceptTerms()
        }
    }

    /// Accepts the terms, scrolling to each checkbox since large text pushes them off screen
    func acceptTerms() {
        if !app.staticTexts["Terms & Conditions"].waitForExistence(timeout: 10) {
            attachScreen(named: "terms missing")
            XCTFail("the terms screen never appeared")
        }

        [
            "onboarding.terms.check.backup",
            "onboarding.terms.check.legal",
            "onboarding.terms.check.financial",
            "onboarding.terms.check.recovery",
            "onboarding.terms.check.agreement",
            "onboarding.terms.agree",
        ].forEach { identifier in
            let element = app.buttons[identifier]
            scrollToHittable(element)
            element.tap()
        }
    }

    func openSettings() {
        let openSidebar = app.buttons["app.sidebar.open"]
        XCTAssertTrue(openSidebar.waitForExistence(timeout: 30))
        openSidebar.tap()

        let settings = app.buttons["Settings"]
        XCTAssertTrue(settings.waitForExistence(timeout: 10))
        settings.tap()
        XCTAssertTrue(app.navigationBars["Settings"].waitForExistence(timeout: 10))
    }

    /// Types a PIN on the number pad that is on screen
    func enterPin(_ pin: String) {
        for digit in pin {
            let key = app.buttons[String(digit)]
            XCTAssertTrue(key.waitForExistence(timeout: 5), "number pad key \(digit) is missing")
            key.tap()
        }
    }

    /// The detail's Verify control: Verify Now before the backup is verified, Verify Again after
    var verifyControl: XCUIElement {
        app.buttons.matching(identifier: CloudBackupUIIdentifier.action("verify")).firstMatch
    }

    /// Waits until a restored backup's detail has loaded its inventory and shows the backup as not
    /// verified, with Verify Now on its card
    ///
    /// A restore doesn't count as verification, and the detail starts none on its own, so a
    /// restored backup stays not verified until the user taps Verify Now
    func assertRestoredBackupNotVerified(
        _ control: CloudBackupHostControl,
        file: StaticString = #filePath,
        line: UInt = #line
    ) {
        control.waitForAppState("a restored backup should wait for Verify Now", timeout: 60, file: file, line: line) {
            $0.inventoryLoaded && $0.verification == "notVerified"
        }
        control.assertNextActions(
            "a restored backup should offer Verify",
            available: ["verify"],
            file: file,
            line: line
        )

        scrollToHittable(verifyControl)
        XCTAssertTrue(
            verifyControl.label.hasPrefix("Verify Now"),
            "the restored backup's card offered \"\(verifyControl.label)\", not Verify Now",
            file: file,
            line: line
        )
    }

    /// Verifies a restored backup the way a user does, with Verify Now on its card, which the
    /// scripted provider answers with the passkey the restore found
    func verifyRestoredBackup(
        _ control: CloudBackupHostControl,
        file: StaticString = #filePath,
        line: UInt = #line
    ) {
        assertRestoredBackupNotVerified(control, file: file, line: line)
        verifyControl.tap()

        control.waitForAppState("Verify Now never verified the restored backup", timeout: 60, file: file, line: line) {
            $0.inventoryLoaded && $0.verification == "verified" && !$0.awaitingUploadConfirmation
        }
    }

    /// Scrolls the current screen until `element` can be tapped
    func scrollToHittable(_ element: XCUIElement, initialWait: TimeInterval = 5, maxSwipes: Int = 20) {
        // lists only build rows near the screen, so a row further down doesn't exist until a
        // swipe brings it close; wait first for content that is still loading
        _ = element.waitForExistence(timeout: initialWait)

        // slow swipes, since a fling at large text sizes can carry a row past the screen, and a
        // second pass back up for an element the test already scrolled past
        var swipes = 0
        while !(element.exists && element.isHittable), swipes < maxSwipes * 2 {
            if swipes < maxSwipes {
                app.swipeUp(velocity: .slow)
            } else {
                app.swipeDown(velocity: .slow)
            }
            swipes += 1
        }

        let reachable = element.exists && element.isHittable
        if !reachable { attachScreen(named: "unreachable \(element)") }
        XCTAssertTrue(reachable, "\(element) is not reachable after \(swipes) swipes")
    }

    /// Keeps a screenshot and the element tree in the result bundle, since layout failures are
    /// hard to read from the assertion alone
    func attachScreen(named name: String) {
        XCTContext.runActivity(named: name) { activity in
            let screenshot = XCTAttachment(screenshot: app.screenshot())
            screenshot.lifetime = .keepAlways
            activity.add(screenshot)

            let tree = XCTAttachment(string: app.debugDescription)
            tree.lifetime = .keepAlways
            activity.add(tree)
        }
    }

    func button(startingWith labelPrefix: String) -> XCUIElement {
        app.buttons.matching(NSPredicate(format: "label BEGINSWITH %@", labelPrefix)).firstMatch
    }

    func staticText(containing fragment: String) -> XCUIElement {
        app.staticTexts.matching(NSPredicate(format: "label CONTAINS[c] %@", fragment)).firstMatch
    }

    /// A listed row's action button, by the identifier both platforms share, such as `restore` or
    /// `delete` (`docs/cloud_backup.md`)
    func rowAction(_ recordId: String, _ action: String) -> XCUIElement {
        app.buttons[CloudBackupUIIdentifier.row(recordId, action)]
    }
}
