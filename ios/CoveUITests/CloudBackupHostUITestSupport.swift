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

    func launch(scenario: String, arguments: [String] = []) {
        app.launchEnvironment["COVE_CLOUD_BACKUP_UI_RESET"] = "1"
        app.launchEnvironment["COVE_CLOUD_BACKUP_UI_SCENARIO"] = scenario
        app.launchArguments += arguments
        app.launch()
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
}
